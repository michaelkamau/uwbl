//! Configuration (`/etc/uwbl/config.toml`) and per-power-source profiles.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::backend::FanMode;
use crate::color::Rgb;
use crate::effect::Effect;
use crate::{Error, Result};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/uwbl/config.toml";
pub const DEFAULT_STATE_PATH: &str = "/var/lib/uwbld/state.toml";

/// Which power source a profile applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerSource {
    Ac,
    Battery,
}

impl PowerSource {
    pub fn name(self) -> &'static str {
        match self {
            PowerSource::Ac => "ac",
            PowerSource::Battery => "battery",
        }
    }
}

impl std::fmt::Display for PowerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for PowerSource {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ac" | "mains" | "plugged" => Ok(PowerSource::Ac),
            "battery" | "bat" | "dc" => Ok(PowerSource::Battery),
            o => Err(Error::Invalid(format!("unknown power source {o:?}; expected ac|battery"))),
        }
    }
}

/// What the keyboard and fans should do while a given power source is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Master switch; when `false` the backlight is off regardless of `brightness`.
    pub enabled: bool,
    /// 1..=4 (0 is treated as "enabled = false" when read from the hardware).
    pub brightness: u8,
    pub effect: Effect,
    /// `None` leaves the firmware's current fan mode untouched.
    pub fan_mode: Option<FanMode>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            enabled: true,
            brightness: 3,
            effect: Effect::default(),
            fan_mode: None,
        }
    }
}

impl Profile {
    /// Windows Control Center defaults: AC = level 3, battery = off.
    pub fn default_for(source: PowerSource) -> Self {
        match source {
            PowerSource::Ac => Self::default(),
            PowerSource::Battery => Self {
                enabled: false,
                brightness: 2,
                ..Self::default()
            },
        }
    }

    /// Effective LED brightness taking the master switch into account.
    pub fn effective_brightness(&self) -> u8 {
        if self.enabled {
            self.brightness.clamp(1, crate::MAX_BRIGHTNESS)
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    pub ac: Profile,
    pub battery: Profile,
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            ac: Profile::default_for(PowerSource::Ac),
            battery: Profile::default_for(PowerSource::Battery),
        }
    }
}

impl Profiles {
    pub fn get(&self, s: PowerSource) -> &Profile {
        match s {
            PowerSource::Ac => &self.ac,
            PowerSource::Battery => &self.battery,
        }
    }
    pub fn get_mut(&mut self, s: PowerSource) -> &mut Profile {
        match s {
            PowerSource::Ac => &mut self.ac,
            PowerSource::Battery => &mut self.battery,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Turn the backlight off after this many seconds without keyboard/mouse input
    /// (0 disables). Restored on the next input event.
    pub idle_off_secs: u32,
    /// Re-apply the active profile after resume from suspend.
    pub restore_on_resume: bool,
    /// When the Fn brightness keys are used, store the new level in the active profile.
    pub follow_hw_brightness: bool,
    /// Use the in-memory fake backend instead of sysfs (development / unsupported hardware).
    pub fake_backend: bool,
    /// Override sysfs root (tests).
    pub sysfs_root: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            idle_off_secs: 0,
            restore_on_resume: true,
            follow_hw_brightness: true,
            fake_backend: false,
            sysfs_root: "/".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub profiles: Profiles,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Self::parse(&text)
    }

    /// Load `path`, returning defaults if it does not exist.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self> {
        match fs::read_to_string(path.as_ref()) {
            Ok(t) => Self::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::Io {
                path: path.as_ref().display().to_string(),
                source: e,
            }),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config is always serialisable")
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
        atomic_write(path, self.to_toml().as_bytes())
    }
}

/// Write via a temp file + rename so a crash never leaves a truncated file.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)
        .and_then(|_| fs::rename(&tmp, path))
        .map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })
}

/// Example configuration shipped to `/etc/uwbl/config.toml`.
pub fn example_config() -> String {
    let mut c = Config::default();
    c.profiles.ac.effect.color = Rgb::new(0, 0, 255);
    c.profiles.ac.fan_mode = Some(FanMode::Balanced);
    format!(
        "# uwbld configuration. Values here are the defaults; runtime changes made through\n\
         # `uwbl` or the tray are stored separately in {DEFAULT_STATE_PATH}.\n\
         # Colours accept \"#rrggbb\", \"r,g,b\" or preset names (red, orange, yellow, green,\n\
         # blue, cyan, violet, white, ...). Effects: static, breathing, cycle, rainbow.\n\
         # Fan modes: balanced (Gaming) or performance (Beast).\n\n{}",
        c.to_toml()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::EffectKind;

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.profiles.battery.effect.kind = EffectKind::Breathing;
        c.profiles.battery.fan_mode = Some(FanMode::Performance);
        let text = c.to_toml();
        let back = Config::parse(&text).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let c = Config::parse(
            r#"
            [general]
            idle_off_secs = 120
            [profiles.ac]
            brightness = 4
            [profiles.ac.effect]
            kind = "rainbow"
            color = "cyan"
            speed = 8
            "#,
        )
        .unwrap();
        assert_eq!(c.general.idle_off_secs, 120);
        assert!(c.general.restore_on_resume);
        assert_eq!(c.profiles.ac.brightness, 4);
        assert_eq!(c.profiles.ac.effect.kind, EffectKind::Rainbow);
        assert_eq!(c.profiles.ac.effect.color, Rgb::new(0, 255, 255));
        assert_eq!(c.profiles.ac.effect.colors.len(), 7);
        assert!(!c.profiles.battery.enabled);
    }

    #[test]
    fn example_parses() {
        Config::parse(&example_config()).unwrap();
    }

    #[test]
    fn effective_brightness() {
        let mut p = Profile::default();
        assert_eq!(p.effective_brightness(), 3);
        p.brightness = 0;
        assert_eq!(p.effective_brightness(), 1);
        p.enabled = false;
        assert_eq!(p.effective_brightness(), 0);
    }

    #[test]
    fn save_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub/config.toml");
        let c = Config::default();
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        assert_eq!(Config::load_or_default(tmp.path().join("nope.toml")).unwrap(), c);
    }
}
