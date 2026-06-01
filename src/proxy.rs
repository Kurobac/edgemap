use std::io;

use log::{debug, error, info, warn};
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags};
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::os::fd::BorrowedFd;

use crate::device::HidrawDevice;
use crate::report;
use crate::uhid::UhidDevice;

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn setup_signal_handler() {
    unsafe {
        let handler = SigHandler::SigAction(handle_signal);
        let action = SigAction::new(handler, SaFlags::empty(), SigSet::empty());
        let _ = sigaction(Signal::SIGINT, &action);
        let _ = sigaction(Signal::SIGTERM, &action);
    }
}

extern "C" fn handle_signal(
    _sig: libc::c_int,
    _info: *mut libc::siginfo_t,
    _ctx: *mut libc::c_void,
) {
    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
}

fn get_cached_report(report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x05 => Some(vec![
            0x05, 0xff, 0xfc, 0xff, 0xfe, 0xff, 0x83, 0x22, 0x78, 0xdd,
            0x92, 0x22, 0x5f, 0xdd, 0x95, 0x22, 0x6d, 0xdd, 0x1c, 0x02,
            0x1c, 0x02, 0xf2, 0x1f, 0xed, 0xdf, 0xe3, 0x20, 0xda, 0xe0,
            0xee, 0x1f, 0xdf, 0xdf, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00,
        ]),
        0x08 => Some(vec![0u8; 48]),
        0x09 => Some(vec![
            0x09, 0xd4, 0x2f, 0x4b, 0x26, 0x18, 0xc2, 0x08, 0x25,
            0x00, 0x1e, 0x00, 0xee, 0x74, 0xd0, 0xbc, 0x00, 0x00, 0x00, 0x00,
        ]),
        0x0A => Some(vec![0u8; 27]),
        0x20 => Some(vec![
            0x20, 0x4a, 0x75, 0x6e, 0x20, 0x31, 0x39, 0x20, 0x32,
            0x30, 0x32, 0x33, 0x31, 0x34, 0x3a, 0x34, 0x37, 0x3a, 0x33, 0x34,
            0x03, 0x00, 0x44, 0x00, 0x08, 0x02, 0x00, 0x01, 0x36, 0x00,
            0x00, 0x01, 0xc1, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x54, 0x01, 0x00, 0x00, 0x14, 0x00,
            0x00, 0x00, 0x0b, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
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

pub struct Proxy {
    hidraw: HidrawDevice,
    uhid: UhidDevice,
}

impl Proxy {
    pub fn new(hidraw: HidrawDevice, uhid: UhidDevice) -> Self {
        Self { hidraw, uhid }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let ep_fd = Epoll::new(EpollCreateFlags::empty())?;

        let hidraw_bfd = unsafe {
            BorrowedFd::borrow_raw(self.hidraw.as_raw_fd())
        };
        let uhid_bfd = unsafe {
            BorrowedFd::borrow_raw(self.uhid.as_raw_fd())
        };

        let hidraw_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            1,
        );
        ep_fd.add(&hidraw_bfd, hidraw_event)?;

        let uhid_event = EpollEvent::new(
            EpollFlags::EPOLLIN | EpollFlags::EPOLLERR | EpollFlags::EPOLLHUP,
            2,
        );
        ep_fd.add(&uhid_bfd, uhid_event)?;

        info!("Proxy running. Press Ctrl+C to stop.");

        let mut seq: u8 = 0;
        let mut events = [EpollEvent::empty(); 8];

        while RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
            match ep_fd.wait(&mut events, 16u16) {
                Ok(n) => {
                    for i in 0..n {
                        let fd_num = events[i].data() as u64;

                        if fd_num == 1 {
                            self.handle_hidraw_input(&mut seq)?;
                        } else if fd_num == 2 {
                            self.handle_uhid_event()?;
                        }
                    }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    error!("epoll wait error: {e}");
                    break;
                }
            }
        }

        info!("Proxy stopped.");
        Ok(())
    }

    fn handle_hidraw_input(&mut self, seq: &mut u8) -> io::Result<()> {
        let mut buf = [0u8; report::USB_INPUT_REPORT_SIZE];

        loop {
            match self.hidraw.read_input(&mut buf) {
                Ok(n) if n >= report::USB_INPUT_REPORT_SIZE => {
                    *seq = seq.wrapping_add(1);
                    buf[7] = *seq;

                    if let Err(e) = self.uhid.send_input(&buf) {
                        error!("Failed to send UHID input: {e}");
                    }
                }
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(ref e) if e.raw_os_error() == Some(libc::EIO) => {
                    error!("hidraw I/O error (EIO). Controller disconnected?");
                    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Err(e) => {
                    error!("hidraw read error: {e}");
                    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_uhid_event(&mut self) -> io::Result<()> {
        loop {
            match self.uhid.recv_event() {
                Ok(Some(event)) => {
                    use crate::uhid::UhidEvent;
                    match event {
                        UhidEvent::Start => {
                            info!("UHID device started");
                        }
                        UhidEvent::Stop => {
                            warn!("UHID device stopped");
                        }
                        UhidEvent::Open => {
                            debug!("UHID device opened by client");
                        }
                        UhidEvent::Close => {
                            debug!("UHID device closed by client");
                        }
                        UhidEvent::Output { rtype, ref data } => {
                            // rtype is UHID report type: 0=Feature, 1=Output, 2=Input
                            if rtype == 1 {
                                debug!("UHID OUTPUT: size={}", data.len());
                                if let Err(e) = self.hidraw.write_output(data) {
                                    error!("Failed to forward output report: {e}");
                                }
                            } else {
                                debug!("UHID Output with unexpected rtype={rtype}, ignoring");
                            }
                        }
                        UhidEvent::GetReport { id, rnum, rtype } => {
                            debug!("UHID GET_REPORT: id={id}, rnum={rnum}, rtype={rtype}");
                            match get_cached_report(rnum) {
                                Some(data) => {
                                    debug!("GET_REPORT rnum={rnum}: served from cache");
                                    let _ = self.uhid.send_get_report_reply(id, 0, &data);
                                }
                                None => {
                                    debug!("GET_REPORT rnum={rnum}: not cached, returning error");
                                    let _ = self.uhid.send_get_report_reply(id, 1, &[]);
                                }
                            }
                        }
                        UhidEvent::Unknown(t) => {
                            debug!("Unknown UHID event type: {t}");
                        }
                        UhidEvent::SetReport { id, .. } => {
                            debug!("UHID SET_REPORT id={id}, replying OK");
                            let _ = self.uhid.send_set_report_reply(id, 0);
                        }
                    }
                }
                Ok(None) => break,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    error!("UHID read error: {e}");
                    break;
                }
            }
        }
        Ok(())
    }
}
