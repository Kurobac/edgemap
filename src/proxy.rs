use std::collections::{HashMap, HashSet};
use std::env;
use std::io;
use std::sync::{Arc, RwLock};

use log::{debug, error, info, trace, warn};
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use std::os::fd::BorrowedFd;

use crate::codec::{
    CodecError, CodecPipeline, FeatureReportCache, PhysicalCodec, PhysicalOutputState,
    SourceCodec, TargetCodec,
};
use crate::control::{ControlRequest, ControlServer};
use crate::device::{HidrawDevice, SonyDeviceKind};
use crate::mapping::{ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, Target, Trigger, TurboConfig};
use crate::report::Button;
use crate::shutdown::ShutdownSignal;
use crate::uhid::UhidDevice;
use std::time::{Duration, Instant};

fn apply_target_to_state(state: &mut crate::report::GamepadState, target: &Target, on: bool) {
    use crate::mapping::StickDir;
    match target {
        Target::Button(btn) => state.set_button(*btn, on),
        Target::TriggerFull(t) => match t {
            Trigger::L2 => { state.set_button(Button::L2, on); state.l2_analog = if on { 255 } else { 0 }; }
            Trigger::R2 => { state.set_button(Button::R2, on); state.r2_analog = if on { 255 } else { 0 }; }
        },
        Target::Stick(dir) => {
            match dir {
                StickDir::LsUp => if on { state.left_stick_y = 0 } else { state.left_stick_y = 128 },
                StickDir::LsDown => if on { state.left_stick_y = 255 } else { state.left_stick_y = 128 },
                StickDir::LsLeft => if on { state.left_stick_x = 0 } else { state.left_stick_x = 128 },
                StickDir::LsRight => if on { state.left_stick_x = 255 } else { state.left_stick_x = 128 },
                StickDir::RsUp => if on { state.right_stick_y = 0 } else { state.right_stick_y = 128 },
                StickDir::RsDown => if on { state.right_stick_y = 255 } else { state.right_stick_y = 128 },
                StickDir::RsLeft => if on { state.right_stick_x = 0 } else { state.right_stick_x = 128 },
                StickDir::RsRight => if on { state.right_stick_x = 255 } else { state.right_stick_x = 128 },
            }
        }
        Target::Macro(_) => {}
        Target::Keyboard(_) => {}
    }
}

struct TurboRuntime {
    src: Button,
    interval_ms: u64,
    delay_ms: u64,
    active: bool,
    turbo_active: bool,
    phase: bool,
    press_time: Instant,
    last_toggle: Instant,
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

struct ComboRuntime {
    modifier: Button,
    key: Button,
    output: Target,
    active: bool,
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
    action: crate::mapping::StepTarget,
    press_ms: u64,
    release_ms: u64,
    pressed: bool,
    done: bool,
}

struct MacroRuntime {
    name: String,
    trigger: Button,
    steps: Vec<MacroStepRuntime>,
    active: bool,
    mode: MacroMode,
    source: MacroSource,
    step_start: Instant,
}

impl MacroRuntime {
    fn from_macro_rule(rule: &MacroRule) -> Self {
        Self {
            name: rule.name.clone(),
            trigger: rule.trigger,
            steps: rule.steps.iter().map(|s| MacroStepRuntime {
                action: s.action.clone(),
                press_ms: s.press_ms,
                release_ms: s.release_ms,
                pressed: false,
                done: false,
            }).collect(),
            active: false,
            mode: rule.mode.clone(),
            source: rule.source.clone(),
            step_start: Instant::now(),
        }
    }

    fn activate(&mut self, now: Instant) {
        if self.active {
            return; // single-shot: ignore re-activation
        }
        self.active = true;
        self.step_start = now;
        for step in &mut self.steps {
            step.pressed = false;
            step.done = false;
        }
    }

    fn deactivate(&mut self, state: &mut crate::report::GamepadState, keyboard_events: &mut Vec<(u16, bool)>) {
        for step in &mut self.steps {
            if step.pressed {
                match &step.action {
                    crate::mapping::StepTarget::Gamepad(btn) => state.set_button(*btn, false),
                    crate::mapping::StepTarget::Keyboard(code) => keyboard_events.push((*code, false)),
                }
            }
            step.pressed = false;
            step.done = false;
        }
        self.active = false;
    }

    fn tick(&mut self, state: &mut crate::report::GamepadState, now: Instant, keyboard_events: &mut Vec<(u16, bool)>) {
        let elapsed = now.duration_since(self.step_start).as_millis() as u64;
        let mut all_done = true;
        for step in &mut self.steps {
            if step.done {
                continue;
            }
            if elapsed >= step.press_ms && !step.pressed {
                step.pressed = true;
                match &step.action {
                    crate::mapping::StepTarget::Gamepad(btn) => state.set_button(*btn, true),
                    crate::mapping::StepTarget::Keyboard(code) => keyboard_events.push((*code, true)),
                }
                debug!("macro step pressed: name={}, elapsed_ms={elapsed}, target={:?}", self.name, step.action);
            }
            if elapsed >= step.release_ms && step.pressed {
                step.pressed = false;
                step.done = true;
                match &step.action {
                    crate::mapping::StepTarget::Gamepad(btn) => state.set_button(*btn, false),
                    crate::mapping::StepTarget::Keyboard(code) => keyboard_events.push((*code, false)),
                }
                debug!("macro step released: name={}, elapsed_ms={elapsed}, target={:?}", self.name, step.action);
            } else if !step.done {
                all_done = false;
            }
            if step.pressed {
                match &step.action {
                    crate::mapping::StepTarget::Gamepad(btn) => state.set_button(*btn, true),
                    crate::mapping::StepTarget::Keyboard(code) => keyboard_events.push((*code, true)),
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

static ALL_BUTTONS: &[Button] = &[
    Button::Square, Button::Cross, Button::Circle, Button::Triangle,
    Button::L1, Button::R1, Button::L2, Button::R2,
    Button::Create, Button::Options, Button::L3, Button::R3,
    Button::PS, Button::Touchpad, Button::TouchpadLeft, Button::TouchpadRight, Button::Mic,
    Button::DpadUp, Button::DpadDown, Button::DpadLeft, Button::DpadRight,
    Button::FnLeft, Button::FnRight, Button::LeftPaddle, Button::RightPaddle,
];

struct RepeatInput {
    interval: Duration,
    timestamp_delta: u32,
    mode: RepeatMode,
    target: RepeatTarget,
    next_tick: Instant,
    last_report: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
enum RepeatMode {
    Passthrough,
    SeqOnly,
    SeqAndTimestamp,
}

#[derive(Clone, Copy)]
enum RepeatTarget {
    Ds5Usb,
    Ds4Usb,
}

impl RepeatInput {
    fn from_env(codec: CodecPipeline) -> Option<Self> {
        if codec.source != SourceCodec::Ds5Bt {
            return None;
        }

        let (target, mode, hz) = match codec.target {
            TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced => {
                let mode = repeat_mode_from_env()
                    .expect("repeat environment was validated at daemon startup");
                let hz = repeat_hz_from_env("DSEUHID_BT_DS5_USB_REPEAT_HZ", Some(1000))
                    .expect("repeat environment was validated at daemon startup")?;
                (RepeatTarget::Ds5Usb, mode, hz)
            }
            TargetCodec::Ds4Usb => {
                let hz = repeat_hz_from_env("DSEUHID_BT_DS4_USB_REPEAT_HZ", None)
                    .expect("repeat environment was validated at daemon startup")?;
                (RepeatTarget::Ds4Usb, RepeatMode::SeqOnly, hz)
            }
        };
        if matches!(mode, RepeatMode::Passthrough) {
            return None;
        }
        let target_name = match target {
            RepeatTarget::Ds5Usb => "DS5 USB",
            RepeatTarget::Ds4Usb => "DS4 USB",
        };
        let interval = Duration::from_nanos(1_000_000_000 / hz);
        let timestamp_delta = ((interval.as_nanos() / 333).max(1)) as u32;
        let mode_name = match mode {
            RepeatMode::Passthrough => "passthrough",
            RepeatMode::SeqOnly => "seq_only",
            RepeatMode::SeqAndTimestamp => "seq_ts",
        };
        debug!(
            "Bluetooth input repeat enabled: target={target_name}, rate_hz={hz}, mode={mode_name}"
        );
        debug!(
            "Bluetooth input repeat timing: interval_us={}, timestamp_delta={timestamp_delta}",
            interval.as_micros()
        );
        Some(Self {
            interval,
            timestamp_delta,
            mode,
            target,
            next_tick: Instant::now(),
            last_report: None,
        })
    }

    fn timeout_ms(&self) -> u16 {
        let now = Instant::now();
        if self.next_tick <= now {
            return 0;
        }
        let remaining = self.next_tick.duration_since(now);
        let ms = remaining.as_millis();
        if ms == 0 {
            1
        } else {
            ms.min(u16::MAX as u128) as u16
        }
    }

    fn store(&mut self, report: &[u8]) {
        self.last_report = Some(report.to_vec());
    }

    fn send_due(&mut self, uhid: &UhidDevice, seq: &mut u8) -> io::Result<()> {
        while self.next_tick <= Instant::now() {
            let Some(report) = self.last_report.as_mut() else {
                self.next_tick += self.interval;
                continue;
            };
            advance_repeat_report(report, seq, self.timestamp_delta, self.mode, self.target);
            uhid.send_input(report)?;
            self.next_tick += self.interval;
        }
        Ok(())
    }
}

fn repeat_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!(
            "invalid environment variable: name={name}, value is not valid Unicode"
        )),
    }
}

fn parse_repeat_mode(value: &str) -> Result<RepeatMode, String> {
    match value {
        "passthrough" => Ok(RepeatMode::Passthrough),
        "seq_only" => Ok(RepeatMode::SeqOnly),
        "seq_ts" => Ok(RepeatMode::SeqAndTimestamp),
        _ => Err(format!(
            "invalid DSEUHID_BT_DS5_USB_REPEAT_MODE={value}; expected passthrough|seq_only|seq_ts"
        )),
    }
}

fn repeat_mode_from_env() -> Result<RepeatMode, String> {
    repeat_env("DSEUHID_BT_DS5_USB_REPEAT_MODE")?
        .as_deref()
        .map_or(Ok(RepeatMode::SeqOnly), parse_repeat_mode)
}

fn parse_repeat_hz(env_name: &str, value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(hz) if (1..=2000).contains(&hz) => Ok(hz),
        _ => Err(format!(
            "invalid {env_name}={value}; expected integer 1..=2000"
        )),
    }
}

fn repeat_hz_from_env(env_name: &str, default: Option<u64>) -> Result<Option<u64>, String> {
    match repeat_env(env_name)? {
        Some(value) => parse_repeat_hz(env_name, &value).map(Some),
        None => Ok(default),
    }
}

pub(crate) fn validate_repeat_env() -> Result<(), String> {
    repeat_mode_from_env()?;
    repeat_hz_from_env("DSEUHID_BT_DS5_USB_REPEAT_HZ", Some(1000))?;
    repeat_hz_from_env("DSEUHID_BT_DS4_USB_REPEAT_HZ", None)?;
    Ok(())
}

fn advance_repeat_report(
    report: &mut [u8],
    seq: &mut u8,
    timestamp_delta: u32,
    mode: RepeatMode,
    target: RepeatTarget,
) {
    match target {
        RepeatTarget::Ds5Usb => advance_ds5_usb_repeat_report(report, seq, timestamp_delta, mode),
        RepeatTarget::Ds4Usb => advance_ds4_usb_repeat_report(report, seq),
    }
}

fn advance_ds5_usb_repeat_report(
    report: &mut [u8],
    seq: &mut u8,
    timestamp_delta: u32,
    mode: RepeatMode,
) {
    if report.len() < 32 {
        return;
    }
    *seq = seq.wrapping_add(1);
    report[7] = *seq;
    if matches!(mode, RepeatMode::SeqOnly) {
        return;
    }
    let timestamp = u32::from_le_bytes([report[28], report[29], report[30], report[31]])
        .wrapping_add(timestamp_delta);
    report[28..32].copy_from_slice(&timestamp.to_le_bytes());
}

fn advance_ds4_usb_repeat_report(report: &mut [u8], seq: &mut u8) {
    if report.len() < 35 {
        return;
    }
    *seq = seq.wrapping_add(1) & 0x3F;
    report[7] = (report[7] & 0x03) | (*seq << 2);
    report[10..12].copy_from_slice(&(*seq as u16).to_le_bytes());
    report[34] = *seq;
}

#[derive(Debug, PartialEq)]
pub enum ExitReason {
    UserShutdown,
    DeviceGone,
    ConfigChanged,
    FatalError,
}

pub(crate) struct ProxyInit {
    pub(crate) hidraw: HidrawDevice,
    pub(crate) uhid: UhidDevice,
    pub(crate) keyboard: crate::keyboard::KeyboardDevice,
    pub(crate) mapping: Arc<RwLock<MappingConfig>>,
    pub(crate) config_path: Option<String>,
    pub(crate) report_cache: FeatureReportCache,
    pub(crate) codec: CodecPipeline,
    pub(crate) source_kind: SonyDeviceKind,
    pub(crate) output_device_config: String,
}

struct MappingRuntimes {
    turbo: Vec<TurboRuntime>,
    combo: Vec<ComboRuntime>,
    macros: Vec<MacroRuntime>,
}

impl MappingRuntimes {
    fn from_mapping(mapping: &MappingConfig) -> Self {
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

static DISCONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Proxy {
    hidraw: HidrawDevice,
    uhid: UhidDevice,
    mapping: Arc<RwLock<MappingConfig>>,
    config_path: Option<String>,
    report_cache: FeatureReportCache,
    codec: CodecPipeline,
    source_kind: SonyDeviceKind,
    output_device_config: String,
    recreate_uhid: bool,
    keyboard: crate::keyboard::KeyboardDevice,
    last_keyboard: HashMap<u16, bool>,
    last_snapshot: Option<crate::report::GamepadState>,
    last_output: Option<crate::report::GamepadState>,
    repeat_input: Option<RepeatInput>,
    physical_output_state: PhysicalOutputState,
    physical_set_report_unsupported_warned: HashSet<u8>,
    turbo_runtimes: Vec<TurboRuntime>,
    combo_runtimes: Vec<ComboRuntime>,
    macro_runtimes: Vec<MacroRuntime>,
}

struct CachedReport {
    source: CachedReportSource,
    data: Vec<u8>,
}

enum CachedReportSource {
    PhysicalCache,
    TargetFallback,
}

impl Proxy {
    fn get_cached_report(&self, report_id: u8) -> Option<CachedReport> {
        if let Some(data) = self.report_cache.get(report_id) {
            return Some(CachedReport {
                source: CachedReportSource::PhysicalCache,
                data: data.to_vec(),
            });
        }
        self.codec.target.fallback_feature_report(report_id).map(|data| CachedReport {
            source: CachedReportSource::TargetFallback,
            data,
        })
    }

    pub(crate) fn new(init: ProxyInit) -> Self {
        let ProxyInit {
            hidraw,
            uhid,
            keyboard,
            mapping,
            config_path,
            report_cache,
            codec,
            source_kind,
            output_device_config,
        } = init;
        let repeat_input = RepeatInput::from_env(codec);
        let runtimes = {
            let mapping = mapping.read().unwrap();
            MappingRuntimes::from_mapping(&mapping)
        };
        Self {
            hidraw,
            uhid,
            mapping,
            config_path,
            report_cache,
            codec,
            source_kind,
            output_device_config,
            recreate_uhid: false,
            keyboard,
            last_keyboard: HashMap::new(),
            last_snapshot: None,
            last_output: None,
            repeat_input,
            physical_output_state: PhysicalOutputState::default(),
            physical_set_report_unsupported_warned: HashSet::new(),
            turbo_runtimes: runtimes.turbo,
            combo_runtimes: runtimes.combo,
            macro_runtimes: runtimes.macros,
        }
    }

    pub fn forget_restore_on_physical_disconnect(&mut self) {
        self.hidraw.clear_restored_paths();
    }

    pub fn config_path(&self) -> Option<&str> {
        self.config_path.as_deref()
    }

    fn reload_config(&mut self) -> Result<(), (&'static str, String)> {
        let path = self
            .config_path
            .clone()
            .ok_or_else(|| ("no-config", "no config path is active".to_string()))?;
        self.reload_config_from(path)
    }

    fn reload_config_from(&mut self, path: String) -> Result<(), (&'static str, String)> {
        let cfg = match crate::config::Config::load(&path) {
            Ok(cfg) => cfg,
            Err(e) => {
                return Err(("load-failed", e));
            }
        };
        if let Err(e) = crate::config::validate(&cfg) {
            return Err(("validation-failed", e));
        }
        let new_mapping = match cfg.to_mapping_config() {
            Ok(m) => {
                // warn for missing button sections
                for name in crate::config::ALL_BUTTON_NAMES {
                    if !cfg.buttons.contains_key(*name) {
                        debug!("button not configured; using passthrough: button={name}");
                    }
                }
                warn_ignored_edge_passthroughs(&cfg, self.source_kind, self.codec.target);
                m
            }
            Err(e) => {
                return Err(("mapping-failed", e));
            }
        };
        let new_output_device = cfg.output_device.clone();
        let new_runtimes = MappingRuntimes::from_mapping(&new_mapping);
        *self.mapping.write().unwrap() = new_mapping;
        info!("config reloaded: path={path}");
        self.config_path = Some(path);
        self.last_snapshot = None;
        self.last_output = None;
        if new_output_device != self.output_device_config {
            info!(
                "output device changed: previous={}, current={}",
                self.output_device_config,
                new_output_device
            );
            info!("virtual HID device recreation requested");
            self.recreate_uhid = true;
        }
        self.output_device_config = new_output_device;
        self.turbo_runtimes = new_runtimes.turbo;
        self.combo_runtimes = new_runtimes.combo;
        self.macro_runtimes = new_runtimes.macros;
        Ok(())
    }

    fn log_button_diff(&mut self, snapshot: &crate::report::GamepadState, output: &crate::report::GamepadState) {
        let mut phys_changes: Vec<String> = Vec::new();
        let prev = self.last_snapshot.as_ref();

        for btn in ALL_BUTTONS.iter() {
            let now = snapshot.button(*btn);
            let was = prev.is_some_and(|p| p.button(*btn));
            if now != was {
                phys_changes.push(format!("{}{}", if now { "+" } else { "-" }, btn.name()));
            }
        }

        if !phys_changes.is_empty() {
            let mut out_names: Vec<&str> = Vec::new();
            for btn in ALL_BUTTONS.iter() {
                if output.button(*btn) {
                    out_names.push(btn.name());
                }
            }
            let out_display = if out_names.is_empty() { "[none]".to_string() } else { out_names.join(" ") };
            debug!("controller button changes: buttons=[{}]", phys_changes.join(" "));
            debug!("virtual buttons active: buttons=[{out_display}]");
        }

        self.last_snapshot = Some(snapshot.clone());
        self.last_output = Some(output.clone());
    }

    pub fn run(&mut self, shutdown: &ShutdownSignal, control: &mut ControlServer) -> ExitReason {
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);

        let ep_fd = match Epoll::new(EpollCreateFlags::EPOLL_CLOEXEC) {
            Ok(fd) => fd,
            Err(e) => {
                error!("failed to create epoll instance: {e}");
                return ExitReason::FatalError;
            }
        };

        let hidraw_bfd = unsafe {
            BorrowedFd::borrow_raw(self.hidraw.as_raw_fd())
        };
        let uhid_bfd = unsafe {
            BorrowedFd::borrow_raw(self.uhid.as_raw_fd())
        };

        let hidraw_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            1,
        );
        if let Err(e) = ep_fd.add(hidraw_bfd, hidraw_event) {
            error!("failed to register hidraw fd with epoll: {e}");
            return ExitReason::FatalError;
        }

        let uhid_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            2,
        );
        if let Err(e) = ep_fd.add(uhid_bfd, uhid_event) {
            error!("failed to register UHID fd with epoll: {e}");
            return ExitReason::FatalError;
        }

        let control_event = EpollEvent::new(EpollFlags::EPOLLIN, 3);
        if let Err(e) = ep_fd.add(control.as_fd(), control_event) {
            error!("failed to register control socket with epoll: {e}");
            return ExitReason::FatalError;
        }

        let shutdown_event = EpollEvent::new(EpollFlags::EPOLLIN, 4);
        if let Err(e) = ep_fd.add(shutdown.as_fd(), shutdown_event) {
            error!("failed to register shutdown signal fd with epoll: {e}");
            return ExitReason::FatalError;
        }

        let mut control_state = control.state();
        control_state.uhid_ready = true;
        control.set_state(control_state);
        info!("proxy started");

        let mut seq: u8 = 0;
        let mut events = [EpollEvent::empty(); 8];
        let mut shutdown_requested = false;

        'run: while !DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
            let timeout = self.repeat_input
                .as_ref()
                .map_or(16u16, RepeatInput::timeout_ms);
            match ep_fd.wait(&mut events, timeout) {
                Ok(n) => {
                    for ev in events.iter().take(n) {
                        let fd_num = ev.data();

                        if fd_num == 1 {
                            if let Err(e) = self.handle_hidraw_input(&mut seq) {
                                error!("hidraw event handler failed: {e}");
                                break 'run;
                            }
                        } else if fd_num == 2 {
                            if let Err(e) = self.handle_uhid_event() {
                                error!("UHID event handler failed: {e}");
                                break 'run;
                            }
                        } else if fd_num == 3 {
                            if let Err(e) = self.handle_control_requests(control) {
                                error!("control socket event handler failed: {e}");
                                break 'run;
                            }
                        } else if fd_num == 4 {
                            match shutdown.consume() {
                                Ok(true) => shutdown_requested = true,
                                Ok(false) => {
                                    error!("shutdown signal fd was readable but contained no signal")
                                }
                                Err(e) => error!("failed to read shutdown signal: {e}"),
                            }
                            break 'run;
                        }
                    }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("epoll wait failed: {e}");
                    break;
                }
            }
            if let Some(repeat) = self.repeat_input.as_mut() {
                if let Err(e) = repeat.send_due(&self.uhid, &mut seq) {
                    error!("failed to send repeated input report: {e}");
                    break;
                }
            }
            if self.recreate_uhid {
                break;
            }
        }

        info!("proxy stopped");

        if shutdown_requested {
            ExitReason::UserShutdown
        } else if self.recreate_uhid {
            ExitReason::ConfigChanged
        } else if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
            ExitReason::DeviceGone
        } else {
            ExitReason::FatalError
        }
    }

    fn handle_control_requests(&mut self, control: &mut ControlServer) -> io::Result<()> {
        for pending in control.drain_requests()? {
            let request = pending.request;
            let result = match &request {
                ControlRequest::Reload => {
                    info!("control request received: action=reload");
                    self.reload_config()
                }
                ControlRequest::SwitchConfig(path) => {
                    info!("control request received: action=switch-config, path={path}");
                    self.reload_config_from(path.clone())
                }
            };
            match result {
                Ok(()) => {
                    control.reply_ok(pending.client, &request);
                    let mut state = control.state();
                    state.needs_config = false;
                    control.set_state(state);
                }
                Err((code, _detail)) => {
                    error!("control request failed; previous config retained: code={code}");
                    control.reply_error(
                        pending.client,
                        code,
                        public_control_error_message(code),
                    );
                }
            }
        }
        Ok(())
    }

    fn handle_hidraw_input(&mut self, seq: &mut u8) -> io::Result<()> {
        self.hidraw.re_restrict_self();
        let input_report_size = self.codec.source.input_report_size();
        let mut buf = vec![0u8; input_report_size];

        // Proxy owns physical hidraw reads; SourceCodec owns the byte format.
        // Keep transport-specific input parsing out of the event loop.
        loop {
            match self.hidraw.read_input(&mut buf) {
                Ok(n) if n >= input_report_size => {
                    *seq = seq.wrapping_add(1);

                    let out_report = if let Ok(mut frame) = self.codec.source.decode_input(&buf[..n]) {
                        let mut state = frame.state.clone();
                        let physical_snapshot = state.clone();

                        // touchpad split mode under read lock
                        let m = self.mapping.read().unwrap();
                        if m.split_touchpad {
                            state.set_button(Button::Touchpad, false);
                            if let Some(side) = frame.touchpad_split_button() {
                                state.set_button(side, true);
                            }
                        }
                        drop(m);

                        // ========== L1: Physical Input Filtering ==========

                        // L1: TURBO (reads physical_snapshot, writes state)
                        let mut keyboard_events: Vec<(u16, bool)> = Vec::new();
                        for t in &mut self.turbo_runtimes {
                            let pressed = physical_snapshot.button(t.src);
                            if t.active || pressed {
                                state.set_button(t.src, false);
                                match t.src {
                                    Button::L2 => state.l2_analog = 0,
                                    Button::R2 => state.r2_analog = 0,
                                    _ => {}
                                }
                            }
                            if pressed && !t.active {
                                t.active = true;
                                t.turbo_active = false;
                                t.phase = true;
                                t.press_time = Instant::now();
                                state.set_button(t.src, true);
                                debug!("turbo pressed: source={:?}, mode=one-shot", t.src);
                            } else if !pressed && t.active {
                                t.active = false;
                                t.turbo_active = false;
                                state.set_button(t.src, false);
                                debug!("turbo released: source={:?}", t.src);
                            } else if t.active && !t.turbo_active && t.delay_ms > 0 {
                                if t.press_time.elapsed().as_millis() >= t.delay_ms as u128 {
                                    t.turbo_active = true;
                                    t.last_toggle = Instant::now();
                                    debug!(
                                        "turbo delay elapsed; toggling started: source={:?}, interval_ms={}",
                                        t.src,
                                        t.interval_ms
                                    );
                                }
                            } else if t.active && !t.turbo_active {
                                t.turbo_active = true;
                                t.last_toggle = Instant::now();
                                debug!("turbo toggling started: source={:?}, interval_ms={}", t.src, t.interval_ms);
                            } else if t.active && t.turbo_active
                                && t.last_toggle.elapsed().as_millis() >= t.interval_ms as u128 {
                                    t.phase = !t.phase;
                                    t.last_toggle = Instant::now();
                                    debug!("turbo phase changed: source={:?}, active={}", t.src, t.phase);
                                }
                            if t.active {
                                state.set_button(t.src, t.phase);
                            }
                        }

                        // L1: COMBO detection + suppression
                        let mut combo_triggers: Vec<(&Target, bool)> = Vec::new();
                        if !self.combo_runtimes.is_empty() {
                            let pre_combo = state.clone();
                            for c in &mut self.combo_runtimes {
                                let mod_held = pre_combo.button(c.modifier);
                                let key_held = pre_combo.button(c.key);
                                if mod_held {
                                    state.set_button(c.modifier, false);
                                    state.set_button(c.key, false);
                                    match c.modifier {
                                        Button::L2 => state.l2_analog = 0,
                                        Button::R2 => state.r2_analog = 0,
                                        _ => {}
                                    }
                                    match c.key {
                                        Button::L2 => state.l2_analog = 0,
                                        Button::R2 => state.r2_analog = 0,
                                        _ => {}
                                    }
                                }
                                let trigger = mod_held && key_held;
                                if trigger {
                                    c.active = true;
                                    combo_triggers.push((&c.output, true));
                                } else if c.active {
                                    c.active = false;
                                }
                            }
                        }

                        // L1: BLOCK suppression
                        let m = self.mapping.read().unwrap();
                        for btn in &m.blocked_buttons {
                            state.set_button(*btn, false);
                            match *btn {
                                Button::L2 => state.l2_analog = 0,
                                Button::R2 => state.r2_analog = 0,
                                _ => {}
                            }
                        }
                        drop(m);

                        // freeze L1 output
                        let l1 = state.clone();

                        // ========== L2: Virtual Input Generation ==========

                        // L2: MACRO detection (reads L1, Physical source only)
                        let now = Instant::now();
                        for m in &mut self.macro_runtimes {
                            if m.source != MacroSource::Physical {
                                continue;
                            }
                            let pressed = l1.button(m.trigger);
                            if pressed && !m.active {
                                m.activate(now);
                            }
                            if !pressed && m.active && matches!(m.mode, MacroMode::Hold) {
                                m.deactivate(&mut state, &mut keyboard_events);
                            }
                        }

                        // L2: REMAP (reads L1, writes state)
                        let m = self.mapping.read().unwrap();
                        m.apply(&l1, &mut state, &mut keyboard_events);
                        drop(m);

                        // L2: COMBO injection (writes state, or manages Combo-source macros)
                        for (target, _active) in &combo_triggers {
                            match target {
                                Target::Macro(name) => {
                                    for m in &mut self.macro_runtimes {
                                        if m.name == *name && m.source == MacroSource::Combo {
                                            m.activate(now);
                                        }
                                    }
                                }
                                Target::Keyboard(code) => keyboard_events.push((*code, *_active)),
                                _ => apply_target_to_state(&mut state, target, *_active),
                            }
                        }
                        // deactivate Combo-source macros when no active combo references them
                        for m in &mut self.macro_runtimes {
                            if m.source != MacroSource::Combo || !m.active { continue; }
                            if !matches!(m.mode, MacroMode::Hold) { continue; }
                            let any_combo_active = self.combo_runtimes.iter().any(|c| {
                                c.active && matches!(&c.output, Target::Macro(n) if n == &m.name)
                            });
                            if !any_combo_active {
                                m.deactivate(&mut state, &mut keyboard_events);
                            }
                        }

                        // L2: MACRO injection (writes macro step buttons to state)
                        for m in &mut self.macro_runtimes {
                            if m.active {
                                m.tick(&mut state, now, &mut keyboard_events);
                            }
                        }

                        // flush keyboard after all L2 sources have pushed events
                        // last-write-wins: later events overwrite earlier ones for same key
                        let mut current: HashMap<u16, bool> = HashMap::new();
                        for (code, pressed) in &keyboard_events {
                            current.insert(*code, *pressed);
                        }
                        let mut failed_releases = Vec::new();
                        for &code in self.last_keyboard.keys() {
                            if !current.contains_key(&code) {
                                if !self.keyboard.release(code) {
                                    failed_releases.push(code);
                                }
                            }
                        }
                        for (&code, &pressed) in &current {
                            if pressed {
                                let _ = self.keyboard.press(code);
                            } else {
                                let _ = self.keyboard.release(code);
                            }
                        }
                        self.last_keyboard = current;
                        for code in failed_releases {
                            self.last_keyboard.insert(code, true);
                        }

                        // ========== L3: Output ==========
                        frame.state = state.clone();
                        let out = self.codec.target
                            .encode_input(&frame, *seq)
                            .expect("DS5 USB source should encode to selected USB target");
                        if self.codec.target == TargetCodec::Ds4Usb {
                            trace!("DS4 input bytes: range=0..16, data={:02x?}", &out[..16]);
                            trace!("DS4 input bytes: range=16..32, data={:02x?}", &out[16..32]);
                        }
                        // per-frame output at trace level
                        {
                            let mut btn_names: Vec<&str> = Vec::new();
                            for btn in ALL_BUTTONS.iter() {
                                if state.button(*btn) {
                                    btn_names.push(btn.name());
                                }
                            }
                            trace!("virtual buttons active: buttons=[{}]", btn_names.join(" "));
                        }
                        self.log_button_diff(&physical_snapshot, &state);
                        out.to_vec()
                    } else {
                        warn!(
                            "failed to decode source input report; frame dropped: source={:?}, size={}, report_id={}",
                            self.codec.source,
                            n,
                            report_id_label(&buf[..n])
                        );
                        continue;
                    };

                    if let Some(repeat) = self.repeat_input.as_mut() {
                        // In repeat mode, physical BT frames update the latest target report only.
                        // The repeat scheduler is the sole UHID input sender, so the configured
                        // rate is an output cadence instead of "physical rate + repeat rate".
                        repeat.store(&out_report);
                    } else {
                        self.uhid.send_input(&out_report)?;
                    }
                }
                Ok(n) => {
                    trace!(
                        "short source input report ignored: source={:?}, size={n}, minimum_size={}, report_id={}",
                        self.codec.source,
                        input_report_size,
                        report_id_label(&buf[..n])
                    );
                    continue;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if is_disconnect_io_error(e) => {
                    warn!("failed to read input report: {e}");
                    info!("controller disconnected");
                    DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Err(e) => {
                    error!("failed to read input report: {e}");
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn handle_uhid_event(&mut self) -> io::Result<()> {
        loop {
            match self.uhid.recv_event() {
                Ok(Some(event)) => {
                    use crate::uhid::UhidEvent;
                    match event {
                        UhidEvent::Start => {
                            info!("virtual HID device started");
                        }
                        UhidEvent::Stop => {
                            warn!("virtual HID device stopped by kernel");
                        }
                        UhidEvent::Open => {
                            debug!("virtual HID device opened by client");
                        }
                        UhidEvent::Close => {
                            debug!("virtual HID device closed by client");
                        }
                        UhidEvent::Output { rtype, ref data } => {
                            if rtype == 1 {
                                trace!(
                                    "UHID OUTPUT received: size={}, report_id={}",
                                    data.len(),
                                    report_id_label(data)
                                );
                                // TargetCodec identifies the virtual output
                                // format; PhysicalCodec converts it for the
                                // real hidraw transport before the final write.
                                let encoded = self.codec.target
                                    .decode_output(data)
                                    .and_then(|command| self.codec.physical.encode_output(&command, &mut self.physical_output_state));
                                match encoded {
                                    Ok(encoded) => {
                                        if let Err(e) = self.hidraw.write_output(&encoded) {
                                            if is_disconnect_io_error(&e) {
                                                warn!("failed to write output report: {e}");
                                                info!("controller disconnected");
                                                DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                                break;
                                            }
                                            error!("failed to write output report: {e}");
                                        }
                                    }
                                    Err(CodecError::InvalidReport) => {
                                        warn!(
                                            "invalid output report dropped: target={:?}, controller={:?}",
                                            self.codec.target,
                                            self.codec.physical
                                        );
                                        warn!(
                                            "output report metadata: rtype={rtype}, size={}, report_id={}",
                                            data.len(),
                                            report_id_label(data)
                                        );
                                    }
                                }
                            } else {
                                warn!(
                                    "UHID OUTPUT ignored: unexpected rtype={rtype}, size={}, report_id={}",
                                    data.len(),
                                    report_id_label(data)
                                );
                            }
                        }
                        UhidEvent::GetReport { id, rnum, rtype } => {
                            trace!("UHID GET_REPORT received: id={id}, rnum={rnum}, rtype={rtype}");
                            match self.get_cached_report(rnum) {
                                Some(report) => {
                                    match report.source {
                                        CachedReportSource::PhysicalCache => {
                                            trace!("GET_REPORT served from cache: rnum={rnum}");
                                        }
                                        CachedReportSource::TargetFallback => {
                                            trace!("GET_REPORT served from target response: rnum={rnum}");
                                        }
                                    }
                                    if let Err(e) = self.uhid.send_get_report_reply(id, 0, &report.data) {
                                        warn!("failed to send GET_REPORT reply: {e}");
                                    }
                                }
                                None => {
                                    warn!("GET_REPORT unavailable; returning error: rnum={rnum}");
                                    if let Err(e) = self.uhid.send_get_report_reply(id, 1, &[]) {
                                        warn!("failed to send GET_REPORT reply: {e}");
                                    }
                                }
                            }
                        }
                        UhidEvent::Unknown(t) => {
                            warn!("unknown UHID event type: type={t}");
                        }
                        UhidEvent::SetReport { id, rnum, rtype, ref data } => {
                            trace!(
                                "UHID SET_REPORT received: id={id}, rnum={rnum}, rtype={rtype}, size={}, report_id={}",
                                data.len(),
                                report_id_label(data)
                            );
                            let mut reply_err = 0;
                            if rtype == 0 {
                                // PhysicalCodec decides whether this target
                                // feature report can be forwarded to hidraw.
                                // BT forwards only reports whose shape/CRC is
                                // known; other vendor/test commands are
                                // dropped without affecting the input path.
                                if let Some(full_data) =
                                    self.codec.physical.encode_set_report(
                                        self.codec.target,
                                        rnum,
                                        data,
                                    )
                                {
                                    if let Err(e) = self.hidraw.send_feature_report(&full_data) {
                                        warn!("failed to forward SET_REPORT: rnum={rnum}, error={e}");
                                        reply_err = 1;
                                        if is_disconnect_io_error(&e) {
                                            DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                        }
                                    }
                                } else if self.codec.physical == PhysicalCodec::Ds5Bt
                                    && self.physical_set_report_unsupported_warned.insert(rnum)
                                {
                                    debug!(
                                        "Bluetooth SET_REPORT dropped: unsupported, rnum=0x{rnum:02x}, size={}, report_id={}",
                                        data.len(),
                                        report_id_label(data)
                                    );
                                    debug!("Bluetooth SET_REPORT data: prefix_32={}", hex_prefix(data, 32));
                                }
                            }
                            if let Err(e) = self.uhid.send_set_report_reply(id, reply_err) {
                                warn!("failed to send SET_REPORT reply: {e}");
                            }
                            if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                                break;
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

fn public_control_error_message(code: &str) -> &'static str {
    match code {
        "load-failed" => "configuration load failed",
        "validation-failed" => "configuration validation failed",
        "mapping-failed" => "configuration mapping failed",
        "no-config" => "no config path is active",
        _ => "control request failed",
    }
}

pub(crate) fn warn_ignored_edge_passthroughs(
    cfg: &crate::config::Config,
    source_kind: SonyDeviceKind,
    target: TargetCodec,
) {
    if source_kind != SonyDeviceKind::DualSenseEdge {
        return;
    }
    if matches!(target, TargetCodec::Ds5UsbAuto) {
        return;
    }

    for name in ["left_paddle", "right_paddle", "left_fn", "right_fn"] {
        let remap = cfg.buttons.get(name).and_then(|button| button.remap.as_deref());
        if remap.is_none() || remap == Some("passthrough") {
            warn!("passthrough source may be ignored by target: source={name}");
        }
    }
}

fn is_disconnect_io_error(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EIO | libc::ENODEV | libc::ENXIO)
    )
}

fn report_id_label(data: &[u8]) -> String {
    match data.first() {
        Some(report_id) => format!("0x{report_id:02x}"),
        None => "none".to_string(),
    }
}

fn hex_prefix(data: &[u8], max_len: usize) -> String {
    let shown = data.len().min(max_len);
    let mut out = String::with_capacity(shown * 3 + 16);
    for (i, byte) in data[..shown].iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    if shown < data.len() {
        out.push_str(" ...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_mode_validation_accepts_only_named_modes() {
        assert!(matches!(parse_repeat_mode("passthrough"), Ok(RepeatMode::Passthrough)));
        assert!(matches!(parse_repeat_mode("seq_only"), Ok(RepeatMode::SeqOnly)));
        assert!(matches!(parse_repeat_mode("seq_ts"), Ok(RepeatMode::SeqAndTimestamp)));
        let error = match parse_repeat_mode("invalid") {
            Err(error) => error,
            Ok(_) => panic!("invalid repeat mode was accepted"),
        };
        assert_eq!(
            error,
            "invalid DSEUHID_BT_DS5_USB_REPEAT_MODE=invalid; expected passthrough|seq_only|seq_ts"
        );
    }

    #[test]
    fn repeat_rate_validation_enforces_inclusive_bounds() {
        assert_eq!(parse_repeat_hz("TEST_REPEAT_HZ", "1"), Ok(1));
        assert_eq!(parse_repeat_hz("TEST_REPEAT_HZ", "2000"), Ok(2000));
        for value in ["0", "2001", "invalid"] {
            assert_eq!(
                parse_repeat_hz("TEST_REPEAT_HZ", value).unwrap_err(),
                format!(
                    "invalid TEST_REPEAT_HZ={value}; expected integer 1..=2000"
                )
            );
        }
    }

    #[test]
    fn ds5_repeat_advances_sequence_without_timestamp_in_seq_only_mode() {
        let mut report = [0u8; 64];
        report[28..32].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        let mut seq = 0x41;

        advance_repeat_report(
            &mut report,
            &mut seq,
            10,
            RepeatMode::SeqOnly,
            RepeatTarget::Ds5Usb,
        );

        assert_eq!(seq, 0x42);
        assert_eq!(report[7], 0x42);
        assert_eq!(&report[28..32], &0x1234_5678u32.to_le_bytes());
    }

    #[test]
    fn ds4_repeat_advances_ds4_sequence_fields() {
        let mut report = [0u8; 64];
        report[7] = 0x03;
        report[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        report[34] = 0x12;
        let mut seq = 0x3F;

        advance_repeat_report(
            &mut report,
            &mut seq,
            10,
            RepeatMode::SeqOnly,
            RepeatTarget::Ds4Usb,
        );

        assert_eq!(seq, 0);
        assert_eq!(report[7], 0x03);
        assert_eq!(&report[10..12], &0u16.to_le_bytes());
        assert_eq!(report[34], 0);
    }

    #[test]
    fn public_control_errors_do_not_expose_config_details() {
        assert_eq!(
            public_control_error_message("load-failed"),
            "configuration load failed"
        );
        assert_eq!(
            public_control_error_message("validation-failed"),
            "configuration validation failed"
        );
        assert_eq!(
            public_control_error_message("mapping-failed"),
            "configuration mapping failed"
        );
    }
}
