use std::time::{Duration, Instant};

use crate::codec::HapticsFrame;

const TONE_SAMPLES: u64 = HapticsFrame::RATE;
const RIGHT_START: u64 = TONE_SAMPLES + HapticsFrame::RATE / 2;
const TOTAL_SAMPLES: u64 = RIGHT_START + TONE_SAMPLES;
const SILENCE_FRAME: u64 = TOTAL_SAMPLES.div_ceil(HapticsFrame::FRAMES as u64);
const FRAME_NUMERATOR_NS: u64 = HapticsFrame::FRAMES as u64 * 1_000_000_000;

const PCM_PERIOD: Duration = Duration::from_nanos(FRAME_NUMERATOR_NS.div_ceil(HapticsFrame::RATE));

#[derive(Default)]
pub(super) struct LiveHaptics {
    frames: std::collections::VecDeque<HapticsFrame>,
    deadline: Option<Instant>,
}

impl LiveHaptics {
    pub(super) fn push(&mut self, frame: HapticsFrame, now: Instant) {
        if self.deadline.is_none() && frame == HapticsFrame::SILENCE {
            return;
        }
        if self.frames.len() == 3 {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
        // One block of initial buffering absorbs PipeWire quantum boundaries.
        self.deadline.get_or_insert(now + PCM_PERIOD);
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn take_due_frame(&mut self, now: Instant) -> Option<HapticsFrame> {
        let deadline = self.deadline?;
        if now < deadline {
            return None;
        }
        let skipped = (now.duration_since(deadline).as_nanos() / PCM_PERIOD.as_nanos()) as usize;
        for _ in 0..skipped.min(self.frames.len()) {
            self.frames.pop_front();
        }
        if let Some(frame) = self.frames.pop_front() {
            if frame == HapticsFrame::SILENCE
                && self.frames.iter().all(|f| *f == HapticsFrame::SILENCE)
            {
                self.frames.clear();
                self.deadline = None;
            } else {
                self.deadline = Some(deadline + PCM_PERIOD * (skipped as u32 + 1));
            }
            Some(frame)
        } else {
            self.deadline = None;
            Some(HapticsFrame::SILENCE)
        }
    }
}

/// Finite PCM producer for hardware testing. Absolute sample time keeps a
/// late wakeup from replaying old haptics or extending the test indefinitely.
pub(super) struct HapticsDemo {
    start: Instant,
    next_frame: u64,
}

impl HapticsDemo {
    pub(super) fn new(start: Instant) -> Self {
        Self {
            start,
            next_frame: 0,
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        (self.next_frame <= SILENCE_FRAME).then(|| {
            self.start
                + Duration::from_nanos(
                    (self.next_frame * FRAME_NUMERATOR_NS).div_ceil(HapticsFrame::RATE),
                )
        })
    }

    pub(super) fn take_due_frame(&mut self, now: Instant) -> Option<HapticsFrame> {
        if now < self.next_deadline()? {
            return None;
        }
        let elapsed_frame = (now.duration_since(self.start).as_nanos() * HapticsFrame::RATE as u128
            / FRAME_NUMERATOR_NS as u128)
            .min(SILENCE_FRAME as u128) as u64;
        let frame_index = self.next_frame.max(elapsed_frame);
        self.next_frame = frame_index + 1;
        let mut frame = HapticsFrame::SILENCE;
        for (i, stereo) in frame.0.chunks_exact_mut(2).enumerate() {
            let sample = frame_index * HapticsFrame::FRAMES as u64 + i as u64;
            let (channel, local_sample) = if sample < TONE_SAMPLES {
                (0, sample)
            } else if (RIGHT_START..TOTAL_SAMPLES).contains(&sample) {
                (1, sample - RIGHT_START)
            } else {
                continue;
            };
            // 75 Hz, peak 24/127, with 10 ms ramps to avoid edge clicks.
            let ramp = local_sample.min(TONE_SAMPLES - 1 - local_sample).min(30) as f64 / 30.0;
            let phase =
                std::f64::consts::TAU * 75.0 * local_sample as f64 / HapticsFrame::RATE as f64;
            stereo[channel] = (phase.sin() * 24.0 * ramp).round() as i8;
        }
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_queue_bounds_latency_skips_late_frames_and_stops_on_underrun() {
        let start = Instant::now();
        let mut live = LiveHaptics::default();
        live.push(HapticsFrame::SILENCE, start);
        assert!(live.next_deadline().is_none());
        for i in 1..=5 {
            live.push(HapticsFrame([i; 64]), start);
        }
        assert_eq!(live.frames.len(), 3);
        assert!(live.take_due_frame(start).is_none());
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD),
            Some(HapticsFrame([3; 64]))
        );
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 3),
            Some(HapticsFrame([5; 64]))
        );
        assert!(live.take_due_frame(start + PCM_PERIOD * 3).is_none());
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 4),
            Some(HapticsFrame::SILENCE)
        );
        assert!(live.next_deadline().is_none());
        live.push(HapticsFrame([7; 64]), start + PCM_PERIOD * 5);
        assert_eq!(
            live.take_due_frame(start + Duration::from_secs(10)),
            Some(HapticsFrame::SILENCE)
        );
        assert!(live.next_deadline().is_none());
    }

    #[test]
    fn continuous_idle_capture_sends_one_stop_and_releases_playback() {
        let start = Instant::now();
        let mut live = LiveHaptics::default();
        live.push(HapticsFrame([10; 64]), start);
        live.take_due_frame(start + PCM_PERIOD).unwrap();
        live.push(HapticsFrame::SILENCE, start + PCM_PERIOD);
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 2),
            Some(HapticsFrame::SILENCE)
        );
        for _ in 0..10 {
            live.push(HapticsFrame::SILENCE, start + PCM_PERIOD * 3);
        }
        assert!(live.next_deadline().is_none());
        assert!(live.frames.is_empty());
    }

    #[test]
    fn demo_separates_channels_and_finishes_with_silence() {
        let start = Instant::now();
        let mut demo = HapticsDemo::new(start);
        let mut count = 0;
        let mut energy = [0u64; 2];
        let mut last = None;
        while let Some(deadline) = demo.next_deadline() {
            let frame = demo.take_due_frame(deadline).unwrap();
            for (i, stereo) in frame.0.chunks_exact(2).enumerate() {
                let sample = count * HapticsFrame::FRAMES as u64 + i as u64;
                if sample < TONE_SAMPLES {
                    assert_eq!(stereo[1], 0);
                } else if (RIGHT_START..TOTAL_SAMPLES).contains(&sample) {
                    assert_eq!(stereo[0], 0);
                } else {
                    assert_eq!(stereo, &[0, 0]);
                }
                for ch in 0..2 {
                    assert!(stereo[ch].abs() <= 24);
                    energy[ch] += stereo[ch].unsigned_abs() as u64;
                }
            }
            last = Some(frame);
            count += 1;
        }
        assert_eq!(count, SILENCE_FRAME + 1);
        assert_eq!(last, Some(HapticsFrame::SILENCE));
        assert!(energy[0] > 0);
        assert_eq!(energy[0], energy[1]);
        assert!(demo
            .take_due_frame(start + Duration::from_secs(10))
            .is_none());
    }

    #[test]
    fn late_wakeups_skip_old_samples_without_bursting() {
        let start = Instant::now();
        let mut demo = HapticsDemo::new(start);
        demo.take_due_frame(start).unwrap();
        assert_eq!(
            demo.next_deadline(),
            Some(start + Duration::from_nanos(10_666_667))
        );
        assert!(demo
            .take_due_frame(start + Duration::from_millis(5))
            .is_none());
        let now = start + Duration::from_millis(1700);
        let frame = demo.take_due_frame(now).unwrap();
        assert!(frame.0.chunks_exact(2).all(|stereo| stereo[0] == 0));
        assert!(frame.0.chunks_exact(2).any(|stereo| stereo[1] != 0));
        assert!(demo.next_deadline().unwrap() > now);
        assert!(demo.take_due_frame(now).is_none());
        assert_eq!(
            demo.take_due_frame(start + Duration::from_secs(5)),
            Some(HapticsFrame::SILENCE)
        );
        assert!(demo.next_deadline().is_none());
    }
}
