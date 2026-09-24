//! Hears the resident (issue #17): Silero VAD over the call's frames, an utterance cut at every
//! pause of [`UTTERANCE_GAP`], and each utterance transcribed and logged with how long after
//! the resident stopped its text was ready.
//!
//! A stand-in until turn-taking lands (issue #19): there, the endpointer from issue #12
//! decides when the resident's turn is over. Here every short pause closes an utterance, which
//! is also when that design asks the models.

use std::time::Instant;

use tokio::sync::mpsc;
use turn::vad::{Silero, WINDOW};
use turn::{Gate, WINDOW_S};

use crate::audio::CORE_RATE_HZ;
use crate::stt::Stt;
use crate::telephony::Frame;

/// Silence that closes an utterance. Long enough not to split words (Silero's gate can end
/// speech inside a word), short enough to leave whisper most of the 1 s budget (issue #17).
pub const UTTERANCE_GAP: f64 = 0.3;

/// Audio kept from before VAD onset, so the first syllable is not cut off.
const PRE_ROLL: f64 = 0.2;

/// One window at or above the speech threshold starts speech, as Silero's `VADIterator`.
const START_WINDOWS: u32 = 1;

const PRE_ROLL_SAMPLES: usize = (PRE_ROLL * CORE_RATE_HZ as f64) as usize;

/// What the resident said between two pauses. Times are seconds of media time from the start
/// of the call.
#[derive(Debug)]
pub struct Utterance {
    pub start: f64,
    pub end: f64,
    /// 16 kHz, from [`PRE_ROLL`] before `start` to [`UTTERANCE_GAP`] after `end`.
    pub audio: Vec<i16>,
}

/// Cuts the resident's audio into utterances. Pure: fed one VAD window at a time, with the
/// window's speech probability.
pub struct Segmenter {
    gate: Gate,
    /// Start time of the next window.
    t: f64,
    /// Start time of the open utterance, if any.
    open: Option<f64>,
    buffer: Vec<i16>,
}

impl Default for Segmenter {
    fn default() -> Self {
        Segmenter { gate: Gate::new(START_WINDOWS), t: 0.0, open: None, buffer: Vec::new() }
    }
}

impl Segmenter {
    /// Media time at the end of the last window pushed.
    pub fn now(&self) -> f64 {
        self.t
    }

    pub fn push(&mut self, window: &[i16], p: f32) -> Option<Utterance> {
        let t = self.t;
        self.t += WINDOW_S;
        self.buffer.extend_from_slice(window);
        if let Some(turn::GateEvent::SpeechStart(start)) = self.gate.push(t, p) {
            self.open.get_or_insert(start);
        }
        match self.open {
            Some(start) if !self.gate.speaking && self.gate.silence(t) >= UTTERANCE_GAP => {
                self.open = None;
                let audio = std::mem::take(&mut self.buffer);
                Some(Utterance { start, end: self.gate.since, audio })
            }
            Some(_) => None,
            None => {
                let excess = self.buffer.len().saturating_sub(PRE_ROLL_SAMPLES + WINDOW);
                self.buffer.drain(..excess);
                None
            }
        }
    }
}

/// Listens to one call until its audio ends. `call` names the call in the log.
pub async fn listen(call: String, mut frames: mpsc::Receiver<Frame>, mut vad: Silero, stt: Stt) {
    let mut segmenter = Segmenter::default();
    let mut pending: Vec<i16> = Vec::with_capacity(WINDOW * 2);
    while let Some(frame) = frames.recv().await {
        pending.extend_from_slice(&frame);
        while pending.len() >= WINDOW {
            let window: Vec<i16> = pending.drain(..WINDOW).collect();
            let samples: Vec<f32> = window.iter().map(|&s| s as f32 / 32_768.0).collect();
            // About 0.1 ms a window on this laptop (issue #12), so it runs inline.
            let p = match vad.prob(&samples) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("agent: {call}: VAD failed, no longer listening: {e}");
                    return;
                }
            };
            let Some(utterance) = segmenter.push(&window, p) else { continue };
            // Frames arrive in real time, so the resident stopped this long before now
            // (inferred: it holds only while the line isn't delivering a backlog).
            let stopped = Instant::now()
                - std::time::Duration::from_secs_f64(segmenter.now() - utterance.end);
            let (stt, call) = (stt.clone(), call.clone());
            tokio::spawn(async move {
                let span = format!("{:.2}-{:.2} s", utterance.start, utterance.end);
                match stt.transcribe(utterance.audio).await {
                    Ok(t) => eprintln!(
                        "agent: {call}: heard {span}: {:?} (text {} ms after the resident \
                         stopped; whisper {} ms)",
                        t.text,
                        stopped.elapsed().as_millis(),
                        t.took.as_millis(),
                    ),
                    Err(e) => eprintln!("agent: {call}: heard {span}, transcription failed: {e}"),
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pushes `n` windows of probability `p`; returns what came out.
    fn feed(s: &mut Segmenter, n: usize, p: f32) -> Vec<Utterance> {
        (0..n).filter_map(|_| s.push(&[0; WINDOW], p)).collect()
    }

    fn windows(seconds: f64) -> usize {
        (seconds / WINDOW_S).round() as usize
    }

    #[test]
    fn silence_alone_yields_nothing_and_keeps_only_the_pre_roll() {
        let mut s = Segmenter::default();
        assert!(feed(&mut s, windows(5.0), 0.0).is_empty());
        assert!(s.buffer.len() <= PRE_ROLL_SAMPLES + WINDOW);
    }

    #[test]
    fn a_pause_of_the_gap_closes_the_utterance() {
        let mut s = Segmenter::default();
        feed(&mut s, windows(1.0), 0.0);
        feed(&mut s, windows(2.0), 0.9);
        let out = feed(&mut s, windows(UTTERANCE_GAP) + 1, 0.0);
        assert_eq!(out.len(), 1);
        let u = &out[0];
        assert!((u.start - 0.992).abs() < 1e-9, "start {}", u.start);
        assert!((u.end - 3.008).abs() < 1e-9, "end {}", u.end);
        // Pre-roll + speech + the gap, in whole windows.
        let expected = (u.end - u.start + PRE_ROLL + UTTERANCE_GAP) * CORE_RATE_HZ as f64;
        assert!((u.audio.len() as f64 - expected).abs() <= 2.0 * WINDOW as f64);
    }

    #[test]
    fn a_shorter_pause_keeps_one_utterance() {
        let mut s = Segmenter::default();
        feed(&mut s, windows(1.0), 0.9);
        assert!(feed(&mut s, windows(UTTERANCE_GAP) - 2, 0.0).is_empty());
        feed(&mut s, windows(1.0), 0.9);
        let out = feed(&mut s, windows(UTTERANCE_GAP) + 1, 0.0);
        assert_eq!(out.len(), 1);
        assert!(out[0].start < 0.05 && out[0].end > 2.0);
    }
}
