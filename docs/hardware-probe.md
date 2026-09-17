# Hardware probe – Eluktronics RP-17

Status: **partial live verification**. On 2026-09-17, the RP-17 was recovered from
a post-suspend EC transport failure using the 0.1.1 WMI fallback. The full
`sudo ./scripts/probe.sh --test` checklist below has not been completed.

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
| 1 | `modprobe uniwill-laptop` loads without `force=1` | matched by DMI entry, `dmesg` shows no "unsupported" | Patched module loaded without force; DMI quirk selected. |
| 2 | `/sys/class/leds/uniwill:multicolor:kbd_backlight` exists | `max_brightness = 4`, `multi_index = red green blue` | |
| 3 | `echo 1..4 > brightness` | keyboard steps through 4 levels | |
| 4 | `echo "50 0 0" > multi_intensity` (then G, B, white, orange) | keyboard changes colour | |
| 5 | If 4 fails: `0x078C` bit0 (white-only) set? `0x0766` bit5 needed? | | |
| 6 | Fn + brightness key | `brightness_hw_changed` updates and `omarchy-eluktonics-keyboard status` follows | |
| 7 | `/sys/firmware/acpi/platform_profile_choices` | `balanced performance` | |
| 8 | `omarchy-eluktonics-keyboard fan performance` | fans audibly ramp (Beast) | |
| 9 | `omarchy-eluktonics-keyboard boost on` | `fan_boost` reads 1, fans at max | |
| 10 | hwmon `uniwill`: CPU/GPU temps, 2 fan RPMs plausible | | Recovered from 255 C / 65535 RPM to 51/40 C and 2191/684 RPM. |
| 11 | Suspend / resume | colour + brightness restored within ~2 s | Backlight and violet colour restored after live S3 resumes; user confirmed lighting works. EC readings remained valid. |
| 12 | Unplug / plug AC | battery / AC profile applied | |
| 13 | Reboot | module autoloads, `uwbld` and the Omarchy widget restore the last settings | |

## Notes / dmesg excerpts

The daemon remained running throughout two problematic resumes. `ECRR` reads
returned `0xff`; brightness stayed at 4 despite requests for 1 and 2. Local ACPI
disassembly showed that `ECRR`/`ECRW` access `0xfe200000 + register`, whereas
`WMBC` method 4 uses the working ACPI EC mailbox. Loading the patched module
logged `EC memory window unavailable; using WMI mailbox`, restored violet RGB
intensities (`27 0 50`), and made hardware brightness follow requests again.

## Decisions taken based on the probe

Keep the fast MMIO transport on healthy systems and use the checked WMI fallback
only for the RP-17 when its uncached project ID is unreadable or `0xff`.
