//! Turn detection: VAD, end-of-turn detectors and the barge-in policy.
//!
//! Shared by the turn-detection prototype harness and the live agent, so the parameters
//! tuned in the prototype are the ones that ship (issue #25).

pub mod vad;

/// Silero window at 16 kHz: 512 samples = 32 ms (Silero `utils_vad.py`, `OnnxWrapper.__call__`).
pub const WINDOW_S: f64 = 0.032;

/// Speech when p >= 0.5; speech ends when p < 0.35 (Silero `VADIterator`: threshold - 0.15).
pub const VAD_ON: f32 = 0.5;
pub const VAD_OFF: f32 = 0.35;

/// VAD hysteresis shared by every detector: speech starts after `start_windows` windows
/// at or above [`VAD_ON`], and ends at the first window below [`VAD_OFF`].
#[derive(Clone, Debug)]
pub struct Gate {
    pub start_windows: u32,
    run: u32,
    pub speaking: bool,
    /// Start time of the first window of the current speech or silence run.
    pub since: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GateEvent {
    /// Speech started; the value is when (start of its first window).
    SpeechStart(f64),
    /// Speech stopped; the value is when silence began.
    SpeechEnd(f64),
}

impl Gate {
    pub fn new(start_windows: u32) -> Self {
        Gate { start_windows, run: 0, speaking: false, since: 0.0 }
    }

    /// `t` is the start time of this window.
    pub fn push(&mut self, t: f64, p: f32) -> Option<GateEvent> {
        if self.speaking {
            if p < VAD_OFF {
                self.speaking = false;
                self.since = t;
                return Some(GateEvent::SpeechEnd(t));
            }
        } else if p >= VAD_ON {
            self.run += 1;
            if self.run >= self.start_windows {
                self.speaking = true;
                self.run = 0;
                self.since = t - (self.start_windows - 1) as f64 * WINDOW_S;
                return Some(GateEvent::SpeechStart(self.since));
            }
        } else {
            self.run = 0;
        }
        None
    }

    /// Seconds of silence at the end of window `t`, or 0 while speaking.
    pub fn silence(&self, t: f64) -> f64 {
        if self.speaking { 0.0 } else { t + WINDOW_S - self.since }
    }
}
