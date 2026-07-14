use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::Path;

use nix::sys::socket::{connect, socket, AddressFamily, SockFlag, SockType, UnixAddr};

use super::protocol::{parse_server_packet, ControlRequest, ServerPacket};
use super::transport::{recv_packet, send_packet, RecvPacket};

pub struct ControlClient {
    fd: OwnedFd,
}

impl ControlClient {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
            None,
        )?;
        let address = UnixAddr::new(path)?;
        connect(fd.as_raw_fd(), &address)?;
        Ok(Self { fd })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn send_request(&self, request: &ControlRequest) -> io::Result<()> {
        send_packet(self.fd.as_raw_fd(), &request.encode())
    }

    pub fn receive(&self) -> io::Result<Option<ServerPacket>> {
        match recv_packet(self.fd.as_raw_fd())? {
            RecvPacket::Data(packet) => parse_server_packet(&packet)
                .map(Some)
                .map_err(|message| io::Error::new(io::ErrorKind::InvalidData, message)),
            RecvPacket::Closed => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "dseuhid control socket closed",
            )),
            RecvPacket::WouldBlock => Ok(None),
        }
    }
}
