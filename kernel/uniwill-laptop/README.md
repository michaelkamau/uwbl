# uniwill-laptop (out-of-tree build with RP-17 support)

This directory is a copy of the mainline Linux driver
`drivers/platform/x86/uniwill/` (Armin Wolf, GPL-2.0-or-later) with the following
local additions, clearly marked `Out-of-tree addition` in the source:

| Change | Why |
|---|---|
| DMI entry `Eluktronics Inc.` / `RP-17` → `eluktronics_rp17_descriptor` | Ubuntu's 7.0 module has no keyboard-backlight support at all, and mainline only matches TUXEDO boards. The RP-17 (Tongfang GK7NR0R) gets a single-zone RGB keyboard backlight with 5 brightness levels, temps, fans, fn-lock, super-key, charge modes. |
| `platform_profile` (`balanced` / `performance`) | Fan/performance mode via EC `0x0751` turbo bit — the Control Center's Gaming / Beast modes. |
| `fan_boost` sysfs attribute on `/sys/bus/platform/devices/INOU0000:00/` | The Control Center's Fan Boost button (trigger `0x0767.2`, status `0x0768.2`). |
| RP-17 WMI EC fallback at probe/resume | The firmware's `ECRR`/`ECRW` memory window can become inaccessible after S3, returning `0xff` and ignoring writes. Check the uncached project ID before restoring registers and switch to the firmware's ACPI EC mailbox when needed. |
| Kbuild compat shims (`UW_HAVE_MC_MAX_INTENSITY`, `UW_HAVE_WMI_MIN_EVENT_SIZE`) | Build against kernels that predate those core changes (e.g. Ubuntu 7.0). |

Because the module keeps the name `uniwill-laptop` and DKMS installs to
`/lib/modules/<ver>/updates/dkms/`, it shadows the in-tree copy automatically.

## Build / install

```sh
# build only (needs make, gcc matching the kernel, linux-headers)
make
# install through DKMS (recommended; rebuilt on kernel upgrades)
sudo dkms add ./kernel/uniwill-laptop        # from the repo root; registers uniwill-laptop-uwbl/0.1.1
sudo dkms install uniwill-laptop-uwbl/0.1.1
sudo modprobe uniwill-laptop
```

## What appears after loading

- `/sys/class/leds/uniwill:multicolor:kbd_backlight/` (`brightness` 0–4, `multi_intensity` "R G B" 0–50,
  `brightness_hw_changed`) — consumed by UPower/KDE and by `uwbld`
- `/sys/class/hwmon/hwmonN` (`name` = `uniwill`): CPU/GPU temps, fan RPM, fan PWM (read-only)
- `/sys/firmware/acpi/platform_profile` (`balanced`, `performance`)
- `/sys/bus/platform/devices/INOU0000:00/{fan_boost,fn_lock,super_key_enable}`

## RP-17 resume recovery

If the daemon is running but lighting commands do nothing, check the kernel's
`uniwill` hwmon readings. Temperatures of 255000 (255 C), fan speeds of 65535,
and brightness stuck at 4 indicate an inaccessible EC memory window, not a
stopped daemon. Restarting only `uwbld` cannot repair this.

The RP-17 quirk checks the EC transport at probe and before resume's regcache
restore. It keeps the fast memory interface when healthy; otherwise it verifies
and switches to WMI method 4 on `ABBC0F6F-8EA1-11D1-00A0-C90629100000`, logging
`EC memory window unavailable; using WMI mailbox`. The fallback stays active
until the module is reloaded. WMI transactions are slower, so animated effects
may run at a lower frame rate in fallback mode.

The mailbox reply layout is specific to the RP-17 firmware: read data is in byte
0 and `0xfefefefe` is an error, not data. Do not enable this quirk for other
models without checking their firmware. Missing/malformed replies fail explicitly.
No raw chipset register writes or firmware modifications are needed.

The RP-17 DMI entry does not need `force=1`. Remove any old forced-loading option
from local modprobe configuration; it enables unrelated, unsupported features.

## Upstreaming

The DMI entry is upstream-ready (`0001-platform-x86-uniwill-add-Eluktronics-RP-17.patch` can be
generated with `git format-patch` against the pristine copy in the first commit of this directory).
The fan-mode/fan-boost additions need hardware validation on more Uniwill boards before submission.
