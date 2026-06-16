# edgemap

DualSense HID remapper — remap buttons, turbo, combos, macros without losing DualSense-specific features. Three components: `dseuhid` (root daemon, UHID proxy), `edgemap` (user CLI, config management), and `edgemap-gui` (PyQt6 config editor). All DualSense-specific features (adaptive triggers, HD haptics, gyro, touchpad) pass through untouched.

## Disclaimer

This project is entirely vibecoded. The author has no programming knowledge and cannot review the code. 

This project was created solely for the author's own needs. Remap without losing any DualSense features.

SDL-based remappers cannot preserve DualSense-specific features such as adaptive triggers and HD haptics. 
Steam Input is slightly different — games supporting [Native mode](https://partner.steamgames.com/doc/features/steam_controller/concepts) retain DualSense features while using Steam Input, but for Legacy Mode only games, Steam maps the controller to a standard XInput device.

This project is provided AS IS. The author has tested it to the best of their ability,
but makes no warranty regarding functionality or stability.

## Note on HD haptics

dseuhid does not touch the USB audio channel that carries DualSense HD haptics
data. Some games work fine out of the box (e.g. Genshin Impact). However,
certain games (e.g. Cyberpunk 2077) use Windows ContainerId or device-tree
traversal to locate the audio endpoint associated with the controller — the
UHID virtual device cannot satisfy this. These games require a [patched Protonbuild](https://github.com/Kurobac/proton-eg-patch) 
to restore HD haptics. If HD haptics don't work with a physical
DualSense on vanilla Proton, dseuhid will not help either.

See [docs/HD-HAPTICS-FIX.md](docs/HD-HAPTICS-FIX.md) for the technical details.

## Install

### Arch Linux

```bash
# AUR package is not published yet.
```

### Other Linux (pre-compiled binary)

Download the latest tarball from the [Releases](https://github.com/kurobac/edgemap/releases) page:

```bash
tar xzf edgemap-v*.tar.gz
cd edgemap-v*-x86_64/
sudo ./install.sh
# then enable systemd services
sudo systemctl daemon-reload
sudo systemctl enable --now dseuhid
systemctl --user daemon-reload
systemctl --user enable --now edgemap
```
>**Note:** install.sh assumes a writable /usr.
>If you are using an immutable distribution,
>please install manually.

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
| Keyboard target | Remap, combo, macro, and split-touchpad output to 106 uinput keyboard keys |
| Profile auto-switch | Match running processes (comm/cmdline), auto-switch remap config |
| Hot reload | Mtime-based (edgemap) or FIFO command |
| Passthrough | All DualSense HID data — gyro, touchpad, LED, rumble, adaptive triggers, HD haptics — forwarded untouched |
| Regular DualSense | Both DualSense (0x0CE6) and DualSense Edge (0x0DF2) supported |
| DSE→DS virtualization | `output_device = "dualsense"` makes Edge appear as regular DS for game compatibility |
| GET_REPORT cache | IMU calibration data read from physical device on startup (accurate gyro) |
| GUI config editor | PyQt6 native editor — remap, turbo, combo, macro, macro manager, save/load |

> **Note:** `output_device = "dualsense"` on a regular DualSense is harmless — the device
> is already recognized as DS. Edge-specific button configs (paddles, FN keys) are
> silently ignored on regular DualSense; they simply have no physical counterpart.

## Configuration

Button sections use lowercase names. Missing sections and `remap = "passthrough"` both leave
the physical input untouched.

```toml
version = 2
output_device = "auto" # "auto" or "dualsense"

[cross]
remap = "circle"

[right_paddle]
remap = "key:space"

[left_paddle]
remap = "combo"

[[left_paddle.combos]]
key = "cross"
output = "triangle"
```

`edgemap daemon` creates `~/.config/edgemap/edgemap.toml` for profile switching:

```toml
config = "default.toml"

[profiles.example]
config = "example.toml"
match_process = "game.exe"
```

Profiles are checked in declaration order. `match_process` is an exact process-name match;
`match_cmdline` is a substring match. When both are set, both must match.

## Not supported (by design)

- Bluetooth / wireless
- Multiple controllers
- D‑Bus API, inotify watch
- Other source / target device

## Requirements

- Linux with `uhid` kernel module
- `acl` utilities (`getfacl` and `setfacl`) for hiding/restoring physical device nodes
- Kernel: tested 7.0, should work 6.7+, may work 5.12+
- DualSense (0x0CE6) or DualSense Edge (0x0DF2) controller, USB only
- Root for `dseuhid` only
- `python-pyqt6` for GUI support (optional)
- `libnotify` for profile-switch notifications (optional)

## Special Thanks

*   **[inputplumber](https://github.com/ShadowBlip/InputPlumber)** - The initial demo of this project was built based on its core concepts and ideas.
*   **[dualsense-tester](https://github.com/daidr/dualsense-tester)** - This repository served as a valuable reference for establishing and defining our HID behaviors.
*   **Deepseek and Opencode** - The tools built the software.
*   **KDE Community** - We use icon from Breeze.

## License

GPL-3.0-or-later
