//! D-Bus server side: `org.uniwill.Backlight1` interface, polkit checks, logind resume hook.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use log::{debug, info, warn};
use uwbl_core::{EffectKind, FanMode, PowerSource, Profile, Rgb, Status};
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{interface, proxy, Connection};

use crate::Shared;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Backlight {
    shared: Arc<Shared>,
    conn: Connection,
}

fn failed(e: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

fn invalid(e: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::InvalidArgs(e.to_string())
}

impl Backlight {
    fn status(&self) -> Status {
        self.shared.ctl.lock().unwrap().status()
    }

    /// Run a mutation under the controller lock, then wake the effect loop / emitter.
    fn mutate(
        &self,
        f: impl FnOnce(&mut uwbl_core::Controller) -> uwbl_core::Result<()>,
    ) -> fdo::Result<()> {
        {
            let mut ctl = self.shared.ctl.lock().unwrap();
            f(&mut ctl).map_err(failed)?;
        }
        self.shared.notify_changed();
        Ok(())
    }

    async fn authorize(&self, hdr: &Header<'_>) -> fdo::Result<()> {
        if !self.shared.polkit {
            return Ok(());
        }
        let sender = hdr
            .sender()
            .ok_or_else(|| fdo::Error::AccessDenied("no sender".into()))?
            .to_string();
        match polkit::check(&self.conn, &sender).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(fdo::Error::AccessDenied(format!(
                "not authorised for {}",
                uwbl_dbus::POLKIT_ACTION
            ))),
            Err(e) => {
                warn!("polkit check failed: {e}");
                Err(fdo::Error::AccessDenied(format!("polkit unavailable: {e}")))
            }
        }
    }
}

#[interface(name = "org.uniwill.Backlight1")]
impl Backlight {
    fn get_status(&self) -> fdo::Result<String> {
        serde_json::to_string(&self.status()).map_err(failed)
    }

    async fn set_enabled(&self, #[zbus(header)] hdr: Header<'_>, on: bool) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        self.mutate(|c| c.set_enabled(on))
    }

    async fn set_brightness(&self, #[zbus(header)] hdr: Header<'_>, level: u8) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        self.mutate(|c| c.set_brightness(level))
    }

    async fn set_color(&self, #[zbus(header)] hdr: Header<'_>, color: &str) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        let rgb: Rgb = color.parse().map_err(invalid)?;
        self.mutate(|c| c.set_color(rgb))
    }

    async fn set_effect(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        effect: &str,
        speed: u8,
    ) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        let kind: EffectKind = effect.parse().map_err(invalid)?;
        let speed = (speed != 0).then_some(speed);
        self.mutate(|c| c.set_effect(kind, speed))
    }

    async fn set_speed(&self, #[zbus(header)] hdr: Header<'_>, speed: u8) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        if !(1..=10).contains(&speed) {
            return Err(invalid("speed must be 1..=10"));
        }
        self.mutate(|c| c.set_speed(speed))
    }

    async fn set_cycle_colors(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        colors: Vec<String>,
    ) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        let parsed: Result<Vec<Rgb>, _> = colors.iter().map(|c| c.parse()).collect();
        let parsed = parsed.map_err(invalid)?;
        self.mutate(|c| c.set_cycle_colors(parsed))
    }

    async fn set_fan_mode(&self, #[zbus(header)] hdr: Header<'_>, mode: &str) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        let mode: FanMode = mode.parse().map_err(invalid)?;
        self.mutate(|c| c.set_fan_mode(mode))
    }

    async fn set_fan_boost(&self, #[zbus(header)] hdr: Header<'_>, on: bool) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        self.mutate(|c| c.set_fan_boost(on))
    }

    async fn set_profile(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        source: &str,
        profile_json: &str,
    ) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        let source: PowerSource = source.parse().map_err(invalid)?;
        let profile: Profile = serde_json::from_str(profile_json).map_err(invalid)?;
        self.mutate(|c| c.set_profile(source, profile))
    }

    async fn reset_to_config(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        self.mutate(|c| c.reset_to_config())
    }

    async fn reapply(&self, #[zbus(header)] hdr: Header<'_>) -> fdo::Result<()> {
        self.authorize(&hdr).await?;
        self.mutate(|c| c.apply_all())
    }

    #[zbus(signal)]
    pub async fn status_changed(emitter: &SignalEmitter<'_>, status_json: &str)
        -> zbus::Result<()>;

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.status().enabled
    }
    #[zbus(property)]
    fn brightness(&self) -> u8 {
        self.status().brightness
    }
    #[zbus(property)]
    fn color(&self) -> String {
        self.status().color.to_hex()
    }
    #[zbus(property)]
    fn effect(&self) -> String {
        self.status().effect.name().to_string()
    }
    #[zbus(property)]
    fn speed(&self) -> u8 {
        self.status().speed
    }
    #[zbus(property)]
    fn fan_mode(&self) -> String {
        self.status()
            .fan_mode
            .map(|m| m.to_string())
            .unwrap_or_default()
    }
    #[zbus(property)]
    fn fan_boost(&self) -> bool {
        self.status().fan_boost.unwrap_or(false)
    }
    #[zbus(property)]
    fn power_source(&self) -> String {
        self.status().power_source.to_string()
    }
    #[zbus(property)]
    fn version(&self) -> String {
        VERSION.to_string()
    }
}

pub async fn serve(shared: Arc<Shared>, session_bus: bool) -> Result<Connection> {
    let builder = if session_bus {
        zbus::connection::Builder::session()?
    } else {
        zbus::connection::Builder::system()?
    };
    // The interface needs the connection for polkit; build the connection first, then attach.
    let conn = builder.build().await.context("connecting to D-Bus")?;
    conn.object_server()
        .at(
            uwbl_dbus::OBJECT_PATH,
            Backlight {
                shared,
                conn: conn.clone(),
            },
        )
        .await?;
    conn.request_name(uwbl_dbus::BUS_NAME)
        .await
        .with_context(|| {
            format!(
                "requesting bus name {} (check the D-Bus policy file)",
                uwbl_dbus::BUS_NAME
            )
        })?;
    Ok(conn)
}

/// After every change: emit `StatusChanged` + `PropertiesChanged`, and persist state (debounced).
pub async fn emit_changes(shared: Arc<Shared>, conn: Connection, state_path: PathBuf) {
    let iface = match conn
        .object_server()
        .interface::<_, Backlight>(uwbl_dbus::OBJECT_PATH)
        .await
    {
        Ok(i) => i,
        Err(e) => {
            warn!("cannot get interface ref: {e}");
            return;
        }
    };
    loop {
        shared.changed.notified().await;
        // Coalesce bursts (animations do not go through here, only user/hw changes).
        tokio::time::sleep(Duration::from_millis(50)).await;

        let status = shared.ctl.lock().unwrap().status();
        let json = serde_json::to_string(&status).unwrap_or_default();
        let emitter = iface.signal_emitter();
        if let Err(e) = Backlight::status_changed(emitter, &json).await {
            debug!("StatusChanged emit failed: {e}");
        }
        let b = iface.get().await;
        let _ = tokio::join!(
            b.enabled_changed(emitter),
            b.brightness_changed(emitter),
            b.color_changed(emitter),
            b.effect_changed(emitter),
            b.speed_changed(emitter),
            b.fan_mode_changed(emitter),
            b.fan_boost_changed(emitter),
            b.power_source_changed(emitter),
        );
        drop(b);

        crate::save_state(&shared, &state_path);
    }
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// Re-apply the profile after resume; the EC may have been reset or the OS may have changed the
/// LED brightness during suspend.
pub async fn watch_sleep(shared: Arc<Shared>, conn: Connection) {
    // logind lives on the system bus even when we are serving on the session bus for testing.
    let sys = if conn.is_bus() && std::env::var_os("DBUS_SYSTEM_BUS_ADDRESS").is_none() {
        match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                warn!("no system bus for logind: {e}");
                return;
            }
        }
    } else {
        conn
    };
    let proxy = match LoginManagerProxy::new(&sys).await {
        Ok(p) => p,
        Err(e) => {
            warn!("logind proxy: {e}");
            return;
        }
    };
    let mut stream = match proxy.receive_prepare_for_sleep().await {
        Ok(s) => s,
        Err(e) => {
            warn!("cannot subscribe to PrepareForSleep: {e}");
            return;
        }
    };
    while let Some(sig) = stream.next().await {
        let Ok(args) = sig.args() else { continue };
        if args.start {
            debug!("system going to sleep");
            continue;
        }
        info!("resumed from sleep; re-applying profile");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        {
            let mut ctl = shared.ctl.lock().unwrap();
            if let Err(e) = ctl.resume() {
                warn!("re-apply after resume failed: {e}");
            }
        }
        shared.notify_changed();
    }
}

mod polkit {
    use super::*;

    #[proxy(
        interface = "org.freedesktop.PolicyKit1.Authority",
        default_service = "org.freedesktop.PolicyKit1",
        default_path = "/org/freedesktop/PolicyKit1/Authority"
    )]
    trait Authority {
        #[allow(clippy::type_complexity)]
        fn check_authorization(
            &self,
            subject: &(&str, HashMap<&str, Value<'_>>),
            action_id: &str,
            details: HashMap<&str, &str>,
            flags: u32,
            cancellation_id: &str,
        ) -> zbus::Result<(bool, bool, HashMap<String, OwnedValue>)>;
    }

    const ALLOW_USER_INTERACTION: u32 = 1;

    pub async fn check(conn: &Connection, sender: &str) -> Result<bool> {
        let proxy = AuthorityProxy::new(conn).await?;
        let mut subject = HashMap::new();
        subject.insert("name", Value::from(sender));
        let (authorized, _challenge, _details) = proxy
            .check_authorization(
                &("system-bus-name", subject),
                uwbl_dbus::POLKIT_ACTION,
                HashMap::new(),
                ALLOW_USER_INTERACTION,
                "",
            )
            .await?;
        Ok(authorized)
    }
}
