//! Local PCM transport. One datagram is a monotonic timestamp followed by
//! 32 stereo signed-8-bit samples at 3 kHz, optionally followed by one Opus
//! speaker frame. It is never a physical HID report.
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::{fs::PermissionsExt, net::UnixDatagram};
use std::path::{Path, PathBuf};

pub const HAPTICS_SOCKET: &str = "haptics.sock";
pub const PCM_SAMPLES: usize = 64;
pub const OPUS_BYTES: usize = 200;
const PACKET_SIZE: usize = 8 + PCM_SAMPLES;
const AUDIO_PACKET_SIZE: usize = PACKET_SIZE + OPUS_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    pub haptics: [i8; PCM_SAMPLES],
    pub speaker: Option<[u8; OPUS_BYTES]>,
}

impl AudioFrame {
    pub const SILENCE: Self = Self {
        haptics: [0; PCM_SAMPLES],
        speaker: None,
    };
}
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
    send_audio(
        socket,
        &AudioFrame {
            haptics: *samples,
            speaker: None,
        },
    )
}

pub fn send_audio(socket: &UnixDatagram, frame: &AudioFrame) -> io::Result<()> {
    let mut packet = [0u8; AUDIO_PACKET_SIZE];
    packet[..8].copy_from_slice(&monotonic_ns()?.to_le_bytes());
    for (dst, src) in packet[8..PACKET_SIZE].iter_mut().zip(frame.haptics.iter()) {
        *dst = *src as u8;
    }
    let size = if let Some(speaker) = &frame.speaker {
        packet[PACKET_SIZE..].copy_from_slice(speaker);
        AUDIO_PACKET_SIZE
    } else {
        PACKET_SIZE
    };
    socket.send(&packet[..size])?;
    Ok(())
}

fn decode(packet: &[u8], now: u64) -> Option<AudioFrame> {
    if packet.len() != PACKET_SIZE && packet.len() != AUDIO_PACKET_SIZE {
        return None;
    }
    let timestamp = u64::from_le_bytes(packet[..8].try_into().ok()?);
    if now.checked_sub(timestamp)? > MAX_AGE_NS {
        return None;
    }
    Some(AudioFrame {
        haptics: std::array::from_fn(|i| packet[8 + i] as i8),
        speaker: (packet.len() == AUDIO_PACKET_SIZE)
            .then(|| packet[PACKET_SIZE..].try_into().unwrap()),
    })
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

    pub fn drain(&self, mut accept: impl FnMut(AudioFrame)) -> io::Result<()> {
        // Bound work per epoll turn; the fd stays readable if packets remain.
        for _ in 0..16 {
            let mut packet = [0u8; AUDIO_PACKET_SIZE + 1];
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
        assert_eq!(
            decode(&packet, 100),
            Some(AudioFrame {
                haptics: [-1; PCM_SAMPLES],
                speaker: None
            })
        );
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
        assert_eq!(
            received,
            [AudioFrame {
                haptics: samples,
                speaker: None
            }]
        );
        let audio = AudioFrame {
            haptics: samples,
            speaker: Some([0xab; OPUS_BYTES]),
        };
        send_audio(&sender, &audio).unwrap();
        let mut received = Vec::new();
        receiver.drain(|p| received.push(p)).unwrap();
        assert_eq!(received, [audio]);
        sender.send(&[0; AUDIO_PACKET_SIZE + 1]).unwrap();
        receiver
            .drain(|_| panic!("oversized datagram accepted"))
            .unwrap();
        drop(receiver);
        assert!(!dir.join(HAPTICS_SOCKET).exists());
        std::fs::remove_dir(dir).unwrap();
    }
}
