use std::io;
use std::os::fd::RawFd;

use nix::sys::socket::{recv, send, MsgFlags};

use super::MAX_PACKET_SIZE;

pub(super) fn send_packet(fd: RawFd, packet: &[u8]) -> io::Result<()> {
    if packet.len() > MAX_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "control packet exceeds size limit",
        ));
    }
    match send(fd, packet, MsgFlags::MSG_NOSIGNAL | MsgFlags::MSG_DONTWAIT) {
        Ok(written) if written == packet.len() => Ok(()),
        Ok(written) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("short seqpacket write: {written} of {}", packet.len()),
        )),
        Err(error) => Err(error.into()),
    }
}

pub(super) enum RecvPacket {
    Data(Vec<u8>),
    Closed,
    WouldBlock,
}

pub(super) fn recv_packet(fd: RawFd) -> io::Result<RecvPacket> {
    let mut buffer = vec![0u8; MAX_PACKET_SIZE];
    match recv(
        fd,
        &mut buffer,
        MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_TRUNC,
    ) {
        Ok(0) => Ok(RecvPacket::Closed),
        Ok(size) if size > buffer.len() => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control packet exceeds size limit",
        )),
        Ok(size) => {
            buffer.truncate(size);
            Ok(RecvPacket::Data(buffer))
        }
        Err(nix::errno::Errno::EAGAIN) => Ok(RecvPacket::WouldBlock),
        Err(error) => Err(error.into()),
    }
}
