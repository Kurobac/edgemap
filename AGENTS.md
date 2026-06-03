# AGENTS.md — dseuhid

Single-binary UHID proxy for DualSense Edge (PID 0x0DF2, USB only). Replaces kernel `hid-playstation` driver — physical hidraw → three-layer pipeline (L1 filter → L2 generate → L3 output) → UHID virtual device. No async runtime, single epoll loop.

## Commands

```bash
cargo build               # 0 warnings (binaries: dseuhid + edgemap)
cargo test                # 135 tests total (68 dseuhid + 67 edgemap shared-module imports)
cargo run                 # daemon, needs root (/dev/uhid + /dev/hidraw)
cargo run -- monitor      # raw HID button + stick debug (no root needed)
cargo run -- touchdemo    # touchpad coordinate debug (no root needed)
cargo run -- version
cargo run -- help
cargo run --bin edgemap -- help  # edgemap CLI help
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
| `src/main.rs` | Entry: subcommand dispatch → FIFO setup/teardown → device detection → config load → UHID create → proxy loop → reconnect |
| `src/proxy.rs` | **Three-layer pipeline** (L1→L2→L3), epoll loop (hidraw + UHID + FIFO), turbo/combo/macro runtimes, FIFO command handler, reload |
| `src/mapping.rs` | `MappingConfig` (remap rules, blocked_buttons, combo/macro configs), `Target` enum, `apply()` two-phase remap with snapshot isolation |
| `src/config.rs` | TOML parse → `validate()` (37 rules) → `to_mapping_config()`. Combo, macro, turbo, remap all fully wired. `default_content()` for auto-created config. |
| `src/report.rs` | HID input report parse/write, `GamepadState`, `Button` enum |
| `src/device.rs` | `find_dualsense()` — Edge-only (0DF2), skip UHID virtuals, skip regular DS (0CE6). Node hiding via `chmod 000` + batch `setfacl --restore=-`. |
| `src/uhid.rs` | Raw UHID ioctl wrapper (create2, input2, get/set report reply) |
| `src/descriptor.rs` | Fixed 389-byte HID report descriptor |
| `src/monitor.rs` | `dseuhid monitor` — raw HID button + analog stick debug (threshold 5, 80ms throttle, reads physical hidraw directly) |
| `src/touchdemo.rs` | `dseuhid touchdemo` — touchpad coordinate debug + zone detection |
| `src/bin/edgemap.rs` | User-side CLI (`validate`, `create-config`, `reload`, `switch-config`), no root, communicates via FIFO |

## Three-layer pipeline (L1 → L2 → L3)

Order inside `handle_hidraw_input()`:

**L1: Physical Input Filtering**
1. **Turbo** — reads `physical_snapshot`, suppresses source button, toggles target every frame
2. **Combo detection** — reads pre-suppress clone, suppresses modifier+key (including analog for L2/R2)
3. **Block** — `blocked_buttons` suppression (including analog for L2/R2)
4. **Freeze** — clone state as `l1` (immutable reference for L2)

**L2: Virtual Input Generation**
1. **Macro detection** — reads L1 (only `MacroSource::Physical`), triggers on edge
2. **Remap** — `apply(&l1, &mut state)` — two-phase, snapshot isolation, clears source then sets targets
3. **Combo injection** — applies active combo outputs (after remap to prevent source-clear wipe, bugfix #33)
4. **Macro injection** — active macro steps write buttons every frame

**L3: Output**
- `apply_state_to_report()` writes final state to HID report buffer → `UHID_INPUT2`

## Quirks

- **Root required** for daemon — `cargo test`, `cargo build`, subcommands work without root.
- **Config**: no default path. `-c`/`--config-path` optional — if omitted, starts in passthrough mode. edgemap is the intended way to manage config.
- **Hot reload**: `kill -HUP <pid>` or `echo reload > /run/dseuhid/control` — re-reads config, rebuilds all runtimes.
- **FIFO control**: `/run/dseuhid/control` (0666), PID at `/run/dseuhid/pid`. Commands: `reload`, `switch-config <path>`. Non-root users can write to it. FIFO fd is dup'd for safe reconnects.
- **Byte 10 high nibble** = DSE Edge buttons: FnLeft=0x10, FnRight=0x20, LeftPaddle=0x40, RightPaddle=0x80. Byte 11 low nibble must be preserved, high nibble zeroed.
- **Two-phase apply** (`mapping.rs`): Phase 1 clears source + collects targets, Phase 2 sets all targets atomically. Prevents cross-map (A→B, B→A) collisions.
- **Snapshot isolation**: `apply()` clones state before rules evaluate — rules read snapshot, write to live state. Prevents rule ordering artifacts.
- **DSE buttons excluded from targets** — only standard buttons, stick dirs, and trigger-full are valid targets. Edge buttons (paddles, Fn) can only be sources.
- **Device detection** skips virtual UHID devices (checks `/sys/class/hidraw/N/device/uevent` for `DRIVER=uhid`) to avoid recursively proxying itself.
- **`Target::Block` removed** — replaced by `MappingConfig.blocked_buttons` (L1 suppression, not L2 remap). `remap="block"` in config maps to this.
- **`apply()` signature**: `apply(&self, l1: &GamepadState, state: &mut GamepadState)` — reads frozen L1 output, writes to mutable state.
- **Combo injection never clears** — only pushes activation, not deactivation. State re-parse handles cleanup naturally.
- **Static `AtomicBool` globals** (`RUNNING`, `DISCONNECTED`, `SHOULD_RELOAD`) communicate between signal handlers and the main epoll loop. Signal handlers installed at startup, not inside device loop.

## Key constraints

- Only DualSense Edge (0DF2, USB). No Bluetooth, no regular DualSense (0CE6).
- Config `[button_name]` sections are case-sensitive lowercase. Unknown button names → validation error.
- `remap="block"` disables a button entirely (L1 suppression).
- Combo format: `[modifier] remap="combo"` + `[[modifier.combos]]` entries. Modifier+key held simultaneously → inject output.
- Macro format: `[button] remap="macro_name"` + `[macros.macro_name]` with `sequence = [...]` and optional `mode = "hold"`/`"single"`. Combo output can be a macro name (`Target::Macro(String)`, `MacroSource::Combo`).
- Macro names must not shadow built-in targets (e.g. `l2_full` conflicts with `TriggerFull(L2)`).
- Known HID limitation: d-pad hat switch cannot encode 3+ simultaneous directions.

## Migration reference

Combo, macro, and turbo were originally in an InputPlumber-based companion at `/home/kurobac/Projects/ds/companion/`. All three features have been ported to dseuhid's direct-in-report approach (no D-Bus). The old companion is only useful for historical reference.
