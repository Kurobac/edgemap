# AGENTS.md — edgemap

DualSense UHID proxy project. Two binaries: `dseuhid` (UHID proxy daemon) and `edgemap` (user-side CLI). Userspace proxy: physical hidraw → `SourceCodec` → `ControllerFrame` → three-layer pipeline (L1 filter → L2 generate → L3 output) → `TargetCodec` → UHID virtual device. Output reports flow back through `TargetCodec` → `OutputCommand` → `PhysicalCodec` → physical hidraw. No async runtime, single epoll loop.

## Commands

```bash
cargo build               # 0 warnings (binaries: dseuhid + edgemap)
cargo test                # 211 tests total (121 dseuhid + 90 edgemap)
cargo run -- version
cargo run -- help
cargo run --bin edgemap -- help  # edgemap CLI help
python3 edgemap-gui-v6.py        # config editor GUI (PyQt6)
QT_QPA_PLATFORM=offscreen python3 -m unittest discover -s tests -p 'test_gui.py' -v  # 21 GUI tests
```

GitHub CI runs `cargo build`, `cargo test`, and the PyQt6 offscreen GUI test suite. No enforced lint or typecheck step exists.

Systemd units: `dseuhid.service` (system), `edgemap.service` (user).

```bash
makepkg -si              # build + install via PKGBUILD
```

## important notice
- 在 commit 间切换测试时，始终 `cargo clean` 后再 `cargo build`，避免增量编译缓存污染结果（见 BUGFIX #64）。
- 使用中文。技术名词可以使用英语。
- 本项目所有重点均在HIDRAW，以及原生支持dualsense的游戏。

## Architecture

| File | Role |
|------|------|
| `src/main.rs` | Entry: subcommand dispatch → daemon lock/control socket setup → device detection → config load → UHID create → proxy loop → reconnect |
| `src/proxy.rs` | **Three-layer pipeline** (L1→L2→L3), epoll loop (hidraw + UHID + control socket), turbo/combo/macro runtimes, transactional reload |
| `src/control.rs` | `flock`-based single-instance guard plus versioned Unix `SOCK_SEQPACKET` server/client protocol, acknowledged commands, and `uhid_ready`/`needs_config` state snapshots |
| `src/codec.rs` | Source/physical/target codec boundary: `SourceCodec`, `ControllerFrame`, `TargetCodec`, `PhysicalCodec`, feature report cache policy, output command wrappers |
| `src/mapping.rs` | `MappingConfig` (remap rules, blocked_buttons, combo/macro configs), `Target` enum, `apply()` two-phase remap with snapshot isolation |
| `src/config.rs` | TOML parse → `validate()` (37 rules) → `to_mapping_config()`. Combo, macro, turbo, remap all fully wired. `default_content()` for auto-created config. |
| `src/report.rs` | DS5/DS4 USB wire-format helpers, `GamepadState`, `Button` enum. Not a transport-neutral HID model. |
| `src/device.rs` | libudev-based initialized hidraw enumeration and hidraw/input hotplug monitoring for DSE Edge (0DF2) and regular DS (0CE6), USB or Bluetooth source. Waits for associated event/js nodes to finish udev initialization before hiding them, and skips UHID virtuals. Reads HID report descriptor from physical device via HIDIOCGRDESC; read failure aborts open. Node hiding via `chmod 000` + batch `setfacl --restore=-`. |
| `src/uhid.rs` | Raw UHID wrapper (create2, input2, get/set report reply), complete-write checks, UHID event size validation |
| `src/keyboard.rs` | uinput keyboard device: `KeyboardDevice` (open/create, press/release, flush_held, Drop destroy), 107 keycode constants, `resolve_keycode()` name→code mapping |
| `src/descriptor.rs` | Built-in target HID descriptors: `DS_USB_DESCRIPTOR`, `DS_EDGE_USB_DESCRIPTOR` (BT Edge auto target identity), and `DS4_USB_DESCRIPTOR` (used when `output_device = "dualshock4"`) |
| `src/shutdown.rs` | signalfd-based SIGINT/SIGTERM handling shared by both daemons; poll/epoll integration, interruptible retry delays, and child-process signal-mask reset |
| `src/bin/edgemap.rs` | User-side CLI (`validate`, `create-config`, `reload`, `switch-config`), no root. Daemon mode (`d`/`daemon`): auto-create configs under `$XDG_CONFIG_HOME/edgemap` (default `~/.config/edgemap`), profile auto-switch by 3-second process snapshots, persistent control socket state subscription, inotify/signalfd monitoring, and `notify-send` desktop notifications. |
| `edgemap-gui-v6.py` | PyQt6 config editor: two-column layout, button remap, turbo, combo/macro popup editors, macro manager, profile quick-switch, toolbar with KDE-native icons. |

## Three-layer pipeline (L1 → L2 → L3)

Input order inside `handle_hidraw_input()`:

**Source codec**
1. `SourceCodec::decode_input()` decodes hidraw bytes into `ControllerFrame`
2. Bad or short source frames are dropped; never raw-forward unknown report bytes

**L1: Physical Input Filtering**
1. **Turbo** — reads `physical_snapshot`, suppresses source button, toggles source for L2 remap
2. **Combo detection** — reads pre-suppress clone, suppresses modifier+key (including analog for L2/R2)
3. **Block** — `blocked_buttons` suppression (including analog for L2/R2)
4. **Freeze** — clone state as `l1` (immutable reference for L2)

**L2: Virtual Input Generation**
1. **Macro detection** — reads L1 (only `MacroSource::Physical`), triggers on edge
2. **Remap** — `apply(&l1, &mut state, &mut keyboard_events)` — two-phase, snapshot isolation, clears source then sets targets (gamepad or keyboard)
3. **Combo injection** — applies active combo outputs (after remap to prevent source-clear wipe, bugfix #33), pushes keyboard events
4. **Macro injection** — active macro steps write buttons/keyboard events every frame
5. **Keyboard flush** — unified press/release of all keyboard events across L2 sources (#75)

**L3: Output**
- `TargetCodec::encode_input()` writes the selected virtual target report → `UHID_INPUT2`

**Physical output path**
- UHID OUTPUT: `TargetCodec::decode_output()` → `OutputCommand` → `PhysicalCodec::encode_output()` → physical hidraw write
- UHID SET_REPORT: `PhysicalCodec::encode_set_report()` decides whether a target feature report can be forwarded to physical hidraw

## Codec boundaries

- `SourceCodec` owns physical input report size and byte-format decoding.
- `ControllerFrame` carries `GamepadState`, optional motion data, and source report backing.
- `TargetCodec` owns virtual input encoding, target output decoding, USB identity, and target GET_REPORT seed/fallback behavior.
- `PhysicalCodec` owns physical output encoding, SET_REPORT forwarding policy, and which physical feature reports are safe to cache.
- DS5 USB target keeps the DS5 USB source report as backing where possible. DS4 target converts input/output through DS4-specific USB report code.
- DS5/DS4 USB byte layout helpers in `report.rs` must not be reused for Bluetooth layouts; add BT-specific codecs first.

## Error handling policy

- Bad/short input frames: drop the frame. Do not raw-forward unknown source bytes to the virtual target.
- Single output or feature-report failures: reply with an error or drop that request while keeping the input path running.
- hidraw disconnect errors (`EIO`, `ENODEV`, `ENXIO`): stop the current proxy and wait for reconnect.
- Malformed UHID events, UHID read errors, and UHID input write errors: stop the current proxy loop.
- Startup config and required daemon setup failures are fatal. Hot reload failures keep the previous live config.
- Permission hiding/restoration failures are warning-level unless a direct node restriction call returns an error to the caller.

## Quirks

- **Root required** for daemon — `cargo test`, `cargo build`, subcommands work without root.
- **Config**: no default path. `-c`/`--config-path` optional — if omitted, starts in passthrough mode. edgemap is the intended way to manage config.
- **Hot reload**: `edgemap reload` sends an acknowledged control request. dseuhid re-reads the current live config and replies success only after load/validate/build commits; failure keeps the previous mapping, runtimes, and path.
- **Control socket**: `/run/dseuhid/control.sock` is a mode-0666 Unix `SOCK_SEQPACKET` endpoint. Versioned request/reply packets carry `reload` and `switch-config`; hello/state packets carry `uhid_ready` and `needs_config`. `/run/dseuhid/daemon.lock` uses `flock` for atomic single-instance ownership and contains the PID only for diagnostics.
- **edgemap daemon**: auto-creates `edgemap.toml` + `default.toml` under `$XDG_CONFIG_HOME/edgemap` (default `~/.config/edgemap`) on first run. Profiles in `[profiles.*]` sections with `match_process` (comm exact) and/or `match_cmdline` (substring), first match in declaration order wins. Each 3-second profile scan reads each PID's required `comm`/`cmdline` data at most once. A persistent control connection reports dseuhid lifetime and UHID/config state; inotify watches `edgemap.toml` and socket recreation. Only `needs_config=true`, an edgemap.toml reload, or a genuinely changed profile decision makes the daemon re-inject; manual config switches otherwise remain active until the daemon chooses a different profile. Sends notifications only after acknowledged switches.
- **Byte 10 high nibble** = DSE Edge buttons: FnLeft=0x10, FnRight=0x20, LeftPaddle=0x40, RightPaddle=0x80. Byte 11 low nibble must be preserved, high nibble zeroed.
- **Two-phase apply** (`mapping.rs`): Phase 1 clears source + collects targets, Phase 2 sets all targets atomically. Prevents cross-map (A→B, B→A) collisions.
- **Snapshot isolation**: `apply()` clones state before rules evaluate — rules read snapshot, write to live state. Prevents rule ordering artifacts.
- **DSE buttons excluded from targets** — only standard buttons, stick dirs, and trigger-full are valid targets. Edge buttons (paddles, Fn) can only be sources.
- **Device detection** skips virtual UHID devices (checks `/sys/class/hidraw/N/device/uevent` for `DRIVER=uhid`) to avoid recursively proxying itself.
- **GET_REPORT cache**: physical codec policy reads physical feature reports 0x05 (IMU calibration) and 0x20 (firmware info) for DS5 USB/BT physical devices backing DS5 USB targets. BT feature reports are CRC-validated with seed 0xA3 and kept full-size in the USB target cache. Read/validation failures warn and fall back to target responses. Report 0x09 (MAC address) is intentionally skipped — caching it would duplicate the physical device's MAC in sysfs, causing `hid-playstation` probe failure (#63).
- **Bluetooth source**: DS5/Edge BT input report 0x31 is decoded into `ControllerFrame` with USB-compatible backing. Virtual targets remain USB UHID only. BT physical main output is supported by wrapping DS5 USB target output into the DS5 BT 0x31 output envelope with sequence tag and CRC; BT SET_REPORT/vendor feature-report forwarding is intentionally unsupported for now. Genshin Impact sends feature report 0x08, but hardware rejected a naive HIDIOCSFEATURE transfer even with a feature CRC tail. For BT source → DS5 USB target gyro cadence issues, dseuhid normalizes UHID input cadence by repeating the latest UHID input at 1000Hz by default; repeat frames advance `raw[7]` while keeping the sensor timestamp unchanged until a real BT frame arrives. A 250Hz retest also avoids the original severe drift, so stable cadence appears more important than raw rate. `DSEUHID_BT_DS5_USB_REPEAT_HZ` overrides the DS5 target rate, and `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` restores the original one-physical-frame-to-one-UHID-frame behavior. BT source → DS4 USB target repeat is opt-in only via `DSEUHID_BT_DS4_USB_REPEAT_HZ`; it advances DS4 sequence fields without DS5 timestamp handling.
- **`Target::Block` removed** — replaced by `MappingConfig.blocked_buttons` (L1 suppression, not L2 remap). `remap="block"` in config maps to this.
- **`apply()` signature**: `apply(&self, l1: &GamepadState, state: &mut GamepadState, keyboard_out: &mut Vec<(u16, bool)>)` — reads frozen L1 output, writes to mutable state, pushes keyboard events.
- **Combo injection never clears** — only pushes activation, not deactivation. State re-parse handles cleanup naturally.
- **Passthrough mode**: `remap = "passthrough"` leaves a button untouched with no L2 processing. This is a mapping-layer passthrough, not a promise that every physical button can be represented by every virtual target. Edge paddle/FN passthrough is meaningful for Edge auto target; DS4 target drops those buttons, and forced DS5 writes the bits but visibility depends on target descriptor/driver behavior.
- **Suspend/resume**: daemon fd survives, but udev resets hidraw node permissions. `re_restrict_self()` called at top of `handle_hidraw_input` checks mode & 0o777 and re-applies `chmod 000` on first post-resume input packet (#70, #71).
- **ACL restore**: each hidden node is checked independently. If logind/udev reapplies a non-zero mode, `re_restrict_self()` snapshots the new mode/ACL before hiding it again; shutdown restores the latest snapshot (#91).
- **Kernel compatibility**: tested Linux 7.0, should work 6.7+, may work 5.12+. Requires UHID + `hid-playstation` driver.
- **Target/builtin lists**: Python `TARGETS` and `builtin` must stay in sync with Rust `is_valid_target()` / `resolve_target()` / macro-name validation. Cross-reference comments added at each list.

## Key constraints

- DualSense Edge (0DF2) + regular DualSense (0CE6), USB or Bluetooth source hidraw. Virtual target is still USB UHID only; no Bluetooth target.
- Bluetooth physical SET_REPORT / vendor feature-report forwarding is not implemented. Current BT support covers input, main output report forwarding, and GET_REPORT cache for 0x05/0x20.
- `-c` config path resets to passthrough on device reconnect; edgemap is the recommended way to set config.
- `output_device = "dualsense"` in config TOML: virtualize as regular DS (0x0CE6 PID + DS descriptor). Reload triggers UHID recreate.
- `output_device = "dualshock4"` in config TOML: virtualize as DS4 (0x09CC PID + DS4 descriptor, Beta). Native DS4 games under Proton may need the DS4 UHID MI_03 identity patch.
- Edge-only sources (paddles/Fn) are valid sources but not valid targets. If an Edge-only source is passthrough with a non-Edge target, warn that it may be ignored; users should remap it to a standard button or keyboard key.
- Config `[button_name]` sections are case-sensitive lowercase. Unknown button names → validation error.
- `remap="block"` disables a button entirely (L1 suppression).
- Combo format: `[modifier] remap="combo"` + `[[modifier.combos]]` entries. Modifier+key held simultaneously → inject output.
- Macro format: `[button] remap="macro_name"` + `[macros.macro_name]` with `sequence = [...]` and optional `mode = "hold"`/`"single"`. Combo output can be a macro name (`Target::Macro(String)`, `MacroSource::Combo`).
- Macro names must not shadow built-in targets (e.g. `l2_full` conflicts with `TriggerFull(L2)`).
- **Keyboard target format**: `key:<keyname>` (e.g. `key:space`, `key:a`, `key:enter`). 107 keycodes supported. Valid in remap targets, combo outputs, and macro step keys. Validation deferred to TOML save time.
- `StepTarget` enum: `Gamepad(Button)` or `Keyboard(u16)` — used in macro steps. Resolved by `resolve_step_target()` in config.rs.
- Known HID limitation: d-pad hat switch cannot encode 3+ simultaneous directions.
- edgemap profile format: `[profiles.<name>]` with `config = "<path>"` (relative to the XDG config directory, `~`, or absolute) + `match_process` / `match_cmdline` (case-insensitive, both optional; AND logic if both set). Profiles matched in TOML declaration order.

## Release

Tarball layout (GitHub Actions builds on tag):

```
edgemap-v1.1.0-x86_64.tar.gz
├── install.sh                 # sudo ./install.sh
├── dseuhid                    → /usr/local/bin/
├── edgemap                    → /usr/local/bin/
├── edgemap-gui                → /usr/local/bin/
├── usr/lib/systemd/
│   ├── system/dseuhid.service
│   └── user/edgemap.service
└── usr/share/
    ├── applications/edgemap.desktop
    ├── icons/hicolor/scalable/apps/edgemap.svg
    └── zsh/site-functions/{_dseuhid,_edgemap}
```

CI builds service files with `/usr/local/bin/` paths (`sed` at packaging time). AUR PKGBUILD uses `/usr/bin/` (repo defaults).

## Future plans

(None currently)

## Migration reference

Combo, macro, and turbo were originally in an InputPlumber-based companion at `/home/kurobac/Projects/ds/companion/`. All three features have been ported to dseuhid's direct-in-report approach (no D-Bus). The old companion is only useful for historical reference.
