use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use log::{debug, error, info, log_enabled, Level};

use crate::codec::{Ds5ControlKind, Ds5UsbOutput, HapticsFrame, OutputCommand, PhysicalCodec};

use super::{is_disconnect_io_error, Proxy, DISCONNECTED};

struct SentControl {
    sent_at: Instant,
    report: Ds5UsbOutput,
}

impl SentControl {
    fn is_repeat(&self, output: &Ds5UsbOutput, now: Instant) -> bool {
        // Refresh on the next request after 100 ms in case another writer intervened.
        self.report == *output && now.duration_since(self.sent_at) < Duration::from_millis(100)
    }
}

#[derive(Default)]
pub(super) struct SentControls {
    lightbar: Option<SentControl>,
    triggers_off: Option<SentControl>,
}

impl SentControls {
    fn slot(&mut self, kind: Ds5ControlKind) -> Option<(&mut Option<SentControl>, SendOutcome)> {
        match kind {
            Ds5ControlKind::Lightbar => Some((&mut self.lightbar, SendOutcome::SkippedLightbar)),
            Ds5ControlKind::BothTriggersOff => {
                Some((&mut self.triggers_off, SendOutcome::SkippedTriggersOff))
            }
            Ds5ControlKind::Empty => None,
            Ds5ControlKind::Other => {
                *self = Self::default();
                None
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SendOutcome {
    Written,
    SkippedLightbar,
    SkippedTriggersOff,
}

#[derive(Default)]
pub(super) struct BtOutputStats {
    started: Option<Instant>,
    written: u64,
    skipped_lightbar: u64,
    skipped_triggers_off: u64,
    failed: u64,
}

impl BtOutputStats {
    fn record(&mut self, result: &io::Result<SendOutcome>, now: Instant) {
        match result {
            Ok(SendOutcome::Written) => self.written += 1,
            Ok(SendOutcome::SkippedLightbar) => self.skipped_lightbar += 1,
            Ok(SendOutcome::SkippedTriggersOff) => self.skipped_triggers_off += 1,
            Err(_) => self.failed += 1,
        }
        let elapsed = now.duration_since(*self.started.get_or_insert(now));
        if elapsed >= Duration::from_secs(5) {
            let received =
                self.written + self.skipped_lightbar + self.skipped_triggers_off + self.failed;
            debug!(
                "BT control output: elapsed_ms={}, received={received}, written={}, skipped_lightbar={}, skipped_triggers_off={}, failed={}",
                elapsed.as_millis(), self.written, self.skipped_lightbar, self.skipped_triggers_off, self.failed
            );
            *self = Self::default();
        }
    }
}

fn output_codec_error(error: crate::codec::CodecError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("output codec error: {error:?}"),
    )
}

impl Proxy {
    pub(super) fn send_target_output(&mut self, data: &[u8]) -> io::Result<()> {
        let result = self
            .codec
            .target
            .decode_output(data)
            .map_err(output_codec_error)
            .and_then(|command| self.send_output_command(&command));
        if self.codec.physical == PhysicalCodec::Ds5Bt && log_enabled!(Level::Debug) {
            self.bt_output_stats.record(&result, Instant::now());
        }
        result.map(|_| ())
    }

    fn send_output_command(&mut self, command: &OutputCommand) -> io::Result<SendOutcome> {
        let tracked = match (self.codec.physical, command) {
            (PhysicalCodec::Ds5Bt, OutputCommand::Ds5Usb(output)) => {
                let kind = output.control_kind().map_err(output_codec_error)?;
                self.sent_controls
                    .slot(kind)
                    .map(|(slot, skipped)| (slot, output, skipped))
            }
            _ => None,
        };
        if let Some((Some(last), output, skipped)) = &tracked {
            if last.is_repeat(output, Instant::now()) {
                return Ok(*skipped);
            }
        }
        let report = self
            .codec
            .physical
            .encode_output(command, &mut self.physical_output_state)
            .map_err(output_codec_error)?;
        if let Err(error) = self.hidraw.write_output(&report) {
            self.sent_controls = SentControls::default();
            if is_disconnect_io_error(&error) {
                DISCONNECTED.store(true, Ordering::SeqCst);
            }
            return Err(error);
        }
        if let Some((slot, output, _)) = tracked {
            *slot = Some(SentControl {
                sent_at: Instant::now(),
                report: output.clone(),
            });
        }
        Ok(SendOutcome::Written)
    }

    fn send_live_audio(&mut self, frame: crate::control::haptics::AudioFrame) -> io::Result<()> {
        let speaker_active = frame.speaker.is_some();
        if speaker_active != self.speaker_active {
            self.send_output_command(&OutputCommand::Ds5Usb(Ds5UsbOutput::speaker_demo_mode(
                speaker_active,
            )))?;
            self.speaker_active = speaker_active;
            info!(
                "Bluetooth speaker demo {}",
                if speaker_active { "started" } else { "stopped" }
            );
        }
        if let Some(speaker) = frame.speaker {
            self.send_output_command(&OutputCommand::Audio {
                haptics: HapticsFrame(frame.haptics),
                speaker,
            })?;
        } else {
            self.send_output_command(&OutputCommand::Haptics(HapticsFrame(frame.haptics)))?;
        }
        Ok(())
    }

    fn stop_speaker(&mut self) {
        if self.speaker_active {
            self.speaker_active = false;
            if !DISCONNECTED.load(Ordering::SeqCst) {
                if let Err(error) = self.send_output_command(&OutputCommand::Ds5Usb(
                    Ds5UsbOutput::speaker_demo_mode(false),
                )) {
                    error!("failed to mute speaker: {error}");
                }
            }
        }
    }

    pub(super) fn handle_haptics_tick(&mut self, now: Instant) {
        if let Some(frame) = self.live_haptics.take_due_frame(now) {
            if let Err(error) = self.send_live_audio(frame) {
                self.stop_speaker();
                self.live_haptics = super::LiveHaptics::default();
                error!("live haptics stopped after output failure: {error}");
            }
        }
    }

    pub(super) fn stop_live_haptics(&mut self) {
        self.stop_speaker();
        let active = self.live_haptics.next_deadline().is_some();
        self.live_haptics = super::LiveHaptics::default();
        if active && !DISCONNECTED.load(Ordering::SeqCst) {
            if let Err(error) =
                self.send_output_command(&OutputCommand::Haptics(HapticsFrame::SILENCE))
            {
                error!("failed to send haptics silence: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt_output_counts_each_outcome_and_starts_a_fresh_window() {
        let mut stats = BtOutputStats::default();
        let start = Instant::now();
        for outcome in [
            Ok(SendOutcome::Written),
            Ok(SendOutcome::SkippedLightbar),
            Ok(SendOutcome::SkippedTriggersOff),
            Err(io::Error::from(io::ErrorKind::InvalidData)),
        ] {
            stats.record(&outcome, start);
        }
        stats.record(
            &Ok(SendOutcome::Written),
            start + Duration::from_millis(4999),
        );
        assert_eq!(
            (
                stats.written,
                stats.skipped_lightbar,
                stats.skipped_triggers_off,
                stats.failed
            ),
            (2, 1, 1, 1)
        );
        stats.record(&Ok(SendOutcome::Written), start + Duration::from_secs(5));
        assert!(stats.started.is_none());
        let resumed = start + Duration::from_secs(60);
        stats.record(&Ok(SendOutcome::Written), resumed);
        assert_eq!(stats.started, Some(resumed));
        assert_eq!(
            (
                stats.written,
                stats.skipped_lightbar,
                stats.skipped_triggers_off,
                stats.failed
            ),
            (1, 0, 0, 0)
        );
    }

    #[test]
    fn repeated_report_refreshes_at_100_ms() {
        let now = Instant::now();
        let report = Ds5UsbOutput::speaker_demo_mode(false);
        let last = SentControl {
            sent_at: now,
            report: report.clone(),
        };
        assert!(last.is_repeat(&report, now + Duration::from_millis(99)));
        assert!(!last.is_repeat(&report, now + Duration::from_millis(100)));
    }
}
