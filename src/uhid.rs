use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};

use log::{debug, info};

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

#[derive(Debug, PartialEq, Eq)]
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

fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let written = unsafe {
        libc::write(fd, data.as_ptr() as *const libc::c_void, data.len())
    };
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    if written as usize != data.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("short write: wrote {written} of {} bytes", data.len()),
        ));
    }
    Ok(())
}

fn invalid_event(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_uhid_event(buf: &[u8]) -> io::Result<Option<UhidEvent>> {
    if buf.len() < 4 {
        return Ok(None);
    }

    let ev_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let ev = match UhidEventType::try_from(ev_type) {
        Ok(UhidEventType::Start) => UhidEvent::Start,
        Ok(UhidEventType::Stop) => UhidEvent::Stop,
        Ok(UhidEventType::Open) => UhidEvent::Open,
        Ok(UhidEventType::Close) => UhidEvent::Close,
        Ok(UhidEventType::Output) => {
            if buf.len() < 4103 {
                return Err(invalid_event(format!(
                    "short UHID OUTPUT event: {} bytes",
                    buf.len()
                )));
            }
            let size = u16::from_le_bytes([buf[4100], buf[4101]]) as usize;
            if size > 4096 {
                return Err(invalid_event(format!("UHID OUTPUT too large: {size} bytes")));
            }
            let end = 4 + size;
            if buf.len() < end {
                return Err(invalid_event(format!(
                    "short UHID OUTPUT payload: {} bytes, need {end}",
                    buf.len()
                )));
            }
            UhidEvent::Output {
                rtype: buf[4102],
                data: buf[4..end].to_vec(),
            }
        }
        Ok(UhidEventType::GetReport) => {
            if buf.len() < 10 {
                return Err(invalid_event(format!(
                    "short UHID GET_REPORT event: {} bytes",
                    buf.len()
                )));
            }
            let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let rnum = buf[8];
            let rtype = buf[9];
            UhidEvent::GetReport { id, rnum, rtype }
        }
        Ok(UhidEventType::SetReport) => {
            if buf.len() < 12 {
                return Err(invalid_event(format!(
                    "short UHID SET_REPORT event: {} bytes",
                    buf.len()
                )));
            }
            let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let rnum = buf[8];
            let rtype = buf[9];
            let size = u16::from_le_bytes([buf[10], buf[11]]) as usize;
            if size > 4096 {
                return Err(invalid_event(format!("UHID SET_REPORT too large: {size} bytes")));
            }
            let end = 12 + size;
            if buf.len() < end {
                return Err(invalid_event(format!(
                    "short UHID SET_REPORT payload: {} bytes, need {end}",
                    buf.len()
                )));
            }
            UhidEvent::SetReport {
                id,
                rnum,
                rtype,
                data: buf[12..end].to_vec(),
            }
        }
        Ok(_) => UhidEvent::Unknown(ev_type),
        Err(_) => UhidEvent::Unknown(ev_type),
    };

    Ok(Some(ev))
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

        let total_size = off_rd_data + rd_data.len();
        write_all_fd(self.fd.as_raw_fd(), &buf[..total_size])?;

        debug!(
            "UHID create2: name={name}, rd_size={}, bus={bus}, vid={vendor:04x}, pid={product:04x}, written={total_size}",
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

        write_all_fd(self.fd.as_raw_fd(), &buf)?;

        info!("UHID destroy sent");
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

        let total_size = 6 + data.len();
        write_all_fd(self.fd.as_raw_fd(), &buf[..total_size])?;
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

        let total_size = 12 + data.len();
        if let Err(e) = write_all_fd(self.fd.as_raw_fd(), &buf[..total_size]) {
            log::error!("uhid GET_REPORT reply write failed: {e}");
            return Err(e);
        }
        Ok(())
    }

    pub fn send_set_report_reply(&self, id: u32, err: u16) -> io::Result<()> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        buf[0..4].copy_from_slice(&UhidEventType::SetReportReply.to_u32_le());
        buf[4..8].copy_from_slice(&id.to_le_bytes());
        buf[8..10].copy_from_slice(&err.to_le_bytes());

        if let Err(e) = write_all_fd(self.fd.as_raw_fd(), &buf[..10]) {
            log::error!("uhid SET_REPORT reply write failed: {e}");
            return Err(e);
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

        let ev = match parse_uhid_event(&buf[..n as usize])? {
            Some(ev) => ev,
            None => return Ok(None),
        };

        debug!("UHID recv: {ev:?}");
        Ok(Some(ev))
    }
}

impl Drop for UhidDevice {
    fn drop(&mut self) {
        if let Err(e) = crate::write_connected_state(false) {
            log::warn!("failed to write connected file: {e}");
        }
        let _ = self.destroy();
    }
}

impl UhidEventType {
    fn to_u32_le(self) -> [u8; 4] {
        (self as u32).to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_type_bytes(event_type: UhidEventType) -> [u8; 4] {
        event_type.to_u32_le()
    }

    #[test]
    fn parse_output_rejects_short_header() {
        let mut buf = vec![0u8; 4102];
        buf[0..4].copy_from_slice(&event_type_bytes(UhidEventType::Output));

        let err = parse_uhid_event(&buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn parse_output_rejects_oversized_payload() {
        let mut buf = vec![0u8; 4103];
        buf[0..4].copy_from_slice(&event_type_bytes(UhidEventType::Output));
        buf[4100..4102].copy_from_slice(&(4097u16).to_le_bytes());

        let err = parse_uhid_event(&buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn parse_set_report_rejects_short_payload() {
        let mut buf = vec![0u8; 13];
        buf[0..4].copy_from_slice(&event_type_bytes(UhidEventType::SetReport));
        buf[10..12].copy_from_slice(&(2u16).to_le_bytes());

        let err = parse_uhid_event(&buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn parse_set_report_accepts_exact_payload() {
        let mut buf = vec![0u8; 14];
        buf[0..4].copy_from_slice(&event_type_bytes(UhidEventType::SetReport));
        buf[4..8].copy_from_slice(&7u32.to_le_bytes());
        buf[8] = 0x05;
        buf[9] = 0;
        buf[10..12].copy_from_slice(&(2u16).to_le_bytes());
        buf[12..14].copy_from_slice(&[0xaa, 0xbb]);

        assert_eq!(
            parse_uhid_event(&buf).unwrap(),
            Some(UhidEvent::SetReport {
                id: 7,
                rnum: 0x05,
                rtype: 0,
                data: vec![0xaa, 0xbb],
            })
        );
    }
}
