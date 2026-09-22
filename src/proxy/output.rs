use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

use log::{error, info};

use crate::codec::{Ds5UsbOutput, HapticsFrame, OutputCommand};

use super::{is_disconnect_io_error, Proxy, DISCONNECTED};

impl Proxy {
    pub(super) fn write_physical_output(&self, report: &[u8]) -> io::Result<()> {
        self.hidraw
            .write_output(report)
            .map(|_| ())
            .inspect_err(|error| {
                error!("failed to write physical output report: {error}");
                if is_disconnect_io_error(error) {
                    DISCONNECTED.store(true, Ordering::SeqCst);
                }
            })
    }

    fn send_output_command(&mut self, command: &OutputCommand) -> io::Result<()> {
        let report = self
            .codec
            .physical
            .encode_output(command, &mut self.physical_output_state)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("output encoding failed: {error:?}"),
                )
            })?;
        self.write_physical_output(&report)
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
            let _ = self.send_output_command(&OutputCommand::Haptics(HapticsFrame::SILENCE));
        }
    }
}
