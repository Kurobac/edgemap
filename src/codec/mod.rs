use crate::descriptor;
use crate::device::{self, DeviceInfo, SonyDeviceKind, SourceTransport};
use crate::model::{Button, GamepadState};

mod ds4_usb;
mod ds5_bt;
mod ds5_usb;
mod feature;
mod types;

use feature::{report_with_id, DS5_PHYSICAL_FEATURE_REPORTS_TO_CACHE};
pub use feature::{FeatureReportCache, PhysicalFeatureReportRequest};
#[allow(unused_imports)]
pub use types::{ControllerFrame, MotionFrame, SourceReport, TouchpadContact, TouchpadFrame};
pub use types::{Ds4UsbOutput, Ds5UsbOutput, OutputCommand};

#[cfg(test)]
use ds5_bt::{
    ps_crc32, FEATURE_CRC32_SEED as PS_FEATURE_CRC32_SEED, INPUT_CRC32_SEED as PS_INPUT_CRC32_SEED,
    INPUT_CRC_SIZE as DS5_BT_CRC_SIZE, INPUT_REPORT_ID as DS5_BT_INPUT_REPORT_ID,
    INPUT_REPORT_SIZE as DS5_BT_INPUT_REPORT_SIZE, OUTPUT_CRC32_SEED as PS_OUTPUT_CRC32_SEED,
    OUTPUT_CRC_OFFSET as DS5_BT_OUTPUT_CRC_OFFSET,
    OUTPUT_PAYLOAD_OFFSET as DS5_BT_OUTPUT_PAYLOAD_OFFSET,
    OUTPUT_REPORT_ID as DS5_BT_OUTPUT_REPORT_ID, OUTPUT_REPORT_SIZE as DS5_BT_OUTPUT_REPORT_SIZE,
    OUTPUT_TAG as DS5_BT_OUTPUT_TAG, USB_OUTPUT_REPORT_ID as DS5_USB_OUTPUT_REPORT_ID,
    USB_OUTPUT_REPORT_MAX_SIZE as DS5_USB_OUTPUT_REPORT_MAX_SIZE,
    USB_OUTPUT_REPORT_MIN_SIZE as DS5_USB_OUTPUT_REPORT_MIN_SIZE,
};

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
        Self {
            source,
            physical,
            target,
        }
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
            (
                SonyDeviceKind::DualSense | SonyDeviceKind::DualSenseEdge,
                SourceTransport::Bluetooth,
            ) => Self::Ds5Bt,
        }
    }

    pub fn input_report_size(self) -> usize {
        match self {
            Self::Ds5Usb => ds5_usb::INPUT_REPORT_SIZE,
            Self::Ds5Bt => ds5_bt::INPUT_REPORT_SIZE,
        }
    }

    pub fn decode_input(self, raw: &[u8]) -> CodecResult<ControllerFrame> {
        match self {
            Self::Ds5Usb => ds5_usb::decode_input(raw),
            Self::Ds5Bt => ds5_bt::decode_input(raw),
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
    pub fn feature_reports_to_cache(
        self,
        target: TargetCodec,
    ) -> &'static [PhysicalFeatureReportRequest] {
        match (self, target) {
            (Self::Ds5Usb, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                &DS5_PHYSICAL_FEATURE_REPORTS_TO_CACHE
            }
            (Self::Ds5Usb, TargetCodec::Ds4Usb) => &[],
            // DS5 Bluetooth feature reports use the same IDs and total sizes
            // as USB for calibration (0x05) and firmware info (0x20), but the
            // last 4 bytes are a feature CRC32 with seed 0xA3. Hardware tests
            // showed that the useful fields live before that tail. Validate
            // the CRC first but keep the full-size report in the USB target
            // cache; truncating the CRC tail would change the GET_REPORT
            // length expected by hid-playstation.
            (Self::Ds5Bt, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                &DS5_PHYSICAL_FEATURE_REPORTS_TO_CACHE
            }
            (Self::Ds5Bt, TargetCodec::Ds4Usb) => &[],
        }
    }

    pub fn encode_output(
        self,
        command: &OutputCommand,
        state: &mut PhysicalOutputState,
    ) -> CodecResult<Vec<u8>> {
        match (self, command) {
            (Self::Ds5Usb, OutputCommand::Ds5Usb(output)) => {
                ds5_usb::encode_physical_output_from_ds5_usb(output)
            }
            (Self::Ds5Usb, OutputCommand::Ds4Usb(output)) => {
                ds5_usb::encode_physical_output_from_ds4_usb(output)
            }
            (Self::Ds5Bt, OutputCommand::Ds5Usb(output)) => {
                ds5_bt::encode_output_from_ds5_usb(output, state)
            }
            (Self::Ds5Bt, OutputCommand::Ds4Usb(output)) => {
                let ds5 = ds4_usb::convert_output_to_ds5(output.as_bytes());
                ds5_bt::encode_output_from_ds5_usb_bytes(&ds5, state)
            }
        }
    }

    pub fn decode_feature_report(
        self,
        request: PhysicalFeatureReportRequest,
        raw: Vec<u8>,
    ) -> CodecResult<Vec<u8>> {
        match self {
            Self::Ds5Usb => ds5_usb::decode_feature_report(request, raw),
            Self::Ds5Bt => ds5_bt::decode_feature_report(request, raw),
        }
    }

    pub fn encode_set_report(
        self,
        target: TargetCodec,
        report_id: u8,
        data: &[u8],
    ) -> Option<Vec<u8>> {
        match (self, target) {
            (Self::Ds5Usb, TargetCodec::Ds5UsbAuto | TargetCodec::Ds5UsbForced) => {
                Some(report_with_id(report_id, data))
            }
            (Self::Ds5Usb, TargetCodec::Ds4Usb) => None,
            // DS5 BT feature reports advertise the same IDs as USB, but
            // hardware rejects a naive HIDIOCSFEATURE transfer even when a
            // feature CRC is appended. Keep BT SET_REPORT disabled until the
            // actual framing/transport rule is confirmed from hardware traces.
            (Self::Ds5Bt, _) => None,
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

    pub fn encode_input(
        self,
        frame: &ControllerFrame,
        seq: u8,
    ) -> CodecResult<[u8; ds5_usb::INPUT_REPORT_SIZE]> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => ds5_usb::encode_input(frame, seq),
            Self::Ds4Usb => ds4_usb::encode_input(frame, seq),
        }
    }

    pub fn decode_output(self, data: &[u8]) -> CodecResult<OutputCommand> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => {
                Ok(OutputCommand::Ds5Usb(ds5_usb::decode_output(data)))
            }
            Self::Ds4Usb => Ok(OutputCommand::Ds4Usb(ds4_usb::decode_output(data))),
        }
    }

    pub fn seed_feature_reports(self, cache: &mut FeatureReportCache) {
        match self {
            Self::Ds4Usb => ds4_usb::seed_feature_reports(cache),
            Self::Ds5UsbAuto | Self::Ds5UsbForced => {}
        }
    }

    pub fn fallback_feature_report(self, report_id: u8) -> Option<Vec<u8>> {
        match self {
            Self::Ds5UsbAuto | Self::Ds5UsbForced => ds5_usb::fallback_feature_report(report_id),
            Self::Ds4Usb => ds4_usb::fallback_feature_report(report_id),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ds5_usb_raw() -> [u8; ds5_usb::INPUT_REPORT_SIZE] {
        let mut raw = [0u8; ds5_usb::INPUT_REPORT_SIZE];
        raw[0] = ds5_usb::INPUT_REPORT_ID;
        raw
    }

    fn ds5_bt_raw_from_usb(
        usb: &[u8; ds5_usb::INPUT_REPORT_SIZE],
    ) -> [u8; DS5_BT_INPUT_REPORT_SIZE] {
        let mut raw = [0u8; DS5_BT_INPUT_REPORT_SIZE];
        raw[0] = DS5_BT_INPUT_REPORT_ID;
        raw[2..65].copy_from_slice(&usb[1..]);
        let crc = ps_crc32(
            PS_INPUT_CRC32_SEED,
            &raw[..DS5_BT_INPUT_REPORT_SIZE - DS5_BT_CRC_SIZE],
        );
        raw[74..78].copy_from_slice(&crc.to_le_bytes());
        raw
    }

    fn ds5_bt_feature_report(report_id: u8, size: usize) -> Vec<u8> {
        assert!(
            size >= 4,
            "BT feature report test helper requires room for CRC"
        );
        let mut raw = vec![0u8; size];
        raw[0] = report_id;
        for (i, byte) in raw[1..size - 4].iter_mut().enumerate() {
            *byte = (i as u8).wrapping_add(1);
        }
        let crc = ps_crc32(PS_FEATURE_CRC32_SEED, &raw[..size - 4]);
        raw[size - 4..].copy_from_slice(&crc.to_le_bytes());
        raw
    }

    #[test]
    fn ds5_usb_frame_encodes_remapped_state_over_backing_report() {
        let mut raw = ds5_usb_raw();
        raw[11] = 0x07;
        raw[33] = 0x80;

        let mut frame = ds5_usb::decode_input(&raw).unwrap();
        frame.state.set_button(Button::Cross, true);

        let out = ds5_usb::encode_input(&frame, 0x42).unwrap();
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

        let frame = ds5_usb::decode_input(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadLeft));

        raw[34] = 0xC0;
        raw[35] = 0x03;
        let frame = ds5_usb::decode_input(&raw).unwrap();
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

        let frame = ds5_usb::decode_input(&raw).unwrap();
        let touchpad = frame.touchpad().unwrap();

        assert!(touchpad.button);
        assert_eq!(
            touchpad.contacts[0],
            TouchpadContact {
                active: true,
                id: 0x05,
                x: 0x234,
                y: 0x561,
            }
        );
        assert_eq!(
            touchpad.contacts[1],
            TouchpadContact {
                active: false,
                id: 0x06,
                x: 0xA78,
                y: 0xBC9,
            }
        );
    }

    #[test]
    fn touchpad_split_uses_first_active_contact() {
        let mut raw = ds5_usb_raw();
        raw[10] = 0x02;
        raw[33] = 0x80;
        raw[37] = 0x01;
        raw[38] = 0xC0;
        raw[39] = 0x03;

        let frame = ds5_usb::decode_input(&raw).unwrap();
        assert_eq!(frame.touchpad_split_button(), Some(Button::TouchpadRight));
    }

    #[test]
    fn motion_frame_decodes_ds5_usb_raw_axes() {
        let mut raw = ds5_usb_raw();
        let values = [-1000i16, 2000, -3000, 4000, -5000, 6000];
        for (i, value) in values.iter().enumerate() {
            raw[16 + i * 2..18 + i * 2].copy_from_slice(&value.to_le_bytes());
        }

        let frame = ds5_usb::decode_input(&raw).unwrap();
        assert_eq!(
            frame.motion,
            Some(MotionFrame {
                gyro: [-1000, 2000, -3000],
                accel: [4000, -5000, 6000],
            })
        );
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
        let frame = ds5_bt::decode_input(&bt).unwrap();

        assert_eq!(frame.state.left_stick_x, 10);
        assert_eq!(frame.state.left_stick_y, 20);
        assert_eq!(frame.state.right_stick_x, 30);
        assert_eq!(frame.state.right_stick_y, 40);
        assert_eq!(frame.state.l2_analog, 50);
        assert_eq!(frame.state.r2_analog, 60);
        assert!(frame.state.button(Button::Cross));
        assert!(frame.state.button(Button::Touchpad));
        assert!(frame.state.button(Button::LeftPaddle));
        assert_eq!(
            frame.motion,
            Some(MotionFrame {
                gyro: [-123, 456, -789],
                accel: [111, -222, 333],
            })
        );
        assert_eq!(
            frame.touchpad().unwrap().contacts[0],
            TouchpadContact {
                active: true,
                id: 0x07,
                x: 0x234,
                y: 0x561,
            }
        );
    }

    #[test]
    fn ds5_bt_frame_rejects_bad_crc_or_report_shape() {
        let usb = ds5_usb_raw();
        let mut bt = ds5_bt_raw_from_usb(&usb);
        bt[10] ^= 0x01;
        assert!(matches!(
            ds5_bt::decode_input(&bt),
            Err(CodecError::InvalidReport)
        ));

        let mut bt = ds5_bt_raw_from_usb(&usb);
        bt[0] = 0x01;
        assert!(matches!(
            ds5_bt::decode_input(&bt),
            Err(CodecError::InvalidReport)
        ));
        assert!(matches!(
            ds5_bt::decode_input(&bt[..77]),
            Err(CodecError::InvalidReport)
        ));
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
        let ds5 = PhysicalCodec::Ds5Usb
            .encode_output(&command, &mut state)
            .unwrap();

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
        let encoded = PhysicalCodec::Ds5Usb
            .encode_output(&command, &mut state)
            .unwrap();

        assert_eq!(encoded, ds5);
    }

    #[test]
    fn target_codec_selects_expected_input_codec() {
        let mut raw = ds5_usb_raw();
        raw[33] = 0x80;
        let mut frame = ds5_usb::decode_input(&raw).unwrap();
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
        let mut frame = ds5_bt::decode_input(&bt).unwrap();
        frame.state.set_button(Button::Cross, true);

        let ds5 = TargetCodec::Ds5UsbAuto.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds5[0], ds5_usb::INPUT_REPORT_ID);
        assert_eq!(ds5[8] & 0x20, 0x20);

        let ds4 = TargetCodec::Ds4Usb.encode_input(&frame, 0x10).unwrap();
        assert_eq!(ds4[0], ds5_usb::INPUT_REPORT_ID);
        assert_eq!(ds4[5] & 0x20, 0x20);
    }

    #[test]
    fn source_codec_selects_ds5_usb_for_current_sony_devices() {
        for kind in [SonyDeviceKind::DualSense, SonyDeviceKind::DualSenseEdge] {
            let source = SourceCodec::from_device(kind, SourceTransport::Usb);
            assert_eq!(source, SourceCodec::Ds5Usb);
            assert_eq!(source.physical_codec(), PhysicalCodec::Ds5Usb);
            assert_eq!(source.input_report_size(), ds5_usb::INPUT_REPORT_SIZE);
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
        assert_eq!(
            edge_auto.report_descriptor,
            &descriptor::DS_EDGE_USB_DESCRIPTOR
        );
    }

    #[test]
    fn ds4_target_seeds_feature_reports() {
        let mut cache = FeatureReportCache::new();
        TargetCodec::Ds4Usb.seed_feature_reports(&mut cache);

        assert_eq!(cache.get(0x02).unwrap().len(), 37);
        assert_eq!(
            cache.get(0x12).unwrap()[1..7],
            [0x01, 0x00, 0x00, 0x37, 0x13, 0xC0]
        );
        assert_eq!(cache.get(0xA3).unwrap().len(), 49);
    }

    #[test]
    fn ds5_target_fallback_uses_fake_pairing_mac() {
        let data = TargetCodec::Ds5UsbAuto
            .fallback_feature_report(0x09)
            .unwrap();

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
            PhysicalCodec::Ds5Usb.encode_set_report(TargetCodec::Ds5UsbAuto, 0x31, &[0x31, 0x11],),
            Some(vec![0x31, 0x11])
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
    fn physical_ds5_bt_does_not_forward_feature_set_reports_yet() {
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_set_report(TargetCodec::Ds5UsbAuto, 0x08, &[0x08; 48]),
            None
        );
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_set_report(TargetCodec::Ds5UsbAuto, 0x80, &[0x80; 64]),
            None
        );
        assert_eq!(
            PhysicalCodec::Ds5Bt.encode_set_report(TargetCodec::Ds4Usb, 0x08, &[0x08; 48]),
            None
        );
    }

    #[test]
    fn physical_ds5_usb_requests_only_safe_ds5_target_feature_reports() {
        let auto_requests = PhysicalCodec::Ds5Usb.feature_reports_to_cache(TargetCodec::Ds5UsbAuto);
        let forced_requests =
            PhysicalCodec::Ds5Usb.feature_reports_to_cache(TargetCodec::Ds5UsbForced);
        let bt_requests = PhysicalCodec::Ds5Bt.feature_reports_to_cache(TargetCodec::Ds5UsbAuto);

        assert_eq!(
            auto_requests,
            [
                PhysicalFeatureReportRequest {
                    report_id: 0x05,
                    size: 41
                },
                PhysicalFeatureReportRequest {
                    report_id: 0x20,
                    size: 64
                },
            ]
        );
        assert_eq!(forced_requests, auto_requests);
        assert_eq!(bt_requests, auto_requests);
        assert!(!auto_requests.iter().any(|r| r.report_id == 0x09));
        assert!(PhysicalCodec::Ds5Usb
            .feature_reports_to_cache(TargetCodec::Ds4Usb)
            .is_empty());
        assert!(PhysicalCodec::Ds5Bt
            .feature_reports_to_cache(TargetCodec::Ds4Usb)
            .is_empty());
    }

    #[test]
    fn physical_feature_report_decode_validates_usb_shape() {
        let request = PhysicalFeatureReportRequest {
            report_id: 0x05,
            size: 41,
        };
        let mut raw = vec![0u8; 41];
        raw[0] = 0x05;

        assert_eq!(
            PhysicalCodec::Ds5Usb.decode_feature_report(request, raw.clone()),
            Ok(raw)
        );

        let mut wrong_id = vec![0u8; 41];
        wrong_id[0] = 0x20;
        assert_eq!(
            PhysicalCodec::Ds5Usb.decode_feature_report(request, wrong_id),
            Err(CodecError::InvalidReport)
        );
        assert_eq!(
            PhysicalCodec::Ds5Usb.decode_feature_report(request, vec![0x05; 40]),
            Err(CodecError::InvalidReport)
        );
    }

    #[test]
    fn physical_ds5_bt_feature_report_validates_crc_and_keeps_full_size() {
        let request = PhysicalFeatureReportRequest {
            report_id: 0x05,
            size: 41,
        };
        let raw = ds5_bt_feature_report(0x05, 41);

        assert_eq!(
            PhysicalCodec::Ds5Bt.decode_feature_report(request, raw.clone()),
            Ok(raw)
        );

        let mut bad_crc = ds5_bt_feature_report(0x05, 41);
        bad_crc[40] ^= 0x01;
        assert_eq!(
            PhysicalCodec::Ds5Bt.decode_feature_report(request, bad_crc),
            Err(CodecError::InvalidReport)
        );

        let wrong_id = ds5_bt_feature_report(0x20, 41);
        assert_eq!(
            PhysicalCodec::Ds5Bt.decode_feature_report(request, wrong_id),
            Err(CodecError::InvalidReport)
        );

        let wrong_size = ds5_bt_feature_report(0x05, 40);
        assert_eq!(
            PhysicalCodec::Ds5Bt.decode_feature_report(request, wrong_size),
            Err(CodecError::InvalidReport)
        );
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

        let bt = PhysicalCodec::Ds5Bt
            .encode_output(&command, &mut state)
            .unwrap();

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
        assert_eq!(
            crc,
            ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET])
        );
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

        let bt = PhysicalCodec::Ds5Bt
            .encode_output(&command, &mut state)
            .unwrap();

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
        assert_eq!(
            crc,
            ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET])
        );
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

        let bt = PhysicalCodec::Ds5Bt
            .encode_output(&command, &mut state)
            .unwrap();

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
        assert_eq!(
            crc,
            ps_crc32(PS_OUTPUT_CRC32_SEED, &bt[..DS5_BT_OUTPUT_CRC_OFFSET])
        );
    }

    #[test]
    fn physical_ds5_bt_output_sequence_increments_and_wraps() {
        let mut usb = [0u8; DS5_USB_OUTPUT_REPORT_MIN_SIZE];
        usb[0] = DS5_USB_OUTPUT_REPORT_ID;
        let command = TargetCodec::Ds5UsbAuto.decode_output(&usb).unwrap();
        let mut state = PhysicalOutputState::default();

        for expected in 0..16u8 {
            let bt = PhysicalCodec::Ds5Bt
                .encode_output(&command, &mut state)
                .unwrap();
            assert_eq!(bt[1], expected << 4);
        }
        let bt = PhysicalCodec::Ds5Bt
            .encode_output(&command, &mut state)
            .unwrap();
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

        let bt = PhysicalCodec::Ds5Bt
            .encode_output(&command, &mut state)
            .unwrap();

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
        let command = TargetCodec::Ds5UsbAuto
            .decode_output(&[0x02, 0x01])
            .unwrap();
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
        assert_eq!(
            TargetCodec::from_output_device("auto"),
            TargetCodec::Ds5UsbAuto
        );
        assert_eq!(
            TargetCodec::from_output_device("dualsense"),
            TargetCodec::Ds5UsbForced
        );
        assert_eq!(
            TargetCodec::from_output_device("dualshock4"),
            TargetCodec::Ds4Usb
        );
        assert_eq!(
            TargetCodec::from_output_device("unknown"),
            TargetCodec::Ds5UsbAuto
        );
    }
}
