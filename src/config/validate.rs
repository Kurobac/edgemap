use std::collections::HashSet;

use crate::mapping::StepTarget;
use crate::report::Button;

use super::targets::{is_valid_src, is_valid_target, resolve_step_target, resolve_target};
use super::Config;

pub fn validate(cfg: &Config) -> Result<(), String> {
    if !matches!(
        cfg.output_device.as_str(),
        "auto" | "dualsense" | "dualshock4"
    ) {
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
        if let Some(btn) = Button::from_name(btn_name) {
            if btn_name.to_lowercase() == btn.name() && btn_name != btn.name() {
                return Err(format!(
                    "[{btn_name}] section names must be lowercase (use \"{}\")",
                    btn.name()
                ));
            }
        }
        let btn_conf = &cfg.buttons[btn_name];
        let remap = btn_conf.remap.as_deref().unwrap_or("");

        if btn_name == "touchpad" && remap == "split" {
            has_split = true;
            continue;
        }

        let is_combo =
            btn_name != "touchpad_left" && btn_name != "touchpad_right" && remap == "combo";
        let has_combos = !btn_conf.combos.is_empty();

        if matches!(btn_name.as_str(), "touchpad_left" | "touchpad_right") && remap == "combo" {
            return Err(format!(
                "[{btn_name}] touchpad partitions cannot use combo mode"
            ));
        }
        if is_combo && btn_conf.combos.is_empty() {
            return Err(format!(
                "[{btn_name}] remap=\"combo\" requires at least one combo entry"
            ));
        }
        if !is_combo && has_combos {
            return Err(format!(
                "[{btn_name}] remap and combos are mutually exclusive (use remap=\"combo\" with combos)"
            ));
        }

        let mut seen_keys = HashSet::new();
        let is_fn_modifier = btn_name == "left_fn" || btn_name == "right_fn";
        for c in &btn_conf.combos {
            let key_btn = match Button::from_name(&c.key) {
                Some(b) => b,
                None => return Err(format!("[{btn_name}] unknown combo key: {}", c.key)),
            };
            if key_btn.name() == btn_name.as_str() {
                return Err(format!(
                    "[{btn_name}] combo key cannot be the same as the modifier button"
                ));
            }
            if key_btn == Button::Mic
                || key_btn == Button::L2Analog
                || key_btn == Button::R2Analog
                || key_btn == Button::TouchpadLeft
                || key_btn == Button::TouchpadRight
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

        if btn_conf.turbo && matches!(btn_name.as_str(), "l2" | "r2") {
            let target_is_trigger = matches!(remap, "l2" | "r2") || remap.is_empty();
            if target_is_trigger {
                return Err(format!(
                    "[{btn_name}] turbo with trigger target '{remap}' is not supported"
                ));
            }
        }

        if btn_conf.turbo {
            let has_macro_output = match btn_conf.remap.as_deref() {
                Some(r) if cfg.macros.contains_key(r) => true,
                Some("combo") => btn_conf
                    .combos
                    .iter()
                    .any(|c| cfg.macros.contains_key(&c.output)),
                _ => false,
            };
            if has_macro_output {
                return Err(format!(
                    "[{btn_name}] turbo and macros are mutually exclusive"
                ));
            }
            if btn_conf.remap.as_deref() == Some("passthrough") {
                return Err(format!(
                    "[{btn_name}] turbo and passthrough are mutually exclusive"
                ));
            }
        }

        if btn_name == "touchpad_left" {
            has_touch_left = true;
        }
        if btn_name == "touchpad_right" {
            has_touch_right = true;
        }

        if remap != "block"
            && !remap.is_empty()
            && !is_valid_target(remap)
            && !cfg.macros.contains_key(remap)
        {
            return Err(format!("[{btn_name}] unknown target: {remap}"));
        }
    }

    for (name, m) in &cfg.macros {
        if Button::from_name(name).is_some() {
            return Err(format!(
                "Macro name '{name}' conflicts with a standard button name"
            ));
        }
        if name == "passthrough" {
            return Err(
                "Macro name 'passthrough' conflicts with the passthrough remap target".into(),
            );
        }
        if resolve_target(name).is_some() {
            return Err(format!(
                "Macro name '{name}' conflicts with a built-in target"
            ));
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
            if let Some(StepTarget::Gamepad(btn)) = step_target {
                if btn == Button::Mic
                    || btn == Button::L2Analog
                    || btn == Button::R2Analog
                    || btn == Button::TouchpadLeft
                    || btn == Button::TouchpadRight
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
        let left_rm = cfg
            .buttons
            .get("touchpad_left")
            .and_then(|c| c.remap.as_deref())
            .unwrap_or("block");
        let right_rm = cfg
            .buttons
            .get("touchpad_right")
            .and_then(|c| c.remap.as_deref())
            .unwrap_or("block");
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
