//! Smart Turn, asked whether the resident has finished (issue #19, the rule from issue #12):
//! the resident's turn ends after [`TURN_END`](crate::conversation::TURN_END) of silence,
//! unless Smart Turn, asked at [`ASK_AT`], says it is very unlikely to be over; then the agent
//! waits up to [`HOLD_TO`]. The model can only add waiting time, never take the turn early:
//! on the prototype's audio it was reliable in only that one direction (0 of 16 "um" pauses
//! called complete).
//!
//! Like speech to text, one worker thread runs the model for every call.

use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use turn::smart_turn::SmartTurn;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Silence after which Smart Turn is asked (issue #12).
pub const ASK_AT: f64 = 0.2;
/// P(complete) below this holds the floor (issue #12).
pub const HOLD_BELOW: f32 = 0.05;
/// How long a held turn waits (issue #12).
pub const HOLD_TO: f64 = 3.0;
/// Audio from before the turn that Smart Turn is given too (issue #12, after Pipecat's
/// `pre_speech_ms`).
pub const PRE_SPEECH: f64 = 0.5;
/// The most audio Smart Turn looks at (`turn::smart_turn::SECONDS`).
pub const WINDOW_S: f64 = turn::smart_turn::SECONDS as f64;

pub struct Verdict {
    /// P(the turn is complete).
    pub p: f32,
    pub took: Duration,
}

type Job = (Vec<f32>, oneshot::Sender<Result<Verdict, Error>>);

/// A handle to the worker thread. Cheap to clone.
#[derive(Clone)]
pub struct EndOfTurn {
    jobs: std::sync::mpsc::Sender<Job>,
}

impl EndOfTurn {
    pub fn start(mut model: SmartTurn) -> Self {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (audio, reply) in queue {
                let started = Instant::now();
                let result = model
                    .predict(&audio)
                    .map(|p| Verdict { p, took: started.elapsed() })
                    .map_err(Error::from);
                let _ = reply.send(result);
            }
        });
        EndOfTurn { jobs }
    }

    /// Asks about 16 kHz audio in [-1, 1]: the turn so far, ending now.
    pub async fn ask(&self, audio: Vec<f32>) -> Result<Verdict, Error> {
        let (reply, answer) = oneshot::channel();
        self.jobs.send((audio, reply)).map_err(|_| "the Smart Turn thread has stopped")?;
        answer.await.map_err(|_| "the Smart Turn thread has stopped")?
    }
}
