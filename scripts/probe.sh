#!/usr/bin/env bash
# Hardware probe for the Eluktronics RP-17 / Uniwill EC keyboard backlight.
#
#   sudo ./scripts/probe.sh            # non-destructive report
#   sudo ./scripts/probe.sh --test     # also blink the keyboard through a few colours/levels
#
# Writes a report to docs/hardware-probe-<date>.txt (and prints it). Share that file when
# reporting problems. It never changes anything persistently; --test restores the previous state.
set -uo pipefail
[[ $EUID -eq 0 ]] || { echo "run as root: sudo $0 [--test]" >&2; exit 1; }

HERE=$(cd "$(dirname "$0")/.." && pwd)
OUT="$HERE/docs/hardware-probe-$(date +%Y%m%d-%H%M%S).txt"
mkdir -p "$HERE/docs"
exec > >(tee "$OUT") 2>&1

section() { printf '\n===== %s =====\n' "$*"; }
run() { printf '$ %s\n' "$*"; "$@" 2>&1 || true; }

section "System"
run uname -r
run cat /sys/class/dmi/id/sys_vendor /sys/class/dmi/id/product_name /sys/class/dmi/id/product_sku /sys/class/dmi/id/board_name /sys/class/dmi/id/bios_version

section "ACPI / WMI devices"
run ls /sys/bus/acpi/devices | grep -iE 'INOU|UNIW|WMI' || true
run ls /sys/bus/wmi/devices

section "Module"
run modinfo -n uniwill-laptop
run modinfo -F version uniwill-laptop
run dkms status
if ! lsmod | grep -q '^uniwill_laptop'; then
    echo "module not loaded; trying modprobe uniwill-laptop"
    run modprobe uniwill-laptop
    sleep 1
fi
run lsmod | grep -E '^uniwill|^wmi' || true
run dmesg --ctime | grep -iE 'uniwill|INOU|kbd_backlight|platform_profile' | tail -30

section "LED class"
for led in /sys/class/leds/*kbd_backlight*; do
    [[ -e $led ]] || continue
    echo "LED: $led -> $(readlink -f "$led")"
    for f in brightness max_brightness multi_intensity multi_index brightness_hw_changed trigger; do
        [[ -e $led/$f ]] && printf '  %-22s %s\n' "$f" "$(tr -d '\n' < "$led/$f" | cut -c1-80)"
    done
    echo "  device attrs:"; ls "$led/device" 2>/dev/null | tr '\n' ' '; echo
done
ls /sys/class/leds/*kbd_backlight* >/dev/null 2>&1 || echo "NO kbd_backlight LED FOUND"

section "Platform profile / fan"
run cat /sys/firmware/acpi/platform_profile_choices
run cat /sys/firmware/acpi/platform_profile
for d in /sys/devices/platform/uniwill*; do
    [[ -d $d ]] || continue
    echo "platform device: $d"
    for f in fan_boost fn_lock super_key charge_type; do
        [[ -e $d/$f ]] && printf '  %-12s %s\n' "$f" "$(cat "$d/$f")"
    done
done
for h in /sys/class/hwmon/hwmon*; do
    n=$(cat "$h/name" 2>/dev/null)
    [[ $n == uniwill ]] || continue
    echo "hwmon $h ($n):"
    for f in "$h"/{fan,temp}*_{input,label}; do [[ -e $f ]] && printf '  %-14s %s\n' "$(basename "$f")" "$(cat "$f")"; done
done

section "Power supply"
for p in /sys/class/power_supply/*; do
    printf '%s type=%s online=%s\n' "$(basename "$p")" "$(cat "$p/type" 2>/dev/null)" "$(cat "$p/online" 2>/dev/null || echo -)"
done

section "uwbld"
run systemctl --no-pager --lines=10 status uwbld.service
run omarchy-eluktonics-keyboard status

if [[ ${1:-} == --test ]]; then
    section "Live test"
    led=$(ls -d /sys/class/leds/*kbd_backlight* 2>/dev/null | head -1)
    if [[ -z $led ]]; then
        echo "no LED, cannot test"
    else
        systemctl stop uwbld.service 2>/dev/null
        old_b=$(cat "$led/brightness"); old_i=$(cat "$led/multi_intensity" 2>/dev/null || echo "")
        echo "saved brightness=$old_b intensity='$old_i'"
        for lvl in 1 2 3 4; do echo "brightness $lvl"; echo $lvl > "$led/brightness"; sleep 0.7; done
        if [[ -n $old_i ]]; then
            for c in "50 0 0:red" "0 50 0:green" "0 0 50:blue" "50 50 50:white" "50 25 0:orange"; do
                echo "multi_intensity ${c#*:} (${c%%:*})"; echo "${c%%:*}" > "$led/multi_intensity"; sleep 0.8
            done
        fi
        echo "off"; echo 0 > "$led/brightness"; sleep 0.7
        [[ -n $old_i ]] && echo "$old_i" > "$led/multi_intensity"
        echo "$old_b" > "$led/brightness"
        echo "restored"
        systemctl start uwbld.service 2>/dev/null
        echo
        echo "Did the keyboard change brightness 1..4, then show red/green/blue/white/orange, then go off?"
        echo "Record the answer in docs/hardware-probe.md."
    fi
fi

echo
echo "report written to $OUT"
