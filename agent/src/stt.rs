//! Speech to text: whisper.cpp through whisper-rs, on the CPU (issue #17), or on an NVIDIA GPU
//! with the `cuda` feature (issue #52).
//!
//! One worker thread transcribes for every call, one utterance at a time. Whisper blocks for
//! hundreds of milliseconds, so it can't run on the task reading a call's frames: the line
//! only holds a second of audio for the core.

use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio::CORE_RATE_HZ;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Fastest of 4, 8, 12 and 16 threads for `tiny.en` on this laptop (issue #17).
pub const THREADS: i32 = 8;

/// What the call is about, given to whisper as its initial prompt. A word heard between two
/// readings then leans to the call's topics: on a live call's clips, "I haven't had a fault
/// lately" became "a fall" with it, for tiny.en and large-v3-turbo alike (issue #52).
const TOPICS: &str = "An automated check-in call with an older person who lives alone. It asks \
    how they feel, whether they have had a fall, any pain, whether they have eaten today, and \
    whether they need anything.";

pub struct Whisper {
    context: WhisperContext,
}

impl Whisper {
    pub fn load(model: &str) -> Result<Self, Error> {
        let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())?;
        Ok(Whisper { context })
    }

    /// Transcribes 16 kHz audio. Whisper always encodes a 30 s window: shrinking it to the
    /// clip's length (`audio_ctx`) was about 4x faster but made whisper repeat phrases on
    /// about a third of the turns tried (issue #17).
    pub fn transcribe(&self, audio: &[i16]) -> Result<String, Error> {
        let mut clip: Vec<f32> = audio.iter().map(|&s| s as f32 / 32_768.0).collect();
        // whisper.cpp wants at least 1 s; pad short clips with silence (inferred need,
        // carried over from the turn-detection prototype).
        let min = CORE_RATE_HZ as usize * 11 / 10;
        if clip.len() < min {
            clip.resize(min, 0.0);
        }
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_n_threads(THREADS);
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_initial_prompt(TOPICS);
        let mut state = self.context.create_state()?;
        state.full(params, &clip)?;
        let mut text = String::new();
        for i in 0..state.full_n_segments() {
            if let Some(segment) = state.get_segment(i) {
                text.push_str(&segment.to_str_lossy()?);
            }
        }
        Ok(text.trim().to_string())
    }
}

pub struct Transcript {
    pub text: String,
    /// Time whisper spent on it, not counting the wait for the worker.
    pub took: Duration,
}

type Job = (Vec<i16>, oneshot::Sender<Result<Transcript, Error>>);

/// A handle to the worker thread. Cheap to clone.
#[derive(Clone)]
pub struct Stt {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl Stt {
    pub fn start(whisper: Whisper) -> Self {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (audio, reply) in queue {
                let started = Instant::now();
                let result = whisper
                    .transcribe(&audio)
                    .map(|text| Transcript { text, took: started.elapsed() });
                let _ = reply.send(result);
            }
        });
        Stt { jobs }
    }

    pub async fn transcribe(&self, audio: Vec<i16>) -> Result<Transcript, Error> {
        let (reply, answer) = oneshot::channel();
        self.jobs.send((audio, reply)).map_err(|_| "the speech-to-text thread has stopped")?;
        answer.await.map_err(|_| "the speech-to-text thread has stopped")?
    }
}
