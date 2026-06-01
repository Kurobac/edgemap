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

#[derive(Debug, Clone, Default)]
pub struct MappingConfig {
    pub rules: Vec<RemapRule>,
}

impl MappingConfig {
    pub fn from_rules(rules: Vec<RemapRule>) -> Self {
        Self { rules }
    }

    pub fn apply(&self, state: &mut GamepadState) {
        for rule in &self.rules {
            if !state.button(rule.src) {
                continue;
            }
            state.set_button(rule.src, false);
            match &rule.dst {
                Target::Button(btn) => {
                    state.set_button(*btn, true);
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
            }
        }
    }
}
