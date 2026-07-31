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

mod pipeline;
mod repeat;
mod runtime;
mod uhid_events;

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
            physical_output_state: PhysicalOutputState::default(),
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

    fn next_timing_deadline(&self) -> Option<Instant> {
        self.runtimes
            .next_deadline()
            .into_iter()
            .chain(
                self.repeat_input
                    .as_ref()
                    .and_then(RepeatInput::next_deadline),
            )
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

        let mut control_state = control.state();
        control_state.uhid_ready = true;
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
            }
            if self.recreate_uhid {
                break 'run ExitReason::ConfigChanged;
            }
            if let Err(error) = self.arm_timing_timer(&timing_timer) {
                error!("{error}");
                break 'run ExitReason::FatalError;
            }
        };

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
                    error!("control request failed; previous config retained: code={code}");
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

    use nix::sys::socket::{recv, socketpair, AddressFamily, MsgFlags, SockFlag, SockType};

    use crate::codec::{PhysicalCodec, SourceCodec};
    use crate::mapping::TurboConfig;
    use crate::uhid::UhidEventType;

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
