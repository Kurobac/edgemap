// Key codes from linux/input-event-codes.h.
const KEY_A: u16 = 30;
const KEY_B: u16 = 48;
const KEY_C: u16 = 46;
const KEY_D: u16 = 32;
const KEY_E: u16 = 18;
const KEY_F: u16 = 33;
const KEY_G: u16 = 34;
const KEY_H: u16 = 35;
const KEY_I: u16 = 23;
const KEY_J: u16 = 36;
const KEY_K: u16 = 37;
const KEY_L: u16 = 38;
const KEY_M: u16 = 50;
const KEY_N: u16 = 49;
const KEY_O: u16 = 24;
const KEY_P: u16 = 25;
const KEY_Q: u16 = 16;
const KEY_R: u16 = 19;
const KEY_S: u16 = 31;
const KEY_T: u16 = 20;
const KEY_U: u16 = 22;
const KEY_V: u16 = 47;
const KEY_W: u16 = 17;
const KEY_X: u16 = 45;
const KEY_Y: u16 = 21;
const KEY_Z: u16 = 44;
const KEY_0: u16 = 11;
const KEY_1: u16 = 2;
const KEY_2: u16 = 3;
const KEY_3: u16 = 4;
const KEY_4: u16 = 5;
const KEY_5: u16 = 6;
const KEY_6: u16 = 7;
const KEY_7: u16 = 8;
const KEY_8: u16 = 9;
const KEY_9: u16 = 10;
const KEY_F1: u16 = 59;
const KEY_F2: u16 = 60;
const KEY_F3: u16 = 61;
const KEY_F4: u16 = 62;
const KEY_F5: u16 = 63;
const KEY_F6: u16 = 64;
const KEY_F7: u16 = 65;
const KEY_F8: u16 = 66;
const KEY_F9: u16 = 67;
const KEY_F10: u16 = 68;
const KEY_F11: u16 = 87;
const KEY_F12: u16 = 88;
const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_HOME: u16 = 102;
const KEY_END: u16 = 107;
const KEY_PAGEUP: u16 = 104;
const KEY_PAGEDOWN: u16 = 109;
const KEY_INSERT: u16 = 110;
const KEY_DELETE: u16 = 111;
const KEY_LEFTCTRL: u16 = 29;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;
const KEY_ENTER: u16 = 28;
const KEY_SPACE: u16 = 57;
const KEY_TAB: u16 = 15;
const KEY_ESC: u16 = 1;
const KEY_BACKSPACE: u16 = 14;
const KEY_CAPSLOCK: u16 = 58;
const KEY_KP0: u16 = 82;
const KEY_KP1: u16 = 79;
const KEY_KP2: u16 = 80;
const KEY_KP3: u16 = 81;
const KEY_KP4: u16 = 75;
const KEY_KP5: u16 = 76;
const KEY_KP6: u16 = 77;
const KEY_KP7: u16 = 71;
const KEY_KP8: u16 = 72;
const KEY_KP9: u16 = 73;
const KEY_KPDOT: u16 = 83;
const KEY_KPENTER: u16 = 96;
const KEY_KPPLUS: u16 = 78;
const KEY_KPMINUS: u16 = 74;
const KEY_KPASTERISK: u16 = 55;
const KEY_KPSLASH: u16 = 98;
const KEY_NUMLOCK: u16 = 69;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_LEFTBRACE: u16 = 26;
const KEY_RIGHTBRACE: u16 = 27;
const KEY_BACKSLASH: u16 = 43;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_COMMA: u16 = 51;
const KEY_DOT: u16 = 52;
const KEY_SLASH: u16 = 53;
const KEY_GRAVE: u16 = 41;
const KEY_VOLUMEUP: u16 = 115;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_MUTE: u16 = 113;
const KEY_PLAYPAUSE: u16 = 164;
const KEY_STOPCD: u16 = 166;
const KEY_PREVIOUSSONG: u16 = 165;
const KEY_NEXTSONG: u16 = 163;

pub fn resolve_keycode(name: &str) -> Option<u16> {
    match name {
        "a" => Some(KEY_A),
        "b" => Some(KEY_B),
        "c" => Some(KEY_C),
        "d" => Some(KEY_D),
        "e" => Some(KEY_E),
        "f" => Some(KEY_F),
        "g" => Some(KEY_G),
        "h" => Some(KEY_H),
        "i" => Some(KEY_I),
        "j" => Some(KEY_J),
        "k" => Some(KEY_K),
        "l" => Some(KEY_L),
        "m" => Some(KEY_M),
        "n" => Some(KEY_N),
        "o" => Some(KEY_O),
        "p" => Some(KEY_P),
        "q" => Some(KEY_Q),
        "r" => Some(KEY_R),
        "s" => Some(KEY_S),
        "t" => Some(KEY_T),
        "u" => Some(KEY_U),
        "v" => Some(KEY_V),
        "w" => Some(KEY_W),
        "x" => Some(KEY_X),
        "y" => Some(KEY_Y),
        "z" => Some(KEY_Z),
        "0" => Some(KEY_0),
        "1" => Some(KEY_1),
        "2" => Some(KEY_2),
        "3" => Some(KEY_3),
        "4" => Some(KEY_4),
        "5" => Some(KEY_5),
        "6" => Some(KEY_6),
        "7" => Some(KEY_7),
        "8" => Some(KEY_8),
        "9" => Some(KEY_9),
        "f1" => Some(KEY_F1),
        "f2" => Some(KEY_F2),
        "f3" => Some(KEY_F3),
        "f4" => Some(KEY_F4),
        "f5" => Some(KEY_F5),
        "f6" => Some(KEY_F6),
        "f7" => Some(KEY_F7),
        "f8" => Some(KEY_F8),
        "f9" => Some(KEY_F9),
        "f10" => Some(KEY_F10),
        "f11" => Some(KEY_F11),
        "f12" => Some(KEY_F12),
        "up" => Some(KEY_UP),
        "down" => Some(KEY_DOWN),
        "left" => Some(KEY_LEFT),
        "right" => Some(KEY_RIGHT),
        "home" => Some(KEY_HOME),
        "end" => Some(KEY_END),
        "pageup" | "pgup" => Some(KEY_PAGEUP),
        "pagedown" | "pgdn" => Some(KEY_PAGEDOWN),
        "insert" | "ins" => Some(KEY_INSERT),
        "delete" | "del" => Some(KEY_DELETE),
        "leftctrl" | "lctrl" => Some(KEY_LEFTCTRL),
        "rightctrl" | "rctrl" => Some(KEY_RIGHTCTRL),
        "leftshift" | "lshift" => Some(KEY_LEFTSHIFT),
        "rightshift" | "rshift" => Some(KEY_RIGHTSHIFT),
        "leftalt" | "lalt" => Some(KEY_LEFTALT),
        "rightalt" | "ralt" => Some(KEY_RIGHTALT),
        "leftsuper" | "lsuper" | "leftmeta" | "lmeta" => Some(KEY_LEFTMETA),
        "rightsuper" | "rsuper" | "rightmeta" | "rmeta" => Some(KEY_RIGHTMETA),
        "enter" | "return" => Some(KEY_ENTER),
        "space" => Some(KEY_SPACE),
        "tab" => Some(KEY_TAB),
        "escape" | "esc" => Some(KEY_ESC),
        "backspace" | "bksp" => Some(KEY_BACKSPACE),
        "capslock" | "caps" => Some(KEY_CAPSLOCK),
        "kp0" => Some(KEY_KP0),
        "kp1" => Some(KEY_KP1),
        "kp2" => Some(KEY_KP2),
        "kp3" => Some(KEY_KP3),
        "kp4" => Some(KEY_KP4),
        "kp5" => Some(KEY_KP5),
        "kp6" => Some(KEY_KP6),
        "kp7" => Some(KEY_KP7),
        "kp8" => Some(KEY_KP8),
        "kp9" => Some(KEY_KP9),
        "kpdot" | "kp." => Some(KEY_KPDOT),
        "kpenter" => Some(KEY_KPENTER),
        "kpplus" | "kp+" => Some(KEY_KPPLUS),
        "kpminus" | "kp-" => Some(KEY_KPMINUS),
        "kp*" => Some(KEY_KPASTERISK),
        "kpslash" => Some(KEY_KPSLASH),
        "numlock" | "numlk" => Some(KEY_NUMLOCK),
        "minus" | "-" => Some(KEY_MINUS),
        "equal" | "=" => Some(KEY_EQUAL),
        "leftbrace" | "[" => Some(KEY_LEFTBRACE),
        "rightbrace" | "]" => Some(KEY_RIGHTBRACE),
        "backslash" | "\\" => Some(KEY_BACKSLASH),
        "semicolon" | ";" => Some(KEY_SEMICOLON),
        "apostrophe" | "'" => Some(KEY_APOSTROPHE),
        "comma" | "," => Some(KEY_COMMA),
        "dot" | "." => Some(KEY_DOT),
        "slash" | "/" => Some(KEY_SLASH),
        "grave" | "`" => Some(KEY_GRAVE),
        "volumeup" | "volup" => Some(KEY_VOLUMEUP),
        "volumedown" | "voldown" => Some(KEY_VOLUMEDOWN),
        "mute" => Some(KEY_MUTE),
        "playpause" | "play" => Some(KEY_PLAYPAUSE),
        "stop" => Some(KEY_STOPCD),
        "previoussong" | "prev" => Some(KEY_PREVIOUSSONG),
        "nextsong" | "next" => Some(KEY_NEXTSONG),
        _ => None,
    }
}

pub const ALL_KEYCODES: &[u16] = &[
    KEY_A,
    KEY_B,
    KEY_C,
    KEY_D,
    KEY_E,
    KEY_F,
    KEY_G,
    KEY_H,
    KEY_I,
    KEY_J,
    KEY_K,
    KEY_L,
    KEY_M,
    KEY_N,
    KEY_O,
    KEY_P,
    KEY_Q,
    KEY_R,
    KEY_S,
    KEY_T,
    KEY_U,
    KEY_V,
    KEY_W,
    KEY_X,
    KEY_Y,
    KEY_Z,
    KEY_0,
    KEY_1,
    KEY_2,
    KEY_3,
    KEY_4,
    KEY_5,
    KEY_6,
    KEY_7,
    KEY_8,
    KEY_9,
    KEY_F1,
    KEY_F2,
    KEY_F3,
    KEY_F4,
    KEY_F5,
    KEY_F6,
    KEY_F7,
    KEY_F8,
    KEY_F9,
    KEY_F10,
    KEY_F11,
    KEY_F12,
    KEY_UP,
    KEY_DOWN,
    KEY_LEFT,
    KEY_RIGHT,
    KEY_HOME,
    KEY_END,
    KEY_PAGEUP,
    KEY_PAGEDOWN,
    KEY_INSERT,
    KEY_DELETE,
    KEY_LEFTCTRL,
    KEY_RIGHTCTRL,
    KEY_LEFTSHIFT,
    KEY_RIGHTSHIFT,
    KEY_LEFTALT,
    KEY_RIGHTALT,
    KEY_LEFTMETA,
    KEY_RIGHTMETA,
    KEY_ENTER,
    KEY_SPACE,
    KEY_TAB,
    KEY_ESC,
    KEY_BACKSPACE,
    KEY_CAPSLOCK,
    KEY_KP0,
    KEY_KP1,
    KEY_KP2,
    KEY_KP3,
    KEY_KP4,
    KEY_KP5,
    KEY_KP6,
    KEY_KP7,
    KEY_KP8,
    KEY_KP9,
    KEY_KPDOT,
    KEY_KPENTER,
    KEY_KPPLUS,
    KEY_KPMINUS,
    KEY_KPASTERISK,
    KEY_KPSLASH,
    KEY_NUMLOCK,
    KEY_MINUS,
    KEY_EQUAL,
    KEY_LEFTBRACE,
    KEY_RIGHTBRACE,
    KEY_BACKSLASH,
    KEY_SEMICOLON,
    KEY_APOSTROPHE,
    KEY_COMMA,
    KEY_DOT,
    KEY_SLASH,
    KEY_GRAVE,
    KEY_VOLUMEUP,
    KEY_VOLUMEDOWN,
    KEY_MUTE,
    KEY_PLAYPAUSE,
    KEY_STOPCD,
    KEY_PREVIOUSSONG,
    KEY_NEXTSONG,
];
