use std::path::Path;

use serde::Deserialize;

use super::super::paths::resolve_config_path;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProfileConfig {
    pub(crate) config: String,
    #[serde(default)]
    pub(crate) match_process: String,
    #[serde(default)]
    pub(crate) match_cmdline: String,
}

fn read_comm(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_lowercase())
}

fn read_cmdline(pid: u32) -> Option<String> {
    let data = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if data.is_empty() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&data)
            .replace('\0', " ")
            .to_lowercase(),
    )
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessSnapshot {
    pub(crate) pid: u32,
    pub(crate) comm: Option<String>,
    pub(crate) cmdline: Option<String>,
}

pub(crate) fn profile_matches(process: &ProcessSnapshot, profile: &ProfileConfig) -> bool {
    if profile.match_process.is_empty() && profile.match_cmdline.is_empty() {
        return false;
    }
    if !profile.match_process.is_empty() {
        let comm = match process.comm.as_deref() {
            Some(comm) => comm,
            None => return false,
        };
        if comm != profile.match_process {
            return false;
        }
    }
    if !profile.match_cmdline.is_empty() {
        let cmdline = match process.cmdline.as_deref() {
            Some(cmdline) => cmdline,
            None => return false,
        };
        if !cmdline.contains(&profile.match_cmdline) {
            return false;
        }
    }
    true
}

fn snapshot_processes(profiles: &[(String, ProfileConfig)]) -> Vec<ProcessSnapshot> {
    let need_comm = profiles
        .iter()
        .any(|(_, profile)| !profile.match_process.is_empty());
    let need_cmdline = profiles
        .iter()
        .any(|(_, profile)| !profile.match_cmdline.is_empty());

    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            Some(ProcessSnapshot {
                pid,
                comm: need_comm.then(|| read_comm(pid)).flatten(),
                cmdline: need_cmdline.then(|| read_cmdline(pid)).flatten(),
            })
        })
        .collect()
}

pub(crate) fn find_matching_profile(
    profiles: &[(String, ProfileConfig)],
    config_dir: &Path,
    base_config: &str,
) -> Result<Option<String>, String> {
    let processes = snapshot_processes(profiles);
    for (profile_name, profile_cfg) in profiles {
        for process in &processes {
            if profile_matches(process, profile_cfg) {
                log::debug!("profile matched: name={profile_name}, pid={}", process.pid);
                return resolve_config_path(&profile_cfg.config, config_dir).map(Some);
            }
        }
    }
    resolve_config_path(base_config, config_dir).map(Some)
}
