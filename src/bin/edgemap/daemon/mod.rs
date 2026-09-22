mod audio;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use super::cli::USAGE;
use super::control_session::*;
use super::paths::*;

pub(crate) mod monitor;
pub(crate) mod profile;

#[cfg(test)]
use monitor::{is_runtime_file, watch_parent};
use monitor::{wait_for_daemon_activity, DaemonActivity, DaemonMonitor};
use profile::{find_matching_profile, ProfileConfig};
#[cfg(test)]
use profile::{profile_matches, ProcessSnapshot};

use dseuhid::{config, control, shutdown};
use serde::Deserialize;
use shutdown::{unblock_shutdown_signals_in_child, ShutdownSignal};

const DEFAULT_CONFIG_FILE: &str = "default.toml";
fn needs_config_became_true(previous: Option<bool>, current: bool) -> bool {
    current && previous != Some(true)
}

fn send_notification(summary: &str, body: &str) {
    let mut command = std::process::Command::new("notify-send");
    command
        .args(["-a", "edgemap", summary, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unblock_shutdown_signals_in_child(&mut command);
    match command.spawn() {
        Ok(child) => {
            if let Err(error) = reap_child(child) {
                log::warn!("failed to start notify-send child reaper: {error}");
            }
        }
        Err(error) => log::debug!("failed to start notify-send: {error}"),
    }
}

fn reap_child(mut child: std::process::Child) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("edgemap-child-reaper".to_string())
        .spawn(move || {
            if let Err(error) = child.wait() {
                log::debug!("failed to reap notify-send child: {error}");
            }
        })
}

struct DaemonState {
    base_config: String,
    base_config_raw: String,
    profiles: Vec<(String, ProfileConfig)>,
    valid_profiles: Vec<(String, String)>,
    dir: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DaemonConfigFile {
    #[serde(default = "default_config_file")]
    config: String,
    #[serde(default)]
    profiles: toml::Table,
}

fn default_config_file() -> String {
    DEFAULT_CONFIG_FILE.to_string()
}

fn load_edgemap_config(path: &Path) -> Result<DaemonState, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read edgemap config: path={}, error={e}",
            path.display()
        )
    })?;
    let root: DaemonConfigFile = toml::from_str(&content).map_err(|e| {
        format!(
            "failed to parse edgemap config: path={}, error={e}",
            path.display()
        )
    })?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let base_config_raw = root.config;
    let base_config = resolve_config_path(&base_config_raw, &dir)?;

    // defer validation of base/default config to daemon loop (pre-injection)

    let mut profiles: Vec<(String, ProfileConfig)> = Vec::with_capacity(root.profiles.len());
    for (name, value) in root.profiles {
        let mut profile = value.try_into::<ProfileConfig>().map_err(|error| {
            format!(
                "failed to parse profile: path={}, name={name}, error={error}",
                path.display()
            )
        })?;
        profile.match_process = profile.match_process.to_lowercase();
        profile.match_cmdline = profile.match_cmdline.to_lowercase();
        profiles.push((name, profile));
    }

    let mut valid_profiles: Vec<(String, String)> = Vec::new();
    for (name, pcfg) in &profiles {
        let p_path = resolve_config_path(&pcfg.config, &dir)?;
        if pcfg.match_process.is_empty() && pcfg.match_cmdline.is_empty() {
            log::warn!("profile skipped: name={name}, reason=no match criteria");
            continue;
        }
        // defer config existence/validation to daemon loop (pre-injection)
        valid_profiles.push((name.clone(), p_path));
    }

    Ok(DaemonState {
        base_config,
        base_config_raw,
        profiles,
        valid_profiles,
        dir,
    })
}

fn reload_edgemap_config(state: &mut DaemonState, path: &Path) -> Result<(), String> {
    let replacement = load_edgemap_config(path)?;
    *state = replacement;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigApplyFailure {
    selected_path: String,
    message: String,
}

#[derive(Default)]
struct ConfigApplyTracker {
    selected_path: Option<String>,
    effective_path: Option<String>,
    last_failure: Option<ConfigApplyFailure>,
    force: bool,
}

struct PendingConfigApply {
    selected_path: String,
    target_path: String,
    target_label: String,
    active_config: config::ActiveConfig,
    failure_after_ack: Option<ConfigApplyFailure>,
}

enum ConfigApplyPlan {
    NoChange,
    Retain { failure_changed: bool },
    Request(PendingConfigApply),
}

impl ConfigApplyTracker {
    fn force_unknown(&mut self) {
        self.effective_path = None;
        self.force = true;
    }

    fn set_failure(&mut self, failure: ConfigApplyFailure) -> bool {
        let changed = self.last_failure.as_ref() != Some(&failure);
        self.last_failure = Some(failure);
        changed
    }

    fn prepare<F>(
        &mut self,
        selected_path: &str,
        selected_label: &str,
        base_path: &str,
        mut load: F,
    ) -> ConfigApplyPlan
    where
        F: FnMut(&str) -> Result<config::ActiveConfig, String>,
    {
        let failed_selection_is_current = self
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.selected_path == selected_path);
        if !self.force
            && self.effective_path.as_deref() == Some(selected_path)
            && !failed_selection_is_current
        {
            self.last_failure = None;
            return ConfigApplyPlan::NoChange;
        }

        match load(selected_path) {
            Ok(active_config) => {
                if !self.force && self.effective_path.as_deref() == Some(selected_path) {
                    self.last_failure = None;
                    return ConfigApplyPlan::NoChange;
                }
                ConfigApplyPlan::Request(PendingConfigApply {
                    selected_path: selected_path.to_string(),
                    target_path: selected_path.to_string(),
                    target_label: selected_label.to_string(),
                    active_config,
                    failure_after_ack: None,
                })
            }
            Err(selected_error) => {
                let mut failure_message = selected_error;
                if selected_path == base_path {
                    let failure_changed = self.set_failure(ConfigApplyFailure {
                        selected_path: selected_path.to_string(),
                        message: failure_message,
                    });
                    return ConfigApplyPlan::Retain { failure_changed };
                }

                if !self.force && self.effective_path.as_deref() == Some(base_path) {
                    let failure_changed = self.set_failure(ConfigApplyFailure {
                        selected_path: selected_path.to_string(),
                        message: failure_message,
                    });
                    return ConfigApplyPlan::Retain { failure_changed };
                }

                match load(base_path) {
                    Ok(active_config) => {
                        let failure_after_ack = ConfigApplyFailure {
                            selected_path: selected_path.to_string(),
                            message: failure_message,
                        };
                        ConfigApplyPlan::Request(PendingConfigApply {
                            selected_path: selected_path.to_string(),
                            target_path: base_path.to_string(),
                            target_label: "default config".to_string(),
                            active_config,
                            failure_after_ack: Some(failure_after_ack),
                        })
                    }
                    Err(base_error) => {
                        failure_message.push_str("; default config unavailable: ");
                        failure_message.push_str(&base_error);
                        let failure_changed = self.set_failure(ConfigApplyFailure {
                            selected_path: selected_path.to_string(),
                            message: failure_message,
                        });
                        ConfigApplyPlan::Retain { failure_changed }
                    }
                }
            }
        }
    }

    fn acknowledge(&mut self, pending: &PendingConfigApply) -> bool {
        self.selected_path = Some(pending.selected_path.clone());
        self.effective_path = Some(pending.target_path.clone());
        self.force = false;
        match &pending.failure_after_ack {
            Some(failure) => self.set_failure(failure.clone()),
            None => {
                self.last_failure = None;
                false
            }
        }
    }

    fn record_request_failure(&mut self, pending: &PendingConfigApply, message: &str) -> bool {
        let message = match &pending.failure_after_ack {
            Some(validation_failure) => format!(
                "{}; failed to apply default config: {message}",
                validation_failure.message
            ),
            None => message.to_string(),
        };
        self.set_failure(ConfigApplyFailure {
            selected_path: pending.selected_path.clone(),
            message,
        })
    }
}

fn load_valid_config(path: &str) -> Result<config::ActiveConfig, String> {
    if !Path::new(path).exists() {
        return Err(format!("config not found: path={path}"));
    }
    let active_config = config::ActiveConfig::read(path)
        .map_err(|error| format!("failed to load config: path={path}, error={error}"))?;
    let parsed = active_config
        .parse()
        .map_err(|error| format!("failed to parse config: path={path}, error={error}"))?;
    config::validate(&parsed)
        .map_err(|error| format!("config validation failed: path={path}, error={error}"))?;
    Ok(active_config)
}

pub(crate) fn cmd_daemon(args: &[String]) -> ! {
    let mut config_arg: Option<&str> = None;

    // parse optional --config <path> from args
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_arg = Some(&args[i + 1]);
            i += 1;
        } else {
            eprintln!("error: unknown argument '{}'", args[i]);
            eprintln!("Usage: edgemap daemon [--config <PATH>]");
            std::process::exit(1);
        }
        i += 1;
    }

    let edgemap_config_path = match config_arg {
        Some(path) if Path::new(path).is_absolute() => Ok(PathBuf::from(path)),
        Some(path) if path.starts_with('~') => {
            resolve_config_path(path, Path::new("")).map(PathBuf::from)
        }
        Some(path) => edgemap_config_dir()
            .and_then(|dir| resolve_config_path(path, &dir))
            .map(PathBuf::from),
        None => edgemap_config_dir().map(|dir| dir.join(EDGEMAP_CONFIG_FILE)),
    }
    .unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let state_dir = edgemap_state_dir().unwrap_or_else(|e| {
        log::error!("failed to resolve state directory: {e}");
        std::process::exit(1);
    });
    let _daemon_lock =
        control::DaemonLock::acquire_named(&state_dir, "edgemap.lock", "edgemap daemon")
            .unwrap_or_else(|e| {
                log::error!("failed to acquire edgemap daemon lock: {e}");
                std::process::exit(1);
            });

    let dir = edgemap_config_path.parent().unwrap_or(Path::new("."));

    if !edgemap_config_path.exists() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!(
                "failed to create config directory: path={}, error={e}",
                dir.display()
            );
            std::process::exit(1);
        }
        let default_toml_content = config::default_content();
        let default_remap_path = dir.join(DEFAULT_CONFIG_FILE);
        if !default_remap_path.exists() {
            if let Err(e) = std::fs::write(&default_remap_path, default_toml_content) {
                log::error!(
                    "failed to write config: path={}, error={e}",
                    default_remap_path.display()
                );
                std::process::exit(1);
            }
            log::info!("config created: path={}", default_remap_path.display());
        }
        let edgemap_toml = format!("config = \"{DEFAULT_CONFIG_FILE}\"\n");
        if let Err(e) = std::fs::write(&edgemap_config_path, edgemap_toml) {
            log::error!(
                "failed to write config: path={}, error={e}",
                edgemap_config_path.display()
            );
            std::process::exit(1);
        }
        log::info!("config created: path={}", edgemap_config_path.display());
    }

    let config_path = edgemap_config_path.clone();
    let mut state = match load_edgemap_config(&config_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("failed to load edgemap config: {e}");
            std::process::exit(1);
        }
    };

    let shutdown = ShutdownSignal::new().unwrap_or_else(|e| {
        log::error!("failed to initialize signal handling: {e}");
        std::process::exit(1);
    });

    // SIGPIPE keeps its existing default behavior; SIGINT/SIGTERM use signalfd.
    unsafe {
        let handler = libc::SIG_DFL;
        let _ = libc::signal(libc::SIGPIPE, handler);
    }

    log::info!("edgemap daemon started");
    log::info!("edgemap config: path={}", config_path.display());

    let mut monitor = DaemonMonitor::new(&config_path).unwrap_or_else(|e| {
        log::error!("failed to initialize daemon monitor: {e}");
        std::process::exit(1);
    });

    let mut audio = audio::AudioManager::default();
    let mut apply_tracker = ConfigApplyTracker::default();
    let mut control_client: Option<control::ControlClient> = None;
    let mut control_state: Option<control::ControlState> = None;
    let mut warned_not_running = false;
    let mut activity = DaemonActivity::new();

    let run_result: Result<(), String> = loop {
        if activity.shutdown_requested {
            break Ok(());
        }
        if activity.config_changed {
            activity.config_changed = false;
            match reload_edgemap_config(&mut state, &config_path) {
                Ok(()) => {
                    apply_tracker.force_unknown();
                    activity.profile_due = true;
                    log::info!("edgemap config reloaded: path={}", config_path.display());
                }
                Err(e) => {
                    log::error!("failed to reload edgemap config; previous config retained: {e}")
                }
            }
        }

        if activity.runtime_changed {
            activity.runtime_changed = false;
            let previous_state = control_state;
            let was_alive = control_client.is_some();
            let mut disconnect_reason = None;

            if let Some(client) = control_client.as_ref() {
                match drain_control_state(client, |state| {
                    audio.update(state.bt_haptics.filter(|_| state.uhid_ready))
                }) {
                    Ok(Some(state)) => control_state = Some(state),
                    Ok(None) => {}
                    Err(e) => {
                        disconnect_reason = Some(e);
                        control_client = None;
                        control_state = None;
                    }
                }
            } else {
                match connect_control() {
                    Ok((client, state)) => {
                        control_client = Some(client);
                        control_state = Some(state);
                        warned_not_running = false;
                    }
                    Err(e) => {
                        if !warned_not_running {
                            log::info!("waiting for dseuhid: {e}");
                            warned_not_running = true;
                        }
                    }
                }
            }

            if control_client.is_none() {
                if previous_state.is_some_and(|state| state.uhid_ready) {
                    log::info!("virtual HID device unavailable");
                }
                if was_alive {
                    log::warn!(
                        "dseuhid control connection lost: reason={}",
                        disconnect_reason
                            .as_deref()
                            .unwrap_or("control socket closed")
                    );
                }
            } else if let Some(state) = control_state {
                if !was_alive {
                    log::info!("dseuhid control connection established");
                    apply_tracker.force_unknown();
                    activity.profile_due = true;
                }
                let previous_ready = previous_state.is_some_and(|old| old.uhid_ready);
                if state.uhid_ready && !previous_ready {
                    log::info!("virtual HID device ready");
                } else if !state.uhid_ready && previous_ready {
                    log::info!("virtual HID device unavailable");
                }
                let previous_needs = previous_state.map(|old| old.needs_config);
                if needs_config_became_true(previous_needs, state.needs_config) {
                    apply_tracker.force_unknown();
                    activity.profile_due = true;
                }
            }
        }

        audio.update(control_state.and_then(|state| state.bt_haptics.filter(|_| state.uhid_ready)));

        if !control_state.is_some_and(|state| state.uhid_ready) {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &shutdown,
                control_client.as_ref(),
                &mut activity,
            ) {
                break Err(format!("daemon wait failed: {e}"));
            }
            continue;
        }

        if !activity.profile_due {
            if let Err(e) = wait_for_daemon_activity(
                &mut monitor,
                &shutdown,
                control_client.as_ref(),
                &mut activity,
            ) {
                break Err(format!("daemon wait failed: {e}"));
            }
            continue;
        }
        activity.profile_due = false;

        let wanted = if state.valid_profiles.is_empty() {
            state.base_config.clone()
        } else {
            let valid: Vec<_> = state
                .profiles
                .iter()
                .filter(|(name, _)| state.valid_profiles.iter().any(|(vn, _)| vn == name))
                .cloned()
                .collect();
            match find_matching_profile(&valid, &state.dir, &state.base_config_raw) {
                Ok(Some(path)) => path,
                Ok(None) => state.base_config.clone(),
                Err(e) => {
                    log::error!("failed to resolve profile config: {e}");
                    if let Err(wait_error) = wait_for_daemon_activity(
                        &mut monitor,
                        &shutdown,
                        control_client.as_ref(),
                        &mut activity,
                    ) {
                        break Err(format!("daemon wait failed: {wait_error}"));
                    }
                    continue;
                }
            }
        };

        let wanted_label = state
            .profiles
            .iter()
            .find(|(_, profile)| {
                resolve_config_path(&profile.config, &state.dir).as_deref() == Ok(wanted.as_str())
            })
            .map(|(name, _)| format!("profile '{name}'"))
            .unwrap_or_else(|| "default config".to_string());
        let plan = apply_tracker.prepare(
            &wanted,
            &wanted_label,
            &state.base_config,
            load_valid_config,
        );
        match plan {
            ConfigApplyPlan::NoChange => {}
            ConfigApplyPlan::Retain { failure_changed } => {
                if failure_changed {
                    if let Some(failure) = &apply_tracker.last_failure {
                        log::warn!(
                            "config decision failed; previous config retained: {}",
                            failure.message
                        );
                    }
                }
            }
            ConfigApplyPlan::Request(pending) => {
                let request = control::ControlRequest::SwitchConfig(pending.active_config.clone());
                let result = match (control_client.as_ref(), control_state.as_mut()) {
                    (Some(client), Some(control_state)) => {
                        send_daemon_control_request(client, &request, &shutdown, control_state)
                    }
                    _ => Err(DaemonRequestError::Failed(
                        "dseuhid control connection is unavailable".to_string(),
                    )),
                };
                match result {
                    Ok(()) => {
                        let failure_changed = apply_tracker.acknowledge(&pending);
                        if failure_changed {
                            if let Some(failure) = &apply_tracker.last_failure {
                                log::warn!(
                                    "profile config invalid; using default config: {}",
                                    failure.message
                                );
                            }
                        }
                        log::info!("config applied: source={}", pending.target_label);
                        log::info!("config path: path={}", pending.target_path);
                        send_notification(
                            "edgemap",
                            &format!("Switched to {}", pending.target_label),
                        );
                    }
                    Err(DaemonRequestError::Shutdown) => {
                        break Ok(());
                    }
                    Err(DaemonRequestError::Failed(error)) => {
                        if apply_tracker.record_request_failure(&pending, &error) {
                            log::warn!("dseuhid control request failed: {error}");
                        }
                        activity.runtime_changed = true;
                    }
                }
            }
        }
        if let Err(e) = wait_for_daemon_activity(
            &mut monitor,
            &shutdown,
            control_client.as_ref(),
            &mut activity,
        ) {
            break Err(format!("daemon wait failed: {e}"));
        }
    };

    if let Err(error) = &run_result {
        log::error!("{error}");
    }
    drop(audio);
    log::info!("edgemap daemon stopped");
    std::process::exit(if run_result.is_ok() { 0 } else { 1 });
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn usage_uses_conventional_placeholders() {
        assert!(USAGE.contains("Usage: edgemap <COMMAND> [ARGS]"));
        assert!(USAGE.contains("switch-config <PATH>"));
        assert!(!USAGE.contains("  r, reload"));
        assert!(!USAGE.contains("<path>"));
    }

    #[test]
    fn absolute_xdg_path_is_used_without_home() {
        assert_eq!(
            resolve_xdg_dir(Some(Path::new("/tmp/xdg")), None, Path::new(".config")),
            Ok(PathBuf::from("/tmp/xdg/edgemap"))
        );
    }

    #[test]
    fn invalid_xdg_paths_fall_back_to_home() {
        for xdg in [Path::new(""), Path::new("relative/path")] {
            assert_eq!(
                resolve_xdg_dir(Some(xdg), Some("/home/test"), Path::new(".config")),
                Ok(PathBuf::from("/home/test/.config/edgemap"))
            );
        }
    }

    #[test]
    fn missing_home_rejects_xdg_fallback() {
        assert!(resolve_xdg_dir(None, None, Path::new(".local/state")).is_err());
    }

    #[test]
    fn absolute_config_path_does_not_need_home() {
        assert_eq!(
            resolve_config_path_with_home("/tmp/config.toml", Path::new("/base"), None),
            Ok("/tmp/config.toml".to_string())
        );
        assert_eq!(watch_parent(Path::new("edgemap.toml")), Path::new("."));
    }

    #[test]
    fn tilde_config_path_requires_home() {
        assert!(resolve_config_path_with_home("~/config.toml", Path::new("/base"), None).is_err());
        assert_eq!(
            resolve_config_path_with_home("~/config.toml", Path::new("/base"), Some("/home/test")),
            Ok("/home/test/config.toml".to_string())
        );
    }

    fn profile(process: &str, cmdline: &str) -> ProfileConfig {
        ProfileConfig {
            config: "test.toml".to_string(),
            match_process: process.to_string(),
            match_cmdline: cmdline.to_string(),
        }
    }

    fn process(comm: Option<&str>, cmdline: Option<&str>) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 42,
            comm: comm.map(str::to_string),
            cmdline: cmdline.map(str::to_string),
        }
    }

    #[test]
    fn profile_match_requires_all_configured_fields() {
        let cfg = profile("game", "--profile edge");
        assert!(profile_matches(
            &process(Some("game"), Some("/usr/bin/game --profile edge")),
            &cfg
        ));
        assert!(!profile_matches(
            &process(Some("game"), Some("/usr/bin/game --profile default")),
            &cfg
        ));
        assert!(!profile_matches(
            &process(Some("launcher"), Some("/usr/bin/game --profile edge")),
            &cfg
        ));
    }

    #[test]
    fn profile_match_rejects_missing_process_data() {
        assert!(!profile_matches(&process(None, None), &profile("game", "")));
        assert!(!profile_matches(&process(None, None), &profile("", "game")));
    }

    #[test]
    fn empty_profile_does_not_match() {
        assert!(!profile_matches(
            &process(Some("game"), Some("game")),
            &profile("", "")
        ));
    }

    #[test]
    fn daemon_monitor_detects_config_write() {
        assert!(is_runtime_file(std::ffi::OsStr::new("control.sock")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("connected")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("needs-config")));
        assert!(!is_runtime_file(std::ffi::OsStr::new("unrelated")));
        assert!(needs_config_became_true(None, true));
        assert!(needs_config_became_true(Some(false), true));
        assert!(!needs_config_became_true(Some(true), true));
        assert!(!needs_config_became_true(Some(true), false));

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("edgemap-inotify-{}-{unique}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let config_path = dir.join("edgemap.toml");
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        assert_ne!(monitor.run_watch.is_some(), monitor.runtime_watch.is_some());
        std::fs::write(&config_path, "config = \"changed.toml\"\n").unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();

        assert!(wake.config_changed);

        let result = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) };
        assert_eq!(result, 0);
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.shutdown);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn daemon_monitor_recovers_after_config_directory_recreation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "edgemap-config-watch-{}-{unique}",
            std::process::id()
        ));
        let config_dir = root.join("edgemap");
        let config_path = config_dir.join("edgemap.toml");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        assert!(monitor.config_watch.is_some());
        assert!(monitor.config_parent_watch.is_none());

        std::fs::remove_file(&config_path).unwrap();
        std::fs::remove_dir(&config_dir).unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.config_changed);
        assert!(monitor.config_watch.is_none());
        assert!(monitor.config_parent_watch.is_some());

        std::fs::create_dir(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"restored.toml\"\n").unwrap();
        let wake = monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(wake.config_changed);
        assert!(monitor.config_watch.is_some());
        assert!(monitor.config_parent_watch.is_none());

        drop(monitor);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn daemon_monitor_fails_if_config_parent_disappears() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "edgemap-config-parent-{}-{unique}",
            std::process::id()
        ));
        let config_dir = root.join("edgemap");
        let config_path = config_dir.join("edgemap.toml");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(&config_path, "config = \"default.toml\"\n").unwrap();

        let shutdown = ShutdownSignal::new().unwrap();
        let mut monitor = DaemonMonitor::new(&config_path).unwrap();
        std::fs::remove_file(&config_path).unwrap();
        std::fs::remove_dir(&config_dir).unwrap();
        monitor
            .wait(Instant::now() + Duration::from_secs(1), &shutdown, None)
            .unwrap();
        assert!(monitor.config_parent_watch.is_some());

        std::fs::remove_dir(&root).unwrap();
        let error = match monitor.wait(Instant::now() + Duration::from_secs(1), &shutdown, None) {
            Err(error) => error,
            Ok(_) => panic!("config parent removal should fail the monitor"),
        };
        assert!(error.contains("config parent directory watch lost"));
    }

    fn write_edgemap_config(root: &Path, content: &str) -> PathBuf {
        std::fs::create_dir_all(root).unwrap();
        let path = root.join("edgemap.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn edgemap_config_rejects_invalid_root_and_profile_fields() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("edgemap-schema-{}-{unique}", std::process::id()));

        for (content, expected) in [
            ("config = 7\n", "config"),
            ("profiles = 7\n", "profiles"),
            (
                "config = \"default.toml\"\nunknown = true\n",
                "unknown",
            ),
            (
                "[profiles.game]\nconfig = \"game.toml\"\nmatch_process = 7\n",
                "game",
            ),
            (
                "[profiles.game]\nconfig = \"game.toml\"\nmatch_process = \"game\"\nunknown = true\n",
                "unknown",
            ),
        ] {
            let path = write_edgemap_config(&root, content);
            let error = load_edgemap_config(&path).err().unwrap();
            assert!(
                error.contains(expected),
                "error did not mention {expected:?}: {error}"
            );
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn edgemap_config_preserves_toml_profile_declaration_order() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("edgemap-order-{}-{unique}", std::process::id()));
        let path = write_edgemap_config(
            &root,
            concat!(
                "config = \"default.toml\"\n",
                "profiles.inline = { config = \"inline.toml\", match_process = \"inline\" } # inline\n",
                "profiles.dotted.config = \"dotted.toml\"\n",
                "profiles.dotted.match_process = \"dotted\"\n",
                "[profiles.alpha]\n",
                "config = \"alpha.toml\"\n",
                "match_process = \"alpha\"\n",
                "[profiles.\"game.with.dot\"] # quoted\n",
                "config = \"quoted.toml\"\n",
                "match_process = \"quoted\"\n",
            ),
        );

        let state = load_edgemap_config(&path).unwrap();
        let names: Vec<_> = state
            .profiles
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["inline", "dotted", "alpha", "game.with.dot"]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_reload_keeps_the_previous_daemon_state() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("edgemap-reload-{}-{unique}", std::process::id()));
        let path = write_edgemap_config(
            &root,
            "config = \"first.toml\"\n[profiles.game]\nconfig = \"game.toml\"\nmatch_process = \"game\"\n",
        );
        let mut state = load_edgemap_config(&path).unwrap();
        let old_base = state.base_config.clone();
        let old_profiles: Vec<_> = state
            .profiles
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        std::fs::write(
            &path,
            "config = \"second.toml\"\n[profiles.broken]\nconfig = 7\n",
        )
        .unwrap();
        assert!(reload_edgemap_config(&mut state, &path).is_err());
        assert_eq!(state.base_config, old_base);
        assert_eq!(
            state
                .profiles
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            old_profiles
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    fn active_config_for_test(path: &str) -> config::ActiveConfig {
        config::ActiveConfig::from_content(path.to_string(), config::default_content().to_string())
            .unwrap()
    }

    #[test]
    fn invalid_profile_is_rechecked_without_reapplying_the_effective_base() {
        let base = "/configs/default.toml";
        let profile = "/configs/game.toml";
        let mut tracker = ConfigApplyTracker {
            selected_path: Some(base.to_string()),
            effective_path: Some(base.to_string()),
            ..Default::default()
        };

        for expected_new_failure in [true, false] {
            let mut loaded = Vec::new();
            let plan = tracker.prepare(profile, "profile 'game'", base, |path| {
                loaded.push(path.to_string());
                Err(format!("invalid config: {path}"))
            });
            assert_eq!(loaded, [profile]);
            assert!(matches!(
                plan,
                ConfigApplyPlan::Retain {
                    failure_changed
                } if failure_changed == expected_new_failure
            ));
            assert_eq!(tracker.selected_path.as_deref(), Some(base));
            assert_eq!(tracker.effective_path.as_deref(), Some(base));
        }

        let mut loaded = Vec::new();
        let plan = tracker.prepare(profile, "profile 'game'", base, |path| {
            loaded.push(path.to_string());
            Ok(active_config_for_test(path))
        });
        let ConfigApplyPlan::Request(pending) = plan else {
            panic!("repaired profile should request an apply");
        };
        assert_eq!(loaded, [profile]);
        assert_eq!(pending.target_path, profile);
        tracker.acknowledge(&pending);
        assert_eq!(tracker.selected_path.as_deref(), Some(profile));
        assert_eq!(tracker.effective_path.as_deref(), Some(profile));
        assert!(tracker.last_failure.is_none());
    }

    #[test]
    fn failed_profile_falls_back_once_and_keeps_revalidating_only_the_candidate() {
        let base = "/configs/default.toml";
        let profile = "/configs/game.toml";
        let mut tracker = ConfigApplyTracker::default();
        tracker.force_unknown();
        let mut loaded = Vec::new();
        let plan = tracker.prepare(profile, "profile 'game'", base, |path| {
            loaded.push(path.to_string());
            if path == profile {
                Err("profile is invalid".to_string())
            } else {
                Ok(active_config_for_test(path))
            }
        });
        let ConfigApplyPlan::Request(pending) = plan else {
            panic!("unknown live state should request the base fallback");
        };
        assert_eq!(loaded, [profile, base]);
        assert_eq!(pending.target_path, base);
        assert!(pending.failure_after_ack.is_some());
        assert!(tracker.last_failure.is_none());
        assert!(tracker.acknowledge(&pending));
        assert_eq!(tracker.selected_path.as_deref(), Some(profile));
        assert_eq!(tracker.effective_path.as_deref(), Some(base));
        assert!(tracker.last_failure.is_some());

        loaded.clear();
        let plan = tracker.prepare(profile, "profile 'game'", base, |path| {
            loaded.push(path.to_string());
            Err("profile is invalid".to_string())
        });
        assert_eq!(loaded, [profile]);
        assert!(matches!(
            plan,
            ConfigApplyPlan::Retain {
                failure_changed: false
            }
        ));
    }

    #[test]
    fn successful_same_path_is_not_reloaded_until_live_state_is_forced_unknown() {
        let path = "/configs/game.toml";
        let mut tracker = ConfigApplyTracker {
            selected_path: Some("/configs/previous.toml".to_string()),
            effective_path: Some(path.to_string()),
            ..Default::default()
        };

        for _ in 0..2 {
            let plan = tracker.prepare(path, "profile 'game'", "/configs/default.toml", |_| {
                panic!("an unchanged effective path must not reread its file")
            });
            assert!(matches!(plan, ConfigApplyPlan::NoChange));
            assert_eq!(
                tracker.selected_path.as_deref(),
                Some("/configs/previous.toml")
            );
        }

        tracker.force_unknown();
        let plan = tracker.prepare(path, "profile 'game'", "/configs/default.toml", |path| {
            Ok(active_config_for_test(path))
        });
        let ConfigApplyPlan::Request(pending) = plan else {
            panic!("needs_config/new daemon lifetime must force reinjection");
        };
        tracker.acknowledge(&pending);
        assert_eq!(tracker.effective_path.as_deref(), Some(path));
    }

    #[test]
    fn repaired_effective_path_does_not_advance_selected_without_an_ack() {
        let path = "/configs/game.toml";
        let mut tracker = ConfigApplyTracker {
            selected_path: Some("/configs/previous.toml".to_string()),
            effective_path: Some(path.to_string()),
            last_failure: Some(ConfigApplyFailure {
                selected_path: path.to_string(),
                message: "previous validation failure".to_string(),
            }),
            ..Default::default()
        };

        let plan = tracker.prepare(path, "profile 'game'", "/configs/default.toml", |path| {
            Ok(active_config_for_test(path))
        });

        assert!(matches!(plan, ConfigApplyPlan::NoChange));
        assert_eq!(
            tracker.selected_path.as_deref(),
            Some("/configs/previous.toml")
        );
        assert_eq!(tracker.effective_path.as_deref(), Some(path));
        assert!(tracker.last_failure.is_none());
    }

    #[test]
    fn failed_control_request_does_not_advance_selected_or_effective_state() {
        let mut tracker = ConfigApplyTracker {
            selected_path: Some("/configs/old.toml".to_string()),
            effective_path: Some("/configs/old.toml".to_string()),
            ..Default::default()
        };
        let plan = tracker.prepare(
            "/configs/new.toml",
            "profile 'new'",
            "/configs/default.toml",
            |path| Ok(active_config_for_test(path)),
        );
        let ConfigApplyPlan::Request(pending) = plan else {
            panic!("a changed valid selection should request an apply");
        };

        assert!(tracker.record_request_failure(&pending, "control request failed"));
        assert_eq!(tracker.selected_path.as_deref(), Some("/configs/old.toml"));
        assert_eq!(tracker.effective_path.as_deref(), Some("/configs/old.toml"));
    }

    #[test]
    fn failed_fallback_request_does_not_alternate_failure_state() {
        let base = "/configs/default.toml";
        let profile = "/configs/game.toml";
        let mut tracker = ConfigApplyTracker::default();
        tracker.force_unknown();

        for expected_changed in [true, false] {
            let plan = tracker.prepare(profile, "profile 'game'", base, |path| {
                if path == profile {
                    Err("profile is invalid".to_string())
                } else {
                    Ok(active_config_for_test(path))
                }
            });
            let ConfigApplyPlan::Request(pending) = plan else {
                panic!("the fallback should remain pending until it is acknowledged");
            };
            assert_eq!(
                tracker.record_request_failure(&pending, "control request failed"),
                expected_changed
            );
            assert!(tracker.selected_path.is_none());
            assert!(tracker.effective_path.is_none());
        }
    }

    #[test]
    fn child_reaper_waits_for_process_exit() {
        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = nix::unistd::Pid::from_raw(child.id() as i32);
        reap_child(child).unwrap().join().unwrap();
        // The child must already have been reaped, not merely handed to a thread.
        assert_eq!(
            nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)),
            Err(nix::errno::Errno::ECHILD)
        );
    }
}
