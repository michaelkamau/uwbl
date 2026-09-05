#!/usr/bin/env bash
# Remove everything install.sh put on the system.  sudo ./uninstall.sh [--purge]
set -euo pipefail
[[ $EUID -eq 0 ]] || { echo "run as root: sudo $0" >&2; exit 1; }

PREFIX=${PREFIX:-/usr}
DKMS_NAME=uniwill-laptop-uwbl
PURGE=0
[[ ${1:-} == --purge ]] && PURGE=1

systemctl disable --now uwbld.service 2>/dev/null || true
pkill -x uwbl-tray 2>/dev/null || true

for v in $(dkms status "$DKMS_NAME" 2>/dev/null | sed -n "s|^$DKMS_NAME[/,] *\([^,: ]*\).*|\1|p" | sort -u); do
    dkms remove "$DKMS_NAME/$v" --all || true
    rm -rf "/usr/src/$DKMS_NAME-$v"
done
modprobe -r uniwill-laptop 2>/dev/null || true

rm -f "$PREFIX/bin/uwbld" "$PREFIX/bin/uwbl" "$PREFIX/bin/uwbl-tray" \
      /etc/systemd/system/uwbld.service \
      /etc/dbus-1/system.d/org.uniwill.Backlight1.conf \
      "$PREFIX/share/polkit-1/actions/org.uniwill.backlight1.policy" \
      /etc/udev/rules.d/99-uwbl.rules \
      /etc/modprobe.d/uniwill-laptop-uwbl.conf \
      /etc/modules-load.d/uniwill-laptop-uwbl.conf \
      /etc/xdg/autostart/uwbl-tray.desktop \
      "$PREFIX/share/applications/uwbl-tray.desktop"

if [[ $PURGE == 1 ]]; then
    rm -rf /etc/uwbl /var/lib/uwbld
else
    echo "kept /etc/uwbl and /var/lib/uwbld (use --purge to remove)"
fi

systemctl daemon-reload
udevadm control --reload
depmod -a
echo "uwbl removed. The in-tree uniwill-laptop module (without RP-17 support) is active again after reboot."
