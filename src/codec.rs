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
    Ds5Bt,
}

impl SourceCodec {
    pub fn from_device(kind: SonyDeviceKind, transport: SourceTransport) -> Self {
        match (kind, transport) {
            (SonyDeviceKind::DualSense | SonyDeviceKind::DualSenseEdge, SourceTransport::Usb) => {
                Self::Ds5Usb
            }
            (SonyDeviceKind::DualSense | SonyDeviceKind::DualSenseEdge, SourceTransport::Bluetooth) => {
                Self::Ds5Bt
            }
        }
    }

    pub fn input_report_size(self) -> usize {
        match self {
            Self::Ds5Usb => report::USB_INPUT_REPORT_SIZE,
            Self::Ds5Bt => DS5_BT_INPUT_REPORT_SIZE,
        }
    }

    pub fn decode_input(self, raw: &[u8]) -> CodecResult<ControllerFrame> {
        match self {
            Self::Ds5Usb => input_ds5_usb::decode(raw),
            Self::Ds5Bt => input_ds5_bt::decode(raw),
        }
    }

    pub fn physical_codec(self) -> PhysicalCodec {
        match self {
            Self::Ds5Usb => PhysicalCodec::Ds5Usb,
            Self::Ds5Bt => PhysicalCodec::Ds5Bt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalCodec {
    Ds5Usb,
    Ds5Bt,
}

#[derive(Debug, Default)]
pub struct PhysicalOutputState {
    ds5_bt_seq: u8,
}

impl PhysicalCodec {
    pub fn feature_reports_to_cache(self, target: TargetCodec) -> &'static [PhysicalFeatureReportRequest] {
        match (self, target) {
            (Self::Ds5Usb, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                target_ds5_usb::PHYSICAL_FEATURE_REPORTS_TO_CACHE
            }
            (Self::Ds5Usb, TargetCodec::Ds4Usb) => &[],
            // DS5 Bluetooth feature reports use the same IDs for calibration
            // and firmware info, but carry Bluetooth feature CRC framing. Do
            // not cache or hand them to the USB virtual target until there is
            // an explicit BT-feature-to-USB-feature conversion path.
            (Self::Ds5Bt, _) => &[],
        }
    }

    pub fn encode_output(self, command: &OutputCommand, state: &mut PhysicalOutputState) -> CodecResult<Vec<u8>> {
        match (self, command) {
            (Self::Ds5Usb, OutputCommand::Ds5Usb(output)) => {
                physical_ds5_usb::encode_output_from_ds5_usb(output)
            }
            (Self::Ds5Usb, OutputCommand::Ds4Usb(output)) => {
                physical_ds5_usb::encode_output_from_ds4_usb(output)
            }
            (Self::Ds5Bt, OutputCommand::Ds5Usb(output)) => {
                physical_ds5_bt::encode_output_from_ds5_usb(output, state)
            }
            (Self::Ds5Bt, OutputCommand::Ds4Usb(output)) => {
                let ds5 = report::convert_ds4_output_to_ds5(output.as_bytes());
                physical_ds5_bt::encode_output_from_ds5_usb_bytes(&ds5, state)
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
            // Bluetooth SET_REPORT needs feature-report framing/CRC and is
            // mainly used by vendor/test tools in the known cases so far
            // (for example factory speaker commands). Games normally drive
            // rumble, LEDs, mic LED, player LEDs, and adaptive triggers through
            // the main output report path below, so keep this unsupported until
            // a real game/hardware need appears.
            (Self::Ds5Bt, _) => None,
        }
    }
}

// TODO(output-abstraction): Ds5UsbOutput is intentionally still a raw
// target-format wrapper. The BT physical path wraps this USB payload in the
// known 0x31 Bluetooth envelope, which preserves rumble, LEDs, mic LED, player
// LEDs, and adaptive-trigger payloads for current DS5/Edge targets. We do not
// normalize those fields into structured commands until a new target/device
// needs real semantic conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds5UsbOutput {
    raw: Vec<u8>,
}

impl Ds5UsbOutput {
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds4UsbOutput {
    raw: Vec<u8>,
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

    pub fn decode_output(self, data: &[u8]) -> CodecResult<OutputCommand> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => {
                Ok(OutputCommand::Ds5Usb(target_ds5_usb::decode_output(data)))
            }
            Self::Ds4Usb => Ok(OutputCommand::Ds4Usb(target_ds4_usb::decode_output(data))),
        }
    }

    pub fn seed_feature_reports(self, cache: &mut FeatureReportCache) {
        match self {
            Self::Ds4Usb => target_ds4_usb::seed_feature_reports(cache),
            Self::Ds5UsbAuto | Self::Ds5UsbForced => {}
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
                report_descriptor: match source.transport {
                    SourceTransport::Usb => physical_report_descriptor,
                    SourceTransport::Bluetooth => match source.kind {
                        SonyDeviceKind::DualSense => &descriptor::DS_USB_DESCRIPTOR,
                        SonyDeviceKind::DualSenseEdge => &descriptor::DS_EDGE_USB_DESCRIPTOR,
                    },
                },
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

pub const DS5_BT_INPUT_REPORT_ID: u8 = 0x31;
pub const DS5_BT_INPUT_REPORT_SIZE: usize = 78;
const DS5_BT_INPUT_COMMON_OFFSET: usize = 2;
const DS5_BT_CRC_SIZE: usize = 4;
const DS5_USB_OUTPUT_REPORT_ID: u8 = 0x02;
const DS5_USB_OUTPUT_REPORT_MIN_SIZE: usize = 48;
const DS5_USB_OUTPUT_REPORT_MAX_SIZE: usize = 64;
const DS5_BT_OUTPUT_REPORT_ID: u8 = 0x31;
const DS5_BT_OUTPUT_REPORT_SIZE: usize = 78;
const DS5_BT_OUTPUT_TAG: u8 = 0x10;
const DS5_BT_OUTPUT_PAYLOAD_OFFSET: usize = 3;
const DS5_BT_OUTPUT_CRC_OFFSET: usize = 74;
const PS_INPUT_CRC32_SEED: u8 = 0xA1;
const PS_OUTPUT_CRC32_SEED: u8 = 0xA2;

#[derive(Debug, Clone)]
pub enum SourceReport {
    Ds5Usb([u8; report::USB_INPUT_REPORT_SIZE]),
    Ds5Bt {
        usb_backing: [u8; report::USB_INPUT_REPORT_SIZE],
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
            SourceReport::Ds5Bt { usb_backing, .. } => Some(TouchpadFrame {
                button: usb_backing[10] & 0x02 != 0,
                contacts: [
                    parse_ds5_usb_touchpad_contact(usb_backing, 33),
                    parse_ds5_usb_touchpad_contact(usb_backing, 37),
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

fn crc32_le_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn ps_crc32(seed: u8, data: &[u8]) -> u32 {
    let crc = crc32_le_update(0xFFFF_FFFF, &[seed]);
    !crc32_le_update(crc, data)
}

fn check_ps_crc32(seed: u8, data: &[u8], expected: u32) -> bool {
    ps_crc32(seed, data) == expected
}

// DualSense Bluetooth input reports carry the same 63-byte common payload as
// USB input reports, wrapped with Bluetooth-specific header/trailer bytes.

fn ds5_bt_to_usb_backing(raw: &[u8; DS5_BT_INPUT_REPORT_SIZE]) -> [u8; report::USB_INPUT_REPORT_SIZE] {
    let mut usb = [0u8; report::USB_INPUT_REPORT_SIZE];
    usb[0] = report::USB_INPUT_REPORT_ID;
    usb[1..].copy_from_slice(
        &raw[DS5_BT_INPUT_COMMON_OFFSET..DS5_BT_INPUT_COMMON_OFFSET + report::USB_INPUT_REPORT_SIZE - 1],
    );
    usb
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

pub mod input_ds5_bt {
    use super::*;

    pub fn decode(raw: &[u8]) -> CodecResult<ControllerFrame> {
        if raw.len() < DS5_BT_INPUT_REPORT_SIZE {
            return Err(CodecError::InvalidReport);
        }
        if raw[0] != DS5_BT_INPUT_REPORT_ID {
            return Err(CodecError::InvalidReport);
        }

        let mut source = [0u8; DS5_BT_INPUT_REPORT_SIZE];
        source.copy_from_slice(&raw[..DS5_BT_INPUT_REPORT_SIZE]);
        let crc_offset = DS5_BT_INPUT_REPORT_SIZE - DS5_BT_CRC_SIZE;
        let expected_crc = u32::from_le_bytes([
            source[crc_offset],
            source[crc_offset + 1],
            source[crc_offset + 2],
            source[crc_offset + 3],
        ]);
        if !check_ps_crc32(PS_INPUT_CRC32_SEED, &source[..crc_offset], expected_crc) {
            return Err(CodecError::InvalidReport);
        }

        let usb_backing = ds5_bt_to_usb_backing(&source);
        let state = report::parse_input_report(&usb_backing).ok_or(CodecError::InvalidReport)?;
        let motion = Some(parse_ds5_usb_motion(&usb_backing));
        Ok(ControllerFrame {
            state,
            motion,
            source_report: SourceReport::Ds5Bt {
                usb_backing,
            },
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
            SourceReport::Ds5Bt { usb_backing, .. } => {
                let mut out = *usb_backing;
                report::apply_state_to_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }

    pub fn decode_output(data: &[u8]) -> Ds5UsbOutput {
        Ds5UsbOutput { raw: data.to_vec() }
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
            SourceReport::Ds5Bt { usb_backing, .. } => {
                let mut out = *usb_backing;
                if let Some(motion) = frame.motion {
                    write_ds5_usb_motion(&mut out, motion);
                }
                report::apply_state_to_ds4_report(&mut out, &frame.state, seq);
                Ok(out)
            }
        }
    }

    pub fn decode_output(data: &[u8]) -> Ds4UsbOutput {
        Ds4UsbOutput { raw: data.to_vec() }
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

    pub fn encode_output_from_ds5_usb(output: &Ds5UsbOutput) -> CodecResult<Vec<u8>> {
        Ok(output.as_bytes().to_vec())
    }

    pub fn encode_output_from_ds4_usb(output: &Ds4UsbOutput) -> CodecResult<Vec<u8>> {
        Ok(report::convert_ds4_output_to_ds5(output.as_bytes()).to_vec())
    }
}

pub mod physical_ds5_bt {
    use super::*;

    pub fn encode_output_from_ds5_usb(
        output: &Ds5UsbOutput,
        state: &mut PhysicalOutputState,
    ) -> CodecResult<Vec<u8>> {
        encode_output_from_ds5_usb_bytes(output.as_bytes(), state)
    }

    pub fn encode_output_from_ds5_usb_bytes(
        usb: &[u8],
        state: &mut PhysicalOutputState,
    ) -> CodecResult<Vec<u8>> {
        if usb.len() < DS5_USB_OUTPUT_REPORT_MIN_SIZE
            || usb.len() > DS5_USB_OUTPUT_REPORT_MAX_SIZE
            || usb[0] != DS5_USB_OUTPUT_REPORT_ID
        {
            return Err(CodecError::InvalidReport);
        }

        let mut bt = vec![0u8; DS5_BT_OUTPUT_REPORT_SIZE];
        bt[0] = DS5_BT_OUTPUT_REPORT_ID;
        bt[1] = (state.ds5_bt_seq & 0x0F) << 4;
        bt[2] = DS5_BT_OUTPUT_TAG;
        let payload_len = usb.len() - 1;
        bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET..DS5_BT_OUTPUT_PAYLOAD_OFFSET + payload_len]
            .copy_from_slice(&usb[1..]);

        state.ds5_bt_seq = (state.ds5_bt_seq + 1) & 0x0F;

        let crc = ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET]);
        bt[DS5_BT_OUTPUT_CRC_OFFSET..DS5_BT_OUTPUT_CRC_OFFSET + 4]
            .copy_from_slice(&crc.to_le_bytes());
        Ok(bt)
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

    fn ds5_bt_raw_from_usb(usb: &[u8; report::USB_INPUT_REPORT_SIZE]) -> [u8; DS5_BT_INPUT_REPORT_SIZE] {
        let mut raw = [0u8; DS5_BT_INPUT_REPORT_SIZE];
        raw[0] = DS5_BT_INPUT_REPORT_ID;
        raw[2..65].copy_from_slice(&usb[1..]);
        let crc = ps_crc32(PS_INPUT_CRC32_SEED, &raw[..DS5_BT_INPUT_REPORT_SIZE - DS5_BT_CRC_SIZE]);
        raw[74..78].copy_from_slice(&crc.to_le_bytes());
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
    fn ds5_bt_frame_decodes_common_payload_like_usb() {
        let mut usb = ds5_usb_raw();
        usb[1] = 10;
        usb[2] = 20;
        usb[3] = 30;
        usb[4] = 40;
        usb[5] = 50;
        usb[6] = 60;
        usb[8] = 0x20;
        usb[10] = 0x42;
        usb[16..18].copy_from_slice(&(-123i16).to_le_bytes());
        usb[18..20].copy_from_slice(&(456i16).to_le_bytes());
        usb[20..22].copy_from_slice(&(-789i16).to_le_bytes());
        usb[22..24].copy_from_slice(&(111i16).to_le_bytes());
        usb[24..26].copy_from_slice(&(-222i16).to_le_bytes());
        usb[26..28].copy_from_slice(&(333i16).to_le_bytes());
        usb[33] = 0x07;
        usb[34] = 0x34;
        usb[35] = 0x12;
        usb[36] = 0x56;

        let bt = ds5_bt_raw_from_usb(&usb);
        let frame = input_ds5_bt::decode(&bt).unwrap();

        assert_eq!(frame.state.left_stick_x, 10);
        assert_eq!(frame.state.left_stick_y, 20);
        assert_eq!(frame.state.right_stick_x, 30);
        assert_eq!(frame.state.right_stick_y, 40);
        assert_eq!(frame.state.l2_analog, 50);
        assert_eq!(frame.state.r2_analog, 60);
        assert!(frame.state.button(Button::Cross));
        assert!(frame.state.button(Button::Touchpad));
        assert!(frame.state.button(Button::LeftPaddle));
        assert_eq!(frame.motion, Some(MotionFrame {
            gyro: [-123, 456, -789],
            accel: [111, -222, 333],
        }));
        assert_eq!(frame.touchpad().unwrap().contacts[0], TouchpadContact {
            active: true,
            id: 0x07,
            x: 0x234,
            y: 0x561,
        });
    }

    #[test]
    fn ds5_bt_frame_rejects_bad_crc_or_report_shape() {
        let usb = ds5_usb_raw();
        let mut bt = ds5_bt_raw_from_usb(&usb);
        bt[10] ^= 0x01;
        assert!(matches!(input_ds5_bt::decode(&bt), Err(CodecError::InvalidReport)));

        let mut bt = ds5_bt_raw_from_usb(&usb);
        bt[0] = 0x01;
        assert!(matches!(input_ds5_bt::decode(&bt), Err(CodecError::InvalidReport)));
        assert!(matches!(input_ds5_bt::decode(&bt[..77]), Err(CodecError::InvalidReport)));
    }

    #[test]
    fn ds4_output_conversion_is_exposed_through_codec_boundary() {
        let mut ds4 = [0u8; 32];
        ds4[0] = 0x05;
        ds4[1] = 0x01;
        ds4[4] = 64;
        ds4[5] = 128;
        let command = TargetCodec::Ds4Usb.decode_output(&ds4).unwrap();
        let mut state = PhysicalOutputState::default();
        let ds5 = PhysicalCodec::Ds5Usb.encode_output(&command, &mut state).unwrap();

        assert_eq!(ds5[0], 0x02);
        assert_eq!(ds5[1] & 0x03, 0x03);
        assert_eq!(ds5[3], 64);
        assert_eq!(ds5[4], 128);
    }

    #[test]
    fn ds5_output_passthrough_is_exposed_through_codec_boundary() {
        let ds5 = [0x02, 0x01, 0x02, 0x03];
        let command = TargetCodec::Ds5UsbAuto.decode_output(&ds5).unwrap();
        let mut state = PhysicalOutputState::default();
        let encoded = PhysicalCodec::Ds5Usb.encode_output(&command, &mut state).unwrap();

        assert_eq!(encoded, ds5);
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
    fn target_codecs_encode_bt_source_frames_to_usb_targets() {
        let mut usb = ds5_usb_raw();
        usb[33] = 0x80;
        let bt = ds5_bt_raw_from_usb(&usb);
        let mut frame = input_ds5_bt::decode(&bt).unwrap();
        frame.state.set_button(Button::Cross, true);

        let ds5 = TargetCodec::Ds5UsbAuto.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds5[0], report::USB_INPUT_REPORT_ID);
        assert_eq!(ds5[8] & 0x20, 0x20);

        let ds4 = TargetCodec::Ds4Usb.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds4[0], report::USB_INPUT_REPORT_ID);
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
    fn source_codec_selects_ds5_bt_for_current_sony_devices() {
        for kind in [SonyDeviceKind::DualSense, SonyDeviceKind::DualSenseEdge] {
            let source = SourceCodec::from_device(kind, SourceTransport::Bluetooth);
            assert_eq!(source, SourceCodec::Ds5Bt);
            assert_eq!(source.physical_codec(), PhysicalCodec::Ds5Bt);
            assert_eq!(source.input_report_size(), DS5_BT_INPUT_REPORT_SIZE);
        }
    }

    #[test]
    fn codec_pipeline_selects_usb_and_bt_codecs() {
        let pipeline = CodecPipeline::from_device_and_output(
            SonyDeviceKind::DualSenseEdge,
            SourceTransport::Usb,
            "dualshock4",
        );

        assert_eq!(pipeline.source, SourceCodec::Ds5Usb);
        assert_eq!(pipeline.physical, PhysicalCodec::Ds5Usb);
        assert_eq!(pipeline.target, TargetCodec::Ds4Usb);

        let pipeline = CodecPipeline::from_device_and_output(
            SonyDeviceKind::DualSenseEdge,
            SourceTransport::Bluetooth,
            "dualshock4",
        );

        assert_eq!(pipeline.source, SourceCodec::Ds5Bt);
        assert_eq!(pipeline.physical, PhysicalCodec::Ds5Bt);
        assert_eq!(pipeline.target, TargetCodec::Ds4Usb);
    }

    #[test]
    fn target_codec_usb_identity_preserves_usb_auto_and_forced_ds5_modes() {
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
    fn target_codec_usb_identity_uses_builtin_descriptors_for_bt_auto() {
        let ds_source = DeviceInfo {
            path: std::path::PathBuf::from("/dev/hidraw0"),
            vid: device::SONY_VID,
            pid: device::DS5_PID,
            kind: SonyDeviceKind::DualSense,
            transport: SourceTransport::Bluetooth,
        };
        let edge_source = DeviceInfo {
            path: std::path::PathBuf::from("/dev/hidraw1"),
            vid: device::SONY_VID,
            pid: device::DS5_EDGE_PID,
            kind: SonyDeviceKind::DualSenseEdge,
            transport: SourceTransport::Bluetooth,
        };
        let physical_desc = [0x01, 0x02, 0x03];

        let ds_auto = TargetCodec::Ds5UsbAuto.usb_identity(&ds_source, &physical_desc);
        assert_eq!(ds_auto.product_id, device::DS5_PID as u32);
        assert_eq!(ds_auto.report_descriptor, &descriptor::DS_USB_DESCRIPTOR);

        let edge_auto = TargetCodec::Ds5UsbAuto.usb_identity(&edge_source, &physical_desc);
        assert_eq!(edge_auto.product_id, device::DS5_EDGE_PID as u32);
        assert_eq!(edge_auto.report_descriptor, &descriptor::DS_EDGE_USB_DESCRIPTOR);
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
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_set_report(TargetCodec::Ds5UsbAuto, 0x31, &data),
            None
        );
    }

    #[test]
    fn physical_ds5_usb_requests_only_safe_ds5_target_feature_reports() {
        let auto_requests = PhysicalCodec::Ds5Usb.feature_reports_to_cache(TargetCodec::Ds5UsbAuto);
        let forced_requests = PhysicalCodec::Ds5Usb.feature_reports_to_cache(TargetCodec::Ds5UsbForced);

        assert_eq!(auto_requests, [
            PhysicalFeatureReportRequest { report_id: 0x05, size: 41 },
            PhysicalFeatureReportRequest { report_id: 0x20, size: 64 },
        ]);
        assert_eq!(forced_requests, auto_requests);
        assert!(!auto_requests.iter().any(|r| r.report_id == 0x09));
        assert!(PhysicalCodec::Ds5Usb.feature_reports_to_cache(TargetCodec::Ds4Usb).is_empty());
        assert!(PhysicalCodec::Ds5Bt.feature_reports_to_cache(TargetCodec::Ds5UsbAuto).is_empty());
        assert!(PhysicalCodec::Ds5Bt.feature_reports_to_cache(TargetCodec::Ds4Usb).is_empty());
    }

    #[test]
    fn physical_ds5_bt_wraps_ds5_usb_output_with_sequence_and_crc() {
        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MAX_SIZE];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        for (i, byte) in usb[1..].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_add(1);
        }
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        let mut state = PhysicalOutputState::default();

        let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();

        assert_eq!(bt.len(), DS5_BT_OUTPUT_REPORT_SIZE);
        assert_eq!(bt[0], DS5_BT_OUTPUT_REPORT_ID);
        assert_eq!(bt[1], 0x00);
        assert_eq!(bt[2], DS5_BT_OUTPUT_TAG);
        assert_eq!(
            &bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET..DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1],
            &usb[1..]
        );
        assert!(
            bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1..DS5_BT_OUTPUT_CRC_OFFSET]
                .iter()
                .all(|b| *b == 0)
        );
        let crc = u32::from_le_bytes([bt[74], bt[75], bt[76], bt[77]]);
        assert_eq!(crc, ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET]));
    }

    #[test]
    fn physical_ds5_bt_wraps_regular_ds5_usb_output_and_zero_pads_tail() {
        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MIN_SIZE];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        for (i, byte) in usb[1..].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_add(1);
        }
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        let mut state = PhysicalOutputState::default();

        let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();

        assert_eq!(bt[0], DS5_BT_OUTPUT_REPORT_ID);
        assert_eq!(bt[1], 0x00);
        assert_eq!(bt[2], DS5_BT_OUTPUT_TAG);
        assert_eq!(
            &bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET..DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1],
            &usb[1..]
        );
        assert!(
            bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1..DS5_BT_OUTPUT_CRC_OFFSET]
                .iter()
                .all(|b| *b == 0)
        );
        let crc = u32::from_le_bytes([bt[74], bt[75], bt[76], bt[77]]);
        assert_eq!(crc, ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET]));
    }

    #[test]
    fn physical_ds5_bt_wraps_edge_ds5_usb_output() {
        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MAX_SIZE];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        for (i, byte) in usb[1..].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_add(1);
        }
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        let mut state = PhysicalOutputState::default();

        let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();

        assert_eq!(bt[0], DS5_BT_OUTPUT_REPORT_ID);
        assert_eq!(
            &bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET..DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1],
            &usb[1..]
        );
        assert!(
            bt[DS5_BT_OUTPUT_PAYLOAD_OFFSET + usb.len() - 1..DS5_BT_OUTPUT_CRC_OFFSET]
                .iter()
                .all(|b| *b == 0)
        );
        let crc = u32::from_le_bytes([bt[74], bt[75], bt[76], bt[77]]);
        assert_eq!(crc, ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET]));
    }

    #[test]
    fn physical_ds5_bt_output_sequence_increments_and_wraps() {
        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MIN_SIZE];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        let mut state = PhysicalOutputState::default();

        for expected in 0..16u8 {
            let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();
            assert_eq!(bt[1], expected << 4);
        }
        let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();
        assert_eq!(bt[1], 0x00);
    }

    #[test]
    fn physical_ds5_bt_wraps_converted_ds4_output() {
        let mut ds4 = [0u8; 32];
        ds4[0] = 0x05;
        ds4[1] = 0x03;
        ds4[4] = 64;
        ds4[5] = 128;
        ds4[6] = 1;
        ds4[7] = 2;
        ds4[8] = 3;
        let command = TargetCodec::Ds4Usb.decode_output(&ds4).unwrap();
        let mut state = PhysicalOutputState::default();

        let bt = PhysicalCodec::Ds5Bt.encode_output(&command, &mut state).unwrap();

        assert_eq!(bt[0], DS5_BT_OUTPUT_REPORT_ID);
        assert_eq!(bt[3], 0x03);
        assert_eq!(bt[5], 64);
        assert_eq!(bt[6], 128);
        assert_eq!(bt[47], 1);
        assert_eq!(bt[48], 2);
        assert_eq!(bt[49], 3);
    }

    #[test]
    fn physical_ds5_bt_rejects_invalid_ds5_usb_output() {
        let command = TargetCodec::Ds5UsbAuto.decode_output(&[0x02, 0x01]).unwrap();
        let mut state = PhysicalOutputState::default();
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_output(&command, &mut state),
            Err(CodecError::InvalidReport)
        );

        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MAX_SIZE];
        usb[0] = 0x31;
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_output(&command, &mut state),
            Err(CodecError::InvalidReport)
        );

        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MAX_SIZE + 1];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_output(&command, &mut state),
            Err(CodecError::InvalidReport)
        );
    }

    #[test]
    fn target_codec_selects_from_output_device_config() {
        assert_eq!(TargetCodec::from_output_device("auto"), TargetCodec::Ds5UsbAuto);
        assert_eq!(TargetCodec::from_output_device("dualsense"), TargetCodec::Ds5UsbForced);
        assert_eq!(TargetCodec::from_output_device("dualshock4"), TargetCodec::Ds4Usb);
        assert_eq!(TargetCodec::from_output_device("unknown"), TargetCodec::Ds5UsbAuto);
    }
}
