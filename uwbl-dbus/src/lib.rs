//! D-Bus contract shared by `uwbld` and `omarchy-eluktonics-keyboard`.
//!
//! Interface `org.uniwill.Backlight1` on the **system** bus:
//!
//! | Member | Signature | Notes |
//! |---|---|---|
//! | `GetStatus()` | `→ s` | JSON-encoded [`uwbl_core::Status`] |
//! | `SetEnabled(b)` | | master switch for the active profile |
//! | `SetBrightness(y)` | | `0..=4`; 0 disables |
//! | `SetColor(s)` | | `#rrggbb`, `r,g,b` or preset name; switches to `static` unless breathing |
//! | `SetEffect(s, y)` | | effect name, speed `1..=10` (0 = keep current) |
//! | `SetSpeed(y)` | | `1..=10` |
//! | `SetCycleColors(as)` | | colour list for the `cycle` effect |
//! | `SetFanMode(s)` | | `balanced` / `performance` |
//! | `SetFanBoost(b)` | | |
//! | `SetProfile(s, s)` | | power source (`ac`/`battery`), JSON [`uwbl_core::Profile`] |
//! | `ResetToConfig()` | | discard runtime state |
//! | `Reapply()` | | re-push everything to the EC |
//! | signal `StatusChanged(s)` | | JSON status after every change |
//!
//! Properties (read-only, `PropertiesChanged` emitted): `Enabled b`, `Brightness y`, `Color s`,
//! `Effect s`, `Speed y`, `FanMode s`, `FanBoost b`, `PowerSource s`, `Version s`.

use zbus::proxy;

pub const BUS_NAME: &str = "org.uniwill.Backlight1";
pub const OBJECT_PATH: &str = "/org/uniwill/Backlight1";
pub const INTERFACE: &str = "org.uniwill.Backlight1";
/// polkit action checked by the daemon for every mutating call.
pub const POLKIT_ACTION: &str = "org.uniwill.backlight1.control";

pub use uwbl_core::Status;

#[proxy(
    interface = "org.uniwill.Backlight1",
    default_service = "org.uniwill.Backlight1",
    default_path = "/org/uniwill/Backlight1"
)]
pub trait Backlight {
    fn get_status(&self) -> zbus::Result<String>;
    fn set_enabled(&self, on: bool) -> zbus::Result<()>;
    fn set_brightness(&self, level: u8) -> zbus::Result<()>;
    fn set_color(&self, color: &str) -> zbus::Result<()>;
    fn set_effect(&self, effect: &str, speed: u8) -> zbus::Result<()>;
    fn set_speed(&self, speed: u8) -> zbus::Result<()>;
    fn set_cycle_colors(&self, colors: Vec<String>) -> zbus::Result<()>;
    fn set_fan_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_fan_boost(&self, on: bool) -> zbus::Result<()>;
    fn set_profile(&self, source: &str, profile_json: &str) -> zbus::Result<()>;
    fn reset_to_config(&self) -> zbus::Result<()>;
    fn reapply(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn status_changed(&self, status_json: String) -> zbus::Result<()>;

    #[zbus(property)]
    fn enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn brightness(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn color(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn effect(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn speed(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn fan_mode(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn fan_boost(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn power_source(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
}

impl BacklightProxy<'_> {
    /// Convenience: fetch and decode the status.
    pub async fn status(&self) -> zbus::Result<Status> {
        let json = self.get_status().await?;
        serde_json::from_str(&json)
            .map_err(|e| zbus::Error::Failure(format!("bad status JSON: {e}")))
    }
}

/// Connect to the daemon on the system bus (or the session bus when `UWBL_SESSION_BUS` is set,
/// which is how the test/dev setup runs without root).
pub async fn connect() -> zbus::Result<BacklightProxy<'static>> {
    let conn = if std::env::var_os("UWBL_SESSION_BUS").is_some() {
        zbus::Connection::session().await?
    } else {
        zbus::Connection::system().await?
    };
    BacklightProxy::new(&conn).await
}
