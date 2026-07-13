# edgemap

DualSense HID remapper — remap buttons, turbo, combos, macros while preserving native Sony HID behavior as much as possible. Three components: `dseuhid` (root daemon, UHID proxy), `edgemap` (user CLI, config management), and `edgemap-gui` (PyQt6 config editor). The daemon reads the physical controller through hidraw, converts reports through source/target codecs, and exposes a virtual USB HID device through UHID.

## Disclaimer

This project is entirely vibecoded. The author has no programming knowledge and cannot review the code. 

This project was created solely for the author's own needs. Remap without losing any DualSense features.

SDL-based remappers cannot preserve DualSense-specific features such as adaptive triggers and HD haptics. 
Steam Input is slightly different — games supporting [Native mode](https://partner.steamgames.com/doc/features/steam_controller/concepts) retain DualSense features while using Steam Input, but for Legacy Mode only games, Steam maps the controller to a standard XInput device.

This project is provided AS IS. The author has tested it to the best of their ability,
but makes no warranty regarding functionality or stability.

## Note on Proton patches

Some games may require a [patched Proton build](https://github.com/Kurobac/proton-eg-patch)
for DualSense HD haptics or the DualShock 4 target (`output_device = "dualshock4"`).

## Install

### Arch Linux

Install from AUR:

```bash
paru -S edgemap
# or use your favourite AUR helper
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

When upgrading, install both binaries from the same release and restart both
services so the control protocol versions stay in sync:

```bash
sudo systemctl restart dseuhid
systemctl --user restart edgemap
```

>**Note:** install.sh assumes a writable /usr.
>If you are using an immutable distribution,
>please install manually.

### Build from source

```bash
cargo build --release
```

edgemap daemon auto-creates `edgemap.toml` + `default.toml` under
`$XDG_CONFIG_HOME/edgemap` (default: `~/.config/edgemap`) on first run.
Run `edgemap create-config` to print a template with full inline documentation.

### Daemon coordination and limits

The daemons communicate through the versioned Unix `SOCK_SEQPACKET` endpoint
`/run/dseuhid/control.sock`. The server accepts at most 16 simultaneous clients
and delivers at most one request per event-loop turn so control traffic cannot
starve HID processing.

Configuration paths must resolve to regular files no larger than 256 KiB.
Nonblocking open and a bounded read prevent FIFOs, device nodes, and dynamically
generated pseudo-files from causing unbounded reads. Socket-triggered configuration
failures return fixed category messages without echoing parser details or file
contents; run `edgemap validate <path>` for detailed diagnostics under the caller's
own permissions.

Both systemd units cap memory at 64 MiB, tasks at 32, and open file descriptors at
128, and disable core dumps. These limits are final containment boundaries rather
than normal operating targets.

## Features

| Feature | Description |
|---------|-------------|
| Remap | Any button → button, stick direction, or full trigger press |
| Turbo | Hold-to-repeat with configurable interval and delay |
| Combo | Modifier key + key → injected output |
| Macro | Timed key sequences, hold (loop) and single (one-shot) modes |
| Keyboard target | Remap, combo, macro, and split-touchpad output to 107 uinput keyboard keys |
| Profile auto-switch | Match running processes (comm/cmdline), auto-switch remap config |
| Hot reload | inotify-based (`edgemap.toml`) or acknowledged Unix socket command |
| Event-driven hotplug | libudev device discovery with immediate controller add/remove handling |
| Native HID behavior | DS5 USB target keeps source backing where possible; BT physical output wraps USB target output into the DS5 BT main-output envelope |
| Regular DualSense | DualSense (0x0CE6) and DualSense Edge (0x0DF2) supported over USB and Bluetooth source hidraw |
| DSE→DS virtualization | `output_device = "dualsense"` makes Edge appear as regular DS for game compatibility |
| DualShock 4 target (Beta) | `output_device = "dualshock4"` exposes a DS4-compatible UHID target for native DS4 games |
| GET_REPORT cache | DS5 USB/BT physical devices read calibration/firmware data on startup |
| GUI config editor | PyQt6 native editor — remap, turbo, combo, macro, macro manager, save/load |

> **Note:** `output_device = "dualsense"` on a regular DualSense is harmless — the device
> is already recognized as DS. Edge-specific button configs (paddles, FN keys) are
> silently ignored on regular DualSense; they simply have no physical counterpart.

## Configuration

Button sections use lowercase names. Missing sections and `remap = "passthrough"` both leave
the physical input untouched.

```toml
version = 2
output_device = "auto" # "auto", "dualsense", or "dualshock4"

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

`edgemap daemon` creates `edgemap.toml` in the XDG config directory for profile switching:

```toml
config = "default.toml"

[profiles.example]
config = "example.toml"
match_process = "game.exe"
```

Profiles are checked in declaration order. `match_process` is an exact process-name match;
`match_cmdline` is a substring match. When both are set, both must match.

### DualShock 4 target (Beta)

`output_device = "dualshock4"` makes the virtual controller identify as a
DualShock 4 (`054C:09CC`, `Wireless Controller`). This is intended for games
with native DS4 support while keeping dseuhid's remap/combo/macro pipeline.

For best Proton compatibility, use the DS4 UHID MI_03 identity patch from
[proton-eg-patch](https://github.com/Kurobac/proton-eg-patch). Tests show that
some native DS4 games require the Windows device path to look like
`VID_054C&PID_09CC&MI_03` with version `0100` before they perform the complete
Sony feature init sequence (`0x12 -> 0xA3 -> 0x14 -> 0x02`). Without the patch,
some games still accept buttons and show Sony icons, while others may only do a
partial init or ignore the controller.

### Bluetooth source support

DualSense and DualSense Edge can be used as physical Bluetooth source devices.
The virtual controller is still exposed as a USB UHID target; dseuhid does not
create a Bluetooth virtual target.

Bluetooth source input is decoded into the same remap/combo/macro pipeline as
USB input. DS5 USB target output is wrapped back into the DualSense Bluetooth
main-output report, so normal vibration, player LEDs, mic LED, lightbar, and
adaptive-trigger payloads can work when games use the main output path.

Bluetooth SET_REPORT / vendor feature-report forwarding is not implemented.
Known vendor/test commands are dropped, so hardware-test tools may miss some
factory or speaker functions. Games that rely on normal DualSense output
reports should continue to work.

For BT source → DS5 USB target, dseuhid repeats the latest virtual input at a
stable cadence by default to avoid gyro drift in some games.
Use `DSEUHID_BT_DS5_USB_REPEAT_HZ` to override the default
`1000Hz`, or `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` to disable it. DS4
target repeat is disabled by default and can be enabled with
`DSEUHID_BT_DS4_USB_REPEAT_HZ`.

## Not supported

- Multiple controllers
- D‑Bus control API
- Non-Sony device

## Requirements

- Linux with `uhid` kernel module
- `libudev` for initialized hidraw device discovery and hotplug monitoring
- `acl` utilities (`getfacl` and `setfacl`) for hiding/restoring physical device nodes
- Kernel: tested 7.0, should work 6.7+, may work 5.12+
- DualSense (0x0CE6) or DualSense Edge (0x0DF2) controller over USB or Bluetooth source hidraw
- Root for `dseuhid` only
- `python-pyqt6` for GUI support (optional)
- `libnotify` for profile-switch notifications (optional)
- Configuration files must be regular files no larger than 256 KiB

## Special Thanks

*   **[inputplumber](https://github.com/ShadowBlip/InputPlumber)** - The initial demo of this project was built based on its core concepts and ideas.
*   **[dualsense-tester](https://github.com/daidr/dualsense-tester)** - This repository served as a valuable reference for establishing and defining our HID behaviors.

## License

GPL-3.0-or-later
