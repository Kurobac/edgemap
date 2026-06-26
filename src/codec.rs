use crate::descriptor;
use crate::device::{self, DeviceInfo, SourceTransport, SonyDeviceKind};
use crate::report::{self, Button, GamepadState};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    InvalidReport,
}

pub type CodecResult<T> = Result<T, CodecError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecPipeline {
    pub source: SourceCodec,
    pub physical: PhysicalCodec,
    pub target: TargetCodec,
}

impl CodecPipeline {
    pub fn from_device_and_output(
        kind: SonyDeviceKind,
        transport: SourceTransport,
        output_device: &str,
    ) -> Self {
        let source = SourceCodec::from_device(kind, transport);
        let physical = source.physical_codec();
        let target = TargetCodec::from_output_device(output_device);
        Self { source, physical, target }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalFeatureReportRequest {
    pub report_id: u8,
    pub size: usize,
}

#[derive(Debug, Default)]
pub struct FeatureReportCache {
    reports: HashMap<u8, Vec<u8>>,
}

impl FeatureReportCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, report_id: u8, data: Vec<u8>) {
        self.reports.insert(report_id, data);
    }

    pub fn into_inner(self) -> HashMap<u8, Vec<u8>> {
        self.reports
    }
}

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

    pub fn physical_codec(self) -> PhysicalCodec {
        match self {
            Self::Ds5Usb => PhysicalCodec::Ds5Usb,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalCodec {
    Ds5Usb,
}

impl PhysicalCodec {
    pub fn encode_output(self, target: TargetCodec, data: &[u8]) -> CodecResult<Vec<u8>> {
        match (self, target) {
            (Self::Ds5Usb, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                physical_ds5_usb::encode_output_from_ds5_usb(data)
            }
            (Self::Ds5Usb, TargetCodec::Ds4Usb) => {
                physical_ds5_usb::encode_output_from_ds4_usb(data)
            }
        }
    }

    pub fn encode_set_report(self, target: TargetCodec, report_id: u8, data: &[u8]) -> Option<Vec<u8>> {
        match (self, target) {
            (Self::Ds5Usb, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                let mut full_data = vec![report_id];
                full_data.extend_from_slice(data);
                Some(full_data)
            }
            (Self::Ds5Usb, TargetCodec::Ds4Usb) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCodec {
    Ds5UsbAuto,
    Ds5UsbForced,
    Ds4Usb,
}

impl TargetCodec {
    pub fn from_output_device(output_device: &str) -> Self {
        match output_device {
            "dualshock4" => Self::Ds4Usb,
            "dualsense" => Self::Ds5UsbForced,
            _ => Self::Ds5UsbAuto,
        }
    }

    pub fn encode_input(self, frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => target_ds5_usb::encode_input(frame, seq),
            Self::Ds4Usb => target_ds4_usb::encode_input(frame, seq),
        }
    }

    pub fn seed_feature_reports(self, cache: &mut FeatureReportCache) {
        match self {
            Self::Ds4Usb => target_ds4_usb::seed_feature_reports(cache),
            Self::Ds5UsbAuto | Self::Ds5UsbForced => {}
        }
    }

    pub fn physical_feature_reports_to_cache(self) -> &'static [PhysicalFeatureReportRequest] {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => target_ds5_usb::PHYSICAL_FEATURE_REPORTS_TO_CACHE,
            Self::Ds4Usb => &[],
        }
    }

    pub fn fallback_feature_report(self, report_id: u8) -> Option<Vec<u8>> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => target_ds5_usb::fallback_feature_report(report_id),
            Self::Ds4Usb => target_ds4_usb::fallback_feature_report(report_id),
        }
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
    source_report: SourceReport,
}

impl ControllerFrame {
    pub fn touchpad(&self) -> Option<TouchpadFrame> {
        match &self.source_report {
            SourceReport::Ds5Usb(raw) => Some(TouchpadFrame {
                button: raw[10] & 0x02 != 0,
                contacts: [
                    parse_ds5_usb_touchpad_contact(raw, 33),
                    parse_ds5_usb_touchpad_contact(raw, 37),
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

fn read_i16_le(raw: &[u8; report::USB_INPUT_REPORT_SIZE], offset: usize) -> i16 {
    i16::from_le_bytes([raw[offset], raw[offset + 1]])
}

fn parse_ds5_usb_motion(raw: &[u8; report::USB_INPUT_REPORT_SIZE]) -> MotionFrame {
    MotionFrame {
        gyro: [
            read_i16_le(raw, 16),
            read_i16_le(raw, 18),
            read_i16_le(raw, 20),
        ],
        accel: [
            read_i16_le(raw, 22),
            read_i16_le(raw, 24),
            read_i16_le(raw, 26),
        ],
    }
}

fn write_ds5_usb_motion(raw: &mut [u8; report::USB_INPUT_REPORT_SIZE], motion: MotionFrame) {
    for (i, value) in motion.gyro.iter().chain(motion.accel.iter()).enumerate() {
        raw[16 + i * 2..18 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
}

fn parse_ds5_usb_touchpad_contact(raw: &[u8; report::USB_INPUT_REPORT_SIZE], base: usize) -> TouchpadContact {
    let contact = raw[base];
    let x = ((raw[base + 2] as u16 & 0x0F) << 8) | raw[base + 1] as u16;
    let y = ((raw[base + 3] as u16) << 4) | ((raw[base + 2] as u16 >> 4) & 0x0F);
    TouchpadContact {
        active: contact & 0x80 == 0,
        id: contact & 0x7F,
        x,
        y,
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
        let motion = Some(parse_ds5_usb_motion(&source));
        Ok(ControllerFrame {
            state,
            motion,
            source_report: SourceReport::Ds5Usb(source),
        })
    }
}

pub mod target_ds5_usb {
    use super::*;

    pub const PHYSICAL_FEATURE_REPORTS_TO_CACHE: &[PhysicalFeatureReportRequest] = &[
        PhysicalFeatureReportRequest { report_id: 0x05, size: 41 },
        PhysicalFeatureReportRequest { report_id: 0x20, size: 64 },
    ];

    pub fn encode_input(frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match &frame.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                report::apply_state_to_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }

    pub fn fallback_feature_report(report_id: u8) -> Option<Vec<u8>> {
        match report_id {
            0x05 => Some(vec![
                0x05, 0xff, 0xfc, 0xff, 0xfe, 0xff, 0x83, 0x22, 0x78, 0xdd,
                0x92, 0x22, 0x5f, 0xdd, 0x95, 0x22, 0x6d, 0xdd, 0x1c, 0x02,
                0x1c, 0x02, 0xf2, 0x1f, 0xed, 0xdf, 0xe3, 0x20, 0xda, 0xe0,
                0xee, 0x1f, 0xdf, 0xdf, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00,
            ]),
            0x08 => Some(vec![0u8; 48]),
            // Do not read or cache physical report 0x09. It contains the DS5
            // pairing MAC, and copying the physical MAC makes hid-playstation
            // create duplicate power_supply names. Use this fake fallback.
            0x09 => Some(vec![
                0x09, 0xd4, 0x2f, 0x4b, 0x26, 0x18, 0xc2, 0x08, 0x25,
                0x00, 0x1e, 0x00, 0xee, 0x74, 0xd0, 0xbc, 0x00, 0x00, 0x00, 0x00,
            ]),
            0x0A => Some(vec![0u8; 27]),
            0x20 => Some(vec![
                0x20, 0x4a, 0x75, 0x6e, 0x20, 0x31, 0x39, 0x20, 0x32,
                0x30, 0x32, 0x33, 0x31, 0x34, 0x3a, 0x34, 0x37, 0x3a, 0x33, 0x34,
                0x03, 0x00, 0x44, 0x00, 0x08, 0x02, 0x00, 0x01, 0x36, 0x00,
                0x00, 0x01, 0xc1, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x54, 0x01, 0x00, 0x00, 0x14, 0x00,
                0x00, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
            ]),
            0x21 => Some(vec![0u8; 5]),
            0x22 => Some(vec![0u8; 64]),
            0x70..=0x7B => Some(vec![0u8; 64]),
            0x80 | 0x81 | 0x83 | 0x84 | 0xE0 | 0xF0 | 0xF1 | 0xF4 | 0x60..=0x65 | 0x68 => {
                Some(vec![0u8; 64])
            }
            0x82 => Some(vec![0u8; 10]),
            0x85 | 0xF5 => Some(vec![0u8; 4]),
            0xA0 => Some(vec![0u8; 2]),
            0xF2 => Some(vec![0u8; 53]),
            _ => None,
        }
    }
}

pub mod target_ds4_usb {
    use super::*;

    pub fn encode_input(frame: &ControllerFrame, seq: u8) -> CodecResult<[u8; report::USB_INPUT_REPORT_SIZE]> {
        match &frame.source_report {
            SourceReport::Ds5Usb(raw) => {
                let mut out = *raw;
                if let Some(motion) = frame.motion {
                    write_ds5_usb_motion(&mut out, motion);
                }
                report::apply_state_to_ds4_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }

    pub fn seed_feature_reports(cache: &mut FeatureReportCache) {
        // DS4 calibration data (report 0x02, 37 bytes). Produces 1:1 scale +
        // zero bias so raw gyro/accel passes through unchanged.
        let mut cal = vec![0u8; 37];
        cal[0] = 0x02;
        let w16 = |buf: &mut [u8], off, v: u16| buf[off..off+2].copy_from_slice(&v.to_le_bytes());
        w16(&mut cal,  7, 1024);        // gyro_pitch_plus
        w16(&mut cal,  9, (-1024i16) as u16); // gyro_pitch_minus
        w16(&mut cal, 11, 1024);        // gyro_yaw_plus
        w16(&mut cal, 13, (-1024i16) as u16); // gyro_yaw_minus
        w16(&mut cal, 15, 1024);        // gyro_roll_plus
        w16(&mut cal, 17, (-1024i16) as u16); // gyro_roll_minus
        w16(&mut cal, 19, 1);           // gyro_speed_plus
        w16(&mut cal, 21, 1);           // gyro_speed_minus
        w16(&mut cal, 23, 8192);        // acc_x_plus
        w16(&mut cal, 25, (-8192i16) as u16); // acc_x_minus
        w16(&mut cal, 27, 8192);        // acc_y_plus
        w16(&mut cal, 29, (-8192i16) as u16); // acc_y_minus
        w16(&mut cal, 31, 8192);        // acc_z_plus
        w16(&mut cal, 33, (-8192i16) as u16); // acc_z_minus
        cache.insert(0x02, cal);

        // DS4 firmware info (report 0xA3, 49 bytes). Layout matches real DS4
        // dump (ViGEmBus/eccelerator reference).
        let mut fw = vec![0u8; 49];
        fw[0] = 0xA3;
        fw[1..12].copy_from_slice(b"Aug  3 2013");
        fw[17..25].copy_from_slice(b"07:01:12");
        w16(&mut fw, 34, 0x0001);   // hw_version
        w16(&mut fw, 36, 0x0331);   // sub-version
        w16(&mut fw, 41, 0x0049);   // fw_version (real DS4 value)
        fw[43] = 0x05;
        w16(&mut fw, 46, 0x0380);   // build number
        cache.insert(0xA3, fw);

        let mut mac = vec![0u8; 16];
        mac[0] = 0x12;
        // MAC addresses in DS4 reversed byte order (matching ViGEmBus convention).
        // Bytes 7-9: USB connection status (0x08 0x25 0x00 from real DS4 dump).
        mac[1..7].copy_from_slice(&[0x01, 0x00, 0x00, 0x37, 0x13, 0xC0]); // target MAC (reversed C0:13:37:00:00:01)
        mac[7] = 0x08;
        mac[8] = 0x25;
        mac[9] = 0x00;
        // bytes 10-15: host MAC — USB connection: all zero (matching reWASD)
        cache.insert(0x12, mac);
    }

    pub fn fallback_feature_report(report_id: u8) -> Option<Vec<u8>> {
        match report_id {
            // DS4 USB descriptors advertise vendor feature reports 0x80/0x81
            // with 6-byte payloads. A real DS4 was observed to reject GET
            // 0x80 with EPIPE but return 7 bytes for GET 0x81. ViGEmBus also
            // advertises these reports without documenting a useful 0x81
            // meaning. Before the codec split, Proxy's shared fallback table
            // answered 0x81 for every target, including DS4, so this request
            // failed silently. Moving fallbacks into target codecs changed that
            // behavior by making DS4 return None; Linux/desktop probe behavior
            // still expects 0x81 to be readable, so provide a zeroed
            // compatibility response here.
            0x81 => Some(vec![0x81, 0, 0, 0, 0, 0, 0]),
            _ => None,
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
    fn touchpad_frame_decodes_two_contacts() {
        let mut raw = ds5_usb_raw();
        raw[10] = 0x02;
        raw[33] = 0x05;
        raw[34] = 0x34;
        raw[35] = 0x12;
        raw[36] = 0x56;
        raw[37] = 0x86;
        raw[38] = 0x78;
        raw[39] = 0x9A;
        raw[40] = 0xBC;

        let frame = input_ds5_usb::decode(&raw).unwrap();
        let touchpad = frame.touchpad().unwrap();

        assert!(touchpad.button);
        assert_eq!(touchpad.contacts[0], TouchpadContact {
            active: true,
            id: 0x05,
            x: 0x234,
            y: 0x561,
        });
        assert_eq!(touchpad.contacts[1], TouchpadContact {
            active: false,
            id: 0x06,
            x: 0xA78,
            y: 0xBC9,
        });
    }

    #[test]
    fn touchpad_split_uses_first_active_contact() {
        let mut raw = ds5_usb_raw();
        raw[10] = 0x02;
        raw[33] = 0x80;
        raw[37] = 0x01;
        raw[38] = 0xC0;
        raw[39] = 0x03;

        let frame = input_ds5_usb::decode(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadRight));
    }

    #[test]
    fn motion_frame_decodes_ds5_usb_raw_axes() {
        let mut raw = ds5_usb_raw();
        let values = [-1000i16, 2000, -3000, 4000, -5000, 6000];
        for (i, value) in values.iter().enumerate() {
            raw[16 + i * 2..18 + i * 2].copy_from_slice(&value.to_le_bytes());
        }

        let frame = input_ds5_usb::decode(&raw).unwrap();
        assert_eq!(frame.motion, Some(MotionFrame {
            gyro: [-1000, 2000, -3000],
            accel: [4000, -5000, 6000],
        }));
    }

    #[test]
    fn ds4_output_conversion_is_exposed_through_codec_boundary() {
        let mut ds4 = [0u8; 32];
        ds4[0] = 0x05;
        ds4[1] = 0x01;
        ds4[4] = 64;
        ds4[5] = 128;
        let ds5 = PhysicalCodec::Ds5Usb
            .encode_output(TargetCodec::Ds4Usb, &ds4)
            .unwrap();

        assert_eq!(ds5[0], 0x02);
        assert_eq!(ds5[1] & 0x03, 0x03);
        assert_eq!(ds5[3], 64);
        assert_eq!(ds5[4], 128);
    }

    #[test]
    fn target_codec_selects_expected_input_codec() {
        let mut raw = ds5_usb_raw();
        raw[33] = 0x80;
        let mut frame = input_ds5_usb::decode(&raw).unwrap();
        frame.state.set_button(Button::Cross, true);

        let ds5 = TargetCodec::Ds5UsbAuto.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds5[8] & 0x20, 0x20);

        let ds4 = TargetCodec::Ds4Usb.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds4[5] & 0x20, 0x20);
    }

    #[test]
    fn source_codec_selects_ds5_usb_for_current_sony_devices() {
        for kind in [SonyDeviceKind::DualSense, SonyDeviceKind::DualSenseEdge] {
            let source = SourceCodec::from_device(kind, SourceTransport::Usb);
            assert_eq!(source, SourceCodec::Ds5Usb);
            assert_eq!(source.physical_codec(), PhysicalCodec::Ds5Usb);
            assert_eq!(source.input_report_size(), report::USB_INPUT_REPORT_SIZE);
        }
    }

    #[test]
    fn codec_pipeline_selects_current_usb_codecs() {
        let pipeline = CodecPipeline::from_device_and_output(
            SonyDeviceKind::DualSenseEdge,
            SourceTransport::Usb,
            "dualshock4",
        );

        assert_eq!(pipeline.source, SourceCodec::Ds5Usb);
        assert_eq!(pipeline.physical, PhysicalCodec::Ds5Usb);
        assert_eq!(pipeline.target, TargetCodec::Ds4Usb);
    }

    #[test]
    fn target_codec_usb_identity_preserves_auto_and_forced_ds5_modes() {
        let source = DeviceInfo {
            path: std::path::PathBuf::from("/dev/hidraw0"),
            vid: device::SONY_VID,
            pid: device::DS5_EDGE_PID,
            kind: SonyDeviceKind::DualSenseEdge,
            transport: SourceTransport::Usb,
        };
        let physical_desc = [0x01, 0x02, 0x03];

        let auto = TargetCodec::Ds5UsbAuto.usb_identity(&source, &physical_desc);
        assert_eq!(auto.product_id, device::DS5_EDGE_PID as u32);
        assert_eq!(auto.report_descriptor, &physical_desc);
        assert_eq!(auto.label, "DualSense Edge (auto)");

        let forced = TargetCodec::Ds5UsbForced.usb_identity(&source, &physical_desc);
        assert_eq!(forced.product_id, device::DS5_PID as u32);
        assert_eq!(forced.report_descriptor, &descriptor::DS_USB_DESCRIPTOR);
        assert_eq!(forced.label, "DualSense (forced)");
    }

    #[test]
    fn ds4_target_seeds_feature_reports() {
        let mut cache = FeatureReportCache::new();
        TargetCodec::Ds4Usb.seed_feature_reports(&mut cache);
        let cache = cache.into_inner();

        assert_eq!(cache.get(&0x02).unwrap().len(), 37);
        assert_eq!(cache.get(&0x12).unwrap()[1..7], [0x01, 0x00, 0x00, 0x37, 0x13, 0xC0]);
        assert_eq!(cache.get(&0xA3).unwrap().len(), 49);
    }

    #[test]
    fn ds5_target_fallback_uses_fake_pairing_mac() {
        let data = TargetCodec::Ds5UsbAuto.fallback_feature_report(0x09).unwrap();

        assert_eq!(data.len(), 20);
        assert_eq!(data[0], 0x09);
        assert_eq!(&data[1..7], &[0xd4, 0x2f, 0x4b, 0x26, 0x18, 0xc2]);
    }

    #[test]
    fn ds4_target_fallback_serves_vendor_probe_reports() {
        assert!(TargetCodec::Ds4Usb.fallback_feature_report(0x80).is_none());
        assert_eq!(
            TargetCodec::Ds4Usb.fallback_feature_report(0x81),
            Some(vec![0x81, 0, 0, 0, 0, 0, 0])
        );
        assert!(TargetCodec::Ds4Usb.fallback_feature_report(0x09).is_none());
    }

    #[test]
    fn physical_codec_set_report_policy_respects_target_codec() {
        let data = [0x11, 0x22, 0x33];

        assert_eq!(
            PhysicalCodec::Ds5Usb.encode_set_report(TargetCodec::Ds5UsbAuto, 0x31, &data),
            Some(vec![0x31, 0x11, 0x22, 0x33])
        );
        assert_eq!(
            PhysicalCodec::Ds5Usb.encode_set_report(TargetCodec::Ds5UsbForced, 0x31, &data),
            Some(vec![0x31, 0x11, 0x22, 0x33])
        );
        assert_eq!(
            PhysicalCodec::Ds5Usb.encode_set_report(TargetCodec::Ds4Usb, 0x31, &data),
            None
        );
    }

    #[test]
    fn ds5_targets_request_only_safe_physical_feature_reports() {
        let requests = TargetCodec::Ds5UsbAuto.physical_feature_reports_to_cache();

        assert_eq!(requests, [
            PhysicalFeatureReportRequest { report_id: 0x05, size: 41 },
            PhysicalFeatureReportRequest { report_id: 0x20, size: 64 },
        ]);
        assert!(!requests.iter().any(|r| r.report_id == 0x09));
        assert!(TargetCodec::Ds4Usb.physical_feature_reports_to_cache().is_empty());
    }

    #[test]
    fn target_codec_selects_from_output_device_config() {
        assert_eq!(TargetCodec::from_output_device("auto"), TargetCodec::Ds5UsbAuto);
        assert_eq!(TargetCodec::from_output_device("dualsense"), TargetCodec::Ds5UsbForced);
        assert_eq!(TargetCodec::from_output_device("dualshock4"), TargetCodec::Ds4Usb);
        assert_eq!(TargetCodec::from_output_device("unknown"), TargetCodec::Ds5UsbAuto);
    }
}
