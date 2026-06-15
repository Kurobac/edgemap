#!/bin/bash
set -e

install -m755 dseuhid /usr/local/bin/
install -m755 edgemap /usr/local/bin/
install -m755 edgemap-gui /usr/local/bin/
install -Dm755 usr/lib/systemd/system/dseuhid.service /usr/lib/systemd/system/
install -Dm644 usr/lib/systemd/user/edgemap.service /usr/lib/systemd/user/
install -Dm644 usr/share/icons/hicolor/scalable/apps/edgemap.svg /usr/share/icons/hicolor/scalable/apps/
install -Dm644 usr/share/zsh/site-functions/_dseuhid /usr/share/zsh/site-functions/
install -Dm644 usr/share/zsh/site-functions/_edgemap /usr/share/zsh/site-functions/

systemctl daemon-reload
systemctl --user daemon-reload
