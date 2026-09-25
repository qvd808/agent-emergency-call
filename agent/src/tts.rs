//! Text to speech: Piper through piper-rs, on the CPU (issue #25). The agent's voice is
//! `en_US-hfc_female-medium`, picked by ear by the maintainer from samples of six Piper voices
//! as a softphone hears them.
//!
//! Each reply is spoken in a [`Tone`] that follows the turn's status: warm for small talk and
//! good news, slower and steadier once something is wrong. Piper has no emotion or pitch
//! control, so a tone is its three sampling settings plus a reshaping of the pitch after
//! synthesis ([`prosody`](crate::prosody)).
//!
//! The reply's punctuation is put back before Piper sees it (issue #38). piper-rs 0.2.0
//! phonemizes through espeak-rs 0.2.0, which here returns no clause punctuation at all:
//! "Oh no, I'm sorry to hear that." became `ˈoʊ nˈoʊaɪm sˈɑːɹi tə hˈɪɹ ðˈæt`, and "Hello.
//! This is" became `həlˈoʊðɪs`. The voice never saw a comma, a full stop or a question mark,
//! and sentences ran into each other. So the text is split into clauses here and each
//! clause's mark is added back, as Piper's own phonemizer does (rhasspy/piper-phonemize@ba3cc06,
//! `src/phonemize.cpp:108-124`), and each sentence is synthesised on its own with a pause
//! after it, as Piper itself does (rhasspy/piper@73c04d8, `src/cpp/piper.cpp:481`).
//!
//! Like speech-to-text, one worker thread synthesises for every call: Piper blocks, and the
//! task that runs a call has to keep reading its frames.

use std::path::Path;
use std::time::{Duration, Instant};

use piper_rs::Piper;
use tokio::sync::oneshot;

use crate::audio::{CORE_RATE_HZ, resample_clip};
use crate::prosody::{self, Shape};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// How a line is delivered. Each is Piper's (length_scale, noise_scale, noise_w): speaking
/// time, how much the pitch and energy vary, and how much the phoneme lengths vary. The voice's
/// own defaults are 1.0, 0.667 and 0.8 (`en_US-hfc_female-medium.onnx.json`).
///
/// Piper's settings barely move the pitch: in five runs over six check-in lines, measured with
/// Praat, Warm and Steady were never more than 0.34 semitones apart in how much their pitch
/// varied (issue #37). So each tone also has a [`Shape`], and that is what makes them sound
/// different. Both shapes are chosen from those measurements and not yet heard on a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 10% slower than the voice's default, with more variation in pitch and timing: the
    /// "warm" sample the maintainer picked. Its pitch then rises and falls half as much again,
    /// a semitone higher: 5.2-5.4 semitones of variation instead of Piper's 3.6-3.8.
    Warm,
    /// Slower still, for a reply to something worrying and for the escalation script. Its
    /// pitch sits half a semitone lower and moves a little more than Piper's (4.0-4.1
    /// semitones instead of 3.7-3.9), so it sounds calm rather than flat.
    Steady,
}

impl Tone {
    fn settings(self) -> (f32, f32, f32, Shape) {
        match self {
            Tone::Warm => (1.1, 0.8, 0.9, Shape { shift_semitones: 1.0, range: 1.5 }),
            Tone::Steady => (1.2, 0.6, 0.7, Shape { shift_semitones: -0.5, range: 1.1 }),
        }
    }
}

/// Silence between sentences: Piper's own default, `sentenceSilenceSeconds = 0.2f`
/// (rhasspy/piper@73c04d8, `src/cpp/piper.hpp:67`).
const SENTENCE_SILENCE_S: f64 = 0.2;

pub struct Voice {
    piper: Piper,
    /// The espeak-ng voice the model was trained with, `espeak.voice` in its config.
    language: String,
}

impl Voice {
    /// `model` is the `.onnx` file; its `.onnx.json` config sits next to it.
    pub fn load(model: &str) -> Result<Self, Error> {
        let config = format!("{model}.json");
        let piper = Piper::new(Path::new(model), Path::new(&config))?;
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config)?)?;
        let language =
            json["espeak"]["voice"].as_str().ok_or("the voice config has no espeak.voice")?.to_string();
        Ok(Voice { piper, language })
    }

    /// Speaks `text` as 16 kHz audio, the core's rate.
    pub fn speak(&mut self, text: &str, tone: Tone) -> Result<Vec<i16>, Error> {
        let (length, noise, noise_w, shape) = tone.settings();
        let mut samples: Vec<i16> = Vec::new();
        let mut rate = CORE_RATE_HZ;
        for (i, sentence) in self.phonemize(text)?.iter().enumerate() {
            let (audio, r) =
                self.piper.create(sentence, true, None, Some(length), Some(noise), Some(noise_w))?;
            rate = r;
            if i > 0 {
                samples.extend(std::iter::repeat_n(0, (SENTENCE_SILENCE_S * rate as f64) as usize));
            }
            samples.extend(audio.iter().map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16));
        }
        let core = resample_clip(&samples, rate, CORE_RATE_HZ);
        Ok(prosody::reshape(&core, CORE_RATE_HZ, shape))
    }

    /// The phonemes of each sentence, with every clause's punctuation kept.
    fn phonemize(&self, text: &str) -> Result<Vec<String>, Error> {
        let mut sentences = Vec::new();
        let mut sentence = String::new();
        for (words, mark) in clauses(text) {
            let phonemes = espeak_rs::text_to_phonemes(&words, &self.language, None)?.join(" ");
            if phonemes.trim().is_empty() {
                continue;
            }
            sentence.push_str(phonemes.trim());
            sentence.push(mark);
            if matches!(mark, '.' | '?' | '!') {
                sentences.push(std::mem::take(&mut sentence));
            } else {
                sentence.push(' ');
            }
        }
        Ok(sentences)
    }
}

/// Splits text into clauses, each with the mark that ends it: `.`, `?` and `!` end a sentence,
/// `,`, `:` and `;` a clause inside one, the marks piper-phonemize keeps. Text after the last
/// mark ends with a full stop, and a run of marks such as "..." counts once.
fn clauses(text: &str) -> Vec<(String, char)> {
    let mut clauses = Vec::new();
    let mut words = String::new();
    for c in text.chars() {
        if matches!(c, '.' | '?' | '!' | ',' | ':' | ';') {
            if !words.trim().is_empty() {
                clauses.push((words.trim().to_string(), c));
            }
            words.clear();
        } else {
            words.push(c);
        }
    }
    if !words.trim().is_empty() {
        clauses.push((words.trim().to_string(), '.'));
    }
    clauses
}

pub struct Speech {
    /// 16 kHz.
    pub audio: Vec<i16>,
    /// Time Piper and the resampler spent on it, not counting the wait for the worker.
    pub took: Duration,
}

type Job = (String, Tone, oneshot::Sender<Result<Speech, Error>>);

/// A handle to the worker thread. Cheap to clone.
#[derive(Clone)]
pub struct Tts {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl Tts {
    pub fn start(mut voice: Voice) -> Self {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (text, tone, reply) in queue {
                let started = Instant::now();
                let result = voice
                    .speak(&text, tone)
                    .map(|audio| Speech { audio, took: started.elapsed() });
                let _ = reply.send(result);
            }
        });
        Tts { jobs }
    }

    pub async fn speak(&self, text: &str, tone: Tone) -> Result<Speech, Error> {
        let (reply, answer) = oneshot::channel();
        self.jobs
            .send((text.to_string(), tone, reply))
            .map_err(|_| "the speech thread has stopped")?;
        answer.await.map_err(|_| "the speech thread has stopped")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(clauses: &[(&str, char)]) -> Vec<(String, char)> {
        clauses.iter().map(|&(w, c)| (w.to_string(), c)).collect()
    }

    #[test]
    fn each_clause_keeps_its_mark() {
        assert_eq!(
            clauses("Oh no, I'm sorry to hear that. Were you able to get up by yourself?"),
            owned(&[
                ("Oh no", ','),
                ("I'm sorry to hear that", '.'),
                ("Were you able to get up by yourself", '?'),
            ])
        );
    }

    #[test]
    fn a_run_of_marks_counts_once() {
        assert_eq!(clauses("Oh no... I see!?"), owned(&[("Oh no", '.'), ("I see", '!')]));
    }

    #[test]
    fn unfinished_text_ends_with_a_full_stop() {
        assert_eq!(clauses("Take care, and goodbye"), owned(&[("Take care", ','), ("and goodbye", '.')]));
        assert_eq!(clauses("  "), owned(&[]));
    }
}
