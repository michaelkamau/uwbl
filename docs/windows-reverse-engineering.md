# Reverse-engineering notes: Eluktronics Control Center 1.1.0.55

Source: `reference/ControlCenterU_1.1.0.55_Eluktronics2_T1/` (Inno Setup installer,
Uniwill "GamingCenter" OEM build). Payload extracted with `innoextract`; the .NET
assemblies (`GCUService.exe`, `ControlCenter.exe`, `DefaultTool/*.dll`) were mined for
constants with `dnfile`. All constants are in [gcuservice_constants.txt](gcuservice_constants.txt).
No proprietary code is reproduced in this repository; only register addresses and
semantics, which are also documented by the mainline `uniwill-laptop` driver and
`tuxedo-drivers`.

## Architecture of the Windows software

| Piece | Role |
|---|---|
| `GCUService.exe` (Windows service, .NET) | Talks to the EC through WMI, owns keyboard/fan state, exposes an MQTT broker (`M2Mqtt.Net.dll`) for the UI. |
| `ControlCenter.exe` (WPF UI) | Front end; RGB pickers, fan-mode buttons, OSD. |
| `DefaultTool/` | Helper DLLs: `inpoutx64.dll` (raw port I/O), `SharpDX.RawInput`, `Gma.UserActivityMonitor` (idle detection for backlight idle-off), `NAudio` (music mode). |
| `*.reg` | Default registry values: `RGBKeyboard` → `ACLight=3`, `DCLight=0`, 7 default colours. |

### EC access

`AcpiTest_MULong.GetSetULong` (WMI GUID `ABBC0F6F-8EA1-11D1-00A0-C90629100000`,
method 4). 8-byte argument `{addr_lo, addr_hi, data_lo, data_hi, 0, read?1:0, 0, 0}`.
On Linux the mainline driver uses the equivalent ACPI methods `ECRR`/`ECRW` on
`INOU0000` (regmap, ~6 ms per access) and only uses WMI GUID `ABBC0F72…` for hotkey
events.

### Keyboard backends in the Windows code

| Backend | Transport | Present on RP-17? |
|---|---|---|
| `FourZone` | ITE USB HID, usage page `0xFF12` | No (no ITE device in `lsusb`) |
| `MEZone_*` (per-key) | ITE USB HID, usage pages `0xFF02`/`0xFF03` | No |
| `SingleZone` | EC RAM registers (below) | **Yes** |

The RP-17 (`DMI: Eluktronics Inc. / RP-17 / RP-17 G1`) is a Tongfang GK7NR0R and
reports EC project ID `0x10` (GK7NXXR family), the same board as the TUXEDO Polaris
17 Gen1 AMD and the XMG Apex 17 (2020).

## EC register map (SingleZone keyboard + fans)

| Addr | Name (Windows) | Bits / values | Linux driver use |
|---|---|---|---|
| `0x0740` | `PROJECT_ID` | `0x10` = GK7NXXR | probe check |
| `0x0741` | `AP_OEM` | bit0 = application present / manual control | set on probe |
| `0x0748..4B` | lightbar | not fitted on RP-17 | – |
| `0x0751` | `MANUAL_FAN_CTRL` | bits2:0 level, bit4 TURBO, bit5 HIGH, bit6 BOOST, bit7 USER | **our patch**: `platform_profile` balanced ⇔ turbo clear, performance ⇔ turbo set |
| `0x0766` | `SUPPORT_2` | bit2 RGB keyboard present, bit5 china mode | feature detection |
| `0x0767` | `TRIGGER` | bit2 fan boost toggle, bit5 apply RGB, bit7 rainbow | RGB apply; **our patch**: `fan_boost` write |
| `0x0768` | `SWITCH_STATUS` | bit2 fan boost active | **our patch**: `fan_boost` read |
| `0x0769/6A/6B` | `RGB_R/G/B` | 0..50 (`RGB_LEVEL_MAX=0x32`, Windows scales 0..255 by `/5`); `0,0,0` = restore EC default | `multi_intensity` |
| `0x076C/6D/6E` | default R/G/B | read-only | – |
| `0x076F` | music mode | `0xFE` on | not implemented |
| `0x0782` | | bit6 enable china mode | – |
| `0x078C` | `KBD_STATUS` | bit0 white-only, bit1 power off, bit4 apply, bits7:5 brightness 0..4 | `brightness` |

Hotkey / OSD event codes (WMI event GUID `ABBC0F72…`): `0x3B..0x3F` = KB LED level
0..4, `0xB1` illumination down, `0xB2` up, `0xB9` toggle. The kernel driver turns
these into `brightness_hw_changed` notifications, so UPower/KDE show the OSD.

## Behaviour we replicate

| Windows feature | Linux implementation |
|---|---|
| Monochrome (static, 30 presets) | `uwbl color …`, 16 named presets + any `#rrggbb` |
| Manual (6-colour cycle, software timer) | `effect = cycle`, `colors = […]` (software, ≤15 fps) |
| Breathing (software) | `effect = breathing` (never fully dark: floor 8 %) |
| Rainbow (EC `0x0767.7`) | software rainbow (hue sweep) – EC rainbow bit not exposed by the kernel LED API |
| `ACLight=3` / `DCLight=0` | `[profiles.ac]` / `[profiles.battery]`, switched from `/sys/class/power_supply` |
| Idle-off timer (`Gma.UserActivityMonitor`) | `idle_off_secs`, daemon reads `/dev/input/event*` |
| Gaming / Beast fan mode | `platform_profile` balanced / performance |
| Fan Boost button | `fan_boost` sysfs attribute |
| Restore after sleep | logind `PrepareForSleep(false)` → re-apply |

Out of scope: lightbar, music mode, per-key/4-zone HID keyboards, GPU/CPU
overclocking pages, MQTT UI protocol.

## Open questions for the hardware probe

1. Do `multi_intensity` writes actually change the colour on this firmware, or is the
   keyboard white-only (`0x078C` bit0)? → `scripts/probe.sh --test`.
2. Does `0x0766` bit5 (china mode) need to be set for RGB to take effect?
3. Does the turbo bit in `0x0751` behave as Beast mode (fans audibly faster)?
4. Does a brightness change via Fn produce a `brightness_hw_changed` event?
