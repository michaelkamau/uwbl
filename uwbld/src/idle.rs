//! Idle-off: turn the backlight off after N seconds without keyboard/mouse input.
//!
//! As root we can read `/dev/input/event*` directly; any event on any device counts as activity.
//! Devices are re-scanned periodically so hot-plugged keyboards are picked up.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, warn};
use tokio::io::AsyncReadExt;

use crate::Shared;

const INPUT_EVENT_SIZE: usize = 24; // struct input_event on 64-bit

pub async fn run(shared: Arc<Shared>, timeout: Duration) {
    let watched: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    let mut rescan = tokio::time::interval(Duration::from_secs(10));
    let mut check = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = rescan.tick() => spawn_readers(&shared, &watched),
            _ = check.tick() => {
                let idle_ms = shared.now_ms().saturating_sub(shared.last_activity.load(Ordering::Relaxed));
                if idle_ms >= timeout.as_millis() as u64 {
                    set_idle(&shared, true);
                }
            }
            _ = shared.activity.notified() => set_idle(&shared, false),
        }
    }
}

fn set_idle(shared: &Shared, off: bool) {
    let changed = {
        let mut ctl = shared.ctl.lock().unwrap();
        if ctl.is_idle_off() == off {
            false
        } else {
            if let Err(e) = ctl.set_idle_off(off) {
                warn!("idle toggle failed: {e}");
            }
            true
        }
    };
    if changed {
        shared.notify_changed();
    }
}

fn spawn_readers(shared: &Arc<Shared>, watched: &Arc<Mutex<HashSet<PathBuf>>>) {
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return };
    for entry in dir.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"));
        if !is_event || !watched.lock().unwrap().insert(path.clone()) {
            continue;
        }
        tokio::spawn(read_device(shared.clone(), path, watched.clone()));
    }
}

async fn read_device(shared: Arc<Shared>, path: PathBuf, watched: Arc<Mutex<HashSet<PathBuf>>>) {
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            debug!("cannot open {}: {e}", path.display());
            watched.lock().unwrap().remove(&path);
            return;
        }
    };
    debug!("watching {} for activity", path.display());
    let mut buf = [0u8; INPUT_EVENT_SIZE * 8];
    loop {
        match file.read(&mut buf).await {
            Ok(0) | Err(_) => {
                debug!("{} gone", path.display());
                watched.lock().unwrap().remove(&path);
                return;
            }
            Ok(_) => {
                shared.last_activity.store(shared.now_ms(), Ordering::Relaxed);
                shared.activity.notify_one();
            }
        }
    }
}
