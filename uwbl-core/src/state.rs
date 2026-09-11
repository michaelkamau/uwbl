//! Persisted runtime state and the status snapshot shared over D-Bus.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::backend::FanMode;
use crate::color::Rgb;
use crate::config::{atomic_write, PowerSource, Profiles};
use crate::effect::EffectKind;
use crate::{Error, Result};

/// Runtime-modified profiles, persisted to `/var/lib/uwbld/state.toml` so choices made through
/// the CLI/tray survive reboots without editing the config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct State {
    pub profiles: Profiles,
    pub fan_boost: bool,
}

impl State {
    pub fn load(path: impl AsRef<Path>) -> Result<Option<Self>> {
        match std::fs::read_to_string(path.as_ref()) {
            Ok(t) => toml::from_str(&t)
                .map(Some)
                .map_err(|e| Error::Config(format!("state file: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io {
                path: path.as_ref().display().to_string(),
                source: e,
            }),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
        let text = toml::to_string_pretty(self).expect("state is serialisable");
        atomic_write(path, text.as_bytes())
    }
}

/// Point-in-time view returned by the CLI and D-Bus API.
/// Serialised as JSON for `--json` and the D-Bus `GetStatus` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub backend: String,
    pub power_source: PowerSource,
    pub enabled: bool,
    pub brightness: u8,
    pub max_brightness: u8,
    pub color: Rgb,
    pub effect: EffectKind,
    pub speed: u8,
    pub idle_off: bool,
    pub fan_mode: Option<FanMode>,
    pub fan_boost: Option<bool>,
    pub profiles: Profiles,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("state.toml");
        assert_eq!(State::load(&p).unwrap(), None);
        let mut s = State::default();
        s.profiles.ac.brightness = 1;
        s.fan_boost = true;
        s.save(&p).unwrap();
        assert_eq!(State::load(&p).unwrap(), Some(s));
    }
}
