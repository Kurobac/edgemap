# edgemap

DualSense HID remapper — remap buttons, turbo, combos, macros without losing DualSense-specific features. Three components: `dseuhid` (root daemon, UHID proxy), `edgemap` (user CLI, config management), and `edgemap-gui` (PyQt6 config editor). All DualSense-specific features (adaptive triggers, HD haptics, gyro, touchpad) pass through untouched.

## Disclaimer

This project is entirely vibecoded. The author has no programming knowledge and cannot review the code. 

This project was created solely for the author's own needs. Remap without losing any DualSense features.

SDL-based remappers cannot preserve DualSense-specific features such as adaptive triggers and HD haptics. Steam Input is slightly different — games supporting [Native mode](https://partner.steamgames.com/doc/features/steam_controller/concepts) retain DualSense features while using Steam Input, but for Legacy Mode only games, Steam maps the controller to a standard XInput device.

This project is provided ASIS. The author has tested it to the best of their ability,
but makes no warranty regarding functionality or stability.

## Quick start

```bash
cargo build --release
sudo install -m755 target/release/dseuhid /usr/bin/dseuhid
sudo install -m755 target/release/edgemap /usr/bin/edgemap
sudo install -m755 edgemap-gui-v6.py /usr/bin/edgemap-gui
sudo install -m644 edgemap.svg /usr/share/icons/hicolor/scalable/apps/edgemap.svg
sudo install -m644 dseuhid.service /usr/lib/systemd/system/dseuhid.service
install -Dm644 edgemap.service ~/.config/systemd/user/edgemap.service

sudo systemctl daemon-reload
sudo systemctl enable --now dseuhid
systemctl --user daemon-reload
systemctl --user enable --now edgemap
```

edgemap daemon auto-creates `~/.config/edgemap/edgemap.toml` + `default.toml` on first run.
Run `edgemap create-config` to print a template with full inline documentation.

## Features

| Feature | Description |
|---------|-------------|
| Remap | Any button → button, stick direction, or full trigger press |
| Turbo | Hold-to-repeat with configurable interval and delay |
| Combo | Modifier key + key → injected output |
| Macro | Timed key sequences, hold (loop) and single (one-shot) modes |
| Profile auto-switch | Match running processes (comm/cmdline), auto-switch remap config |
| Hot reload | Mtime-based (edgemap) or FIFO command |
| Passthrough | All DualSense HID data — gyro, touchpad, LED, rumble, adaptive triggers, HD haptics — forwarded untouched |
| Regular DualSense | Both DualSense (0x0CE6) and DualSense Edge (0x0DF2) supported |
| DSE→DS virtualization | `--force-dualsense` flag makes Edge appear as regular DS for game compatibility |
| GET_REPORT cache | IMU calibration data read from physical device on startup (accurate gyro) |
| GUI config editor | PyQt6 native editor — remap, turbo, combo, macro, macro manager, save/load |

## Not supported (by design)

- Bluetooth / wireless
- Multiple controllers
- D‑Bus API, inotify watch

## Requirements

- Linux, systemd, `uhid` kernel module
- DualSense (0x0CE6) or DualSense Edge (0x0DF2) controller, USB only
- Root for `dseuhid` only
- pyqt6 for GUI support

## Special Thanks

*   **[inputplumber](https://github.com/ShadowBlip/InputPlumber)** - The initial demo of this project was built based on its core concepts and ideas.
*   **[dualsense-tester](https://github.com/daidr/dualsense-tester)** - This repository served as a valuable reference for establishing and defining our HID behaviors.
*   **Deepseek and Opencode** - The tools built the software.
*   **KDE Community** - We use icon from Breeze.

## License

GPL-3.0-or-later
