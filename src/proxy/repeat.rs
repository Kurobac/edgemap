use std::env;
use std::io;
use std::time::{Duration, Instant};

use log::debug;

use crate::codec::{CodecPipeline, SourceCodec, TargetCodec};
use crate::uhid::UhidDevice;

pub(super) struct RepeatInput {
    interval: Duration,
    timestamp_delta: u32,
    mode: RepeatMode,
    target: RepeatTarget,
    next_tick: Instant,
    last_report: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RepeatMode {
    Passthrough,
    SeqOnly,
    SeqAndTimestamp,
}

#[derive(Clone, Copy)]
pub(super) enum RepeatTarget {
    Ds5Usb,
    Ds4Usb,
}

impl RepeatInput {
    pub(super) fn from_env(codec: CodecPipeline) -> Option<Self> {
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

    pub(super) fn timeout_ms(&self) -> u16 {
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

    pub(super) fn store(&mut self, report: &[u8]) {
        self.last_report = Some(report.to_vec());
    }

    pub(super) fn send_due(&mut self, uhid: &UhidDevice, seq: &mut u8) -> io::Result<()> {
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

pub(super) fn parse_repeat_mode(value: &str) -> Result<RepeatMode, String> {
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

pub(super) fn parse_repeat_hz(env_name: &str, value: &str) -> Result<u64, String> {
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

pub(super) fn advance_repeat_report(
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
