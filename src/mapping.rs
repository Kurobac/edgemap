use crate::report::Button;

#[derive(Debug, Clone)]
pub struct RemapRule {
    pub src: Button,
    pub dst: Button,
}

impl RemapRule {
    pub fn new(src: Button, dst: Button) -> Self {
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

    pub fn apply(&self, state: &mut crate::report::GamepadState) {
        for rule in &self.rules {
            if state.button(rule.src) {
                state.set_button(rule.src, false);
                state.set_button(rule.dst, true);
            }
        }
    }
}
