use serde::Deserialize;
use std::collections::HashMap;

use crate::mapping::{MappingConfig, RemapRule, StickDir, Target, Trigger, TurboConfig};
use crate::report::Button;

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    #[serde(flatten)]
    pub buttons: HashMap<String, ButtonConfig>,
    #[serde(default)]
    pub macros: HashMap<String, MacroConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct ComboConfig {
    pub key: String,
    pub output: String,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct MacroConfig {
    pub sequence: Vec<MacroStep>,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
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
    // standard buttons, excluding edge-specific ones and partition buttons
    matches!(Button::from_name(name), Some(btn) if
        btn != Button::FnLeft
        && btn != Button::FnRight
        && btn != Button::LeftPaddle
        && btn != Button::RightPaddle
        && btn != Button::Mic
        && btn != Button::TouchpadLeft
        && btn != Button::TouchpadRight
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
        let mut split_touchpad = false;

        for (btn_name, btn_conf) in &self.buttons {
            let src = Button::from_name(btn_name)
                .ok_or_else(|| format!("Unknown source button: {btn_name}"))?;

            // turbo buttons handled separately via build_turbo_configs
            if btn_conf.turbo {
                continue;
            }

            if src == Button::Touchpad && btn_conf.remap.as_deref() == Some("split") {
                split_touchpad = true;
                continue; // handled below
            }

            let dst = match btn_conf.remap.as_deref() {
                None => continue,
                Some("block") => Target::Block,
                Some(target) => resolve_target(target)
                    .ok_or_else(|| format!("Unknown target '{target}' for button '{btn_name}'"))?,
            };
            rules.push(RemapRule::new(src, dst));
        }

        if split_touchpad {
            // touchpad_left and touchpad_right must both be configured
            let left_dst = self.buttons.get("touchpad_left")
                .and_then(|c| c.remap.as_deref())
                .ok_or("split touchpad requires [touchpad_left] to be configured")?;
            let right_dst = self.buttons.get("touchpad_right")
                .and_then(|c| c.remap.as_deref())
                .ok_or("split touchpad requires [touchpad_right] to be configured")?;

            // Validate targets (block not allowed in split mode)
            if left_dst == "block" {
                return Err("touchpad_left: remap=\"block\" is not allowed in split mode".into());
            }
            if right_dst == "block" {
                return Err("touchpad_right: remap=\"block\" is not allowed in split mode".into());
            }

            let left = resolve_target(left_dst)
                .ok_or_else(|| format!("Unknown target '{left_dst}' for touchpad_left"))?;
            let right = resolve_target(right_dst)
                .ok_or_else(|| format!("Unknown target '{right_dst}' for touchpad_right"))?;

            rules.push(RemapRule::new(Button::TouchpadLeft, left));
            rules.push(RemapRule::new(Button::TouchpadRight, right));
        }

        let mut mapping = MappingConfig::from_rules_split(rules, split_touchpad);
        mapping.turbo_configs = self.build_turbo_configs();
        Ok(mapping)
    }

    pub fn build_turbo_configs(&self) -> Vec<TurboConfig> {
        let mut configs = Vec::new();
        for (btn_name, btn_conf) in &self.buttons {
            if !btn_conf.turbo {
                continue;
            }
            let src = match Button::from_name(btn_name) {
                Some(b) => b,
                None => continue,
            };
            let dst = match btn_conf.remap.as_deref() {
                Some("block") => continue,
                None => Target::Button(src),
                Some(target) => match resolve_target(target) {
                    Some(t) => t,
                    None => continue,
                },
            };
            configs.push(TurboConfig {
                src,
                dst,
                interval_ms: btn_conf.turbo_interval_ms,
                delay_ms: btn_conf.turbo_delay_ms,
            });
        }
        configs
    }
}

pub const ALL_BUTTON_NAMES: &[&str] = &[
    "square", "cross", "circle", "triangle",
    "l1", "r1", "l2", "r2",
    "l3", "r3",
    "options", "create", "ps",
    "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    "touchpad",
    "left_paddle", "right_paddle", "left_fn", "right_fn",
];

pub fn validate(cfg: &Config) -> Result<(), String> {
    let mut has_split = false;
    let mut has_touch_left = false;
    let mut has_touch_right = false;

    for btn_name in cfg.buttons.keys() {
        if !is_valid_src(btn_name) {
            return Err(format!(
                "Unknown source button: {btn_name} (valid names: square cross circle triangle \
                 l1 l2 l3 r1 r2 r3 options create ps dpad_up dpad_down dpad_left dpad_right \
                 touchpad touchpad_left touchpad_right left_paddle right_paddle left_fn right_fn)"
            ));
        }
        let btn_conf = &cfg.buttons[btn_name];
        let remap = btn_conf.remap.as_deref().unwrap_or("");

        if btn_name == "touchpad" && remap == "split" {
            has_split = true;
            continue; // validated below
        }

        // turbo with trigger source + trigger target (analog transfer) is not allowed
        if btn_conf.turbo && matches!(btn_name.as_str(), "l2" | "r2") {
            let target_is_trigger = matches!(remap, "l2" | "r2") || remap == "";
            if target_is_trigger {
                return Err(format!("[{btn_name}] turbo with trigger target '{remap}' is not supported"));
            }
        }

        if btn_conf.turbo && remap == "block" {
            return Err(format!("[{btn_name}] turbo cannot be combined with remap=\"block\""));
        }

        if btn_name == "touchpad_left" {
            has_touch_left = true;
        }
        if btn_name == "touchpad_right" {
            has_touch_right = true;
        }

        if remap != "block" && !remap.is_empty() && !is_valid_target(remap) {
            return Err(format!("[{btn_name}] unknown target: {remap}"));
        }
    }

    if has_split {
        if !has_touch_left {
            return Err("split touchpad requires [touchpad_left] to be configured".into());
        }
        if !has_touch_right {
            return Err("split touchpad requires [touchpad_right] to be configured".into());
        }
        let left_rm = cfg.buttons.get("touchpad_left").and_then(|c| c.remap.as_deref()).unwrap_or("block");
        let right_rm = cfg.buttons.get("touchpad_right").and_then(|c| c.remap.as_deref()).unwrap_or("block");
        if left_rm == "block" {
            return Err("touchpad_left: remap=\"block\" is not allowed in split mode".into());
        }
        if right_rm == "block" {
            return Err("touchpad_right: remap=\"block\" is not allowed in split mode".into());
        }
    } else if has_touch_left || has_touch_right {
        return Err("touchpad_left/right require [touchpad] remap = \"split\"".into());
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
#   touchpad touchpad_left touchpad_right
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
#
# Split touchpad mode (use left/right partitions):
#   [touchpad]
#   remap = "split"
#   [touchpad_left]
#   remap = "cross"
#   [touchpad_right]
#   remap = "circle"
# In split mode, both touchpad_left and touchpad_right must be configured.

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
        assert!(validate(&parse("[cross]\nremap = \"block\"\n")).is_ok());
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
        let cfg = parse("[cross]\nremap = \"block\"\n");
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
