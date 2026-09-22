use std::time::{Duration, Instant};

use crate::codec::HapticsFrame;
use crate::control::haptics::AudioFrame;

const FRAME_NUMERATOR_NS: u64 = HapticsFrame::FRAMES as u64 * 1_000_000_000;

const PCM_PERIOD: Duration = Duration::from_nanos(FRAME_NUMERATOR_NS.div_ceil(HapticsFrame::RATE));

fn parse_bt_haptics_buffer(value: &str) -> Result<u8, String> {
    match value.parse::<u8>() {
        Ok(buffer) if buffer != 0 => Ok(buffer),
        _ => Err(format!(
            "invalid DSEUHID_BT_HAPTICS_BUFFER={value}; expected decimal integer 1..=255"
        )),
    }
}

pub(crate) fn bt_haptics_buffer_from_env() -> Result<u8, String> {
    match std::env::var("DSEUHID_BT_HAPTICS_BUFFER") {
        Ok(value) => parse_bt_haptics_buffer(&value),
        Err(std::env::VarError::NotPresent) => Ok(crate::codec::DEFAULT_BT_HAPTICS_BUFFER),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("invalid DSEUHID_BT_HAPTICS_BUFFER: value is not valid Unicode".into())
        }
    }
}

#[derive(Default)]
pub(super) struct LiveHaptics {
    frames: std::collections::VecDeque<AudioFrame>,
    deadline: Option<Instant>,
}

impl LiveHaptics {
    pub(super) fn push(&mut self, frame: AudioFrame, now: Instant) {
        if self.deadline.is_none() && frame == AudioFrame::SILENCE {
            return;
        }
        // One block of initial buffering absorbs PipeWire quantum boundaries.
        let deadline = self.deadline.get_or_insert(now + PCM_PERIOD);
        if self.frames.len() == 3 {
            self.frames.pop_front();
            // The deadline belongs to the queue head, including after overflow.
            *deadline += PCM_PERIOD;
        }
        self.frames.push_back(frame);
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn take_due_frame(&mut self, now: Instant) -> Option<AudioFrame> {
        let deadline = self.deadline?;
        if now < deadline {
            return None;
        }
        let skipped = (now.duration_since(deadline).as_nanos() / PCM_PERIOD.as_nanos()) as usize;
        for _ in 0..skipped.min(self.frames.len()) {
            self.frames.pop_front();
        }
        if let Some(frame) = self.frames.pop_front() {
            if frame == AudioFrame::SILENCE && self.frames.iter().all(|f| *f == AudioFrame::SILENCE)
            {
                self.frames.clear();
                self.deadline = None;
            } else {
                self.deadline = Some(deadline + PCM_PERIOD * (skipped as u32 + 1));
            }
            Some(frame)
        } else {
            self.deadline = None;
            Some(AudioFrame::SILENCE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haptics_buffer_accepts_nonzero_bytes_in_decimal() {
        for (text, expected) in [("1", 1), ("32", 32), ("64", 64), ("255", 255)] {
            assert_eq!(parse_bt_haptics_buffer(text), Ok(expected));
        }
        for text in ["", "0", "256", "-1", "0x20", "32.0", "invalid"] {
            assert!(parse_bt_haptics_buffer(text).is_err(), "{text}");
        }
    }

    #[test]
    fn live_queue_skips_late_frames_and_stops_on_underrun() {
        let start = Instant::now();
        let mut live = LiveHaptics::default();
        live.push(AudioFrame::SILENCE, start);
        assert!(live.next_deadline().is_none());
        for i in 1..=3 {
            live.push(
                AudioFrame {
                    haptics: [i; 64],
                    speaker: None,
                },
                start,
            );
        }
        assert_eq!(live.frames.len(), 3);
        assert!(live.take_due_frame(start).is_none());
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD),
            Some(AudioFrame {
                haptics: [1; 64],
                speaker: None
            })
        );
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 3),
            Some(AudioFrame {
                haptics: [3; 64],
                speaker: None
            })
        );
        assert!(live.take_due_frame(start + PCM_PERIOD * 3).is_none());
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 4),
            Some(AudioFrame::SILENCE)
        );
        assert!(live.next_deadline().is_none());
        live.push(
            AudioFrame {
                haptics: [7; 64],
                speaker: None,
            },
            start + PCM_PERIOD * 5,
        );
        assert_eq!(
            live.take_due_frame(start + Duration::from_secs(10)),
            Some(AudioFrame::SILENCE)
        );
        assert!(live.next_deadline().is_none());
    }

    #[test]
    fn overflow_and_late_tick_preserve_current_and_future_audio_frames() {
        for speaker in [None, Some([0x5a; 200])] {
            let start = Instant::now();
            let mut live = LiveHaptics::default();
            live.push(
                AudioFrame {
                    haptics: [1; 64],
                    speaker,
                },
                start,
            );
            // The next epoll turn drains four arrivals before servicing its timer.
            let now = start + PCM_PERIOD * 4;
            for i in 2..=5 {
                live.push(
                    AudioFrame {
                        haptics: [i; 64],
                        speaker,
                    },
                    now,
                );
            }
            assert_eq!(live.frames.len(), 3);
            assert_eq!(live.next_deadline(), Some(start + PCM_PERIOD * 3));
            assert!(live.take_due_frame(start + PCM_PERIOD).is_none());
            assert_eq!(
                live.take_due_frame(now),
                Some(AudioFrame {
                    haptics: [4; 64],
                    speaker
                })
            );
            assert_eq!(live.next_deadline(), Some(start + PCM_PERIOD * 5));
            assert!(live.take_due_frame(now).is_none());
            assert_eq!(
                live.take_due_frame(start + PCM_PERIOD * 5),
                Some(AudioFrame {
                    haptics: [5; 64],
                    speaker
                })
            );
            assert_eq!(
                live.take_due_frame(start + PCM_PERIOD * 6),
                Some(AudioFrame::SILENCE)
            );
            assert!(live.next_deadline().is_none());
        }
    }

    #[test]
    fn continuous_idle_capture_sends_one_stop_and_releases_playback() {
        let start = Instant::now();
        let mut live = LiveHaptics::default();
        live.push(
            AudioFrame {
                haptics: [10; 64],
                speaker: None,
            },
            start,
        );
        live.take_due_frame(start + PCM_PERIOD).unwrap();
        live.push(AudioFrame::SILENCE, start + PCM_PERIOD);
        assert_eq!(
            live.take_due_frame(start + PCM_PERIOD * 2),
            Some(AudioFrame::SILENCE)
        );
        for _ in 0..10 {
            live.push(AudioFrame::SILENCE, start + PCM_PERIOD * 3);
        }
        assert!(live.next_deadline().is_none());
        assert!(live.frames.is_empty());
    }
}
