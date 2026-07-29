#!/usr/bin/env bash
set -Eeuo pipefail

cd -- "$(dirname -- "$(readlink -f -- "$0")")"

usage() {
    echo "Usage: ${0##*/} [uninstall]" >&2
}

case $# in
0)
    action=install
    ;;
1)
    if [[ $1 != uninstall ]]; then
        usage
        exit 1
    fi
    action=uninstall
    ;;
*)
    usage
    exit 1
    ;;
esac

if ((EUID != 0)); then
    echo "error: installation and uninstallation require root" >&2
    exit 1
fi

if [[ -n ${DESTDIR:-} ]]; then
    echo "error: DESTDIR is not supported; install to the fixed system paths" >&2
    exit 1
fi

if [[ $action == uninstall ]]; then
    echo "Uninstalling edgemap..."

    rm -f -- \
        /usr/local/bin/dseuhid \
        /usr/local/bin/edgemap \
        /usr/local/bin/edgemap-gui \
        /usr/lib/systemd/system/dseuhid.service \
        /usr/lib/systemd/user/edgemap.service \
        /usr/share/applications/edgemap.desktop \
        /usr/share/icons/hicolor/scalable/apps/edgemap.svg \
        /usr/share/zsh/site-functions/_dseuhid \
        /usr/share/zsh/site-functions/_edgemap
    rm -rf -- /usr/local/lib/edgemap-gui

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

install -Dm755 dseuhid /usr/local/bin/dseuhid
install -Dm755 edgemap /usr/local/bin/edgemap
install -Dm644 usr/lib/systemd/system/dseuhid.service \
    /usr/lib/systemd/system/dseuhid.service
install -Dm644 usr/lib/systemd/user/edgemap.service \
    /usr/lib/systemd/user/edgemap.service
install -Dm644 usr/share/applications/edgemap.desktop \
    /usr/share/applications/edgemap.desktop
install -Dm644 usr/share/icons/hicolor/scalable/apps/edgemap.svg \
    /usr/share/icons/hicolor/scalable/apps/edgemap.svg
install -Dm644 usr/share/zsh/site-functions/_dseuhid \
    /usr/share/zsh/site-functions/_dseuhid
install -Dm644 usr/share/zsh/site-functions/_edgemap \
    /usr/share/zsh/site-functions/_edgemap

gui_dir=/usr/local/lib/edgemap-gui/edgemap_gui
rm -rf -- "$gui_dir"
install -d -m755 "$gui_dir"
cp -a usr/local/lib/edgemap-gui/edgemap_gui/. "$gui_dir/"
install -Dm755 edgemap-gui /usr/local/bin/edgemap-gui

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
