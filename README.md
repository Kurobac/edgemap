# edgemap

A Linux remapping tool for DualSense and DualSense Edge that preserves native Sony controller features.

edgemap lets you customize controller inputs while keeping features such as adaptive triggers, HD haptics, and motion controls available to games with native DualSense support.

It works by creating a virtual Sony HID controller through Linux UHID, allowing input remapping without emulating a generic XInput controller.

> [!IMPORTANT]
> When using edgemap with Proton, many games need a patched Proton build to use DualSense HD haptics or recognize the virtual DualShock 4. See [proton-eg-patch](https://github.com/Kurobac/proton-eg-patch).

## Features

**Remapping and advanced inputs**

Remap controller buttons to other buttons, stick directions, triggers, or keyboard keys. Create turbo actions, button combinations, and timed macros. Split touchpad input is also supported.

**Profiles**

Create different configurations for different games. Profiles can be switched manually or automatically based on running processes. A PyQt6 GUI provides an editor for mappings, macros, and profiles.

**Native controller support**

Supports DualSense and DualSense Edge over both USB and Bluetooth.

The virtual controller can retain the original Sony identity or emulate a standard DualSense. An experimental DualShock 4 output mode is also available.

Bluetooth HD haptics are supported through a virtual PipeWire audio device, allowing compatible games to use audio-based haptic effects wirelessly.

## Installation

### Arch Linux

Install from the [AUR](https://aur.archlinux.org/packages/edgemap):

### Other Linux distributions

Download the precompiled x86_64 package from [Releases](https://github.com/Kurobac/edgemap/releases), extract it, and run:

```bash
sudo ./install.sh
```

### Getting started

Enable the system and user services:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now dseuhid
systemctl --user daemon-reload
systemctl --user enable --now edgemap
```

Then launch `edgemap-gui` to create and manage your configurations.

The GUI requires Python 3.11+ and PyQt6. Bluetooth HD haptics additionally require PipeWire and `pw-cat`.

## Compatibility and limitations

edgemap is designed primarily for games with native DualSense support. For games without native DualSense support, you can use edgemap alongside Steam Input or similar tools.

Currently, edgemap supports one DualSense or DualSense Edge controller at a time.

For detailed feature behavior, controller output modes, Bluetooth handling, and troubleshooting, see the [User Guide](docs/USER_GUIDE.md).

## Requirements

- Linux kernel 6.7+ (tested on 7.0) with UHID support
- Python 3.11+ and PyQt6 for the GUI
- PipeWire and `pw-cat` for Bluetooth HD haptics

## Disclaimer

This project is entirely vibecoded. The author tests it through actual use but cannot independently audit the code. It is provided as-is, with no warranty.

## Acknowledgments

Thanks to [InputPlumber](https://github.com/ShadowBlip/InputPlumber), [dualsense-tester](https://github.com/daidr/dualsense-tester), and [SAxense](https://github.com/egormanga/SAxense) for their ideas and technical references.

## License

[GPL-3.0-or-later](LICENSE).
