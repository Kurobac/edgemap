# Development reference — edgemap

This is the detailed, on-demand project reference. Read the sections relevant to the current task after consulting the compact repository instructions in `AGENTS.md`.

DualSense UHID proxy project. Two binaries: `dseuhid` (UHID proxy daemon) and `edgemap` (user-side CLI). Userspace proxy: physical hidraw → `SourceCodec` → `ControllerFrame` → three-layer pipeline (L1 filter → L2 generate → L3 output) → `TargetCodec` → UHID virtual device. Output reports flow back through `TargetCodec` → `OutputCommand` → `PhysicalCodec` → physical hidraw. No async runtime, single epoll loop.

## Commands

```bash
cargo build               # 0 warnings (binaries: dseuhid + edgemap)
cargo test                # 264 tests total (102 library + 113 dseuhid + 31 edgemap + 18 CLI integration)
cargo run -- version
cargo run -- help
cargo run --bin edgemap -- help  # edgemap CLI help
PYTHONPATH=gui python3 -m edgemap_gui  # config editor GUI from source (PyQt6)
QT_QPA_PLATFORM=offscreen python3 -m unittest discover -s tests -p 'test_gui.py' -v  # 44 GUI tests
```

GitHub CI pins third-party actions and runs `cargo fmt --check`, locked debug builds/tests, the fixed-path installer integration test on a disposable runner, `cargo clippy --locked --all-targets -- -D warnings`, release-tag validation, and the PyQt6 offscreen GUI suite under Python 3.11. Tagged releases repeat the Rust checks, installer test the release binaries that will be staged, verify the tag matches `Cargo.toml`, and publish only after the GUI job succeeds.

Systemd units: `dseuhid.service` (system), `edgemap.service` (user).

Installation, packaging, and system-level verification are documented separately in `docs/INSTALLATION_TESTING.md` and are outside the ordinary build/test workflow.

## important notice
- This project is focused on games which supports sony controller natively.
- PKGBUILD in root directory is for local testing only. It is a vcs package. No need to update pkgver.

## Architecture

| File | Role |
|------|------|
| `src/lib.rs` | Shared package library used by both binaries. Exposes capabilities, config, control, keycodes, mapping, model, and shutdown modules; it is not a promised third-party SDK. |
| `src/main.rs` | Thin dseuhid entry: CLI dispatch, logging/startup checks, and final handoff to `daemon::run()`. |
| `src/daemon.rs` | Process-level daemon lifecycle: shutdown signal, daemon lock/control server, active config, controller reconnect loop, and UHID recreation decisions. |
| `src/session.rs` | Builds one physical-controller session: hidraw, codecs, feature cache, UHID, keyboard, mapping, and Proxy. Returns typed session outcomes to the outer daemon loop. |
| `src/proxy/` | epoll ownership and **three-layer pipeline** (L1→L2→L3). `pipeline.rs` performs fd-free source/timer transforms and output-intent reduction, `runtime.rs` owns turbo/combo/macro timing, `repeat.rs` owns BT cadence/sequence, and `uhid_events.rs` handles OUTPUT/GET/SET. One monotonic one-shot timerfd is armed to the earliest runtime or repeat deadline. |
| `src/control/` | Versioned Unix `SOCK_SEQPACKET` protocol, complete packet transport, client/server, and `flock`-based named daemon locks. Public names are re-exported from `control/mod.rs`. |
| `src/codec/` | Source/physical/target codec boundary. Protocol-specific wire formats live in `ds5_usb.rs`, `ds5_bt.rs`, and `ds4_usb.rs`; `feature.rs` owns feature-cache policy and `types.rs` owns transport frame/command types. |
| `src/mapping.rs` | `MappingConfig` (remap rules, blocked_buttons, combo/macro configs), typed `Target`, and two-phase remap collection with snapshot isolation. `OutputIntent` unions digital/keyboard owners and reduces trigger contributions by maximum. |
| `src/model.rs` | Transport-neutral `Button`, `GamepadState`, and canonical button enumeration. Contains no USB/BT report offsets. |
| `src/keycodes.rs` | Canonical keyboard names, Linux keycodes, and `resolve_keycode()` shared by config validation, capabilities, and uinput setup. |
| `src/capabilities.rs` | Stable versioned TOML capability contract consumed by the GUI through `edgemap capabilities`. Publishes output devices, button/target sets, reserved macro names, and keyboard codes. |
| `src/config/` | `ActiveConfig` bounded regular-file snapshot (64 KiB), Serde schema, semantic validation, deterministic default TOML, target resolution, and compilation to `MappingConfig`. `config/mod.rs` keeps the public facade. |
| `src/device/` | libudev monitor/discovery, Sony device/transport identification, hidraw IO/ioctls, and device-node lifecycle. `HidrawDevice` composes the device-access guard and skips UHID virtuals. |
| `src/uhid.rs` | Raw UHID wrapper (create2, input2, get/set report reply), complete-write checks, UHID event size validation |
| `src/keyboard.rs` | uinput keyboard lifecycle and desired-state synchronization. It diffs the desired key set against successfully held keys, so failed presses/releases remain eligible for retry. Registered names and numeric codes come from shared `keycodes.rs`. |
| `src/descriptor.rs` | Built-in target HID descriptors: `DS_USB_DESCRIPTOR`, `DS_EDGE_USB_DESCRIPTOR` (BT Edge auto target identity), and `DS4_USB_DESCRIPTOR` (used when `output_device = "dualshock4"`) |
| `src/shutdown.rs` | signalfd-based SIGINT/SIGTERM handling shared by both daemons; poll/epoll integration, interruptible retry delays, and child-process signal-mask reset |
| `src/bin/edgemap/` | User CLI and daemon application. `cli.rs` preserves stdout/stderr/exit behavior, `control_session.rs` owns acknowledged requests/state drain, `paths.rs` owns XDG resolution, and `daemon/` owns monitoring, profiles, notifications, and the daemon state machine. |
| `gui/edgemap_gui/` | Maintainable PyQt6 source package: capability parsing, config document/snapshot, deterministic serializers, durable atomic saves (including parent-directory fsync), editor lifecycle, profile/combo/macro/keyboard dialogs, and Rust CLI integration. |
| `gui/edgemap-gui` | Small installation-prefix-aware Python 3.11+ launcher for the private Python package; no zipapp or generated GUI artifact is tracked. |
| `scripts/stage_release.sh` | Sole release-tree builder shared by CI and the tag workflow. Validates inputs, installs only source `.py` files plus the canonical `LICENSE`, rewrites service binary prefixes, and refuses to overwrite an existing output directory. |
| `install.sh` | Cwd-independent release installer/uninstaller; operational details live in `docs/INSTALLATION_TESTING.md`. |

## Three-layer pipeline (L1 → L2 → L3)

Input order inside `handle_hidraw_input()`:

**Source codec**
1. `SourceCodec::decode_input()` decodes hidraw bytes into `ControllerFrame`
2. Bad or short source frames are dropped; never raw-forward unknown report bytes

**L1: Physical Input Filtering**
1. **Touchpad split derivation** — when split mode is enabled, clear the parent press and derive exactly one left/right child from the decoded report before taking the physical snapshot
2. **Physical snapshot** — freeze the split-aware source state used by turbo and diagnostics
3. **Turbo** — reads `physical_snapshot`, suppresses its source, and advances/toggles the source for L2 processing
4. **Combo detection** — reads a post-turbo clone, suppresses modifier+key (including analog for L2/R2)
5. **Block** — `blocked_buttons` suppression (including analog for L2/R2)
6. **Freeze** — clone state as `l1` (immutable reference for L2)

**L2: Virtual Input Generation**
1. **Macro detection** — source-frame transforms read L1 and activate/deactivate only `MacroSource::Physical` runtimes
2. **Remap collection** — `collect(&l1, &mut state, &mut intent)` evaluates a frozen snapshot, clears sources, and contributes targets
3. **Combo injection** — active combo outputs contribute after remap so source clearing cannot erase them; combo macros activate only while observing a source frame
4. **Macro contribution** — active steps advance and contribute their current targets
5. **Intent reduction** — digital buttons and keyboard keys are set unions across remap/combo/macro owners; L2/R2 analog contributions use the maximum value. Stick writes remain ordered state mutations and are not ownership-reduced.
6. **Keyboard synchronization** — diff the complete desired key set against the successfully held set, preserving shared ownership and retrying failed transitions on later transforms

**L3: Output**
- `TargetCodec::encode_input()` writes the selected virtual target report → `UHID_INPUT2`

**Physical output path**
- UHID OUTPUT: `TargetCodec::decode_output()` → `OutputCommand` → `PhysicalCodec::encode_output()` → physical hidraw write
- UHID SET_REPORT: `PhysicalCodec::encode_set_report()` decides whether a target feature report can be forwarded to physical hidraw

**Timing path**
- Source transforms cache the latest decoded `ControllerFrame`. Timer transforms reuse that frame but do not observe fresh source edges, so deadlines cannot retrigger physical/combo macro activation.
- Turbo, macro transitions, and Bluetooth repeat expose their next deadlines. The epoll loop arms one `CLOCK_MONOTONIC` one-shot timerfd to the earliest deadline.
- Late wakeups advance runtime phase directly to `now` and schedule the first future deadline instead of replaying missed transitions. A timer turn emits at most one due target report; when BT repeat is active, runtime changes update its cached report and remain on the repeat cadence.

## Codec boundaries

- `SourceCodec` owns physical input report size and byte-format decoding.
- `ControllerFrame` carries `GamepadState`, optional motion data, and source report backing.
- `TargetCodec` owns virtual input encoding, target output decoding, USB identity, and target GET_REPORT seed/fallback behavior.
- `PhysicalCodec` owns physical output encoding, SET_REPORT forwarding policy, and which physical feature reports are safe to cache.
- DS5 USB target keeps the DS5 USB source report as backing where possible. DS4 target converts input/output through DS4-specific USB report code.
- DS5/DS4 USB byte layout helpers in `src/codec/ds5_usb.rs` and `src/codec/ds4_usb.rs` must not be reused for Bluetooth layouts; Bluetooth envelopes and CRC handling belong in `src/codec/ds5_bt.rs`.

## Bluetooth haptics

- `OutputCommand::Haptics(HapticsFrame)` is the PCM entry into the physical codec:
  32 interleaved stereo sample frames, signed 8-bit, 3 kHz (64 bytes per block).
  USB physical HID rejects this command; USB audio is a separate transport.
- `ds5_bt.rs` encodes the SAxense 142-byte `0x32` container with control `0x11`
  and PCM `0x12` sub-packets. The control payload is `FE 00 00 00 00 40 counter`,
  keeping microphone streaming disabled and setting the controller audio buffer
  to 64 instead of SAxense's maximum 255. The audio counter advances per PCM
  packet; the outer four-bit sequence is shared with ordinary `0x31` output.
  Both report types use the existing `0xA2`-seed CRC.
- `DSEUHID_BT_HAPTICS_BUFFER` on `dseuhid` overrides the PCM-only `0x32`
  controller buffer (decimal 1–255, default 64; not milliseconds). Invalid values
  fail daemon startup. Each BT session logs the selected value; restart the proxy
  after changing the environment. The combined speaker report keeps its buffer
  value of 64. A value of 32 produced intermittent missed short haptics during
  repeated Genshin activity-page switching; 64 was more stable in that test.
  Example: `sudo env DSEUHID_BT_HAPTICS_BUFFER=32 ./target/debug/dseuhid`.
- `proxy/output.rs` owns the shared physical write/error path. Ordinary game
  output retains its flags. No rumble/PCM priority policy is applied.
  Session teardown sends silence if still connected; disconnect discards queued
  audio. Individual output failures stop live playback without stopping input.
- `edgemap daemon` creates the PipeWire sink `edgemap.dualsense` when the control
  state reports `uhid_ready=1` with a `bt_haptics` model, and destroys it on disconnect or daemon
  shutdown. A UHID session recreation also recreates the PCM endpoint and sink.
  USB sessions and DS4 output mode do not create a BT PCM endpoint or virtual
  audio sink. Switching to DS4 tears down the audio session; switching back to
  auto or DualSense recreates it for a Bluetooth source.
  Every state transition is observed, including
  disconnect/reconnect notifications drained together. Audio identity follows the
  virtual DS5 target: auto preserves DualSense/Edge, forced DualSense uses 0x0ce6.
  A model change recreates the sink under the same stable name.
  If the connected PCM receiver closes before the control notification arrives,
  Unix datagram `ECONNREFUSED` ends the old capture normally. It does not retry
  or attach the old worker to a new session; control state owns the next start.
- `daemon/audio.rs` runs `pw-cat` in the user's PipeWire session as an Audio/Sink:
  interleaved F32LE, 48 kHz, FL/FR/RL/RR. A 255-tap Hamming-windowed sinc low-pass
  (1250 Hz cutoff, about 2.65 ms group delay) precedes 16:1 decimation of RL/RR to
  signed 8-bit stereo at 3 kHz. FL/FR are normally discarded. For the opt-in
  speaker demo (`EDGEMAP_SPEAKER_DEMO=1` on `edgemap daemon`), `audio/speaker.rs`
  resamples each 512-frame front block to 480 stereo frames (16:15 linear
  interpolation) and uses system libopus: 48 kHz, AUDIO, 160 kbit/s CBR, 200 bytes.
  Both paths therefore share the 10.667 ms physical clock. Eight encoded silent
  blocks flush the speaker after activity, then stop; rear-only input never starts
  Opus playback. The capture worker owns no physical HID fd.
- The sink publishes `device.bus=usb`, Sony VID 0x054c, model-specific PID
  (0x0ce6/0x0df2), manufacturer/product descriptions, nicknames, `device.class=sound`
  and `device.form-factor=controller`. PipeWire maps the last key to PulseAudio's
  `device.form_factor`. It remains a virtual sink, with session priority zero.
  Existing Wine can use bus/VID/PID to construct a USB-shaped audio device path;
  these properties do not create a sysfs USB parent or supply a ContainerId.
- `control/haptics.rs` carries one 64-byte PCM block plus a little-endian u64
  CLOCK_MONOTONIC timestamp over `/run/dseuhid/haptics.sock` (Unix datagram, 0666).
  The proxy owns this socket for the Bluetooth session, drains at most 16 datagrams
  per turn, and rejects malformed, future-dated, or more than 100 ms old blocks.
  A nonblocking sender drops a block if the socket is full. Speaker demo packets
  append 200 Opus bytes (272 bytes total); PCM-only packets retain the 72-byte
  format. The receiver accepts exactly these two sizes and queues both channels
  together, so dropping a late block does not separate their timelines.
- Live PCM shares the existing timerfd and physical output sequence. The queue
  retains at most three blocks, starts with one block of buffering, and emits at
  most one block per due tick. Late ticks discard missed blocks. Underrun sends a
  silent block; continuous zero input does not keep physical playback active.
  Live PCM does not select rumble/audio mode.
- The opt-in speaker path adds `OutputCommand::Audio`, encoded only on BT as
  a single 398-byte `0x36` report: control `0x91` (FE, five buffer lengths of 64,
  one counter), state `0x90` (63 bytes), PCM `0x92` (64 bytes), and Opus `0x93`
  (200 bytes). This follows the combined carrier in DS5Dongle, mdrv-ds and
  LinuxAudio4Dualsense5. Microphone streaming stays disabled. Both outer sequence
  and audio counter advance once per combined block and remain continuous when
  switching to/from PCM-only `0x32` output.
- An earlier demo interleaved standalone `0x32` and `0x35` reports with different
  buffer declarations/counter positions. On Edge, PCM-only sections vibrated but
  adding Opus silenced both lanes. An HCI capture confirmed valid CRCs, nonzero
  PCM and decodable, non-silent Opus; standalone coexistence was not established.
  Combined playback replaces that path and was confirmed working on Edge:
  speaker-only, haptics-only, and simultaneous speaker/haptics demo segments.
  Native game speaker behavior and game-controlled audio settings remain untested.
- The first Opus block starts with a minimal `0x31` audio-mode update; the same
  audio state is carried in every `0x36`. The demo uses native speaker audio
  control 0x09, volume 100, preamp 0x0A, and power-save control 0x10 (microphone
  muted, output paths powered). Only audio-related valid bits are set; LEDs,
  triggers and microphone volume remain untouched. End/underrun/session teardown
  mutes speaker volume; disconnect skips writes. This is explicit demo setup,
  not game volume arbitration. Plain PCM retains its previous behavior. libopus
  is loaded with `dlopen` only when speaker capture starts, and remains loaded
  until its encoder is destroyed. It is optional for startup and HD haptics and
  is not a link-time dependency. Missing libraries/symbols explicitly fail the
  speaker request; they do not silently select a PCM-only mode.
- Protocol v3 encodes `bt_haptics=none|dualsense|dualsense-edge` in hello/state
  packets, replacing the v2 boolean. Both daemons must be updated
  together. The control socket remains separate from binary PCM. `pw-cat` must be
  installed for capture; audio process/socket failures are logged. The input proxy
  continues operating when audio is unavailable. Audio demos live in
  [`scripts/haptics_audio_demo.py`](../scripts/haptics_audio_demo.py) and
  [`scripts/speaker_audio_demo.py`](../scripts/speaker_audio_demo.py). Native
  PipeWire integration tests are in [`daemon/audio.rs`](../src/bin/edgemap/daemon/audio.rs);
  they are ignored by default and require a live user PipeWire session plus
  `pw-cat` and `pactl`.

## Error handling policy

- Bad/short input frames: drop the frame. Do not raw-forward unknown source bytes to the virtual target.
- Single output or feature-report failures: reply with an error or drop that request while keeping the input path running.
- hidraw disconnect errors (`EIO`, `ENODEV`, `ENXIO`): stop the current proxy and wait for reconnect.
- Malformed UHID events, UHID read errors, and UHID input write errors: stop the current proxy loop.
- Startup config and required daemon setup failures are fatal. Runtime config-apply failures keep the previous live config.
- Device-node lifecycle failure behavior is documented in `docs/INSTALLATION_TESTING.md`.

## Quirks

- **Config**: no default path. `-c`/`--config-path` optional — if omitted, starts in passthrough mode. edgemap is the intended way to manage config.
- **Config switching**: `edgemap switch-config` reads and validates a configuration under the user account, then sends the source label and complete TOML content in one acknowledged seqpacket. dseuhid parses, validates, builds, and commits that in-memory content transactionally; it never opens the client-provided path. Failed applies preserve the previous mapping, runtimes, active content, and output-device setting.
- **Control socket**: `/run/dseuhid/control.sock` is a Unix `SOCK_SEQPACKET` endpoint with at most 16 active clients and one delivered request per event-loop turn. The versioned request protocol carries `switch-config`; hello/state packets carry `uhid_ready`, `needs_config`, and `bt_haptics`. Config failure replies expose only fixed category messages. `/run/dseuhid/daemon.lock` uses `flock` for atomic single-instance ownership and contains the PID only for diagnostics. Access details live in `docs/INSTALLATION_TESTING.md`.
- **Config file limits**: `-c`, edgemap CLI, validation, and profile selection accept only regular files no larger than 64 KiB. Files are opened nonblocking and reads are independently capped, rejecting FIFO/device nodes and preventing unbounded pseudo-file reads. Runtime socket content is capped to the same size.
- **edgemap daemon**: auto-creates `edgemap.toml` + `default.toml` under `$XDG_CONFIG_HOME/edgemap` (default `~/.config/edgemap`) on first run. Profiles in `[profiles.*]` sections with `match_process` (comm exact) and/or `match_cmdline` (substring), first match in TOML declaration order wins. Each 3-second profile scan reads each PID's required `comm`/`cmdline` data at most once. A persistent control connection reports dseuhid lifetime and UHID/config state; inotify watches `edgemap.toml` and socket recreation, while periodic/state-triggered resynchronization closes watch replacement races and recovers from queue overflow. Selected, effective, and failed config decisions advance only after acknowledged applies; an invalid selected profile may fall back to the validated base config without hiding the failure. Only `needs_config=true`, an edgemap.toml reload, or a genuinely changed profile decision makes the daemon re-inject; manual config switches otherwise remain active until the daemon chooses a different profile. Sends notifications only after acknowledged switches.
- **edgemap single instance**: daemon mode holds an exclusive `flock` on `$XDG_STATE_HOME/edgemap/edgemap.lock` (fallback `~/.local/state/edgemap/edgemap.lock`). The file contains the PID for diagnostics; process lifetime is determined only by the kernel lock.
- **Byte 10 high nibble** = DSE Edge buttons: FnLeft=0x10, FnRight=0x20, LeftPaddle=0x40, RightPaddle=0x80. Byte 11 low nibble must be preserved, high nibble zeroed.
- **Two-phase mapping** (`mapping.rs`): source rules are evaluated from one frozen snapshot while source clears and target intent are collected; reduced digital/trigger targets are applied afterward. Stick targets mutate the output state directly but still read the frozen rule snapshot. This prevents cross-map (A→B, B→A) collisions.
- **Snapshot isolation**: `collect()` clones L1 before rules evaluate — every rule condition reads the snapshot while writes go to output state/intent. This prevents rule-order activation artifacts.
- **DSE buttons excluded from targets** — only standard buttons, stick dirs, and trigger-full are valid targets. Edge buttons (paddles, Fn) can only be sources.
- **Device detection** skips virtual UHID devices (checks `/sys/class/hidraw/N/device/uevent` for `DRIVER=uhid`) to avoid recursively proxying itself.
- **GET_REPORT cache**: physical codec policy reads physical feature reports 0x05 (IMU calibration) and 0x20 (firmware info) for DS5 USB/BT physical devices backing DS5 USB targets. BT feature reports are CRC-validated with seed 0xA3 and kept full-size in the USB target cache. Read/validation failures warn and fall back to target responses. Report 0x09 (MAC address) is intentionally skipped — caching it would duplicate the physical device's MAC in sysfs, causing `hid-playstation` probe failure (#63).
- **Bluetooth source**: DS5/Edge BT input report 0x31 is decoded into `ControllerFrame` with USB-compatible backing. Virtual targets remain USB UHID only. BT physical main output is supported by wrapping DS5 USB target output into the DS5 BT 0x31 output envelope with sequence tag and CRC; BT SET_REPORT/vendor feature-report forwarding is intentionally unsupported for now. Genshin Impact sends feature report 0x08, but hardware rejected a naive HIDIOCSFEATURE transfer even with a feature CRC tail. For BT source → DS5 USB target gyro cadence issues, dseuhid normalizes UHID input cadence by repeating the latest UHID input at 1000Hz by default; repeat frames advance `raw[7]` while keeping the sensor timestamp unchanged until a real BT frame arrives. A 250Hz retest also avoids the original severe drift, so stable cadence appears more important than raw rate. `DSEUHID_BT_DS5_USB_REPEAT_HZ` overrides the DS5 target rate, and `DSEUHID_BT_DS5_USB_REPEAT_MODE=passthrough` restores the original one-physical-frame-to-one-UHID-frame behavior. BT source → DS4 USB target repeat is opt-in only via `DSEUHID_BT_DS4_USB_REPEAT_HZ`; it advances DS4 sequence fields without DS5 timestamp handling.
- **`Target::Block` removed** — replaced by `MappingConfig.blocked_buttons` (L1 suppression, not L2 remap). `remap="block"` in config maps to this.
- **Mapping collection**: the proxy uses `collect(&self, l1: &GamepadState, state: &mut GamepadState, intent: &mut OutputIntent)` so all L2 producers share one ownership-aware reduction. The public `apply()` helper retains its compatibility signature for direct mapping users and converts the collected keyboard owners into pressed events.
- **Combo injection is additive** — an active combo contributes its target but never clears another producer's target. Each transform recomputes the complete intent, so inactive combos disappear naturally while shared owners remain active.
- **Passthrough mode**: `remap = "passthrough"` leaves a button untouched with no L2 processing. This is a mapping-layer passthrough, not a promise that every physical button can be represented by every virtual target. Edge paddle/FN passthrough is meaningful for Edge auto target; DS4 target drops those buttons, and forced DS5 writes the bits but visibility depends on target descriptor/driver behavior.
- **Kernel compatibility**: tested Linux 7.0, should work 6.7+, may work 5.12+. Requires UHID + `hid-playstation` driver.
- **GUI capability source**: `edgemap capabilities` is the only source for GUI button, target, reserved macro-name, output-device, and keyboard lists. The GUI refuses to start if the versioned TOML contract cannot be queried or parsed; it must not carry stale fallback lists. Rust tests compare advertised source/gamepad sets with validator rules in both directions.

## Key constraints

- DualSense Edge (0DF2) + regular DualSense (0CE6), USB or Bluetooth source hidraw. Virtual target is still USB UHID only; no Bluetooth target.
- Bluetooth physical SET_REPORT / vendor feature-report forwarding is not implemented. Current BT support covers input, main output report forwarding, and GET_REPORT cache for 0x05/0x20.
- `-c` config path resets to passthrough on device reconnect; edgemap is the recommended way to set config.
- `output_device = "dualsense"` in config TOML: virtualize as regular DS (0x0CE6 PID + DS descriptor). Applying a changed output target triggers UHID recreation from the retained in-memory config.
- `output_device = "dualshock4"` in config TOML: virtualize as DS4 (0x09CC PID + DS4 descriptor, Beta). Native DS4 games under Proton may need the DS4 UHID MI_03 identity patch.
- Edge-only sources (paddles/Fn) are valid sources but not valid targets. If an Edge-only source is passthrough with a non-Edge target, warn that it may be ignored; users should remap it to a standard button or keyboard key.
- Config `[button_name]` sections are case-sensitive lowercase. Unknown button names → validation error.
- `remap="block"` disables a button entirely (L1 suppression).
- Combo format: `[modifier] remap="combo"` + `[[modifier.combos]]` entries. Modifier+key held simultaneously → inject output.
- Macro format: `[button] remap="macro_name"` + `[macros.macro_name]` with `sequence = [...]` and optional `mode = "hold"`/`"single"`. Combo output can be a macro name (`Target::Macro(String)`, `MacroSource::Combo`).
- Macro names must not shadow built-in targets (e.g. `l2_full` conflicts with `TriggerFull(L2)`).
- **Keyboard target format**: `key:<keyname>` (e.g. `key:space`, `key:a`, `key:enter`). 107 keycodes supported. Valid in remap targets, combo outputs, and macro step keys. Validation deferred to TOML save time.
- `StepTarget` enum: `Gamepad(Button)` or `Keyboard(u16)` — used in macro steps. Resolved by `resolve_step_target()` in `src/config/targets.rs`; compilation returns an error rather than substituting an invalid step.
- Known HID limitation: d-pad hat switch cannot encode 3+ simultaneous directions.
- edgemap profile format: `[profiles.<name>]` with `config = "<path>"` (relative to the XDG config directory, `~`, or absolute) + `match_process` / `match_cmdline` (case-insensitive, both optional; AND logic if both set). Profiles matched in TOML declaration order.

## Release

Release packaging and installation details live in `docs/INSTALLATION_TESTING.md`.

## Future plans

(None currently)

## Migration reference

Combo, macro, and turbo were originally in an InputPlumber-based companion at `/home/kurobac/Projects/ds/companion/`. All three features have been ported to dseuhid's direct-in-report approach (no D-Bus). The old companion is only useful for historical reference.
