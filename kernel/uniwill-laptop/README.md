# uniwill-laptop (out-of-tree build with RP-17 support)

This directory is a copy of the mainline Linux driver
`drivers/platform/x86/uniwill/` (Armin Wolf, GPL-2.0-or-later) with the following
local additions, clearly marked `Out-of-tree addition` in the source:

| Change | Why |
|---|---|
| DMI entry `Eluktronics Inc.` / `RP-17` → `eluktronics_rp17_descriptor` | Ubuntu's 7.0 module has no keyboard-backlight support at all, and mainline only matches TUXEDO boards. The RP-17 (Tongfang GK7NR0R) gets a single-zone RGB keyboard backlight with 5 brightness levels, temps, fans, fn-lock, super-key, charge modes. |
| `platform_profile` (`balanced` / `performance`) | Fan/performance mode via EC `0x0751` turbo bit — the Control Center's Gaming / Beast modes. |
| `fan_boost` sysfs attribute on `/sys/bus/platform/devices/INOU0000:00/` | The Control Center's Fan Boost button (trigger `0x0767.2`, status `0x0768.2`). |
| Kbuild compat shims (`UW_HAVE_MC_MAX_INTENSITY`, `UW_HAVE_WMI_MIN_EVENT_SIZE`) | Build against kernels that predate those core changes (e.g. Ubuntu 7.0). |

Because the module keeps the name `uniwill-laptop` and DKMS installs to
`/lib/modules/<ver>/updates/dkms/`, it shadows the in-tree copy automatically.

## Build / install

```sh
# build only (needs make, gcc matching the kernel, linux-headers)
make
# install through DKMS (recommended; rebuilt on kernel upgrades)
sudo dkms add ./kernel/uniwill-laptop        # from the repo root; registers uniwill-laptop-uwbl/0.1.0
sudo dkms install uniwill-laptop-uwbl/0.1.0
sudo modprobe uniwill-laptop
```

## What appears after loading

- `/sys/class/leds/uniwill:multicolor:kbd_backlight/` (`brightness` 0–4, `multi_intensity` "R G B" 0–50,
  `brightness_hw_changed`) — consumed by UPower/KDE and by `uwbld`
- `/sys/class/hwmon/hwmonN` (`name` = `uniwill`): CPU/GPU temps, fan RPM, fan PWM (read-only)
- `/sys/firmware/acpi/platform_profile` (`balanced`, `performance`)
- `/sys/bus/platform/devices/INOU0000:00/{fan_boost,fn_lock,super_key_enable}`

## Upstreaming

The DMI entry is upstream-ready (`0001-platform-x86-uniwill-add-Eluktronics-RP-17.patch` can be
generated with `git format-patch` against the pristine copy in the first commit of this directory).
The fan-mode/fan-boost additions need hardware validation on more Uniwill boards before submission.
