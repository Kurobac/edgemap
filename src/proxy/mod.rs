use std::collections::HashSet;
use std::fmt;
use std::io;
use std::sync::{Arc, RwLock};

use log::{debug, error, info, trace, warn};
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use nix::sys::timerfd::{ClockId, Expiration, TimerFd, TimerFlags, TimerSetTimeFlags};
use std::os::fd::{AsFd, BorrowedFd};

use crate::codec::{
    CodecPipeline, ControllerFrame, FeatureReportCache, PhysicalOutputState, TargetCodec,
};
use crate::config::ActiveConfig;
use crate::control::{ControlRequest, ControlServer};
use crate::device::{HidrawDevice, SonyDeviceKind};
use crate::mapping::MappingConfig;
use crate::shutdown::ShutdownSignal;
use crate::uhid::UhidDevice;
use std::time::{Duration, Instant};

mod haptics;
mod output;
mod pipeline;
mod repeat;
mod runtime;
mod uhid_events;

pub(crate) use haptics::bt_haptics_buffer_from_env;
use haptics::LiveHaptics;
use pipeline::{transform, transform_timer};
pub(crate) use repeat::validate_repeat_env;
use repeat::RepeatInput;
use runtime::{MappingRuntimes, ALL_BUTTONS};

#[cfg(test)]
use repeat::{advance_repeat_report, parse_repeat_hz, parse_repeat_mode, RepeatMode, RepeatTarget};

#[derive(Debug, PartialEq)]
pub enum ExitReason {
    UserShutdown,
    DeviceGone,
    ConfigChanged,
    FatalError,
}

struct EscapedLogValue<'a>(&'a str);

impl fmt::Display for EscapedLogValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.0)
    }
}

pub(crate) struct ProxyInit {
    pub(crate) hidraw: HidrawDevice,
    pub(crate) uhid: UhidDevice,
    pub(crate) keyboard: crate::keyboard::KeyboardDevice,
    pub(crate) mapping: Arc<RwLock<MappingConfig>>,
    pub(crate) active_config: Option<ActiveConfig>,
    pub(crate) report_cache: FeatureReportCache,
    pub(crate) codec: CodecPipeline,
    pub(crate) source_kind: SonyDeviceKind,
    pub(crate) output_device_config: String,
}

static DISCONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Proxy {
    hidraw: HidrawDevice,
    uhid: UhidDevice,
    mapping: Arc<RwLock<MappingConfig>>,
    active_config: Option<ActiveConfig>,
    report_cache: FeatureReportCache,
    codec: CodecPipeline,
    source_kind: SonyDeviceKind,
    output_device_config: String,
    recreate_uhid: bool,
    keyboard: crate::keyboard::KeyboardDevice,
    last_snapshot: Option<crate::model::GamepadState>,
    last_output: Option<crate::model::GamepadState>,
    last_frame: Option<ControllerFrame>,
    repeat_input: Option<RepeatInput>,
    physical_output_state: PhysicalOutputState,
    live_haptics: LiveHaptics,
    speaker_active: bool,
    physical_set_report_unsupported_warned: HashSet<u8>,
    runtimes: MappingRuntimes,
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
        self.codec
            .target
            .fallback_feature_report(report_id)
            .map(|data| CachedReport {
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
            active_config,
            report_cache,
            codec,
            source_kind,
            output_device_config,
        } = init;
        let repeat_input = RepeatInput::from_env(codec);
        let mut physical_output_state = PhysicalOutputState::default();
        if codec.physical == crate::codec::PhysicalCodec::Ds5Bt {
            physical_output_state.ds5_bt_haptics_buffer = bt_haptics_buffer_from_env()
                .expect("haptics environment was validated at daemon startup");
            info!(
                "Bluetooth haptics buffer: value={}",
                physical_output_state.ds5_bt_haptics_buffer
            );
        }
        let runtimes = {
            let mapping = mapping.read().unwrap();
            MappingRuntimes::from_mapping(&mapping)
        };
        Self {
            hidraw,
            uhid,
            mapping,
            active_config,
            report_cache,
            codec,
            source_kind,
            output_device_config,
            recreate_uhid: false,
            keyboard,
            last_snapshot: None,
            last_output: None,
            last_frame: None,
            repeat_input,
            physical_output_state,
            live_haptics: LiveHaptics::default(),
            speaker_active: false,
            physical_set_report_unsupported_warned: HashSet::new(),
            runtimes,
        }
    }

    pub fn forget_restore_on_physical_disconnect(&mut self) {
        self.hidraw.clear_restored_paths();
    }

    pub fn active_config(&self) -> Option<&ActiveConfig> {
        self.active_config.as_ref()
    }

    fn apply_active_config(
        &mut self,
        active_config: ActiveConfig,
    ) -> Result<(), (&'static str, String)> {
        let cfg = match active_config.parse() {
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
        info!(
            "config applied: source={}",
            EscapedLogValue(active_config.source())
        );
        self.active_config = Some(active_config);
        self.last_snapshot = None;
        self.last_output = None;
        self.last_frame = None;
        if let Some(repeat) = self.repeat_input.as_mut() {
            repeat.clear();
        }
        self.keyboard.sync(&HashSet::new());
        if new_output_device != self.output_device_config {
            info!(
                "output device changed: previous={}, current={}",
                self.output_device_config, new_output_device
            );
            info!("virtual HID device recreation requested");
            self.recreate_uhid = true;
        }
        self.output_device_config = new_output_device;
        self.runtimes = new_runtimes;
        Ok(())
    }

    fn log_button_diff(
        &mut self,
        snapshot: &crate::model::GamepadState,
        output: &crate::model::GamepadState,
    ) {
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
            let out_display = if out_names.is_empty() {
                "[none]".to_string()
            } else {
                out_names.join(" ")
            };
            debug!(
                "controller button changes: buttons=[{}]",
                phys_changes.join(" ")
            );
            debug!("virtual buttons active: buttons=[{out_display}]");
        }

        self.last_snapshot = Some(snapshot.clone());
        self.last_output = Some(output.clone());
    }

    fn encode_frame(
        &mut self,
        frame: &ControllerFrame,
        now: Instant,
        timer_tick: bool,
        seq: u8,
    ) -> Vec<u8> {
        let pipeline = {
            let mapping = self.mapping.read().unwrap();
            if timer_tick {
                transform_timer(frame, &mapping, &mut self.runtimes, now)
            } else {
                transform(frame, &mapping, &mut self.runtimes, now)
            }
        };

        self.keyboard.sync(&pipeline.keyboard);

        let mut output_frame = frame.clone();
        output_frame.state = pipeline.state.clone();
        let out = self
            .codec
            .target
            .encode_input(&output_frame, seq)
            .expect("DS5 USB source should encode to selected USB target");
        if self.codec.target == TargetCodec::Ds4Usb {
            trace!("DS4 input bytes: range=0..16, data={:02x?}", &out[..16]);
            trace!("DS4 input bytes: range=16..32, data={:02x?}", &out[16..32]);
        }
        let mut button_names = Vec::new();
        for button in ALL_BUTTONS {
            if pipeline.state.button(*button) {
                button_names.push(button.name());
            }
        }
        trace!(
            "virtual buttons active: buttons=[{}]",
            button_names.join(" ")
        );
        self.log_button_diff(&pipeline.physical_snapshot, &pipeline.state);
        out.to_vec()
    }

    fn haptics_device(&self) -> crate::control::HapticsDevice {
        use crate::control::HapticsDevice;
        match (self.codec.target, self.source_kind) {
            (TargetCodec::Ds5UsbForced, _) => HapticsDevice::DualSense,
            (_, SonyDeviceKind::DualSense) => HapticsDevice::DualSense,
            // DS4 emulation changes the gamepad only; its audio endpoint stays
            // native to the physical DualSense, as with a USB source.
            (_, SonyDeviceKind::DualSenseEdge) => HapticsDevice::DualSenseEdge,
        }
    }

    fn next_timing_deadline(&self) -> Option<Instant> {
        self.runtimes
            .next_deadline()
            .into_iter()
            .chain(
                self.repeat_input
                    .as_ref()
                    .and_then(RepeatInput::next_deadline),
            )
            .chain(self.live_haptics.next_deadline())
            .min()
    }

    fn arm_timing_timer(&self, timer: &TimerFd) -> Result<(), String> {
        let Some(deadline) = self.next_timing_deadline() else {
            return timer
                .unset()
                .map_err(|error| format!("failed to disarm timing timer: {error}"));
        };
        let delay = one_shot_delay(deadline, Instant::now());
        timer
            .set(
                Expiration::OneShot(delay.into()),
                TimerSetTimeFlags::empty(),
            )
            .map_err(|error| format!("failed to arm timing timer: {error}"))
    }

    fn handle_timing_tick(&mut self, seq: &mut u8, now: Instant) -> io::Result<()> {
        let haptics_active = self.live_haptics.next_deadline().is_some();
        self.handle_haptics_tick(now);
        if haptics_active && DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let runtime_due = self
            .runtimes
            .next_deadline()
            .is_some_and(|deadline| deadline <= now);
        let repeat_due = self
            .repeat_input
            .as_ref()
            .and_then(RepeatInput::next_deadline)
            .is_some_and(|deadline| deadline <= now);
        if !runtime_due && !repeat_due {
            return Ok(());
        }

        let updated_report = if runtime_due {
            let frame = self.last_frame.clone().ok_or_else(|| {
                io::Error::other("timing runtime is active without a cached controller frame")
            })?;
            if self.repeat_input.is_none() {
                *seq = seq.wrapping_add(1);
            }
            Some(self.encode_frame(&frame, now, true, *seq))
        } else {
            None
        };

        if let Some(repeat) = self.repeat_input.as_mut() {
            if let Some(report) = updated_report.as_deref() {
                repeat.store_runtime(report);
            }
            if let Some(report) = repeat.prepare_report(now, seq) {
                self.uhid.send_input(report)?;
            }
        } else if let Some(report) = updated_report {
            self.uhid.send_input(&report)?;
        }
        Ok(())
    }

    fn clear_timing_state(&mut self) {
        self.stop_live_haptics();
        self.last_frame = None;
        if let Some(repeat) = self.repeat_input.as_mut() {
            repeat.clear();
        }
        let mapping = self.mapping.read().unwrap();
        self.runtimes = MappingRuntimes::from_mapping(&mapping);
        self.keyboard.sync(&HashSet::new());
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
        let timing_timer = match TimerFd::new(ClockId::CLOCK_MONOTONIC, TimerFlags::TFD_CLOEXEC) {
            Ok(timer) => timer,
            Err(error) => {
                error!("failed to create monotonic timing timer: {error}");
                return ExitReason::FatalError;
            }
        };

        let hidraw_bfd = unsafe { BorrowedFd::borrow_raw(self.hidraw.as_raw_fd()) };
        let uhid_bfd = unsafe { BorrowedFd::borrow_raw(self.uhid.as_raw_fd()) };

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

        let timing_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            5,
        );
        if let Err(error) = ep_fd.add(timing_timer.as_fd(), timing_event) {
            error!("failed to register timing timer with epoll: {error}");
            return ExitReason::FatalError;
        }

        let pcm_receiver = if self.codec.physical == crate::codec::PhysicalCodec::Ds5Bt {
            match crate::control::haptics::PcmReceiver::bind(control.runtime_dir()) {
                Ok(receiver) => {
                    match ep_fd.add(receiver.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 6)) {
                        Ok(()) => Some(receiver),
                        Err(error) => {
                            error!("Bluetooth PCM socket registration failed: {error}");
                            None
                        }
                    }
                }
                Err(error) => {
                    error!("Bluetooth PCM socket unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };
        let mut control_state = control.state();
        control_state.uhid_ready = true;
        control_state.bt_haptics = pcm_receiver.as_ref().map(|_| self.haptics_device());
        control.set_state(control_state);
        info!("proxy started");

        let mut seq: u8 = 0;
        let mut events = [EpollEvent::empty(); 8];

        let exit_reason = 'run: loop {
            let mut timing_ready = false;
            match ep_fd.wait(&mut events, EpollTimeout::NONE) {
                Ok(n) => {
                    for ev in events.iter().take(n) {
                        let fd_num = ev.data();
                        let failure = EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP;

                        if fd_num == 1 {
                            if ev.events().intersects(failure) {
                                warn!("hidraw fd reported a poll failure");
                                info!("controller disconnected");
                                break 'run ExitReason::DeviceGone;
                            }
                            if let Err(e) = self.handle_hidraw_input(&mut seq) {
                                error!("hidraw event handler failed: {e}");
                                break 'run ExitReason::FatalError;
                            }
                            if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                                break 'run ExitReason::DeviceGone;
                            }
                        } else if fd_num == 2 {
                            if ev.events().intersects(failure) {
                                error!("UHID fd reported a poll failure");
                                break 'run ExitReason::FatalError;
                            }
                            if let Err(e) = self.handle_uhid_event() {
                                error!("UHID event handler failed: {e}");
                                break 'run ExitReason::FatalError;
                            }
                            if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                                break 'run ExitReason::DeviceGone;
                            }
                        } else if fd_num == 3 {
                            if ev.events().intersects(failure) {
                                error!("control socket fd reported a poll failure");
                                break 'run ExitReason::FatalError;
                            }
                            if let Err(e) = self.handle_control_requests(control) {
                                error!("control socket event handler failed: {e}");
                                break 'run ExitReason::FatalError;
                            }
                            if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                                break 'run ExitReason::DeviceGone;
                            }
                            if self.recreate_uhid {
                                break 'run ExitReason::ConfigChanged;
                            }
                        } else if fd_num == 4 {
                            if ev.events().intersects(failure) {
                                error!("shutdown signal fd reported a poll failure");
                                break 'run ExitReason::FatalError;
                            }
                            break 'run match shutdown.consume() {
                                Ok(true) => ExitReason::UserShutdown,
                                Ok(false) => {
                                    error!(
                                        "shutdown signal fd was readable but contained no signal"
                                    );
                                    ExitReason::FatalError
                                }
                                Err(e) => {
                                    error!("failed to read shutdown signal: {e}");
                                    ExitReason::FatalError
                                }
                            };
                        } else if fd_num == 6 {
                            if let Some(receiver) = &pcm_receiver {
                                let now = Instant::now();
                                if let Err(error) = receiver.drain(|samples| {
                                    self.live_haptics.push(samples, now);
                                }) {
                                    error!("Bluetooth PCM receive failed: {error}");
                                }
                            }
                        } else if fd_num == 5 {
                            if ev.events().intersects(failure) {
                                error!("timing timer fd reported a poll failure");
                                break 'run ExitReason::FatalError;
                            }
                            if let Err(error) = timing_timer.wait() {
                                error!("failed to consume timing timer expiration: {error}");
                                break 'run ExitReason::FatalError;
                            }
                            timing_ready = true;
                        } else {
                            error!("unknown epoll event token: token={fd_num}");
                            break 'run ExitReason::FatalError;
                        }
                    }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("epoll wait failed: {e}");
                    break 'run ExitReason::FatalError;
                }
            }
            if timing_ready {
                if let Err(error) = self.handle_timing_tick(&mut seq, Instant::now()) {
                    error!("timing event handler failed: {error}");
                    break 'run ExitReason::FatalError;
                }
                if DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst) {
                    break 'run ExitReason::DeviceGone;
                }
            }
            if self.recreate_uhid {
                break 'run ExitReason::ConfigChanged;
            }
            if let Err(error) = self.arm_timing_timer(&timing_timer) {
                error!("{error}");
                break 'run ExitReason::FatalError;
            }
        };

        if exit_reason == ExitReason::DeviceGone {
            self.live_haptics = LiveHaptics::default();
        }
        self.clear_timing_state();
        info!("proxy stopped");
        exit_reason
    }

    fn handle_control_requests(&mut self, control: &mut ControlServer) -> io::Result<()> {
        for pending in control.drain_requests()? {
            let request = pending.request;
            let result = match &request {
                ControlRequest::SwitchConfig(active_config) => {
                    info!(
                        "control request received: action=switch-config, source={}",
                        EscapedLogValue(active_config.source())
                    );
                    self.apply_active_config(active_config.clone())
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
                    error!("control request failed: code={code}");
                    control.reply_error(pending.client, code, public_control_error_message(code));
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
                Ok(0) => {
                    warn!("failed to read input report: end of file");
                    info!("controller disconnected");
                    DISCONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Ok(n) if n >= input_report_size => {
                    *seq = seq.wrapping_add(1);
                    let Ok(frame) = self.codec.source.decode_input(&buf[..n]) else {
                        warn!(
                            "failed to decode source input report; frame dropped: source={:?}, size={}, report_id={}",
                            self.codec.source,
                            n,
                            report_id_label(&buf[..n])
                        );
                        continue;
                    };
                    let now = Instant::now();
                    self.last_frame = Some(frame.clone());
                    let out_report = self.encode_frame(&frame, now, false, *seq);

                    if let Some(repeat) = self.repeat_input.as_mut() {
                        // In repeat mode, physical BT frames update the latest target report only.
                        // The repeat scheduler is the sole UHID input sender, so the configured
                        // rate is an output cadence instead of "physical rate + repeat rate".
                        repeat.store_source(&out_report, now);
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
}

fn one_shot_delay(deadline: Instant, now: Instant) -> Duration {
    let delay = deadline.saturating_duration_since(now);
    if delay.is_zero() {
        Duration::from_nanos(1)
    } else {
        delay
    }
}

fn public_control_error_message(code: &str) -> &'static str {
    match code {
        "load-failed" => "configuration load failed",
        "validation-failed" => "configuration validation failed",
        "mapping-failed" => "configuration mapping failed",
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
        let remap = cfg
            .buttons
            .get(name)
            .and_then(|button| button.remap.as_deref());
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
    use std::os::fd::{AsRawFd, OwnedFd};

    use nix::sys::socket::{recv, send, socketpair, AddressFamily, MsgFlags, SockFlag, SockType};

    use crate::codec::{PhysicalCodec, SourceCodec};
    use crate::mapping::TurboConfig;
    use crate::uhid::UhidEventType;

    // Event handlers share the process-wide disconnect flag, like the single production loop.
    static EVENT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn packet_pair() -> (OwnedFd, OwnedFd) {
        socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        )
        .unwrap()
    }

    fn send_test_packet(fd: &OwnedFd, packet: &[u8]) {
        assert_eq!(
            send(fd.as_raw_fd(), packet, MsgFlags::MSG_NOSIGNAL).unwrap(),
            packet.len()
        );
    }

    fn output_event(data: &[u8]) -> Vec<u8> {
        let mut event = vec![0; 4103];
        event[..4].copy_from_slice(&6u32.to_le_bytes());
        event[4..4 + data.len()].copy_from_slice(data);
        event[4100..4102].copy_from_slice(&(data.len() as u16).to_le_bytes());
        event[4102] = 1;
        event
    }

    fn frame_with(buttons: &[crate::model::Button]) -> ControllerFrame {
        let mut raw = [0u8; 64];
        raw[0] = 0x01;
        raw[8] = 8;
        let mut frame = SourceCodec::Ds5Usb.decode_input(&raw).unwrap();
        for button in buttons {
            frame.state.set_button(*button, true);
        }
        frame
    }

    fn test_proxy(mapping: MappingConfig) -> (Proxy, OwnedFd) {
        let hidraw_file = std::fs::File::open("/dev/null").unwrap();
        let hidraw = HidrawDevice::from_test_fd(OwnedFd::from(hidraw_file));
        let (uhid_fd, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let codec = CodecPipeline {
            source: SourceCodec::Ds5Usb,
            physical: PhysicalCodec::Ds5Usb,
            target: TargetCodec::Ds5UsbAuto,
        };
        let proxy = Proxy::new(ProxyInit {
            hidraw,
            uhid: UhidDevice::from_test_fd(uhid_fd),
            keyboard: crate::keyboard::KeyboardDevice::dummy(),
            mapping: Arc::new(RwLock::new(mapping)),
            active_config: None,
            report_cache: FeatureReportCache::new(),
            codec,
            source_kind: SonyDeviceKind::DualSense,
            output_device_config: "auto".to_string(),
        });
        (proxy, receiver)
    }

    fn prime_runtime_and_repeat(proxy: &mut Proxy, frame: &ControllerFrame, start: Instant) {
        proxy.last_frame = Some(frame.clone());
        let report = proxy.encode_frame(frame, start, false, 1);
        let mut repeat = RepeatInput::with_test_interval(Duration::from_millis(1));
        repeat.store_source(&report, start);
        proxy.repeat_input = Some(repeat);
        assert_eq!(proxy.next_timing_deadline(), Some(start));
    }

    fn receive_uhid_packet(receiver: &OwnedFd) -> Option<Vec<u8>> {
        let mut packet = vec![0u8; crate::uhid::UHID_EVENT_SIZE];
        match recv(receiver.as_raw_fd(), &mut packet, MsgFlags::MSG_DONTWAIT) {
            Ok(size) => {
                packet.truncate(size);
                Some(packet)
            }
            Err(nix::errno::Errno::EAGAIN) => None,
            Err(error) => panic!("failed to receive test UHID packet: {error}"),
        }
    }

    fn assert_one_input_packet_then_empty(receiver: &OwnedFd) {
        let packet = receive_uhid_packet(receiver).expect("one UHID input packet");
        assert!(packet.len() >= 6);
        assert_eq!(
            u32::from_le_bytes(packet[0..4].try_into().unwrap()),
            UhidEventType::Input2 as u32
        );
        let report_size = u16::from_le_bytes(packet[4..6].try_into().unwrap()) as usize;
        assert_eq!(packet.len(), 6 + report_size);
        assert!(receive_uhid_packet(receiver).is_none());
    }

    fn turbo_mapping() -> MappingConfig {
        MappingConfig {
            turbo_configs: vec![TurboConfig {
                src: crate::model::Button::Cross,
                interval_ms: 10,
                delay_ms: 0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn untrusted_log_values_escape_control_characters() {
        assert_eq!(
            EscapedLogValue("/tmp/config\nforged\t\"entry\"").to_string(),
            "\"/tmp/config\\nforged\\t\\\"entry\\\"\""
        );
    }

    #[test]
    fn elapsed_one_shot_deadline_arms_with_a_positive_delay() {
        let now = Instant::now();
        assert_eq!(one_shot_delay(now, now), Duration::from_nanos(1));
        assert_eq!(
            one_shot_delay(now + Duration::from_millis(4), now),
            Duration::from_millis(4)
        );
    }

    #[test]
    fn late_runtime_and_repeat_deadlines_send_one_proxy_report_per_turn() {
        let start = Instant::now();
        let (mut proxy, receiver) = test_proxy(turbo_mapping());
        let frame = frame_with(&[crate::model::Button::Cross]);
        prime_runtime_and_repeat(&mut proxy, &frame, start);

        let late = start + Duration::from_secs(2) + Duration::from_millis(3);
        let mut seq = 1;
        proxy.handle_timing_tick(&mut seq, late).unwrap();
        assert!(proxy
            .runtimes
            .next_deadline()
            .is_some_and(|deadline| deadline > late));
        assert!(proxy
            .repeat_input
            .as_ref()
            .and_then(RepeatInput::next_deadline)
            .is_some_and(|deadline| deadline > late));

        // A duplicate readiness observation in the same turn cannot emit again.
        proxy.handle_timing_tick(&mut seq, late).unwrap();
        assert_one_input_packet_then_empty(&receiver);
    }

    #[test]
    fn config_switch_discards_old_runtime_and_repeat_emissions() {
        let start = Instant::now();
        let (mut proxy, receiver) = test_proxy(turbo_mapping());
        let frame = frame_with(&[crate::model::Button::Cross]);
        prime_runtime_and_repeat(&mut proxy, &frame, start);

        let replacement = ActiveConfig::from_content(
            "replacement.toml".to_string(),
            crate::config::default_content().to_string(),
        )
        .unwrap();
        proxy.apply_active_config(replacement).unwrap();

        assert!(proxy.last_frame.is_none());
        assert_eq!(proxy.next_timing_deadline(), None);
        let mut seq = 1;
        proxy
            .handle_timing_tick(&mut seq, start + Duration::from_secs(2))
            .unwrap();
        assert!(receive_uhid_packet(&receiver).is_none());
    }

    #[test]
    fn rejected_config_preserves_live_mapping_runtimes_keyboard_and_repeat() {
        use crate::model::Button;

        let original = ActiveConfig::from_content(
            "original.toml".into(),
            r#"
version = 2
[cross]
remap = "circle"
turbo = true
turbo_interval_ms = 10
[square]
remap = "held"
[macros.held]
mode = "hold"
sequence = [{ key = "key:space", press_ms = 0, release_ms = 50 }]
"#
            .into(),
        )
        .unwrap();
        for (content, code) in [
            ("not valid TOML", "load-failed"),
            (
                "version = 2\noutput_device = \"invalid\"",
                "validation-failed",
            ),
        ] {
            let (mut proxy, receiver) = test_proxy(MappingConfig::default());
            proxy.apply_active_config(original.clone()).unwrap();
            let start = Instant::now();
            let frame = frame_with(&[Button::Cross, Button::Square]);
            prime_runtime_and_repeat(&mut proxy, &frame, start);
            assert_eq!(proxy.keyboard.recorded_key_events(), [(57, true)]);
            let deadline = proxy.next_timing_deadline();

            let invalid =
                ActiveConfig::from_content("invalid.toml".into(), content.into()).unwrap();
            assert_eq!(proxy.apply_active_config(invalid).unwrap_err().0, code);
            assert_eq!(proxy.active_config(), Some(&original));
            assert_eq!(proxy.output_device_config, "auto");
            assert!(!proxy.recreate_uhid);
            assert!(proxy.last_frame.is_some());
            assert!(proxy.last_snapshot.as_ref().unwrap().button(Button::Cross));
            assert!(proxy.last_output.as_ref().unwrap().button(Button::Circle));
            assert_eq!(proxy.next_timing_deadline(), deadline);
            assert!(proxy.runtimes.turbo[0].active);
            assert!(proxy.runtimes.macros[0].active);
            assert_eq!(proxy.keyboard.recorded_key_events(), [(57, true)]);

            // Cached reports and subsequent timer transforms must still use the old config.
            let mut seq = 1;
            proxy.handle_timing_tick(&mut seq, start).unwrap();
            let packet = receive_uhid_packet(&receiver).unwrap();
            assert_eq!(packet[6 + 8] & 0x60, 0x40); // Circle, not Cross.
            proxy
                .handle_timing_tick(&mut seq, start + Duration::from_millis(10))
                .unwrap();
            let packet = receive_uhid_packet(&receiver).unwrap();
            assert_eq!(packet[6 + 8] & 0x60, 0); // Old turbo reaches its off phase.
            proxy
                .handle_timing_tick(&mut seq, start + Duration::from_millis(20))
                .unwrap();
            let packet = receive_uhid_packet(&receiver).unwrap();
            assert_eq!(packet[6 + 8] & 0x60, 0x40); // The next on phase still applies the old remap.
            assert_eq!(proxy.keyboard.recorded_key_events(), [(57, true)]);
        }
    }

    #[test]
    fn output_target_change_requests_recreation_and_retains_config_content() {
        for output in ["auto", "dualsense", "dualshock4"] {
            let (mut proxy, _) = test_proxy(MappingConfig::default());
            proxy.keyboard.press(57);
            let config = ActiveConfig::from_content(
                "/nonexistent/retained.toml".into(),
                format!("version = 2\noutput_device = \"{output}\"\n[cross]\nremap = \"circle\"\n"),
            )
            .unwrap();
            proxy.apply_active_config(config.clone()).unwrap();
            assert_eq!(proxy.recreate_uhid, output != "auto");
            assert_eq!(proxy.output_device_config, output);
            assert_eq!(proxy.active_config(), Some(&config));
            assert_eq!(
                proxy.keyboard.recorded_key_events(),
                [(57, true), (57, false)]
            );
            let frame = frame_with(&[crate::model::Button::Cross]);
            let report = proxy.encode_frame(&frame, Instant::now(), false, 1);
            assert_eq!(report[8] & 0x60, 0x40);
            // Once a recreation request has been consumed, reapplying the same target
            // must not request another recreation.
            proxy.recreate_uhid = false;
            proxy.apply_active_config(config).unwrap();
            assert!(!proxy.recreate_uhid);
        }
    }

    #[test]
    fn control_apply_reports_failure_without_commit_and_acknowledges_success() {
        let dir = std::env::temp_dir().join(format!("proxy-apply-{}", std::process::id()));
        let initial = crate::control::ControlState {
            uhid_ready: true,
            needs_config: true,
            bt_haptics: None,
        };
        let mut server = ControlServer::bind(&dir, initial).unwrap();
        let client = crate::control::ControlClient::connect(&dir.join("control.sock")).unwrap();
        assert!(server.drain_requests().unwrap().is_empty());
        assert_eq!(
            client.receive().unwrap(),
            Some(crate::control::ServerPacket::Hello(initial))
        );
        let (mut proxy, _) = test_proxy(MappingConfig::default());
        let old = ActiveConfig::from_content("old.toml".into(), "version = 2\n".into()).unwrap();
        proxy.apply_active_config(old.clone()).unwrap();

        let bad = ActiveConfig::from_content(
            "private-source.toml".into(),
            "version = 2\noutput_device = \"private-invalid-target\"".into(),
        )
        .unwrap();
        client
            .send_request(&ControlRequest::SwitchConfig(bad))
            .unwrap();
        proxy.handle_control_requests(&mut server).unwrap();
        assert_eq!(
            client.receive().unwrap(),
            Some(crate::control::ServerPacket::Error {
                code: "validation-failed".into(),
                message: "configuration validation failed".into(),
            })
        );
        assert_eq!(client.receive().unwrap(), None); // No ACK or success state after rejection.
        assert_eq!(server.state(), initial);
        assert_eq!(proxy.active_config(), Some(&old));

        let next = ActiveConfig::from_content(
            "/nonexistent/profile.toml".into(),
            "version = 2\noutput_device = \"dualshock4\"".into(),
        )
        .unwrap();
        client
            .send_request(&ControlRequest::SwitchConfig(next.clone()))
            .unwrap();
        proxy.handle_control_requests(&mut server).unwrap();
        assert_eq!(
            client.receive().unwrap(),
            Some(crate::control::ServerPacket::OkSwitchConfig)
        );
        assert_eq!(
            client.receive().unwrap(),
            Some(crate::control::ServerPacket::State(
                crate::control::ControlState {
                    uhid_ready: true,
                    needs_config: false,
                    bt_haptics: None,
                }
            ))
        );
        assert_eq!(proxy.active_config(), Some(&next));
        assert!(proxy.recreate_uhid);
        drop(client);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_handler_drops_bad_frames_then_forwards_valid_input_and_detects_eof() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, receiver) = test_proxy(MappingConfig::default());
        let (physical, sender) = packet_pair();
        proxy.hidraw = HidrawDevice::from_test_fd(physical);
        send_test_packet(&sender, &[1, 0]);
        send_test_packet(&sender, &[0xff; 64]);
        let report = proxy
            .codec
            .target
            .encode_input(&frame_with(&[crate::model::Button::Cross]), 0)
            .unwrap();
        send_test_packet(&sender, &report);
        let mut seq = 0;
        proxy.handle_hidraw_input(&mut seq).unwrap();
        let packet = receive_uhid_packet(&receiver).unwrap();
        assert_eq!(&packet[..6], &[12, 0, 0, 0, 64, 0]);
        assert_eq!(packet[6 + 8] & 0x20, 0x20);
        assert!(receive_uhid_packet(&receiver).is_none());
        assert!(!DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst));
        drop(sender);
        proxy.handle_hidraw_input(&mut seq).unwrap();
        assert!(DISCONNECTED.swap(false, std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn audio_identity_tracks_dualsense_target_and_preserves_native_audio_for_ds4() {
        use crate::control::HapticsDevice;
        let (mut proxy, _peer) = test_proxy(MappingConfig::default());
        for (source, target, expected) in [
            (
                SonyDeviceKind::DualSense,
                TargetCodec::Ds5UsbAuto,
                HapticsDevice::DualSense,
            ),
            (
                SonyDeviceKind::DualSenseEdge,
                TargetCodec::Ds5UsbAuto,
                HapticsDevice::DualSenseEdge,
            ),
            (
                SonyDeviceKind::DualSenseEdge,
                TargetCodec::Ds5UsbForced,
                HapticsDevice::DualSense,
            ),
            (
                SonyDeviceKind::DualSense,
                TargetCodec::Ds5UsbForced,
                HapticsDevice::DualSense,
            ),
            (
                SonyDeviceKind::DualSenseEdge,
                TargetCodec::Ds4Usb,
                HapticsDevice::DualSenseEdge,
            ),
            (
                SonyDeviceKind::DualSense,
                TargetCodec::Ds4Usb,
                HapticsDevice::DualSense,
            ),
        ] {
            proxy.source_kind = source;
            proxy.codec.target = target;
            assert_eq!(proxy.haptics_device(), expected);
        }
    }

    #[test]
    fn speaker_demo_shares_pcm_timer_and_mutes_on_underrun_or_teardown() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, _uhid_peer) = test_proxy(MappingConfig::default());
        let (physical, peer) = packet_pair();
        proxy.hidraw = HidrawDevice::from_test_fd(physical);
        proxy.codec.physical = PhysicalCodec::Ds5Bt;
        let mut sequence = 0;
        for teardown in [false, true] {
            proxy.live_haptics.push(
                crate::control::haptics::AudioFrame {
                    haptics: [12; 64],
                    speaker: Some([0x5a; 200]),
                },
                Instant::now(),
            );
            let deadline = proxy.next_timing_deadline().unwrap();
            proxy.handle_timing_tick(&mut 0, deadline).unwrap();
            for (id, length) in [(0x31, 78), (0x36, 398)] {
                let report = receive_uhid_packet(&peer).unwrap();
                assert_eq!(report.len(), length);
                assert_eq!(&report[..2], &[id, sequence << 4]);
                sequence += 1;
                if id == 0x31 {
                    assert_eq!(&report[3..5], &[0xa1, 0x82]);
                    assert_eq!(report[8], 100);
                    assert_eq!(report[10], 0x09);
                    assert_eq!(report[12], 0x10);
                    assert_eq!(report[40], 0x0a);
                    // No trigger, LED, microphone volume or rumble values are set.
                    for (index, value) in report[3..74].iter().enumerate() {
                        if ![3, 4, 8, 10, 12, 40].contains(&(index + 3)) {
                            assert_eq!(*value, 0);
                        }
                    }
                }
            }
            assert!(receive_uhid_packet(&peer).is_none());
            assert!(proxy.speaker_active);
            if teardown {
                proxy.clear_timing_state();
            } else {
                proxy
                    .handle_timing_tick(&mut 0, proxy.next_timing_deadline().unwrap())
                    .unwrap();
            }
            let mute = receive_uhid_packet(&peer).unwrap();
            assert_eq!(&mute[..4], &[0x31, sequence << 4, 0x10, 0x20]);
            sequence += 1;
            assert!(mute[4..74].iter().all(|&b| b == 0));
            let silence = receive_uhid_packet(&peer).unwrap();
            assert_eq!(&silence[..2], &[0x32, sequence << 4]);
            sequence += 1;
            assert!(silence[13..77].iter().all(|&b| b == 0));
            assert!(!proxy.speaker_active);
            assert!(proxy.next_timing_deadline().is_none());
            assert!(receive_uhid_packet(&peer).is_none());
        }
    }

    #[test]
    fn live_pcm_uses_timer_without_mode_output_and_stops_on_teardown() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, _uhid_peer) = test_proxy(MappingConfig::default());
        let (physical, peer) = packet_pair();
        proxy.hidraw = HidrawDevice::from_test_fd(physical);
        proxy.codec.physical = PhysicalCodec::Ds5Bt;
        let now = Instant::now();
        let samples = std::array::from_fn(|i| i as i8 - 32);
        proxy.live_haptics.push(
            crate::control::haptics::AudioFrame {
                haptics: samples,
                speaker: None,
            },
            now,
        );
        assert!(receive_uhid_packet(&peer).is_none());
        let deadline = proxy.next_timing_deadline().unwrap();
        proxy.handle_timing_tick(&mut 0, deadline).unwrap();
        let report = receive_uhid_packet(&peer).unwrap();
        assert_eq!(report.len(), 142);
        assert_eq!(report[0], 0x32);
        assert_eq!(&report[13..77], samples.map(|s| s as u8));
        assert!(receive_uhid_packet(&peer).is_none());
        proxy.clear_timing_state();
        let stop = receive_uhid_packet(&peer).unwrap();
        assert_eq!(stop[0], 0x32);
        assert!(stop[13..77].iter().all(|b| *b == 0));
        assert!(proxy.next_timing_deadline().is_none());
    }

    #[test]
    fn live_haptics_write_failure_preserves_input_path() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, uhid_peer) = test_proxy(MappingConfig::default());
        let (physical, peer) = packet_pair();
        proxy.hidraw = HidrawDevice::from_test_fd(physical);
        proxy.codec.physical = PhysicalCodec::Ds5Bt;
        let start = Instant::now();
        proxy.live_haptics.push(
            crate::control::haptics::AudioFrame {
                haptics: [12; 64],
                speaker: None,
            },
            start,
        );
        let deadline = proxy.next_timing_deadline().unwrap();
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let physical =
            std::mem::replace(&mut proxy.hidraw, HidrawDevice::from_test_fd(full.into()));
        proxy.handle_timing_tick(&mut 0, deadline).unwrap();
        assert!(proxy.live_haptics.next_deadline().is_none());
        assert!(!DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst));
        proxy.hidraw = physical;
        let input = proxy
            .codec
            .target
            .encode_input(&frame_with(&[]), 0)
            .unwrap();
        send_test_packet(&peer, &input);
        proxy.handle_hidraw_input(&mut 0).unwrap();
        assert_one_input_packet_then_empty(&uhid_peer);
    }

    #[test]
    fn output_handler_drops_invalid_reports_and_keeps_running_after_write_failure() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, receiver) = test_proxy(MappingConfig::default());
        let (physical, physical_peer) = packet_pair();
        proxy.hidraw = HidrawDevice::from_test_fd(physical);
        proxy.codec.physical = PhysicalCodec::Ds5Bt;
        let mut valid = vec![0; 48];
        valid[0] = 2;
        valid[3] = 67;
        send_test_packet(&receiver, &output_event(&[2])); // Too short for BT output.
        send_test_packet(&receiver, &output_event(&valid));
        proxy.handle_uhid_event().unwrap();
        let output = receive_uhid_packet(&physical_peer).unwrap();
        assert_eq!(&output[..3], &[0x31, 0, 0x10]);
        assert_eq!(&output[3..50], &valid[1..]);
        assert!(receive_uhid_packet(&physical_peer).is_none());

        // ENOSPC is an output failure, not a device-disconnect error.
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let original =
            std::mem::replace(&mut proxy.hidraw, HidrawDevice::from_test_fd(full.into()));
        send_test_packet(&receiver, &output_event(&valid));
        proxy.handle_uhid_event().unwrap();
        assert!(!DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst));
        proxy.hidraw = original;
        let input = proxy
            .codec
            .target
            .encode_input(&frame_with(&[]), 0)
            .unwrap();
        send_test_packet(&physical_peer, &input);
        proxy.handle_hidraw_input(&mut 0).unwrap();
        assert_one_input_packet_then_empty(&receiver);
    }

    #[test]
    fn feature_handler_replies_with_cache_fallback_missing_and_forwarding_error() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        DISCONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let (mut proxy, receiver) = test_proxy(MappingConfig::default());
        let fallback = proxy.codec.target.fallback_feature_report(0x09).unwrap();
        proxy.report_cache.insert(0x09, vec![9, 42, 43]);
        for (id, report, expected, error) in
            [(1u32, 0x09, vec![9, 42, 43], 0u16), (2, 0xff, vec![], 1)]
        {
            let mut event = vec![9, 0, 0, 0];
            event.extend_from_slice(&id.to_le_bytes());
            event.extend_from_slice(&[report, 0]);
            send_test_packet(&receiver, &event);
            proxy.handle_uhid_event().unwrap();
            let reply = receive_uhid_packet(&receiver).unwrap();
            assert_eq!(&reply[..4], &[10, 0, 0, 0]);
            assert_eq!(&reply[4..8], &id.to_le_bytes());
            assert_eq!(&reply[8..10], &error.to_le_bytes());
            assert_eq!(&reply[10..12], &(expected.len() as u16).to_le_bytes());
            assert_eq!(&reply[12..], expected);
        }
        proxy.report_cache = FeatureReportCache::new();
        send_test_packet(&receiver, &[9, 0, 0, 0, 3, 0, 0, 0, 9, 0]);
        proxy.handle_uhid_event().unwrap();
        let reply = receive_uhid_packet(&receiver).unwrap();
        assert_eq!(&reply[4..10], &[3, 0, 0, 0, 0, 0]);
        assert_eq!(&reply[12..], fallback);

        // /dev/null cannot serve HIDIOCSFEATURE; the error must be acknowledged.
        send_test_packet(&receiver, &[13, 0, 0, 0, 4, 0, 0, 0, 8, 0, 1, 0, 8]);
        proxy.handle_uhid_event().unwrap();
        assert_eq!(
            receive_uhid_packet(&receiver).unwrap(),
            [14, 0, 0, 0, 4, 0, 0, 0, 1, 0]
        );
        assert!(!DISCONNECTED.load(std::sync::atomic::Ordering::SeqCst));
        // A subsequent request is still served after the failed feature operation.
        send_test_packet(&receiver, &[9, 0, 0, 0, 5, 0, 0, 0, 9, 0]);
        proxy.handle_uhid_event().unwrap();
        assert_eq!(&receive_uhid_packet(&receiver).unwrap()[12..], fallback);
    }

    #[test]
    fn malformed_uhid_event_and_kernel_stop_are_fatal_handler_errors() {
        let _guard = EVENT_TEST_LOCK.lock().unwrap();
        let (mut proxy, receiver) = test_proxy(MappingConfig::default());
        send_test_packet(&receiver, &[6, 0, 0, 0]);
        assert_eq!(
            proxy.handle_uhid_event().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        send_test_packet(&receiver, &[3, 0, 0, 0]);
        assert_eq!(
            proxy.handle_uhid_event().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn disconnect_or_shutdown_cleanup_discards_old_timing_emissions() {
        let start = Instant::now();
        let (mut proxy, receiver) = test_proxy(turbo_mapping());
        let frame = frame_with(&[crate::model::Button::Cross]);
        prime_runtime_and_repeat(&mut proxy, &frame, start);

        // run() calls this common cleanup for every exit reason, including
        // physical disconnect and user shutdown.
        proxy.clear_timing_state();

        assert!(proxy.last_frame.is_none());
        assert_eq!(proxy.next_timing_deadline(), None);
        let mut seq = 1;
        proxy
            .handle_timing_tick(&mut seq, start + Duration::from_secs(2))
            .unwrap();
        assert!(receive_uhid_packet(&receiver).is_none());
    }

    #[test]
    fn repeat_mode_validation_accepts_only_named_modes() {
        assert!(matches!(
            parse_repeat_mode("passthrough"),
            Ok(RepeatMode::Passthrough)
        ));
        assert!(matches!(
            parse_repeat_mode("seq_only"),
            Ok(RepeatMode::SeqOnly)
        ));
        assert!(matches!(
            parse_repeat_mode("seq_ts"),
            Ok(RepeatMode::SeqAndTimestamp)
        ));
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
                format!("invalid TEST_REPEAT_HZ={value}; expected integer 1..=2000")
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
