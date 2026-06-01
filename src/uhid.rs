use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};

use log::debug;

const UHID_DEVICE: &str = "/dev/uhid";
pub const UHID_EVENT_SIZE: usize = 4384;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UhidEventType {
    Destroy = 1,
    Start = 2,
    Stop = 3,
    Open = 4,
    Close = 5,
    Output = 6,
    GetReport = 9,
    GetReportReply = 10,
    Create2 = 11,
    Input2 = 12,
    SetReport = 13,
    SetReportReply = 14,
}

impl TryFrom<u32> for UhidEventType {
    type Error = ();
    fn try_from(v: u32) -> Result<Self, ()> {
        Ok(match v {
            1 => Self::Destroy,
            2 => Self::Start,
            3 => Self::Stop,
            4 => Self::Open,
            5 => Self::Close,
            6 => Self::Output,
            9 => Self::GetReport,
            10 => Self::GetReportReply,
            11 => Self::Create2,
            12 => Self::Input2,
            13 => Self::SetReport,
            14 => Self::SetReportReply,
            _ => return Err(()),
        })
    }
}

#[derive(Debug)]
pub enum UhidEvent {
    Start,
    Stop,
    Open,
    Close,
    Output {
        rtype: u8,
        data: Vec<u8>,
    },
    GetReport {
        id: u32,
        rnum: u8,
        rtype: u8,
    },
    SetReport {
        id: u32,
        rnum: u8,
        rtype: u8,
        data: Vec<u8>,
    },
    Unknown(u32),
}

pub struct UhidDevice {
    fd: OwnedFd,
    created: bool,
}

impl UhidDevice {
    pub fn open() -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(UHID_DEVICE)?;

        Ok(Self {
            fd: OwnedFd::from(file),
            created: false,
        })
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        name: &str,
        phys: &str,
        uniq: &str,
        bus: u16,
        vendor: u32,
        product: u32,
        version: u32,
        country: u32,
        rd_data: &[u8],
    ) -> io::Result<()> {
        if rd_data.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "report descriptor too large (max 4096)",
            ));
        }

        let mut buf = [0u8; UHID_EVENT_SIZE];
        buf[0..4].copy_from_slice(&UhidEventType::Create2.to_u32_le());

        let off_name = 4;
        let off_phys = off_name + 128;
        let off_uniq = off_phys + 64;
        let off_rd_size = off_uniq + 64;
        let off_bus = off_rd_size + 2;
        let off_vendor = off_bus + 2;
        let off_product = off_vendor + 4;
        let off_version = off_product + 4;
        let off_country = off_version + 4;
        let off_rd_data = off_country + 4;

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(127);
        buf[off_name..off_name + name_len].copy_from_slice(&name_bytes[..name_len]);

        let phys_bytes = phys.as_bytes();
        let phys_len = phys_bytes.len().min(63);
        buf[off_phys..off_phys + phys_len].copy_from_slice(&phys_bytes[..phys_len]);

        let uniq_bytes = uniq.as_bytes();
        let uniq_len = uniq_bytes.len().min(63);
        buf[off_uniq..off_uniq + uniq_len].copy_from_slice(&uniq_bytes[..uniq_len]);

        buf[off_rd_size..off_rd_size + 2]
            .copy_from_slice(&(rd_data.len() as u16).to_le_bytes());
        buf[off_bus..off_bus + 2].copy_from_slice(&bus.to_le_bytes());
        buf[off_vendor..off_vendor + 4].copy_from_slice(&vendor.to_le_bytes());
        buf[off_product..off_product + 4].copy_from_slice(&product.to_le_bytes());
        buf[off_version..off_version + 4].copy_from_slice(&version.to_le_bytes());
        buf[off_country..off_country + 4].copy_from_slice(&country.to_le_bytes());
        buf[off_rd_data..off_rd_data + rd_data.len()].copy_from_slice(rd_data);

        let fd = self.fd.as_raw_fd();
        let total_size = off_rd_data + rd_data.len();
        let written = unsafe {
            libc::write(fd, buf.as_ptr() as *const libc::c_void, total_size)
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }

        debug!(
            "UHID create2: name={name}, rd_size={}, bus={bus}, vid={vendor:04x}, pid={product:04x}, written={written}",
            rd_data.len()
        );
        self.created = true;
        Ok(())
    }

    pub fn destroy(&mut self) -> io::Result<()> {
        if !self.created {
            return Ok(());
        }

        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&UhidEventType::Destroy.to_u32_le());

        let fd = self.fd.as_raw_fd();
        let written = unsafe {
            libc::write(fd, buf.as_ptr() as *const libc::c_void, 8)
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }

        debug!("UHID destroy sent");
        self.created = false;
        Ok(())
    }

    pub fn send_input(&self, data: &[u8]) -> io::Result<()> {
        if data.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "input data too large",
            ));
        }

        let mut buf = [0u8; UHID_EVENT_SIZE];
        buf[0..4].copy_from_slice(&UhidEventType::Input2.to_u32_le());
        buf[4..6].copy_from_slice(&(data.len() as u16).to_le_bytes());
        buf[6..6 + data.len()].copy_from_slice(data);

        let fd = self.fd.as_raw_fd();
        let total_size = 6 + data.len();
        let written = unsafe {
            libc::write(fd, buf.as_ptr() as *const libc::c_void, total_size)
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn send_get_report_reply(&self, id: u32, err: u16, data: &[u8]) -> io::Result<()> {
        if data.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "data too large",
            ));
        }

        let mut buf = [0u8; UHID_EVENT_SIZE];
        buf[0..4].copy_from_slice(&UhidEventType::GetReportReply.to_u32_le());
        buf[4..8].copy_from_slice(&id.to_le_bytes());
        buf[8..10].copy_from_slice(&err.to_le_bytes());
        buf[10..12].copy_from_slice(&(data.len() as u16).to_le_bytes());
        buf[12..12 + data.len()].copy_from_slice(data);

        let fd = self.fd.as_raw_fd();
        let total_size = 12 + data.len();
        unsafe {
            libc::write(fd, buf.as_ptr() as *const libc::c_void, total_size);
        }
        Ok(())
    }

    pub fn recv_event(&self) -> io::Result<Option<UhidEvent>> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        let fd = self.fd.as_raw_fd();

        let n = unsafe {
            libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, UHID_EVENT_SIZE)
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(err);
        }

        if n < 4 {
            return Ok(None);
        }

        let ev_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let ev = match UhidEventType::try_from(ev_type) {
            Ok(UhidEventType::Start) => UhidEvent::Start,
            Ok(UhidEventType::Stop) => UhidEvent::Stop,
            Ok(UhidEventType::Open) => UhidEvent::Open,
            Ok(UhidEventType::Close) => UhidEvent::Close,
            Ok(UhidEventType::Output) => {
                let rtype = buf[4102];
                let size = u16::from_le_bytes([buf[4100], buf[4101]]) as usize;
                let size = size.min(4096);
                UhidEvent::Output {
                    rtype,
                    data: buf[4..4 + size].to_vec(),
                }
            }
            Ok(UhidEventType::GetReport) => {
                let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
                let rnum = buf[8];
                let rtype = buf[9];
                UhidEvent::GetReport { id, rnum, rtype }
            }
            Ok(UhidEventType::SetReport) => {
                let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
                let rnum = buf[8];
                let rtype = buf[9];
                let size = u16::from_le_bytes([buf[10], buf[11]]) as usize;
                let size = size.min(4096);
                UhidEvent::SetReport {
                    id,
                    rnum,
                    rtype,
                    data: buf[12..12 + size].to_vec(),
                }
            }
            Ok(_) => UhidEvent::Unknown(ev_type),
            Err(_) => UhidEvent::Unknown(ev_type),
        };

        debug!("UHID recv: {ev:?}");
        Ok(Some(ev))
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}

impl UhidEventType {
    fn to_u32_le(self) -> [u8; 4] {
        (self as u32).to_le_bytes()
    }
}
