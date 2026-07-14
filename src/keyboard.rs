use std::collections::HashSet;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;

use log::{info, warn};

use crate::keycodes::ALL_KEYCODES;

// uinput ioctls
const UI_SET_EVBIT: u32 = 0x40045564;
const UI_SET_KEYBIT: u32 = 0x40045565;
const UI_DEV_DESTROY: u64 = (0x55u64 << 8) | 0x02;

#[repr(C)]
struct InputEvent {
    time_sec: libc::time_t,
    time_usec: libc::suseconds_t,
    ev_type: u16,
    code: u16,
    value: i32,
}

impl Drop for KeyboardDevice {
    fn drop(&mut self) {
        self.flush_held();
        if let Some(fd) = &self.fd {
            let ret = unsafe { libc::ioctl(fd.as_raw_fd(), UI_DEV_DESTROY) };
            if ret < 0 {
                warn!(
                    "failed to destroy virtual keyboard: {}",
                    io::Error::last_os_error()
                );
            } else {
                info!("uinput UI_DEV_DESTROY sent");
            }
        }
    }
}

pub struct KeyboardDevice {
    fd: Option<OwnedFd>,
    held_keys: HashSet<u16>,
}

impl KeyboardDevice {
    pub fn open() -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")?;

        let fd = OwnedFd::from(file);
        let raw = fd.as_raw_fd();

        set_bit(raw, UI_SET_EVBIT, 0x01).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("failed to enable uinput EV_KEY capability: {e}"),
            )
        })?;
        set_bit(raw, UI_SET_EVBIT, 0x00).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("failed to enable uinput EV_SYN capability: {e}"),
            )
        })?;

        for code in ALL_KEYCODES {
            set_bit(raw, UI_SET_KEYBIT, *code).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("failed to enable uinput key capability: code={code}, error={e}"),
                )
            })?;
        }

        unsafe {
            Self::create_device(raw)?;
        }

        Ok(Self {
            fd: Some(fd),
            held_keys: HashSet::new(),
        })
    }

    pub fn dummy() -> Self {
        Self {
            fd: None,
            held_keys: HashSet::new(),
        }
    }

    pub fn press(&mut self, code: u16) -> bool {
        if self.held_keys.contains(&code) {
            return true;
        }
        if self.send_event(code, 1).is_ok() {
            self.held_keys.insert(code);
            true
        } else {
            false
        }
    }

    pub fn release(&mut self, code: u16) -> bool {
        if !self.held_keys.contains(&code) {
            return true;
        }
        if self.send_event(code, 0).is_ok() {
            self.held_keys.remove(&code);
            true
        } else {
            false
        }
    }

    pub fn flush_held(&mut self) {
        let held: Vec<u16> = self.held_keys.iter().copied().collect();
        for code in held {
            let _ = self.release(code);
        }
    }

    fn send_event(&self, code: u16, value: i32) -> io::Result<()> {
        if let Some(ref fd) = self.fd {
            let ev = InputEvent {
                time_sec: 0,
                time_usec: 0,
                ev_type: 0x01,
                code,
                value,
            };
            write_input_event(fd.as_raw_fd(), &ev)
                .inspect_err(|e| log::error!("failed to write uinput key event: {e}"))?;
            let syn = InputEvent {
                time_sec: 0,
                time_usec: 0,
                ev_type: 0x00,
                code: 0,
                value: 0,
            };
            write_input_event(fd.as_raw_fd(), &syn)
                .inspect_err(|e| log::error!("failed to write uinput SYN event: {e}"))?;
        }
        Ok(())
    }

    unsafe fn create_device(fd: libc::c_int) -> io::Result<()> {
        #[repr(C)]
        struct UinputUserDev {
            name: [u8; 80],
            id: InputId,
            ff_effects_max: u32,
            absmax: [i32; 64],
            absmin: [i32; 64],
            absfuzz: [i32; 64],
            absflat: [i32; 64],
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct InputId {
            bustype: u16,
            vendor: u16,
            product: u16,
            version: u16,
        }

        let name = b"edgemap Keyboard\0";
        let mut name_buf = [0u8; 80];
        name_buf[..name.len()].copy_from_slice(name);

        let dev = UinputUserDev {
            name: name_buf,
            id: InputId {
                bustype: 0x0003,
                vendor: 0x054C,
                product: 0x0DF2,
                version: 0x0100,
            },
            ff_effects_max: 0,
            absmax: [0i32; 64],
            absmin: [0i32; 64],
            absfuzz: [0i32; 64],
            absflat: [0i32; 64],
        };

        let ret = libc::write(
            fd,
            &dev as *const UinputUserDev as *const libc::c_void,
            std::mem::size_of::<UinputUserDev>(),
        );
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        let request = ((0x55u64) << 8) | (0x01u64);
        let ret = libc::ioctl(fd, request);
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
}

fn write_input_event(fd: libc::c_int, event: &InputEvent) -> io::Result<()> {
    let size = std::mem::size_of::<InputEvent>();
    let ret = unsafe { libc::write(fd, event as *const InputEvent as *const libc::c_void, size) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else if ret as usize != size {
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short uinput event write",
        ))
    } else {
        Ok(())
    }
}

fn set_bit(fd: libc::c_int, cmd: u32, bit: u16) -> io::Result<()> {
    let cmd_nr = match cmd {
        UI_SET_EVBIT => 0x64,
        UI_SET_KEYBIT => 0x65,
        _ => return Ok(()),
    };
    let request = ((1u64) << 30) | ((0x55u64) << 8) | (cmd_nr as u64) | ((4u64) << 16);
    let ret = unsafe { libc::ioctl(fd, request, bit as libc::c_ulong) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_tracks_key_only_after_successful_write() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .unwrap();
        let mut keyboard = KeyboardDevice {
            fd: Some(OwnedFd::from(file)),
            held_keys: HashSet::new(),
        };

        let key_a = crate::keycodes::resolve_keycode("a").unwrap();
        assert!(keyboard.press(key_a));

        assert!(keyboard.held_keys.contains(&key_a));
    }

    #[test]
    fn failed_press_is_retried_and_failed_release_stays_held() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let mut keyboard = KeyboardDevice {
            fd: Some(OwnedFd::from(file)),
            held_keys: HashSet::new(),
        };

        let key_a = crate::keycodes::resolve_keycode("a").unwrap();
        assert!(!keyboard.press(key_a));
        assert!(!keyboard.held_keys.contains(&key_a));

        keyboard.held_keys.insert(key_a);
        assert!(!keyboard.release(key_a));
        assert!(keyboard.held_keys.contains(&key_a));
    }
}
