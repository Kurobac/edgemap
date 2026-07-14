mod compile;
mod io;
mod schema;
mod targets;
mod validate;

pub use io::{ActiveConfig, MAX_CONFIG_FILE_SIZE};
pub use schema::{ButtonConfig, ComboConfig, Config, MacroConfig, MacroStep};

pub use validate::validate;
pub const ALL_BUTTON_NAMES: &[&str] = &[
    "square",
    "cross",
    "circle",
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
    "left_paddle",
    "right_paddle",
    "left_fn",
    "right_fn",
];

#[allow(dead_code)]
pub fn default_content() -> &'static str {
    include_str!("default.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{MacroMode, Target};
    use crate::report::Button;

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
            "cross",
            "circle",
            "square",
            "triangle",
            "l1",
            "l2",
            "l3",
            "r1",
            "r2",
            "r3",
            "options",
            "create",
            "ps",
            "dpad_up",
            "dpad_down",
            "dpad_left",
            "dpad_right",
            "touchpad",
            "left_paddle",
            "right_paddle",
            "left_fn",
            "right_fn",
        ] {
            let cfg = parse(&format!("[{src}]\nremap = \"cross\"\n"));
            assert!(validate(&cfg).is_ok(), "source {src} should be valid");
        }
    }

    #[test]
    fn all_valid_targets() {
        for target in &[
            "cross",
            "circle",
            "square",
            "triangle",
            "l1",
            "l2",
            "l3",
            "r1",
            "r2",
            "r3",
            "options",
            "create",
            "ps",
            "dpad_up",
            "dpad_down",
            "dpad_left",
            "dpad_right",
            "touchpad",
            "l2_full",
            "r2_full",
            "ls_up",
            "ls_down",
            "ls_left",
            "ls_right",
            "rs_up",
            "rs_down",
            "rs_left",
            "rs_right",
            "key:space",
            "key:a",
            "key:enter",
            "key:f1",
        ] {
            let cfg = parse(&format!("[cross]\nremap = \"{target}\"\n"));
            assert!(validate(&cfg).is_ok(), "target {target} should be valid");
        }
    }

    #[test]
    fn unknown_source() {
        assert!(validate(&parse("[banana]\nremap = \"l1\"\n"))
            .unwrap_err()
            .contains("Unknown source button"));
    }

    #[test]
    fn unknown_target() {
        assert!(validate(&parse("[cross]\nremap = \"nope\"\n"))
            .unwrap_err()
            .contains("unknown target"));
    }

    #[test]
    fn keyboard_target_unknown_key() {
        assert!(validate(&parse("[cross]\nremap = \"key:banana\"\n"))
            .unwrap_err()
            .contains("unknown target"));
    }

    #[test]
    fn mic_not_allowed_as_source() {
        assert!(validate(&parse("[mic]\nremap = \"cross\"\n"))
            .unwrap_err()
            .contains("Unknown source button: mic"));
    }

    #[test]
    fn mic_not_allowed_as_target() {
        assert!(validate(&parse("[cross]\nremap = \"mic\"\n"))
            .unwrap_err()
            .contains("unknown target"));
    }

    #[test]
    fn edge_buttons_not_allowed_as_target() {
        for edge in &["left_paddle", "right_paddle", "left_fn", "right_fn"] {
            let cfg = parse(&format!("[cross]\nremap = \"{edge}\"\n"));
            assert!(
                validate(&cfg).unwrap_err().contains("unknown target"),
                "edge button {edge} should not be a valid target"
            );
        }
    }

    #[test]
    fn analog_buttons_not_allowed_as_target() {
        for analog in &["l2_analog", "r2_analog"] {
            let cfg = parse(&format!("[cross]\nremap = \"{analog}\"\n"));
            assert!(
                validate(&cfg).unwrap_err().contains("unknown target"),
                "analog {analog} should not be a valid target"
            );
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
            .unwrap_err()
            .contains("section names must be lowercase"));
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
    fn active_config_read_rejects_non_regular_and_oversized_files() {
        let non_regular = ActiveConfig::read("/dev/null").unwrap_err();
        assert!(non_regular.contains("not a regular file"));

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dseuhid-oversized-config-{}-{unique}.toml",
            std::process::id()
        ));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_CONFIG_FILE_SIZE as u64 + 1).unwrap();
        drop(file);

        let error = ActiveConfig::read(path.to_str().unwrap()).unwrap_err();
        assert!(error.contains("exceeds"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn active_config_keeps_the_content_that_was_read() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dseuhid-active-config-{}-{unique}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "version = 2\n[cross]\nremap = \"circle\"\n").unwrap();

        let active = ActiveConfig::read(path.to_str().unwrap()).unwrap();
        std::fs::write(&path, "not valid TOML").unwrap();

        let config = active.parse().unwrap();
        assert_eq!(config.buttons["cross"].remap.as_deref(), Some("circle"));
        std::fs::remove_file(path).unwrap();
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
        assert!(toml::from_str::<Config>(
            "version = 2\n[cross]\nremap = \"cross\"\ngarbage = 123\n"
        )
        .is_err());
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
        let e = validate(&parse(
            "[left_paddle]\nremap = \"m\"\n[macros.m]\nsequence = []\n",
        ))
        .unwrap_err();
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
