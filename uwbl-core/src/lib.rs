//! Core library for controlling the Uniwill/Eluktronics single-zone RGB keyboard backlight
//! through the Linux LED class exposed by the `uniwill-laptop` kernel driver.
//!
//! The library is deliberately free of async runtimes and D-Bus so it can be unit-tested
//! against a fake sysfs tree. Layers:
//!
//! * [`backend`] – access to `/sys/class/leds/*kbd_backlight*` and the fan/platform-profile files
//! * [`color`] – sRGB ⇄ EC intensity (0..=50) conversion and named presets
//! * [`effect`] – software lighting effects (static, breathing, cycle, rainbow)
//! * [`config`] – TOML configuration and AC/battery profiles
//! * [`controller`] – synchronous glue applying profiles/effects to a backend
//! * [`state`] – the persisted runtime state shared by daemon, CLI and tray

pub mod backend;
pub mod color;
pub mod config;
pub mod controller;
pub mod effect;
pub mod state;

pub use backend::{Backend, FakeBackend, FanMode, SysfsBackend};
pub use color::Rgb;
pub use config::{Config, PowerSource, Profile, Profiles};
pub use controller::Controller;
pub use effect::{Effect, EffectEngine, EffectKind};
pub use state::{State, Status};

/// Maximum brightness level exposed by the kernel driver (0..=4).
pub const MAX_BRIGHTNESS: u8 = 4;
/// Maximum per-channel intensity accepted by the EC (0..=50).
pub const MAX_INTENSITY: u8 = 50;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("keyboard backlight LED not found under {0}")]
    LedNotFound(String),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unexpected value in {path}: {value:?}")]
    Parse { path: String, value: String },
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
