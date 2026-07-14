use crate::model::{Button, GamepadState};

use super::ds5_usb::{self, parse_touchpad_contact};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds5UsbOutput {
    pub(super) raw: Vec<u8>,
}

impl Ds5UsbOutput {
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds4UsbOutput {
    pub(super) raw: Vec<u8>,
}

impl Ds4UsbOutput {
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputCommand {
    Ds5Usb(Ds5UsbOutput),
    Ds4Usb(Ds4UsbOutput),
}

#[derive(Debug, Clone)]
pub enum SourceReport {
    Ds5Usb([u8; ds5_usb::INPUT_REPORT_SIZE]),
    Ds5Bt {
        usb_backing: [u8; ds5_usb::INPUT_REPORT_SIZE],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchpadContact {
    pub active: bool,
    pub id: u8,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchpadFrame {
    pub button: bool,
    pub contacts: [TouchpadContact; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionFrame {
    pub gyro: [i16; 3],
    pub accel: [i16; 3],
}

#[derive(Debug, Clone)]
pub struct ControllerFrame {
    pub state: GamepadState,
    pub motion: Option<MotionFrame>,
    pub(super) source_report: SourceReport,
}

impl ControllerFrame {
    pub fn touchpad(&self) -> Option<TouchpadFrame> {
        match &self.source_report {
            SourceReport::Ds5Usb(raw) => Some(TouchpadFrame {
                button: raw[10] & 0x02 != 0,
                contacts: [
                    parse_touchpad_contact(raw, 33),
                    parse_touchpad_contact(raw, 37),
                ],
            }),
            SourceReport::Ds5Bt { usb_backing, .. } => Some(TouchpadFrame {
                button: usb_backing[10] & 0x02 != 0,
                contacts: [
                    parse_touchpad_contact(usb_backing, 33),
                    parse_touchpad_contact(usb_backing, 37),
                ],
            }),
        }
    }

    pub fn touchpad_split_button(&self) -> Option<Button> {
        let touchpad = self.touchpad()?;
        if !touchpad.button {
            return None;
        }
        let contact = touchpad.contacts.iter().find(|contact| contact.active)?;
        Some(if contact.x < 960 {
            Button::TouchpadLeft
        } else {
            Button::TouchpadRight
        })
    }
}
