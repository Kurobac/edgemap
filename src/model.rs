use std::fmt;

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
pub const ALL_BUTTONS: [Button; BUTTON_COUNT] = [
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
    Button::L2Analog,
    Button::R2Analog,
];

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
