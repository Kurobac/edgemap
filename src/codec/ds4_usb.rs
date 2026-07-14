use super::*;

fn apply_state(raw: &mut [u8; ds5_usb::INPUT_REPORT_SIZE], state: &GamepadState, seq: u8) {
    raw[0] = ds5_usb::INPUT_REPORT_ID;
    raw[1] = state.left_stick_x;
    raw[2] = state.left_stick_y;
    raw[3] = state.right_stick_x;
    raw[4] = state.right_stick_y;

    let mut b5 = match (
        state.button(Button::DpadUp),
        state.button(Button::DpadDown),
        state.button(Button::DpadLeft),
        state.button(Button::DpadRight),
    ) {
        (false, false, false, false) => 8,
        (true, false, false, false) => 0,
        (true, false, false, true) => 1,
        (false, false, false, true) => 2,
        (false, true, false, true) => 3,
        (false, true, false, false) => 4,
        (false, true, true, false) => 5,
        (false, false, true, false) => 6,
        (true, false, true, false) => 7,
        _ => 8,
    };
    for (button, mask) in [
        (Button::Square, 0x10),
        (Button::Cross, 0x20),
        (Button::Circle, 0x40),
        (Button::Triangle, 0x80),
    ] {
        if state.button(button) {
            b5 |= mask;
        }
    }
    raw[5] = b5;

    let mut b6 = 0;
    for (button, mask) in [
        (Button::L1, 0x01),
        (Button::R1, 0x02),
        (Button::L2, 0x04),
        (Button::R2, 0x08),
        (Button::Create, 0x10),
        (Button::Options, 0x20),
        (Button::L3, 0x40),
        (Button::R3, 0x80),
    ] {
        if state.button(button) {
            b6 |= mask;
        }
    }
    raw[6] = b6;

    let mut b7 = (seq & 0x3f) << 2;
    if state.button(Button::PS) {
        b7 |= 0x01;
    }
    if state.button(Button::Touchpad) {
        b7 |= 0x02;
    }
    raw[7] = b7;
    raw[8] = state.l2_analog;
    raw[9] = state.r2_analog;

    let p0 = [raw[33], raw[34], raw[35], raw[36]];
    let p1 = [raw[37], raw[38], raw[39], raw[40]];
    let gyro_accel: [u8; 12] = raw[16..28].try_into().unwrap();
    raw[10..64].fill(0);
    raw[13..25].copy_from_slice(&gyro_accel);
    raw[10..12].copy_from_slice(&(seq as u16).to_le_bytes());
    raw[12] = 0x06;

    let mut active = false;
    let mut write_point = |base: usize, point: [u8; 4]| {
        if point[0] & 0x80 == 0 {
            active = true;
            let y = ((point[3] as u16) << 4) | ((point[2] as u16 >> 4) & 0x0f);
            let scaled_y = ((y as u32) * 942 / 1080) as u16;
            raw[base] = point[0];
            raw[base + 1] = point[1];
            raw[base + 2] = (point[2] & 0x0f) | (((scaled_y & 0x0f) as u8) << 4);
            raw[base + 3] = (scaled_y >> 4) as u8;
        } else {
            raw[base] = 0x80;
        }
    };
    write_point(35, p0);
    write_point(39, p1);
    raw[33] = if active { 1 } else { 0 };
    raw[34] = seq & 0x3f;
    for offset in [44, 47, 53, 56] {
        raw[offset] = 0x80;
    }
    raw[30] = 0x1b;
}

pub(super) fn encode_input(
    frame: &ControllerFrame,
    seq: u8,
) -> CodecResult<[u8; ds5_usb::INPUT_REPORT_SIZE]> {
    match &frame.source_report {
        SourceReport::Ds5Usb(raw) => {
            let mut out = *raw;
            if let Some(motion) = frame.motion {
                ds5_usb::write_motion(&mut out, motion);
            }
            apply_state(&mut out, &frame.state, seq);
            Ok(out)
        }
        SourceReport::Ds5Bt { usb_backing, .. } => {
            let mut out = *usb_backing;
            if let Some(motion) = frame.motion {
                ds5_usb::write_motion(&mut out, motion);
            }
            apply_state(&mut out, &frame.state, seq);
            Ok(out)
        }
    }
}

pub(super) fn decode_output(data: &[u8]) -> Ds4UsbOutput {
    Ds4UsbOutput { raw: data.to_vec() }
}

pub(super) fn convert_output_to_ds5(ds4: &[u8]) -> [u8; 63] {
    let mut ds5 = [0u8; 63];
    ds5[0] = 0x02;

    if ds4.len() < 11 {
        return ds5;
    }
    let offset = if ds4.len() >= 32 && ds4[0] == 0x05 {
        1
    } else {
        0
    };
    if ds4.len() < offset + 11 {
        return ds5;
    }

    let flags = ds4[offset];
    if flags & 0x01 != 0 {
        ds5[1] |= 0x03;
        ds5[3] = ds4[offset + 3];
        ds5[4] = ds4[offset + 4];
    }
    if flags & 0x02 != 0 {
        ds5[2] |= 0x04;
        ds5[45] = ds4[offset + 5];
        ds5[46] = ds4[offset + 6];
        ds5[47] = ds4[offset + 7];
    }
    ds5
}

pub(super) fn seed_feature_reports(cache: &mut FeatureReportCache) {
    let mut calibration = vec![0u8; 37];
    calibration[0] = 0x02;
    let write_u16 = |buf: &mut [u8], offset, value: u16| {
        buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes())
    };
    write_u16(&mut calibration, 7, 1024);
    write_u16(&mut calibration, 9, (-1024i16) as u16);
    write_u16(&mut calibration, 11, 1024);
    write_u16(&mut calibration, 13, (-1024i16) as u16);
    write_u16(&mut calibration, 15, 1024);
    write_u16(&mut calibration, 17, (-1024i16) as u16);
    write_u16(&mut calibration, 19, 1);
    write_u16(&mut calibration, 21, 1);
    write_u16(&mut calibration, 23, 8192);
    write_u16(&mut calibration, 25, (-8192i16) as u16);
    write_u16(&mut calibration, 27, 8192);
    write_u16(&mut calibration, 29, (-8192i16) as u16);
    write_u16(&mut calibration, 31, 8192);
    write_u16(&mut calibration, 33, (-8192i16) as u16);
    cache.insert(0x02, calibration);

    let mut firmware = vec![0u8; 49];
    firmware[0] = 0xA3;
    firmware[1..12].copy_from_slice(b"Aug  3 2013");
    firmware[17..25].copy_from_slice(b"07:01:12");
    write_u16(&mut firmware, 34, 0x0001);
    write_u16(&mut firmware, 36, 0x0331);
    write_u16(&mut firmware, 41, 0x0049);
    firmware[43] = 0x05;
    write_u16(&mut firmware, 46, 0x0380);
    cache.insert(0xA3, firmware);

    let mut mac = vec![0u8; 16];
    mac[0] = 0x12;
    mac[1..7].copy_from_slice(&[0x01, 0x00, 0x00, 0x37, 0x13, 0xC0]);
    mac[7] = 0x08;
    mac[8] = 0x25;
    mac[9] = 0x00;
    cache.insert(0x12, mac);
}

pub(super) fn fallback_feature_report(report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x81 => Some(vec![0x81, 0, 0, 0, 0, 0, 0]),
        _ => None,
    }
}
