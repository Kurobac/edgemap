use std::collections::HashMap;
use std::time::Instant;

use log::debug;

use crate::codec::ControllerFrame;
use crate::mapping::{MacroMode, MacroSource, MappingConfig, Target};
use crate::model::{Button, GamepadState};

use super::runtime::{apply_target_to_state, MappingRuntimes};

pub(super) struct PipelineOutput {
    pub(super) state: GamepadState,
    pub(super) physical_snapshot: GamepadState,
    pub(super) keyboard_events: Vec<(u16, bool)>,
}

pub(super) fn transform(
    frame: &ControllerFrame,
    mapping: &MappingConfig,
    runtimes: &mut MappingRuntimes,
    now: Instant,
) -> PipelineOutput {
    let mut state = frame.state.clone();

    if mapping.split_touchpad {
        state.set_button(Button::Touchpad, false);
        if let Some(side) = frame.touchpad_split_button() {
            state.set_button(side, true);
        }
    }

    let physical_snapshot = state.clone();
    let mut keyboard_events = Vec::new();

    // L1: turbo
    for turbo in &mut runtimes.turbo {
        let pressed = physical_snapshot.button(turbo.src);
        if turbo.active || pressed {
            suppress_button(&mut state, turbo.src);
        }
        if pressed && !turbo.active {
            turbo.active = true;
            turbo.turbo_active = false;
            turbo.phase = true;
            turbo.press_time = now;
            state.set_button(turbo.src, true);
            debug!("turbo pressed: source={:?}, mode=one-shot", turbo.src);
        } else if !pressed && turbo.active {
            turbo.active = false;
            turbo.turbo_active = false;
            state.set_button(turbo.src, false);
            debug!("turbo released: source={:?}", turbo.src);
        } else if turbo.active && !turbo.turbo_active && turbo.delay_ms > 0 {
            if now.saturating_duration_since(turbo.press_time).as_millis() >= turbo.delay_ms as u128
            {
                turbo.turbo_active = true;
                turbo.last_toggle = now;
                debug!(
                    "turbo delay elapsed; toggling started: source={:?}, interval_ms={}",
                    turbo.src, turbo.interval_ms
                );
            }
        } else if turbo.active && !turbo.turbo_active {
            turbo.turbo_active = true;
            turbo.last_toggle = now;
            debug!(
                "turbo toggling started: source={:?}, interval_ms={}",
                turbo.src, turbo.interval_ms
            );
        } else if turbo.active
            && turbo.turbo_active
            && now.saturating_duration_since(turbo.last_toggle).as_millis()
                >= turbo.interval_ms as u128
        {
            turbo.phase = !turbo.phase;
            turbo.last_toggle = now;
            debug!(
                "turbo phase changed: source={:?}, active={}",
                turbo.src, turbo.phase
            );
        }
        if turbo.active {
            state.set_button(turbo.src, turbo.phase);
        }
    }

    // L1: combo detection and suppression
    let mut combo_triggers = Vec::new();
    if !runtimes.combo.is_empty() {
        let pre_combo = state.clone();
        for combo in &mut runtimes.combo {
            let modifier_held = pre_combo.button(combo.modifier);
            let key_held = pre_combo.button(combo.key);
            if modifier_held {
                suppress_button(&mut state, combo.modifier);
                suppress_button(&mut state, combo.key);
            }
            let trigger = modifier_held && key_held;
            if trigger {
                combo.active = true;
                combo_triggers.push(combo.output.clone());
            } else if combo.active {
                combo.active = false;
            }
        }
    }

    // L1: explicit block
    for button in &mapping.blocked_buttons {
        suppress_button(&mut state, *button);
    }

    let l1 = state.clone();

    // L2: physical macro detection
    for runtime in &mut runtimes.macros {
        if runtime.source != MacroSource::Physical {
            continue;
        }
        let pressed = l1.button(runtime.trigger);
        if pressed && !runtime.active {
            runtime.activate(now);
        }
        if !pressed && runtime.active && matches!(runtime.mode, MacroMode::Hold) {
            runtime.deactivate(&mut state, &mut keyboard_events);
        }
    }

    // L2: remap
    mapping.apply(&l1, &mut state, &mut keyboard_events);

    // L2: combo injection
    for target in &combo_triggers {
        match target {
            Target::Macro(name) => {
                for runtime in &mut runtimes.macros {
                    if runtime.name == *name && runtime.source == MacroSource::Combo {
                        runtime.activate(now);
                    }
                }
            }
            Target::Keyboard(code) => keyboard_events.push((*code, true)),
            _ => apply_target_to_state(&mut state, target, true),
        }
    }

    for runtime in &mut runtimes.macros {
        if runtime.source != MacroSource::Combo
            || !runtime.active
            || !matches!(runtime.mode, MacroMode::Hold)
        {
            continue;
        }
        let any_combo_active = runtimes.combo.iter().any(|combo| {
            combo.active && matches!(&combo.output, Target::Macro(name) if name == &runtime.name)
        });
        if !any_combo_active {
            runtime.deactivate(&mut state, &mut keyboard_events);
        }
    }

    // L2: macro injection
    for runtime in &mut runtimes.macros {
        if runtime.active {
            runtime.tick(&mut state, now, &mut keyboard_events);
        }
    }

    PipelineOutput {
        state,
        physical_snapshot,
        keyboard_events,
    }
}

fn suppress_button(state: &mut GamepadState, button: Button) {
    state.set_button(button, false);
    match button {
        Button::L2 => state.l2_analog = 0,
        Button::R2 => state.r2_analog = 0,
        _ => {}
    }
}

pub(super) fn merge_keyboard_events(events: &[(u16, bool)]) -> HashMap<u16, bool> {
    let mut current = HashMap::new();
    for (code, pressed) in events {
        current.insert(*code, *pressed);
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::SourceCodec;
    use crate::mapping::{ComboRule, MacroRule, MacroStep, RemapRule, StepTarget, TurboConfig};
    use std::time::Duration;

    fn frame_with(buttons: &[Button]) -> ControllerFrame {
        let mut raw = [0u8; 64];
        raw[0] = 0x01;
        raw[8] = 8;
        let mut frame = SourceCodec::Ds5Usb.decode_input(&raw).unwrap();
        for button in buttons {
            frame.state.set_button(*button, true);
        }
        frame
    }

    fn touchpad_frame(pressed: bool, x: u16) -> ControllerFrame {
        assert!(x <= 0x0fff);
        let mut raw = [0u8; 64];
        raw[0] = 0x01;
        raw[8] = 8;
        raw[10] = if pressed { 0x02 } else { 0 };
        raw[33] = 0x00;
        raw[34] = x as u8;
        raw[35] = ((x >> 8) as u8) & 0x0f;
        raw[37] = 0x80;
        SourceCodec::Ds5Usb.decode_input(&raw).unwrap()
    }

    fn assert_split_child_turbo(x: u16, side: Button) {
        let mapping = MappingConfig {
            split_touchpad: true,
            rules: vec![RemapRule::new(side, Target::Button(Button::Circle))],
            turbo_configs: vec![TurboConfig {
                src: side,
                interval_ms: 20,
                delay_ms: 0,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let pressed = touchpad_frame(true, x);
        let released = touchpad_frame(false, x);
        let start = Instant::now();

        let first = transform(&pressed, &mapping, &mut runtimes, start);
        assert!(!first.physical_snapshot.button(Button::Touchpad));
        assert!(first.physical_snapshot.button(side));
        assert!(runtimes.turbo[0].active);
        assert!(first.state.button(Button::Circle));

        transform(
            &pressed,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(1),
        );
        let off = transform(
            &pressed,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(21),
        );
        assert!(!runtimes.turbo[0].phase);
        assert!(!off.state.button(Button::Circle));

        let release = transform(
            &released,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(22),
        );
        assert!(!runtimes.turbo[0].active);
        assert!(!runtimes.turbo[0].turbo_active);
        assert!(!release.physical_snapshot.button(side));
        assert!(!release.state.button(Button::Circle));
    }

    #[test]
    fn left_split_child_turbo_uses_derived_physical_snapshot() {
        assert_split_child_turbo(959, Button::TouchpadLeft);
    }

    #[test]
    fn right_split_child_turbo_uses_derived_physical_snapshot() {
        assert_split_child_turbo(960, Button::TouchpadRight);
    }

    #[test]
    fn turbo_modifier_off_phase_allows_chord_key_passthrough() {
        let mapping = MappingConfig {
            turbo_configs: vec![TurboConfig {
                src: Button::L1,
                interval_ms: 10,
                delay_ms: 0,
            }],
            combo_configs: vec![ComboRule {
                modifier: Button::L1,
                key: Button::Cross,
                output: Target::Button(Button::Circle),
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let frame = frame_with(&[Button::L1, Button::Cross]);
        let start = Instant::now();

        let on = transform(&frame, &mapping, &mut runtimes, start);
        assert!(!on.state.button(Button::L1));
        assert!(!on.state.button(Button::Cross));
        assert!(on.state.button(Button::Circle));

        transform(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(1),
        );
        let off = transform(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(11),
        );
        assert!(!off.state.button(Button::L1));
        assert!(off.state.button(Button::Cross));
        assert!(!off.state.button(Button::Circle));
        assert!(!runtimes.combo[0].active);
    }

    #[test]
    fn block_happens_before_remap_freeze() {
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(
                Button::Cross,
                Target::Button(Button::Circle),
            )],
            blocked_buttons: vec![Button::Cross],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let output = transform(
            &frame_with(&[Button::Cross]),
            &mapping,
            &mut runtimes,
            Instant::now(),
        );
        assert!(!output.state.button(Button::Cross));
        assert!(!output.state.button(Button::Circle));
    }

    #[test]
    fn combo_injects_after_source_suppression() {
        let mapping = MappingConfig {
            combo_configs: vec![ComboRule {
                modifier: Button::L1,
                key: Button::Cross,
                output: Target::Button(Button::Circle),
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let output = transform(
            &frame_with(&[Button::L1, Button::Cross]),
            &mapping,
            &mut runtimes,
            Instant::now(),
        );
        assert!(!output.state.button(Button::L1));
        assert!(!output.state.button(Button::Cross));
        assert!(output.state.button(Button::Circle));
    }

    #[test]
    fn block_suppresses_initial_turbo_phase() {
        let mapping = MappingConfig {
            turbo_configs: vec![TurboConfig {
                src: Button::Cross,
                interval_ms: 20,
                delay_ms: 0,
            }],
            blocked_buttons: vec![Button::Cross],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let output = transform(
            &frame_with(&[Button::Cross]),
            &mapping,
            &mut runtimes,
            Instant::now(),
        );
        assert!(!output.state.button(Button::Cross));
    }

    #[test]
    fn physical_macro_runs_after_remap() {
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(
                Button::Cross,
                Target::Macro("test".to_string()),
            )],
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "test".to_string(),
                mode: MacroMode::Hold,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 0,
                    release_ms: 100,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let output = transform(
            &frame_with(&[Button::Cross]),
            &mapping,
            &mut runtimes,
            Instant::now(),
        );
        assert!(!output.state.button(Button::Cross));
        assert!(output.state.button(Button::Circle));
    }

    #[test]
    fn keyboard_merge_is_last_write_wins() {
        let merged = merge_keyboard_events(&[(30, true), (31, true), (30, false)]);
        assert_eq!(merged.get(&30), Some(&false));
        assert_eq!(merged.get(&31), Some(&true));
    }
}
