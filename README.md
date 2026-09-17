# omarchy-eluktonics-keyboard

An Omarchy plugin and command-line app for the Eluktronics RP-17 single-zone
RGB keyboard. It provides quick controls for power, four brightness levels,
colour presets, and static, breathing, cycle, and rainbow effects.

The hardware backend is adapted from
[uwbl](https://github.com/michaelkamau/uwbl), including its RP-17
`uniwill-laptop` DKMS driver and system daemon.

```
┌────────────┐  D-Bus (system)  ┌──────────┐  sysfs LED / platform_profile  ┌──────────────────┐
│ Omarchy bar plugin / CLI      │──▶│  uwbld   │───────────────────────────▶│ uniwill-laptop.ko│─▶ EC
│ omarchy-eluktonics-keyboard   │◀──│ (daemon) │                            │  (patched, DKMS) │
└────────────┘  StatusChanged   └──────────┘                                └──────────────────┘
```

| Component | What it is |
|---|---|
| `kernel/uniwill-laptop/` | Mainline `uniwill-laptop` driver + RP-17 DMI entry, `platform_profile` fan mode and `fan_boost` attribute. Installed with DKMS, shadows the (older) in-tree module. |
| `uwbl-core` | Library: sysfs backend, colour model, software effects (breathing / cycle / rainbow), TOML config, AC/battery profiles, controller. Fully unit-tested against a fake backend. |
| `uwbld` | Root daemon: owns the LED, applies the profile for the current power source, runs effects at ≤15 fps, re-applies after resume, follows the Fn brightness keys, optional idle-off, persists changes. Exposes `org.uniwill.Backlight1` on the system bus (polkit-protected). |
| `omarchy-eluktonics-keyboard` | CLI client used directly and by the shell plugin. |
| `Panel.qml` / `manifest.json` | Native Omarchy bar widget and popup controls. |
| `docs/` | Reverse-engineering notes from the Windows package, hardware probe template. |
| `reference/` | The original Windows installer (not needed at runtime). |

## Install

This plugin targets Omarchy on the Eluktronics RP-17. Install the build
dependencies first:

```bash
omarchy pkg add base-devel dkms linux-headers rust
omarchy plugin add https://github.com/michaelkamau/omarchy-eluktonics-keyboard --enable
cd ~/.config/omarchy/plugins/michaelkamau.eluktronics-keyboard
sudo ./install.sh
```

`install.sh` honours `SKIP_KERNEL=1` (don't touch the kernel module) and
`SKIP_BUILD=1` (use existing `target/release` binaries). `sudo ./uninstall.sh [--purge]`
removes everything.

For development from this checkout, validate the shell plugin with:

```bash
omarchy plugin validate .
```

### First run on new hardware

The keyboard and EC resume-recovery path have been exercised on a live RP-17;
the remaining hardware checklist is in
[docs/hardware-probe.md](docs/hardware-probe.md). After installing on new hardware, run

```sh
sudo ./scripts/probe.sh --test
```

It dumps DMI/ACPI/LED/hwmon information to `docs/hardware-probe-<date>.txt` and,
with `--test`, steps the keyboard through brightness 1–4 and red/green/blue/white/orange.
If the module refuses to load with "unsupported device", uncomment
`options uniwill-laptop force=1` in `/etc/modprobe.d/uniwill-laptop-uwbl.conf`.

## Usage

```sh
omarchy-eluktonics-keyboard                         # status
omarchy-eluktonics-keyboard color blue              # preset, #rrggbb, or r,g,b
omarchy-eluktonics-keyboard brightness 4            # 0–4, up, or down
omarchy-eluktonics-keyboard effect breathing -s 7   # effect and speed 1–10
omarchy-eluktonics-keyboard cycle red white blue
omarchy-eluktonics-keyboard fan performance
omarchy-eluktonics-keyboard boost on
omarchy-eluktonics-keyboard profile show
omarchy-eluktonics-keyboard reset
omarchy-eluktonics-keyboard watch --json
```

Changes made through the CLI/plugin apply to the profile of the **current power
source** and are persisted in `/var/lib/uwbld/state.toml`. Defaults live in
`/etc/uwbl/config.toml` (`uwbld --print-config` prints a commented example):

```toml
[general]
idle_off_secs = 0          # turn off after N s without keyboard/mouse input (0 = never)
restore_on_resume = true
follow_hw_brightness = true # Fn keys update the profile

[profiles.ac]
enabled = true
brightness = 3
fan_mode = "balanced"
[profiles.ac.effect]
kind = "static"
color = "#0000ff"
speed = 5

[profiles.battery]
enabled = false            # Windows default: DCLight = 0
```

The Fn brightness keys keep working through the kernel driver; the daemon notices
(`brightness_hw_changed`) and updates the profile so the level survives a reboot.
The Fn brightness keys continue to work because the LED is exposed as a
standard `kbd_backlight` device.

### Backlight unresponsive after sleep

Check `systemctl status uwbld` and `journalctl -b -k -g uniwill`. On the RP-17,
the firmware can lose its EC memory window during suspend: the daemon stays
running, but writes have no effect and hwmon reports impossible temperatures
and fan speeds. Kernel module version 0.1.1 adds a model-specific WMI fallback
before restoring hardware state. Rebuild/install the kernel module, not just
the daemon; see [kernel recovery notes](kernel/uniwill-laptop/README.md#rp-17-resume-recovery).

Separately, the default battery profile is **off**. To keep lighting enabled
when unplugged, run `omarchy-eluktonics-keyboard profile set battery '{"enabled":true}'`.
This preference is saved and used on subsequent resumes.

### D-Bus API

`org.uniwill.Backlight1` at `/org/uniwill/Backlight1` on the system bus – see
`uwbl-dbus/src/lib.rs` for the full table. Mutating calls need the polkit action
`org.uniwill.backlight1.control` (allowed for active local sessions without a
password).

```sh
busctl call org.uniwill.Backlight1 /org/uniwill/Backlight1 org.uniwill.Backlight1 SetColor s red
busctl get-property org.uniwill.Backlight1 /org/uniwill/Backlight1 org.uniwill.Backlight1 Brightness
```

## Development

```sh
cargo test --workspace && cargo clippy --workspace --all-targets
# run everything unprivileged with a fake backend on the session bus:
cargo run -p uwbld -- --fake --session-bus --config /tmp/c.toml --state /tmp/s.toml &
UWBL_SESSION_BUS=1 cargo run -p omarchy-eluktonics-keyboard -- effect rainbow
# kernel module only:
make -C kernel/uniwill-laptop
```

## Hardware notes (short version)

* RP-17 = Tongfang GK7NR0R (≈ TUXEDO Polaris 17 Gen1 AMD / XMG Apex 17 2020),
  Uniwill EC reached via ACPI `INOU0000` (`ECRR`/`ECRW`), WMI GUID `ABBC0F72…` for events.
* Keyboard backlight: EC `0x0769/6A/6B` R/G/B 0–50, `0x078C` bits 7:5 brightness 0–4,
  `0x0767` bit 5 apply. Windows FourZone/per-key paths are USB-HID and do not exist here.
* Fan: `0x0751` bit 4 TURBO (Beast), `0x0767` bit 2 fan-boost trigger, `0x0768` bit 2 status.
* Full register map and the Windows constants: [docs/windows-reverse-engineering.md](docs/windows-reverse-engineering.md),
  [docs/gcuservice_constants.txt](docs/gcuservice_constants.txt).

## License

GPL-2.0-only (the kernel driver is derived from GPL-2.0-or-later mainline code).
