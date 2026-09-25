//! The telephony seam (issue #13). The agent's core sees a call's audio only through [`Media`]
//! and never sees AudioSocket, channel names or the line's format. Call control is
//! [`CallControl`]: hanging up, and transferring to the dispatcher through AMI (issue #20);
//! placing calls lands with outbound check-ins.
//!
//! Media is a plain handle of channels rather than a trait: an adapter builds one per call and
//! runs its own task behind it. A Twilio adapter would build the same handle.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use ami::{Ami, Channel};

use crate::audio::{CORE_FRAME, CORE_RATE_HZ};

pub mod ami;
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
    Hangup,
}

/// Ends and redirects calls. Cheap to clone.
#[derive(Clone)]
pub struct CallControl {
    commands: mpsc::UnboundedSender<Command>,
    pbx: Pbx,
}

/// The PBX side of Call control: what finds the call's channel and transfers it.
#[derive(Clone)]
enum Pbx {
    /// AMI isn't configured, so a transfer fails.
    None,
    /// Asterisk's manager interface, and the call's AudioSocket UUID, which finds its channel.
    Ami(Ami, String),
    /// The eval's stand-in for Asterisk.
    Recorder(Recorder),
}

/// Stands in for the PBX in the eval (issue #23): the caller's extension is known up front, and
/// each transfer is handed to whoever plays Asterisk, which ends the call's AudioSocket as
/// Asterisk does after a `Redirect`.
#[derive(Clone)]
pub struct Recorder {
    pub caller: String,
    pub transfers: mpsc::UnboundedSender<String>,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

const NO_AMI: &str = "AMI is not configured (.env, AMI_*)";

impl CallControl {
    pub(crate) fn new(commands: mpsc::UnboundedSender<Command>) -> Self {
        Self { commands, pbx: Pbx::None }
    }

    /// Lets this call be transferred through `ami`. `uuid` is the call's AudioSocket UUID.
    pub fn with_ami(mut self, ami: Ami, uuid: String) -> Self {
        self.pbx = Pbx::Ami(ami, uuid);
        self
    }

    /// Hands this call's transfers to `recorder` instead of a PBX.
    pub fn with_recorder(mut self, recorder: Recorder) -> Self {
        self.pbx = Pbx::Recorder(recorder);
        self
    }

    /// The call's channel on the PBX, with the caller's extension.
    pub async fn channel(&self) -> Result<Channel, Error> {
        match &self.pbx {
            Pbx::None => Err(NO_AMI.into()),
            Pbx::Ami(ami, uuid) => ami.find_channel(uuid).await,
            Pbx::Recorder(recorder) => Ok(Channel {
                name: "eval".to_string(),
                caller: Some(recorder.caller.clone()),
            }),
        }
    }

    /// Transfers the live call to `extension`, an internal extension. Returns what the PBX
    /// said; success means the call was handed off, not that anyone answered.
    pub async fn transfer(&self, extension: &str) -> Result<String, Error> {
        match &self.pbx {
            Pbx::None => Err(NO_AMI.into()),
            Pbx::Ami(ami, _) => {
                let channel = self.channel().await?;
                ami.redirect(&channel.name, extension).await
            }
            Pbx::Recorder(recorder) => {
                recorder.transfers.send(extension.to_string())?;
                Ok("recorded by the eval".to_string())
            }
        }
    }

    /// Hangs up at once, dropping any audio not yet written to the line. To let the last
    /// words finish, wait for a mark queued after them first.
    pub fn hangup(&self) {
        let _ = self.commands.send(Command::Hangup);
    }
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
