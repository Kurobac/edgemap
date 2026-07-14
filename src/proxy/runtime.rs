use std::time::Instant;

use log::debug;

use crate::mapping::{
    ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, StepTarget, StickDir, Target,
    Trigger, TurboConfig,
};
use crate::model::{Button, GamepadState};

pub(super) fn apply_target_to_state(state: &mut GamepadState, target: &Target, on: bool) {
    match target {
        Target::Button(btn) => state.set_button(*btn, on),
        Target::TriggerFull(t) => match t {
            Trigger::L2 => {
                state.set_button(Button::L2, on);
                state.l2_analog = if on { 255 } else { 0 };
            }
            Trigger::R2 => {
                state.set_button(Button::R2, on);
                state.r2_analog = if on { 255 } else { 0 };
            }
        },
        Target::Stick(dir) => match dir {
            StickDir::LsUp => state.left_stick_y = if on { 0 } else { 128 },
            StickDir::LsDown => state.left_stick_y = if on { 255 } else { 128 },
            StickDir::LsLeft => state.left_stick_x = if on { 0 } else { 128 },
            StickDir::LsRight => state.left_stick_x = if on { 255 } else { 128 },
            StickDir::RsUp => state.right_stick_y = if on { 0 } else { 128 },
            StickDir::RsDown => state.right_stick_y = if on { 255 } else { 128 },
            StickDir::RsLeft => state.right_stick_x = if on { 0 } else { 128 },
            StickDir::RsRight => state.right_stick_x = if on { 255 } else { 128 },
        },
        Target::Macro(_) | Target::Keyboard(_) => {}
    }
}

pub(super) struct TurboRuntime {
    pub(super) src: Button,
    pub(super) interval_ms: u64,
    pub(super) delay_ms: u64,
    pub(super) active: bool,
    pub(super) turbo_active: bool,
    pub(super) phase: bool,
    pub(super) press_time: Instant,
    pub(super) last_toggle: Instant,
}

impl TurboRuntime {
    fn from_config(cfg: &TurboConfig) -> Self {
        Self {
            src: cfg.src,
            interval_ms: cfg.interval_ms,
            delay_ms: cfg.delay_ms,
            active: false,
            turbo_active: false,
            phase: false,
            press_time: Instant::now(),
            last_toggle: Instant::now(),
        }
    }
}

pub(super) struct ComboRuntime {
    pub(super) modifier: Button,
    pub(super) key: Button,
    pub(super) output: Target,
    pub(super) active: bool,
}

impl ComboRuntime {
    fn from_combo_rule(rule: &ComboRule) -> Self {
        Self {
            modifier: rule.modifier,
            key: rule.key,
            output: rule.output.clone(),
            active: false,
        }
    }
}

struct MacroStepRuntime {
    action: StepTarget,
    press_ms: u64,
    release_ms: u64,
    pressed: bool,
    done: bool,
}

pub(super) struct MacroRuntime {
    pub(super) name: String,
    pub(super) trigger: Button,
    steps: Vec<MacroStepRuntime>,
    pub(super) active: bool,
    pub(super) mode: MacroMode,
    pub(super) source: MacroSource,
    step_start: Instant,
}

impl MacroRuntime {
    fn from_macro_rule(rule: &MacroRule) -> Self {
        Self {
            name: rule.name.clone(),
            trigger: rule.trigger,
            steps: rule
                .steps
                .iter()
                .map(|step| MacroStepRuntime {
                    action: step.action.clone(),
                    press_ms: step.press_ms,
                    release_ms: step.release_ms,
                    pressed: false,
                    done: false,
                })
                .collect(),
            active: false,
            mode: rule.mode.clone(),
            source: rule.source.clone(),
            step_start: Instant::now(),
        }
    }

    pub(super) fn activate(&mut self, now: Instant) {
        if self.active {
            return;
        }
        self.active = true;
        self.step_start = now;
        for step in &mut self.steps {
            step.pressed = false;
            step.done = false;
        }
    }

    pub(super) fn deactivate(
        &mut self,
        state: &mut GamepadState,
        keyboard_events: &mut Vec<(u16, bool)>,
    ) {
        for step in &mut self.steps {
            if step.pressed {
                match &step.action {
                    StepTarget::Gamepad(btn) => state.set_button(*btn, false),
                    StepTarget::Keyboard(code) => keyboard_events.push((*code, false)),
                }
            }
            step.pressed = false;
            step.done = false;
        }
        self.active = false;
    }

    pub(super) fn tick(
        &mut self,
        state: &mut GamepadState,
        now: Instant,
        keyboard_events: &mut Vec<(u16, bool)>,
    ) {
        let elapsed = now.duration_since(self.step_start).as_millis() as u64;
        let mut all_done = true;
        for step in &mut self.steps {
            if step.done {
                continue;
            }
            if elapsed >= step.press_ms && !step.pressed {
                step.pressed = true;
                match &step.action {
                    StepTarget::Gamepad(btn) => state.set_button(*btn, true),
                    StepTarget::Keyboard(code) => keyboard_events.push((*code, true)),
                }
                debug!(
                    "macro step pressed: name={}, elapsed_ms={elapsed}, target={:?}",
                    self.name, step.action
                );
            }
            if elapsed >= step.release_ms && step.pressed {
                step.pressed = false;
                step.done = true;
                match &step.action {
                    StepTarget::Gamepad(btn) => state.set_button(*btn, false),
                    StepTarget::Keyboard(code) => keyboard_events.push((*code, false)),
                }
                debug!(
                    "macro step released: name={}, elapsed_ms={elapsed}, target={:?}",
                    self.name, step.action
                );
            } else if !step.done {
                all_done = false;
            }
            if step.pressed {
                match &step.action {
                    StepTarget::Gamepad(btn) => state.set_button(*btn, true),
                    StepTarget::Keyboard(code) => keyboard_events.push((*code, true)),
                }
            }
        }
        if all_done {
            match self.mode {
                MacroMode::Hold => {
                    debug!("macro loop restarted: name={}", self.name);
                    self.step_start = now;
                    for step in &mut self.steps {
                        step.pressed = false;
                        step.done = false;
                    }
                }
                MacroMode::Single => {
                    debug!("macro completed: name={}", self.name);
                    self.deactivate(state, keyboard_events);
                }
            }
        }
    }
}

pub(super) static ALL_BUTTONS: &[Button] = &[
    Button::Square,
    Button::Cross,
    Button::Circle,
    Button::Triangle,
    Button::L1,
    Button::R1,
    Button::L2,
    Button::R2,
    Button::Create,
    Button::Options,
    Button::L3,
    Button::R3,
    Button::PS,
    Button::Touchpad,
    Button::TouchpadLeft,
    Button::TouchpadRight,
    Button::Mic,
    Button::DpadUp,
    Button::DpadDown,
    Button::DpadLeft,
    Button::DpadRight,
    Button::FnLeft,
    Button::FnRight,
    Button::LeftPaddle,
    Button::RightPaddle,
];

pub(super) struct MappingRuntimes {
    pub(super) turbo: Vec<TurboRuntime>,
    pub(super) combo: Vec<ComboRuntime>,
    pub(super) macros: Vec<MacroRuntime>,
}

impl MappingRuntimes {
    pub(super) fn from_mapping(mapping: &MappingConfig) -> Self {
        Self {
            turbo: mapping
                .turbo_configs
                .iter()
                .map(TurboRuntime::from_config)
                .collect(),
            combo: mapping
                .combo_configs
                .iter()
                .map(ComboRuntime::from_combo_rule)
                .collect(),
            macros: mapping
                .macro_configs
                .iter()
                .map(MacroRuntime::from_macro_rule)
                .collect(),
        }
    }
}
