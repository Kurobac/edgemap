use std::collections::{HashMap, HashSet};

use crate::model::Button;

use super::targets::{
    into_mapping_target, is_reserved_macro_name, is_valid_src, parse_target, resolve_step_target,
    ParsedTarget,
};
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
        let parsed_remap = if remap.is_empty() {
            Some(ParsedTarget::Passthrough)
        } else {
            parse_target(remap, &cfg.macros)
        };

        if btn_name == "touchpad" && matches!(parsed_remap, Some(ParsedTarget::Split)) {
            has_split = true;
            continue;
        }

        let is_combo = btn_name != "touchpad_left"
            && btn_name != "touchpad_right"
            && matches!(parsed_remap, Some(ParsedTarget::Combo));
        let has_combos = !btn_conf.combos.is_empty();

        if matches!(btn_name.as_str(), "touchpad_left" | "touchpad_right")
            && matches!(parsed_remap, Some(ParsedTarget::Combo))
        {
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
        for combo in &btn_conf.combos {
            let key_btn = match Button::from_name(&combo.key) {
                Some(button) => button,
                None => return Err(format!("[{btn_name}] unknown combo key: {}", combo.key)),
            };
            if key_btn.name() == btn_name.as_str() {
                return Err(format!(
                    "[{btn_name}] combo key cannot be the same as the modifier button"
                ));
            }
            if matches!(
                key_btn,
                Button::Mic
                    | Button::L2Analog
                    | Button::R2Analog
                    | Button::TouchpadLeft
                    | Button::TouchpadRight
            ) {
                return Err(format!("[{btn_name}] invalid combo key: {}", combo.key));
            }

            let parsed_output = parse_target(&combo.output, &cfg.macros)
                .ok_or_else(|| format!("[{btn_name}] unknown combo output: {}", combo.output))?;
            if matches!(parsed_output, ParsedTarget::Passthrough) {
                return Err(format!("[{btn_name}] combo output cannot be passthrough"));
            }
            if into_mapping_target(parsed_output).is_none() {
                return Err(format!(
                    "[{btn_name}] unknown combo output: {}",
                    combo.output
                ));
            }
            if !seen_keys.insert(key_btn) {
                return Err(format!("[{btn_name}] duplicate combo key '{}'", combo.key));
            }
            let is_face = matches!(
                key_btn,
                Button::Cross | Button::Circle | Button::Square | Button::Triangle
            );
            if is_fn_modifier && is_face {
                return Err(format!(
                    "[{btn_name}] FN+face combos ({}+{}) conflict with firmware profile switching",
                    btn_name, combo.key
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

        if btn_conf.turbo && btn_conf.turbo_interval_ms == 0 {
            return Err(format!(
                "[{btn_name}] turbo_interval_ms must be greater than 0"
            ));
        }

        if btn_conf.turbo {
            let has_macro_output = match &parsed_remap {
                Some(ParsedTarget::MacroRef(_)) => true,
                Some(ParsedTarget::Combo) => btn_conf.combos.iter().any(|combo| {
                    matches!(
                        parse_target(&combo.output, &cfg.macros),
                        Some(ParsedTarget::MacroRef(_))
                    )
                }),
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

        match parsed_remap {
            Some(ParsedTarget::Passthrough | ParsedTarget::Block | ParsedTarget::Combo) => {}
            Some(parsed) if into_mapping_target(parsed.clone()).is_some() => {}
            _ => return Err(format!("[{btn_name}] unknown target: {remap}")),
        }
    }

    let no_macros = HashMap::new();
    for (name, macro_config) in &cfg.macros {
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
        if matches!(
            parse_target(name, &no_macros),
            Some(ParsedTarget::TriggerFull(_) | ParsedTarget::Stick(_))
        ) {
            return Err(format!(
                "Macro name '{name}' conflicts with a built-in target"
            ));
        }
        if is_reserved_macro_name(name) {
            return Err(format!("Macro name '{name}' is reserved"));
        }
        if macro_config.mode != "hold" && macro_config.mode != "single" {
            return Err(format!("Macro '{name}': mode must be 'hold' or 'single'"));
        }
        if macro_config.sequence.is_empty() {
            return Err(format!("Macro '{name}': sequence must not be empty"));
        }
        for step in &macro_config.sequence {
            if resolve_step_target(&step.key).is_none() {
                return Err(format!("Macro '{name}': unknown key '{}'", step.key));
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
        for child in ["touchpad_left", "touchpad_right"] {
            let remap = cfg
                .buttons
                .get(child)
                .and_then(|config| config.remap.as_deref())
                .unwrap_or("block");
            let parsed = parse_target(remap, &cfg.macros)
                .ok_or_else(|| format!("Unknown target '{remap}' for {child}"))?;
            if matches!(parsed, ParsedTarget::Block) {
                return Err(format!(
                    "{child}: remap=\"block\" is not allowed in split mode"
                ));
            }
            if into_mapping_target(parsed).is_none() {
                return Err(format!("Unknown target '{remap}' for {child}"));
            }
        }
    } else if has_touch_left || has_touch_right {
        return Err("touchpad_left/right require [touchpad] remap = \"split\"".into());
    }

    if cfg.version != 2 {
        return Err(format!("version must be 2, got {}", cfg.version));
    }

    cfg.to_mapping_config().map(|_| ())
}
