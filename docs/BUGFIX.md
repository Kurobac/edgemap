# BUGFIX.md — edgemap

## #1–#37: before v0.2.0 (UHID core, remap, turbo, combo, macro)

### #1 — ACL lost after exit
**Root cause:** `setfacl -b` removed ACL, restore didn't re-apply.
**Fix:** Save ACL before hiding, restore on exit. (`18e826e` + `b5ad76a`)

### #2 — WebHID/CP2077 SET_REPORT timeout
**Root cause:** UHID didn't reply SET_REPORT_REPLY → kernel timeout EIO → Chrome abandoned device.
**Fix:** Reply `UHID_SET_REPORT_REPLY` to every SET_REPORT. (`450cc4d`)

### #3 — All output reports silently dropped
**Root cause:** `rtype == USB_OUTPUT_REPORT_ID` always false (rtype is UHID type, not HID report ID).
**Fix:** Check actual report type correctly. (`56c6528`)

### #4 — SET_REPORT replied OK but data not forwarded
**Root cause:** `send_set_report_reply(id, 0)` but data never written to real hardware.
**Fix:** Forward SET_REPORT data to physical device via hidraw. (`373efce`)

### #5 — Hotplug: EIO → exit instead of reconnect
**Root cause:** No outer loop.
**Fix:** Wrap device detection + proxy in outer reconnect loop. (`fb39885`)

### #6 — Ctrl+C during reconnect wait ignored
**Root cause:** Inner wait loop didn't check RUNNING flag.
**Fix:** Check RUNNING in all wait loops. (`fb39885`)

### #7 — restore_permissions panic on DeviceGone
**Root cause:** Node already removed, path doesn't exist.
**Fix:** `skip_restore()` + exists check before restore. (`fb39885`)

### #8 — Remap broke FN/back buttons
**Root cause:** `parse_input_report` / `apply_state_to_report` read/wrote byte 11 (should be byte 10).
**Fix:** Correct byte position to byte 10. (`27ae79a`)

### #9 — FN/back buttons left/right swapped
**Root cause:** Byte 10 high nibble bit→label mapping was wrong: 0x10=FnLeft not RightPaddle, 0x40=LeftPaddle not LeftFn.
**Fix:** Correct mapping: FnLeft=0x10, FnRight=0x20, LeftPaddle=0x40, RightPaddle=0x80. (`6e3ceec`)

### #10 — Cross-mapping (A→B + B→A) random result
**Root cause:** `apply()` used same `state` for reads and writes → rule1 output contaminated rule2 input.
**Fix:** Clone state as snapshot before rules evaluate (snapshot isolation). (`86bcf92`)

### #11 — Multi-key simultaneous missing outputs
**Root cause:** Deferred button targets set immediately within rule loop → later `rule.clear()` undid them.
**Fix:** Two-phase apply: Phase 1 clears sources + collects targets, Phase 2 sets all atomically. (`86bcf92`)

### #12 — PS/Mic button leak
**Root cause:** `apply_state_to_report` preserved `raw[10] & 0x0F` → low nibble from previous frame leaked.
**Fix:** Zero high nibble in byte 10 before writing Edge button bits. (`6e3ceec`)

### #13 — Trigger analog leak
**Root cause:** `apply()` cleared L2/R2 digital bit but not `l2_analog`/`r2_analog`.
**Fix:** Clear analog values when clearing trigger source button. (`6e3ceec`)

### #14 — GET_REPORT/SET_REPORT/UHID Output spam at debug
**Root cause:** Default `debug!` level flooded logs.
**Fix:** Demote to `trace!` level. (`bc6defa`)

### #15 — Device found log duplicated
**Root cause:** Both `find_dualsense()` and `main.rs` logged the same info.
**Fix:** Remove duplicate log. (`bc6defa`)

### #16 — UHID virtual device matched by find_dualsense
**Root cause:** No physical/UHID distinction → could proxy own virtual device recursively.
**Fix:** Check `/sys/class/hidraw/N/device/uevent` for `DRIVER=uhid`. (`42bf5ca`)

### #17 — Touchpad coordinate wrong (x=3800 out of range)
**Root cause:** Byte offset: `touchdata` starts at byte 33, not 34; nibble: low nibble = x_hi, not high nibble.
**Fix:** Correct byte offset and nibble extraction. (`81b330d`)

### #18 — Split mode allowed without both partitions
**Root cause:** Validation missing.
**Fix:** Require both `touchpad_left` and `touchpad_right` sections when split mode enabled. (`81b330d`)

### #19 — Split mode allowed with touchpad section missing
**Root cause:** `contains_key("touchpad")` guard was too specific.
**Fix:** Accept split mode without explicit touchpad section. (`48739ae`)

### #20 — Trigger self-map lost analog
**Root cause:** `apply()` unconditionally cleared L2/R2 analog on all targets.
**Fix:** Preserve analog on self-map (L2→L2, R2→R2). (`8ad808b`)

### #21 — Trigger swap (L2→R2) lost analog
**Root cause:** Self-map fix only handled self, not cross-trigger.
**Fix:** Transfer analog values on trigger swap. (`9aa83b8`)

### #22 — Turbo source button leaked through
**Root cause:** `to_mapping_config()` skips turbo buttons → no RemapRule to suppress source.
**Fix:** L1 turbo suppresses source button directly. (`ef20805`)

### #23 — Turbo target not maintained between frames
**Root cause:** Only applied on toggle event, not every frame.
**Fix:** Per-frame maintain: `if step.pressed { state.set_button(..., true); }` every frame. (`ef20805`)

### #24 — Turbo on L2/R2 source leaked analog
**Root cause:** Source suppression only cleared digital bit, not analog.
**Fix:** Clear `l2_analog`/`r2_analog` when turbo suppresses L2/R2 source. (`ef20805`)

### #25 — `remap="none"` semantics confused with missing remap
**Root cause:** `unwrap_or("none")` treated missing remap as explicit block.
**Fix:** Rename `none` → `block`. Missing remap = passthrough (no suppression). (`9ad6dbb`)

### #26 — `[cross] turbo=true` (no remap) silently skipped
**Root cause:** `build_turbo_configs()` skipped `None` remap instead of self-targeting.
**Fix:** Self-target when remap is missing and turbo is enabled. (`9ad6dbb`)

### #27 — SIGHUP kills process when device not connected
**Root cause:** `setup_reload_handler()` registered inside device loop; default handler terminated process.
**Fix:** Install SIGHUP handler at startup, not inside the device loop. (`6616c24`)

### #28 — `turbo=true remap="combo"` combo configs skipped
**Root cause:** `to_mapping_config()` turbo `continue` jumped over combo config building.
**Fix:** Stop skipping combo config build for turbo buttons. (`3394651`)

### #29 — `turbo=true remap="combo"` turbo runtime not built
**Root cause:** `build_turbo_configs()` `resolve_target("combo")` returned None.
**Fix:** Allow combo target in turbo resolve. (`3394651`)

### #30 — Combo→macro reset every frame, stuck at 0ms
**Root cause:** L2 macro detection (read L1, trigger suppressed by combo) deactivated macro; combo injection simultaneously activated it.
**Fix:** Introduce `MacroSource` enum (Physical vs Combo). Combo manages its own macros exclusively. (`5053099`)

### #31 — Macro output only on first frame, then invisible
**Root cause:** `tick()` only wrote button on press event; subsequent frames state was recreated from physical buf, no maintain.
**Fix:** Per-frame maintain (same pattern as turbo bugfix #23). (`5053099`)

### #32 — Inactive combo cleared physical button input
**Root cause:** `combo_triggers` unconditionally pushed `(target, false)` every frame, L2 injection cleared buttons even when combo not triggered.
**Fix:** Only push on activation, never push deactivation. (`f1752a6`)

### #33 — Remap source-clear wiped combo-injected button
**Root cause:** COMBO injection before REMAP → remap Phase 1 `state.set_button(src, false)` erased combo output.
**Fix:** Swap order: COMBO injection after REMAP. (`3f095db`)

### #34 — Combo injection `push(false)` caused premature release
**Root cause:** `push(&c.output, false)` on combo deactivation overrode multi-source buttons.
**Fix:** Combo injection never clears — only pushes activation, not deactivation. State re-parse handles cleanup naturally. (`f1752a6`)

### #35 — Combo modifier L2/R2 analog leaked
**Root cause:** L1 combo suppression cleared modifier digital bit but not `l2_analog`/`r2_analog`.
**Fix:** Clear analog for L2/R2 combo modifiers. (`9bed9de`)

### #36 — Macro mode silent fallthrough
**Root cause:** `mode = "banana"` passed validation and defaulted to `Hold` silently.
**Fix:** Reject unknown mode values. (`9bed9de`)

### #37 — Macro name shadowed built-in target
**Root cause:** `macros.l2_full` passed validation, `resolve_target` always matched `TriggerFull(L2)` first, macro unreachable.
**Fix:** Reject macro names that conflict with built-in target names. (`9bed9de`)

---

## #38–#62: after v0.2.0 (edgemap daemon, packaging, GUI)

### #38 — mkfifo/chmod `ENAMETOOLONG` / garbage filenames
**Root cause:** `&str::as_ptr()` is not null-terminated. `libc::mkfifo()` and `libc::chmod()` scan memory past the string until hitting a random zero byte, producing either `ENAMETOOLONG` or files with garbage-suffixed names.

**Fix:** Use `CString::new(path)` which guarantees null termination. (`8b54ac4`)

### #39 — Subcommands silently ignored extra arguments
**Root cause:** `dseuhid monitor 123` and `edgemap reload xxx` only checked `args[1]` and ignored `args[2..]`.

**Fix:** Add `args.len()` guards for all subcommands in both binaries. (`552def2`, `2a008f5`)

### #40 — edgemap daemon per-second `"dseuhid not running?"` spam
**Root cause:** `try_send_fifo_command()` printed a warning on every failure. The daemon loop called it every second regardless of state.

**Fix:** Make `try_send_fifo_command()` silent (return `bool` only). Track state transitions in the daemon loop, print only on `offline → online` and `online → offline`. (`87ebd24`)

### #41 — edgemap daemon wrote `switch-config` to FIFO every second
**Root cause:** `try_send_fifo_command()` both detected dseuhid alive AND wrote data. Called every loop iteration → dseuhid reloaded the same config every second.

**Fix:** Split into `check_dseuhid_alive()` (open + close, no write) and `try_send_fifo_command()` (open + write + close). Only call the write function on state transition `offline → online`. (`87ebd24`)

### #42 — edgemap daemon no warning when dseuhid absent at startup
**Root cause:** The daemon loop only compared alive state across iterations; initial state was unset.

**Fix:** After startup, do one `check_dseuhid_alive()` call. If false, print `"dseuhid not running, waiting..."`. (`87ebd24`)

### #43 — Profile matching used alphabetical order, not declaration order
**Root cause:** `toml::Value::Table` is `BTreeMap` (sorted alphabetically). `[profiles.firefox]` (`f`) was checked before `[profiles.opencode]` (`o`).

**Fix:** `extract_profile_order()` scans the raw TOML text for `[profiles.NAME]` headers to recover declaration order. Sort parsed profiles by this order before matching. (`e404df1`)

### #44 — Profile matching loop order: PIDs outer, profiles inner
**Root cause:** `find_matching_profile()` iterated `/proc/<pid>` as the outer loop and profiles as the inner loop. Since `/proc` order is nondeterministic (kernel-dependent), one background process could beat another.

**Fix:** Swap loops — profiles outer (declaration order guarantees priority), PIDs inner. (`1c4f8ec`)

### #45 — Auto-generated `edgemap.toml` used absolute path for `config` key
**Root cause:** `config = "/home/kurobac/.config/edgemap/default.toml"` — verbose, not portable, not user-friendly.

**Fix:** Write `config = "default.toml"` (relative). Add `resolve_config_path()` to handle relative paths (join to `~/.config/edgemap/`), `~`-prefixed, and absolute paths uniformly. (`87ebd24`)

### #46 — Daemon log showed remap path as "config:", misleading
**Root cause:** `log::info!("config: {remap_path}")` displayed the remap config path, making it look like edgemap's own config.

**Fix:** Show `edgemap_config_path` in the initial log, let the `"applied profile ..."` log carry the remap path. (`87ebd24`)

### #47 — Cargo.toml version left at 0.1.0 after v0.2.0 tag
**Root cause:** The tag was created before bumping the version string in `Cargo.toml`.

**Fix:** Post-tag commit `e71b347` to sync the version. Future tags now ensure version is bumped first.

### #48 — No singleton detection: multiple daemon instances coexist
**Root cause:** Neither dseuhid nor edgemap checked for existing instances before starting. Running a second `sudo dseuhid` would remove the first's FIFO and interfere with device discovery.

**Fix:** Singleton detection via PID file + `kill(pid, 0)` liveness check. dseuhid uses `/run/dseuhid/pid` (FHS), edgemap uses `~/.local/state/edgemap/edgemap.pid` (XDG_STATE_HOME). PID cleaned on graceful exit. (`23863e5`)

### #49 — Unvalidated profile path sent to dseuhid causing load failure
**Root cause:** `find_matching_profile()` used `state.profiles` (all declared profiles) instead of only validated ones. A profile whose `config` file doesn't exist or failed validation could still match and send an invalid `switch-config` path to dseuhid.

**Fix:** Filter `state.profiles` by `state.valid_profiles` before passing to `find_matching_profile()`. Only profiles with existing, valid config files participate in matching.

### #50 — systemctl restart dseuhid not detected by edgemap
**Root cause:** `systemctl restart` completes in <0.3s — faster than edgemap's 1s polling interval. `check_dseuhid_alive()` never saw the FIFO disappear, so `current_config` was never cleared and the profile was never re-injected.

**Fix:** Track dseuhid's PID via `/run/dseuhid/pid`. When the PID changes (restart occurred), clear `current_config` to force re-injection on the next iteration. Covers restart, crash+restart, and first-start scenarios.

### #51 — Macro picker popped up on GUI startup (Qt/Breeze layout passthrough bug)
**Root cause:** `QStatusBar.addPermanentWidget()` triggers an internal layout recalculation that propagates upward to QMainWindow. Under the Breeze theme, this causes `QAction.triggered` signals on the toolbar to fire spuriously during the layout pass, activating `_open_macros()` and showing MacroPicker at app startup.

**Fix:** Wrap `act_macros.triggered.connect(...)` in `QTimer.singleShot(0, ...)`, deferring the signal connection until after the initial layout pass completes. This prevents Breeze from misfiring the trigger during layout recalculation.

### #52 — GUI: New button did not reset config or rebuild UI
**Root cause:** `_new_config()` only updated `self.current_file` and the profile button text — it did not actually reset `self.config` to defaults or rebuild the UI tables.

**Fix:** Fully reset `self.config` to default mappings and call `_build_ui()` to regenerate all widgets. Also added dirty-detection (unsaved changes confirm dialog) powered by remap/turbo writeback to `self.config` on every widget change.

### #53 — GUI: _build_ui() duplicated toolbars and profile buttons on each call
**Root cause:** `_build_ui()` unconditionally created new QToolBar (`self.addToolBar()`) and QPushButton (`self.statusBar().addPermanentWidget()`), without removing previously-created instances.

**Fix:** Check for existing `self.toolbar`/`self.profile_btn` before creating. Remove old toolbar via `self.removeToolBar()`. Reuse profile button — only rebuild its QMenu. Prevents widget accumulation on every `_build_ui()` call.

### #54 — GUI: Turbo spinboxes changed values on mouse wheel scroll
**Root cause:** QSpinBox defaults to mouse-wheel value increment/decrement.

**Fix:** Set `spinbox.wheelEvent = lambda e: None` on all spinboxes (turbo I:/D: + MacroEditor press/release ms).

### #55 — GUI: Focus randomly jumped to menu/profile buttons on startup
**Root cause:** QToolButton and QPushButton are in Qt's tab-focus-chain; the first focusable widget grabs initial focus on window activation.

**Fix:** Set `setFocusPolicy(Qt.FocusPolicy.NoFocus)` on both the menu QToolButton and the profile QPushButton.

### #56 — GUI: New icon showed wrong image (Qt logo) in Breeze theme
**Root cause:** `SP_TitleBarMenuButton` has no Breeze icon mapping — falls back to a generic Qt logo.

**Fix:** Use `QIcon.fromTheme("application-menu")` for the menu icon and `QIcon.fromTheme("document-new")` for New. Freedesktop-compatible icon themes (including Breeze) provide correct icons for these names.

### #57 — GUI: Profile button width broke with long filenames; menu misaligned
**Root cause:** Profile button had no width constraint, and its QMenu was independently-sized.

**Fix:** Set `setMinimumWidth(100)`, `setMaximumWidth(250)` on the button, and sync the menu width to the button's width via `aboutToShow.connect(lambda: menu.setMinimumWidth(btn.width()))`. Truncate displayed filenames > 32 chars (middle ellipsis).

### #58 — Hotplug not detected by edgemap — config not applied after USB reconnect
**Root cause:** edgemap only tracked dseuhid's PID and FIFO liveness. On USB disconnect/reconnect, the UHID device is recreated but the PID stays the same and the FIFO never closes — so edgemap never knew to re-inject the config.

**Fix:** dseuhid now writes `/run/dseuhid/connected` every time UHID is created. edgemap polls its mtime — any change triggers `current_config.clear()` → re-inject on next iteration. Covers USB hotplug, `systemctl restart`, and first-start.

### #59 — Profile config validation deferred to injection time
**Root cause:** edgemap daemon validated all profile configs at startup. A profile referencing a non-existent or invalid config file was skipped entirely — requiring mtime-based edgemap.toml reload to re-add it after the config file was created.

**Fix:** Defer config file existence and validation to `switch-config` injection time. Profiles are always loaded into `valid_profiles` regardless of config file state. If a config becomes valid later, it auto-recovers on the next daemon iteration without requiring an edgemap.toml reload. Invalid profiles gracefully fall back to base_config.

### #60 — Hotplug detection reworked with connected/disconnected marker content
**Root cause:** Original fix (#58) used mtime-only detection which couldn't distinguish device state — edgemap would attempt injection even when no controller was connected, generating spurious warnings.

**Fix:** dseuhid now writes `"connected"` on UHID creation and `"disconnected"` on device loss. edgemap reads content: `"disconnected"` skips injection entirely, `"connected"` with mtime change triggers re-injection. Also adds Gamepad connect/disconnect desktop notifications.

### #61 — ComboDialog: adding a new entry reset all existing entries to cross/circle
**Root cause:** `_rebuild()` destroyed all QComboBox widgets via `setRowCount(0)` without first reading their current values. The QComboBox in their destructors may have emitted signals that mutated `self.combos` before the table was rebuilt, causing all entries to revert to their defaults.

**Fix:** Read `currentText()` from each existing combo widget into `self.combos` BEFORE calling `setRowCount(0)`. The saved state is then used to populate the rebuilt rows correctly.

### #62 — GUI: Open button crashed with TypeError (expected str, got bool)
**Root cause:** `QAction.triggered` signal emits a `bool` (checked) parameter. Connecting it directly to `_open_config(path=None)` passed `path=False`, causing `subprocess.run(["edgemap","v",False])` to fail.

**Fix:** Wrap the connection in a lambda that discards the bool: `act_open.triggered.connect(lambda: self._open_config())`. Profile menu connections already used `lambda p=path:` and were unaffected.

### #63 — GET_REPORT cache: caching 0x09 (pairing info) from physical device breaks UHID device probe
**Root cause:** Feature report 0x09 is `DS_FEATURE_REPORT_PAIRING_INFO` — NOT firmware version. Bytes 1-6 are the Bluetooth MAC address. The kernel `hid-playstation` driver's `dualsense_get_mac_address()` reads this report during `probe()`. The driver uses the MAC to generate `hdev->uniq` and create a power_supply named `ps-controller-battery-XX:XX:XX:XX:XX:XX`. When the physical device has the same MAC as our cached value, the virtual device tries to register a power_supply with an already-existing sysfs name → `-EEXIST` → probe failure → `hid_hw_stop()` silently removes the virtual device from the HID bus.

This is specific to Sony's `hid-playstation` driver (not a general kernel limitation). The kernel's HID subsystem doesn't enforce `hdev->uniq` uniqueness; the conflict is in the power_supply sysfs namespace.

**Fix:** Skip caching report 0x09. Only cache 0x05 (`DS_FEATURE_REPORT_CALIBRATION` — gyro/accel calibration, 41 bytes) and 0x20 (`DS_FEATURE_REPORT_FIRMWARE_INFO` — firmware version, 64 bytes). 0x09 falls back to the hardcoded value (original developer's device MAC, which is different from any real device and thus never conflicts).

**Diagnosis process:**
1. Bisect isolated the regression to the GET_REPORT cache read loop.
2. Pinpointed 0x09 as the specific report by testing each cached report individually (0x05 only → pass, 0x09 only → fail, 0x20 only → pass, 0x05+0x20 → pass).
3. A standalone `hidraw-test` demo confirmed `HIDIOCGFEATURE` ioctl on the same fd does not corrupt the main fd.
4. **Byte-level bisect** on 0x09's 20 bytes narrowed the trigger to the 6-byte MAC address field (bytes 1-6). Individual byte changes all passed; full MAC replacement failed; changing only 1 bit of the MAC restored functionality — proving it was a **MAC address conflict**, not data format corruption.
5. **Cross-device verification**: re-enabled 0x09 caching and switched from DSE to regular DS. Failed (DS MAC self-conflict). Disabled cache, DS worked (hardcoded DSE MAC ≠ physical DS MAC). Final proof.


### #64 — Cargo incremental build cache produces corrupted binaries
**Root cause:** `git checkout` between branches followed by `cargo build` (without `cargo clean`) can produce binaries that pass bisect tests but fail in production. `cargo` incremental compilation may re-use object files from a previous build with incompatible intermediate representations, silently producing a broken binary that crashes or behaves incorrectly.

**Fix:** Always run `cargo clean` before `cargo build` when switching between commits for bisect or regression testing. Add `cargo clean` to the test procedure in AGENTS.md.

### #65 — FIFO buffer too small for long config paths in `switch-config`
**Root cause:** `handle_fifo_command()` used a fixed 256-byte buffer. A `switch-config <path>` command with a path longer than ~240 characters would be truncated at the buffer boundary, with the tail bytes arriving in the next `read()` and being discarded as unrecognized input. Since Linux `PATH_MAX` = 4096, valid absolute paths could up to that length but would be silently truncated.

**Fix:** Increase FIFO read buffer from 256 to 4096 bytes.

### Cleanup — dead code removal (pre-1.0)
- Removed `trigger_reload()` (proxy.rs) — never called; reload is handled via `SHOULD_RELOAD` atomic.
- Removed `dualsense_usb_descriptor()` (descriptor.rs) — unused wrapper; callers use `DS_EDGE_USB_DESCRIPTOR` constant directly.
- Removed `USB_OUTPUT_REPORT_SIZE` and `USB_OUTPUT_REPORT_ID` (report.rs) — never referenced.
- Removed unnecessary `#[allow(dead_code)]` from `DS5_PID` (device.rs) and 5 serde config structs (config.rs) — all actively used now.

### #66 — `dup_fifo_fd()`: negative file descriptor passed to `from_raw_fd()` after failed `dup()`

**Root cause:** `dup_fifo_fd()` calls `libc::dup(raw)` which returns `-1` on error. The code checked `fd < 0` and logged an error, but didn't return or exit — falling through to `from_raw_fd(-1)`, undefined behavior that could silently create a broken File from an invalid fd.

**Fix:** Add `std::process::exit(1)` after logging the error, before `from_raw_fd(fd)`.

### #67 — `edgemap-gui-v6.py`: duplicate `return {}` in `load_config()`

**Root cause:** In the `OSError` except handler of `load_config()`, there were two consecutive `return {}` statements (lines 73-74). The second was unreachable dead code — harmless but sloppy.

**Fix:** Remove the duplicate `return {}`.

### #68 — `/run/dseuhid/connected` not cleaned up on clean shutdown

**Root cause:** `teardown_fifo()` removed `FIFO_PATH` and `PID_PATH` but not `/run/dseuhid/connected`. After a clean daemon shutdown (`systemctl stop` or `kill`), the file remained with content `"connected"`, misleading edgemap into believing a daemon was still running with a controller attached.

**Fix:** Add `remove_file("/run/dseuhid/connected")` to `teardown_fifo()`. Also added a one-time `info!("Waiting for DualSense device...")` log in the device detection wait loop so the daemon indicates it's alive and waiting rather than silently polling.

### #69 — Turbo toggle overrides physical button press on same target

**Root cause:** In L1 turbo, `apply_target_to_state(&mut state, &t.dst, t.phase)` unconditionally SET the target button to the toggle phase (true/false). If the user was physically holding the target button (e.g., holding B while A is turbo→B), the turbo would clear it on the next toggle off phase, replacing the physical long-press with a rapid-fire toggle.

**Fix:** After writing the turbo toggle value, check whether the physical snapshot has the target button pressed. If so, force it to `true`, overriding the turbo toggle back to on. Excluded self-turbo (`*btn != t.src`) to avoid locking self-turbo permanently on.


