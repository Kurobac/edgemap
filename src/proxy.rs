use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, RwLock};

use log::{debug, error, info, trace, warn};
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::os::fd::BorrowedFd;

use crate::codec::{CodecError, CodecPipeline, PhysicalCodec, PhysicalOutputState, SourceCodec, TargetCodec};
use crate::device::{HidrawDevice, SonyDeviceKind};
use crate::mapping::{ComboRule, MacroMode, MacroRule, MacroSource, MappingConfig, Target, Trigger, TurboConfig};
use crate::report::Button;
use crate::uhid::UhidDevice;
use std::time::Instant;

const DS5_USB_SYNTHETIC_SENSOR_TIMESTAMP_DELTA: u32 = 3000;


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
                debug!("macro '{}': +{elapsed}ms press {:?}", self.name, step.action);
            }
            if elapsed >= step.release_ms && step.pressed {
                step.pressed = false;
                step.done = true;
                match &step.action {
                    crate::mapping::StepTarget::Gamepad(btn) => state.set_button(*btn, false),
                    crate::mapping::StepTarget::Keyboard(code) => keyboard_events.push((*code, false)),
                }
                debug!("macro '{}': +{elapsed}ms release {:?}", self.name, step.action);
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
                    debug!("macro '{}': loop, resetting", self.name);
                    self.step_start = now;
                    for step in &mut self.steps {
                        step.pressed = false;
                        step.done = false;
                    }
                }
                MacroMode::Single => {
                    debug!("macro '{}': completed", self.name);
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

#[derive(Debug, PartialEq)]
pub enum ExitReason {
    UserShutdown,
    DeviceGone,
    ConfigChanged,
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static DISCONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_running() -> bool {
    RUNNING.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn setup_signal_handler() {
    unsafe {
        let handler = SigHandler::SigAction(handle_signal);
        let action = SigAction::new(handler, SaFlags::empty(), SigSet::empty());
        let _ = sigaction(Signal::SIGINT, &action);
        let _ = sigaction(Signal::SIGTERM, &action);
    }
}

extern "C" fn handle_signal(
    _sig: libc::c_int,
    _info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
}

pub struct Proxy {
    hidraw: HidrawDevice,
    uhid: UhidDevice,
    mapping: Arc<RwLock<MappingConfig>>,
    config_path: String,
    report_cache: HashMap<u8, Vec<u8>>,
    codec: CodecPipeline,
    source_kind: SonyDeviceKind,
    output_device_config: String,
    recreate_uhid: bool,
    keyboard: crate::keyboard::KeyboardDevice,
    last_keyboard: HashMap<u16, bool>,
    last_snapshot: Option<crate::report::GamepadState>,
    last_output: Option<crate::report::GamepadState>,
    ds5_usb_sensor_timestamp: u32,
    physical_output_state: PhysicalOutputState,
    physical_set_report_unsupported_warned: HashSet<u8>,
    turbo_runtimes: Vec<TurboRuntime>,
    combo_runtimes: Vec<ComboRuntime>,
    macro_runtimes: Vec<MacroRuntime>,
    fifo_fd: OwnedFd,
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
        if let Some(data) = self.report_cache.get(&report_id) {
            return Some(CachedReport {
                source: CachedReportSource::PhysicalCache,
                data: data.clone(),
            });
        }
        self.codec.target.fallback_feature_report(report_id).map(|data| CachedReport {
            source: CachedReportSource::TargetFallback,
            data,
        })
    }

    pub fn new(hidraw: HidrawDevice, uhid: UhidDevice, mapping: Arc<RwLock<MappingConfig>>, config_path: &str, report_cache: HashMap<u8, Vec<u8>>, codec: CodecPipeline, source_kind: SonyDeviceKind, output_device_config: String, keyboard: crate::keyboard::KeyboardDevice, fifo_file: std::fs::File) -> Self {
        let fifo_fd = OwnedFd::from(fifo_file);
        let (turbo_runtimes, combo_runtimes, macro_runtimes) = {
            let m = mapping.read().unwrap();
            let turbos: Vec<_> = m.turbo_configs.iter()
                .map(TurboRuntime::from_config)
                .collect();
            let combos: Vec<_> = m.combo_configs.iter()
                .map(ComboRuntime::from_combo_rule)
                .collect();
            let macros: Vec<_> = m.macro_configs.iter()
                .map(MacroRuntime::from_macro_rule)
                .collect();
            (turbos, combos, macros)
        };
        Self { hidraw, uhid, mapping, config_path: config_path.to_string(), report_cache, codec, source_kind, output_device_config, recreate_uhid: false, keyboard, last_keyboard: HashMap::new(), last_snapshot: None, last_output: None, ds5_usb_sensor_timestamp: 0, physical_output_state: PhysicalOutputState::default(), physical_set_report_unsupported_warned: HashSet::new(), turbo_runtimes, combo_runtimes, macro_runtimes, fifo_fd }
    }

    pub fn forget_restore_on_physical_disconnect(&mut self) {
        self.hidraw.clear_restored_paths();
    }

    pub fn config_path(&self) -> &str {
        &self.config_path
    }

    fn reload_config(&mut self) {
        if self.config_path.is_empty() {
            info!("No config path set, skipping reload (running passthrough)");
            return;
        }
        let path = self.config_path.clone();
        self.reload_config_from(path);
    }

    fn reload_config_from(&mut self, path: String) {
        let cfg = match crate::config::Config::load(&path) {
            Ok(cfg) => cfg,
            Err(e) => {
                error!("Failed to load config on reload: {e}; keeping previous config");
                return;
            }
        };
        if let Err(e) = crate::config::validate(&cfg) {
            error!("Config reload validation failed: {e}; keeping previous config");
            return;
        }
        let new_mapping = match cfg.to_mapping_config() {
            Ok(m) => {
                // warn for missing button sections
                for name in crate::config::ALL_BUTTON_NAMES {
                    if !cfg.buttons.contains_key(*name) {
                        warn!("{name}: not configured, passthrough");
                    }
                }
                warn_ignored_edge_passthroughs(&cfg, self.source_kind, self.codec.target);
                m
            }
            Err(e) => {
                error!("Failed to build mapping on reload: {e}; keeping previous config");
                return;
            }
        };
        let new_output_device = cfg.output_device.clone();
        *self.mapping.write().unwrap() = new_mapping;
        self.config_path = path;
        self.last_snapshot = None;
        self.last_output = None;
        info!("Config reloaded from {}", self.config_path);
        if new_output_device != self.output_device_config {
            info!("output_device changed ({} → {}), will recreate virtual device", self.output_device_config, new_output_device);
            self.recreate_uhid = true;
        }
        // rebuild turbo runtimes from the new mapping
        self.turbo_runtimes = self.mapping.read().unwrap().turbo_configs.iter()
            .map(TurboRuntime::from_config)
            .collect();
        // rebuild combo runtimes from the new mapping
        self.combo_runtimes = self.mapping.read().unwrap().combo_configs.iter()
            .map(ComboRuntime::from_combo_rule)
            .collect();
        // rebuild macro runtimes from the new mapping
        self.macro_runtimes = self.mapping.read().unwrap().macro_configs.iter()
            .map(MacroRuntime::from_macro_rule)
            .collect();
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
            debug!("in: {}  →  out: {}", phys_changes.join(" "), out_display);
        }

        self.last_snapshot = Some(snapshot.clone());
        self.last_output = Some(output.clone());
    }

    pub fn run(&mut self) -> ExitReason {
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);

        let ep_fd = match Epoll::new(EpollCreateFlags::empty()) {
            Ok(fd) => fd,
            Err(e) => {
                error!("Failed to create epoll: {e}");
                return ExitReason::UserShutdown;
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
            error!("Failed to add hidraw to epoll: {e}");
            return ExitReason::UserShutdown;
        }

        let uhid_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            2,
        );
        if let Err(e) = ep_fd.add(uhid_bfd, uhid_event) {
            error!("Failed to add uhid to epoll: {e}");
            return ExitReason::UserShutdown;
        }

        let fifo_bfd = unsafe { BorrowedFd::borrow_raw(self.fifo_fd.as_raw_fd()) };
        let fifo_event = EpollEvent::new(
            EpollFlags::EPOLLIN,
            3,
        );
        if let Err(e) = ep_fd.add(fifo_bfd, fifo_event) {
            warn!("Failed to add FIFO to epoll: {e} (control pipe unavailable)");
        }

        info!("Proxy running. Press Ctrl+C to stop.");

        let mut seq: u8 = 0;
        let mut events = [EpollEvent::empty(); 8];

        'run: while RUNNING.load(std::sync::atomic::Ordering::SeqCst)
            && !DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
            match ep_fd.wait(&mut events, 16u16) {
                Ok(n) => {
                    for ev in events.iter().take(n) {
                        let fd_num = ev.data();

                        if fd_num == 1 {
                            if let Err(e) = self.handle_hidraw_input(&mut seq) {
                                error!("hidraw handler error: {e}");
                                break 'run;
                            }
                        } else if fd_num == 2 {
                            if let Err(e) = self.handle_uhid_event() {
                                error!("UHID handler error: {e}");
                                break 'run;
                            }
                        } else if fd_num == 3 {
                            self.handle_fifo_command();
                        }
                    }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("epoll wait error: {e}");
                    break;
                }
            }
            if self.recreate_uhid {
                break;
            }
        }

        info!("Proxy stopped.");

        if !RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
            ExitReason::UserShutdown
        } else if self.recreate_uhid {
            ExitReason::ConfigChanged
        } else if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
            ExitReason::DeviceGone
        } else {
            ExitReason::UserShutdown
        }
    }

    fn handle_fifo_command(&mut self) {
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe {
                libc::read(
                    self.fifo_fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n <= 0 {
                break;
            }
            let data = &buf[..n as usize];
            for line in data.split(|b| *b == b'\n') {
                let line = line.trim_ascii();
                if line.is_empty() {
                    continue;
                }
                if line == b"reload" {
                    info!("FIFO: reload requested");
                    self.reload_config();
                } else if let Some(path) = line.strip_prefix(b"switch-config ") {
                    let path_str = String::from_utf8_lossy(path).trim().to_string();
                    info!("FIFO: switch-config to {}", path_str);
                    self.reload_config_from(path_str);
                } else {
                    debug!("FIFO: unknown command: {}", String::from_utf8_lossy(line));
                }
            }
        }
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
                                debug!("turbo {:?}: press (one-shot)", t.src);
                            } else if !pressed && t.active {
                                t.active = false;
                                t.turbo_active = false;
                                state.set_button(t.src, false);
                                debug!("turbo {:?}: released", t.src);
                            } else if t.active && !t.turbo_active && t.delay_ms > 0 {
                                if t.press_time.elapsed().as_millis() >= t.delay_ms as u128 {
                                    t.turbo_active = true;
                                    t.last_toggle = Instant::now();
                                    debug!("turbo {:?}: delay expired, starting toggle (interval={}ms)", t.src, t.interval_ms);
                                }
                            } else if t.active && !t.turbo_active {
                                t.turbo_active = true;
                                t.last_toggle = Instant::now();
                                debug!("turbo {:?}: starting toggle (interval={}ms)", t.src, t.interval_ms);
                            } else if t.active && t.turbo_active
                                && t.last_toggle.elapsed().as_millis() >= t.interval_ms as u128 {
                                    t.phase = !t.phase;
                                    t.last_toggle = Instant::now();
                                    debug!("turbo {:?}: toggle → {}", t.src, t.phase);
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
                        if self.codec.source == SourceCodec::Ds5Bt
                            && matches!(
                                self.codec.target,
                                TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced
                            )
                        {
                            // Bluetooth input uses a different timestamp
                            // domain. For a USB virtual target, synthesize the
                            // DS5 USB-scale sensor timestamp and keep motion
                            // axes unchanged.
                            self.ds5_usb_sensor_timestamp =
                                self.ds5_usb_sensor_timestamp.wrapping_add(
                                    DS5_USB_SYNTHETIC_SENSOR_TIMESTAMP_DELTA,
                                );
                            frame.sensor_timestamp_override =
                                Some(self.ds5_usb_sensor_timestamp);
                        }
                        let out = self.codec.target
                            .encode_input(&frame, *seq)
                            .expect("DS5 USB source should encode to selected USB target");
                        if self.codec.target == TargetCodec::Ds4Usb {
                            trace!("ds4 raw[..32]: {:02x?}", &out[..32]);
                        }
                        // per-frame output at trace level
                        {
                            let mut btn_names: Vec<&str> = Vec::new();
                            for btn in ALL_BUTTONS.iter() {
                                if state.button(*btn) {
                                    btn_names.push(btn.name());
                                }
                            }
                            trace!("out: [{}]", btn_names.join(" "));
                        }
                        self.log_button_diff(&physical_snapshot, &state);
                        out.to_vec()
                    } else {
                        warn!(
                            "source input decode failed; dropping frame: source={:?}, size={}, report_id={}",
                            self.codec.source,
                            n,
                            report_id_label(&buf[..n])
                        );
                        continue;
                    };

                    self.uhid.send_input(&out_report)?;
                }
                Ok(n) => {
                    trace!(
                        "short hidraw input report ignored: source={:?}, size={n}, expected>={}, report_id={}",
                        self.codec.source,
                        input_report_size,
                        report_id_label(&buf[..n])
                    );
                    continue;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if is_disconnect_io_error(e) => {
                    warn!("hidraw input failed; controller disconnected? {e}");
                    DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Err(e) => {
                    error!("hidraw read error: {e}");
                    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
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
                            info!("UHID device started");
                        }
                        UhidEvent::Stop => {
                            warn!("UHID device stopped");
                        }
                        UhidEvent::Open => {
                            debug!("UHID device opened by client");
                        }
                        UhidEvent::Close => {
                            debug!("UHID device closed by client");
                        }
                        UhidEvent::Output { rtype, ref data } => {
                            if rtype == 1 {
                                trace!(
                                    "UHID OUTPUT: size={}, report_id={}",
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
                                                warn!("hidraw output failed; controller disconnected? {e}");
                                                DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                                break;
                                            }
                                            error!("Failed to forward output report: {e}");
                                        }
                                    }
                                    Err(CodecError::InvalidReport) => {
                                        warn!(
                                            "Dropping invalid target output report: target={:?}, physical={:?}, rtype={rtype}, size={}, report_id={}",
                                            self.codec.target,
                                            self.codec.physical,
                                            data.len(),
                                            report_id_label(data)
                                        );
                                    }
                                }
                            } else {
                                warn!(
                                    "UHID Output with unexpected rtype={rtype}, size={}, report_id={}, ignoring",
                                    data.len(),
                                    report_id_label(data)
                                );
                            }
                        }
                        UhidEvent::GetReport { id, rnum, rtype } => {
                            trace!("UHID GET_REPORT: id={id}, rnum={rnum}, rtype={rtype}");
                            match self.get_cached_report(rnum) {
                                Some(report) => {
                                    match report.source {
                                        CachedReportSource::PhysicalCache => {
                                            trace!("GET_REPORT rnum={rnum}: served from physical cache");
                                        }
                                        CachedReportSource::TargetFallback => {
                                            trace!("GET_REPORT rnum={rnum}: served from target fallback");
                                        }
                                    }
                                    if let Err(e) = self.uhid.send_get_report_reply(id, 0, &report.data) {
                                        warn!("Failed to send GET_REPORT reply: {e}");
                                    }
                                }
                                None => {
                                    warn!("GET_REPORT rnum={rnum}: not cached, returning error");
                                    if let Err(e) = self.uhid.send_get_report_reply(id, 1, &[]) {
                                        warn!("Failed to send GET_REPORT reply: {e}");
                                    }
                                }
                            }
                        }
                        UhidEvent::Unknown(t) => {
                            warn!("Unknown UHID event type: {t}");
                        }
                        UhidEvent::SetReport { id, rnum, rtype, ref data } => {
                            trace!(
                                "UHID SET_REPORT id={id}, rnum={rnum}, rtype={rtype}, size={}, report_id={}",
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
                                        warn!("Failed to forward set_report rnum={rnum}: {e}");
                                        reply_err = 1;
                                        if is_disconnect_io_error(&e) {
                                            DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                        }
                                    }
                                } else if self.codec.physical == PhysicalCodec::Ds5Bt
                                    && self.physical_set_report_unsupported_warned.insert(rnum)
                                {
                                    warn!(
                                        "physical Bluetooth SET_REPORT forwarding is not supported for this report yet; dropping rnum=0x{rnum:02x}, size={}, report_id={}, data={}",
                                        data.len(),
                                        report_id_label(data),
                                        hex_prefix(data, 64)
                                    );
                                }
                            }
                            if let Err(e) = self.uhid.send_set_report_reply(id, reply_err) {
                                warn!("Failed to send SET_REPORT reply: {e}");
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
            warn!("{name}: passthrough may be ignored by the target");
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
