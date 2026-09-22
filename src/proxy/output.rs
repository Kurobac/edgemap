use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

use log::{error, info};

use crate::codec::{Ds5UsbOutput, HapticsFrame, OutputCommand, PhysicalCodec};

use super::{is_disconnect_io_error, HapticsDemo, Proxy, DISCONNECTED};

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

    pub(super) fn start_haptics_demo(
        &mut self,
        now: Instant,
    ) -> Result<(), (&'static str, String)> {
        if self.codec.physical != PhysicalCodec::Ds5Bt {
            return Err((
                "unsupported-output",
                "physical controller is not Bluetooth".into(),
            ));
        }
        if self.haptics_demo.is_some() {
            return Err(("haptics-busy", "demo is still active".into()));
        }
        // Only the explicitly requested demo selects a mode. Normal game
        // output keeps its flags intact; PCM encoding itself has no override.
        self.send_output_command(&OutputCommand::Ds5Usb(Ds5UsbOutput::audio_haptics_mode()))
            .map_err(|error| ("output-failed", error.to_string()))?;
        self.haptics_demo = Some(HapticsDemo::new(now));
        info!("Bluetooth haptics demo started: left 1s, silence 0.5s, right 1s");
        Ok(())
    }

    pub(super) fn handle_haptics_tick(&mut self, now: Instant) {
        let Some(frame) = self
            .haptics_demo
            .as_mut()
            .and_then(|demo| demo.take_due_frame(now))
        else {
            return;
        };
        if let Err(error) = self.send_output_command(&OutputCommand::Haptics(frame)) {
            self.haptics_demo = None;
            error!("haptics demo stopped after output failure: {error}");
        } else if self
            .haptics_demo
            .as_ref()
            .is_some_and(|demo| demo.next_deadline().is_none())
        {
            self.haptics_demo = None;
            info!("Bluetooth haptics demo completed");
        }
    }

    pub(super) fn stop_haptics_demo(&mut self) {
        if self.haptics_demo.take().is_some() && !DISCONNECTED.load(Ordering::SeqCst) {
            if let Err(error) =
                self.send_output_command(&OutputCommand::Haptics(HapticsFrame::SILENCE))
            {
                error!("failed to send haptics stop frame: {error}");
            }
        }
    }
}
