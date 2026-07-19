#!/usr/bin/env bash
set -e

cd -- "$(dirname -- "$(readlink -f -- "$0")")"

action=install
if (($# > 1)) || (($# == 1)) && [[ $1 != uninstall ]]; then
    echo "Usage: ${0##*/} [uninstall]" >&2
    exit 1
fi
if (($# == 1)); then
    action=uninstall
fi

install_root=${DESTDIR:-}
if [[ -n $install_root ]]; then
    if [[ $install_root != /* ]]; then
        echo "error: DESTDIR must be an absolute path" >&2
        exit 1
    fi
    if [[ $install_root =~ ^/+$ ]]; then
        echo "error: DESTDIR=/ is not a staging directory" >&2
        exit 1
    fi
    install_root=${install_root%/}
elif ((EUID != 0)); then
    echo "error: changes to / require root (or set an absolute DESTDIR for staging)" >&2
    exit 1
fi

if [[ $action == uninstall ]]; then
    echo "Uninstalling edgemap..."

    rm -f -- \
        "$install_root/usr/local/bin/dseuhid" \
        "$install_root/usr/local/bin/edgemap" \
        "$install_root/usr/local/bin/edgemap-gui" \
        "$install_root/usr/lib/systemd/system/dseuhid.service" \
        "$install_root/usr/lib/systemd/user/edgemap.service" \
        "$install_root/usr/share/applications/edgemap.desktop" \
        "$install_root/usr/share/icons/hicolor/scalable/apps/edgemap.svg" \
        "$install_root/usr/share/zsh/site-functions/_dseuhid" \
        "$install_root/usr/share/zsh/site-functions/_edgemap"
    rm -rf -- "$install_root/usr/local/lib/edgemap-gui"

    if [[ -n $install_root ]]; then
        echo "Staged uninstall complete: $install_root"
        exit 0
    fi

    echo
    echo "Uninstallation complete. Services were not changed automatically."
    echo "If they were enabled, run:"
    echo "  sudo systemctl disable --now dseuhid"
    echo "  systemctl --user disable --now edgemap"
    echo "  sudo systemctl daemon-reload"
    echo "  systemctl --user daemon-reload"
    exit 0
fi

required_files=(
    dseuhid
    edgemap
    edgemap-gui
    usr/lib/systemd/system/dseuhid.service
    usr/lib/systemd/user/edgemap.service
    usr/share/applications/edgemap.desktop
    usr/share/icons/hicolor/scalable/apps/edgemap.svg
    usr/share/zsh/site-functions/_dseuhid
    usr/share/zsh/site-functions/_edgemap
    usr/local/lib/edgemap-gui/edgemap_gui/__init__.py
)

for file in "${required_files[@]}"; do
    if [[ ! -f $file ]]; then
        echo "error: release payload is missing: $file" >&2
        exit 1
    fi
done

echo "Installing edgemap..."

install -Dm755 dseuhid "$install_root/usr/local/bin/dseuhid"
install -Dm755 edgemap "$install_root/usr/local/bin/edgemap"
install -Dm644 usr/lib/systemd/system/dseuhid.service \
    "$install_root/usr/lib/systemd/system/dseuhid.service"
install -Dm644 usr/lib/systemd/user/edgemap.service \
    "$install_root/usr/lib/systemd/user/edgemap.service"
install -Dm644 usr/share/applications/edgemap.desktop \
    "$install_root/usr/share/applications/edgemap.desktop"
install -Dm644 usr/share/icons/hicolor/scalable/apps/edgemap.svg \
    "$install_root/usr/share/icons/hicolor/scalable/apps/edgemap.svg"
install -Dm644 usr/share/zsh/site-functions/_dseuhid \
    "$install_root/usr/share/zsh/site-functions/_dseuhid"
install -Dm644 usr/share/zsh/site-functions/_edgemap \
    "$install_root/usr/share/zsh/site-functions/_edgemap"

gui_dir="$install_root/usr/local/lib/edgemap-gui/edgemap_gui"
rm -rf -- "$gui_dir"
install -d -m755 "$gui_dir"
cp -a usr/local/lib/edgemap-gui/edgemap_gui/. "$gui_dir/"
install -Dm755 edgemap-gui "$install_root/usr/local/bin/edgemap-gui"

if [[ -n $install_root ]]; then
    echo "Staged installation complete: $install_root"
    exit 0
fi

if ! command -v python3 >/dev/null 2>&1 ||
    ! python3 -c 'import PyQt6' >/dev/null 2>&1; then
    echo "warning: python-pyqt6 is not installed; edgemap-gui cannot start" >&2
fi
if ! command -v notify-send >/dev/null 2>&1; then
    echo "warning: notify-send is not installed; profile-switch notifications are disabled" >&2
fi

echo
echo "Installation complete. Start the services with:"
echo "  sudo systemctl daemon-reload"
echo "  sudo systemctl enable --now dseuhid"
echo "  systemctl --user daemon-reload"
echo "  systemctl --user enable --now edgemap"
echo
echo "After upgrading, restart both services:"
echo "  sudo systemctl restart dseuhid"
echo "  systemctl --user restart edgemap"
