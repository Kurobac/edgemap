//! Local PCM transport. One datagram is a monotonic timestamp followed by
//! 32 stereo signed-8-bit samples at 3 kHz; it is never a physical HID report.
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::{fs::PermissionsExt, net::UnixDatagram};
use std::path::{Path, PathBuf};

pub const HAPTICS_SOCKET: &str = "haptics.sock";
pub const PCM_SAMPLES: usize = 64;
const PACKET_SIZE: usize = 8 + PCM_SAMPLES;
const MAX_AGE_NS: u64 = 100_000_000;

fn monotonic_ns() -> io::Result<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64)
}

pub fn send_pcm(socket: &UnixDatagram, samples: &[i8; PCM_SAMPLES]) -> io::Result<()> {
    let mut packet = [0u8; PACKET_SIZE];
    packet[..8].copy_from_slice(&monotonic_ns()?.to_le_bytes());
    for (dst, src) in packet[8..].iter_mut().zip(samples) {
        *dst = *src as u8;
    }
    socket.send(&packet)?;
    Ok(())
}

fn decode(packet: &[u8], now: u64) -> Option<[i8; PCM_SAMPLES]> {
    if packet.len() != PACKET_SIZE {
        return None;
    }
    let timestamp = u64::from_le_bytes(packet[..8].try_into().ok()?);
    if now.checked_sub(timestamp)? > MAX_AGE_NS {
        return None;
    }
    Some(std::array::from_fn(|i| packet[8 + i] as i8))
}

pub struct PcmReceiver {
    socket: UnixDatagram,
    path: PathBuf,
}

impl PcmReceiver {
    pub fn bind(dir: &Path) -> io::Result<Self> {
        let path = dir.join(HAPTICS_SOCKET);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let receiver = Self {
            socket: UnixDatagram::bind(&path)?,
            path,
        };
        receiver.socket.set_nonblocking(true)?;
        std::fs::set_permissions(&receiver.path, std::fs::Permissions::from_mode(0o666))?;
        Ok(receiver)
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }

    pub fn drain(&self, mut accept: impl FnMut([i8; PCM_SAMPLES])) -> io::Result<()> {
        // Bound work per epoll turn; the fd stays readable if packets remain.
        for _ in 0..16 {
            let mut packet = [0u8; PACKET_SIZE + 1];
            match self.socket.recv(&mut packet) {
                Ok(n) => {
                    if let Some(samples) = decode(&packet[..n], monotonic_ns()?) {
                        accept(samples);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl Drop for PcmReceiver {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_size_stale_and_future_packets() {
        let mut packet = [255; PACKET_SIZE];
        packet[..8].copy_from_slice(&100u64.to_le_bytes());
        assert_eq!(decode(&packet, 100), Some([-1; PCM_SAMPLES]));
        assert!(decode(&packet, 99).is_none());
        assert!(decode(&packet, MAX_AGE_NS + 101).is_none());
        assert!(decode(&packet[..PACKET_SIZE - 1], 100).is_none());
        assert!(decode(&[0; PACKET_SIZE + 1], 100).is_none());
    }

    #[test]
    fn datagrams_round_trip_and_socket_is_removed() {
        let dir = std::env::temp_dir().join(format!("edgemap-pcm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let receiver = PcmReceiver::bind(&dir).unwrap();
        let sender = UnixDatagram::unbound().unwrap();
        sender.connect(dir.join(HAPTICS_SOCKET)).unwrap();
        let samples = std::array::from_fn(|i| i as i8 - 32);
        send_pcm(&sender, &samples).unwrap();
        let mut received = Vec::new();
        receiver.drain(|p| received.push(p)).unwrap();
        assert_eq!(received, [samples]);
        drop(receiver);
        assert!(!dir.join(HAPTICS_SOCKET).exists());
        std::fs::remove_dir(dir).unwrap();
    }
}
