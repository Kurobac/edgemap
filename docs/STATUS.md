# edgemap — Project Status (2026-06-05)

## Overview

UHID proxy for DualSense and DualSense Edge wireless controllers (PID 0x0CE6 / 0x0DF2, USB only). Two binaries: `dseuhid` (daemon, root) and `edgemap` (user CLI). Replaces the kernel `hid-playstation` driver with a userspace virtual HID device, applies button remapping frame-by-frame, and forwards all other HID data (gyro, touchpad, LED, rumble, adaptive trigger FFB, HD haptics via audio) transparently.

Written in Rust. Zero async runtime. Single epoll loop. Root required for `/dev/uhid` and `/dev/hidraw` access (daemon only).

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
| v0.7.0 | `7ec92d0` | **Regular DualSense support (0x0CE6)**: device detection for both DS and DSE; HID report descriptor read from physical device via HIDIOCGRDESC; `--force-dualsense` flag to virtualize dualsense edge as regular DS |
| v0.7.1 | `10f1fbb` | **GET_REPORT cache (0x05 calibration, 0x20 firmware)**: read calibration and firmware reports from physical device on startup; skip 0x09 (MAC address) to avoid sysfs naming conflict (BUGFIX #63) |
| v0.7.2 | `6919430` | **Cleanup + fixes**: remove dead code (monitor, touchdemo, trigger_reload, dualsense_usb_descriptor); FIFO buffer 256→4096 (#65); GUI closeEvent unsaved changes prompt; BUGFIX #64 (Cargo incremental build cache) |
| v0.7.3 | `143f1c5` | **Bugfixes**: dup_fifo_fd() missing exit on dup failure (#66); duplicate return{} in load_config (#67); /run/dseuhid/connected cleanup on shutdown + waiting-for-device log (#68); turbo toggle vs physical button press override (#69) |
| v0.7.4 | `e6b5209` | **Bugfixes**: re-restrict hidraw after suspend/resume udev reset (#70, #71); non-root daemon spam prevention + cooldown; FIFO command confirmation output; zsh completions; notify-send app name grouping; GUI taskbar icon fix; turbo source as combo key comment |

## Implemented Features

### Core UHID Proxy
- Virtual DualSense Edge HID device via `/dev/uhid` (UHID_CREATE2)
- Physical hidraw read → report update → UHID_INPUT2 forward
- Game output report (rtype=1) → raw write passthrough to physical hardware
- All 28 feature reports handled (GET_REPORT cache)
- SET_REPORT → UHID_SET_REPORT_REPLY + HIDIOCSFEATURE forward
- Physical device node hiding: `chmod 000` + `setfacl -b`, restore on exit
- ACL save/restore (getfacl → setfacl)

### Gamepad Features (all confirmed working)
| Feature | Status | Tested with |
|---------|--------|-------------|
| Buttons / sticks / D-pad | ✅ | Steam controller test, KDE, WebHID tester |
| Gyroscope / accelerometer | ✅ | Steam test |
| Touchpad (single finger) | ✅ | WebHID tester, dualsense-tester |
| FN / back paddle buttons | ✅ | KDE, WebHID tester, monitor tool |
| LED / player indicators | ✅ | WebHID tester output panel |
| Rumble | ✅ | CP2077, WebHID tester |
| Adaptive trigger FFB | ✅ | CP2077 |
| HD haptics | ✅ | 原神 (Genshin Impact) native DS support |
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
- Default config auto-generated by edgemap daemon at `~/.config/edgemap/default.toml`
- Missing section warnings (all 22 buttons checked at startup)
- Case-sensitive section names (uppercase rejected)

### Hot Reload
- SIGHUP (`kill -HUP <pid>`) triggers config re-read
- `Arc<RwLock<MappingConfig>>` — read lock per frame, write lock on reload
- Failed reload reverts to passthrough (`MappingConfig::default`)
- Debug snapshots cleared on reload
- Turbo runtimes rebuilt on reload

### Turbo System
- **Turbo (hold-to-repeat)**: hold source → one-shot target → (delay_ms) → toggle at interval_ms
- State machine in `proxy.rs`, reads from snapshot, runs after mapping rules
- Per-frame target maintain (not just on toggle events)
- Source suppression: digital + analog cleared while turbo active
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

### Pipeline Architecture (v0.0.9+)
Three-layer per-frame processing model:
```
Layer 1 (physical filter): parse → touchpad → TURBO → BLOCK → freeze(L1)
Layer 2 (virtual generate): REMAP (reads L1, writes state) [+ combo/macro slots]
Layer 3 (output): L1 passthrough + L2 outputs → apply_state_to_report → UHID
```
- **Turbo** runs in L1 before freeze: reads physical snapshot, writes to state
- **Block** (`remap="block"`) now in L1: clears digital + analog (L2/R2)
- **Remap** (`MappingConfig::apply`) reads frozen L1, writes virtual output
- Downstream layers never affect upstream; L2 components are parallel and isolated //todo 需要重构以达到真分离，目前仍在同写一张表

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
- Hot reload: macro runtimes rebuilt on SIGHUP

### Device Detection
- Scan `/dev/hidraw*`, ioctl HIDIOCGRAWINFO for VID/PID
- Physical-only: reject UHID virtual devices (check `/sys/class/hidraw/N/device/uevent`)
- Both DualSense (0x0CE6) and DualSense Edge (0x0DF2) supported
- HID report descriptor read from physical device via HIDIOCGRDESC, `DS_EDGE_USB_DESCRIPTOR` fallback
- `--force-dualsense` flag: override virtual device to regular DS (PID 0x0CE6 + `DS_USB_DESCRIPTOR`)
- State validation: read first input report on open()
- Multi-device: warn if more than one DualSense detected
- EIO cooldown: 2-second sleep after disconnect

### Unit Tests (135 total: 68 dseuhid + 67 edgemap shared-module imports, all passing)
| Module | Tests | Coverage |
|--------|-------|----------|
| `mapping.rs` | 12 | single/multi-key remap, cross-map, self-map, TriggerFull L2/R2, 8 stick dirs, analog clear, snapshots isolation |
| `report.rs` | 11 | byte position for all button groups (face/shoulder/system/Edge), all-button roundtrip, byte11 preservation, stick/trigger values, seq |
| `config.rs` | 42 | valid sources/targets, trigger/stick targets, combo validation (key/output/duplicate/mutex/FN+face), macro validation (empty seq, release>press, name conflict, turbo+macro mutex, combo→macro), block→blocked_buttons, turbo+block allowed, uppercase rejection, default config parse |
| `device.rs` | 1 | sysfs path resolution |

### Tools
| Tool | Binary | Description |
|------|--------|-------------|
| `dseuhid` | main | UHID proxy daemon (+ `--force-dualsense` flag, `version`, `help` subcommands) |
| `edgemap` | `src/bin/edgemap.rs` | User-side CLI: validate, create-config, reload, switch-config (no root). Daemon mode (d/daemon): auto-create config, profile auto-switch, mtime hot reload, notify-send |
| `edgemap-gui-v6.py` | GUI | PyQt6 config editor: two-column layout, remap/turbo/combo/macro editing, macro manager, toolbar with KDE-native icons |
| `completions/` | zsh | zsh completions for `dseuhid` and `edgemap` commands (validate/switch-config auto-complete configs from `~/.config/edgemap/`) |

## Bugfixes (chronological)

See [BUGFIX.md](./BUGFIX.md) for the complete list.

## Future Plans

### Completed — v0.1.0 milestone

- [x] Dead code cleanup (v0.0.8)
- [x] Pipeline refactor: three-layer architecture (v0.0.9)
- [x] Combo: modifier key combinations (v0.0.10)
- [x] Macro: timed key sequences, hold & single modes, combo→macro (v0.0.11)
- [x] `--config-path` flag for custom config file (v0.1.0+1)
- [x] Systemd service unit (v0.1.0+1)
- [x] Compact default config template (v0.1.0+2)
- [x] CLI subcommands: monitor, touchdemo, version, help; unknown command rejection (v0.1.0+4)
- [x] FIFO control daemon: `/run/dseuhid/control` named pipe, non-root reload + switch-config (v0.1.0+5)
- [x] Monitor shows analog sticks (threshold 5, 80ms throttle) (v0.1.0+6)
- [x] edgemap CLI: `validate`, `create-config`, `reload`, `switch-config` (no root, separate binary)
- [x] edgemap daemon: auto-create config, alive-detect dseuhid, auto-inject remap
- [x] edgemap auto-switch profiles by running processes (comm exact + cmdline substring)
- [x] edgemap mtime-based hot reload + notify-send notifications
- [x] Singleton instance detection (dseuhid + edgemap)
- [x] BUGFIX.md (#1-#51 documented with root causes)
- [x] GUI config editor (PyQt6, two-column layout, toolbar with KDE-native icons)

### Planned — Next Features

| Priority | Feature | Complexity | Description |
|----------|---------|-----------|-------------|
| Low | **Trigger source mapping** | Medium | Analog threshold events (half-press vs full-press) |

### Explicitly Abandoned

| Feature | Reason |
|---------|--------|
| D-Bus interface (zbus) | Introduces async runtime, excessive complexity for marginal benefit |
| inotify auto-reload | edgemap mtime-based reload sufficient |
| GUI config generator | Implemented — edgemap-gui-v6.py (PyQt6) |
| Multiple controllers | Not planned. |
