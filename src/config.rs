use serde::Deserialize;
use std::collections::HashMap;

use crate::mapping::{ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, RemapRule, StickDir, Target, Trigger, TurboConfig};
use crate::report::Button;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    #[serde(default = "default_output_device")]
    pub output_device: String,
    #[serde(flatten)]
    pub buttons: HashMap<String, ButtonConfig>,
    #[serde(default)]
    pub macros: HashMap<String, MacroConfig>,
}

fn default_output_device() -> String {
    "auto".to_string()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ComboConfig {
    pub key: String,
    pub output: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroConfig {
    #[serde(default = "default_macro_mode")]
    pub mode: String,
    pub sequence: Vec<MacroStep>,
}

fn default_macro_mode() -> String {
    "hold".into()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroStep {
    pub key: String,
    pub press_ms: u64,
    pub release_ms: u64,
}

fn is_valid_src(name: &str) -> bool {
    Button::from_name(name).is_some() && name != "mic"
        && name != "l2_analog"
        && name != "r2_analog"
}

fn is_valid_target(name: &str) -> bool {
    // keep in sync with edgemap-gui-v6.py TARGETS list
    if matches!(name, "combo" | "passthrough") {
        return true;
    }
    // keyboard targets: "key:<keyname>"
    if let Some(key) = name.strip_prefix("key:") {
        return !key.is_empty() && crate::keyboard::resolve_keycode(key).is_some();
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
        && btn != Button::L2Analog
        && btn != Button::R2Analog
    )
}

fn resolve_target(name: &str) -> Option<Target> {
    // keep in sync with edgemap-gui-v6.py TARGETS + builtin
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

fn resolve_target_or_macro(name: &str, macros: &HashMap<String, MacroConfig>) -> Option<Target> {
    if let Some(key) = name.strip_prefix("key:") {
        return crate::keyboard::resolve_keycode(key).map(Target::Keyboard);
    }
    resolve_target(name).or_else(|| {
        if macros.contains_key(name) {
            Some(Target::Macro(name.to_string()))
        } else {
            None
        }
    })
}

fn resolve_step_target(key: &str) -> Option<crate::mapping::StepTarget> {
    if let Some(kc) = key.strip_prefix("key:") {
        return crate::keyboard::resolve_keycode(kc).map(crate::mapping::StepTarget::Keyboard);
    }
    Button::from_name(key).map(crate::mapping::StepTarget::Gamepad)
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

            // turbo buttons with remap=combo: build combos in L1, no RemapRule needed
            if btn_conf.turbo && btn_conf.remap.as_deref() == Some("combo") {
                for c in &btn_conf.combos {
                    let key = Button::from_name(&c.key)
                        .ok_or_else(|| format!("Unknown combo key '{}' in [{btn_name}]", c.key))?;
                    let output = resolve_target_or_macro(&c.output, &self.macros)
                        .ok_or_else(|| format!("Unknown combo output '{}' in [{btn_name}]", c.output))?;
                    combo_configs.push(ComboRule { modifier: src, key, output });
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
                    let output = resolve_target_or_macro(&c.output, &self.macros)
                        .ok_or_else(|| format!("Unknown combo output '{}' in [{btn_name}]", c.output))?;
                    combo_configs.push(ComboRule { modifier: src, key, output });
                }
                continue;
            }

            let dst = match btn_conf.remap.as_deref() {
                Some("passthrough") => continue,
                None => continue,
                Some("block") => {
                    blocked_buttons.push(src);
                    continue;
                }
                Some(target) => resolve_target_or_macro(target, &self.macros)
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

            let left = resolve_target_or_macro(left_dst, &self.macros)
                .ok_or_else(|| format!("Unknown target '{left_dst}' for touchpad_left"))?;
            let right = resolve_target_or_macro(right_dst, &self.macros)
                .ok_or_else(|| format!("Unknown target '{right_dst}' for touchpad_right"))?;

            rules.push(RemapRule::new(Button::TouchpadLeft, left));
            rules.push(RemapRule::new(Button::TouchpadRight, right));
        }

        let mut mapping = MappingConfig::from_rules_split(rules, split_touchpad);
        mapping.turbo_configs = self.build_turbo_configs();
        mapping.blocked_buttons = blocked_buttons;
        mapping.combo_configs = combo_configs;
        mapping.macro_configs = self.build_macro_configs();
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
            configs.push(TurboConfig {
                src,
                interval_ms: btn_conf.turbo_interval_ms,
                delay_ms: btn_conf.turbo_delay_ms,
            });
        }
        configs
    }

    pub fn build_macro_configs(&self) -> Vec<MacroRule> {
        let mut configs: Vec<MacroRule> = Vec::new();

        // Physical trigger: remap points directly to a macro name
        for (btn_name, btn_conf) in &self.buttons {
            if btn_conf.turbo {
                continue;
            }
            let trigger = match Button::from_name(btn_name) {
                Some(b) => b,
                None => continue,
            };
            let remap = btn_conf.remap.as_deref().unwrap_or("");
            let macro_name = if matches!(remap, "block" | "combo") || remap.is_empty() {
                continue;
            } else {
                remap
            };
            let mcfg = match self.macros.get(macro_name) {
                Some(m) => m,
                None => continue,
            };
            let mode = match mcfg.mode.as_str() {
                "single" => MacroMode::Single,
                _ => MacroMode::Hold,
            };
            let steps: Vec<crate::mapping::MacroStep> = mcfg.sequence.iter().map(|s| {
                crate::mapping::MacroStep {
                    action: resolve_step_target(&s.key).unwrap_or(crate::mapping::StepTarget::Gamepad(Button::Cross)),
                    press_ms: s.press_ms,
                    release_ms: s.release_ms,
                }
            }).collect();
            configs.push(MacroRule {
                trigger,
                name: macro_name.to_string(),
                mode,
                steps,
                source: MacroSource::Physical,
            });
        }

        // Combo trigger: combo output points to a macro name
        for btn_conf in self.buttons.values() {
            if !matches!(btn_conf.remap.as_deref(), Some("combo")) {
                continue;
            }
            for c in &btn_conf.combos {
                let mcfg = match self.macros.get(&c.output) {
                    Some(m) => m,
                    None => continue,
                };
                let mode = match mcfg.mode.as_str() {
                    "single" => MacroMode::Single,
                    _ => MacroMode::Hold,
                };
                let steps: Vec<crate::mapping::MacroStep> = mcfg.sequence.iter().map(|s| {
                    crate::mapping::MacroStep {
                        action: resolve_step_target(&s.key).unwrap_or(crate::mapping::StepTarget::Gamepad(Button::Cross)),
                        press_ms: s.press_ms,
                        release_ms: s.release_ms,
                    }
                }).collect();
                configs.push(MacroRule {
                    trigger: Button::Cross,
                    name: c.output.clone(),
                    mode,
                    steps,
                    source: MacroSource::Combo,
                });
            }
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
    if !matches!(cfg.output_device.as_str(), "auto" | "dualsense" | "dualshock4") {
        return Err(format!(
            "Unknown output_device: {} (valid: auto, dualsense, dualshock4)",
            cfg.output_device
        ));
    }

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
            if c.output == "passthrough" {
                return Err(format!("[{btn_name}] combo output cannot be passthrough"));
            }
            if !is_valid_target(&c.output) && !cfg.macros.contains_key(&c.output) {
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
            let target_is_trigger = matches!(remap, "l2" | "r2") || remap.is_empty();
            if target_is_trigger {
                return Err(format!("[{btn_name}] turbo with trigger target '{remap}' is not supported"));
            }
        }

        // turbo + macro mutual exclusion (same button, including combo→macro)
        // NOTE: turbo on the combo KEY (not the modifier) is NOT validated
        // (e.g. [a] remap="combo" key="b" output="my_macro" + [b] turbo=true).
        // Turbo toggles the source button which combo detection sees; the
        // macro fires at turbo rate. Too edge-case to warrant a validation rule.
        if btn_conf.turbo {
            let has_macro_output = match btn_conf.remap.as_deref() {
                Some(r) if cfg.macros.contains_key(r) => true,
                Some("combo") => btn_conf.combos.iter().any(|c| cfg.macros.contains_key(&c.output)),
                _ => false,
            };
            if has_macro_output {
                return Err(format!("[{btn_name}] turbo and macros are mutually exclusive"));
            }
            if btn_conf.remap.as_deref() == Some("passthrough") {
                return Err(format!("[{btn_name}] turbo and passthrough are mutually exclusive"));
            }
        }

        if btn_name == "touchpad_left" {
            has_touch_left = true;
        }
        if btn_name == "touchpad_right" {
            has_touch_right = true;
        }

        if remap != "block" && !remap.is_empty() && !is_valid_target(remap)
            && !cfg.macros.contains_key(remap)
        {
            return Err(format!("[{btn_name}] unknown target: {remap}"));
        }
    }

    // macro-wide validation — keep macro name bans in sync with edgemap-gui-v6.py builtin set
    for (name, m) in &cfg.macros {
        if Button::from_name(name).is_some() {
            return Err(format!("Macro name '{name}' conflicts with a standard button name"));
        }
        if name == "passthrough" {
            return Err("Macro name 'passthrough' conflicts with the passthrough remap target".into());
        }
        if resolve_target(name).is_some() {
            return Err(format!("Macro name '{name}' conflicts with a built-in target"));
        }
        if m.mode != "hold" && m.mode != "single" {
            return Err(format!("Macro '{name}': mode must be 'hold' or 'single'"));
        }
        if m.sequence.is_empty() {
            return Err(format!("Macro '{name}': sequence must not be empty"));
        }
        for step in &m.sequence {
            let step_target = resolve_step_target(&step.key);
            if step_target.is_none() {
                return Err(format!("Macro '{name}': unknown key '{}'", step.key));
            }
            if let Some(crate::mapping::StepTarget::Gamepad(btn)) = step_target {
                if btn == Button::Mic || btn == Button::L2Analog || btn == Button::R2Analog
                    || btn == Button::TouchpadLeft || btn == Button::TouchpadRight
                {
                    return Err(format!("Macro '{name}': invalid key '{}'", step.key));
                }
            }
            if step.release_ms <= step.press_ms {
                return Err(format!(
                    "Macro '{name}' step '{}': release_ms ({}) must be > press_ms ({})",
                    step.key, step.release_ms, step.press_ms
                ));
            }
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

    if cfg.version != 2 {
        return Err(format!("version must be 2, got {}", cfg.version));
    }

    Ok(())
}

#[allow(dead_code)]
pub fn default_content() -> &'static str {
    r#"# edgemap remap configuration
# Generated by: edgemap create-config
#
# ── Remap ──────────────────────────────────────
#   [button]
#   remap = "target"
#
#   Standard targets: cross  circle  square  triangle
#                     l1  l2  l3  r1  r2  r3
#                     options  create  ps
#                     dpad_up  dpad_down  dpad_left  dpad_right
#                     touchpad
#
#   Stick targets:    ls_up  ls_down  ls_left  ls_right
#                     rs_up  rs_down  rs_left  rs_right
#
#   Special targets:  block       — disable the button entirely
#                     combo       — modifier key combinations (see below)
#                     <macro_name> — timed key sequence (see below)
#
# ── Turbo (hold-to-repeat) ─────────────────────
#   [button]
#   remap = "cross"
#   turbo = true
#   turbo_interval_ms = 80      # toggle interval (default: 100)
#   turbo_delay_ms = 200        # delay before turbo starts (default: 0)
#
# ── Combo (modifier + key → output) ────────────
#   [modifier]
#   remap = "combo"
#   [[modifier.combos]]         # array of tables
#   key = "cross"
#   output = "circle"
#
# ── Macro (timed key sequence) ─────────────────
#   [button]
#   remap = "my_macro"
#   [macros.my_macro]
#   mode = "hold"               # "hold" loops while pressed, "single" runs once
#   sequence = [
#     { key = "cross",  press_ms = 0,   release_ms = 200 },
#     { key = "circle", press_ms = 100, release_ms = 300 },
#   ]
#
# ── Touchpad Split ─────────────────────────────
#   [touchpad]
#   remap = "split"
#   [touchpad_left]
#   remap = "dpad_left"
#   [touchpad_right]
#   remap = "dpad_right"
#
# ── Global Options ──────────────────────────────
#   output_device = "dualsense"    # force regular DualSense (was force_dualsense)
#   output_device = "dualshock4"   # emulate as DualShock 4

version = 2

# Face Buttons
[cross]
remap = "cross"
[circle]
remap = "circle"
[square]
remap = "square"
[triangle]
remap = "triangle"

# Shoulder Buttons + Triggers
[l1]
remap = "l1"
[r1]
remap = "r1"
[l2]
remap = "l2"
[r2]
remap = "r2"

# Stick Buttons
[l3]
remap = "l3"
[r3]
remap = "r3"

# System Buttons
[create]
remap = "create"
[options]
remap = "options"
[ps]
remap = "ps"
[touchpad]
remap = "touchpad"

# D-Pad
[dpad_up]
remap = "dpad_up"
[dpad_down]
remap = "dpad_down"
[dpad_left]
remap = "dpad_left"
[dpad_right]
remap = "dpad_right"

# DualSense Edge — Back Paddles + Function Keys
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
        let full = format!("version = 2\n{toml_str}");
        toml::from_str(&full).expect("test config should parse")
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
            "key:space", "key:a", "key:enter", "key:f1",
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
    fn keyboard_target_unknown_key() {
        assert!(validate(&parse("[cross]\nremap = \"key:banana\"\n"))
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
    fn analog_buttons_not_allowed_as_target() {
        for analog in &["l2_analog", "r2_analog"] {
            let cfg = parse(&format!("[cross]\nremap = \"{analog}\"\n"));
            assert!(validate(&cfg).unwrap_err().contains("unknown target"),
                "analog {analog} should not be a valid target");
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
        assert_eq!(cfg.output_device, "auto");
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn output_device_config() {
        let cfg = parse("output_device = \"dualshock4\"\n[cross]\nremap = \"cross\"\n");
        assert!(validate(&cfg).is_ok());
        assert_eq!(cfg.output_device, "dualshock4");

        let cfg = parse("output_device = \"dualsense\"\n[cross]\nremap = \"cross\"\n");
        assert!(validate(&cfg).is_ok());
        assert_eq!(cfg.output_device, "dualsense");

        let cfg = parse("output_device = \"nintendo_switch\"\n[cross]\nremap = \"cross\"\n");
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn unknown_field_rejected() {
        // garbage field inside a button section
        assert!(toml::from_str::<Config>("version = 2\n[cross]\nremap = \"cross\"\ngarbage = 123\n").is_err());
        // garbage field inside a combo
        assert!(toml::from_str::<Config>("version = 2\n[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"circle\"\nbad = 1\n").is_err());
        // garbage field inside a macro
        assert!(toml::from_str::<Config>("version = 2\n[left_paddle]\nremap = \"m\"\n[macros.m]\nbad = 1\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 100\n").is_err());
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

    // --- macro tests ---

    #[test]
    fn valid_macro_hold() {
        let cfg = parse("[left_paddle]\nremap = \"m\"\n[macros.m]\nmode = \"hold\"\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 200\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.macro_configs.len(), 1);
        assert!(matches!(mapping.macro_configs[0].mode, MacroMode::Hold));
    }

    #[test]
    fn valid_macro_single() {
        let cfg = parse("[left_paddle]\nremap = \"m\"\n[macros.m]\nmode = \"single\"\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 200\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert!(matches!(mapping.macro_configs[0].mode, MacroMode::Single));
    }

    #[test]
    fn macro_default_mode_hold() {
        let cfg = parse("[left_paddle]\nremap = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 200\n");
        let mapping = cfg.to_mapping_config().unwrap();
        assert!(matches!(mapping.macro_configs[0].mode, MacroMode::Hold));
    }

    #[test]
    fn macro_empty_sequence() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\n[macros.m]\nsequence = []\n")).unwrap_err();
        assert!(e.contains("must not be empty"));
    }

    #[test]
    fn macro_release_le_press() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 100\nrelease_ms = 50\n")).unwrap_err();
        assert!(e.contains("must be > press_ms"));
    }

    #[test]
    fn macro_unknown_key() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"banana\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("unknown key"));
    }

    #[test]
    fn macro_name_conflict() {
        let e = validate(&parse("[left_paddle]\nremap = \"cross\"\n[macros.cross]\n[[macros.cross.sequence]]\nkey = \"circle\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("conflicts with a standard button name"));
    }

    #[test]
    fn macro_turbo_mutex() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\nturbo = true\n[macros.m]\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("turbo and macros are mutually exclusive"));
    }

    #[test]
    fn macro_combo_output() {
        let cfg = parse("[left_paddle]\nremap = \"combo\"\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"circle\"\npress_ms = 0\nrelease_ms = 200\n");
        assert!(validate(&cfg).is_ok());
        let mapping = cfg.to_mapping_config().unwrap();
        assert_eq!(mapping.combo_configs.len(), 1);
        assert!(matches!(mapping.combo_configs[0].output, Target::Macro(_)));
    }

    #[test]
    fn macro_turbo_combo_mutex() {
        let e = validate(&parse("[left_paddle]\nremap = \"combo\"\nturbo = true\n[[left_paddle.combos]]\nkey = \"cross\"\noutput = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"circle\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("turbo and macros are mutually exclusive"));
    }

    #[test]
    fn macro_mode_invalid() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\n[macros.m]\nmode = \"banana\"\n[[macros.m.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("mode must be 'hold' or 'single'"));
    }

    #[test]
    fn macro_name_target_conflict() {
        let e = validate(&parse("[left_paddle]\nremap = \"l2_full\"\n[macros.l2_full]\n[[macros.l2_full.sequence]]\nkey = \"cross\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("conflicts with a built-in target"));
    }

    #[test]
    fn keyboard_macro_step_valid() {
        let cfg = parse("[left_paddle]\nremap = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"key:tab\"\npress_ms = 0\nrelease_ms = 100\n");
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn keyboard_macro_step_rejected() {
        let e = validate(&parse("[left_paddle]\nremap = \"m\"\n[macros.m]\n[[macros.m.sequence]]\nkey = \"key:bad\"\npress_ms = 0\nrelease_ms = 100\n")).unwrap_err();
        assert!(e.contains("unknown key"));
    }
}
