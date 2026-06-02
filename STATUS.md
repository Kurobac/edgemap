# dseuhid — Project Status (2026-06-03)

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
| v0.0.5 | `40fcbe4` | Touchpad split mode + missing-button warnings |
| v0.0.6 | `9aa83b8` | Hot reload (SIGHUP) + trigger analog transfer fix |
| v0.0.7 | `6616c24` | Turbo (hold-to-repeat) + `none`→`block` rename + SIGHUP startup fix |
| v0.0.8 | `83e2460` | Code cleanup: 0 warnings, 43 tests, AGENTS.md, StickDir naming |
| v0.0.9 | `45f5dc7` | **Three-layer pipeline refactor** (L1 filter → L2 generate → L3 output) |
| v0.0.10 | `3394651` | **Combo** (modifier key combinations, L1 suppress + L2 inject) |
| v0.0.11 | `5053099` | **Macro** (timed key sequences, hold & single modes, combo→macro) |

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
- Default config auto-generated at `/etc/dseuhid/config.toml`
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
- Downstream layers never affect upstream; L2 components are parallel and isolated

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
- Edge-only: skip regular DualSense (PID 0x0CE6)
- State validation: read first input report on open()
- Multi-device: warn if more than one Edge detected
- EIO cooldown: 2-second sleep after disconnect

### Unit Tests (66 tests, all passing)
| Module | Tests | Coverage |
|--------|-------|----------|
| `mapping.rs` | 12 | single/multi-key remap, cross-map, self-map, TriggerFull L2/R2, 8 stick dirs, analog clear, snapshots isolation |
| `report.rs` | 11 | byte position for all button groups (face/shoulder/system/Edge), all-button roundtrip, byte11 preservation, stick/trigger values, seq |
| `config.rs` | 42 | valid sources/targets, trigger/stick targets, combo validation (key/output/duplicate/mutex/FN+face), macro validation (empty seq, release>press, name conflict, turbo+macro mutex, combo→macro), block→blocked_buttons, turbo+block allowed, uppercase rejection, default config parse |
| `device.rs` | 1 | sysfs path resolution |

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
| 20 | Trigger self-map lost analog | `apply()` unconditionally cleared L2/R2 analog on all targets | `8ad808b` |
| 21 | Trigger swap (L2→R2) lost analog | Self-map fix only handled self, not cross-trigger | `9aa83b8` |
| 22 | Turbo source button leaked through | `to_mapping_config()` skips turbo buttons → no RemapRule to suppress source | `ef20805` |
| 23 | Turbo target not maintained between frames | Only applied on toggle event, not every frame | `ef20805` |
| 24 | Turbo on L2/R2 source leaked analog | Source suppression only cleared digital bit, not analog | `ef20805` |
| 25 | `remap="none"` semantics confused with missing remap | `unwrap_or("none")` treated missing remap as explicit block | `9ad6dbb` |
| 26 | `[cross] turbo=true` (no remap) silently skipped | `build_turbo_configs()` skipped `None` remap instead of self-targeting | `9ad6dbb` |
| 27 | SIGHUP kills process when device not connected | `setup_reload_handler()` registered inside device loop; default handler terminated process | `6616c24` |
| 28 | `turbo=true remap="combo"` combo configs skipped | `to_mapping_config()` turbo `continue` jumped over combo config building | `3394651` |
| 29 | `turbo=true remap="combo"` turbo runtime not built | `build_turbo_configs()` `resolve_target("combo")` returned None | `3394651` |
| 30 | Combo→macro reset every frame, stuck at 0ms | L2 macro detection (read L1, trigger suppressed by combo) deactivated macro; combo injection simultaneously activated it | `5053099` (MacroSource::Combo) |
| 31 | Macro output only on first frame, then invisible | `tick()` only wrote button on press event; subsequent frames state was recreated from physical buf, no maintain | `5053099` (per-frame maintain) |

## TODO

### High Priority
- [x] **Dead code cleanup**: 25 compiler warnings — done in v0.0.8
- [x] **Pipeline refactor**: three-layer architecture (L1→L2→L3) — done in v0.0.9
- [x] **Combo**: modifier key combinations — done in v0.0.10
- [x] **Macro**: timed key sequences — done in v0.0.11

### Medium Priority
- [ ] **D-Bus interface**: zbus session bus server for runtime control, edgemap CLI client

### Low Priority
- [ ] **Trigger source mapping**: analog threshold-based (half-press vs full-press events)
- [ ] **Touchpad 4-zone**: expand from left/right to quadrant grid
- [ ] **Regular DualSense support**: re-enable PID 0x0CE6

## Commit History

```
5053099 feat: macro (timed key sequences, hold & single modes)         [v0.0.11]
ec8d6ea perf: batch ACL restore via stdin (1 setfacl instead of N)
3394651 fix: turbo+combo coexistence (build combos for turbo buttons)  [v0.0.10]
cbfd298 feat: combo (L1 detect+suppress + L2 inject), 56 tests
53461dd docs: update STATUS.md for v0.0.9
45f5dc7 refactor: v0.0.9 three-layer pipeline (L1→L2→L3)             [v0.0.9]
83e2460 chore: rename StickDir variants to UpperCamelCase             [v0.0.8]
2ff84c9 chore: clean up compiler warnings (30→0), add AGENTS.md
b413c6c update agents.md
6616c24 fix: install SIGHUP handler at startup, not inside device loop  [v0.0.7]
6acf69e docs: update STATUS.md for v0.0.6, turbo, hot reload
9ad6dbb fix: rename none→block, fix self-turbo for missing remap
ef20805 feat: turbo (hold-to-repeat) with configurable interval/delay
9aa83b8 fix: transfer analog values on trigger swap (L2<->R2)        [v0.0.6]
8ad808b fix: preserve analog values on trigger self-map
6da6b0a feat: SIGHUP hot reload without UHID restart
40fcbe4 docs: update STATUS.md with full project overview            [v0.0.5]
48739ae feat: warn when button sections are missing from config
81b330d feat: touchpad split mode (left/right partition)
d9f54b7 test: add 41 unit tests covering mapping, report, config    [v0.0.4]
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
