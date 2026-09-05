//! `uwbld` – system daemon owning the keyboard backlight and fan mode.
//!
//! Runs as root, exposes `org.uniwill.Backlight1` on the system bus and drives the sysfs LED
//! class device created by the `uniwill-laptop` kernel module.

mod dbus;
mod idle;
mod power;

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use log::{debug, error, info, warn};
use tokio::sync::Notify;
use uwbl_core::{Backend, Config, Controller, FakeBackend, PowerSource, State, SysfsBackend};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Configuration file.
    #[arg(long, default_value = uwbl_core::config::DEFAULT_CONFIG_PATH)]
    config: PathBuf,
    /// Runtime state file (persisted profile changes).
    #[arg(long, default_value = uwbl_core::config::DEFAULT_STATE_PATH)]
    state: PathBuf,
    /// Use an in-memory fake backend (development without the kernel module).
    #[arg(long)]
    fake: bool,
    /// Register on the session bus instead of the system bus (development).
    #[arg(long)]
    session_bus: bool,
    /// Skip polkit authorisation checks (development / session bus).
    #[arg(long)]
    no_polkit: bool,
    /// Print the example configuration and exit.
    #[arg(long)]
    print_config: bool,
}

/// Everything the D-Bus interface and the background tasks share.
pub struct Shared {
    pub ctl: Mutex<Controller>,
    /// Woken after any change so the effect loop re-evaluates its schedule.
    pub wake: Notify,
    /// Woken after any change so the D-Bus task emits `StatusChanged` and saves state.
    pub changed: Notify,
    /// Milliseconds since daemon start of the last input event (idle detection).
    pub last_activity: AtomicU64,
    pub activity: Notify,
    pub started: Instant,
    pub polkit: bool,
}

impl Shared {
    pub fn notify_changed(&self) {
        self.wake.notify_one();
        self.changed.notify_one();
    }

    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if args.print_config {
        print!("{}", uwbl_core::config::example_config());
        return Ok(());
    }

    let config = Config::load_or_default(&args.config)
        .with_context(|| format!("loading {}", args.config.display()))?;
    let state = match State::load(&args.state) {
        Ok(s) => s,
        Err(e) => {
            warn!("ignoring unreadable state file {}: {e}", args.state.display());
            None
        }
    };

    let backend: Box<dyn Backend> = if args.fake || config.general.fake_backend {
        warn!("using fake backend – nothing will be written to the hardware");
        Box::new(FakeBackend::new())
    } else {
        Box::new(
            SysfsBackend::discover(&config.general.sysfs_root)
                .context("no keyboard backlight LED found; is the uniwill-laptop module loaded?")?,
        )
    };
    info!("backend: {}", backend.describe());

    let power = power::detect(&config.general.sysfs_root).unwrap_or(PowerSource::Ac);
    let idle_secs = config.general.idle_off_secs;
    let sysfs_root = config.general.sysfs_root.clone();
    let mut ctl = Controller::new(backend, config, state, power);
    if let Err(e) = ctl.apply_all() {
        error!("initial apply failed: {e}");
    }

    let shared = Arc::new(Shared {
        ctl: Mutex::new(ctl),
        wake: Notify::new(),
        changed: Notify::new(),
        last_activity: AtomicU64::new(0),
        activity: Notify::new(),
        started: Instant::now(),
        polkit: !args.no_polkit && !args.session_bus,
    });

    let conn = dbus::serve(shared.clone(), args.session_bus).await?;
    info!(
        "listening on {} as {}",
        if args.session_bus { "session bus" } else { "system bus" },
        uwbl_dbus::BUS_NAME
    );

    tokio::spawn(effect_loop(shared.clone()));
    tokio::spawn(hw_brightness_poll(shared.clone()));
    tokio::spawn(power::watch(shared.clone(), sysfs_root));
    tokio::spawn(dbus::watch_sleep(shared.clone(), conn.clone()));
    tokio::spawn(dbus::emit_changes(shared.clone(), conn.clone(), args.state.clone()));
    if idle_secs > 0 {
        tokio::spawn(idle::run(shared.clone(), Duration::from_secs(idle_secs as u64)));
    }

    wait_for_shutdown().await;
    info!("shutting down");
    save_state(&shared, &args.state);
    Ok(())
}

async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

pub fn save_state(shared: &Shared, path: &std::path::Path) {
    let state = {
        let mut ctl = shared.ctl.lock().unwrap();
        if !ctl.is_dirty() {
            return;
        }
        ctl.take_state()
    };
    match state.save(path) {
        Ok(()) => debug!("state saved to {}", path.display()),
        Err(e) => warn!("cannot save state to {}: {e}", path.display()),
    }
}

/// Drives animated effects; sleeps indefinitely while the effect is static.
async fn effect_loop(shared: Arc<Shared>) {
    loop {
        let next = {
            let mut ctl = shared.ctl.lock().unwrap();
            match ctl.tick(Instant::now()) {
                Ok(n) => n,
                Err(e) => {
                    warn!("effect tick failed: {e}");
                    Some(Duration::from_secs(1))
                }
            }
        };
        match next {
            Some(d) => tokio::select! {
                _ = tokio::time::sleep(d) => {}
                _ = shared.wake.notified() => {}
            },
            None => shared.wake.notified().await,
        }
    }
}

/// Fn-key brightness changes arrive through `brightness_hw_changed`; poll it cheaply.
async fn hw_brightness_poll(shared: Arc<Shared>) {
    let mut interval = tokio::time::interval(Duration::from_millis(400));
    loop {
        interval.tick().await;
        let changed = {
            let mut ctl = shared.ctl.lock().unwrap();
            ctl.poll_hw_brightness().unwrap_or_else(|e| {
                debug!("hw brightness poll: {e}");
                false
            })
        };
        if changed {
            shared.notify_changed();
        }
    }
}
