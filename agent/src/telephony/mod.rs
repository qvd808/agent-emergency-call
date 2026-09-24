//! The telephony seam (issue #13). The agent's core sees a call's audio only through [`Media`]
//! and never sees AudioSocket, channel names or the line's format. Call control (placing,
//! transferring and hanging up calls) lands here once the core needs it.
//!
//! Media is a plain handle of channels rather than a trait: an adapter builds one per call and
//! runs its own task behind it. A Twilio adapter would build the same handle.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::audio::{CORE_FRAME, CORE_RATE_HZ};

pub mod audiosocket;
pub mod line;

/// 20 ms of the resident's audio at 16 kHz.
pub type Frame = [i16; CORE_FRAME];

/// Names a point in the agent's outgoing audio. The adapter reports it once everything queued
/// before it has been written to the line.
pub type MarkId = u64;

/// One call's audio, as the core sees it.
pub struct Media {
    /// The resident's audio: exactly 20 ms per frame, with silence filled in if the line goes
    /// quiet, so frames × 20 ms is media time. `None` once the call's audio has ended.
    pub frames: mpsc::Receiver<Frame>,
    /// Marks, in order, as their audio is written to the line.
    pub played: mpsc::UnboundedReceiver<MarkId>,
    pub speaker: Speaker,
}

/// Sends the agent's audio to the resident. Cheap to clone.
#[derive(Clone)]
pub struct Speaker {
    commands: mpsc::UnboundedSender<Command>,
}

/// What [`Speaker::clear`] dropped: audio and marks that had been queued but not yet
/// written to the line. The dropped marks are never reported as played.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cleared {
    pub dropped: Duration,
    pub dropped_marks: Vec<MarkId>,
}

pub(crate) enum Command {
    Play(Vec<i16>),
    Mark(MarkId),
    Clear(oneshot::Sender<Cleared>),
}

impl Speaker {
    pub(crate) fn new(commands: mpsc::UnboundedSender<Command>) -> Self {
        Self { commands }
    }

    /// Queues 16 kHz audio of any length behind what is already queued. The adapter sends it
    /// in real time, 20 ms every 20 ms. Does nothing once the call has ended.
    pub fn play(&self, audio: Vec<i16>) {
        let _ = self.commands.send(Command::Play(audio));
    }

    /// Queues a mark behind the audio queued so far.
    pub fn mark(&self, id: MarkId) {
        let _ = self.commands.send(Command::Mark(id));
    }

    /// Drops whatever audio has not been written to the line yet, for barge-in. Up to one
    /// frame already inside the resampler still goes out. After the call has ended there is
    /// nothing to drop.
    pub async fn clear(&self) -> Cleared {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::Clear(reply)).is_err() {
            return Cleared::default();
        }
        answer.await.unwrap_or_default()
    }
}

/// How long `samples` at 16 kHz last.
pub fn core_duration(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / CORE_RATE_HZ as f64)
}
