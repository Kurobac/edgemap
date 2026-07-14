use super::*;

pub(super) const INPUT_REPORT_SIZE: usize = 64;
pub(super) const INPUT_REPORT_ID: u8 = 0x01;

pub(super) fn parse_input(data: &[u8]) -> Option<GamepadState> {
    if data.len() < INPUT_REPORT_SIZE || data[0] != INPUT_REPORT_ID {
        return None;
    }

    let mut state = GamepadState::default();
    state.left_stick_x = data[1];
    state.left_stick_y = data[2];
    state.right_stick_x = data[3];
    state.right_stick_y = data[4];
    state.l2_analog = data[5];
    state.r2_analog = data[6];
    state.seq_number = data[7];

    let b0 = data[8];
    state.set_button(Button::Square, b0 & 0x10 != 0);
    state.set_button(Button::Cross, b0 & 0x20 != 0);
    state.set_button(Button::Circle, b0 & 0x40 != 0);
    state.set_button(Button::Triangle, b0 & 0x80 != 0);
    match b0 & 0x0f {
        0 => state.set_button(Button::DpadUp, true),
        1 => {
            state.set_button(Button::DpadUp, true);
            state.set_button(Button::DpadRight, true);
        }
        2 => state.set_button(Button::DpadRight, true),
        3 => {
            state.set_button(Button::DpadDown, true);
            state.set_button(Button::DpadRight, true);
        }
        4 => state.set_button(Button::DpadDown, true),
        5 => {
            state.set_button(Button::DpadDown, true);
            state.set_button(Button::DpadLeft, true);
        }
        6 => state.set_button(Button::DpadLeft, true),
        7 => {
            state.set_button(Button::DpadUp, true);
            state.set_button(Button::DpadLeft, true);
        }
        _ => {}
    }

    let b1 = data[9];
    state.set_button(Button::L1, b1 & 0x01 != 0);
    state.set_button(Button::R1, b1 & 0x02 != 0);
    state.set_button(Button::L2, b1 & 0x04 != 0);
    state.set_button(Button::R2, b1 & 0x08 != 0);
    state.set_button(Button::Create, b1 & 0x10 != 0);
    state.set_button(Button::Options, b1 & 0x20 != 0);
    state.set_button(Button::L3, b1 & 0x40 != 0);
    state.set_button(Button::R3, b1 & 0x80 != 0);

    let b2 = data[10];
    state.set_button(Button::PS, b2 & 0x01 != 0);
    state.set_button(Button::Touchpad, b2 & 0x02 != 0);
    state.set_button(Button::Mic, b2 & 0x04 != 0);
    state.set_button(Button::FnLeft, b2 & 0x10 != 0);
    state.set_button(Button::FnRight, b2 & 0x20 != 0);
    state.set_button(Button::LeftPaddle, b2 & 0x40 != 0);
    state.set_button(Button::RightPaddle, b2 & 0x80 != 0);

    let status0 = data[53];
    state.battery_pct = (status0 & 0x0f).min(10) * 10;
    let charging = (status0 >> 4) & 0x0f;
    state.battery_charging = charging == 0x01 || charging == 0x02;
    state.headphone_connected = data[54] & 0x01 == 0;
    Some(state)
}

fn dpad_value(state: &GamepadState) -> u8 {
    match (
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
    }
}

pub(super) fn apply_state(raw: &mut [u8; INPUT_REPORT_SIZE], state: &GamepadState, seq: u8) {
    raw[0] = INPUT_REPORT_ID;
    raw[1] = state.left_stick_x;
    raw[2] = state.left_stick_y;
    raw[3] = state.right_stick_x;
    raw[4] = state.right_stick_y;
    raw[5] = state.l2_analog;
    raw[6] = state.r2_analog;
    raw[7] = seq;

    let mut b0 = dpad_value(state);
    for (button, mask) in [
        (Button::Square, 0x10),
        (Button::Cross, 0x20),
        (Button::Circle, 0x40),
        (Button::Triangle, 0x80),
    ] {
        if state.button(button) {
            b0 |= mask;
        }
    }
    raw[8] = b0;

    let mut b1 = 0;
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
            b1 |= mask;
        }
    }
    raw[9] = b1;

    let mut b2 = 0;
    for (button, mask) in [
        (Button::PS, 0x01),
        (Button::Touchpad, 0x02),
        (Button::Mic, 0x04),
        (Button::FnLeft, 0x10),
        (Button::FnRight, 0x20),
        (Button::LeftPaddle, 0x40),
        (Button::RightPaddle, 0x80),
    ] {
        if state.button(button) {
            b2 |= mask;
        }
    }
    raw[10] = b2;
    raw[11] &= 0x0f;
}

fn read_i16_le(raw: &[u8; INPUT_REPORT_SIZE], offset: usize) -> i16 {
    i16::from_le_bytes([raw[offset], raw[offset + 1]])
}

pub(super) fn parse_motion(raw: &[u8; INPUT_REPORT_SIZE]) -> MotionFrame {
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

pub(super) fn write_motion(raw: &mut [u8; INPUT_REPORT_SIZE], motion: MotionFrame) {
    for (i, value) in motion.gyro.iter().chain(motion.accel.iter()).enumerate() {
        raw[16 + i * 2..18 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn parse_touchpad_contact(
    raw: &[u8; INPUT_REPORT_SIZE],
    base: usize,
) -> TouchpadContact {
    let contact = raw[base];
    let x = ((raw[base + 2] as u16 & 0x0f) << 8) | raw[base + 1] as u16;
    let y = ((raw[base + 3] as u16) << 4) | ((raw[base + 2] as u16 >> 4) & 0x0f);
    TouchpadContact {
        active: contact & 0x80 == 0,
        id: contact & 0x7f,
        x,
        y,
    }
}

pub(super) fn decode_input(raw: &[u8]) -> CodecResult<ControllerFrame> {
    if raw.len() < INPUT_REPORT_SIZE {
        return Err(CodecError::InvalidReport);
    }
    let mut source = [0u8; INPUT_REPORT_SIZE];
    source.copy_from_slice(&raw[..INPUT_REPORT_SIZE]);
    let state = parse_input(&source).ok_or(CodecError::InvalidReport)?;
    let motion = Some(parse_motion(&source));
    Ok(ControllerFrame {
        state,
        motion,
        source_report: SourceReport::Ds5Usb(source),
    })
}

pub(super) fn encode_input(
    frame: &ControllerFrame,
    seq: u8,
) -> CodecResult<[u8; INPUT_REPORT_SIZE]> {
    match &frame.source_report {
        SourceReport::Ds5Usb(raw) => {
            let mut out = *raw;
            apply_state(&mut out, &frame.state, seq);
            Ok(out)
        }
        SourceReport::Ds5Bt { usb_backing, .. } => {
            let mut out = *usb_backing;
            apply_state(&mut out, &frame.state, seq);
            Ok(out)
        }
    }
}

pub(super) fn decode_output(data: &[u8]) -> Ds5UsbOutput {
    Ds5UsbOutput { raw: data.to_vec() }
}

pub(super) fn fallback_feature_report(report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x05 => Some(vec![
            0x05, 0xff, 0xfc, 0xff, 0xfe, 0xff, 0x83, 0x22, 0x78, 0xdd, 0x92, 0x22, 0x5f, 0xdd,
            0x95, 0x22, 0x6d, 0xdd, 0x1c, 0x02, 0x1c, 0x02, 0xf2, 0x1f, 0xed, 0xdf, 0xe3, 0x20,
            0xda, 0xe0, 0xee, 0x1f, 0xdf, 0xdf, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]),
        0x08 => Some(vec![0u8; 48]),
        0x09 => Some(vec![
            0x09, 0xd4, 0x2f, 0x4b, 0x26, 0x18, 0xc2, 0x08, 0x25, 0x00, 0x1e, 0x00, 0xee, 0x74,
            0xd0, 0xbc, 0x00, 0x00, 0x00, 0x00,
        ]),
        0x0A => Some(vec![0u8; 27]),
        0x20 => Some(vec![
            0x20, 0x4a, 0x75, 0x6e, 0x20, 0x31, 0x39, 0x20, 0x32, 0x30, 0x32, 0x33, 0x31, 0x34,
            0x3a, 0x34, 0x37, 0x3a, 0x33, 0x34, 0x03, 0x00, 0x44, 0x00, 0x08, 0x02, 0x00, 0x01,
            0x36, 0x00, 0x00, 0x01, 0xc1, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x54, 0x01, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x01, 0x00,
            0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
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

pub(super) fn encode_physical_output_from_ds5_usb(output: &Ds5UsbOutput) -> CodecResult<Vec<u8>> {
    Ok(output.as_bytes().to_vec())
}

pub(super) fn encode_physical_output_from_ds4_usb(output: &Ds4UsbOutput) -> CodecResult<Vec<u8>> {
    Ok(ds4_usb::convert_output_to_ds5(output.as_bytes()).to_vec())
}

pub(super) fn decode_feature_report(
    request: PhysicalFeatureReportRequest,
    raw: Vec<u8>,
) -> CodecResult<Vec<u8>> {
    if raw.len() != request.size || raw.first() != Some(&request.report_id) {
        return Err(CodecError::InvalidReport);
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_input() -> [u8; INPUT_REPORT_SIZE] {
        let mut raw = [0u8; INPUT_REPORT_SIZE];
        raw[0] = INPUT_REPORT_ID;
        raw
    }

    fn build_input(state: &GamepadState) -> [u8; INPUT_REPORT_SIZE] {
        let mut raw = raw_input();
        apply_state(&mut raw, state, state.seq_number);
        raw[53] = (state.battery_pct / 10) & 0x0f;
        if state.battery_charging {
            raw[53] |= 0x10;
        }
        raw
    }

    #[test]
    fn test_parse_build_roundtrip() {
        let mut state = GamepadState::default();
        state.set_button(Button::Cross, true);
        state.set_button(Button::L1, true);
        state.left_stick_x = 0x80;
        state.right_stick_y = 0x40;

        let parsed = parse_input(&build_input(&state)).unwrap();
        assert!(parsed.button(Button::Cross));
        assert!(parsed.button(Button::L1));
        assert!(!parsed.button(Button::Square));
        assert_eq!(parsed.left_stick_x, 0x80);
        assert_eq!(parsed.right_stick_y, 0x40);
    }

    #[test]
    fn test_dpad_roundtrip() {
        for (up, down, left, right) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
            (true, false, false, true),
        ] {
            let mut state = GamepadState::default();
            state.set_button(Button::DpadUp, up);
            state.set_button(Button::DpadDown, down);
            state.set_button(Button::DpadLeft, left);
            state.set_button(Button::DpadRight, right);
            let parsed = parse_input(&build_input(&state)).unwrap();
            assert_eq!(parsed.button(Button::DpadUp), up);
            assert_eq!(parsed.button(Button::DpadDown), down);
            assert_eq!(parsed.button(Button::DpadLeft), left);
            assert_eq!(parsed.button(Button::DpadRight), right);
        }
    }

    #[test]
    fn parse_face_buttons() {
        let mut raw = raw_input();
        raw[8] = 0x10;
        let state = parse_input(&raw).unwrap();
        assert!(state.button(Button::Square));
        assert!(!state.button(Button::Cross));
        assert!(!state.button(Button::Circle));
        assert!(!state.button(Button::Triangle));

        raw[8] = 0xa0;
        let state = parse_input(&raw).unwrap();
        assert!(state.button(Button::Cross));
        assert!(state.button(Button::Triangle));
    }

    #[test]
    fn parse_shoulder_stick_buttons() {
        let mut raw = raw_input();
        for (mask, button) in [
            (0x01, Button::L1),
            (0x02, Button::R1),
            (0x04, Button::L2),
            (0x08, Button::R2),
            (0x10, Button::Create),
            (0x20, Button::Options),
            (0x40, Button::L3),
            (0x80, Button::R3),
        ] {
            raw[9] = mask;
            assert!(parse_input(&raw).unwrap().button(button));
        }
    }

    #[test]
    fn parse_system_buttons() {
        let mut raw = raw_input();
        for (mask, button) in [
            (0x01, Button::PS),
            (0x02, Button::Touchpad),
            (0x04, Button::Mic),
        ] {
            raw[10] = mask;
            assert!(parse_input(&raw).unwrap().button(button));
        }
    }

    #[test]
    fn parse_edge_buttons() {
        let mut raw = raw_input();
        for (mask, button) in [
            (0x10, Button::FnLeft),
            (0x20, Button::FnRight),
            (0x40, Button::LeftPaddle),
            (0x80, Button::RightPaddle),
        ] {
            raw[10] = mask;
            assert!(parse_input(&raw).unwrap().button(button));
        }
    }

    #[test]
    fn apply_roundtrip_all_buttons() {
        let mut source = raw_input();
        source[8] = 0xf8;
        source[9] = 0xff;
        source[10] = 0xff;
        let parsed = parse_input(&source).unwrap();
        let mut output = raw_input();
        apply_state(&mut output, &parsed, 0);
        let roundtrip = parse_input(&output).unwrap();

        for button in [
            Button::Square,
            Button::Cross,
            Button::Circle,
            Button::Triangle,
            Button::L1,
            Button::R1,
            Button::L2,
            Button::R2,
            Button::Create,
            Button::Options,
            Button::L3,
            Button::R3,
            Button::PS,
            Button::Touchpad,
            Button::TouchpadLeft,
            Button::TouchpadRight,
            Button::Mic,
            Button::DpadUp,
            Button::DpadDown,
            Button::DpadLeft,
            Button::DpadRight,
            Button::FnLeft,
            Button::FnRight,
            Button::LeftPaddle,
            Button::RightPaddle,
        ] {
            assert_eq!(roundtrip.button(button), parsed.button(button));
        }
    }

    #[test]
    fn byte11_low_nibble_preserved() {
        let mut raw = raw_input();
        raw[11] = 0x07;
        let state = parse_input(&raw).unwrap();
        apply_state(&mut raw, &state, 0);
        assert_eq!(raw[11], 0x07);
    }

    #[test]
    fn sticks_triggers_roundtrip() {
        let mut source = raw_input();
        source[1..7].copy_from_slice(&[0x80, 0x40, 0xff, 0x00, 0xc0, 0x20]);
        let state = parse_input(&source).unwrap();
        let mut output = raw_input();
        apply_state(&mut output, &state, 0);
        let roundtrip = parse_input(&output).unwrap();
        assert_eq!(roundtrip.left_stick_x, 0x80);
        assert_eq!(roundtrip.left_stick_y, 0x40);
        assert_eq!(roundtrip.right_stick_x, 0xff);
        assert_eq!(roundtrip.right_stick_y, 0x00);
        assert_eq!(roundtrip.l2_analog, 0xc0);
        assert_eq!(roundtrip.r2_analog, 0x20);
    }

    #[test]
    fn seq_number_survives_apply() {
        let mut raw = raw_input();
        let state = parse_input(&raw).unwrap();
        apply_state(&mut raw, &state, 0xab);
        assert_eq!(raw[7], 0xab);
    }
}
