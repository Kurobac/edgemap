use std::collections::HashSet;
use std::time::Instant;

use log::debug;

use crate::codec::ControllerFrame;
use crate::mapping::{MacroMode, MacroSource, MappingConfig, OutputIntent, Target};
use crate::model::{Button, GamepadState};

use super::runtime::MappingRuntimes;

pub(super) struct PipelineOutput {
    pub(super) state: GamepadState,
    pub(super) physical_snapshot: GamepadState,
    pub(super) keyboard: HashSet<u16>,
}

pub(super) fn transform(
    frame: &ControllerFrame,
    mapping: &MappingConfig,
    runtimes: &mut MappingRuntimes,
    now: Instant,
) -> PipelineOutput {
    transform_inner(frame, mapping, runtimes, now, true)
}

pub(super) fn transform_timer(
    frame: &ControllerFrame,
    mapping: &MappingConfig,
    runtimes: &mut MappingRuntimes,
    now: Instant,
) -> PipelineOutput {
    transform_inner(frame, mapping, runtimes, now, false)
}

fn transform_inner(
    frame: &ControllerFrame,
    mapping: &MappingConfig,
    runtimes: &mut MappingRuntimes,
    now: Instant,
    observe_source: bool,
) -> PipelineOutput {
    let mut state = frame.state.clone();

    if mapping.split_touchpad {
        state.set_button(Button::Touchpad, false);
        if let Some(side) = frame.touchpad_split_button() {
            state.set_button(side, true);
        }
    }

    let physical_snapshot = state.clone();
    let mut intent = OutputIntent::default();

    // L1: turbo
    for turbo in &mut runtimes.turbo {
        let pressed = physical_snapshot.button(turbo.src);
        let was_active = turbo.active;
        if turbo.active || (observe_source && pressed) {
            suppress_button(&mut state, turbo.src);
        }
        if observe_source {
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
            }
        }
        if turbo.active && (!observe_source || was_active) {
            turbo.advance(now);
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
            combo.active = trigger;
            if combo.active {
                combo_triggers.push(combo.output.clone());
            }
        }
    }

    // L1: explicit block
    for button in &mapping.blocked_buttons {
        suppress_button(&mut state, *button);
    }

    let l1 = state.clone();

    // L2: physical macro detection
    if observe_source {
        for runtime in &mut runtimes.macros {
            if runtime.source != MacroSource::Physical {
                continue;
            }
            let pressed = l1.button(runtime.trigger);
            if pressed && !runtime.active {
                runtime.activate(now);
            }
            if !pressed && runtime.active && matches!(runtime.mode, MacroMode::Hold) {
                runtime.deactivate();
            }
        }
    }

    // L2: remap
    mapping.collect(&l1, &mut state, &mut intent);

    // L2: combo injection
    for target in &combo_triggers {
        match target {
            Target::Macro(name) => {
                if observe_source {
                    for runtime in &mut runtimes.macros {
                        if runtime.name == *name && runtime.source == MacroSource::Combo {
                            runtime.activate(now);
                        }
                    }
                }
            }
            _ => intent.press_target(&mut state, target),
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
            runtime.deactivate();
        }
    }

    // L2: macro injection
    for runtime in &mut runtimes.macros {
        if runtime.active {
            runtime.advance(now);
            runtime.contribute(&mut intent);
        }
    }

    intent.apply_to_state(&mut state);

    PipelineOutput {
        state,
        physical_snapshot,
        keyboard: intent.into_keyboard(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::SourceCodec;
    use crate::keyboard::KeyboardDevice;
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

        transform_timer(&frame, &mapping, &mut runtimes, start);
        let off = transform_timer(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(10),
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
    fn physical_button_survives_shared_macro_release() {
        let mapping = MappingConfig {
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "shared".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 0,
                    release_ms: 10,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        let active = transform(
            &frame_with(&[Button::Cross, Button::Circle]),
            &mapping,
            &mut runtimes,
            start,
        );
        assert!(active.state.button(Button::Circle));

        let completed = transform(
            &frame_with(&[Button::Circle]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(11),
        );
        assert!(completed.state.button(Button::Circle));
        assert!(!runtimes.macros[0].active);
    }

    #[test]
    fn physical_button_survives_shared_remap_release() {
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(
                Button::Cross,
                Target::Button(Button::Circle),
            )],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        let shared = transform(
            &frame_with(&[Button::Cross, Button::Circle]),
            &mapping,
            &mut runtimes,
            start,
        );
        assert!(shared.state.button(Button::Circle));

        let physical_only = transform(
            &frame_with(&[Button::Circle]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(1),
        );
        assert!(physical_only.state.button(Button::Circle));

        let released = transform(
            &frame_with(&[]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(2),
        );
        assert!(!released.state.button(Button::Circle));
    }

    #[test]
    fn remap_output_survives_shared_macro_release() {
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(
                Button::Square,
                Target::Button(Button::Circle),
            )],
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "shared-remap".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 0,
                    release_ms: 10,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        transform(
            &frame_with(&[Button::Cross, Button::Square]),
            &mapping,
            &mut runtimes,
            start,
        );
        let completed = transform(
            &frame_with(&[Button::Square]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(11),
        );

        assert!(!runtimes.macros[0].active);
        assert!(completed.state.button(Button::Circle));
    }

    #[test]
    fn two_macros_share_button_until_last_owner_finishes() {
        let make_macro = |trigger: Button, name: &str, release_ms: u64| MacroRule {
            trigger,
            name: name.to_string(),
            mode: MacroMode::Single,
            steps: vec![MacroStep {
                action: StepTarget::Gamepad(Button::Circle),
                press_ms: 0,
                release_ms,
            }],
            source: MacroSource::Physical,
        };
        let mapping = MappingConfig {
            macro_configs: vec![
                make_macro(Button::Cross, "short", 10),
                make_macro(Button::Square, "long", 100),
            ],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        transform(
            &frame_with(&[Button::Cross, Button::Square]),
            &mapping,
            &mut runtimes,
            start,
        );
        let output = transform(
            &frame_with(&[]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(20),
        );

        assert!(!runtimes.macros[0].active);
        assert!(runtimes.macros[1].active);
        assert!(output.state.button(Button::Circle));
    }

    #[test]
    fn combo_output_survives_shared_macro_release() {
        let mapping = MappingConfig {
            combo_configs: vec![ComboRule {
                modifier: Button::L1,
                key: Button::Cross,
                output: Target::Button(Button::Circle),
            }],
            macro_configs: vec![MacroRule {
                trigger: Button::Square,
                name: "shared".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 0,
                    release_ms: 10,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        transform(
            &frame_with(&[Button::L1, Button::Cross, Button::Square]),
            &mapping,
            &mut runtimes,
            start,
        );
        let output = transform(
            &frame_with(&[Button::L1, Button::Cross]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(20),
        );

        assert!(!runtimes.macros[0].active);
        assert!(output.state.button(Button::Circle));
    }

    #[test]
    fn remap_and_macro_share_keyboard_key_as_one_desired_owner_set() {
        let key = 57;
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(Button::Cross, Target::Keyboard(key))],
            macro_configs: vec![MacroRule {
                trigger: Button::Square,
                name: "shared-key".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Keyboard(key),
                    press_ms: 0,
                    release_ms: 10,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let start = Instant::now();

        let active = transform(
            &frame_with(&[Button::Cross, Button::Square]),
            &mapping,
            &mut runtimes,
            start,
        );
        assert_eq!(active.keyboard, HashSet::from([key]));

        let macro_completed = transform(
            &frame_with(&[Button::Cross]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(20),
        );
        assert_eq!(macro_completed.keyboard, HashSet::from([key]));

        let released = transform(
            &frame_with(&[]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(21),
        );
        assert!(released.keyboard.is_empty());
    }

    #[test]
    fn remap_combo_and_macro_share_one_keyboard_press_and_release() {
        let key = 57;
        let mapping = MappingConfig {
            rules: vec![RemapRule::new(Button::Cross, Target::Keyboard(key))],
            combo_configs: vec![ComboRule {
                modifier: Button::L1,
                key: Button::Circle,
                output: Target::Keyboard(key),
            }],
            macro_configs: vec![MacroRule {
                trigger: Button::Square,
                name: "shared-key".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Keyboard(key),
                    press_ms: 0,
                    release_ms: 10,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let mut keyboard = KeyboardDevice::dummy();
        let start = Instant::now();

        let all_owners = transform(
            &frame_with(&[Button::Cross, Button::L1, Button::Circle, Button::Square]),
            &mapping,
            &mut runtimes,
            start,
        );
        assert_eq!(all_owners.keyboard, HashSet::from([key]));
        keyboard.sync(&all_owners.keyboard);

        let remap_and_combo = transform(
            &frame_with(&[Button::Cross, Button::L1, Button::Circle]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(20),
        );
        assert_eq!(remap_and_combo.keyboard, HashSet::from([key]));
        keyboard.sync(&remap_and_combo.keyboard);

        let combo_only = transform(
            &frame_with(&[Button::L1, Button::Circle]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(21),
        );
        assert_eq!(combo_only.keyboard, HashSet::from([key]));
        keyboard.sync(&combo_only.keyboard);

        let released = transform(
            &frame_with(&[]),
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(22),
        );
        assert!(released.keyboard.is_empty());
        keyboard.sync(&released.keyboard);

        assert_eq!(
            keyboard.recorded_key_events(),
            vec![(key, true), (key, false)]
        );
    }

    #[test]
    fn late_turbo_tick_computes_current_phase_without_catch_up_reports() {
        let mapping = MappingConfig {
            turbo_configs: vec![TurboConfig {
                src: Button::Cross,
                interval_ms: 10,
                delay_ms: 0,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let frame = frame_with(&[Button::Cross]);
        let start = Instant::now();

        transform(&frame, &mapping, &mut runtimes, start);
        assert_eq!(runtimes.next_deadline(), Some(start));

        transform_timer(&frame, &mapping, &mut runtimes, start);
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(10))
        );

        let late = start + Duration::from_millis(25);
        let output = transform_timer(&frame, &mapping, &mut runtimes, late);
        assert!(output.state.button(Button::Cross));
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(30))
        );

        let next = transform_timer(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(35),
        );
        assert!(!next.state.button(Button::Cross));
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(40))
        );
    }

    #[test]
    fn late_turbo_delay_uses_the_original_toggle_cadence() {
        let mapping = MappingConfig {
            turbo_configs: vec![TurboConfig {
                src: Button::Cross,
                interval_ms: 10,
                delay_ms: 20,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let frame = frame_with(&[Button::Cross]);
        let start = Instant::now();

        transform(&frame, &mapping, &mut runtimes, start);
        let late = start + Duration::from_millis(45);
        let output = transform_timer(&frame, &mapping, &mut runtimes, late);

        assert!(output.state.button(Button::Cross));
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(50))
        );
    }

    #[test]
    fn a_late_timer_may_skip_a_short_macro_without_retriggering_it() {
        let mapping = MappingConfig {
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "short".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 5,
                    release_ms: 6,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let held = frame_with(&[Button::Cross]);
        let start = Instant::now();

        let initial = transform(&held, &mapping, &mut runtimes, start);
        assert!(!initial.state.button(Button::Circle));
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(5))
        );

        let late = transform_timer(
            &held,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(10),
        );
        assert!(!late.state.button(Button::Circle));
        assert!(!runtimes.macros[0].active);
        assert_eq!(runtimes.next_deadline(), None);

        transform_timer(
            &held,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(11),
        );
        assert!(!runtimes.macros[0].active);

        transform(
            &held,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(12),
        );
        assert!(runtimes.macros[0].active);
    }

    #[test]
    fn a_short_macro_is_visible_when_timer_hits_both_deadlines() {
        let mapping = MappingConfig {
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "visible".to_string(),
                mode: MacroMode::Single,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 5,
                    release_ms: 6,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let frame = frame_with(&[Button::Cross]);
        let start = Instant::now();

        transform(&frame, &mapping, &mut runtimes, start);
        let pressed = transform_timer(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(5),
        );
        assert!(pressed.state.button(Button::Circle));
        assert_eq!(
            runtimes.next_deadline(),
            Some(start + Duration::from_millis(6))
        );

        let released = transform_timer(
            &frame,
            &mapping,
            &mut runtimes,
            start + Duration::from_millis(6),
        );
        assert!(!released.state.button(Button::Circle));
        assert!(!runtimes.macros[0].active);
    }

    #[test]
    fn late_hold_macro_uses_current_cycle_phase_with_a_future_deadline() {
        let mapping = MappingConfig {
            macro_configs: vec![MacroRule {
                trigger: Button::Cross,
                name: "loop".to_string(),
                mode: MacroMode::Hold,
                steps: vec![MacroStep {
                    action: StepTarget::Gamepad(Button::Circle),
                    press_ms: 0,
                    release_ms: 5,
                }],
                source: MacroSource::Physical,
            }],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        let frame = frame_with(&[Button::Cross]);
        let start = Instant::now();

        transform(&frame, &mapping, &mut runtimes, start);
        let late = start + Duration::from_millis(20);
        let output = transform_timer(&frame, &mapping, &mut runtimes, late);

        assert!(output.state.button(Button::Circle));
        assert_eq!(
            runtimes.next_deadline(),
            Some(late + Duration::from_millis(5))
        );
    }

    #[test]
    fn timer_does_not_start_physical_or_combo_macros() {
        let make_macro = |name: &str, trigger: Button, source: MacroSource| MacroRule {
            trigger,
            name: name.to_string(),
            mode: MacroMode::Single,
            steps: vec![MacroStep {
                action: StepTarget::Gamepad(Button::Circle),
                press_ms: 0,
                release_ms: 10,
            }],
            source,
        };
        let mapping = MappingConfig {
            combo_configs: vec![ComboRule {
                modifier: Button::L1,
                key: Button::Cross,
                output: Target::Macro("combo".to_string()),
            }],
            macro_configs: vec![
                make_macro("physical", Button::Square, MacroSource::Physical),
                make_macro("combo", Button::Cross, MacroSource::Combo),
            ],
            ..Default::default()
        };
        let mut runtimes = MappingRuntimes::from_mapping(&mapping);
        runtimes.combo[0].active = true;
        let frame = frame_with(&[Button::L1, Button::Cross, Button::Square]);

        transform_timer(&frame, &mapping, &mut runtimes, Instant::now());

        assert!(runtimes.macros.iter().all(|runtime| !runtime.active));
    }
}
