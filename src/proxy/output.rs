use std::io;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use log::{debug, error, info};

use crate::codec::{
    Ds5ControlKind, Ds5UsbOutput, HapticsFrame, OutputCommand, PhysicalCodec, TargetCodec,
};

use super::{is_disconnect_io_error, Proxy, DISCONNECTED};

struct SentControl {
    sent_at: Instant,
    report: Vec<u8>,
}

impl SentControl {
    fn is_repeat(&self, output: &[u8], now: Instant) -> bool {
        // Refresh on the next request after 100 ms in case another writer intervened.
        self.report == output && now.duration_since(self.sent_at) < Duration::from_millis(100)
    }
}

#[derive(Default)]
struct SentControls {
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
    SkippedLightbar,
    SkippedTriggersOff,
}

const TOKEN_PERIOD: Duration = Duration::from_nanos(1_000_000_000_u64.div_ceil(30));
const BURST: u32 = 3;

struct PendingControl {
    output: Ds5UsbOutput,
    fields: u64,
    ordering_mask: u64,
    kind: Ds5ControlKind,
    force_audio: bool,
}

pub(super) struct BtControls {
    sent: SentControls,
    pending: Vec<PendingControl>,
    // Time at which all three tokens will have been replenished.
    full_at: Instant,
}

impl Default for BtControls {
    fn default() -> Self {
        Self {
            sent: SentControls::default(),
            pending: Vec::new(),
            full_at: Instant::now(),
        }
    }
}

impl BtControls {
    fn next_token(&self) -> Instant {
        self.full_at - TOKEN_PERIOD * (BURST - 1)
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.pending.first().map(|_| self.next_token())
    }

    fn take_due(&mut self, now: Instant) -> Option<PendingControl> {
        if self.pending.is_empty() || now < self.next_token() {
            return None;
        }
        self.full_at = self.full_at.max(now) + TOKEN_PERIOD;
        Some(self.pending.remove(0))
    }

    fn enqueue(&mut self, control: PendingControl) -> usize {
        // Independent controls keep their place even if both are updated on every game frame.
        if let Some(last) = self
            .pending
            .iter_mut()
            .rfind(|p| p.ordering_mask & control.ordering_mask != 0)
        {
            if last.fields == control.fields {
                *last = control;
                return 1;
            }
        }
        // Keep mixed reports in order. Each survivor supplies at least one field
        // no later report supplies, bounding the queue by the number of valid bits.
        let before = self.pending.len();
        let mut newer = control.fields;
        for i in (0..before).rev() {
            let fields = self.pending[i].fields;
            if fields & !newer == 0 {
                self.pending.remove(i);
            }
            newer |= fields;
        }
        let replaced = before - self.pending.len();
        self.pending.push(control);
        replaced
    }
}

#[derive(Default)]
pub(super) struct BtOutputStats {
    started: Option<Instant>,
    received: u64,
    written: u64,
    coalesced: u64,
    skipped_lightbar: u64,
    skipped_triggers_off: u64,
    failed: u64,
}

impl BtOutputStats {
    fn log_if_due(&mut self, now: Instant) {
        let elapsed = now.duration_since(*self.started.get_or_insert(now));
        if elapsed >= Duration::from_secs(5) {
            debug!(
                "BT control output: elapsed_ms={}, received={}, written={}, coalesced={}, skipped_lightbar={}, skipped_triggers_off={}, failed={}",
                elapsed.as_millis(), self.received, self.written, self.coalesced, self.skipped_lightbar, self.skipped_triggers_off, self.failed
            );
            *self = Self {
                started: Some(now),
                ..Self::default()
            };
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
        if self.codec.physical != PhysicalCodec::Ds5Bt {
            let command = self
                .codec
                .target
                .decode_output(data, self.force_audio_haptics)
                .map_err(output_codec_error)?;
            return self.send_output_command(&command);
        }

        self.submit_bt_control(data, Instant::now())
    }

    pub(super) fn submit_bt_control(&mut self, data: &[u8], now: Instant) -> io::Result<()> {
        self.bt_output_stats.log_if_due(now);
        self.bt_output_stats.received += 1;
        let output = self.codec.target.decode_control(data);
        let (flags, mut kind) = output.control_fields().map_err(|error| {
            self.bt_output_stats.failed += 1;
            output_codec_error(error)
        })?;
        if self.codec.target == TargetCodec::Ds4Usb {
            kind = Ds5ControlKind::Other;
        }
        // Give unflagged reports their own slot, independent of all valid flags.
        let fields = if flags == 0 { 1 << 32 } else { flags };
        if let Some((Some(last), skipped)) = self.bt_controls.sent.slot(kind) {
            if last.is_repeat(output.as_bytes(), now)
                && !self
                    .bt_controls
                    .pending
                    .iter()
                    .any(|p| p.fields & fields != 0)
            {
                match skipped {
                    SendOutcome::SkippedLightbar => self.bt_output_stats.skipped_lightbar += 1,
                    SendOutcome::SkippedTriggersOff => {
                        self.bt_output_stats.skipped_triggers_off += 1
                    }
                }
                return Ok(());
            }
        }

        self.bt_output_stats.coalesced += self.bt_controls.enqueue(PendingControl {
            output,
            fields,
            ordering_mask: Ds5UsbOutput::ordering_mask(flags),
            kind,
            force_audio: self.force_audio_haptics,
        }) as u64;
        self.flush_bt_controls(now)
    }

    pub(super) fn flush_bt_controls(&mut self, now: Instant) -> io::Result<()> {
        self.bt_output_stats.log_if_due(now);
        while let Some(control) = self.bt_controls.take_due(now) {
            // Save the original, before masking rumble. A failed write clears the history.
            if let Some((slot, _)) = self.bt_controls.sent.slot(control.kind) {
                *slot = Some(SentControl {
                    sent_at: now,
                    report: control.output.as_bytes().to_vec(),
                });
            }
            let mut output = control.output;
            if control.force_audio {
                output.force_audio_haptics();
            }
            if let Err(error) = self.send_output_command(&OutputCommand::Ds5Usb(output)) {
                self.bt_output_stats.failed += 1;
                return Err(error);
            }
            self.bt_output_stats.written += 1;
        }
        Ok(())
    }

    fn send_output_command(&mut self, command: &OutputCommand) -> io::Result<()> {
        let report = self
            .codec
            .physical
            .encode_output(command, &mut self.physical_output_state)
            .map_err(output_codec_error)?;
        if let Err(error) = self.hidraw.write_output(&report) {
            self.bt_controls.sent = SentControls::default();
            if is_disconnect_io_error(&error) {
                DISCONNECTED.store(true, Ordering::SeqCst);
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn select_audio_haptics(&mut self) -> io::Result<()> {
        self.send_output_command(&OutputCommand::Ds5Usb(Ds5UsbOutput::audio_haptics_mode()))?;
        for control in &mut self.bt_controls.pending {
            control.force_audio = true;
        }
        self.bt_controls.sent = SentControls::default();
        Ok(())
    }

    pub(super) fn reset_bt_triggers_for_rebuild(&mut self) -> io::Result<()> {
        if self.codec.physical == PhysicalCodec::Ds5Bt && self.codec.target != TargetCodec::Ds4Usb {
            self.send_output_command(&OutputCommand::Ds5Usb(Ds5UsbOutput::triggers_off()))?;
        }
        Ok(())
    }

    fn discard_pending_speaker_volume(&mut self) {
        let volume = u64::from(Ds5UsbOutput::SPEAKER_VOLUME);
        self.bt_controls.pending.retain_mut(|control| {
            if control.fields & volume == 0 {
                return true;
            }
            control.fields &= !volume;
            if control.fields == 0 {
                self.bt_output_stats.coalesced += 1;
                return false;
            }
            control.output.clear_speaker_volume_update();
            control.ordering_mask = Ds5UsbOutput::ordering_mask(control.fields);
            true
        });
    }

    fn set_speaker_mode(&mut self, active: bool) -> io::Result<()> {
        self.send_output_command(&OutputCommand::Ds5Usb(Ds5UsbOutput::speaker_demo_mode(
            active,
        )))?;
        self.discard_pending_speaker_volume();
        self.bt_controls.sent = SentControls::default();
        Ok(())
    }

    fn send_live_audio(&mut self, frame: crate::control::haptics::AudioFrame) -> io::Result<()> {
        let speaker_active = frame.speaker.is_some();
        if speaker_active != self.speaker_active {
            self.set_speaker_mode(speaker_active)?;
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
            // Combined Opus frames also write the speaker volume.
            self.discard_pending_speaker_volume();
        } else {
            self.send_output_command(&OutputCommand::Haptics(HapticsFrame(frame.haptics)))?;
        }
        Ok(())
    }

    fn stop_speaker(&mut self) {
        if self.speaker_active {
            self.speaker_active = false;
            if !DISCONNECTED.load(Ordering::SeqCst) {
                if let Err(error) = self.set_speaker_mode(false) {
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
    fn skipped_and_invalid_reports_start_new_statistics_windows() {
        let (mut proxy, _uhid, _peer, start) = crate::proxy::tests::busy_bt_proxy();
        let mut light = [0; 48];
        light[..3].copy_from_slice(&[2, 0, 4]);
        proxy
            .submit_bt_control(&light, start + Duration::from_millis(4999))
            .unwrap();
        assert_eq!(proxy.bt_output_stats.written, 4);

        let boundary = start + Duration::from_secs(5);
        proxy.submit_bt_control(&light, boundary).unwrap();
        let stats = &proxy.bt_output_stats;
        assert_eq!(stats.started, Some(boundary));
        assert_eq!(
            (stats.received, stats.skipped_lightbar, stats.written),
            (1, 1, 0)
        );

        let after_idle = boundary + Duration::from_secs(10);
        proxy.submit_bt_control(&[], after_idle).unwrap_err();
        let stats = &proxy.bt_output_stats;
        assert_eq!(stats.started, Some(after_idle));
        assert_eq!(
            (stats.received, stats.failed, stats.skipped_lightbar),
            (1, 1, 0)
        );
    }

    #[test]
    fn repeated_report_refreshes_at_100_ms() {
        let now = Instant::now();
        let report = Ds5UsbOutput::speaker_demo_mode(false);
        let last = SentControl {
            sent_at: now,
            report: report.as_bytes().to_vec(),
        };
        assert!(last.is_repeat(report.as_bytes(), now + Duration::from_millis(99)));
        assert!(!last.is_repeat(report.as_bytes(), now + Duration::from_millis(100)));
    }
}
