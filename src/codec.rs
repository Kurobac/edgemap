use crate::descriptor;
use crate::device::{self, DeviceInfo, SourceTransport, SonyDeviceKind};
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
    Ds5UsbAuto,
    Ds5UsbForced,
    Ds4Usb,
}

impl VirtualTarget {
    pub fn from_output_device(output_device: &str) -> Self {
        match output_device {
            "dualshock4" => Self::Ds4Usb,
            "dualsense" => Self::Ds5UsbForced,
            _ => Self::Ds5UsbAuto,
        }
    }

    pub fn encode_input(&self, frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => target_ds5_usb::encode_input(frame, seq),
            Self::Ds4Usb => target_ds4_usb::encode_input(frame, seq),
        }
    }

    pub fn encode_physical_ds5_usb_output(&self, data: &[u8]) -> CodecResult<Vec<u8>> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => physical_ds5_usb::encode_output_from_ds5_usb(data),
            Self::Ds4Usb => physical_ds5_usb::encode_output_from_ds4_usb(data),
        }
    }

    pub fn is_ds4(self) -> bool {
        matches!(self, Self::Ds4Usb)
    }

    pub fn usb_identity<'a>(
        self,
        source: &DeviceInfo,
        physical_report_descriptor: &'a [u8],
    ) -> UsbTargetIdentity<'a> {
        match self {
            Self::Ds4Usb => UsbTargetIdentity {
                name: "Wireless Controller".to_string(),
                // Keep the existing DS4 identity behavior: expose a fake UHID
                // uniq that matches the fake DS4 0x12 MAC report. This was
                // added for DS4 compatibility and is intentionally left as-is.
                uniq: "c0:13:37:00:00:01",
                product_id: device::DS4_PID as u32,
                report_descriptor: &descriptor::DS4_USB_DESCRIPTOR,
                label: "DualShock 4",
            },
            Self::Ds5UsbForced => UsbTargetIdentity {
                name: format!("{} Remapper", source.device_name()),
                // DS5 identity is served through the fake 0x09 feature-report
                // fallback, not UHID uniq. Keep uniq empty to preserve current
                // hid-playstation behavior and avoid physical MAC conflicts.
                uniq: "",
                product_id: device::DS5_PID as u32,
                report_descriptor: &descriptor::DS_USB_DESCRIPTOR,
                label: "DualSense (forced)",
            },
            Self::Ds5UsbAuto => UsbTargetIdentity {
                name: format!("{} Remapper", source.device_name()),
                // See Ds5UsbForced: DS5/Edge targets keep UHID uniq empty.
                uniq: "",
                product_id: source.pid as u32,
                report_descriptor: physical_report_descriptor,
                label: match source.kind {
                    SonyDeviceKind::DualSenseEdge => "DualSense Edge (auto)",
                    SonyDeviceKind::DualSense => "DualSense (auto)",
                },
            },
        }
    }
}

pub struct UsbTargetIdentity<'a> {
    pub name: String,
    pub uniq: &'static str,
    pub product_id: u32,
    pub report_descriptor: &'a [u8],
    pub label: &'static str,
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

        let ds5 = VirtualTarget::Ds5UsbAuto.encode_input(&frame, 0x10).unwrap();
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

    #[test]
    fn virtual_target_usb_identity_preserves_auto_and_forced_ds5_modes() {
        let source = DeviceInfo {
            path: std::path::PathBuf::from("/dev/hidraw0"),
            vid: device::SONY_VID,
            pid: device::DS5_EDGE_PID,
            kind: SonyDeviceKind::DualSenseEdge,
            transport: SourceTransport::Usb,
        };
        let physical_desc = [0x01, 0x02, 0x03];

        let auto = VirtualTarget::Ds5UsbAuto.usb_identity(&source, &physical_desc);
        assert_eq!(auto.product_id, device::DS5_EDGE_PID as u32);
        assert_eq!(auto.report_descriptor, &physical_desc);
        assert_eq!(auto.label, "DualSense Edge (auto)");

        let forced = VirtualTarget::Ds5UsbForced.usb_identity(&source, &physical_desc);
        assert_eq!(forced.product_id, device::DS5_PID as u32);
        assert_eq!(forced.report_descriptor, &descriptor::DS_USB_DESCRIPTOR);
        assert_eq!(forced.label, "DualSense (forced)");
    }
}
