use super::*;

pub(super) const INPUT_REPORT_ID: u8 = 0x31;
pub(super) const INPUT_REPORT_SIZE: usize = 78;
const INPUT_COMMON_OFFSET: usize = 2;
pub(super) const INPUT_CRC_SIZE: usize = 4;
pub(super) const USB_OUTPUT_REPORT_ID: u8 = 0x02;
pub(super) const USB_OUTPUT_REPORT_MIN_SIZE: usize = 48;
pub(super) const USB_OUTPUT_REPORT_MAX_SIZE: usize = 64;
pub(super) const OUTPUT_REPORT_ID: u8 = 0x31;
pub(super) const OUTPUT_REPORT_SIZE: usize = OUTPUT_CRC_OFFSET + 4;
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
    bt[2] = OUTPUT_TAG;
    let payload_len = usb.len() - 1;
    bt[OUTPUT_PAYLOAD_OFFSET..OUTPUT_PAYLOAD_OFFSET + payload_len].copy_from_slice(&usb[1..]);

    finish_output(&mut bt, state);
    Ok(bt)
}

fn finish_output(report: &mut [u8], state: &mut PhysicalOutputState) {
    report[1] = (state.ds5_bt_seq & 0x0F) << 4;
    state.ds5_bt_seq = (state.ds5_bt_seq + 1) & 0x0F;
    let crc_offset = report.len() - 4;
    let crc = ps_crc32(OUTPUT_CRC32_SEED, &report[..crc_offset]);
    report[crc_offset..].copy_from_slice(&crc.to_le_bytes());
}

pub(super) fn encode_haptics(frame: &HapticsFrame, state: &mut PhysicalOutputState) -> Vec<u8> {
    // SAxense's 0x32 container: control 0x11 followed by PCM 0x12.
    // https://github.com/egormanga/SAxense
    // 142 bytes INCLUDE the report ID. 0xFE keeps microphone streaming off.
    let mut bt = vec![0; 142];
    bt[0] = 0x32;
    bt[2..11].copy_from_slice(&[
        0x91,
        7,
        0xFE,
        0,
        0,
        0,
        0,
        0xFF,
        state.ds5_bt_haptics_counter,
    ]);
    bt[11..13].copy_from_slice(&[0x92, HapticsFrame::SAMPLES as u8]);
    for (dest, sample) in bt[13..77].iter_mut().zip(frame.0) {
        *dest = sample as u8;
    }
    state.ds5_bt_haptics_counter = state.ds5_bt_haptics_counter.wrapping_add(1);
    finish_output(&mut bt, state);
    bt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haptics_packet_preserves_signed_pcm_and_matches_independent_crc() {
        let mut frame = HapticsFrame(std::array::from_fn(|i| i as i8 - 32));
        frame.0[0] = i8::MIN;
        frame.0[1] = i8::MAX;
        let mut state = PhysicalOutputState::default();
        let packet = encode_haptics(&frame, &mut state);
        assert_eq!(packet.len(), 142);
        assert_eq!(
            &packet[..13],
            &[0x32, 0, 0x91, 7, 0xFE, 0, 0, 0, 0, 0xFF, 0, 0x92, 64]
        );
        assert_eq!(packet[13..77], frame.0.map(|sample| sample as u8));
        assert!(packet[77..138].iter().all(|&b| b == 0));
        // Python zlib.crc32(b"\xa2" + packet[:138]), independent of ps_crc32.
        assert_eq!(
            u32::from_le_bytes(packet[138..].try_into().unwrap()),
            0x70D272B8
        );
    }

    #[test]
    fn control_and_haptics_share_sequence_but_not_audio_counter() {
        let mut usb = [0; 48];
        usb[0] = 2;
        usb[1] = 0x03;
        usb[39] = 0x04;
        let mut state = PhysicalOutputState::default();
        for i in 0..260u16 {
            let control = encode_output_from_ds5_usb_bytes(&usb, &mut state).unwrap();
            assert_eq!(control[1], ((i * 2) as u8 & 15) << 4);
            assert_eq!(&control[3..50], &usb[1..]);
            let packet = encode_haptics(&HapticsFrame::SILENCE, &mut state);
            assert_eq!(packet[1], ((i * 2 + 1) as u8 & 15) << 4);
            assert_eq!(packet[10], i as u8);
            assert_eq!(
                ps_crc32(0xA2, &packet[..138]),
                u32::from_le_bytes(packet[138..].try_into().unwrap())
            );
        }
    }

    #[test]
    fn pcm_is_not_encoded_as_usb_hid_output() {
        let mut state = PhysicalOutputState::default();
        let command = OutputCommand::Haptics(HapticsFrame::SILENCE);
        assert_eq!(
            PhysicalCodec::Ds5Usb.encode_output(&command, &mut state),
            Err(CodecError::UnsupportedOutput)
        );
        assert_eq!(
            PhysicalCodec::Ds5Bt
                .encode_output(&command, &mut state)
                .unwrap()[1],
            0
        );
    }
}
