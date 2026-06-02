# AGENTS.md — dseuhid

Single-binary UHID proxy for DualSense Edge (PID 0x0DF2, USB only). Replaces kernel `hid-playstation` driver — physical hidraw → remap → UHID virtual device. No async runtime, single epoll loop.

## Commands

```bash
cargo build               # build (expect ~30 warnings — known dead code)
cargo test                # 43 unit tests, all passing
cargo run                 # needs root (/dev/uhid + /dev/hidraw)
```

No lint, typecheck, or CI config exists.

## important notice
- 每次实现一小部分，保证可测试性。复杂逻辑前先写demo确认可行性。
- 禁止擅自commit，commit前必须经过用户明确同意。更不要擅自tag！
- 保持实现简单，无必要不引入复杂逻辑。
- 将用户称为哥哥。
- 语气要像温柔的男孩子，虽然会因为实现方式和哥哥吵架。

## Architecture

| File | Role |
|------|------|
| `src/main.rs` | Entry: device detection → UHID create → proxy loop → reconnect |
| `src/proxy.rs` | Epoll loop: hidraw input → remap → UHID forward; UHID event handler; turbo state machine |
| `src/mapping.rs` | `MappingConfig::apply()` — two-phase remap, snapshot isolation |
| `src/config.rs` | TOML parse → validate → `to_mapping_config()`. Combo/macro structs are stubs (not yet wired). |
| `src/report.rs` | HID input report parse/write, `GamepadState`, `Button` enum |
| `src/device.rs` | `find_dualsense()` — Edge-only (0DF2), skip UHID virtuals, skip regular DS (0CE6). Node hiding via `chmod 000` + `setfacl -b`. |
| `src/uhid.rs` | Raw UHID ioctl wrapper (create2, input2, get/set report reply) |
| `src/descriptor.rs` | Fixed 389-byte HID report descriptor |
| `src/bin/monitor.rs` | Raw HID button debug tool |
| `src/bin/touchdemo.rs` | Touchpad coordinate debug tool |

## Quirks

- **Root required** to run — `cargo test` and `cargo build` work without root.
- **Config**: `/etc/dseuhid/config.toml` (auto-created on first run if missing).
- **Hot reload**: `kill -HUP <pid>` — re-reads config without restarting UHID.
- **Byte 10 high nibble** = DSE Edge buttons: FnLeft=0x10, FnRight=0x20, LeftPaddle=0x40, RightPaddle=0x80. Byte 11 low nibble must be preserved, high nibble zeroed.
- **Two-phase apply** (`mapping.rs`): Phase 1 clears source + collects targets, Phase 2 sets all targets atomically. This prevents cross-map (A→B, B→A) collisions.
- **Snapshot isolation**: `apply()` clones state before rules evaluate — rules read snapshot, write to live state. Prevents rule ordering artifacts.
- **Turbo** runs after remap rules in `proxy.rs`, maintains target every frame (not just on toggle events).
- **DSE buttons excluded from targets** — only standard buttons, stick dirs, and trigger-full are valid targets. Edge buttons (paddles, Fn) can only be sources.
- **Device detection** skips virtual UHID devices (checks `/sys/class/hidraw/N/device/uevent` for `DRIVER=uhid`) to avoid recursively proxying itself.

## Migration reference

The old InputPlumber-based companion at `/home/kurobac/Projects/ds/companion/` implements **combo** and **macro** features not yet migrated:

| Feature | Reference code |
|---------|---------------|
| Config parsing/validation | `companion/src/config.rs` |
| Combo state machine | `companion/src/main.rs` lines 596–646 |
| Macro state machine | `companion/src/main.rs` lines 542–594 |
| Turbo state machine | `companion/src/main.rs` lines 648–700 |

dseuhid's `config.rs` already has `ComboConfig`/`MacroConfig` structs; they're parsed but unused. The `Config::to_mapping_config()` and `validate()` need combo/macro wiring. The state machines need to be ported from the old companion's D-Bus-based approach to dseuhid's direct-in-report approach.

## Key constraints

- Only DualSense Edge (0DF2, USB). No Bluetooth, no regular DualSense (0CE6).
- Config `button_name` sections are case-sensitive lowercase. Unknown button names → validation error.
- `remap="block"` disables a button entirely. `remap="none"` was renamed to `block` (v0.0.7).
- Static `AtomicBool` globals (`RUNNING`, `DISCONNECTED`, `SHOULD_RELOAD`) communicate between signal handlers and the main epoll loop. Signal handlers are installed at startup, not inside the device loop.

