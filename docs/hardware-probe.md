# Hardware probe – Eluktronics RP-17

Status: **not yet run** (the module was built but could not be loaded during development
because no root access was available). Fill this in after `sudo ./scripts/probe.sh --test`.

## Machine

| | |
|---|---|
| DMI | `Eluktronics Inc.` / `RP-17` / sku `RP-17 G1` |
| Board | Tongfang GK7NR0R (Uniwill EC, ACPI `INOU0000:00`, WMI `ABBC0F6A…72`) |
| CPU / GPU | Ryzen 7 4800H / RTX 2060 |
| OS / kernel | Omarchy / current Arch kernel |

## Checklist

Run `sudo ./scripts/probe.sh --test` and record the outcome of each item.

| # | Check | Expected | Result |
|---|---|---|---|
| 1 | `modprobe uniwill-laptop` loads without `force=1` | matched by DMI entry, `dmesg` shows no "unsupported" | |
| 2 | `/sys/class/leds/uniwill:multicolor:kbd_backlight` exists | `max_brightness = 4`, `multi_index = red green blue` | |
| 3 | `echo 1..4 > brightness` | keyboard steps through 4 levels | |
| 4 | `echo "50 0 0" > multi_intensity` (then G, B, white, orange) | keyboard changes colour | |
| 5 | If 4 fails: `0x078C` bit0 (white-only) set? `0x0766` bit5 needed? | | |
| 6 | Fn + brightness key | `brightness_hw_changed` updates and `omarchy-eluktonics-keyboard status` follows | |
| 7 | `/sys/firmware/acpi/platform_profile_choices` | `balanced performance` | |
| 8 | `omarchy-eluktonics-keyboard fan performance` | fans audibly ramp (Beast) | |
| 9 | `omarchy-eluktonics-keyboard boost on` | `fan_boost` reads 1, fans at max | |
| 10 | hwmon `uniwill`: CPU/GPU temps, 2 fan RPMs plausible | | |
| 11 | Suspend / resume | colour + brightness restored within ~2 s | |
| 12 | Unplug / plug AC | battery / AC profile applied | |
| 13 | Reboot | module autoloads, `uwbld` and the Omarchy widget restore the last settings | |

## Notes / dmesg excerpts

(paste here)

## Decisions taken based on the probe

(e.g. keep `force=1` fallback, set china-mode bit, adjust fan-mode semantics)
