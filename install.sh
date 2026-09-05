#!/usr/bin/env bash
# Install uwbl: kernel module (DKMS), daemon, CLI, tray and system integration files.
# Run from the repository root:  sudo ./install.sh
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "run as root: sudo $0" >&2
    exit 1
fi

HERE=$(cd "$(dirname "$0")" && pwd)
PREFIX=${PREFIX:-/usr}
DKMS_NAME=uniwill-laptop-uwbl
DKMS_VER=$(sed -n 's/^PACKAGE_VERSION="\(.*\)"/\1/p' "$HERE/kernel/uniwill-laptop/dkms.conf")
SKIP_KERNEL=${SKIP_KERNEL:-0}
SKIP_BUILD=${SKIP_BUILD:-0}

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }

# --- 1. binaries ------------------------------------------------------------------------------
if [[ $SKIP_BUILD == 0 ]]; then
    log "building release binaries"
    # Build as the invoking user so ~/.cargo is used and target/ is not root-owned.
    BUILD_USER=${SUDO_USER:-root}
    # `sudo -u` gives a non-login shell without ~/.cargo/bin on PATH; add it explicitly.
    BUILD_CMD='export PATH="$HOME/.cargo/bin:$PATH"; command -v cargo >/dev/null || { echo "cargo not found for $(id -un); install Rust from https://rustup.rs" >&2; exit 1; }; cargo build --release --workspace'
    if [[ $BUILD_USER != root ]]; then
        sudo -u "$BUILD_USER" -H bash -c "cd '$HERE' && $BUILD_CMD"
    else
        (cd "$HERE" && bash -c "$BUILD_CMD")
    fi
fi
for bin in uwbld uwbl uwbl-tray; do
    [[ -x "$HERE/target/release/$bin" ]] || { echo "missing target/release/$bin (run cargo build --release)" >&2; exit 1; }
done

log "installing binaries to $PREFIX/bin"
install -Dm755 "$HERE/target/release/uwbld" "$PREFIX/bin/uwbld"
install -Dm755 "$HERE/target/release/uwbl" "$PREFIX/bin/uwbl"
install -Dm755 "$HERE/target/release/uwbl-tray" "$PREFIX/bin/uwbl-tray"

# --- 2. system integration --------------------------------------------------------------------
log "installing systemd / D-Bus / polkit / udev files"
install -Dm644 "$HERE/packaging/systemd/uwbld.service" /etc/systemd/system/uwbld.service
install -Dm644 "$HERE/packaging/dbus/org.uniwill.Backlight1.conf" /etc/dbus-1/system.d/org.uniwill.Backlight1.conf
install -Dm644 "$HERE/packaging/polkit/org.uniwill.backlight1.policy" "$PREFIX/share/polkit-1/actions/org.uniwill.backlight1.policy"
install -Dm644 "$HERE/packaging/udev/99-uwbl.rules" /etc/udev/rules.d/99-uwbl.rules
install -Dm644 "$HERE/packaging/modprobe/uniwill-laptop-uwbl.conf" /etc/modprobe.d/uniwill-laptop-uwbl.conf
install -Dm644 "$HERE/packaging/modules-load.conf" /etc/modules-load.d/uniwill-laptop-uwbl.conf
install -Dm644 "$HERE/packaging/autostart/uwbl-tray.desktop" /etc/xdg/autostart/uwbl-tray.desktop
install -Dm644 "$HERE/packaging/autostart/uwbl-tray.desktop" "$PREFIX/share/applications/uwbl-tray.desktop"
if [[ ! -e /etc/uwbl/config.toml ]]; then
    install -Dm644 "$HERE/packaging/config.toml" /etc/uwbl/config.toml
else
    log "keeping existing /etc/uwbl/config.toml"
fi

# --- 3. kernel module via DKMS ----------------------------------------------------------------
if [[ $SKIP_KERNEL == 0 ]]; then
    if ! command -v dkms >/dev/null; then
        echo "dkms not found: sudo apt install dkms linux-headers-\$(uname -r)" >&2
        exit 1
    fi
    log "installing kernel module $DKMS_NAME/$DKMS_VER via DKMS"
    SRC=/usr/src/$DKMS_NAME-$DKMS_VER
    rm -rf "$SRC"
    mkdir -p "$SRC"
    cp "$HERE"/kernel/uniwill-laptop/{uniwill-acpi.c,uniwill-wmi.c,uniwill-wmi.h,Kbuild,Makefile,dkms.conf} "$SRC/"
    if dkms status "$DKMS_NAME/$DKMS_VER" | grep -q .; then
        dkms remove "$DKMS_NAME/$DKMS_VER" --all || true
    fi
    dkms add "$DKMS_NAME/$DKMS_VER"
    dkms install "$DKMS_NAME/$DKMS_VER"

    log "loading module"
    if lsmod | grep -q '^uniwill_laptop'; then
        systemctl stop uwbld.service 2>/dev/null || true
        modprobe -r uniwill-laptop || true
    fi
    modprobe uniwill-laptop || {
        echo "modprobe failed; see 'dmesg | tail'. If it says the device is unsupported try:" >&2
        echo "  echo 'options uniwill-laptop force=1' | sudo tee /etc/modprobe.d/uniwill-laptop-uwbl.conf" >&2
    }
fi

# --- 4. activate --------------------------------------------------------------------------------
log "activating services"
udevadm control --reload
systemctl daemon-reload
systemctl reload dbus.service 2>/dev/null || systemctl kill -s HUP dbus.service || true
systemctl enable uwbld.service
if ls /sys/class/leds/*kbd_backlight* >/dev/null 2>&1; then
    systemctl restart uwbld.service
    sleep 1
    systemctl --no-pager --lines=5 status uwbld.service || true
    echo
    uwbl status || true
else
    echo "no kbd_backlight LED found yet; uwbld will start automatically once the module creates it." >&2
    echo "run 'sudo ./scripts/probe.sh' to diagnose." >&2
fi

log "done. Start the tray now with: uwbl-tray & (it autostarts on next login)"
