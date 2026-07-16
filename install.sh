#!/bin/bash
set -e

install -m755 dseuhid /usr/local/bin/
install -m755 edgemap /usr/local/bin/
install -d /usr/local/lib/edgemap-gui
gui_stage=$(mktemp -d /usr/local/lib/edgemap-gui/.edgemap_gui.XXXXXX)
trap 'rm -rf "$gui_stage"' EXIT
cp -a usr/local/lib/edgemap-gui/edgemap_gui/. "$gui_stage/"
rm -rf /usr/local/lib/edgemap-gui/edgemap_gui
mv "$gui_stage" /usr/local/lib/edgemap-gui/edgemap_gui
trap - EXIT
install -m755 edgemap-gui /usr/local/bin/
install -Dm755 usr/lib/systemd/system/dseuhid.service /usr/lib/systemd/system/
install -Dm644 usr/lib/systemd/user/edgemap.service /usr/lib/systemd/user/
install -Dm644 usr/share/applications/edgemap.desktop /usr/share/applications/
install -Dm644 usr/share/icons/hicolor/scalable/apps/edgemap.svg /usr/share/icons/hicolor/scalable/apps/
install -Dm644 usr/share/zsh/site-functions/_dseuhid /usr/share/zsh/site-functions/
install -Dm644 usr/share/zsh/site-functions/_edgemap /usr/share/zsh/site-functions/

if ! python3 -c "import PyQt6" 2> /dev/null; then
    echo
    echo "Optional dependency missing: python-pyqt6"
    echo "The GUI editor will not work."
    echo "Install it using your distribution package manager."
    echo
fi

echo "Installation complete. You can start the daemon using:"
echo "sudo systemctl daemon-reload"
echo "sudo systemctl enable --now dseuhid"
echo "systemctl --user daemon-reload"
echo "systemctl --user enable --now edgemap"
echo
echo "After upgrading both binaries, restart both services:"
echo "sudo systemctl restart dseuhid"
echo "systemctl --user restart edgemap"
