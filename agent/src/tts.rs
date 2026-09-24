//! Text to speech: Piper through piper-rs, on the CPU (issue #25). The agent's voice is
//! `en_US-lessac-medium`, the agent voice in the turn-detection prototype (issue #12).
//!
//! Like speech-to-text, one worker thread synthesises for every call: Piper blocks, and the
//! task that runs a call has to keep reading its frames.

use std::path::Path;
use std::time::{Duration, Instant};

use piper_rs::Piper;
use tokio::sync::oneshot;

use crate::audio::{CORE_RATE_HZ, resample_clip};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct Voice {
    piper: Piper,
}

impl Voice {
    /// `model` is the `.onnx` file; its `.onnx.json` config sits next to it.
    pub fn load(model: &str) -> Result<Self, Error> {
        let config = format!("{model}.json");
        let piper = Piper::new(Path::new(model), Path::new(&config))?;
        Ok(Voice { piper })
    }

    /// Speaks `text` as 16 kHz audio, the core's rate.
    pub fn speak(&mut self, text: &str) -> Result<Vec<i16>, Error> {
        let (samples, rate) = self.piper.create(text, false, None, None, None, None)?;
        let samples: Vec<i16> = samples
            .iter()
            .map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
            .collect();
        Ok(resample_clip(&samples, rate, CORE_RATE_HZ))
    }
}

pub struct Speech {
    /// 16 kHz.
    pub audio: Vec<i16>,
    /// Time Piper and the resampler spent on it, not counting the wait for the worker.
    pub took: Duration,
}

type Job = (String, oneshot::Sender<Result<Speech, Error>>);

/// A handle to the worker thread. Cheap to clone.
#[derive(Clone)]
pub struct Tts {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl Tts {
    pub fn start(mut voice: Voice) -> Self {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (text, reply) in queue {
                let started = Instant::now();
                let result =
                    voice.speak(&text).map(|audio| Speech { audio, took: started.elapsed() });
                let _ = reply.send(result);
            }
        });
        Tts { jobs }
    }

    pub async fn speak(&self, text: &str) -> Result<Speech, Error> {
        let (reply, answer) = oneshot::channel();
        self.jobs.send((text.to_string(), reply)).map_err(|_| "the speech thread has stopped")?;
        answer.await.map_err(|_| "the speech thread has stopped")?
    }
}
