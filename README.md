# edgemap

DualSense HID remapper — remap buttons, turbo, combos, macros without losing DualSense-specific features. Three components: `dseuhid` (root daemon, UHID proxy), `edgemap` (user CLI, config management), and `edgemap-gui` (PyQt6 config editor). All DualSense-specific features (adaptive triggers, HD haptics, gyro, touchpad) pass through untouched.

## Disclaimer

This project is entirely vibecoded. The author has no programming knowledge and cannot review the code. 

This project was created solely for the author's own needs. Remap without losing any DualSense features.

SDL-based remappers cannot preserve DualSense-specific features such as adaptive triggers and HD haptics. Steam Input is slightly different — games supporting [Native mode](https://partner.steamgames.com/doc/features/steam_controller/concepts) retain DualSense features while using Steam Input, but for Legacy Mode only games, Steam maps the controller to a standard XInput device.

This project is provided ASIS. The author has tested it to the best of their ability,
but makes no warranty regarding functionality or stability.

## Install

### Arch Linux

```bash
# TODO: upload to AUR
# yay -S dseuhid
```

### Other Linux (pre-compiled binary)

Download the latest tarball from the [Releases](https://github.com/kurobac/edgemap/releases) page:

```bash
tar xzf edgemap-v*.tar.gz
cd edgemap-v*
sudo ./install.sh
# then enable systemd service
systemctl enable --now dseuhid
systemctl --user enable --now edgemap
```

### Build from source

```bash
cargo build --release
```

### Build from source

```bash
cargo build --release
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
| DSE→DS virtualization | `force_dualsense = true` in config makes Edge appear as regular DS for game compatibility |
| GET_REPORT cache | IMU calibration data read from physical device on startup (accurate gyro) |
| GUI config editor | PyQt6 native editor — remap, turbo, combo, macro, macro manager, save/load |

## Not supported (by design)

- Bluetooth / wireless
- Multiple controllers
- D‑Bus API, inotify watch
- Other source / target device

## Requirements

- Linux with `uhid` kernel module
- Kernel: tested 7.0, should work 6.7+, may work 5.12+
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
