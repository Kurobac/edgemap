use std::fmt;

pub const USB_INPUT_REPORT_SIZE: usize = 64;
pub const USB_INPUT_REPORT_ID: u8 = 0x01;
pub const USB_OUTPUT_REPORT_SIZE: usize = 48;
pub const USB_OUTPUT_REPORT_ID: u8 = 0x02;
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

pub const BUTTON_COUNT: usize = 25;

impl Button {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_lowercase().as_str() {
            "square" => Self::Square,
            "cross" | "x" => Self::Cross,
            "circle" | "o" => Self::Circle,
            "triangle" => Self::Triangle,
            "l1" => Self::L1,
            "r1" => Self::R1,
            "l2" => Self::L2,
            "r2" => Self::R2,
            "create" | "share" => Self::Create,
            "options" => Self::Options,
            "l3" => Self::L3,
            "r3" => Self::R3,
            "ps" | "home" => Self::PS,
            "touchpad" => Self::Touchpad,
            "mic" => Self::Mic,
            "dpad_up" | "dup" => Self::DpadUp,
            "dpad_down" | "ddown" => Self::DpadDown,
            "dpad_left" | "dleft" => Self::DpadLeft,
            "dpad_right" | "dright" => Self::DpadRight,
            "fn_left" | "fnl" => Self::FnLeft,
            "fn_right" | "fnr" => Self::FnRight,
            "left_paddle" | "lp" => Self::LeftPaddle,
            "right_paddle" | "rp" | "r4" => Self::RightPaddle,
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
            Self::Mic => "mic",
            Self::DpadUp => "dpad_up",
            Self::DpadDown => "dpad_down",
            Self::DpadLeft => "dpad_left",
            Self::DpadRight => "dpad_right",
            Self::FnLeft => "fn_left",
            Self::FnRight => "fn_right",
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

    let b3 = data[11];
    state.set_button(Button::FnLeft, b3 & 0x10 != 0);
    state.set_button(Button::FnRight, b3 & 0x20 != 0);
    state.set_button(Button::LeftPaddle, b3 & 0x40 != 0);
    state.set_button(Button::RightPaddle, b3 & 0x80 != 0);

    let status0 = data[53];
    state.battery_pct = (status0 & 0x0F).min(10) * 10;
    let charging = (status0 >> 4) & 0x0F;
    state.battery_charging = charging == 0x01 || charging == 0x02;

    let status1 = data[54];
    state.headphone_connected = status1 & 0x01 == 0;

    Some(state)
}

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
    data[10] = b2;

    let mut b3: u8 = 0;
    if state.button(Button::FnLeft) {
        b3 |= 0x10;
    }
    if state.button(Button::FnRight) {
        b3 |= 0x20;
    }
    if state.button(Button::LeftPaddle) {
        b3 |= 0x40;
    }
    if state.button(Button::RightPaddle) {
        b3 |= 0x80;
    }
    data[11] = b3;

    data[53] = (state.battery_pct / 10) & 0x0F;
    let charging_bits: u8 = if state.battery_charging { 0x10 } else { 0x00 };
    data[53] |= charging_bits;

    data
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
