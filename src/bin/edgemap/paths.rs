use std::env;
use std::path::{Path, PathBuf};

pub(crate) const EDGEMAP_CONFIG_FILE: &str = "edgemap.toml";

pub(super) fn required_home() -> Result<String, String> {
    match env::var("HOME") {
        Ok(home) if !home.is_empty() => Ok(home),
        Ok(_) | Err(env::VarError::NotPresent) => Err("HOME is not set or is empty".to_string()),
        Err(env::VarError::NotUnicode(_)) => Err("HOME is not valid Unicode".to_string()),
    }
}

pub(super) fn resolve_xdg_dir(
    xdg: Option<&Path>,
    home: Option<&str>,
    fallback: &Path,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg {
        if !xdg.as_os_str().is_empty() && xdg.is_absolute() {
            return Ok(xdg.join("edgemap"));
        }
    }
    let home = home.ok_or_else(|| "HOME is not set or is empty".to_string())?;
    Ok(PathBuf::from(home).join(fallback).join("edgemap"))
}

fn xdg_dir(var: &str, fallback: &Path) -> Result<PathBuf, String> {
    let xdg = env::var_os(var).map(PathBuf::from);
    if xdg
        .as_deref()
        .is_some_and(|path| !path.as_os_str().is_empty() && path.is_absolute())
    {
        return resolve_xdg_dir(xdg.as_deref(), None, fallback);
    }
    let home = required_home()?;
    resolve_xdg_dir(xdg.as_deref(), Some(&home), fallback)
}

pub(super) fn edgemap_config_dir() -> Result<PathBuf, String> {
    xdg_dir("XDG_CONFIG_HOME", Path::new(".config"))
}

pub(super) fn edgemap_state_dir() -> Result<PathBuf, String> {
    xdg_dir("XDG_STATE_HOME", Path::new(".local/state"))
}

pub(super) fn resolve_config_path_with_home(
    raw: &str,
    base_dir: &Path,
    home: Option<&str>,
) -> Result<String, String> {
    if raw.starts_with('/') {
        return Ok(raw.to_string());
    }
    if raw == "~" {
        let home = home.ok_or_else(|| "HOME is not set or is empty".to_string())?;
        return Ok(home.to_string());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        let home = home.ok_or_else(|| "HOME is not set or is empty".to_string())?;
        let separator = if home.ends_with('/') { "" } else { "/" };
        return Ok(format!("{home}{separator}{rest}"));
    }
    if raw.starts_with('~') {
        return Err(format!("unsupported home directory syntax: {raw}"));
    }
    Ok(base_dir.join(raw).to_string_lossy().into())
}

pub(super) fn resolve_config_path(raw: &str, base_dir: &Path) -> Result<String, String> {
    let home = if raw == "~" || raw.starts_with("~/") {
        Some(required_home()?)
    } else {
        None
    };
    resolve_config_path_with_home(raw, base_dir, home.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_expands_only_home_or_home_relative_forms() {
        let base = Path::new("/profiles");
        assert_eq!(
            resolve_config_path_with_home("~", base, Some("/home/test")).unwrap(),
            "/home/test"
        );
        assert_eq!(
            resolve_config_path_with_home("~/game.toml", base, Some("/home/test")).unwrap(),
            "/home/test/game.toml"
        );
        assert_eq!(
            resolve_config_path_with_home("~//game.toml", base, Some("/home/test")).unwrap(),
            "/home/test//game.toml"
        );
        assert_eq!(
            resolve_config_path_with_home("game.toml", base, None).unwrap(),
            "/profiles/game.toml"
        );
    }

    #[test]
    fn config_path_rejects_named_user_tilde_syntax() {
        for raw in ["~other", "~other/game.toml"] {
            let error =
                resolve_config_path_with_home(raw, Path::new("/profiles"), None).unwrap_err();
            assert!(error.contains("unsupported"));
        }
    }
}
