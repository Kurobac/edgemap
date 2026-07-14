use crate::mapping::{
    ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, RemapRule, TurboConfig,
};
use crate::report::Button;

use super::targets::{resolve_step_target, resolve_target_or_macro};
use super::Config;

impl Config {
    pub fn parse(source: &str, content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("Invalid config {source}: {e}"))
    }

    pub fn to_mapping_config(&self) -> Result<MappingConfig, String> {
        let mut rules = Vec::new();
        let mut blocked_buttons = Vec::new();
        let mut combo_configs = Vec::new();
        let mut split_touchpad = false;

        for (btn_name, btn_conf) in &self.buttons {
            let src = Button::from_name(btn_name)
                .ok_or_else(|| format!("Unknown source button: {btn_name}"))?;

            if btn_conf.turbo && btn_conf.remap.as_deref() == Some("combo") {
                for c in &btn_conf.combos {
                    let key = Button::from_name(&c.key)
                        .ok_or_else(|| format!("Unknown combo key '{}' in [{btn_name}]", c.key))?;
                    let output =
                        resolve_target_or_macro(&c.output, &self.macros).ok_or_else(|| {
                            format!("Unknown combo output '{}' in [{btn_name}]", c.output)
                        })?;
                    combo_configs.push(ComboRule {
                        modifier: src,
                        key,
                        output,
                    });
                }
                continue;
            }

            if src == Button::Touchpad && btn_conf.remap.as_deref() == Some("split") {
                split_touchpad = true;
                continue;
            }

            if btn_conf.remap.as_deref() == Some("combo") {
                for c in &btn_conf.combos {
                    let key = Button::from_name(&c.key)
                        .ok_or_else(|| format!("Unknown combo key '{}' in [{btn_name}]", c.key))?;
                    let output =
                        resolve_target_or_macro(&c.output, &self.macros).ok_or_else(|| {
                            format!("Unknown combo output '{}' in [{btn_name}]", c.output)
                        })?;
                    combo_configs.push(ComboRule {
                        modifier: src,
                        key,
                        output,
                    });
                }
                continue;
            }

            let dst = match btn_conf.remap.as_deref() {
                Some("passthrough") | None => continue,
                Some("block") => {
                    blocked_buttons.push(src);
                    continue;
                }
                Some(target) => resolve_target_or_macro(target, &self.macros)
                    .ok_or_else(|| format!("Unknown target '{target}' for button '{btn_name}'"))?,
            };
            rules.push(RemapRule::new(src, dst));
        }

        if split_touchpad {
            let left_dst = self
                .buttons
                .get("touchpad_left")
                .and_then(|c| c.remap.as_deref())
                .ok_or("split touchpad requires [touchpad_left] to be configured")?;
            let right_dst = self
                .buttons
                .get("touchpad_right")
                .and_then(|c| c.remap.as_deref())
                .ok_or("split touchpad requires [touchpad_right] to be configured")?;

            if left_dst == "block" {
                return Err("touchpad_left: remap=\"block\" is not allowed in split mode".into());
            }
            if right_dst == "block" {
                return Err("touchpad_right: remap=\"block\" is not allowed in split mode".into());
            }

            let left = resolve_target_or_macro(left_dst, &self.macros)
                .ok_or_else(|| format!("Unknown target '{left_dst}' for touchpad_left"))?;
            let right = resolve_target_or_macro(right_dst, &self.macros)
                .ok_or_else(|| format!("Unknown target '{right_dst}' for touchpad_right"))?;

            rules.push(RemapRule::new(Button::TouchpadLeft, left));
            rules.push(RemapRule::new(Button::TouchpadRight, right));
        }

        let mut mapping = MappingConfig::from_rules_split(rules, split_touchpad);
        mapping.turbo_configs = self.build_turbo_configs();
        mapping.blocked_buttons = blocked_buttons;
        mapping.combo_configs = combo_configs;
        mapping.macro_configs = self.build_macro_configs();
        Ok(mapping)
    }

    pub fn build_turbo_configs(&self) -> Vec<TurboConfig> {
        let mut configs = Vec::new();
        for (btn_name, btn_conf) in &self.buttons {
            if !btn_conf.turbo {
                continue;
            }
            let src = match Button::from_name(btn_name) {
                Some(b) => b,
                None => continue,
            };
            configs.push(TurboConfig {
                src,
                interval_ms: btn_conf.turbo_interval_ms,
                delay_ms: btn_conf.turbo_delay_ms,
            });
        }
        configs
    }

    pub fn build_macro_configs(&self) -> Vec<MacroRule> {
        let mut configs = Vec::new();
        for (btn_name, btn_conf) in &self.buttons {
            if btn_conf.turbo {
                continue;
            }
            let trigger = match Button::from_name(btn_name) {
                Some(b) => b,
                None => continue,
            };
            let remap = btn_conf.remap.as_deref().unwrap_or("");
            let macro_name = if matches!(remap, "block" | "combo") || remap.is_empty() {
                continue;
            } else {
                remap
            };
            let mcfg = match self.macros.get(macro_name) {
                Some(m) => m,
                None => continue,
            };
            let mode = match mcfg.mode.as_str() {
                "single" => MacroMode::Single,
                _ => MacroMode::Hold,
            };
            let steps = mcfg
                .sequence
                .iter()
                .map(|s| crate::mapping::MacroStep {
                    action: resolve_step_target(&s.key)
                        .unwrap_or(crate::mapping::StepTarget::Gamepad(Button::Cross)),
                    press_ms: s.press_ms,
                    release_ms: s.release_ms,
                })
                .collect();
            configs.push(MacroRule {
                trigger,
                name: macro_name.to_string(),
                mode,
                steps,
                source: MacroSource::Physical,
            });
        }

        for btn_conf in self.buttons.values() {
            if !matches!(btn_conf.remap.as_deref(), Some("combo")) {
                continue;
            }
            for c in &btn_conf.combos {
                let mcfg = match self.macros.get(&c.output) {
                    Some(m) => m,
                    None => continue,
                };
                let mode = match mcfg.mode.as_str() {
                    "single" => MacroMode::Single,
                    _ => MacroMode::Hold,
                };
                let steps = mcfg
                    .sequence
                    .iter()
                    .map(|s| crate::mapping::MacroStep {
                        action: resolve_step_target(&s.key)
                            .unwrap_or(crate::mapping::StepTarget::Gamepad(Button::Cross)),
                        press_ms: s.press_ms,
                        release_ms: s.release_ms,
                    })
                    .collect();
                configs.push(MacroRule {
                    trigger: Button::Cross,
                    name: c.output.clone(),
                    mode,
                    steps,
                    source: MacroSource::Combo,
                });
            }
        }
        configs
    }
}
