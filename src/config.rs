use serde::Deserialize;
use std::collections::HashMap;

use crate::mapping::{ComboRule, MappingConfig, RemapRule, StickDir, Target, Trigger, TurboConfig};
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
    100
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
    // mode-switch targets (not button/trigger/stick, but valid remap values)
    if matches!(name, "combo") {
        return true;
    }
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
        "ls_up" => Some(Target::Stick(StickDir::LsUp)),
        "ls_down" => Some(Target::Stick(StickDir::LsDown)),
        "ls_left" => Some(Target::Stick(StickDir::LsLeft)),
        "ls_right" => Some(Target::Stick(StickDir::LsRight)),
        "rs_up" => Some(Target::Stick(StickDir::RsUp)),
        "rs_down" => Some(Target::Stick(StickDir::RsDown)),
        "rs_left" => Some(Target::Stick(StickDir::RsLeft)),
        "rs_right" => Some(Target::Stick(StickDir::RsRight)),
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
        let mut blocked_buttons = Vec::new();
        let mut combo_configs = Vec::new();
        let mut split_touchpad = false;

        for (btn_name, btn_conf) in &self.buttons {
            let src = Button::from_name(btn_name)
                .ok_or_else(|| format!("Unknown source button: {btn_name}"))?;

            // turbo buttons: skip remap (handled by build_turbo_configs), but still build combos
            if btn_conf.turbo {
                if btn_conf.remap.as_deref() == Some("combo") {
                    for c in &btn_conf.combos {
                        let key = Button::from_name(&c.key)
                            .ok_or_else(|| format!("Unknown combo key '{}' in [{btn_name}]", c.key))?;
                        let output = resolve_target(&c.output)
                            .ok_or_else(|| format!("Unknown combo output '{}' in [{btn_name}]", c.output))?;
                        combo_configs.push(ComboRule { modifier: src, key, output });
                    }
                }
                continue;
            }

            if src == Button::Touchpad && btn_conf.remap.as_deref() == Some("split") {
                split_touchpad = true;
                continue;
            }

            // combo mode: build ComboRules from combos vec
            if btn_conf.remap.as_deref() == Some("combo") {
                for c in &btn_conf.combos {
                    let key = Button::from_name(&c.key)
                        .ok_or_else(|| format!("Unknown combo key '{}' in [{btn_name}]", c.key))?;
                    let output = resolve_target(&c.output)
                        .ok_or_else(|| format!("Unknown combo output '{}' in [{btn_name}]", c.output))?;
                    combo_configs.push(ComboRule { modifier: src, key, output });
                }
                continue;
            }

            let dst = match btn_conf.remap.as_deref() {
                None => continue,
                Some("block") => {
                    blocked_buttons.push(src);
                    continue;
                }
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
        mapping.blocked_buttons = blocked_buttons;
        mapping.combo_configs = combo_configs;
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
                Some("block") | Some("combo") => Target::Button(src),
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
        // reject same-as-lowercase-but-different-case section names
        // (e.g. [Cross] is rejected, [left_fn] alias is fine)
        if let Some(btn) = Button::from_name(btn_name) {
            if btn_name.to_lowercase() == btn.name() && btn_name != btn.name() {
                return Err(format!("[{btn_name}] section names must be lowercase (use \"{}\")", btn.name()));
            }
        }
        let btn_conf = &cfg.buttons[btn_name];
        let remap = btn_conf.remap.as_deref().unwrap_or("");

        if btn_name == "touchpad" && remap == "split" {
            has_split = true;
            continue;
        }

        // combo mode validations
        let is_combo = btn_name != "touchpad_left" && btn_name != "touchpad_right" && remap == "combo";
        let has_combos = !btn_conf.combos.is_empty();

        // touchpad partitions cannot use combo mode
        if matches!(btn_name.as_str(), "touchpad_left" | "touchpad_right") && remap == "combo" {
            return Err(format!("[{btn_name}] touchpad partitions cannot use combo mode"));
        }

        if is_combo && btn_conf.combos.is_empty() {
            return Err(format!("[{btn_name}] remap=\"combo\" requires at least one combo entry"));
        }

        if !is_combo && has_combos {
            return Err(format!(
                "[{btn_name}] remap and combos are mutually exclusive (use remap=\"combo\" with combos)"
            ));
        }

        // combo key/output validation
        let mut seen_keys = std::collections::HashSet::new();
        let is_fn_modifier = btn_name == "left_fn" || btn_name == "right_fn";
        for c in &btn_conf.combos {
            let key_btn = match Button::from_name(&c.key) {
                Some(b) => b,
                None => return Err(format!("[{btn_name}] unknown combo key: {}", c.key)),
            };
            if key_btn.name() == btn_name.as_str() {
                return Err(format!("[{btn_name}] combo key cannot be the same as the modifier button"));
            }
            if key_btn == Button::Mic || key_btn == Button::L2Analog || key_btn == Button::R2Analog
                || key_btn == Button::TouchpadLeft || key_btn == Button::TouchpadRight
            {
                return Err(format!("[{btn_name}] invalid combo key: {}", c.key));
            }
            if !is_valid_target(&c.output) {
                return Err(format!("[{btn_name}] unknown combo output: {}", c.output));
            }
            if !seen_keys.insert(&c.key) {
                return Err(format!("[{btn_name}] duplicate combo key '{}'", c.key));
            }
            let is_face = matches!(c.key.as_str(), "cross" | "circle" | "square" | "triangle");
            if is_fn_modifier && is_face {
                return Err(format!(
                    "[{btn_name}] FN+face combos ({}+{}) conflict with firmware profile switching",
                    btn_name, c.key
                ));
            }
        }

        // turbo with trigger source + trigger target (analog transfer) is not allowed
        if btn_conf.turbo && matches!(btn_name.as_str(), "l2" | "r2") {
            let target_is_trigger = matches!(remap, "l2" | "r2") || remap == "";
            if target_is_trigger {
                return Err(format!("[{btn_name}] turbo with trigger target '{remap}' is not supported"));
            }
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
#
# Combo mode (modifier key combinations):
#   [left_paddle]
#   remap = "combo"
#   [[left_paddle.combos]]
#   key = "cross"
#   output = "circle"
#   [[left_paddle.combos]]
#   key = "square"
#   output = "dpad_left"
# Hold modifier + press key → output injected.
# Multiple combos per modifier supported.
# Restrictions: remap="combo" is a mode switch (cannot combine with other remap values).
#   FN buttons cannot combo with face buttons (firmware conflict).
#   Combo key must differ from the modifier button.

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
    fn block_in_blocked_buttons() {
        let cfg = parse("[cross]\nremap = \"block\"\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert!(mapping.rules.is_empty());
        assert_eq!(mapping.blocked_buttons, vec![Button::Cross]);
    }

    #[test]
    fn turbo_block_allowed() {
        let cfg = parse("[cross]\nremap = \"block\"\nturbo = true\n");
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn uppercase_section_rejected() {
        assert!(validate(&parse("[Cross]\nremap = \"circle\"\n"))
            .unwrap_err().contains("section names must be lowercase"));
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

    // --- combo tests ---

    #[test]
    fn valid_combo_config() {
        let cfg = parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert!(mapping.rules.is_empty());
        assert_eq!(mapping.combo_configs.len(), 1);
    }

    #[test]
    fn valid_combo_multiple_keys() {
        let cfg = parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\n[[left_paddle.combos]]\nkey = \"square\"\noutput = \"triangle\"\n");
        assert!(validate(&cfg).is_ok());
        assert_eq!(cfg.to_mapping_config().unwrap().combo_configs.len(), 2);
    }

    #[test]
    fn combo_empty() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\n")).unwrap_err();
        assert!(e.contains("requires at least one combo entry"));
    }

    #[test]
    fn combo_remap_mutex() {
        let e = validate(&parse("[left_paddle]\nremap = \"cross\"\n[[left_paddle.combos]]\nkey = \"square\"\noutput = \"circle\"\n")).unwrap_err();
        assert!(e.contains("remap and combos are mutually exclusive"));
    }

    #[test]
    fn combo_unknown_key() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"banana\"\noutput = \"cross\"\n")).unwrap_err();
        assert!(e.contains("unknown combo key"));
    }

    #[test]
    fn combo_unknown_output() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"banana\"\n")).unwrap_err();
        assert!(e.contains("unknown combo output"));
    }

    #[test]
    fn combo_duplicate_key() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"square\"\n")).unwrap_err();
        assert!(e.contains("duplicate combo key"));
    }

    #[test]
    fn combo_self_key() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"left_paddle\"\noutput = \"cross\"\n")).unwrap_err();
        assert!(e.contains("combo key cannot be the same as the modifier"));
    }

    #[test]
    fn combo_fn_face_rejected() {
        let e = validate(&parse("[left_fn]\nremap = \"combo\"\n[[left_fn.combos]]\nkey = \"cross\"\noutput = \"circle\"\n")).unwrap_err();
        assert!(e.contains("FN+face"));
    }

    #[test]
    fn combo_paddle_face_ok() {
        let cfg = parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\n");
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn combo_touchpad_partition_rejected() {
        let e = validate(&parse("[touchpad]\nremap = \"split\"\n[touchpad_left]\nremap = \"combo\"\n[[touchpad_left.combos]]\nkey = \"cross\"\noutput = \"circle\"\n")).unwrap_err();
        assert!(e.contains("touchpad partitions cannot use combo mode"));
    }

    #[test]
    fn combo_block_rejected() {
        let e = validate(&parse("[left_paddle]\nremap = \"block\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\n")).unwrap_err();
        assert!(e.contains("remap and combos are mutually exclusive"));
    }
}
