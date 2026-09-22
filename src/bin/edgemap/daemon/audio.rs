use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

use dseuhid::control::haptics::{send_audio, send_pcm, AudioFrame, PCM_SAMPLES};

mod speaker;
use dseuhid::control::HapticsDevice;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

const SINK_NAME: &str = "edgemap.dualsense";
const INPUT_RATE: usize = 48_000;
const DECIMATION: usize = 16;
const TAPS: usize = 255;

// Windowed-sinc low-pass before 16:1 decimation. Compute the convolution only
// for retained samples; the front pair never enters the filter history.
struct RearPcm {
    coefficients: [f32; TAPS],
    history: [[f32; 2]; TAPS],
    cursor: usize,
    phase: usize,
}

impl RearPcm {
    fn new() -> Self {
        let mut coefficients = std::array::from_fn(|i| {
            let x = i as f64 - (TAPS / 2) as f64;
            let cutoff = 1250.0 / INPUT_RATE as f64;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (std::f64::consts::TAU * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            let window = 0.54 - 0.46 * (std::f64::consts::TAU * i as f64 / (TAPS - 1) as f64).cos();
            (sinc * window) as f32
        });
        let gain: f32 = coefficients.iter().sum();
        for coefficient in &mut coefficients {
            *coefficient /= gain;
        }
        Self {
            coefficients,
            history: [[0.0; 2]; TAPS],
            cursor: 0,
            phase: 0,
        }
    }

    fn push(&mut self, quad: &[u8]) -> Option<[i8; 2]> {
        for ch in 0..2 {
            let offset = 8 + ch * 4;
            let sample = f32::from_le_bytes(quad[offset..offset + 4].try_into().unwrap());
            self.history[self.cursor][ch] = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
        self.cursor = (self.cursor + 1) % TAPS;
        self.phase += 1;
        if self.phase != DECIMATION {
            return None;
        }
        self.phase = 0;
        let mut stereo = [0.0f32; 2];
        for (i, coefficient) in self.coefficients.iter().enumerate() {
            let sample = self.history[(self.cursor + i) % TAPS];
            for ch in 0..2 {
                stereo[ch] += coefficient * sample[ch];
            }
        }
        Some(stereo.map(|s| (s * 128.0).round().clamp(-128.0, 127.0) as i8))
    }
}

struct Capture(Child);
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn capture_command(sink_name: &str, device: HapticsDevice) -> Command {
    let name = device.product_name();
    let pid = device.product_id();
    let properties = format!(
        "media.class=Audio/Sink node.name={sink_name} \
         node.description=\"{name}\" node.nick=\"{name}\" \
         device.description=\"{name}\" device.product.name=\"{name}\" \
         device.vendor.name=\"Sony Corp.\" device.vendor.id=0x054c \
         device.product.id=0x{pid:04x} device.bus=usb device.class=sound \
         device.form-factor=controller node.virtual=true priority.session=0 \
         node.pause-on-idle=true stream.dont-remix=true channelmix.upmix=false"
    );
    let mut command = Command::new("pw-cat");
    command
        .args([
            "--record",
            "--raw",
            "--target",
            "0",
            "--rate",
            "48000",
            "--channels",
            "4",
            "--channel-map",
            "FL,FR,RL,RR",
            "--format",
            "f32",
            "--latency",
            "512",
            "--properties",
            &properties,
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    dseuhid::shutdown::unblock_shutdown_signals_in_child(&mut command);
    command
}

fn run_capture(
    stop: UnixStream,
    socket: UnixDatagram,
    sink_name: &str,
    device: HapticsDevice,
    speaker_demo: bool,
) -> io::Result<()> {
    let mut speaker = speaker_demo.then(speaker::Speaker::new).transpose()?;
    socket.set_nonblocking(true)?;
    let mut capture = Capture(capture_command(sink_name, device).spawn()?);
    let mut output = capture.0.stdout.take().expect("piped pw-cat output");
    let mut filter = RearPcm::new();
    let mut frame = [0i8; PCM_SAMPLES];
    let mut frame_used = 0;
    let mut bytes = [0u8; 8192 + 16];
    let mut used = 0;
    log::info!("Bluetooth haptics capture started: sink={sink_name}, quad 48000 Hz");
    let result = (|| {
        loop {
            let mut fds = [
                PollFd::new(stop.as_fd(), PollFlags::POLLIN),
                PollFd::new(output.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut fds, PollTimeout::NONE) {
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            }
            if !fds[0].revents().unwrap_or(PollFlags::empty()).is_empty() {
                return Ok(());
            }
            let events = fds[1].revents().unwrap_or(PollFlags::empty());
            if events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
                return Err(io::Error::other("pw-cat capture exited"));
            }
            if !events.contains(PollFlags::POLLIN) {
                continue;
            }
            let n = output.read(&mut bytes[used..8192])?;
            if n == 0 {
                return Err(io::Error::other("pw-cat capture reached EOF"));
            }
            used += n;
            let complete = used / 16 * 16;
            for quad in bytes[..complete].chunks_exact(16) {
                if let Some(speaker) = &mut speaker {
                    speaker.push(quad);
                }
                if let Some(stereo) = filter.push(quad) {
                    frame[frame_used..frame_used + 2].copy_from_slice(&stereo);
                    frame_used += 2;
                    if frame_used == PCM_SAMPLES {
                        let speaker = match &mut speaker {
                            Some(speaker) => speaker.finish_block()?,
                            None => None,
                        };
                        match send_audio(
                            &socket,
                            &AudioFrame {
                                haptics: frame,
                                speaker,
                            },
                        ) {
                            Ok(()) => {}
                            // Drop a late block instead of stalling the audio graph.
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            // A connected Unix datagram returns ECONNREFUSED
                            // when its receiver closes. Proxy teardown can
                            // precede the daemon's control-state notification.
                            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                                log::info!("Bluetooth PCM receiver closed; ending audio session");
                                return Ok(());
                            }
                            Err(e) => return Err(e),
                        }
                        frame_used = 0;
                    }
                }
            }
            bytes.copy_within(complete..used, 0);
            used -= complete;
        }
    })();
    let _ = send_pcm(&socket, &[0; PCM_SAMPLES]);
    result
}

struct AudioBridge {
    stop: UnixStream,
    worker: Option<JoinHandle<()>>,
}

impl AudioBridge {
    fn start(device: HapticsDevice) -> io::Result<Self> {
        let (stop, worker_stop) = UnixStream::pair()?;
        let worker = std::thread::Builder::new()
            .name("bt-haptics".into())
            .spawn(move || {
                let result = (|| {
                    let socket = UnixDatagram::unbound()?;
                    socket.connect("/run/dseuhid/haptics.sock")?;
                    run_capture(
                        worker_stop,
                        socket,
                        SINK_NAME,
                        device,
                        std::env::var("EDGEMAP_SPEAKER_DEMO").as_deref() == Ok("1"),
                    )
                })();
                if let Err(error) = result {
                    log::error!("Bluetooth audio bridge stopped: {error}");
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for AudioBridge {
    fn drop(&mut self) {
        let _ = self.stop.shutdown(std::net::Shutdown::Both);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        log::info!("Bluetooth haptics sink destroyed");
    }
}

#[derive(Default)]
pub(super) struct AudioManager {
    device: Option<HapticsDevice>,
    bridge: Option<AudioBridge>,
}

impl AudioManager {
    pub(super) fn update(&mut self, device: Option<HapticsDevice>) {
        if device == self.device {
            return;
        }
        self.device = device;
        self.bridge = None;
        if let Some(device) = device {
            match AudioBridge::start(device) {
                Ok(bridge) => self.bridge = Some(bridge),
                Err(error) => log::error!("cannot start Bluetooth audio bridge: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires libopus.so.0 to be unavailable in an isolated filesystem"]
    fn missing_opus_rejects_speaker_capture_with_dependency_error() {
        let (_stop, worker_stop) = UnixStream::pair().unwrap();
        let (_receiver, sender) = UnixDatagram::pair().unwrap();
        let error = run_capture(
            worker_stop,
            sender,
            "edgemap.test-missing-opus",
            HapticsDevice::DualSenseEdge,
            true,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Bluetooth speaker requires libopus.so.0"),
            "{error}"
        );
        eprintln!("{error}");
    }

    #[test]
    #[ignore = "requires a live user PipeWire session and pw-cat/pactl"]
    fn pipewire_quad_capture_to_pcm_and_sink_cleanup() {
        for device in [HapticsDevice::DualSense, HapticsDevice::DualSenseEdge] {
            verify_pipewire_capture(device, false, false);
        }
    }

    #[test]
    #[ignore = "requires a live user PipeWire session and pw-cat/pactl"]
    fn pipewire_receiver_close_ends_capture_cleanly() {
        verify_pipewire_capture(HapticsDevice::DualSenseEdge, true, false);
    }

    #[test]
    #[ignore = "requires a live user PipeWire session and pw-cat/pactl"]
    fn pipewire_speaker_and_haptics_capture_together() {
        verify_pipewire_capture(HapticsDevice::DualSenseEdge, false, true);
    }

    fn verify_pipewire_capture(device: HapticsDevice, close_receiver: bool, speaker_demo: bool) {
        use std::io::Write;
        use std::time::{Duration, Instant};
        let name = format!(
            "edgemap.test-{}-{}",
            std::process::id(),
            if close_receiver {
                "close"
            } else if speaker_demo {
                "speaker"
            } else {
                "identity"
            }
        );
        let (receiver, sender) = UnixDatagram::pair().unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (stop, worker_stop) = UnixStream::pair().unwrap();
        let thread_name = name.clone();
        let worker = std::thread::spawn(move || {
            run_capture(worker_stop, sender, &thread_name, device, speaker_demo)
        });
        // Always stop/join the capture, even when an assertion below fails.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let sinks = Command::new("pactl")
                    .args(["list", "short", "sinks"])
                    .output()
                    .unwrap();
                let listing = String::from_utf8_lossy(&sinks.stdout);
                if let Some(line) = listing.lines().find(|line| line.contains(&name)) {
                    assert!(line.contains("4ch 48000Hz"), "{line}");
                    break;
                }
                assert!(Instant::now() < deadline, "sink was not published");
                std::thread::sleep(Duration::from_millis(50));
            }
            let sinks = Command::new("pactl")
                .env("LC_ALL", "C")
                .args(["list", "sinks"])
                .output()
                .unwrap();
            assert!(sinks.status.success());
            let listing = String::from_utf8_lossy(&sinks.stdout);
            let sink = listing
                .split("\nSink #")
                .find(|sink| sink.contains(&name))
                .unwrap();
            let (pid, product) = match device {
                HapticsDevice::DualSense => ("0x0ce6", "DualSense Wireless Controller"),
                HapticsDevice::DualSenseEdge => ("0x0df2", "DualSense Edge Wireless Controller"),
            };
            for (key, value) in [
                ("device.bus", "usb"),
                ("device.vendor.id", "0x054c"),
                ("device.product.id", pid),
                ("device.vendor.name", "Sony Corp."),
                ("device.product.name", product),
                ("device.description", product),
                ("node.nick", product),
                ("device.class", "sound"),
                ("device.form_factor", "controller"),
                ("node.virtual", "true"),
            ] {
                assert!(
                    sink.contains(&format!("{key} = \"{value}\"")),
                    "missing {key}={value}: {sink}"
                );
            }
            assert!(sink.contains(&format!("Description: {product}")), "{sink}");
            eprintln!(
                "PulseAudio identity verified: {product}, USB VID=0x054c PID={pid}, controller"
            );
            let mut playback = Capture(
                Command::new("pw-cat")
                    .args([
                        "--playback",
                        "--raw",
                        "--target",
                        &name,
                        "--rate",
                        "48000",
                        "--channels",
                        "4",
                        "--channel-map",
                        "FL,FR,RL,RR",
                        "--format",
                        "f32",
                        "-",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            let mut stdin = playback.0.stdin.take().unwrap();
            let writer = std::thread::spawn(move || {
                for i in 0..48000 {
                    let tone = (std::f32::consts::TAU * 100.0 * i as f32 / 48000.0).sin() * 0.25;
                    let front = if speaker_demo { tone } else { 0.0 };
                    let values = match i / 12000 {
                        0 => [tone, tone, 0.0, 0.0],
                        1 => [front, front, tone, 0.0],
                        2 => [front, front, 0.0, tone],
                        _ => [0.0; 4],
                    };
                    stdin.write_all(&quad(values)).unwrap();
                }
            });
            let mut energy = [0u64; 2];
            let mut silent_frames = 0;
            let mut speaker_frames = 0;
            let mut combined_frames = 0;
            let mut active_channels = [false; 2];
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let mut packet = [0u8; 273];
                match receiver.recv(&mut packet) {
                    Ok(n) => {
                        assert!(n == 72 || (speaker_demo && n == 272), "packet size {n}");
                        speaker_frames += usize::from(n == 272);
                        let mut active = [false; 2];
                        for stereo in packet[8..72].chunks_exact(2) {
                            for ch in 0..2 {
                                let sample = stereo[ch] as i8;
                                energy[ch] += sample.unsigned_abs() as u64;
                                active[ch] |= sample != 0;
                            }
                        }
                        combined_frames += usize::from(n == 272 && active != [false; 2]);
                        if active == [false; 2] {
                            silent_frames += 1;
                        }
                        for ch in 0..2 {
                            active_channels[ch] |= active[ch] && !active[1 - ch];
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) => panic!("{e}"),
                }
            }
            writer.join().unwrap();
            if speaker_demo {
                assert!(speaker_frames >= 60, "speaker frames: {speaker_frames}");
                assert!(combined_frames >= 30, "combined frames: {combined_frames}");
                eprintln!("speaker capture: Opus frames={speaker_frames}, simultaneous HD={combined_frames}");
            } else {
                assert_eq!(speaker_frames, 0);
            }
            assert!(energy.iter().all(|e| *e > 5000), "{energy:?}");
            assert_eq!(active_channels, [true, true]);
            assert!(silent_frames >= 10, "{silent_frames}");
            eprintln!("quad capture verified: energy={energy:?}, silent frames={silent_frames}");
            if close_receiver {
                // Reproduce a proxy session ending before its control-state
                // notification has stopped the capture worker.
                drop(receiver);
                let deadline = Instant::now() + Duration::from_secs(2);
                while !worker.is_finished() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    worker.is_finished(),
                    "capture did not end after PCM peer closed"
                );
            }
        }));
        stop.shutdown(std::net::Shutdown::Both).unwrap();
        worker.join().unwrap().unwrap();
        let sinks = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .unwrap();
        assert!(!String::from_utf8_lossy(&sinks.stdout).contains(&name));
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    pub(super) fn quad(values: [f32; 4]) -> [u8; 16] {
        let mut bytes = [0; 16];
        for (dst, value) in bytes.chunks_exact_mut(4).zip(values) {
            dst.copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn rear_channels_only_and_exact_decimation() {
        let mut filter = RearPcm::new();
        let output: Vec<_> = (0..48000)
            .filter_map(|_| filter.push(&quad([1.0, -1.0, 0.5, -0.5])))
            .collect();
        assert_eq!(output.len(), 3000);
        assert!(output[100..].iter().all(|s| *s == [64, -64]));
        let mut filter = RearPcm::new();
        assert!((0..1024)
            .filter_map(|_| filter.push(&quad([1.0, 1.0, 0.0, 0.0])))
            .all(|s| s == [0, 0]));
    }

    #[test]
    fn lowpass_retains_haptics_and_rejects_aliasing() {
        for (frequency, min_rms, max_rms) in [(100.0, 44.0, 46.0), (2500.0, 0.0, 0.5)] {
            let mut filter = RearPcm::new();
            let samples: Vec<_> = (0..48000)
                .filter_map(|i| {
                    let s = (std::f32::consts::TAU * frequency * i as f32 / 48000.0).sin() * 0.5;
                    filter.push(&quad([0.0, 0.0, s, 0.0]))
                })
                .skip(100)
                .collect();
            let rms = (samples.iter().map(|s| (s[0] as f64).powi(2)).sum::<f64>()
                / samples.len() as f64)
                .sqrt();
            assert!((min_rms..=max_rms).contains(&rms), "{frequency}: {rms}");
            assert!(samples.iter().all(|s| s[1] == 0));
        }
    }
}
