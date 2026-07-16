use std::fmt::Write;

use crate::keycodes::{resolve_keycode, KEYCODE_NAMES};

pub const VERSION: u32 = 1;

pub const OUTPUT_DEVICES: &[&str] = &["auto", "dualsense", "dualshock4"];

pub const SOURCE_BUTTONS: &[&str] = &[
    "cross",
    "circle",
    "square",
    "triangle",
    "l1",
    "r1",
    "l2",
    "r2",
    "l3",
    "r3",
    "create",
    "options",
    "ps",
    "touchpad",
    "touchpad_left",
    "touchpad_right",
    "dpad_up",
    "dpad_down",
    "dpad_left",
    "dpad_right",
    "left_paddle",
    "right_paddle",
    "left_fn",
    "right_fn",
];

pub const GAMEPAD_TARGETS: &[&str] = &[
    "cross",
    "circle",
    "square",
    "triangle",
    "l1",
    "r1",
    "l2",
    "r2",
    "l3",
    "r3",
    "options",
    "create",
    "ps",
    "dpad_up",
    "dpad_down",
    "dpad_left",
    "dpad_right",
    "touchpad",
];

pub const DIRECTION_TARGETS: &[&str] = &[
    "ls_up", "ls_down", "ls_left", "ls_right", "rs_up", "rs_down", "rs_left", "rs_right",
];

pub const TRIGGER_TARGETS: &[&str] = &["l2_full", "r2_full"];

pub const EDGE_ACTION_BUTTONS: &[&str] = &["left_paddle", "right_paddle", "left_fn", "right_fn"];

fn write_string_array(output: &mut String, name: &str, values: &[&str]) {
    write!(output, "{name} = [").unwrap();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        write!(output, "\"{value}\"").unwrap();
    }
    output.push_str("]\n");
}

pub fn to_toml() -> String {
    let mut output = format!("version = {VERSION}\n");
    write_string_array(&mut output, "output_devices", OUTPUT_DEVICES);
    write_string_array(&mut output, "source_buttons", SOURCE_BUTTONS);

    let mut remap_targets = vec!["block", "passthrough", "combo"];
    remap_targets.extend_from_slice(GAMEPAD_TARGETS);
    remap_targets.extend_from_slice(DIRECTION_TARGETS);
    remap_targets.extend_from_slice(TRIGGER_TARGETS);
    write_string_array(&mut output, "remap_targets", &remap_targets);

    let mut action_buttons = GAMEPAD_TARGETS.to_vec();
    action_buttons.extend_from_slice(EDGE_ACTION_BUTTONS);
    write_string_array(&mut output, "combo_keys", &action_buttons);

    let mut combo_outputs = GAMEPAD_TARGETS.to_vec();
    combo_outputs.extend_from_slice(DIRECTION_TARGETS);
    combo_outputs.extend_from_slice(TRIGGER_TARGETS);
    write_string_array(&mut output, "combo_outputs", &combo_outputs);
    write_string_array(&mut output, "macro_step_buttons", &action_buttons);

    let mut reserved_macro_names = SOURCE_BUTTONS.to_vec();
    reserved_macro_names.extend_from_slice(DIRECTION_TARGETS);
    reserved_macro_names.extend_from_slice(TRIGGER_TARGETS);
    reserved_macro_names.extend_from_slice(&[
        "block",
        "combo",
        "macro",
        "mic",
        "l2_analog",
        "r2_analog",
        "passthrough",
    ]);
    reserved_macro_names.sort_unstable();
    reserved_macro_names.dedup();
    debug_assert!(reserved_macro_names
        .iter()
        .all(|name| crate::config::is_reserved_macro_name(name)));
    write_string_array(&mut output, "reserved_macro_names", &reserved_macro_names);

    for name in KEYCODE_NAMES {
        let code = resolve_keycode(name).expect("canonical key name must resolve");
        write!(
            output,
            "\n[[keyboard_keys]]\nname = \"{name}\"\ncode = {code}\n"
        )
        .unwrap();
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn capabilities_toml_is_parseable_and_complete() {
        let output = to_toml();
        let parsed: toml::Value = toml::from_str(&output).unwrap();
        assert_eq!(parsed["version"].as_integer(), Some(VERSION.into()));
        assert_eq!(
            parsed["source_buttons"].as_array().unwrap().len(),
            SOURCE_BUTTONS.len()
        );
        assert_eq!(
            parsed["keyboard_keys"].as_array().unwrap().len(),
            KEYCODE_NAMES.len()
        );
        assert_eq!(parsed["keyboard_keys"][0]["name"].as_str(), Some("a"));
        assert_eq!(
            parsed["keyboard_keys"][KEYCODE_NAMES.len() - 1]["name"].as_str(),
            Some("nextsong")
        );
    }

    #[test]
    fn capability_lists_match_config_source_and_target_rules() {
        for target in GAMEPAD_TARGETS
            .iter()
            .chain(DIRECTION_TARGETS)
            .chain(TRIGGER_TARGETS)
        {
            assert!(crate::config::targets::is_valid_target(target));
        }
        for source in SOURCE_BUTTONS {
            assert!(crate::config::targets::is_valid_src(source));
        }
        assert!(SOURCE_BUTTONS.contains(&"touchpad_left"));
        assert!(SOURCE_BUTTONS.contains(&"touchpad_right"));

        let advertised_sources: HashSet<_> = SOURCE_BUTTONS.iter().copied().collect();
        let valid_sources: HashSet<_> = crate::model::ALL_BUTTONS
            .iter()
            .map(|button| button.name())
            .filter(|name| crate::config::targets::is_valid_src(name))
            .collect();
        assert_eq!(advertised_sources, valid_sources);

        let advertised_gamepad_targets: HashSet<_> = GAMEPAD_TARGETS.iter().copied().collect();
        let valid_gamepad_targets: HashSet<_> = crate::model::ALL_BUTTONS
            .iter()
            .map(|button| button.name())
            .filter(|name| crate::config::targets::is_valid_target(name))
            .collect();
        assert_eq!(advertised_gamepad_targets, valid_gamepad_targets);
    }

    #[test]
    fn advertised_macro_names_match_validator_reservations() {
        let parsed: toml::Value = toml::from_str(&to_toml()).unwrap();
        let reserved = parsed["reserved_macro_names"].as_array().unwrap();

        for name in reserved {
            assert!(crate::config::is_reserved_macro_name(
                name.as_str().unwrap()
            ));
        }
        for name in ["block", "combo", "macro", "passthrough"] {
            assert!(reserved.iter().any(|value| value.as_str() == Some(name)));
        }
    }
}
