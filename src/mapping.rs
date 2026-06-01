use crate::report::{Button, GamepadState};

#[derive(Debug, Clone)]
pub enum Trigger {
    L2,
    R2,
}

#[derive(Debug, Clone)]
pub enum StickDir {
    LS_Up,
    LS_Down,
    LS_Left,
    LS_Right,
    RS_Up,
    RS_Down,
    RS_Left,
    RS_Right,
}

#[derive(Debug, Clone)]
pub enum Target {
    Button(Button),
    TriggerFull(Trigger),
    Stick(StickDir),
    Block,
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
    pub dst: Target,
    pub interval_ms: u64,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct MappingConfig {
    pub rules: Vec<RemapRule>,
    pub split_touchpad: bool,
    pub turbo_configs: Vec<TurboConfig>,
}

impl MappingConfig {
    #[allow(dead_code)]
    pub fn from_rules(rules: Vec<RemapRule>) -> Self {
        Self { rules, split_touchpad: false, turbo_configs: Vec::new() }
    }

    pub fn from_rules_split(rules: Vec<RemapRule>, split_touchpad: bool) -> Self {
        Self { rules, split_touchpad, turbo_configs: Vec::new() }
    }

    pub fn apply(&self, state: &mut GamepadState) {
        let snapshot = state.clone();
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
                        StickDir::LS_Up => state.left_stick_y = 0,
                        StickDir::LS_Down => state.left_stick_y = 255,
                        StickDir::LS_Left => state.left_stick_x = 0,
                        StickDir::LS_Right => state.left_stick_x = 255,
                        StickDir::RS_Up => state.right_stick_y = 0,
                        StickDir::RS_Down => state.right_stick_y = 255,
                        StickDir::RS_Left => state.right_stick_x = 0,
                        StickDir::RS_Right => state.right_stick_x = 255,
                    }
                }
                Target::Block => {}
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
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&mut s);
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn multi_key() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Square, Target::Button(Button::Triangle)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        s.set_button(Button::Square, true);
        cfg.apply(&mut s);
        assert!(!s.button(Button::Cross));
        assert!(!s.button(Button::Square));
        assert!(s.button(Button::Circle));
        assert!(s.button(Button::Triangle));
    }

    #[test]
    fn cross_map_both_pressed() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        s.set_button(Button::Circle, true);
        cfg.apply(&mut s);
        // deferred targets: both circle and cross are set in Phase 2
        assert!(s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn cross_map_one_pressed() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Cross)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&mut s);
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
    }

    #[test]
    fn self_map_passthrough() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Cross)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&mut s);
        assert!(s.button(Button::Cross)); // self-map preserves
    }

    #[test]
    fn trigger_self_map_preserves_analog() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::L2, Target::Button(Button::L2)),
        ]);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 128;
        cfg.apply(&mut s);
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 128); // self-map preserves analog
    }

    #[test]
    fn trigger_swap_transfers_analog() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::L2, Target::Button(Button::R2)),
        ]);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 100;
        cfg.apply(&mut s);
        assert!(!s.button(Button::L2));
        assert!(s.button(Button::R2));
        assert_eq!(s.l2_analog, 0);   // source cleared
        assert_eq!(s.r2_analog, 100); // transferred
    }

    #[test]
    fn block() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Block),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        s.set_button(Button::Circle, true);
        cfg.apply(&mut s);
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle)); // untouched
    }

    #[test]
    fn trigger_l2_full() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::TriggerFull(Trigger::L2)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        cfg.apply(&mut s);
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::L2));
        assert_eq!(s.l2_analog, 255);
    }

    #[test]
    fn trigger_r2_full() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Circle, Target::TriggerFull(Trigger::R2)),
        ]);
        let mut s = state();
        s.set_button(Button::Circle, true);
        cfg.apply(&mut s);
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
            (StickDir::LS_Up,    ls_y, 0),
            (StickDir::LS_Down,  ls_y, 255),
            (StickDir::LS_Left,  ls_x, 0),
            (StickDir::LS_Right, ls_x, 255),
            (StickDir::RS_Up,    rs_y, 0),
            (StickDir::RS_Down,  rs_y, 255),
            (StickDir::RS_Left,  rs_x, 0),
            (StickDir::RS_Right, rs_x, 255),
        ];
        let base = state();
        for (dir, getter, expected) in cases {
            let mut s = base.clone();
            s.set_button(Button::Cross, true);
            let cfg = MappingConfig::from_rules(vec![
                RemapRule::new(Button::Cross, Target::Stick(dir.clone())),
            ]);
            cfg.apply(&mut s);
            assert!(!s.button(Button::Cross));
            assert_eq!(getter(&s), expected, "dir={:?}", dir);
        }
    }

    #[test]
    fn trigger_source_clears_analog() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::L2, Target::Button(Button::Cross)),
        ]);
        let mut s = state();
        s.set_button(Button::L2, true);
        s.l2_analog = 128;
        cfg.apply(&mut s);
        assert!(!s.button(Button::L2));
        assert!(s.button(Button::Cross));
        assert_eq!(s.l2_analog, 0); // analog cleared
    }

    #[test]
    fn r2_source_clears_analog() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::R2, Target::Button(Button::Circle)),
        ]);
        let mut s = state();
        s.set_button(Button::R2, true);
        s.r2_analog = 200;
        cfg.apply(&mut s);
        assert!(!s.button(Button::R2));
        assert!(s.button(Button::Circle));
        assert_eq!(s.r2_analog, 0);
    }

    #[test]
    fn no_matching_source_unchanged() {
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
        ]);
        let mut s = state();
        s.set_button(Button::Square, true);
        cfg.apply(&mut s);
        assert!(s.button(Button::Square)); // untouched
        assert!(!s.button(Button::Circle)); // no remap triggered
    }

    #[test]
    fn snapshot_isolation() {
        // A→B and B→A should use physical state, not intermediate results
        let cfg = MappingConfig::from_rules(vec![
            RemapRule::new(Button::Cross, Target::Button(Button::Circle)),
            RemapRule::new(Button::Circle, Target::Button(Button::Square)),
        ]);
        let mut s = state();
        s.set_button(Button::Cross, true);
        // Circle NOT pressed physically
        cfg.apply(&mut s);
        // cross→circle fires (cross was pressed)
        // circle→square should NOT fire (circle was NOT physically pressed)
        assert!(!s.button(Button::Cross));
        assert!(s.button(Button::Circle));
        assert!(!s.button(Button::Square)); // not triggered via cascade
    }
}
