//! DualSense's speaker consumes 480 Opus samples in each 512/48000 s interval.
//! Match mdrv-ds's 16:15 resampling and 200-byte, 160 kbit/s CBR payload.
use std::io;
use std::ptr::NonNull;

use dseuhid::control::haptics::OPUS_BYTES;

#[link(name = "opus")]
unsafe extern "C" {
    fn opus_encoder_create(
        rate: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> *mut libc::c_void;
    fn opus_encoder_destroy(encoder: *mut libc::c_void);
    fn opus_encoder_ctl(encoder: *mut libc::c_void, request: i32, ...) -> i32;
    fn opus_encode_float(
        encoder: *mut libc::c_void,
        pcm: *const f32,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;
}

pub(super) struct Speaker {
    encoder: NonNull<libc::c_void>,
    front: [[f32; 2]; 512],
    used: usize,
    silent_tail: usize,
}

impl Speaker {
    pub(super) fn new() -> io::Result<Self> {
        let mut error = 0;
        // OPUS_APPLICATION_AUDIO. The object stays on the capture thread.
        let raw = unsafe { opus_encoder_create(48000, 2, 2049, &mut error) };
        let encoder = NonNull::new(raw)
            .ok_or_else(|| io::Error::other(format!("Opus encoder creation failed: {error}")))?;
        let speaker = Self {
            encoder,
            front: [[0.0; 2]; 512],
            used: 0,
            silent_tail: 0,
        };
        // OPUS_SET_BITRATE, OPUS_SET_VBR. libopus's public ABI uses int varargs.
        for (request, value) in [(4002, 160_000i32), (4006, 0i32)] {
            let result = unsafe { opus_encoder_ctl(speaker.encoder.as_ptr(), request, value) };
            if result != 0 {
                return Err(io::Error::other(format!(
                    "Opus ctl {request} failed: {result}"
                )));
            }
        }
        Ok(speaker)
    }

    pub(super) fn push(&mut self, quad: &[u8]) {
        for ch in 0..2 {
            let sample = f32::from_le_bytes(quad[ch * 4..ch * 4 + 4].try_into().unwrap());
            self.front[self.used][ch] = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
        self.used += 1;
    }

    pub(super) fn finish_block(&mut self) -> io::Result<Option<[u8; OPUS_BYTES]>> {
        assert_eq!(self.used, 512);
        self.used = 0;
        if self.front.iter().flatten().any(|&s| s != 0.0) {
            self.silent_tail = 8;
        } else if self.silent_tail != 0 {
            self.silent_tail -= 1;
        } else {
            return Ok(None);
        }
        let mut pcm = [0.0; 480 * 2];
        for (i, stereo) in pcm.chunks_exact_mut(2).enumerate() {
            // Integer phase avoids drift between the speaker and haptics clocks.
            let index = i * 16 / 15;
            let fraction = (i * 16 % 15) as f32 / 15.0;
            for (ch, sample) in stereo.iter_mut().enumerate() {
                *sample =
                    self.front[index][ch] * (1.0 - fraction) + self.front[index + 1][ch] * fraction;
            }
        }
        let mut packet = [0; OPUS_BYTES];
        let size = unsafe {
            opus_encode_float(
                self.encoder.as_ptr(),
                pcm.as_ptr(),
                480,
                packet.as_mut_ptr(),
                OPUS_BYTES as i32,
            )
        };
        if size != OPUS_BYTES as i32 {
            return Err(io::Error::other(format!(
                "Opus frame must be 200 bytes, got {size}"
            )));
        }
        Ok(Some(packet))
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        // The pointer is exclusively owned, and initialized by libopus.
        unsafe { opus_encoder_destroy(self.encoder.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" {
        fn opus_decoder_create(rate: i32, channels: i32, error: *mut i32) -> *mut libc::c_void;
        fn opus_decoder_destroy(decoder: *mut libc::c_void);
        fn opus_decode_float(
            decoder: *mut libc::c_void,
            data: *const u8,
            len: i32,
            pcm: *mut f32,
            frame_size: i32,
            fec: i32,
        ) -> i32;
    }

    #[test]
    fn opus_round_trip_preserves_front_channels_and_controller_clock() {
        let mut speaker = Speaker::new().unwrap();
        let mut error = 0;
        let decoder = unsafe { opus_decoder_create(48000, 2, &mut error) };
        assert!(!decoder.is_null());
        let mut decoded = Vec::new();
        for block in 0..100 {
            for i in 0..512 {
                let t = (block * 512 + i) as f32 / 48000.0;
                let left = (std::f32::consts::TAU * 750.0 * t).sin() * 0.2;
                let right = (std::f32::consts::TAU * 1500.0 * t).sin() * 0.2;
                speaker.push(&super::super::tests::quad([left, right, 1.0, -1.0]));
            }
            let packet = speaker.finish_block().unwrap().unwrap();
            let mut pcm = [0.0; 960];
            assert_eq!(
                unsafe {
                    opus_decode_float(decoder, packet.as_ptr(), 200, pcm.as_mut_ptr(), 480, 0)
                },
                480
            );
            decoded.extend(pcm.chunks_exact(2).map(|s| [s[0], s[1]]));
        }
        unsafe { opus_decoder_destroy(decoder) };
        // Hardware renders at 45 kHz: 512 input frames -> 480 decoded frames.
        // Count crossings after encoder warmup, measuring at that physical clock.
        let tail = &decoded[4800..];
        for (ch, expected) in [(0, 750.0), (1, 1500.0)] {
            let crossings = tail
                .windows(2)
                .filter(|w| w[0][ch] <= 0.0 && w[1][ch] > 0.0)
                .count();
            let frequency = crossings as f64 * 45000.0 / tail.len() as f64;
            assert!(
                (frequency - expected).abs() < 3.0,
                "channel {ch}: {frequency}"
            );
            let rms = (tail.iter().map(|s| (s[ch] as f64).powi(2)).sum::<f64>()
                / tail.len() as f64)
                .sqrt();
            assert!((0.1..0.2).contains(&rms), "channel {ch}: {rms}");
        }
    }

    #[test]
    fn idle_rear_only_input_stays_idle_and_front_flushes_then_stops() {
        let mut speaker = Speaker::new().unwrap();
        for i in 0..12 {
            for _ in 0..512 {
                let front = if i == 1 { 0.1 } else { 0.0 };
                speaker.push(&super::super::tests::quad([front, front, 1.0, -1.0]));
            }
            assert_eq!(
                speaker.finish_block().unwrap().is_some(),
                (1..=9).contains(&i)
            );
        }
    }
}
