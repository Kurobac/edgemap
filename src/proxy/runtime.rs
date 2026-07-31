use std::time::{Duration, Instant};

use log::debug;

use crate::mapping::{
    ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, OutputIntent, StepTarget, Target,
    TurboConfig,
};
use crate::model::Button;

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

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        if !self.active {
            return None;
        }
        if self.turbo_active {
            self.last_toggle
                .checked_add(Duration::from_millis(self.interval_ms))
        } else {
            self.press_time
                .checked_add(Duration::from_millis(self.delay_ms))
        }
    }

    pub(super) fn advance(&mut self, now: Instant) {
        if !self.active {
            return;
        }
        if !self.turbo_active {
            let Some(start) = self
                .press_time
                .checked_add(Duration::from_millis(self.delay_ms))
            else {
                return;
            };
            if now < start {
                return;
            }
            self.turbo_active = true;
            self.last_toggle = start;
            debug!(
                "turbo toggling started: source={:?}, interval_ms={}",
                self.src, self.interval_ms
            );
        }

        let interval = Duration::from_millis(self.interval_ms);
        assert!(
            !interval.is_zero(),
            "validated turbo interval must be non-zero"
        );
        let elapsed = now.saturating_duration_since(self.last_toggle);
        let intervals = elapsed.as_nanos() / interval.as_nanos();
        if intervals == 0 {
            return;
        }
        if intervals % 2 == 1 {
            self.phase = !self.phase;
        }
        let remainder_ns = elapsed.as_nanos() % interval.as_nanos();
        let remainder =
            duration_from_nanos(remainder_ns).expect("an interval remainder must fit in Duration");
        self.last_toggle = now
            .checked_sub(remainder)
            .expect("a monotonic elapsed remainder must be subtractable");
        debug!(
            "turbo phase synchronized: source={:?}, intervals={intervals}, active={}",
            self.src, self.phase
        );
    }
}

fn duration_from_nanos(value: u128) -> Option<Duration> {
    let seconds = u64::try_from(value / 1_000_000_000).ok()?;
    let nanos = (value % 1_000_000_000) as u32;
    Some(Duration::new(seconds, nanos))
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
    next_transition: Option<Instant>,
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
            next_transition: None,
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
        self.next_transition = self
            .steps
            .iter()
            .filter_map(|step| now.checked_add(Duration::from_millis(step.press_ms)))
            .min();
    }

    pub(super) fn deactivate(&mut self) {
        for step in &mut self.steps {
            step.pressed = false;
            step.done = false;
        }
        self.active = false;
        self.next_transition = None;
    }

    pub(super) fn advance(&mut self, now: Instant) {
        if !self.active {
            return;
        }
        if matches!(&self.mode, MacroMode::Single) {
            self.advance_single(now);
        } else {
            self.advance_hold(now);
        }
    }

    fn advance_single(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.step_start).as_millis();
        for step in &mut self.steps {
            let was_pressed = step.pressed;
            step.done = elapsed >= step.release_ms as u128;
            step.pressed = elapsed >= step.press_ms as u128 && !step.done;
            if step.pressed && !was_pressed {
                debug!(
                    "macro step pressed: name={}, elapsed_ms={elapsed}, target={:?}",
                    self.name, step.action
                );
            }
            if !step.pressed && was_pressed {
                debug!(
                    "macro step released: name={}, elapsed_ms={elapsed}, target={:?}",
                    self.name, step.action
                );
            }
        }
        if self.steps.iter().all(|step| step.done) {
            debug!("macro completed: name={}", self.name);
            self.deactivate();
            return;
        }
        self.next_transition = self
            .steps
            .iter()
            .filter(|step| !step.done)
            .filter_map(|step| {
                let offset = if step.pressed {
                    step.release_ms
                } else {
                    step.press_ms
                };
                self.step_start.checked_add(Duration::from_millis(offset))
            })
            .filter(|deadline| *deadline > now)
            .min();
    }

    fn advance_hold(&mut self, now: Instant) {
        let cycle_ms = self
            .steps
            .iter()
            .map(|step| step.release_ms)
            .max()
            .expect("validated hold macro must contain a step");
        let elapsed_ms = now.saturating_duration_since(self.step_start).as_millis();
        let cycle_ms_u128 = cycle_ms as u128;
        let cycle_index = elapsed_ms / cycle_ms_u128;
        let position_ms = (elapsed_ms % cycle_ms_u128) as u64;
        let cycle_offset_ms = u64::try_from(cycle_index * cycle_ms_u128).ok();
        let cycle_start = cycle_offset_ms
            .and_then(|offset| self.step_start.checked_add(Duration::from_millis(offset)));

        for step in &mut self.steps {
            let was_pressed = step.pressed;
            step.pressed = step.press_ms <= position_ms && position_ms < step.release_ms;
            step.done = false;
            if step.pressed && !was_pressed {
                debug!(
                    "macro step pressed: name={}, cycle_ms={position_ms}, target={:?}",
                    self.name, step.action
                );
            } else if !step.pressed && was_pressed {
                debug!(
                    "macro step released: name={}, cycle_ms={position_ms}, target={:?}",
                    self.name, step.action
                );
            }
        }

        self.next_transition = cycle_start.and_then(|cycle_start| {
            self.steps
                .iter()
                .filter_map(|step| {
                    let offset = if position_ms < step.press_ms {
                        step.press_ms
                    } else if position_ms < step.release_ms {
                        step.release_ms
                    } else {
                        cycle_ms.checked_add(step.press_ms)?
                    };
                    cycle_start.checked_add(Duration::from_millis(offset))
                })
                .filter(|deadline| *deadline > now)
                .min()
        });
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        if self.active {
            self.next_transition
        } else {
            None
        }
    }

    pub(super) fn contribute(&self, intent: &mut OutputIntent) {
        for step in &self.steps {
            if step.pressed {
                intent.press_step(&step.action);
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

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.turbo
            .iter()
            .filter_map(TurboRuntime::next_deadline)
            .chain(self.macros.iter().filter_map(MacroRuntime::next_deadline))
            .min()
    }
}
