//! Hardware backends. [`SysfsBackend`] talks to the kernel driver through sysfs; [`FakeBackend`]
//! keeps everything in memory for tests and for running the daemon on unsupported machines.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::{Error, Result, MAX_BRIGHTNESS, MAX_INTENSITY};

/// Fan / performance mode. Mirrors the Control Center's *Gaming* and *Beast* modes; the kernel
/// exposes them as ACPI `platform_profile` values `balanced` and `performance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FanMode {
    #[default]
    Balanced,
    Performance,
}

impl FanMode {
    pub const ALL: [FanMode; 2] = [FanMode::Balanced, FanMode::Performance];

    /// Value written to `platform_profile`.
    pub fn as_profile(self) -> &'static str {
        match self {
            FanMode::Balanced => "balanced",
            FanMode::Performance => "performance",
        }
    }

    /// Name used by the Windows Control Center.
    pub fn windows_name(self) -> &'static str {
        match self {
            FanMode::Balanced => "Gaming",
            FanMode::Performance => "Beast",
        }
    }
}

impl fmt::Display for FanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_profile())
    }
}

impl FromStr for FanMode {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "balanced" | "gaming" | "normal" | "office" => Ok(FanMode::Balanced),
            "performance" | "beast" | "turbo" => Ok(FanMode::Performance),
            other => Err(Error::Invalid(format!(
                "unknown fan mode {other:?}; expected balanced|performance (aliases: gaming, beast)"
            ))),
        }
    }
}

/// Everything the effect engine and daemon need from the hardware.
pub trait Backend: Send {
    /// Brightness level `0..=max_brightness()`; 0 turns the backlight off.
    fn brightness(&self) -> Result<u8>;
    fn set_brightness(&mut self, level: u8) -> Result<()>;
    fn max_brightness(&self) -> u8 {
        MAX_BRIGHTNESS
    }

    /// Per-channel EC intensity `0..=50`. Implementations must never write `[0,0,0]`.
    fn intensity(&self) -> Result<[u8; 3]>;
    fn set_intensity(&mut self, rgb: [u8; 3]) -> Result<()>;

    /// Returns the last brightness change made by the hardware (Fn keys) if the backend can
    /// report it, consuming the notification. `None` when unsupported or unchanged.
    fn take_hw_brightness_change(&mut self) -> Result<Option<u8>> {
        Ok(None)
    }

    fn supports_fan_mode(&self) -> bool {
        false
    }
    fn fan_mode(&self) -> Result<FanMode> {
        Err(Error::Unsupported("fan mode".into()))
    }
    fn set_fan_mode(&mut self, _mode: FanMode) -> Result<()> {
        Err(Error::Unsupported("fan mode".into()))
    }

    fn supports_fan_boost(&self) -> bool {
        false
    }
    fn fan_boost(&self) -> Result<bool> {
        Err(Error::Unsupported("fan boost".into()))
    }
    fn set_fan_boost(&mut self, _on: bool) -> Result<()> {
        Err(Error::Unsupported("fan boost".into()))
    }

    /// Human-readable description of where the backend points (for `status`).
    fn describe(&self) -> String;
}

fn clamp_intensity(rgb: [u8; 3]) -> Result<[u8; 3]> {
    if rgb.iter().any(|&c| c > MAX_INTENSITY) {
        return Err(Error::Invalid(format!(
            "intensity {rgb:?} exceeds {MAX_INTENSITY}"
        )));
    }
    Ok(if rgb == [0, 0, 0] { [1, 1, 1] } else { rgb })
}

// ---------------------------------------------------------------------------------------------
// sysfs backend
// ---------------------------------------------------------------------------------------------

/// Backend driving `/sys/class/leds/*kbd_backlight*` (multicolor LED class) plus the ACPI
/// platform profile and the `fan_boost` attribute added by our driver patch.
pub struct SysfsBackend {
    led_dir: PathBuf,
    max_brightness: u8,
    multicolor: bool,
    platform_profile: Option<PathBuf>,
    fan_boost: Option<PathBuf>,
    hw_changed: Option<PathBuf>,
    last_hw_changed: Option<String>,
}

impl SysfsBackend {
    /// Discover the keyboard LED below `root` (normally `/`). Prefers a `uniwill` LED but falls
    /// back to any `*kbd_backlight*` LED so the tool degrades to brightness-only on other laptops.
    pub fn discover(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let leds = root.join("sys/class/leds");
        let entries = fs::read_dir(&leds).map_err(|e| Error::Io {
            path: leds.display().to_string(),
            source: e,
        })?;
        let mut candidates: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains("kbd_backlight"))
                    .unwrap_or(false)
            })
            .collect();
        candidates.sort_by_key(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            (!n.starts_with("uniwill"), n)
        });
        let led_dir = candidates
            .into_iter()
            .next()
            .ok_or_else(|| Error::LedNotFound(leds.display().to_string()))?;
        Self::open(led_dir, root)
    }

    /// Open a specific LED directory.
    pub fn open(led_dir: PathBuf, root: &Path) -> Result<Self> {
        let max_brightness = read_u32(&led_dir.join("max_brightness"))?.min(255) as u8;
        let multicolor = led_dir.join("multi_intensity").exists();
        let hw_changed = Some(led_dir.join("brightness_hw_changed")).filter(|p| p.exists());

        let pp = root.join("sys/firmware/acpi/platform_profile");
        let platform_profile = if pp.exists() && platform_profile_is_uniwill(&pp, &led_dir) {
            Some(pp)
        } else {
            None
        };

        let device = led_dir.join("device");
        let fan_boost = Some(device.join("fan_boost")).filter(|p| p.exists());

        Ok(Self {
            led_dir,
            max_brightness,
            multicolor,
            platform_profile,
            fan_boost,
            hw_changed,
            last_hw_changed: None,
        })
    }

    pub fn led_dir(&self) -> &Path {
        &self.led_dir
    }

    /// Path of `brightness_hw_changed`, for inotify/poll watchers in the daemon.
    pub fn hw_changed_path(&self) -> Option<&Path> {
        self.hw_changed.as_deref()
    }

    pub fn is_multicolor(&self) -> bool {
        self.multicolor
    }
}

/// Only take over `platform_profile` if the LED device belongs to the uniwill driver, otherwise
/// we might fight with another vendor driver (e.g. on a ThinkPad).
fn platform_profile_is_uniwill(pp: &Path, led_dir: &Path) -> bool {
    let choices =
        fs::read_to_string(pp.with_file_name("platform_profile_choices")).unwrap_or_default();
    if !choices.contains("balanced") || !choices.contains("performance") {
        return false;
    }
    let driver_ok = fs::read_link(led_dir.join("device/driver"))
        .map(|p| p.to_string_lossy().contains("uniwill"))
        .unwrap_or(false);
    let name_ok = led_dir
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("uniwill"))
        .unwrap_or(false);
    driver_ok || name_ok
}

fn read_string(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })
}

fn read_u32(path: &Path) -> Result<u32> {
    let s = read_string(path)?;
    s.parse().map_err(|_| Error::Parse {
        path: path.display().to_string(),
        value: s,
    })
}

fn write_string(path: &Path, value: &str) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })?;
    f.write_all(value.as_bytes()).map_err(|e| Error::Io {
        path: path.display().to_string(),
        source: e,
    })
}

impl Backend for SysfsBackend {
    fn brightness(&self) -> Result<u8> {
        Ok(read_u32(&self.led_dir.join("brightness"))?.min(255) as u8)
    }

    fn set_brightness(&mut self, level: u8) -> Result<()> {
        if level > self.max_brightness {
            return Err(Error::Invalid(format!(
                "brightness {level} exceeds max {}",
                self.max_brightness
            )));
        }
        write_string(&self.led_dir.join("brightness"), &level.to_string())
    }

    fn max_brightness(&self) -> u8 {
        self.max_brightness
    }

    fn intensity(&self) -> Result<[u8; 3]> {
        if !self.multicolor {
            return Ok([MAX_INTENSITY; 3]);
        }
        let path = self.led_dir.join("multi_intensity");
        let s = read_string(&path)?;
        let parts: Vec<u8> = s
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect();
        if parts.len() != 3 {
            return Err(Error::Parse {
                path: path.display().to_string(),
                value: s,
            });
        }
        Ok([parts[0], parts[1], parts[2]])
    }

    fn set_intensity(&mut self, rgb: [u8; 3]) -> Result<()> {
        if !self.multicolor {
            return Err(Error::Unsupported("colour (LED is not multicolor)".into()));
        }
        let rgb = clamp_intensity(rgb)?;
        write_string(
            &self.led_dir.join("multi_intensity"),
            &format!("{} {} {}", rgb[0], rgb[1], rgb[2]),
        )
    }

    fn take_hw_brightness_change(&mut self) -> Result<Option<u8>> {
        let Some(p) = &self.hw_changed else {
            return Ok(None);
        };
        let s = read_string(p)?;
        if self.last_hw_changed.as_deref() == Some(s.as_str()) {
            return Ok(None);
        }
        self.last_hw_changed = Some(s.clone());
        Ok(s.parse::<u8>().ok())
    }

    fn supports_fan_mode(&self) -> bool {
        self.platform_profile.is_some()
    }

    fn fan_mode(&self) -> Result<FanMode> {
        let p = self
            .platform_profile
            .as_ref()
            .ok_or_else(|| Error::Unsupported("fan mode".into()))?;
        let s = read_string(p)?;
        match s.as_str() {
            "performance" => Ok(FanMode::Performance),
            _ => Ok(FanMode::Balanced),
        }
    }

    fn set_fan_mode(&mut self, mode: FanMode) -> Result<()> {
        let p = self
            .platform_profile
            .as_ref()
            .ok_or_else(|| Error::Unsupported("fan mode".into()))?;
        write_string(p, mode.as_profile())
    }

    fn supports_fan_boost(&self) -> bool {
        self.fan_boost.is_some()
    }

    fn fan_boost(&self) -> Result<bool> {
        let p = self
            .fan_boost
            .as_ref()
            .ok_or_else(|| Error::Unsupported("fan boost".into()))?;
        Ok(read_u32(p)? != 0)
    }

    fn set_fan_boost(&mut self, on: bool) -> Result<()> {
        let p = self
            .fan_boost
            .as_ref()
            .ok_or_else(|| Error::Unsupported("fan boost".into()))?;
        write_string(p, if on { "1" } else { "0" })
    }

    fn describe(&self) -> String {
        format!(
            "sysfs {} (multicolor={}, fan_mode={}, fan_boost={})",
            self.led_dir.display(),
            self.multicolor,
            self.platform_profile.is_some(),
            self.fan_boost.is_some()
        )
    }
}

// ---------------------------------------------------------------------------------------------
// fake backend
// ---------------------------------------------------------------------------------------------

/// In-memory backend. Shared through an `Arc<Mutex<_>>` so tests can inspect writes.
#[derive(Debug, Clone, Default)]
pub struct FakeState {
    pub brightness: u8,
    pub intensity: [u8; 3],
    pub fan_mode: FanMode,
    pub fan_boost: bool,
    pub pending_hw_change: Option<u8>,
    pub writes: usize,
}

#[derive(Clone, Default)]
pub struct FakeBackend {
    pub state: Arc<Mutex<FakeState>>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                brightness: 3,
                intensity: [50, 50, 50],
                ..Default::default()
            })),
        }
    }

    pub fn snapshot(&self) -> FakeState {
        self.state.lock().unwrap().clone()
    }
}

impl Backend for FakeBackend {
    fn brightness(&self) -> Result<u8> {
        Ok(self.state.lock().unwrap().brightness)
    }
    fn set_brightness(&mut self, level: u8) -> Result<()> {
        if level > MAX_BRIGHTNESS {
            return Err(Error::Invalid(format!(
                "brightness {level} > {MAX_BRIGHTNESS}"
            )));
        }
        let mut s = self.state.lock().unwrap();
        s.brightness = level;
        s.writes += 1;
        Ok(())
    }
    fn intensity(&self) -> Result<[u8; 3]> {
        Ok(self.state.lock().unwrap().intensity)
    }
    fn set_intensity(&mut self, rgb: [u8; 3]) -> Result<()> {
        let rgb = clamp_intensity(rgb)?;
        let mut s = self.state.lock().unwrap();
        s.intensity = rgb;
        s.writes += 1;
        Ok(())
    }
    fn take_hw_brightness_change(&mut self) -> Result<Option<u8>> {
        Ok(self.state.lock().unwrap().pending_hw_change.take())
    }
    fn supports_fan_mode(&self) -> bool {
        true
    }
    fn fan_mode(&self) -> Result<FanMode> {
        Ok(self.state.lock().unwrap().fan_mode)
    }
    fn set_fan_mode(&mut self, mode: FanMode) -> Result<()> {
        self.state.lock().unwrap().fan_mode = mode;
        Ok(())
    }
    fn supports_fan_boost(&self) -> bool {
        true
    }
    fn fan_boost(&self) -> Result<bool> {
        Ok(self.state.lock().unwrap().fan_boost)
    }
    fn set_fan_boost(&mut self, on: bool) -> Result<()> {
        self.state.lock().unwrap().fan_boost = on;
        Ok(())
    }
    fn describe(&self) -> String {
        "fake (no hardware)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sysfs(dir: &Path) -> PathBuf {
        let led = dir.join("sys/class/leds/uniwill:multicolor:kbd_backlight");
        fs::create_dir_all(&led).unwrap();
        fs::create_dir_all(led.join("device")).unwrap();
        fs::write(led.join("brightness"), "2\n").unwrap();
        fs::write(led.join("max_brightness"), "4\n").unwrap();
        fs::write(led.join("multi_intensity"), "50 0 0\n").unwrap();
        fs::write(led.join("brightness_hw_changed"), "3\n").unwrap();
        fs::write(led.join("device/fan_boost"), "0\n").unwrap();
        let acpi = dir.join("sys/firmware/acpi");
        fs::create_dir_all(&acpi).unwrap();
        fs::write(acpi.join("platform_profile"), "balanced\n").unwrap();
        fs::write(
            acpi.join("platform_profile_choices"),
            "balanced performance\n",
        )
        .unwrap();
        led
    }

    #[test]
    fn sysfs_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let led = fake_sysfs(tmp.path());
        let mut b = SysfsBackend::discover(tmp.path()).unwrap();
        assert_eq!(b.led_dir(), led);
        assert_eq!(b.brightness().unwrap(), 2);
        assert_eq!(b.intensity().unwrap(), [50, 0, 0]);
        b.set_brightness(4).unwrap();
        b.set_intensity([0, 0, 0]).unwrap();
        assert_eq!(fs::read_to_string(led.join("brightness")).unwrap(), "4");
        assert_eq!(
            fs::read_to_string(led.join("multi_intensity")).unwrap(),
            "1 1 1"
        );
        assert!(b.set_brightness(5).is_err());
        assert!(b.set_intensity([51, 0, 0]).is_err());

        assert!(b.supports_fan_mode());
        b.set_fan_mode(FanMode::Performance).unwrap();
        assert_eq!(b.fan_mode().unwrap(), FanMode::Performance);
        assert!(b.supports_fan_boost());
        b.set_fan_boost(true).unwrap();
        assert!(b.fan_boost().unwrap());

        assert_eq!(b.take_hw_brightness_change().unwrap(), Some(3));
        assert_eq!(b.take_hw_brightness_change().unwrap(), None);
        fs::write(led.join("brightness_hw_changed"), "1\n").unwrap();
        assert_eq!(b.take_hw_brightness_change().unwrap(), Some(1));
    }

    #[test]
    fn discover_prefers_uniwill() {
        let tmp = tempfile::tempdir().unwrap();
        fake_sysfs(tmp.path());
        let other = tmp.path().join("sys/class/leds/asus::kbd_backlight");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("max_brightness"), "3").unwrap();
        let b = SysfsBackend::discover(tmp.path()).unwrap();
        assert!(b.led_dir().ends_with("uniwill:multicolor:kbd_backlight"));
    }

    #[test]
    fn missing_led() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("sys/class/leds")).unwrap();
        assert!(matches!(
            SysfsBackend::discover(tmp.path()),
            Err(Error::LedNotFound(_))
        ));
    }

    #[test]
    fn fan_mode_parse() {
        assert_eq!("beast".parse::<FanMode>().unwrap(), FanMode::Performance);
        assert_eq!("Gaming".parse::<FanMode>().unwrap(), FanMode::Balanced);
        assert!("hyper".parse::<FanMode>().is_err());
    }
}
