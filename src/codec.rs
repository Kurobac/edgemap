use crate::report::{self, Button, GamepadState};

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
    pub fn from_ds5_usb(raw: &[u8]) -> Option<Self> {
        if raw.len() < report::USB_INPUT_REPORT_SIZE {
            return None;
        }
        let mut source = [0u8; report::USB_INPUT_REPORT_SIZE];
        source.copy_from_slice(&raw[..report::USB_INPUT_REPORT_SIZE]);
        let state = report::parse_input_report(&source)?;
        Some(Self {
            state,
            source_report: SourceReport::Ds5Usb(source),
        })
    }

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

    pub fn encode_ds5_usb_input(&self, seq: u8) -> [u8; report::USB_INPUT_REPORT_SIZE] {
        match &self.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                report::apply_state_to_report(&mut out, &self.state, seq);
                out
            }
        }
    }

    pub fn encode_ds4_usb_input(&self, seq: u8) -> [u8; report::USB_INPUT_REPORT_SIZE] {
        match &self.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                report::apply_state_to_ds4_report(&mut out, &self.state, seq);
                out
            }
        }
    }
}

pub fn convert_ds4_usb_output_to_ds5_usb(ds4: &[u8]) -> [u8; 63] {
    report::convert_ds4_output_to_ds5(ds4)
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

        let mut frame = ControllerFrame::from_ds5_usb(&raw).unwrap();
        frame.state.set_button(Button::Cross, true);

        let out = frame.encode_ds5_usb_input(0x42);
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

        let frame = ControllerFrame::from_ds5_usb(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadLeft));

        raw[34] = 0xC0;
        raw[35] = 0x03;
        let frame = ControllerFrame::from_ds5_usb(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadRight));
    }

    #[test]
    fn ds4_output_conversion_is_exposed_through_codec_boundary() {
        let mut ds4 = [0u8; 32];
        ds4[0] = 0x05;
        ds4[1] = 0x01;
        ds4[4] = 64;
        ds4[5] = 128;
        let ds5 = convert_ds4_usb_output_to_ds5_usb(&ds4);

        assert_eq!(ds5[0], 0x02);
        assert_eq!(ds5[1] & 0x03, 0x03);
        assert_eq!(ds5[3], 64);
        assert_eq!(ds5[4], 128);
    }
}
