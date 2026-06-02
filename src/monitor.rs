use std::io;

pub fn run() {
    println!("DSE Button Monitor");
    println!("Press buttons, Ctrl+C to exit.\n");

    let path = match find_dualsense_edge() {
        Some(p) => p,
        None => {
            eprintln!("No DualSense Edge found. Is it connected via USB?");
            std::process::exit(1);
        }
    };

    let fd = open_hidraw(&path);

    let mut first = true;
    let mut prev: [u8; 64] = [0; 64];
    let mut buf = [0u8; 64];

    loop {
        match read_hidraw(fd, &mut buf) {
            Ok(64) => {
                let changed = first
                    || buf[5] != prev[5] || buf[6] != prev[6] // triggers
                    || buf[8..12] != prev[8..12]; // buttons
                if changed {
                    if !first {
                        println!("{}", "-".repeat(60));
                    }
                    first = false;
                    print_report(&buf);
                    prev = buf;
                }
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => {
                eprintln!("Read error: {e}");
                break;
            }
        }
    }
}

fn print_report(buf: &[u8; 64]) {
    println!("L2:{:3} R2:{:3}  Seq:{}", buf[5], buf[6], buf[7]);

    let mut buttons = Vec::new();

    let b8 = buf[8];
    let dpad = b8 & 0x0F;
    match dpad {
        0 => buttons.push("DPad_Up"),
        1 => buttons.push("DPad_Up+Right"),
        2 => buttons.push("DPad_Right"),
        3 => buttons.push("DPad_Down+Right"),
        4 => buttons.push("DPad_Down"),
        5 => buttons.push("DPad_Down+Left"),
        6 => buttons.push("DPad_Left"),
        7 => buttons.push("DPad_Up+Left"),
        _ => {}
    }
    if b8 & 0x10 != 0 { buttons.push("Square"); }
    if b8 & 0x20 != 0 { buttons.push("Cross"); }
    if b8 & 0x40 != 0 { buttons.push("Circle"); }
    if b8 & 0x80 != 0 { buttons.push("Triangle"); }

    let b9 = buf[9];
    if b9 & 0x01 != 0 { buttons.push("L1"); }
    if b9 & 0x02 != 0 { buttons.push("R1"); }
    if b9 & 0x04 != 0 { buttons.push("L2"); }
    if b9 & 0x08 != 0 { buttons.push("R2"); }
    if b9 & 0x10 != 0 { buttons.push("Create"); }
    if b9 & 0x20 != 0 { buttons.push("Options"); }
    if b9 & 0x40 != 0 { buttons.push("L3"); }
    if b9 & 0x80 != 0 { buttons.push("R3"); }

    let b10 = buf[10];
    if b10 & 0x01 != 0 { buttons.push("PS"); }
    if b10 & 0x02 != 0 { buttons.push("Touchpad"); }
    if b10 & 0x04 != 0 { buttons.push("Mic"); }
    if b10 & 0x10 != 0 { buttons.push("LeftFn"); }
    if b10 & 0x20 != 0 { buttons.push("RightFn"); }
    if b10 & 0x40 != 0 { buttons.push("LeftPaddle"); }
    if b10 & 0x80 != 0 { buttons.push("RightPaddle"); }

    if buttons.is_empty() {
        buttons.push("[none]");
    }
    println!("Buttons: {}", buttons.join(" "));
    println!("byte[8..12]: {:02x?}", &buf[8..12]);
}

fn find_dualsense_edge() -> Option<String> {
    let dir = match std::fs::read_dir(HIDRAW_DEV_DIR) {
        Ok(d) => d,
        Err(_) => return None,
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("hidraw") {
            continue;
        }
        let path = format!("/dev/{name}");
        let fd = open_hidraw(&path);
        if fd < 0 { continue; }
        match get_hidraw_info(fd) {
            Ok((vid, pid)) if vid == SONY_VID && pid == DS5_EDGE_PID => {
                return Some(path);
            }
            _ => {}
        }
    }
    None
}

fn open_hidraw(path: &str) -> libc::c_int {
    let cpath = std::ffi::CString::new(path).unwrap();
    unsafe {
        libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK)
    }
}

fn read_hidraw(fd: libc::c_int, buf: &mut [u8; 64]) -> io::Result<usize> {
    let n = unsafe {
        libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 64)
    };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

fn get_hidraw_info(fd: libc::c_int) -> io::Result<(u16, u16)> {
    #[repr(C)]
    struct HidrawDevinfo {
        bustype: u32,
        vendor: u16,
        product: u16,
    }
    let mut info = HidrawDevinfo { bustype: 0, vendor: 0, product: 0 };
    let request = ioc_read(3, std::mem::size_of::<HidrawDevinfo>());
    let ret = unsafe {
        libc::ioctl(fd, request as u64, &mut info)
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((info.vendor, info.product))
    }
}

fn ioc_read(nr: u32, size: usize) -> u64 {
    (2u64 << 30) | ((b'H' as u64) << 8) | ((nr as u64) << 0) | ((size as u64) << 16)
}

const SONY_VID: u16 = 0x054C;
const DS5_EDGE_PID: u16 = 0x0DF2;
const HIDRAW_DEV_DIR: &str = "/dev";
