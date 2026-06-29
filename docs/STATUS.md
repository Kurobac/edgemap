# edgemap — Project Status (2026-06-30)

## Overview

UHID proxy for DualSense and DualSense Edge controllers (PID 0x0CE6 / 0x0DF2) over USB or Bluetooth source hidraw. Two binaries: `dseuhid` (daemon, root) and `edgemap` (user CLI). Reads physical DualSense input via `/dev/hidraw`, decodes it through a source codec, applies button remapping frame-by-frame, and emits a virtual USB HID target through `/dev/uhid`. Native Sony behavior is preserved through explicit codec paths rather than unconditional raw passthrough.

Written in Rust. Zero async runtime. Single epoll loop. Root required for `/dev/uhid` and `/dev/hidraw` access (daemon only). Kernel compatibility: tested 7.0, should work 6.7+, may work 5.12+.

## Version History

| Tag | Commit | Summary |
|-----|--------|---------|
| v0.0.1 | `373efce` | Pure UHID proxy — all HID features confirmed working |
| v0.0.2 | `27ae79a` | Basic remap (Cross→Circle test rule) + byte position fix |
| v0.0.3 | `bc6defa` | TOML config system + 12 bugfixes + log cleanup |
| v0.0.4 | `d9f54b7` | 41 unit tests + hardened device detection |
| v0.0.5 | `40fcbe4` | Touchpad split mode + missing-button warnings |
| v0.0.6 | `9aa83b8` | Hot reload (SIGHUP) + trigger analog transfer fix |
| v0.0.7 | `6616c24` | Turbo (hold-to-repeat) + `none`→`block` rename + SIGHUP startup fix |
| v0.0.8 | `d294917` | Code cleanup: 0 warnings, 43 tests, AGENTS.md, StickDir naming |
| v0.0.9 | `98299bb` | **Three-layer pipeline refactor** (L1 filter → L2 generate → L3 output) |
| v0.0.10 | `1cfcd2b` | **Combo** (modifier key combinations, L1 suppress + L2 inject) |
| v0.0.11 | `0e8186f` | **Macro** (timed key sequences, hold & single modes, combo→macro) |
| v0.1.0 | `2264e94` | **Feature complete**: turbo + combo + macro + remap, 3-layer pipeline, --config-path, systemd unit |
| v0.2.0 | `7bfa313` | **edgemap CLI**: validate, create-config, reload, switch-config; FIFO + subcommand refactor; bugfixes #32-#37 |
| v0.3.0 | `e78dc94` | **edgemap daemon**: auto-create config, alive-detect dseuhid, auto-inject remap on connect |
| v0.4.0 | `43ff3f5` | **edgemap profile auto-switch**: process matching (comm/cmdline), mtime hot reload, notify-send |
| v0.4.1 | `c6a71c3` | **singleton detection**: PID file + kill(0) for dseuhid + edgemap, BUGFIX.md |
| v0.4.2 | `b6967dc` | **PKGBUILD + GPLv3**: packaging, user service, install scripts, bugfixes #48-#49 |
| v0.4.3 | `d3102f6` | **Documentation**: README.md, annotated config template, bugfix consolidation (STATUS.md → BUGFIX.md) |
| v0.4.4 | `8cf87d4` | **Bugfix #50**: detect dseuhid restart via PID change |
| v0.5.0 | `fdee1eb` | **GUI config editor** (PyQt6): UI shell — two-column layout, remap/turbo/combo/macro editing, profile quick-switch, toolbar |
| v0.5.1 | `1fdf001` | **Hotplug fix (#58)**: dseuhid writes /run/dseuhid/connected, edgemap re-injects config on USB reconnect |
| v0.5.2 | `0b954fc` | **Deferred validation + hotplug redo**: profile config validated at injection time; connected/disconnected marker content; bulk validate; version=2 check |
| v0.6.0 | `6a47e24` | **GUI config editor (PyQt6)**: full Save/Save As, edgemap.toml editor, profile quick-switch, toolbar, keyboard shortcuts, macro/combo editors |
| v0.7.0 | `7ec92d0` | **Regular DualSense support (0x0CE6)**: device detection for both DS and DSE; HID report descriptor read from physical device via HIDIOCGRDESC; `force_dualsense` config option to virtualize as regular DS |
| v0.7.1 | `10f1fbb` | **GET_REPORT cache (0x05 calibration, 0x20 firmware)**: read calibration and firmware reports from physical device on startup; skip 0x09 (MAC address) to avoid sysfs naming conflict (BUGFIX #63) |
| v0.7.2 | `6919430` | **Cleanup + fixes**: remove dead code (monitor, touchdemo, trigger_reload, dualsense_usb_descriptor); FIFO buffer 256→4096 (#65); GUI closeEvent unsaved changes prompt; BUGFIX #64 (Cargo incremental build cache) |
| v0.7.3 | `143f1c5` | **Bugfixes**: dup_fifo_fd() missing exit on dup failure (#66); duplicate return{} in load_config (#67); /run/dseuhid/connected cleanup on shutdown + waiting-for-device log (#68); turbo toggle vs physical button press override (#69) |
| v0.7.4 | `e6b5209` | **Bugfixes**: re-restrict hidraw after suspend/resume udev reset (#70, #71); non-root daemon spam prevention + cooldown; FIFO command confirmation output; zsh completions; notify-send app name grouping; GUI taskbar icon fix; turbo source as combo key comment |
| v0.8.0 | `f56968c` | **Keyboard target**: uinput keyboard device for `key:xxx` targets; remap/combo/macro step → keyboard; GUI KeyboardPicker with 106 keycodes; split touchpad→keyboard; turbo+keyboard pipeline fix; six bugfixes (#72-#76); 143 tests (+8) |
| v0.9.0 | `53323f8` | **Code review cleanup**: strict config fields; analog write deferred to Phase 2; SIGHUP reload removed; UHID/uinput write error checking; 149 tests (+6) |
| v1.0.0 | `4bef496` | **Stable release**: `output_device` enum, USB-only detection, UHID state tracking, hardened reload/install/keyboard/ACL failure paths; 153 tests |
| v1.0.1 | `35894d3` | **Path and GUI hardening**: strict XDG/HOME handling, stable passthrough/keyboard/macro editor state, safe TOML/profile serialization, macro reference integrity, GUI CI; 158 Rust + 19 GUI tests |
| v1.0.2 | `4006227` | **DualShock 4 target Beta**: GUI entry and docs for `output_device = "dualshock4"`; native DS4 Proton compatibility notes point to the DS4 UHID MI_03 identity patch in `proton-eg-patch`; 158 Rust + 20 GUI tests |
| v1.1.0 | `TBD` | **Bluetooth source support**: DS5/Edge BT input, BT main-output forwarding, BT GET_REPORT cache, DS5 gyro cadence pacer, codec/error-handling cleanup, safer hot reload; 195 Rust + 21 GUI tests |

## v1.1.0 Release Notes

- DS4 target GUI warning: selecting `DualShock 4 (Beta)` now shows a reminder that some native DS4 games under Proton may require the patched Proton build.
- GUI test coverage: 21 tests with DS4 selection warning and existing-config no-warning coverage.
- Codec/error-handling cleanup: source/physical/target codec boundaries, no raw forwarding on decode failure, stricter UHID event validation, unified hidraw disconnect handling.
- DualSense / DualSense Edge Bluetooth source support: BT input report 0x31 decodes into `ControllerFrame`; virtual targets remain USB UHID.
- Bluetooth physical main output forwarding: DS5/DS4 USB target output is converted into DS5 BT 0x31 output with sequence tag and CRC. Rumble, lightbar, player LED, mic LED, and adaptive trigger payloads are raw-carried through this path.
- BT GET_REPORT cache for 0x05/0x20 is supported with feature CRC32 seed 0xA3 validation; 0x09 physical MAC is still skipped.
- BT SET_REPORT / vendor feature-report forwarding remains unsupported by design for now; known WebHID vendor/test commands and Genshin Impact's feature report 0x08 are debug-logged and dropped. Hardware rejected a naive 0x08 HIDIOCSFEATURE transfer even with a feature CRC tail, so this needs real BT traces before implementation.
- Added `docs/BT_GYRO_INVESTIGATION.md` to record the Genshin BT-source gyro investigation. BT source → DS5 USB target now normalizes UHID input cadence with fixed-rate seq-only repeat by default at 1000Hz; repeat frames advance `raw[7]` but keep the sensor timestamp unchanged. A 250Hz retest also avoids the original severe drift, so cadence stability appears more important than raw rate. `DSEUHID_BT_DS5_USB_REPEAT_HZ` overrides the DS5 target rate, and `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` restores one UHID input per physical BT input. BT source → DS4 USB target repeat is available as an opt-in via `DSEUHID_BT_DS4_USB_REPEAT_HZ`.
- Hot reload and switch-config now keep the previous live config/path on load, validation, or mapping-build failure.
- Output and report diagnostics now log report size/id and distinguish GET_REPORT physical cache from target fallback.

## Implemented Features

### Core UHID Proxy
- Virtual USB HID target via `/dev/uhid` (UHID_CREATE2)
- Physical hidraw read → `SourceCodec` decode → `ControllerFrame` → L1/L2 mapping → `TargetCodec` encode → UHID_INPUT2
- Game output report (rtype=1) → `TargetCodec` decode → `PhysicalCodec` encode → physical hidraw write
- GET_REPORT served from physical feature-report cache, target seed data, or target fallback
- SET_REPORT replies through UHID; physical forwarding is selected by `PhysicalCodec`
- USB and Bluetooth sources are supported for DS5/Edge; all virtual targets are USB UHID targets
- Physical device node hiding: `chmod 000` + `setfacl -b`, restore on exit
- ACL save/restore (getfacl → setfacl)

### Gamepad Features (all confirmed working)
| Feature | Status | Tested with |
|---------|--------|-------------|
| Buttons / sticks / D-pad | ✅ | Steam controller test, KDE, WebHID tester |
| Gyroscope / accelerometer | ✅ | WebHID tester, Steam test |
| Touchpad (single finger) | ✅ | WebHID tester |
| FN / back paddle buttons | ✅ | KDE, WebHID tester, monitor tool |
| LED / player indicators | ✅ | WebHID tester |
| Rumble | ✅ | SDL games |
| Adaptive trigger FFB | ✅ | CP2077 |
| BT main output | ✅ | WebHID tester (rumble, lightbar, player LED, mic LED, adaptive trigger) |
| HD haptics | ✅ | 原神 (Genshin Impact), CP2077 |
| Steam Input bridge | ✅ | Steam games |
| SDL / evdev | ✅ | Non-Steam SDL games |
| Hotplug (USB) | ✅ | Unplug → reconnect → auto-resume |

### Button Remap System
- **Remap**: source button → target button (all standard buttons)
- **TriggerFull**: source → L2/R2 fully depressed (analog = 255)
- **Stick**: source → 8 fixed stick directions (LS/RS up/down/left/right)
- **Block**: source → disabled (`remap = "block"`)
- **Self-map**: source → same target (passthrough, used in default config)
- All standard + Edge buttons supported as source (excluding mic)
- Edge buttons excluded from target options
- All 22+ buttons listed in default config

### Touchpad Split Mode
- `[touchpad] remap = "split"` enables left/right partition
- Coordinate parsing: `buf[33..36]`, x from bytes 34-35, y from bytes 35-36
- Left half (x < 960) → `touchpad_left`, right half (x ≥ 960) → `touchpad_right`
- Both sides must be configured; no partial config allowed
- Touchpad side-buttons excluded from target menu

### Configuration System
- TOML format (compatible with edgemap's field layout)
- `[button_name] remap = "target"` sections
- Validation: unknown source buttons, unknown targets, split mode rules, combo/macro rules
- Default config auto-generated under `$XDG_CONFIG_HOME/edgemap` (default: `~/.config/edgemap`)
- Missing section warnings (all 22 buttons checked at startup)
- Case-sensitive section names (uppercase rejected)

### Hot Reload
- FIFO commands (`edgemap reload`, `edgemap switch-config`) trigger config re-read
- `Arc<RwLock<MappingConfig>>` — read lock per frame, write lock on reload
- Failed reload keeps the previous live config, mapping, runtimes, output-device setting, and config path
- Debug snapshots cleared on reload
- Turbo/combo/macro runtimes rebuilt on reload

### Turbo System
- **Turbo (hold-to-repeat)**: hold source → optional delay → toggle source at interval_ms for L2 processing
- State machine in `proxy.rs`, reads the physical snapshot and runs in L1
- Source suppression: physical digital + analog state is cleared before the toggled source is generated
- Config: `turbo = true`, `turbo_interval_ms` (default 50ms), `turbo_delay_ms` (default 0ms)
- Self-turbo: `[cross] turbo=true` (no remap field) → target = cross itself
- All standard/Edge buttons supported as turbo source

**Turbo edge cases actively rejected:**

| Config | Reason |
|--------|--------|
| `[l2] remap="l2" turbo=true` | Trigger self-map × turbo (analog conflict) |
| `[l2] remap="r2" turbo=true` | Trigger swap × turbo (analog transfer conflict) |
| `[l2] turbo=true` (no remap) | Implicit self-map = trigger self × turbo (same as above) |

**Turbo edge cases allowed:**

| Config | Behavior |
|--------|----------|
| `[l2] remap="l2_full" turbo=true` | Turbo toggle of full L2 press (analog=255/0) |
| `[l2] remap="cross" turbo=true` | Turbo toggle of digital cross (L2 analog cleared) |
| `[cross] remap="l2" turbo=true` | Turbo toggle of digital L2 (analog=0, only digital) |
| `[cross] turbo=true` (no remap) | Self-turbo: cross toggle itself |
| `[cross] remap="block" turbo=true` | Turbo toggle consumed by block L1 filter (no visible output; toggle drives combo key) |

### Pipeline Architecture
Three-layer per-frame processing model:
```
Source codec: hidraw bytes → ControllerFrame
Layer 1 (physical filter): touchpad split → TURBO → combo detection → BLOCK → freeze(L1)
Layer 2 (virtual generate): macro detection → REMAP → combo injection → macro injection → keyboard flush
Layer 3 (output): TargetCodec::encode_input → UHID_INPUT2
```
- **Turbo** runs in L1 before freeze: reads physical snapshot, writes to state
- **Combo detection** runs in L1 before block and suppresses modifier + key
- **Block** (`remap="block"`) now in L1: clears digital + analog (L2/R2)
- **Remap** (`MappingConfig::apply`) reads frozen L1, writes virtual output
- Combo injection runs after remap so source clearing cannot erase combo output
- Keyboard flush combines remap, combo, and macro keyboard events

### Codec Architecture
- `SourceCodec` owns physical input report decoding and input report size.
- `ControllerFrame` carries `GamepadState`, optional motion data, and the source report backing.
- `TargetCodec` owns virtual target input encoding, target output decoding, USB identity, and target GET_REPORT seed/fallback behavior.
- `PhysicalCodec` owns physical output encoding, SET_REPORT forwarding policy, and which physical feature reports are safe to cache.
- DS5 USB target keeps the DS5 USB source report as backing where possible. DS4 target converts input/output through DS4-specific USB report code.
- DS5/DS4 USB byte layout helpers in `report.rs` are wire-format routines, not a transport-neutral HID model.
- DS5 BT source input uses a BT-specific codec and USB-compatible backing; DS5 BT physical output wraps USB target output into the 0x31 Bluetooth main-output envelope. BT physical GET_REPORT cache validates the 0xA3 feature CRC and keeps full-size 0x05/0x20 reports for the USB virtual target.

### Error Handling Policy
- Bad or short input frames are dropped. They are not raw-forwarded to the virtual target.
- Single output or feature-report failures reply with an error or drop that request while the input path keeps running.
- hidraw disconnect errors (`EIO`, `ENODEV`, `ENXIO`) stop the current proxy and wait for reconnect.
- Malformed UHID events, UHID read errors, and UHID input write errors stop the current proxy loop.
- Startup config and required daemon setup failures are fatal. Hot reload failures keep the previous live config.
- Permission hiding/restoration failures are warning-level unless a direct node restriction call returns an error to the caller.

### Combo System (v0.0.10)
- **Modifier key combinations**: hold DSE button + press standard key → mapped output
- Format: `[modifier] remap="combo"` + `[[modifier.combos]]` entries
- L1: detection reads post-turbo snapshot (isolation prevents cross-rule pollution), suppression clears modifier+key from state
- L2: injection writes combo outputs in parallel with remap (both read L1, no cross-talk)
- Turbo+combo allowed: turbo toggle visible to combo detection, output follows turbo phase
- Config validation: remap/combo mutual exclusion, key/output validation, duplicate key reject, self-key reject, FN+face reject, touchpad partition reject
- Combo output can point to macro name (`Target::Macro`) for direct macro activation
- **Known limitation**: d-pad (hat switch) cannot encode 3+ simultaneous directions; conflicting dpad outputs collapse to center per HID spec

### Macro System (v0.0.11)
- **Timed key sequences**: press trigger button → macro plays back step sequence with per-step press_ms/release_ms timing
- Format: `[button] remap="macro_name"` + `[macros.macro_name]` with `sequence = [...]` and optional `mode`
- Two modes: `hold` (hold-to-loop, release-to-stop) and `single` (one-shot, keeps running after release)
- L2: macro detection reads L1 (Physical source) or combo injection (Combo source via `Target::Macro`)
- L2: macro injection writes step buttons every frame (per-frame maintain, same principle as turbo bug #23 fix)
- Config validation: empty sequence reject, release_ms > press_ms, macro name vs button name conflict, same-key turbo+macro mutual exclusion
- Hot reload: macro runtimes rebuilt on FIFO reload

### Keyboard Target (v0.8.0)
- **Keyboard output via uinput**: `remap = "key:<keyname>"` creates keystrokes on a virtual keyboard
- Config format: `key:<name>` as remap target, combo output, or macro step key (e.g. `"key:space"`)
- 107 keycodes supported: letters, numbers, F-keys, navigation, modifiers, symbols, numpad, media
- **uinput device** (`edgemap Keyboard`): auto-created on daemon start, falls back to dummy if unavailable
- **Pipeline integration**:
  - L2 Remap: source button → keyboard event pushed per frame
  - L2 Combo: combo output → keyboard event
  - L2 Macro: macro step with keyboard key → keyboard event via `StepTarget::Keyboard`
  - L2 Keyboard flush: unified per-frame press/release across all sources, runs after all L2 stages
- Turbo + keyboard: turbo toggles source button through L2 remap → keyboard event (no dst needed)
- Split touchpad → keyboard: `[touchpad_left] remap = "key:left"` supported
- GUI: `Keyboard...` entry in remap/combo/macro drop-downs → KeyboardPicker with search filter and 9 categories
- Manual input: typing `key:space` directly in editable comboboxes works, validated at TOML save time

### Device Detection
- Scan `/dev/hidraw*`, ioctl HIDIOCGRAWINFO for VID/PID
- Physical-only: reject UHID virtual devices (check `/sys/class/hidraw/N/device/uevent`)
- USB and Bluetooth source hidraw are accepted for DualSense and DualSense Edge
- Both DualSense (0x0CE6) and DualSense Edge (0x0DF2) supported
- HID report descriptor read from physical device via HIDIOCGRDESC; read failure aborts device open
- USB state validation reads the first full input report on open. BT skips open-time validation because BT may first deliver a minimal 0x01 report; runtime `SourceCodec` validation handles full 0x31 reports and CRC.
- `output_device = "dualsense"` overrides the virtual device to regular DS (PID 0x0CE6 + `DS_USB_DESCRIPTOR`); changing it on reload recreates UHID
- `output_device = "dualshock4"` exposes a DualShock 4 target (PID 0x09CC + `DS4_USB_DESCRIPTOR`, Beta); native DS4 games under Proton should use the DS4 UHID MI_03 identity patch from `proton-eg-patch` for best compatibility
- Multi-device: warn if more than one DualSense detected
- Disconnect cooldown: 2-second sleep after hidraw `EIO` / `ENODEV` / `ENXIO`

### Rust Unit Tests (195 total: 114 dseuhid + 81 edgemap, all passing)
| Module | Tests | Coverage |
|--------|-------|----------|
| `mapping.rs` | 15 | single/multi-key remap, cross-map, self-map, TriggerFull L2/R2, 8 stick dirs, analog clear, snapshots isolation, keyboard target |
| `report.rs` | 10 | byte position for all button groups (face/shoulder/system/Edge), all-button roundtrip, byte11 preservation, stick/trigger values, seq |
| `codec.rs` | 31 | codec selection, USB/BT source paths, USB target identities, BT auto identity, feature report policies/fallbacks, USB/BT feature report validation, DS5/DS4 output conversion, BT output framing/CRC/sequence, BT SET_REPORT non-forwarding policy, touchpad and motion frames |
| `config.rs` | 49 | valid sources/targets (incl. key:xxx), trigger/stick targets, combo validation (key/output/duplicate/mutex/FN+face), macro validation (empty seq, release>press, name conflict, turbo+macro mutex, combo→macro, keyboard step), block→blocked_buttons, turbo+block allowed, uppercase rejection, default config parse |
| `device.rs` | 1 | sysfs hidraw path resolution |
| `proxy.rs` | 2 | BT repeat report sequence handling for DS5 USB and opt-in DS4 USB targets |
| `keyboard.rs` | 2 | successful press tracking, failed press/release state preservation |
| `uhid.rs` | 4 | malformed/oversized UHID OUTPUT and SET_REPORT parsing |
| `edgemap.rs` | 5 | XDG absolute/fallback handling, missing HOME, absolute and tilde config paths |

### GUI Tests (21 total, PyQt6 offscreen)

Coverage includes save/cancel results, macro initialization and reference integrity, TOML quoting, arbitrary profile paths, XDG/HOME handling, passthrough/split serialization, output device serialization, DS4 selection warning behavior, keyboard picker state, action-button styling, and Rust validator compatibility.

### Tools
| Tool | Binary | Description |
|------|--------|-------------|
| `dseuhid` | main | UHID proxy daemon (`-c`/`--config-path`, `version`, `help` subcommands) |
| `edgemap` | `src/bin/edgemap.rs` | User-side CLI: validate, create-config, reload, switch-config (no root). Daemon mode (d/daemon): auto-create config, profile auto-switch, mtime hot reload, notify-send |
| `edgemap-gui-v6.py` | GUI | PyQt6 config editor: XDG-aware config paths, remap/turbo/combo/macro editing, macro reference management, safe TOML serialization |
| `completions/` | zsh | zsh completions for `dseuhid` and `edgemap` commands (validate/switch-config auto-complete configs from `~/.config/edgemap/`) |

## Bugfixes (chronological)

See [BUGFIX.md](./BUGFIX.md) for the complete list.

## Future Plans

### Planned — Next Features

| Priority | Feature | Complexity | Description |
|----------|---------|-----------|-------------|
| Low | **Trigger source mapping** | Medium | Analog threshold events (half-press vs full-press) |

### Explicitly Abandoned

| Feature | Reason |
|---------|--------|
| D-Bus interface (zbus) | Introduces async runtime, excessive complexity for marginal benefit |
| inotify auto-reload | edgemap mtime-based reload sufficient |
| Multiple controllers | Not planned. |
