use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    #[serde(default = "default_output_device")]
    pub output_device: String,
    #[serde(flatten)]
    pub buttons: HashMap<String, ButtonConfig>,
    #[serde(default)]
    pub macros: HashMap<String, MacroConfig>,
}

fn default_output_device() -> String {
    "auto".to_string()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonConfig {
    pub remap: Option<String>,
    #[serde(default)]
    pub turbo: bool,
    #[serde(default = "default_turbo_interval")]
    pub turbo_interval_ms: u64,
    #[serde(default)]
    pub turbo_delay_ms: u64,
    #[serde(default)]
    pub combos: Vec<ComboConfig>,
}

fn default_turbo_interval() -> u64 {
    100
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComboConfig {
    pub key: String,
    pub output: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroConfig {
    #[serde(default = "default_macro_mode")]
    pub mode: String,
    pub sequence: Vec<MacroStep>,
}

fn default_macro_mode() -> String {
    "hold".into()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroStep {
    pub key: String,
    pub press_ms: u64,
    pub release_ms: u64,
}
