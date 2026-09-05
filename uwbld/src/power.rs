//! AC / battery detection via `/sys/class/power_supply`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, warn};
use uwbl_core::PowerSource;

use crate::Shared;

/// `Ac` if any `Mains`-type supply reports `online = 1`; `Battery` if there is a mains supply
/// and none is online; `None` when no mains supply exists (desktop / unknown → caller assumes AC).
pub fn detect(sysfs_root: impl AsRef<Path>) -> Option<PowerSource> {
    let dir = sysfs_root.as_ref().join("sys/class/power_supply");
    let entries = std::fs::read_dir(dir).ok()?;
    let mut saw_mains = false;
    for entry in entries.flatten() {
        let p = entry.path();
        let ty = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if ty.trim() != "Mains" {
            continue;
        }
        saw_mains = true;
        let online = std::fs::read_to_string(p.join("online")).unwrap_or_default();
        if online.trim() == "1" {
            return Some(PowerSource::Ac);
        }
    }
    saw_mains.then_some(PowerSource::Battery)
}

pub async fn watch(shared: Arc<Shared>, sysfs_root: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let Some(power) = detect(&sysfs_root) else {
            continue;
        };
        let changed = {
            let mut ctl = shared.ctl.lock().unwrap();
            if ctl.power_source() == power {
                false
            } else {
                debug!("power source now {power}");
                if let Err(e) = ctl.set_power_source(power) {
                    warn!("switching profile failed: {e}");
                }
                true
            }
        };
        if changed {
            shared.notify_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(root: &Path, name: &str, ty: &str, online: &str) {
        let d = root.join("sys/class/power_supply").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("type"), ty).unwrap();
        std::fs::write(d.join("online"), online).unwrap();
    }

    #[test]
    fn detect_states() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(detect(tmp.path()), None);
        fake(tmp.path(), "BAT0", "Battery\n", "0\n");
        assert_eq!(detect(tmp.path()), None);
        fake(tmp.path(), "ACAD", "Mains\n", "0\n");
        assert_eq!(detect(tmp.path()), Some(PowerSource::Battery));
        fake(tmp.path(), "ACAD", "Mains\n", "1\n");
        assert_eq!(detect(tmp.path()), Some(PowerSource::Ac));
    }
}
