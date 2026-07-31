use std::env;
use std::time::{Duration, Instant};

use log::debug;

use crate::codec::{CodecPipeline, SourceCodec, TargetCodec};

pub(super) struct RepeatInput {
    interval: Duration,
    timestamp_delta: u32,
    mode: RepeatMode,
    target: RepeatTarget,
    next_tick: Option<Instant>,
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
            next_tick: None,
            last_report: None,
        })
    }

    #[cfg(test)]
    pub(super) fn with_test_interval(interval: Duration) -> Self {
        assert!(!interval.is_zero());
        Self {
            interval,
            timestamp_delta: 1,
            mode: RepeatMode::SeqOnly,
            target: RepeatTarget::Ds5Usb,
            next_tick: None,
            last_report: None,
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.next_tick
    }

    pub(super) fn store_source(&mut self, report: &[u8], now: Instant) {
        if self.last_report.is_none() || self.next_tick.is_none() {
            self.next_tick = Some(now);
        }
        self.last_report = Some(report.to_vec());
    }

    pub(super) fn store_runtime(&mut self, report: &[u8]) {
        let mut updated = report.to_vec();
        let previous = self
            .last_report
            .as_deref()
            .expect("runtime repeat refresh requires a source report");
        preserve_repeat_fields(previous, &mut updated, self.mode, self.target);
        self.last_report = Some(updated);
    }

    pub(super) fn prepare_report(&mut self, now: Instant, seq: &mut u8) -> Option<&[u8]> {
        let deadline = self.next_tick?;
        if now < deadline {
            return None;
        }
        self.next_tick = first_future_repeat_deadline(deadline, now, self.interval);
        let report = self.last_report.as_mut()?;
        advance_repeat_report(report, seq, self.timestamp_delta, self.mode, self.target);
        Some(report)
    }

    pub(super) fn clear(&mut self) {
        self.next_tick = None;
        self.last_report = None;
    }
}

fn first_future_repeat_deadline(
    deadline: Instant,
    now: Instant,
    interval: Duration,
) -> Option<Instant> {
    debug_assert!(!interval.is_zero());
    let overdue = now.saturating_duration_since(deadline);
    let interval_ns = interval.as_nanos();
    let remainder_ns = overdue.as_nanos() % interval_ns;
    let until_next_ns = if remainder_ns == 0 {
        interval_ns
    } else {
        interval_ns - remainder_ns
    };
    let until_next_ns = u64::try_from(until_next_ns).ok()?;
    now.checked_add(Duration::from_nanos(until_next_ns))
}

fn preserve_repeat_fields(
    previous: &[u8],
    updated: &mut [u8],
    mode: RepeatMode,
    target: RepeatTarget,
) {
    match target {
        RepeatTarget::Ds5Usb if previous.len() >= 32 && updated.len() >= 32 => {
            updated[7] = previous[7];
            if matches!(mode, RepeatMode::SeqAndTimestamp) {
                updated[28..32].copy_from_slice(&previous[28..32]);
            }
        }
        RepeatTarget::Ds4Usb if previous.len() >= 35 && updated.len() >= 35 => {
            updated[7] = (updated[7] & 0x03) | (previous[7] & 0xfc);
            updated[10..12].copy_from_slice(&previous[10..12]);
            updated[34] = previous[34];
        }
        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn repeat_with_interval(interval: Duration) -> RepeatInput {
        RepeatInput::with_test_interval(interval)
    }

    #[test]
    fn late_repeat_emits_once_and_rebases_to_a_future_deadline() {
        let start = Instant::now();
        for interval in [Duration::from_millis(1), Duration::from_millis(4)] {
            let mut repeat = repeat_with_interval(interval);
            let mut seq = 0;
            repeat.store_source(&[0u8; 64], start);
            assert_eq!(repeat.next_deadline(), Some(start));

            let late = start + Duration::from_secs(2);
            let report = repeat.prepare_report(late, &mut seq).unwrap();
            assert_eq!(report[7], 1);
            assert_eq!(seq, 1);
            assert_eq!(repeat.next_deadline(), Some(late + interval));
            assert!(repeat.prepare_report(late, &mut seq).is_none());
            assert_eq!(seq, 1);
        }
    }

    #[test]
    fn irregular_source_updates_do_not_create_repeat_catch_up() {
        let start = Instant::now();
        let interval = Duration::from_millis(4);
        let mut repeat = repeat_with_interval(interval);
        let mut seq = 10;

        repeat.store_source(&[0u8; 64], start);
        repeat.prepare_report(start, &mut seq).unwrap();
        repeat.store_source(&[0u8; 64], start + Duration::from_millis(1));
        repeat.store_source(&[0u8; 64], start + Duration::from_millis(3));

        let late = start + Duration::from_millis(101);
        repeat.prepare_report(late, &mut seq).unwrap();
        assert_eq!(seq, 12);
        assert_eq!(
            repeat.next_deadline(),
            Some(start + Duration::from_millis(104))
        );
    }

    #[test]
    fn runtime_refresh_waits_for_the_existing_repeat_deadline() {
        let start = Instant::now();
        let interval = Duration::from_millis(4);
        let mut repeat = repeat_with_interval(interval);
        let mut seq = 0;

        repeat.store_source(&[0u8; 64], start);
        repeat.prepare_report(start, &mut seq).unwrap();
        let early = start + Duration::from_millis(1);
        repeat.store_runtime(&[0u8; 64]);
        assert!(repeat.prepare_report(early, &mut seq).is_none());
        assert_eq!(repeat.next_deadline(), Some(start + interval));

        repeat.clear();
        assert_eq!(repeat.next_deadline(), None);
        assert!(repeat.prepare_report(early, &mut seq).is_none());
    }

    #[test]
    fn runtime_refresh_preserves_ds5_repeat_sequence_and_timestamp() {
        let start = Instant::now();
        let mut repeat = repeat_with_interval(Duration::from_millis(1));
        repeat.mode = RepeatMode::SeqAndTimestamp;
        let mut source = [0u8; 64];
        source[28..32].copy_from_slice(&100u32.to_le_bytes());
        let mut seq = 0;

        repeat.store_source(&source, start);
        repeat.prepare_report(start, &mut seq).unwrap();
        let mut runtime = [0u8; 64];
        runtime[7] = 99;
        runtime[28..32].copy_from_slice(&100u32.to_le_bytes());
        repeat.store_runtime(&runtime);

        let stored = repeat.last_report.as_deref().unwrap();
        assert_eq!(stored[7], 1);
        assert_eq!(u32::from_le_bytes(stored[28..32].try_into().unwrap()), 101);
    }

    #[test]
    fn runtime_refresh_preserves_ds4_repeat_sequence_fields() {
        let start = Instant::now();
        let mut repeat = repeat_with_interval(Duration::from_millis(4));
        repeat.target = RepeatTarget::Ds4Usb;
        let mut source = [0u8; 64];
        source[7] = 0x02;
        let mut seq = 7;

        repeat.store_source(&source, start);
        repeat.prepare_report(start, &mut seq).unwrap();
        let mut runtime = [0u8; 64];
        runtime[7] = 0x01;
        repeat.store_runtime(&runtime);

        let stored = repeat.last_report.as_deref().unwrap();
        assert_eq!(stored[7] & 0x03, 0x01);
        assert_eq!(stored[7] >> 2, 8);
        assert_eq!(&stored[10..12], &8u16.to_le_bytes());
        assert_eq!(stored[34], 8);
    }
}
