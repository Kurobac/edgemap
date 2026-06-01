use std::io;
use std::time::{Duration, Instant};

fn main() {
    println!("Touchpad Demo — press touchpad and move finger\n");

    let path = match find_dualsense_edge() {
        Some(p) => p,
        None => { eprintln!("No DualSense Edge found"); std::process::exit(1); }
    };

    let fd = open_hidraw(&path);

    let mut prev: [u8; 64] = [0; 64];
    let mut buf = [0u8; 64];
    let mut last_print = Instant::now();

    loop {
        match read_hidraw(fd, &mut buf) {
            Ok(64) => {
                let pad_pressed = buf[10] & 0x02 != 0;
                let count = buf[33] & 0x0F;
                let f0 = finger(0, &buf);
                let f1 = finger(1, &buf);

                let changed = buf[10] != prev[10]
                    || buf[33..42] != prev[33..42];

                if changed && last_print.elapsed() >= Duration::from_millis(50) {
                    println!("{}", "-".repeat(60));
                    print("Finger0", &f0);
                    print("Finger1", &f1);
                    println!("raw[33..42]: {:02x?}  c0={:02x} c1={:02x}", &buf[33..42], buf[33] & 0x80, buf[37] & 0x80);
                    println!("Press: {}\n", pad_pressed);

                    if pad_pressed && f0.touching {
                        let zone = if f0.x < 960 { "LEFT" } else { "RIGHT" };
                        println!(">> ZONE: {zone}\n");
                    }
                    last_print = Instant::now();
                }
                prev = buf;
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => { eprintln!("Error: {e}"); break; }
        }
    }
}

struct Finger {
    touching: bool,
    x: u16,
    y: u16,
}

fn finger(n: usize, buf: &[u8; 64]) -> Finger {
    let off = 33 + n * 4;
    let touching = buf[off] & 0x80 == 0;
    let x = if touching {
        ((buf[off + 2] as u16 & 0x0F) << 8) | buf[off + 1] as u16
    } else { 0 };
    let y = if touching {
        (buf[off + 3] as u16) << 4 | (buf[off + 2] as u16 >> 4)
    } else { 0 };
    Finger { touching, x, y }
}

fn print(label: &str, f: &Finger) {
    if f.touching {
        println!("{label}: ({:4},{:4})", f.x, f.y);
    } else {
        println!("{label}: (---,---)");
    }
}

// Reuse device detection from monitor
fn find_dualsense_edge() -> Option<String> {
    let dir = std::fs::read_dir("/dev").ok()?;
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("hidraw") { continue; }
        let path = format!("/dev/{name}");
        let fd = open_hidraw(&path);
        if fd < 0 { continue; }
        if let Ok((vid, pid)) = get_hidraw_info(fd) {
            if vid == 0x054C && pid == 0x0DF2 { return Some(path); }
        }
    }
    None
}

fn open_hidraw(path: &str) -> libc::c_int {
    let cpath = std::ffi::CString::new(path).unwrap();
    unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) }
}

fn read_hidraw(fd: libc::c_int, buf: &mut [u8; 64]) -> io::Result<usize> {
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 64) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

fn get_hidraw_info(fd: libc::c_int) -> io::Result<(u16, u16)> {
    #[repr(C)] struct HidrawDevinfo { bustype: u32, vendor: u16, product: u16 }
    let mut info = HidrawDevinfo { bustype: 0, vendor: 0, product: 0 };
    let request = ioc_read(3, std::mem::size_of::<HidrawDevinfo>());
    let ret = unsafe { libc::ioctl(fd, request as u64, &mut info) };
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok((info.vendor, info.product)) }
}

fn ioc_read(nr: u32, size: usize) -> u64 {
    (2u64 << 30) | ((b'H' as u64) << 8) | ((nr as u64) << 0) | ((size as u64) << 16)
}
