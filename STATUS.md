# dseuhid — Project Status (2026-06-01)

## Overview

Single-binary UHID proxy for DualSense Edge wireless controller (PID 0x0DF2, USB only). Replaces the kernel `hid-playstation` driver with a userspace virtual HID device, applies button remapping frame-by-frame, and forwards all other HID data (gyro, touchpad, LED, rumble, adaptive trigger FFB, HD haptics via audio) transparently.

Written in Rust. Zero async runtime. Single epoll loop. Root required for `/dev/uhid` and `/dev/hidraw` access.

## Version History

| Tag | Commit | Summary |
|-----|--------|---------|
| v0.0.1 | `373efce` | Pure UHID proxy — all HID features confirmed working |
| v0.0.2 | `27ae79a` | Basic remap (Cross→Circle test rule) + byte position fix |
| v0.0.3 | `bc6defa` | TOML config system + 12 bugfixes + log cleanup |
| v0.0.4 | `d9f54b7` | 41 unit tests + hardened device detection |

**Unreleased (HEAD):** touchpad split mode + missing-button warnings

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
- **Block**: source → disabled (`remap = "none"`)
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
- Validation: unknown source buttons, unknown targets, split mode rules
- Default config auto-generated at `/etc/dseuhid/config.toml`
- Missing section warnings (all 22 buttons checked at startup)
- Reserved fields: `turbo`, `turbo_interval_ms`, `turbo_delay_ms`, `combos`, `macros` (parsed but unused)

### Device Detection
- Scan `/dev/hidraw*`, ioctl HIDIOCGRAWINFO for VID/PID
- Physical-only: reject UHID virtual devices (check `/sys/class/hidraw/N/device/uevent`)
- Edge-only: skip regular DualSense (PID 0x0CE6)
- State validation: read first input report on open()
- Multi-device: warn if more than one Edge detected
- EIO cooldown: 2-second sleep after disconnect

### Unit Tests (41 tests, all passing)
| Module | Tests | Coverage |
|--------|-------|----------|
| `mapping.rs` | 13 | single/multi-key remap, cross-map, self-map, block, TriggerFull L2/R2, 8 stick dirs, analog clear, snapshots isolation |
| `report.rs` | 11 | byte position for all button groups (face/shoulder/system/Edge), all-button roundtrip, byte11 preservation, stick/trigger values, seq |
| `config.rs` | 17 | valid sources/targets, trigger/stick/block targets, unknown validation, mic/edge exclusions, missing passthrough, default config parse |

### Tools
| Tool | Binary | Description |
|------|--------|-------------|
| `dseuhid` | main | UHID proxy daemon |
| `monitor` | `src/bin/monitor.rs` | Raw HID button bit verification (no UHID) |
| `touchdemo` | `src/bin/touchdemo.rs` | Touchpad coordinate debug + zone detection |

## Bugfixes (chronological)

| # | Bug | Root Cause | Commit |
|---|-----|-----------|--------|
| 1 | ACL lost after exit | `setfacl -b` removed ACL, restore didn't re-apply | `18e826e` + `b5ad76a` |
| 2 | WebHID/CP2077 SET_REPORT timeout | UHID didn't reply SET_REPORT_REPLY → kernel timeout EIO → Chrome abandoned device | `450cc4d` |
| 3 | All output reports silently dropped | `rtype == USB_OUTPUT_REPORT_ID` always false (rtype is UHID type, not HID report ID) | `56c6528` |
| 4 | SET_REPORT replied OK but data not forwarded | `send_set_report_reply(id,0)` but data never written to real hardware | `373efce` |
| 5 | Hotplug: EIO → exit instead of reconnect | No outer loop | `fb39885` |
| 6 | Ctrl+C during reconnect wait ignored | Inner wait loop didn't check RUNNING flag | `fb39885` |
| 7 | restore_permissions panic on DeviceGone | Node already removed, path doesn't exist | `fb39885` skip_restore + exists check |
| 8 | Remap broke FN/back buttons | `parse_input_report` / `apply_state_to_report` read/write byte 11 (should be byte 10) | `27ae79a` |
| 9 | FN/back buttons left/right swapped | Byte 10 high nibble bit→label mapping was wrong: 0x10=FnLeft not RightPaddle, 0x40=LeftPaddle not LeftFn | `6e3ceec` |
| 10 | Cross-mapping (A→B + B→A) random result | `apply()` used same `state` for reads and writes → rule1 output contaminated rule2 input | `86bcf92` (snapshots clone) |
| 11 | Multi-key simultaneous missing outputs | Deferred button targets set immediately within rule loop → later rule.clear() undid them | `86bcf92` (two-phase apply) |
| 12 | PS/Mic button leak | `apply_state_to_report` preserved `raw[10] & 0x0F` → low nibble from previous frame leaked | `6e3ceec` |
| 13 | Trigger analog leak | `apply()` cleared L2/R2 digital bit but not `l2_analog`/`r2_analog` | `6e3ceec` |
| 14 | GET_REPORT/SET_REPORT/UHID Output spam at debug | Default `debug!` level flooded logs | `bc6defa` → `trace!` |
| 15 | Device found log duplicated | Both `find_dualsense()` and `main.rs` logged the same info | `bc6defa` |
| 16 | UHID virtual device matched by find_dualsense | No physical/UHID distinction → could proxy own virtual device recursively | `42bf5ca` |
| 17 | Touchpad coordinate wrong (x=3800 out of range) | Byte offset: `touchdata` starts at byte 33, not 34; nibble: low nibble = x_hi, not high nibble | `81b330d` |
| 18 | Split mode allowed without both partitions | Validation missing | `81b330d` |
| 19 | Split mode allowed with touchpad section missing | `contains_key("touchpad")` guard was too specific | `48739ae` |

## TODO

### High Priority
- [ ] **Hot reload**: `Arc<RwLock<MappingConfig>>` + SIGHUP → reload without UHID restart
- [ ] **Dead code cleanup**: 24 compiler warnings (unused functions, camel case, imports)

### Medium Priority
- [ ] **Turbo**: hold-to-rapid-fire with configurable interval/delay (state machine in `apply()`)
- [ ] **Combo**: modifier key combinations (predefined key→output sequence)
- [ ] **Macro**: timed key sequences, uinput injection for keyboard events
- [ ] **D-Bus interface**: zbus session bus server for runtime control, edgemap CLI client

### Low Priority
- [ ] **Trigger source mapping**: analog threshold-based (half-press vs full-press events)
- [ ] **Touchpad 4-zone**: expand from left/right to quadrant grid
- [ ] **Regular DualSense support**: re-enable PID 0x0CE6 (was removed for simplicity)

## Commit History

```
48739ae feat: warn when button sections are missing from config
81b330d feat: touchpad split mode (left/right partition)
d9f54b7 test: add 41 unit tests covering mapping, report, config   [v0.0.4]
42bf5ca fix: harden device detection
bc6defa chore: cleanup log levels and device messages               [v0.0.3]
86bcf92 fix: defer button target writes to prevent multi-key collision
6e3ceec step2: batch of 12 fixes since TOML config commit
d85859a step2: TOML config loading with all-button remap support
27ae79a step1: add remap support with byte position fix              [v0.0.2]
fb39885 dseuhid: add hotplug support (poll-based)
8fbb7b8 step0: remove dead code from report.rs
373efce dseuhid: forward SET_REPORT data to real hardware via hidraw [v0.0.1]
56c6528 dseuhid: fix output report forwarding
450cc4d dseuhid: reply to SET_REPORT with UHID_SET_REPORT_REPLY
18e826e dseuhid: save and restore ACL when hiding device nodes
b5ad76a dseuhid: fix ACL restore
ed15311 dseuhid: init pure UHID proxy from old implementation
```
