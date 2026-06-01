use serde::Deserialize;
use std::collections::HashMap;

use crate::mapping::{MappingConfig, RemapRule, StickDir, Target, Trigger};
use crate::report::Button;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    #[serde(flatten)]
    pub buttons: HashMap<String, ButtonConfig>,
    #[serde(default)]
    pub macros: HashMap<String, MacroConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ButtonConfig {
    pub remap: Option<String>,
    #[serde(default)]
    pub turbo: bool,
    #[serde(default = "default_turbo_interval")]
    pub turbo_interval_ms: u64,
    #[serde(default)]
    pub turbo_delay_ms: u64,
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
}

fn default_turbo_interval() -> u64 {
    50
}

#[derive(Debug, Default, Deserialize)]
pub struct ComboConfig {
    pub key: String,
    pub output: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct MacroConfig {
    pub sequence: Vec<MacroStep>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MacroStep {
    pub key: String,
    pub press_ms: u64,
    pub release_ms: u64,
}

fn is_valid_src(name: &str) -> bool {
    matches!(
        Button::from_name(name),
        Some(_)
    ) && name != "mic"
        && name != "l2_analog"
        && name != "r2_analog"
}

fn is_valid_target(name: &str) -> bool {
    // special targets
    if matches!(
        name,
        "l2_full"
            | "r2_full"
            | "ls_up"
            | "ls_down"
            | "ls_left"
            | "ls_right"
            | "rs_up"
            | "rs_down"
            | "rs_left"
            | "rs_right"
    ) {
        return true;
    }
    // standard buttons, excluding edge-specific ones
    matches!(Button::from_name(name), Some(btn) if
        btn != Button::FnLeft
        && btn != Button::FnRight
        && btn != Button::LeftPaddle
        && btn != Button::RightPaddle
        && btn != Button::Mic
    )
}

fn resolve_target(name: &str) -> Option<Target> {
    match name {
        "l2_full" => Some(Target::TriggerFull(Trigger::L2)),
        "r2_full" => Some(Target::TriggerFull(Trigger::R2)),
        "ls_up" => Some(Target::Stick(StickDir::LS_Up)),
        "ls_down" => Some(Target::Stick(StickDir::LS_Down)),
        "ls_left" => Some(Target::Stick(StickDir::LS_Left)),
        "ls_right" => Some(Target::Stick(StickDir::LS_Right)),
        "rs_up" => Some(Target::Stick(StickDir::RS_Up)),
        "rs_down" => Some(Target::Stick(StickDir::RS_Down)),
        "rs_left" => Some(Target::Stick(StickDir::RS_Left)),
        "rs_right" => Some(Target::Stick(StickDir::RS_Right)),
        _ => Button::from_name(name).map(Target::Button),
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Self, String> {
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
        toml::from_str(&content).map_err(|e| format!("Invalid config {path}: {e}"))
    }

    pub fn to_mapping_config(&self) -> Result<MappingConfig, String> {
        let mut rules = Vec::new();
        for (btn_name, btn_conf) in &self.buttons {
            let src = Button::from_name(btn_name)
                .ok_or_else(|| format!("Unknown source button: {btn_name}"))?;
            let dst = match btn_conf.remap.as_deref() {
                None => continue,
                Some("none") => Target::Block,
                Some(target) => resolve_target(target)
                    .ok_or_else(|| format!("Unknown target '{target}' for button '{btn_name}'"))?,
            };
            rules.push(RemapRule::new(src, dst));
        }
        Ok(MappingConfig::from_rules(rules))
    }
}

pub fn validate(cfg: &Config) -> Result<(), String> {
    for btn_name in cfg.buttons.keys() {
        if !is_valid_src(btn_name) {
            return Err(format!(
                "Unknown source button: {btn_name} (valid names: square cross circle triangle \
                 l1 l2 l3 r1 r2 r3 options create ps dpad_up dpad_down dpad_left dpad_right \
                 touchpad left_paddle right_paddle left_fn right_fn)"
            ));
        }
        let btn_conf = &cfg.buttons[btn_name];
        let remap = btn_conf.remap.as_deref().unwrap_or("none");
        if remap != "none" && !is_valid_target(remap) {
            return Err(format!("[{btn_name}] unknown target: {remap}"));
        }
    }
    Ok(())
}

pub fn default_content() -> &'static str {
    r#"# dseuhid config
version = 2

# Source (section name): any button below
#   cross circle square triangle
#   l1 l2 l3 r1 r2 r3
#   options create ps
#   dpad_up dpad_down dpad_left dpad_right
#   touchpad
#   left_paddle right_paddle left_fn right_fn
#
# Target options:
#   Standard:  cross circle square triangle
#              l1 l2 l3 r1 r2 r3
#              options create ps touchpad
#              dpad_up dpad_down dpad_left dpad_right
#   Trigger:   l2_full r2_full
#   Stick:     ls_up ls_down ls_left ls_right
#              rs_up rs_down rs_left rs_right

# --- Standard buttons (default: self) ---
[cross]
remap = "cross"
[circle]
remap = "circle"
[square]
remap = "square"
[triangle]
remap = "triangle"
[l1]
remap = "l1"
[r1]
remap = "r1"
[l2]
remap = "l2"
[r2]
remap = "r2"
[l3]
remap = "l3"
[r3]
remap = "r3"
[options]
remap = "options"
[create]
remap = "create"
[ps]
remap = "ps"
[dpad_up]
remap = "dpad_up"
[dpad_down]
remap = "dpad_down"
[dpad_left]
remap = "dpad_left"
[dpad_right]
remap = "dpad_right"
[touchpad]
remap = "touchpad"

# --- DualSense Edge buttons ---
[left_paddle]
remap = "l1"
[right_paddle]
remap = "r1"
[left_fn]
remap = "create"
[right_fn]
remap = "options"
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> Config {
        toml::from_str(toml_str).expect("test config should parse")
    }

    #[test]
    fn valid_simple_remap() {
        assert!(validate(&parse("[cross]\nremap = \"circle\"\n")).is_ok());
    }

    #[test]
    fn valid_block() {
        assert!(validate(&parse("[cross]\nremap = \"none\"\n")).is_ok());
    }

    #[test]
    fn valid_trigger_target() {
        assert!(validate(&parse("[cross]\nremap = \"l2_full\"\n")).is_ok());
    }

    #[test]
    fn valid_stick_target() {
        assert!(validate(&parse("[cross]\nremap = \"ls_up\"\n")).is_ok());
        assert!(validate(&parse("[cross]\nremap = \"rs_right\"\n")).is_ok());
    }

    #[test]
    fn valid_all_standard_sources() {
        for src in &[
            "cross", "circle", "square", "triangle",
            "l1", "l2", "l3", "r1", "r2", "r3",
            "options", "create", "ps",
            "dpad_up", "dpad_down", "dpad_left", "dpad_right",
            "touchpad", "left_paddle", "right_paddle", "left_fn", "right_fn",
        ] {
            let cfg = parse(&format!("[{src}]\nremap = \"cross\"\n"));
            assert!(validate(&cfg).is_ok(), "source {src} should be valid");
        }
    }

    #[test]
    fn all_valid_targets() {
        for target in &[
            "cross", "circle", "square", "triangle",
            "l1", "l2", "l3", "r1", "r2", "r3",
            "options", "create", "ps",
            "dpad_up", "dpad_down", "dpad_left", "dpad_right",
            "touchpad", "l2_full", "r2_full",
            "ls_up", "ls_down", "ls_left", "ls_right",
            "rs_up", "rs_down", "rs_left", "rs_right",
        ] {
            let cfg = parse(&format!("[cross]\nremap = \"{target}\"\n"));
            assert!(validate(&cfg).is_ok(), "target {target} should be valid");
        }
    }

    #[test]
    fn unknown_source() {
        assert!(validate(&parse("[banana]\nremap = \"l1\"\n"))
            .unwrap_err().contains("Unknown source button"));
    }

    #[test]
    fn unknown_target() {
        assert!(validate(&parse("[cross]\nremap = \"nope\"\n"))
            .unwrap_err().contains("unknown target"));
    }

    #[test]
    fn mic_not_allowed_as_source() {
        assert!(validate(&parse("[mic]\nremap = \"cross\"\n"))
            .unwrap_err().contains("Unknown source button: mic"));
    }

    #[test]
    fn mic_not_allowed_as_target() {
        assert!(validate(&parse("[cross]\nremap = \"mic\"\n"))
            .unwrap_err().contains("unknown target"));
    }

    #[test]
    fn edge_buttons_not_allowed_as_target() {
        for edge in &["left_paddle", "right_paddle", "left_fn", "right_fn"] {
            let cfg = parse(&format!("[cross]\nremap = \"{edge}\"\n"));
            assert!(validate(&cfg).unwrap_err().contains("unknown target"),
                "edge button {edge} should not be a valid target");
        }
    }

    #[test]
    fn missing_remap_passthrough() {
        let cfg = parse("[cross]\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert!(mapping.rules.is_empty()); // no rule created
    }

    #[test]
    fn block_creates_rule() {
        let cfg = parse("[cross]\nremap = \"none\"\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.rules.len(), 1);
    }

    #[test]
    fn to_mapping_remap() {
        let cfg = parse("[cross]\nremap = \"circle\"\n");
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.rules.len(), 1);
    }

    #[test]
    fn to_mapping_trigger() {
        let cfg = parse("[cross]\nremap = \"l2_full\"\n");
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.rules.len(), 1);
    }

    #[test]
    fn to_mapping_stick() {
        let cfg = parse("[cross]\nremap = \"ls_up\"\n");
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.rules.len(), 1);
    }

    #[test]
    fn default_config_parses() {
        let cfg: Config = toml::from_str(default_content()).unwrap();
        assert_eq!(cfg.version, 2);
        assert_eq!(cfg.buttons.len(), 22);
        assert!(validate(&cfg).is_ok());
    }
}
