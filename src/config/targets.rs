use std::collections::HashMap;

use crate::mapping::{StickDir, Target, Trigger};
use crate::model::Button;

use super::MacroConfig;

pub(super) fn is_valid_src(name: &str) -> bool {
    Button::from_name(name).is_some() && name != "mic" && name != "l2_analog" && name != "r2_analog"
}

pub(super) fn is_valid_target(name: &str) -> bool {
    if matches!(name, "combo" | "passthrough") {
        return true;
    }
    if let Some(key) = name.strip_prefix("key:") {
        return !key.is_empty() && crate::keycodes::resolve_keycode(key).is_some();
    }
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

pub(super) fn resolve_target(name: &str) -> Option<Target> {
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

pub(super) fn resolve_target_or_macro(
    name: &str,
    macros: &HashMap<String, MacroConfig>,
) -> Option<Target> {
    if let Some(key) = name.strip_prefix("key:") {
        return crate::keycodes::resolve_keycode(key).map(Target::Keyboard);
    }
    resolve_target(name).or_else(|| {
        macros
            .contains_key(name)
            .then(|| Target::Macro(name.to_string()))
    })
}

pub(super) fn resolve_step_target(key: &str) -> Option<crate::mapping::StepTarget> {
    if let Some(kc) = key.strip_prefix("key:") {
        return crate::keycodes::resolve_keycode(kc).map(crate::mapping::StepTarget::Keyboard);
    }
    Button::from_name(key).map(crate::mapping::StepTarget::Gamepad)
}
