use std::collections::HashSet;

use crate::model::{Button, GamepadState};

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

#[derive(Debug, Default)]
pub struct OutputIntent {
    buttons: HashSet<Button>,
    keyboard: HashSet<u16>,
    l2_analog: Option<u8>,
    r2_analog: Option<u8>,
    l2_cleared: bool,
    r2_cleared: bool,
}

impl OutputIntent {
    fn contribute_analog(&mut self, trigger: Trigger, value: u8) {
        let target = match trigger {
            Trigger::L2 => &mut self.l2_analog,
            Trigger::R2 => &mut self.r2_analog,
        };
        *target = Some(target.map_or(value, |current| current.max(value)));
    }

    fn clear_source(&mut self, state: &mut GamepadState, source: Button) {
        state.set_button(source, false);
        match source {
            Button::L2 => self.l2_cleared = true,
            Button::R2 => self.r2_cleared = true,
            _ => {}
        }
    }

    pub fn press_target(&mut self, state: &mut GamepadState, target: &Target) {
        match target {
            Target::Button(button) => {
                self.buttons.insert(*button);
            }
            Target::TriggerFull(trigger) => {
                let button = match trigger {
                    Trigger::L2 => Button::L2,
                    Trigger::R2 => Button::R2,
                };
                self.buttons.insert(button);
                self.contribute_analog(trigger.clone(), 255);
            }
            Target::Stick(direction) => apply_stick_target(state, direction),
            Target::Keyboard(code) => {
                self.keyboard.insert(*code);
            }
            Target::Macro(_) => {}
        }
    }

    pub fn press_step(&mut self, target: &StepTarget) {
        match target {
            StepTarget::Gamepad(button) => {
                self.buttons.insert(*button);
            }
            StepTarget::Keyboard(code) => {
                self.keyboard.insert(*code);
            }
        }
    }

    pub fn apply_to_state(&self, state: &mut GamepadState) {
        let l2_base = if self.l2_cleared { 0 } else { state.l2_analog };
        let r2_base = if self.r2_cleared { 0 } else { state.r2_analog };
        state.l2_analog = self.l2_analog.map_or(l2_base, |value| l2_base.max(value));
        state.r2_analog = self.r2_analog.map_or(r2_base, |value| r2_base.max(value));
        for button in &self.buttons {
            state.set_button(*button, true);
        }
    }

    pub fn into_keyboard(self) -> HashSet<u16> {
        self.keyboard
    }
}

fn apply_stick_target(state: &mut GamepadState, direction: &StickDir) {
    match direction {
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
        Self {
            rules,
            split_touchpad,
            turbo_configs: Vec::new(),
            blocked_buttons: Vec::new(),
            combo_configs: Vec::new(),
            macro_configs: Vec::new(),
        }
    }

    pub fn apply(
        &self,
        l1: &GamepadState,
        state: &mut GamepadState,
        keyboard_out: &mut Vec<(u16, bool)>,
    ) {
        let mut intent = OutputIntent::default();
        self.collect(l1, state, &mut intent);
        intent.apply_to_state(state);
        keyboard_out.extend(intent.keyboard.into_iter().map(|code| (code, true)));
    }

    pub fn collect(&self, l1: &GamepadState, state: &mut GamepadState, intent: &mut OutputIntent) {
        let snapshot = l1.clone();

        for rule in &self.rules {
            if !snapshot.button(rule.src) {
                continue;
            }
            intent.clear_source(state, rule.src);
            match (rule.src, &rule.dst) {
                (Button::L2, Target::Button(Button::L2)) => {
                    intent.contribute_analog(Trigger::L2, snapshot.l2_analog);
                }
                (Button::L2, Target::Button(Button::R2)) => {
                    intent.contribute_analog(Trigger::R2, snapshot.l2_analog);
                }
                (Button::R2, Target::Button(Button::L2)) => {
                    intent.contribute_analog(Trigger::L2, snapshot.r2_analog);
                }
                (Button::R2, Target::Button(Button::R2)) => {
                    intent.contribute_analog(Trigger::R2, snapshot.r2_analog);
                }
                _ => {}
            }
            intent.press_target(state, &rule.dst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> GamepadState {
        GamepadState::default()
    }

    #[test]
    fn single_remap() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(
                Button::Cross,
                Target::Button(Button::Circle),
            )],
            false,
        );
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn multi_key() {
        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
                RemapRule::new(Button::Square, Target::Button(Button::Triangle)),
            ],
            false,
        );
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
        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
                RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
            ],
            false,
        );
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
        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
                RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
            ],
            false,
        );
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn self_map_passthrough() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::Cross, Target::Button(Button::Cross))],
            false,
        );
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::Cross)); // self-map preserves
    }

    #[test]
    fn trigger_self_map_preserves_analog() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::L2, Target::Button(Button::L2))],
            false,
        );
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 128;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 128); // self-map preserves analog
    }

    #[test]
    fn trigger_swap_transfers_analog() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::L2, Target::Button(Button::R2))],
            false,
        );
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 100;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::L2));
        assert!(s.button(Button::R2));
        assert_eq!(s.l2_analog, 0); // source cleared
        assert_eq!(s.r2_analog, 100); // transferred
    }

    #[test]
    fn trigger_cross_map_swaps_analog_independent_of_rule_order() {
        for reverse in [false, true] {
            let mut rules = vec![
                RemapRule::new(Button::L2, Target::Button(Button::R2)),
                RemapRule::new(Button::R2, Target::Button(Button::L2)),
            ];
            if reverse {
                rules.reverse();
            }
            let cfg = MappingConfig::from_rules_split(rules, false);
            let mut s = state();
            s.set_button(Button::L2, true);
            s.set_button(Button::R2, true);
            s.l2_analog = 37;
            s.r2_analog = 211;

            cfg.apply(&s.clone(), &mut s, &mut Vec::new());

            assert!(s.button(Button::L2));
            assert!(s.button(Button::R2));
            assert_eq!(s.l2_analog, 211);
            assert_eq!(s.r2_analog, 37);
        }
    }

    #[test]
    fn trigger_reducer_keeps_maximum_contribution() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::L2, Target::Button(Button::R2))],
            false,
        );
        let mut s = state();
        s.set_button(Button::L2, true);
        s.set_button(Button::R2, true);
        s.l2_analog = 80;
        s.r2_analog = 210;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert_eq!(s.r2_analog, 210);

        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::L2, Target::Button(Button::R2)),
                RemapRule::new(Button::Cross, Target::TriggerFull(Trigger::R2)),
            ],
            false,
        );
        let mut s = state();
        s.set_button(Button::L2, true);
        s.set_button(Button::Cross, true);
        s.l2_analog = 80;
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert_eq!(s.r2_analog, 255);
    }

    #[test]
    fn trigger_l2_full() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(
                Button::Cross,
                Target::TriggerFull(Trigger::L2),
            )],
            false,
        );
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 255);
    }

    #[test]
    fn trigger_r2_full() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(
                Button::Circle,
                Target::TriggerFull(Trigger::R2),
            )],
            false,
        );
        let mut s = state();
        s.set_button(Button::Circle, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(!s.button(Button::Circle));
        assert!(s.button(Button::R2));
        assert_eq!(s.r2_analog, 255);
    }

    #[test]
    fn stick_directions() {
        fn ls_y(s: &GamepadState) -> u8 {
            s.left_stick_y
        }
        fn ls_x(s: &GamepadState) -> u8 {
            s.left_stick_x
        }
        fn rs_y(s: &GamepadState) -> u8 {
            s.right_stick_y
        }
        fn rs_x(s: &GamepadState) -> u8 {
            s.right_stick_x
        }

        type StickCase = (StickDir, fn(&GamepadState) -> u8, u8);
        let cases: Vec<StickCase> = vec![
            (StickDir::LsUp, ls_y, 0),
            (StickDir::LsDown, ls_y, 255),
            (StickDir::LsLeft, ls_x, 0),
            (StickDir::LsRight, ls_x, 255),
            (StickDir::RsUp, rs_y, 0),
            (StickDir::RsDown, rs_y, 255),
            (StickDir::RsLeft, rs_x, 0),
            (StickDir::RsRight, rs_x, 255),
        ];
        let base = state();
        for (dir, getter, expected) in cases {
            let mut s = base.clone();
            s.set_button(Button::Cross, true);
            let cfg = MappingConfig::from_rules_split(
                vec![RemapRule::new(Button::Cross, Target::Stick(dir.clone()))],
                false,
            );
            cfg.apply(&s.clone(), &mut s, &mut Vec::new());
            assert!(!s.button(Button::Cross));
            assert_eq!(getter(&s), expected, "dir={:?}", dir);
        }
    }

    #[test]
    fn trigger_source_clears_analog() {
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::L2, Target::Button(Button::Cross))],
            false,
        );
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
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(Button::R2, Target::Button(Button::Circle))],
            false,
        );
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
        let cfg = MappingConfig::from_rules_split(
            vec![RemapRule::new(
                Button::Cross,
                Target::Button(Button::Circle),
            )],
            false,
        );
        let mut s = state();
        s.set_button(Button::Square, true);
        cfg.apply(&s.clone(), &mut s, &mut Vec::new());
        assert!(s.button(Button::Square)); // untouched
        assert!(!s.button(Button::Circle)); // no remap triggered
    }

    #[test]
    fn snapshot_isolation() {
        // A→B and B→A should use physical state, not intermediate results
        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
                RemapRule::new(Button::Circle, Target::Button(Button::Square)),
            ],
            false,
        );
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
        let cfg = MappingConfig::from_rules_split(
            vec![
                RemapRule::new(Button::Cross, Target::Keyboard(57)), // KEY_SPACE
            ],
            false,
        );
        let mut s = state();
        s.set_button(Button::Cross, true);
        let mut kb: Vec<(u16, bool)> = Vec::new();
        cfg.apply(&s.clone(), &mut s, &mut kb);
        assert!(!s.button(Button::Cross));
        assert_eq!(kb, vec![(57, true)]);
    }
}
