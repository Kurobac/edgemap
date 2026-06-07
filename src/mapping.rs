use crate::report::{Button, GamepadState};

#[derive(Debug, Clone)]
pub enum Trigger {
    L2,
    R2,
}

#[derive(Debug, Clone)]
pub enum StickDir {
    LsUp,
    LsDown,
    LsLeft,
    LsRight,
    RsUp,
    RsDown,
    RsLeft,
    RsRight,
}

#[derive(Debug, Clone)]
pub enum Target {
    Button(Button),
    TriggerFull(Trigger),
    Stick(StickDir),
    Macro(String),
    Keyboard(u16),
}

#[derive(Debug, Clone)]
pub struct RemapRule {
    pub src: Button,
    pub dst: Target,
}

impl RemapRule {
    pub fn new(src: Button, dst: Target) -> Self {
        Self { src, dst }
    }
}

#[derive(Debug, Clone)]
pub struct TurboConfig {
    pub src: Button,
    pub interval_ms: u64,
    pub delay_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ComboRule {
    pub modifier: Button,
    pub key: Button,
    pub output: Target,
}

#[derive(Debug, Clone)]
pub enum MacroMode {
    Hold,
    Single,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MacroSource {
    Physical,
    Combo,
}

#[derive(Debug, Clone)]
pub struct MacroStep {
    pub action: StepTarget,
    pub press_ms: u64,
    pub release_ms: u64,
}

#[derive(Debug, Clone)]
pub enum StepTarget {
    Gamepad(Button),
    Keyboard(u16),
}

#[derive(Debug, Clone)]
pub struct MacroRule {
    pub trigger: Button,
    pub name: String,
    pub mode: MacroMode,
    pub steps: Vec<MacroStep>,
    pub source: MacroSource,
}

#[derive(Debug, Clone, Default)]
pub struct MappingConfig {
    pub rules: Vec<RemapRule>,
    pub split_touchpad: bool,
    pub turbo_configs: Vec<TurboConfig>,
    pub blocked_buttons: Vec<Button>,
    pub combo_configs: Vec<ComboRule>,
    pub macro_configs: Vec<MacroRule>,
}

impl MappingConfig {
    pub fn from_rules_split(rules: Vec<RemapRule>, split_touchpad: bool) -> Self {
        Self { rules, split_touchpad, turbo_configs: Vec::new(), blocked_buttons: Vec::new(), combo_configs: Vec::new(), macro_configs: Vec::new() }
    }

    pub fn apply(&self, l1: &GamepadState, state: &mut GamepadState, keyboard_out: &mut Vec<(u16, bool)>) {
        let snapshot = l1.clone();
        let mut button_targets: Vec<Button> = Vec::new();

        for rule in &self.rules {
            if !snapshot.button(rule.src) {
                continue;
            }
            // Phase 1: clear source button (always)
            state.set_button(rule.src, false);
            match rule.src {
                Button::L2 => {
                    let analog = snapshot.l2_analog;
                    match &rule.dst {
                        Target::Button(Button::L2) => {
                            // self-map: keep analog as-is
                        }
                        Target::Button(Button::R2) | Target::TriggerFull(Trigger::R2) => {
                            // transfer to R2
                            state.r2_analog = analog;
                            state.l2_analog = 0;
                        }
                        Target::TriggerFull(Trigger::L2) => {
                            // TriggerFull sets analog below, clear source
                            state.l2_analog = 0;
                        }
                        _ => {
                            // non-trigger target
                            state.l2_analog = 0;
                        }
                    }
                }
                Button::R2 => {
                    let analog = snapshot.r2_analog;
                    match &rule.dst {
                        Target::Button(Button::R2) => {
                            // self-map: keep analog as-is
                        }
                        Target::Button(Button::L2) | Target::TriggerFull(Trigger::L2) => {
                            // transfer to L2
                            state.l2_analog = analog;
                            state.r2_analog = 0;
                        }
                        Target::TriggerFull(Trigger::R2) => {
                            // TriggerFull sets analog below, clear source
                            state.r2_analog = 0;
                        }
                        _ => {
                            // non-trigger target
                            state.r2_analog = 0;
                        }
                    }
                }
                _ => {}
            }

            match &rule.dst {
                Target::Button(btn) => {
                    button_targets.push(*btn);
                }
                Target::TriggerFull(trigger) => {
                    match trigger {
                        Trigger::L2 => {
                            state.set_button(Button::L2, true);
                            state.l2_analog = 255;
                        }
                        Trigger::R2 => {
                            state.set_button(Button::R2, true);
                            state.r2_analog = 255;
                        }
                    }
                }
                Target::Stick(dir) => {
                    match dir {
                        StickDir::LsUp => state.left_stick_y = 0,
                        StickDir::LsDown => state.left_stick_y = 255,
                        StickDir::LsLeft => state.left_stick_x = 0,
                        StickDir::LsRight => state.left_stick_x = 255,
                        StickDir::RsUp => state.right_stick_y = 0,
                        StickDir::RsDown => state.right_stick_y = 255,
                        StickDir::RsLeft => state.right_stick_x = 0,
                        StickDir::RsRight => state.right_stick_x = 255,
                    }
                }
                Target::Macro(_) => {}
                Target::Keyboard(code) => keyboard_out.push((*code, true)),
            }
        }

        // Phase 2: set all deferred button targets atomically
        for btn in &button_targets {
            state.set_button(*btn, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> GamepadState { GamepadState::default() }

    #[test]
    fn single_remap() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn multi_key() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Square, Target::Button(Button::Triangle)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        s.set_button(Button::Square, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(!s.button(Button::Square));
        assert!(s.button(Button::Circle));
        assert!(s.button(Button::Triangle));
    }

    #[test]
    fn cross_map_both_pressed() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        s.set_button(Button::Circle, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        // deferred targets: both circle and cross are set in Phase 2
        assert!(s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn cross_map_one_pressed() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn self_map_passthrough() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Cross)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::Cross)); // self-map preserves
    }

    #[test]
    fn trigger_self_map_preserves_analog() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::L2, Target::Button(Button::L2)),
        ], false);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 128;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 128); // self-map preserves analog
    }

    #[test]
    fn trigger_swap_transfers_analog() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::L2, Target::Button(Button::R2)),
        ], false);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 100;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::L2));
        assert!(s.button(Button::R2));
        assert_eq!(s.l2_analog, 0);   // source cleared
        assert_eq!(s.r2_analog, 100); // transferred
    }

    #[test]
    fn trigger_l2_full() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::TriggerFull(Trigger::L2)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 255);
    }

    #[test]
    fn trigger_r2_full() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Circle, Target::TriggerFull(Trigger::R2)),
        ], false);
        let mut s = state();
        s.set_button(Button::Circle, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Circle));
        assert!(s.button(Button::R2));
        assert_eq!(s.r2_analog, 255);
    }

    #[test]
    fn stick_directions() {
        fn ls_y(s: &GamepadState) -> u8 { s.left_stick_y }
        fn ls_x(s: &GamepadState) -> u8 { s.left_stick_x }
        fn rs_y(s: &GamepadState) -> u8 { s.right_stick_y }
        fn rs_x(s: &GamepadState) -> u8 { s.right_stick_x }

        let cases: Vec<(StickDir, fn(&GamepadState) -> u8, u8)> = vec![
            (StickDir::LsUp,    ls_y, 0),
            (StickDir::LsDown,  ls_y, 255),
            (StickDir::LsLeft,  ls_x, 0),
            (StickDir::LsRight, ls_x, 255),
            (StickDir::RsUp,    rs_y, 0),
            (StickDir::RsDown,  rs_y, 255),
            (StickDir::RsLeft,  rs_x, 0),
            (StickDir::RsRight, rs_x, 255),
        ];
        let base = state();
        for (dir, getter, expected) in cases {
            let mut s = base.clone();
            s.set_button(Button::Cross, true);
            let cfg = MappingConfig::from_rules_split(vec![
                RemapRule::new(Button::Cross, Target::Stick(dir.clone())),
            ], false);
            cfg.apply(&s.clone(), &mut s, &mut Vec::new());
            assert!(!s.button(Button::Cross));
            assert_eq!(getter(&s), expected, "dir={:?}", dir);
        }
    }

    #[test]
    fn trigger_source_clears_analog() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::L2, Target::Button(Button::Cross)),
        ], false);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 128;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::L2));
        assert!(s.button(Button::Cross));
        assert_eq!(s.l2_analog, 0); // analog cleared
    }

    #[test]
    fn r2_source_clears_analog() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::R2, Target::Button(Button::Circle)),
        ], false);
        let mut s = state();
        s.set_button(Button::R2, true);
        s.r2_analog = 200;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::R2));
        assert!(s.button(Button::Circle));
        assert_eq!(s.r2_analog, 0);
    }

    #[test]
    fn no_matching_source_unchanged() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
        ], false);
        let mut s = state();
        s.set_button(Button::Square, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::Square)); // untouched
        assert!(!s.button(Button::Circle)); // no remap triggered
    }

    #[test]
    fn snapshot_isolation() {
        // A→B and B→A should use physical state, not intermediate results
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Square)),
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        // Circle NOT pressed physically
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        // cross→circle fires (cross was pressed)
        // circle→square should NOT fire (circle was NOT physically pressed)
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
        assert!(!s.button(Button::Square)); // not triggered via cascade
    }

    #[test]
    fn remap_to_keyboard() {
        let cfg = MappingConfig::from_rules_split(vec![
            RemapRule::new(Button::Cross, Target::Keyboard(57)), // KEY_SPACE
        ], false);
        let mut s = state();
        s.set_button(Button::Cross, true);
        let mut kb: Vec<(u16, bool)> = Vec::new();
        cfg.apply(&s.clone(), &mut s, &mut kb);
        assert!(!s.button(Button::Cross));
        assert_eq!(kb, vec![(57, true)]);
    }
}
