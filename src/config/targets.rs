use std::collections::HashMap;

use crate::mapping::{StepTarget, StickDir, Target, Trigger};
use crate::model::Button;

use super::MacroConfig;

#[derive(Debug, Clone)]
pub(super) enum ParsedTarget {
    Passthrough,
    Block,
    Split,
    Combo,
    Button(Button),
    TriggerFull(Trigger),
    Stick(StickDir),
    Keyboard(u16),
    MacroRef(String),
}

pub(crate) fn is_valid_src(name: &str) -> bool {
    Button::from_name(name).is_some() && name != "mic" && name != "l2_analog" && name != "r2_analog"
}

fn is_allowed_button_target(button: Button) -> bool {
    !matches!(
        button,
        Button::FnLeft
            | Button::FnRight
            | Button::LeftPaddle
            | Button::RightPaddle
            | Button::Mic
            | Button::TouchpadLeft
            | Button::TouchpadRight
            | Button::L2Analog
            | Button::R2Analog
    )
}

pub(super) fn parse_target(
    name: &str,
    macros: &HashMap<String, MacroConfig>,
) -> Option<ParsedTarget> {
    let parsed = match name {
        "passthrough" => ParsedTarget::Passthrough,
        "block" => ParsedTarget::Block,
        "split" => ParsedTarget::Split,
        "combo" => ParsedTarget::Combo,
        "l2_full" => ParsedTarget::TriggerFull(Trigger::L2),
        "r2_full" => ParsedTarget::TriggerFull(Trigger::R2),
        "ls_up" => ParsedTarget::Stick(StickDir::LsUp),
        "ls_down" => ParsedTarget::Stick(StickDir::LsDown),
        "ls_left" => ParsedTarget::Stick(StickDir::LsLeft),
        "ls_right" => ParsedTarget::Stick(StickDir::LsRight),
        "rs_up" => ParsedTarget::Stick(StickDir::RsUp),
        "rs_down" => ParsedTarget::Stick(StickDir::RsDown),
        "rs_left" => ParsedTarget::Stick(StickDir::RsLeft),
        "rs_right" => ParsedTarget::Stick(StickDir::RsRight),
        _ => {
            if let Some(key) = name.strip_prefix("key:") {
                return crate::keycodes::resolve_keycode(key).map(ParsedTarget::Keyboard);
            }
            if let Some(button) = Button::from_name(name) {
                ParsedTarget::Button(button)
            } else if macros.contains_key(name) {
                ParsedTarget::MacroRef(name.to_string())
            } else {
                return None;
            }
        }
    };
    Some(parsed)
}

pub(super) fn into_mapping_target(parsed: ParsedTarget) -> Option<Target> {
    match parsed {
        ParsedTarget::Button(button) if is_allowed_button_target(button) => {
            Some(Target::Button(button))
        }
        ParsedTarget::TriggerFull(trigger) => Some(Target::TriggerFull(trigger)),
        ParsedTarget::Stick(direction) => Some(Target::Stick(direction)),
        ParsedTarget::Keyboard(keycode) => Some(Target::Keyboard(keycode)),
        ParsedTarget::MacroRef(name) => Some(Target::Macro(name)),
        ParsedTarget::Passthrough
        | ParsedTarget::Block
        | ParsedTarget::Split
        | ParsedTarget::Combo
        | ParsedTarget::Button(_) => None,
    }
}

#[cfg(test)]
pub(crate) fn is_valid_target(name: &str) -> bool {
    let macros = HashMap::new();
    match parse_target(name, &macros) {
        Some(ParsedTarget::Passthrough | ParsedTarget::Combo) => true,
        Some(parsed) => into_mapping_target(parsed).is_some(),
        None => false,
    }
}

pub fn is_reserved_macro_name(name: &str) -> bool {
    if name.starts_with("key:") || name == "macro" {
        return true;
    }
    parse_target(name, &HashMap::new()).is_some()
}

pub(super) fn resolve_step_target(key: &str) -> Option<StepTarget> {
    match parse_target(key, &HashMap::new())? {
        ParsedTarget::Button(button)
            if !matches!(
                button,
                Button::Mic
                    | Button::L2Analog
                    | Button::R2Analog
                    | Button::TouchpadLeft
                    | Button::TouchpadRight
            ) =>
        {
            Some(StepTarget::Gamepad(button))
        }
        ParsedTarget::Keyboard(keycode) => Some(StepTarget::Keyboard(keycode)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(target: &ParsedTarget) -> &'static str {
        match target {
            ParsedTarget::Passthrough => "passthrough",
            ParsedTarget::Block => "block",
            ParsedTarget::Split => "split",
            ParsedTarget::Combo => "combo",
            ParsedTarget::Button(_) => "button",
            ParsedTarget::TriggerFull(_) => "trigger",
            ParsedTarget::Stick(_) => "stick",
            ParsedTarget::Keyboard(_) => "keyboard",
            ParsedTarget::MacroRef(_) => "macro",
        }
    }

    #[test]
    fn target_parser_covers_every_typed_variant() {
        let mut macros = HashMap::new();
        macros.insert("my_macro".to_string(), MacroConfig::default());

        for (input, expected) in [
            ("passthrough", "passthrough"),
            ("block", "block"),
            ("split", "split"),
            ("combo", "combo"),
            ("cross", "button"),
            ("l2_full", "trigger"),
            ("ls_up", "stick"),
            ("key:a", "keyboard"),
            ("my_macro", "macro"),
        ] {
            let parsed = parse_target(input, &macros).unwrap();
            assert_eq!(kind(&parsed), expected, "target {input}");
        }
    }

    #[test]
    fn key_namespace_never_reaches_macro_lookup() {
        let mut macros = HashMap::new();
        macros.insert("key:not-a-real-key".to_string(), MacroConfig::default());

        assert!(parse_target("key:not-a-real-key", &macros).is_none());
        assert!(is_reserved_macro_name("key:not-a-real-key"));
    }

    #[test]
    fn macro_step_context_reuses_typed_parser_and_filters_internal_buttons() {
        assert!(matches!(
            resolve_step_target("cross"),
            Some(StepTarget::Gamepad(Button::Cross))
        ));
        assert!(matches!(
            resolve_step_target("key:space"),
            Some(StepTarget::Keyboard(_))
        ));
        for invalid in [
            "mic",
            "l2_analog",
            "r2_analog",
            "touchpad_left",
            "touchpad_right",
            "l2_full",
            "combo",
        ] {
            assert!(
                resolve_step_target(invalid).is_none(),
                "macro step {invalid} should be rejected"
            );
        }
        for edge in ["left_paddle", "right_paddle", "left_fn", "right_fn"] {
            assert!(
                matches!(resolve_step_target(edge), Some(StepTarget::Gamepad(_))),
                "macro step {edge} should remain valid"
            );
        }
    }
}
