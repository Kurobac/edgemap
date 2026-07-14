use super::*;

pub(super) const INPUT_REPORT_ID: u8 = 0x31;
pub(super) const INPUT_REPORT_SIZE: usize = 78;
const INPUT_COMMON_OFFSET: usize = 2;
pub(super) const INPUT_CRC_SIZE: usize = 4;
pub(super) const USB_OUTPUT_REPORT_ID: u8 = 0x02;
pub(super) const USB_OUTPUT_REPORT_MIN_SIZE: usize = 48;
pub(super) const USB_OUTPUT_REPORT_MAX_SIZE: usize = 64;
pub(super) const OUTPUT_REPORT_ID: u8 = 0x31;
pub(super) const OUTPUT_REPORT_SIZE: usize = 78;
pub(super) const OUTPUT_TAG: u8 = 0x10;
pub(super) const OUTPUT_PAYLOAD_OFFSET: usize = 3;
pub(super) const OUTPUT_CRC_OFFSET: usize = 74;
pub(super) const INPUT_CRC32_SEED: u8 = 0xA1;
pub(super) const OUTPUT_CRC32_SEED: u8 = 0xA2;
pub(super) const FEATURE_CRC32_SEED: u8 = 0xA3;

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

pub(super) fn ps_crc32(seed: u8, data: &[u8]) -> u32 {
    let crc = crc32_le_update(0xFFFF_FFFF, &[seed]);
    !crc32_le_update(crc, data)
}

fn check_ps_crc32(seed: u8, data: &[u8], expected: u32) -> bool {
    ps_crc32(seed, data) == expected
}

fn to_usb_backing(raw: &[u8; INPUT_REPORT_SIZE]) -> [u8; ds5_usb::INPUT_REPORT_SIZE] {
    let mut usb = [0u8; ds5_usb::INPUT_REPORT_SIZE];
    usb[0] = ds5_usb::INPUT_REPORT_ID;
    usb[1..].copy_from_slice(
        &raw[INPUT_COMMON_OFFSET..INPUT_COMMON_OFFSET + ds5_usb::INPUT_REPORT_SIZE - 1],
    );
    usb
}

pub(super) fn decode_input(raw: &[u8]) -> CodecResult<ControllerFrame> {
    if raw.len() < INPUT_REPORT_SIZE || raw[0] != INPUT_REPORT_ID {
        return Err(CodecError::InvalidReport);
    }

    let mut source = [0u8; INPUT_REPORT_SIZE];
    source.copy_from_slice(&raw[..INPUT_REPORT_SIZE]);
    let crc_offset = INPUT_REPORT_SIZE - INPUT_CRC_SIZE;
    let expected_crc = u32::from_le_bytes([
        source[crc_offset],
        source[crc_offset + 1],
        source[crc_offset + 2],
        source[crc_offset + 3],
    ]);
    if !check_ps_crc32(INPUT_CRC32_SEED, &source[..crc_offset], expected_crc) {
        return Err(CodecError::InvalidReport);
    }

    let usb_backing = to_usb_backing(&source);
    let state = ds5_usb::parse_input(&usb_backing).ok_or(CodecError::InvalidReport)?;
    let motion = Some(ds5_usb::parse_motion(&usb_backing));
    Ok(ControllerFrame {
        state,
        motion,
        source_report: SourceReport::Ds5Bt { usb_backing },
    })
}

pub(super) fn decode_feature_report(
    request: PhysicalFeatureReportRequest,
    raw: Vec<u8>,
) -> CodecResult<Vec<u8>> {
    if raw.len() != request.size || raw.first() != Some(&request.report_id) {
        return Err(CodecError::InvalidReport);
    }
    let crc_offset = raw.len() - 4;
    let expected_crc = u32::from_le_bytes([
        raw[crc_offset],
        raw[crc_offset + 1],
        raw[crc_offset + 2],
        raw[crc_offset + 3],
    ]);
    if !check_ps_crc32(FEATURE_CRC32_SEED, &raw[..crc_offset], expected_crc) {
        return Err(CodecError::InvalidReport);
    }
    Ok(raw)
}

pub(super) fn encode_output_from_ds5_usb(
    output: &Ds5UsbOutput,
    state: &mut PhysicalOutputState,
) -> CodecResult<Vec<u8>> {
    encode_output_from_ds5_usb_bytes(output.as_bytes(), state)
}

pub(super) fn encode_output_from_ds5_usb_bytes(
    usb: &[u8],
    state: &mut PhysicalOutputState,
) -> CodecResult<Vec<u8>> {
    if usb.len() < USB_OUTPUT_REPORT_MIN_SIZE
        || usb.len() > USB_OUTPUT_REPORT_MAX_SIZE
        || usb[0] != USB_OUTPUT_REPORT_ID
    {
        return Err(CodecError::InvalidReport);
    }

    let mut bt = vec![0u8; OUTPUT_REPORT_SIZE];
    bt[0] = OUTPUT_REPORT_ID;
    bt[1] = (state.ds5_bt_seq & 0x0F) << 4;
    bt[2] = OUTPUT_TAG;
    let payload_len = usb.len() - 1;
    bt[OUTPUT_PAYLOAD_OFFSET..OUTPUT_PAYLOAD_OFFSET + payload_len].copy_from_slice(&usb[1..]);

    state.ds5_bt_seq = (state.ds5_bt_seq + 1) & 0x0F;

    let crc = ps_crc32(OUTPUT_CRC32_SEED, &bt[..OUTPUT_CRC_OFFSET]);
    bt[OUTPUT_CRC_OFFSET..OUTPUT_CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
    Ok(bt)
}
