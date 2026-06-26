use crate::device::{SourceTransport, SonyDeviceKind};
use crate::report::{self, Button, GamepadState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    InvalidReport,
}

pub type CodecResult<T> = Result<T, CodecError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCodec {
    Ds5Usb,
}

impl SourceCodec {
    pub fn from_device(kind: SonyDeviceKind, transport: SourceTransport) -> Self {
        match (kind, transport) {
            (SonyDeviceKind::DualSense | SonyDeviceKind::DualSenseEdge, SourceTransport::Usb) => {
                Self::Ds5Usb
            }
        }
    }

    pub fn input_report_size(self) -> usize {
        match self {
            Self::Ds5Usb => report::USB_INPUT_REPORT_SIZE,
        }
    }

    pub fn decode_input(self, raw: &[u8]) -> CodecResult<ControllerFrame> {
        match self {
            Self::Ds5Usb => input_ds5_usb::decode(raw),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualTarget {
    Ds5Usb,
    Ds4Usb,
}

impl VirtualTarget {
    pub fn from_output_device(output_device: &str) -> Self {
        if output_device == "dualshock4" {
            Self::Ds4Usb
        } else {
            Self::Ds5Usb
        }
    }

    pub fn encode_input(&self, frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match self {
            Self::Ds5Usb => target_ds5_usb::encode_input(frame, seq),
            Self::Ds4Usb => target_ds4_usb::encode_input(frame, seq),
        }
    }

    pub fn encode_physical_ds5_usb_output(&self, data: &[u8]) -> CodecResult<Vec<u8>> {
        match self {
            Self::Ds5Usb => physical_ds5_usb::encode_output_from_ds5_usb(data),
            Self::Ds4Usb => physical_ds5_usb::encode_output_from_ds4_usb(data),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SourceReport {
    Ds5Usb([u8; report::USB_INPUT_REPORT_SIZE]),
}

#[derive(Debug, Clone)]
pub struct ControllerFrame {
    pub state: GamepadState,
    source_report: SourceReport,
}

impl ControllerFrame {
    pub fn touchpad_split_button(&self) -> Option<Button> {
        match &self.source_report {
            SourceReport::Ds5Usb(raw) => {
                let pressed = raw[10] & 0x02 != 0;
                if !pressed {
                    return None;
                }
                let f0_contact = raw[33] & 0x80 == 0;
                if !f0_contact {
                    return None;
                }
                let x = ((raw[35] as u16 & 0x0F) << 8) | raw[34] as u16;
                Some(if x < 960 {
                    Button::TouchpadLeft
                } else {
                    Button::TouchpadRight
                })
            }
        }
    }
}

pub mod input_ds5_usb {
    use super::*;

    pub fn decode(raw: &[u8]) -> CodecResult<ControllerFrame> {
        if raw.len() < report::USB_INPUT_REPORT_SIZE {
            return Err(CodecError::InvalidReport);
        }
        let mut source = [0u8; report::USB_INPUT_REPORT_SIZE];
        source.copy_from_slice(&raw[..report::USB_INPUT_REPORT_SIZE]);
        let state = report::parse_input_report(&source).ok_or(CodecError::InvalidReport)?;
        Ok(ControllerFrame {
            state,
            source_report: SourceReport::Ds5Usb(source),
        })
    }
}

pub mod target_ds5_usb {
    use super::*;

    pub fn encode_input(frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match &frame.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                report::apply_state_to_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }
}

pub mod target_ds4_usb {
    use super::*;

    pub fn encode_input(frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match &frame.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                report::apply_state_to_ds4_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }
}

pub mod physical_ds5_usb {
    use super::*;

    pub fn encode_output_from_ds5_usb(data: &[u8]) -> CodecResult<Vec<u8>> {
        Ok(data.to_vec())
    }

    pub fn encode_output_from_ds4_usb(data: &[u8]) -> CodecResult<Vec<u8>> {
        Ok(report::convert_ds4_output_to_ds5(data).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds5_usb_raw() -> [u8; report::USB_INPUT_REPORT_SIZE] {
        let mut raw = [0u8; report::USB_INPUT_REPORT_SIZE];
        raw[0] = report::USB_INPUT_REPORT_ID;
        raw
    }

    #[test]
    fn ds5_usb_frame_encodes_remapped_state_over_backing_report() {
        let mut raw = ds5_usb_raw();
        raw[11] = 0x07;
        raw[33] = 0x80;

        let mut frame = input_ds5_usb::decode(&raw).unwrap();
        frame.state.set_button(Button::Cross, true);

        let out = target_ds5_usb::encode_input(&frame, 0x42).unwrap();
        assert_eq!(out[7], 0x42);
        assert_eq!(out[8] & 0x20, 0x20);
        assert_eq!(out[11] & 0x0F, 0x07);
        assert_eq!(out[33], 0x80);
    }

    #[test]
    fn touchpad_split_button_uses_source_report_backing() {
        let mut raw = ds5_usb_raw();
        raw[10] = 0x02;
        raw[33] = 0x00;
        raw[34] = 0xBF;
        raw[35] = 0x03;

        let frame = input_ds5_usb::decode(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadLeft));

        raw[34] = 0xC0;
        raw[35] = 0x03;
        let frame = input_ds5_usb::decode(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadRight));
    }

    #[test]
    fn ds4_output_conversion_is_exposed_through_codec_boundary() {
        let mut ds4 = [0u8; 32];
        ds4[0] = 0x05;
        ds4[1] = 0x01;
        ds4[4] = 64;
        ds4[5] = 128;
        let ds5 = physical_ds5_usb::encode_output_from_ds4_usb(&ds4).unwrap();

        assert_eq!(ds5[0], 0x02);
        assert_eq!(ds5[1] & 0x03, 0x03);
        assert_eq!(ds5[3], 64);
        assert_eq!(ds5[4], 128);
    }

    #[test]
    fn virtual_target_selects_expected_input_codec() {
        let mut raw = ds5_usb_raw();
        raw[33] = 0x80;
        let mut frame = input_ds5_usb::decode(&raw).unwrap();
        frame.state.set_button(Button::Cross, true);

        let ds5 = VirtualTarget::Ds5Usb.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds5[8] & 0x20, 0x20);

        let ds4 = VirtualTarget::Ds4Usb.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds4[5] & 0x20, 0x20);
    }

    #[test]
    fn source_codec_selects_ds5_usb_for_current_sony_devices() {
        for kind in [SonyDeviceKind::DualSense, SonyDeviceKind::DualSenseEdge] {
            let source = SourceCodec::from_device(kind, SourceTransport::Usb);
            assert_eq!(source, SourceCodec::Ds5Usb);
            assert_eq!(source.input_report_size(), report::USB_INPUT_REPORT_SIZE);
        }
    }
}
