use std::fmt;

// DS5/DS4 USB report helpers. These are wire-format routines, not a
// transport-neutral HID model; add BT-specific codecs before reusing them for
// Bluetooth report layouts.
pub const USB_INPUT_REPORT_SIZE: usize = 64;
pub const USB_INPUT_REPORT_ID: u8 = 0x01;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Button {
    Square = 0,
    Cross,
    Circle,
    Triangle,
    L1,
    R1,
    L2,
    R2,
    Create,
    Options,
    L3,
    R3,
    PS,
    Touchpad,
    TouchpadLeft,
    TouchpadRight,
    Mic,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    FnLeft,
    FnRight,
    LeftPaddle,
    RightPaddle,
    L2Analog,
    R2Analog,
}

pub const BUTTON_COUNT: usize = 27;

impl Button {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_lowercase().as_str() {
            "square" => Self::Square,
            "cross" => Self::Cross,
            "circle" => Self::Circle,
            "triangle" => Self::Triangle,
            "l1" => Self::L1,
            "r1" => Self::R1,
            "l2" => Self::L2,
            "r2" => Self::R2,
            "create" => Self::Create,
            "options" => Self::Options,
            "l3" => Self::L3,
            "r3" => Self::R3,
            "ps" => Self::PS,
            "touchpad" => Self::Touchpad,
            "touchpad_left" => Self::TouchpadLeft,
            "touchpad_right" => Self::TouchpadRight,
            "mic" => Self::Mic,
            "dpad_up" => Self::DpadUp,
            "dpad_down" => Self::DpadDown,
            "dpad_left" => Self::DpadLeft,
            "dpad_right" => Self::DpadRight,
            "left_fn" => Self::FnLeft,
            "right_fn" => Self::FnRight,
            "left_paddle" => Self::LeftPaddle,
            "right_paddle" => Self::RightPaddle,
            "l2_analog" => Self::L2Analog,
            "r2_analog" => Self::R2Analog,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Cross => "cross",
            Self::Circle => "circle",
            Self::Triangle => "triangle",
            Self::L1 => "l1",
            Self::R1 => "r1",
            Self::L2 => "l2",
            Self::R2 => "r2",
            Self::Create => "create",
            Self::Options => "options",
            Self::L3 => "l3",
            Self::R3 => "r3",
            Self::PS => "ps",
            Self::Touchpad => "touchpad",
            Self::TouchpadLeft => "touchpad_left",
            Self::TouchpadRight => "touchpad_right",
            Self::Mic => "mic",
            Self::DpadUp => "dpad_up",
            Self::DpadDown => "dpad_down",
            Self::DpadLeft => "dpad_left",
            Self::DpadRight => "dpad_right",
            Self::FnLeft => "left_fn",
            Self::FnRight => "right_fn",
            Self::LeftPaddle => "left_paddle",
            Self::RightPaddle => "right_paddle",
            Self::L2Analog => "l2_analog",
            Self::R2Analog => "r2_analog",
        }
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, Clone, Default)]
pub struct GamepadState {
    pub buttons: [bool; BUTTON_COUNT],
    pub left_stick_x: u8,
    pub left_stick_y: u8,
    pub right_stick_x: u8,
    pub right_stick_y: u8,
    pub l2_analog: u8,
    pub r2_analog: u8,
    pub seq_number: u8,
    pub battery_pct: u8,
    pub battery_charging: bool,
    pub headphone_connected: bool,
}

impl GamepadState {
    pub fn button(&self, btn: Button) -> bool {
        self.buttons[btn as usize]
    }

    pub fn set_button(&mut self, btn: Button, pressed: bool) {
        self.buttons[btn as usize] = pressed;
    }
}

pub fn parse_input_report(data: &[u8]) -> Option<GamepadState> {
    if data.len() < USB_INPUT_REPORT_SIZE {
        return None;
    }
    if data[0] != USB_INPUT_REPORT_ID {
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
    let dpad_val = b0 & 0x0F;
    state.set_button(Button::Square, b0 & 0x10 != 0);
    state.set_button(Button::Cross, b0 & 0x20 != 0);
    state.set_button(Button::Circle, b0 & 0x40 != 0);
    state.set_button(Button::Triangle, b0 & 0x80 != 0);

    match dpad_val {
        0 => {
            state.set_button(Button::DpadUp, true);
        }
        1 => {
            state.set_button(Button::DpadUp, true);
            state.set_button(Button::DpadRight, true);
        }
        2 => {
            state.set_button(Button::DpadRight, true);
        }
        3 => {
            state.set_button(Button::DpadDown, true);
            state.set_button(Button::DpadRight, true);
        }
        4 => {
            state.set_button(Button::DpadDown, true);
        }
        5 => {
            state.set_button(Button::DpadDown, true);
            state.set_button(Button::DpadLeft, true);
        }
        6 => {
            state.set_button(Button::DpadLeft, true);
        }
        7 => {
            state.set_button(Button::DpadUp, true);
            state.set_button(Button::DpadLeft, true);
        }
        _ => {} // dpad center
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
    // Edge buttons in vendor usage 0x21 bits 0-3 (byte 10 bits 4-7)
    state.set_button(Button::FnLeft, b2 & 0x10 != 0);
    state.set_button(Button::FnRight, b2 & 0x20 != 0);
    state.set_button(Button::LeftPaddle, b2 & 0x40 != 0);
    state.set_button(Button::RightPaddle, b2 & 0x80 != 0);

    let status0 = data[53];
    state.battery_pct = (status0 & 0x0F).min(10) * 10;
    let charging = (status0 >> 4) & 0x0F;
    state.battery_charging = charging == 0x01 || charging == 0x02;

    let status1 = data[54];
    state.headphone_connected = status1 & 0x01 == 0;

    Some(state)
}

#[allow(dead_code)]
pub fn build_input_report(state: &GamepadState) -> [u8; USB_INPUT_REPORT_SIZE] {
    let mut data = [0u8; USB_INPUT_REPORT_SIZE];
    data[0] = USB_INPUT_REPORT_ID;

    data[1] = state.left_stick_x;
    data[2] = state.left_stick_y;
    data[3] = state.right_stick_x;
    data[4] = state.right_stick_y;
    data[5] = state.l2_analog;
    data[6] = state.r2_analog;
    data[7] = state.seq_number;

    let mut b0: u8 = 0;
    if state.button(Button::Square) {
        b0 |= 0x10;
    }
    if state.button(Button::Cross) {
        b0 |= 0x20;
    }
    if state.button(Button::Circle) {
        b0 |= 0x40;
    }
    if state.button(Button::Triangle) {
        b0 |= 0x80;
    }

    let up = state.button(Button::DpadUp);
    let down = state.button(Button::DpadDown);
    let left = state.button(Button::DpadLeft);
    let right = state.button(Button::DpadRight);
    let dpad_val: u8 = match (up, down, left, right) {
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
    b0 |= dpad_val & 0x0F;
    data[8] = b0;

    let mut b1: u8 = 0;
    if state.button(Button::L1) {
        b1 |= 0x01;
    }
    if state.button(Button::R1) {
        b1 |= 0x02;
    }
    if state.button(Button::L2) {
        b1 |= 0x04;
    }
    if state.button(Button::R2) {
        b1 |= 0x08;
    }
    if state.button(Button::Create) {
        b1 |= 0x10;
    }
    if state.button(Button::Options) {
        b1 |= 0x20;
    }
    if state.button(Button::L3) {
        b1 |= 0x40;
    }
    if state.button(Button::R3) {
        b1 |= 0x80;
    }
    data[9] = b1;

    let mut b2: u8 = 0;
    if state.button(Button::PS) {
        b2 |= 0x01;
    }
    if state.button(Button::Touchpad) {
        b2 |= 0x02;
    }
    if state.button(Button::Mic) {
        b2 |= 0x04;
    }
    // Edge buttons in vendor usage 0x21 bits 0-3
    if state.button(Button::FnLeft) { b2 |= 0x10; }
    if state.button(Button::FnRight) { b2 |= 0x20; }
    if state.button(Button::LeftPaddle) { b2 |= 0x40; }
    if state.button(Button::RightPaddle) { b2 |= 0x80; }
    data[10] = b2;

    data[11] = 0;

    data[53] = (state.battery_pct / 10) & 0x0F;
    let charging_bits: u8 = if state.battery_charging { 0x10 } else { 0x00 };
    data[53] |= charging_bits;

    data
}

pub fn apply_state_to_report(raw: &mut [u8; 64], state: &GamepadState, seq: u8) {
    raw[0] = USB_INPUT_REPORT_ID;
    raw[1] = state.left_stick_x;
    raw[2] = state.left_stick_y;
    raw[3] = state.right_stick_x;
    raw[4] = state.right_stick_y;
    raw[5] = state.l2_analog;
    raw[6] = state.r2_analog;
    raw[7] = seq;

    let mut b0: u8 = 0;
    if state.button(Button::Square) { b0 |= 0x10; }
    if state.button(Button::Cross) { b0 |= 0x20; }
    if state.button(Button::Circle) { b0 |= 0x40; }
    if state.button(Button::Triangle) { b0 |= 0x80; }

    let up = state.button(Button::DpadUp);
    let down = state.button(Button::DpadDown);
    let left = state.button(Button::DpadLeft);
    let right = state.button(Button::DpadRight);
    let dpad_val: u8 = match (up, down, left, right) {
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
    b0 |= dpad_val & 0x0F;
    raw[8] = b0;

    let mut b1: u8 = 0;
    if state.button(Button::L1) { b1 |= 0x01; }
    if state.button(Button::R1) { b1 |= 0x02; }
    if state.button(Button::L2) { b1 |= 0x04; }
    if state.button(Button::R2) { b1 |= 0x08; }
    if state.button(Button::Create) { b1 |= 0x10; }
    if state.button(Button::Options) { b1 |= 0x20; }
    if state.button(Button::L3) { b1 |= 0x40; }
    if state.button(Button::R3) { b1 |= 0x80; }
    raw[9] = b1;

    let mut b2: u8 = 0;
    if state.button(Button::PS) { b2 |= 0x01; }
    if state.button(Button::Touchpad) { b2 |= 0x02; }
    if state.button(Button::Mic) { b2 |= 0x04; }
    // Edge buttons in vendor usage 0x21 bits 0-3 (byte 10 high nibble).
    // DS5 targets keep writing them because the descriptor advertises these
    // usages; games may still ignore them. DS4 target encoding has no matching
    // fields, so Edge-only passthrough state is dropped there.
    if state.button(Button::FnLeft) { b2 |= 0x10; }
    if state.button(Button::FnRight) { b2 |= 0x20; }
    if state.button(Button::LeftPaddle) { b2 |= 0x40; }
    if state.button(Button::RightPaddle) { b2 |= 0x80; }
    raw[10] = b2;

    let b3: u8 = raw[11] & 0x0F;
    raw[11] = b3;
}

pub fn apply_state_to_ds4_report(raw: &mut [u8; 64], state: &GamepadState, seq: u8) {
    // DS4 target conversion currently consumes a DS5 USB backing report for
    // touchpad and motion passthrough. Revisit this when a non-USB source
    // provides equivalent data through ControllerFrame abstractions.
    raw[0] = USB_INPUT_REPORT_ID;
    raw[1] = state.left_stick_x;
    raw[2] = state.left_stick_y;
    raw[3] = state.right_stick_x;
    raw[4] = state.right_stick_y;

    let mut b5: u8 = 0;
    if state.button(Button::Square) { b5 |= 0x10; }
    if state.button(Button::Cross) { b5 |= 0x20; }
    if state.button(Button::Circle) { b5 |= 0x40; }
    if state.button(Button::Triangle) { b5 |= 0x80; }

    let up = state.button(Button::DpadUp);
    let down = state.button(Button::DpadDown);
    let left = state.button(Button::DpadLeft);
    let right = state.button(Button::DpadRight);
    let dpad_val: u8 = match (up, down, left, right) {
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
    b5 |= dpad_val & 0x0F;
    raw[5] = b5;

    let mut b6: u8 = 0;
    if state.button(Button::L1) { b6 |= 0x01; }
    if state.button(Button::R1) { b6 |= 0x02; }
    if state.button(Button::L2) { b6 |= 0x04; }
    if state.button(Button::R2) { b6 |= 0x08; }
    if state.button(Button::Create) { b6 |= 0x10; }
    if state.button(Button::Options) { b6 |= 0x20; }
    if state.button(Button::L3) { b6 |= 0x40; }
    if state.button(Button::R3) { b6 |= 0x80; }
    raw[6] = b6;

    let mut b7: u8 = (seq & 0x3F) << 2;
    if state.button(Button::PS) { b7 |= 0x01; }
    if state.button(Button::Touchpad) { b7 |= 0x02; }
    raw[7] = b7;

    raw[8] = state.l2_analog;
    raw[9] = state.r2_analog;

    let p0_contact  = raw[33];
    let p0_x_lo     = raw[34];
    let p0_x_hi_ylo = raw[35];
    let p0_y_hi     = raw[36];
    let p1_contact  = raw[37];
    let p1_x_lo     = raw[38];
    let p1_x_hi_ylo = raw[39];
    let p1_y_hi     = raw[40];

    let gyro_accel: [u8; 12] = raw[16..28].try_into().unwrap();

    raw[10..64].fill(0);

    raw[13..25].copy_from_slice(&gyro_accel);
    raw[10..12].copy_from_slice(&(seq as u16).to_le_bytes());
    raw[12] = 0x06;

    let mut active = false;
    let mut write_point = |base: usize, contact: u8, x_lo: u8, comb: u8, y_hi: u8| {
        if contact & 0x80 == 0 {
            active = true;
            let y_lo = (comb >> 4) & 0x0F;
            let y = ((y_hi as u16) << 4) | (y_lo as u16);
            let ys = ((y as u32) * 942 / 1080) as u16;
            raw[base]     = contact;
            raw[base + 1] = x_lo;
            raw[base + 2] = (comb & 0x0F) | (((ys & 0x0F) as u8) << 4);
            raw[base + 3] = (ys >> 4) as u8;
        } else {
            raw[base] = 0x80;
        }
    };

    write_point(35, p0_contact, p0_x_lo, p0_x_hi_ylo, p0_y_hi);
    write_point(39, p1_contact, p1_x_lo, p1_x_hi_ylo, p1_y_hi);

    raw[33] = if active { 1 } else { 0 };
    raw[34] = seq & 0x3F;

    for &off in &[44, 47, 53, 56] {
        raw[off] = 0x80;
    }

    raw[30] = 0x1B;
}

/// Convert DS4 output report (32 bytes, report ID 0x05) to
/// DS5 output report (63 bytes, report ID 0x02) for compatible vibration.
pub fn convert_ds4_output_to_ds5(ds4: &[u8]) -> [u8; 63] {
    let mut ds5 = [0u8; 63];
    ds5[0] = 0x02;

    if ds4.len() < 11 {
        return ds5;
    }

    let off = if ds4.len() >= 32 && ds4[0] == 0x05 { 1 } else { 0 };
    if ds4.len() < off + 11 {
        return ds5;
    }

    let flags = ds4[off];

    if flags & 0x01 != 0 {
        ds5[1] |= 0x03;
        ds5[3] = ds4[off + 3];
        ds5[4] = ds4[off + 4];
    }

    if flags & 0x02 != 0 {
        ds5[2] |= 0x04;
        ds5[45] = ds4[off + 5];
        ds5[46] = ds4[off + 6];
        ds5[47] = ds4[off + 7];
    }

    ds5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_buf() -> [u8; 64] {
        let mut buf = [0u8; 64];
        buf[0] = 0x01;
        buf
    }

    #[test]
    fn test_parse_build_roundtrip() {
        let mut state = GamepadState::default();
        state.set_button(Button::Cross, true);
        state.set_button(Button::L1, true);
        state.left_stick_x = 0x80;
        state.right_stick_y = 0x40;

        let report = build_input_report(&state);
        let parsed = parse_input_report(&report).unwrap();

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
            let report = build_input_report(&state);
            let parsed = parse_input_report(&report).unwrap();
            assert_eq!(parsed.button(Button::DpadUp), up);
            assert_eq!(parsed.button(Button::DpadDown), down);
            assert_eq!(parsed.button(Button::DpadLeft), left);
            assert_eq!(parsed.button(Button::DpadRight), right);
        }
    }

    // --- byte position verification ---

    #[test]
    fn parse_face_buttons() {
        let mut buf = raw_buf();
        buf[8] = 0x10;
        let s = parse_input_report(&buf).unwrap();
        assert!(s.button(Button::Square));
        assert!(!s.button(Button::Cross));
        assert!(!s.button(Button::Circle));
        assert!(!s.button(Button::Triangle));

        buf[8] = 0xA0; // Cross + Triangle
        let s = parse_input_report(&buf).unwrap();
        assert!(s.button(Button::Cross));
        assert!(s.button(Button::Triangle));
    }

    #[test]
    fn parse_shoulder_stick_buttons() {
        let mut buf = raw_buf();
        buf[9] = 0x01;
        assert!(parse_input_report(&buf).unwrap().button(Button::L1));
        buf[9] = 0x02;
        assert!(parse_input_report(&buf).unwrap().button(Button::R1));
        buf[9] = 0x04;
        assert!(parse_input_report(&buf).unwrap().button(Button::L2));
        buf[9] = 0x08;
        assert!(parse_input_report(&buf).unwrap().button(Button::R2));
        buf[9] = 0x10;
        assert!(parse_input_report(&buf).unwrap().button(Button::Create));
        buf[9] = 0x20;
        assert!(parse_input_report(&buf).unwrap().button(Button::Options));
        buf[9] = 0x40;
        assert!(parse_input_report(&buf).unwrap().button(Button::L3));
        buf[9] = 0x80;
        assert!(parse_input_report(&buf).unwrap().button(Button::R3));
    }

    #[test]
    fn parse_system_buttons() {
        let mut buf = raw_buf();
        buf[10] = 0x01;
        assert!(parse_input_report(&buf).unwrap().button(Button::PS));
        buf[10] = 0x02;
        assert!(parse_input_report(&buf).unwrap().button(Button::Touchpad));
        buf[10] = 0x04;
        assert!(parse_input_report(&buf).unwrap().button(Button::Mic));
    }

    #[test]
    fn parse_edge_buttons() {
        let mut buf = raw_buf();
        buf[10] = 0x10;
        assert!(parse_input_report(&buf).unwrap().button(Button::FnLeft));
        buf[10] = 0x20;
        assert!(parse_input_report(&buf).unwrap().button(Button::FnRight));
        buf[10] = 0x40;
        assert!(parse_input_report(&buf).unwrap().button(Button::LeftPaddle));
        buf[10] = 0x80;
        assert!(parse_input_report(&buf).unwrap().button(Button::RightPaddle));
    }

    #[test]
    fn apply_roundtrip_all_buttons() {
        let mut src = raw_buf();
        // set every standard button in byte 8-10
        src[8] = 0xF8; // dpad=0, Square+Cross+Circle+Triangle
        src[9] = 0xFF; // all shoulder/stick buttons + create/options
        src[10] = 0xFF; // PS+Touch+Mic + all Edge buttons

        let parsed = parse_input_report(&src).unwrap();
        let mut dst = raw_buf();
        apply_state_to_report(&mut dst, &parsed, 0);
        let r2 = parse_input_report(&dst).unwrap();

        for btn in ALL_BUTTONS {
            assert_eq!(r2.button(*btn), parsed.button(*btn),
                "roundtrip mismatch for {}", btn.name());
        }
    }

    #[test]
    fn byte11_low_nibble_preserved() {
        let mut buf = raw_buf();
        buf[11] = 0x07; // random low nibble value
        let s = parse_input_report(&buf).unwrap();
        apply_state_to_report(&mut buf, &s, 0);
        assert_eq!(buf[11] & 0x0F, 0x07); // low nibble preserved
        assert_eq!(buf[11] & 0xF0, 0x00); // high nibble was not set
    }

    #[test]
    fn sticks_triggers_roundtrip() {
        let mut src = raw_buf();
        src[1] = 0x80; src[2] = 0x40; // left stick
        src[3] = 0xFF; src[4] = 0x00; // right stick
        src[5] = 0xC0; src[6] = 0x20; // triggers

        let s = parse_input_report(&src).unwrap();
        let mut dst = raw_buf();
        apply_state_to_report(&mut dst, &s, 0);
        let r2 = parse_input_report(&dst).unwrap();

        assert_eq!(r2.left_stick_x, 0x80);
        assert_eq!(r2.left_stick_y, 0x40);
        assert_eq!(r2.right_stick_x, 0xFF);
        assert_eq!(r2.right_stick_y, 0x00);
        assert_eq!(r2.l2_analog, 0xC0);
        assert_eq!(r2.r2_analog, 0x20);
    }

    #[test]
    fn seq_number_survives_apply() {
        let mut buf = raw_buf();
        let s = parse_input_report(&buf).unwrap();
        apply_state_to_report(&mut buf, &s, 0xAB);
        assert_eq!(buf[7], 0xAB);
    }

    static ALL_BUTTONS: &[Button] = &[
        Button::Square, Button::Cross, Button::Circle, Button::Triangle,
        Button::L1, Button::R1, Button::L2, Button::R2,
        Button::Create, Button::Options, Button::L3, Button::R3,
        Button::PS, Button::Touchpad, Button::TouchpadLeft, Button::TouchpadRight, Button::Mic,
        Button::DpadUp, Button::DpadDown, Button::DpadLeft, Button::DpadRight,
        Button::FnLeft, Button::FnRight, Button::LeftPaddle, Button::RightPaddle,
    ];
}
