# BUGFIX.md — edgemap

## #1–#37: See STATUS.md `## Bugfixes` section

These cover v0.0.1 through v0.1.0 — UHID proxy core, remap, turbo, combo, macro, config validation.
Link: [STATUS.md#bugfixes-chronological](./STATUS.md#bugfixes-chronological)

---

## New entries (v0.2.0–v0.4.0)

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
