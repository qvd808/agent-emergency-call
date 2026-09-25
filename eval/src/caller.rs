//! Stands in for Asterisk and the resident on one call (issue #23). It speaks AudioSocket to
//! the agent exactly as Asterisk does: the UUID first, then one 20 ms frame of 8 kHz audio
//! every 20 ms, the resident's voice or silence. A transfer or a hang-up ends it the way
//! Asterisk ends a call, by closing the socket.
//!
//! It listens to the agent only for loudness: the agent is talking while its frames are loud.
//! The resident says their next line once the agent has been quiet for [`RESPOND_AFTER`], and
//! everything is timed as the resident hears it.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agent::audio::{LINE_FRAME, LINE_RATE_HZ, samples_from_le_bytes, samples_to_le_bytes};
use agent::telephony::audiosocket::{Decoder, Message, Uuid, encode_audio, encode_uuid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// How long the agent must be quiet before the resident answers. Longer than the pause Piper
/// leaves between sentences (0.2 s, `tts.rs`), so a reply isn't answered halfway.
const RESPOND_AFTER: Duration = Duration::from_millis(1000);

/// A frame of the agent's with a sample this loud is the agent talking. The agent's silence is
/// exact zeros, and Piper's speech peaks in the thousands (inferred from 16-bit full scale; not
/// measured).
const LOUD: u16 = 500;

/// Quiet this long between two loud frames starts a new run of the agent's speech.
const QUIET_GAP: Duration = Duration::from_millis(300);

/// A call still going after this long is hung up by the resident.
const MAX_CALL: Duration = Duration::from_secs(180);

const TICK: Duration = Duration::from_millis(20);

/// A scripted line, as audio on the line's 8 kHz.
#[derive(Clone)]
pub struct Rendered {
    /// The line as scripted, pauses included.
    pub text: String,
    pub samples: Vec<i16>,
    pub pauses: Vec<PauseAt>,
}

/// A pause inside a line.
#[derive(Clone)]
pub struct PauseAt {
    /// Sample offset of its first silent sample.
    pub start: usize,
    pub seconds: f64,
    /// What came before it: comma, filler or sentence.
    pub before: &'static str,
}

pub enum Act {
    Say(Rendered),
    Interrupt(f64, Rendered),
    Silent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The agent sent the hangup message.
    AgentHungUp,
    /// The agent closed the socket.
    AgentClosed,
    /// The agent transferred the call; the socket was closed as Asterisk does.
    Transferred,
    /// The resident hung up after [`MAX_CALL`].
    TimedOut,
}

/// One line the resident said, timed. Times are Unix milliseconds, to line up with the call log.
pub struct LineResult {
    pub text: String,
    pub interrupt: bool,
    pub started_ms: f64,
    pub ended_ms: Option<f64>,
    pub pauses: Vec<PauseResult>,
    /// From the end of the line to the agent's next speech, as the resident hears it.
    pub reply_ms: Option<f64>,
    /// For a line said over the agent: from the resident starting to the agent's last loud
    /// frame, once it has gone quiet. `None` if the agent had already stopped.
    pub agent_stopped_ms: Option<f64>,
}

pub struct PauseResult {
    pub seconds: f64,
    pub before: &'static str,
    pub start_ms: f64,
}

pub struct Outcome {
    pub lines: Vec<LineResult>,
    pub ended: Ended,
    pub transfer: Option<String>,
    pub duration: Duration,
}

/// The agent's loudness, as the resident hears it.
#[derive(Default)]
struct Ear {
    last_loud: Option<Instant>,
    /// When the agent's current or latest run of speech began.
    onset: Option<Instant>,
}

impl Ear {
    fn hear(&mut self, pcm: &[u8], now: Instant) {
        if samples_from_le_bytes(pcm).iter().any(|s| s.unsigned_abs() >= LOUD) {
            if self.last_loud.is_none_or(|t| now - t >= QUIET_GAP) {
                self.onset = Some(now);
            }
            self.last_loud = Some(now);
        }
    }

    fn quiet_for(&self, now: Instant) -> Option<Duration> {
        self.last_loud.map(|t| now - t)
    }

    /// When the agent started talking, if that was at or after `since`.
    fn onset_since(&self, since: Instant) -> Option<Instant> {
        self.onset.filter(|&o| o >= since)
    }
}

enum Phase {
    /// Waiting for the agent to speak after `since`, then fall quiet.
    AwaitAgent { since: Instant },
    /// Waiting for the agent to start its next reply, to talk over it `after` seconds in.
    AwaitOnset { since: Instant, after: f64, line: Rendered },
    Speaking { line: Rendered, pos: usize },
    /// Nothing left to say.
    Idle,
}

struct Resident {
    acts: VecDeque<Act>,
    fallback: VecDeque<Rendered>,
    phase: Phase,
    ear: Ear,
    lines: Vec<LineResult>,
    /// Maps this process's clock to Unix time.
    t0: Instant,
    t0_ms: f64,
}

impl Resident {
    fn unix_ms(&self, at: Instant) -> f64 {
        self.t0_ms + (at - self.t0).as_secs_f64() * 1000.0
    }

    fn start_line(&mut self, line: Rendered, interrupt: bool, now: Instant) {
        self.lines.push(LineResult {
            text: line.text.clone(),
            interrupt,
            started_ms: self.unix_ms(now),
            ended_ms: None,
            pauses: Vec::new(),
            reply_ms: None,
            agent_stopped_ms: None,
        });
        self.phase = Phase::Speaking { line, pos: 0 };
    }

    /// The next thing to do once the agent has finished speaking.
    fn next_act(&mut self, now: Instant) {
        match self.acts.pop_front() {
            Some(Act::Say(line)) => self.start_line(line, false, now),
            // Only reached if a script starts with one; there is no reply yet to talk over.
            Some(Act::Interrupt(_, line)) => self.start_line(line, true, now),
            Some(Act::Silent) => self.phase = Phase::Idle,
            None => match self.fallback.pop_front() {
                Some(line) => self.start_line(line, false, now),
                None => self.phase = Phase::Idle,
            },
        }
    }

    /// The resident's next 20 ms for the line.
    fn next_frame(&mut self, now: Instant) -> Vec<i16> {
        self.note_agent(now);
        match &mut self.phase {
            Phase::Idle => {}
            Phase::AwaitAgent { since } => {
                let spoke = self.ear.onset_since(*since).is_some();
                if spoke && self.ear.quiet_for(now).is_some_and(|q| q >= RESPOND_AFTER) {
                    self.next_act(now);
                }
            }
            Phase::AwaitOnset { since, after, .. } => {
                if let Some(onset) = self.ear.onset_since(*since)
                    && (now - onset).as_secs_f64() >= *after
                {
                    let Phase::AwaitOnset { line, .. } = std::mem::replace(&mut self.phase, Phase::Idle)
                    else {
                        unreachable!()
                    };
                    self.start_line(line, true, now);
                }
            }
            Phase::Speaking { .. } => {}
        }

        let now_ms = self.unix_ms(now);
        let Phase::Speaking { line, pos } = &mut self.phase else {
            return vec![0; LINE_FRAME];
        };
        let from = *pos;
        let to = (from + LINE_FRAME).min(line.samples.len());
        let mut frame = line.samples[from..to].to_vec();
        frame.resize(LINE_FRAME, 0);
        *pos = to;
        // A sample inside this frame, timed as it reaches the agent.
        let at = |sample: usize| now_ms + (sample - from) as f64 * 1000.0 / LINE_RATE_HZ as f64;
        let result = self.lines.last_mut().expect("a line is being said");
        for pause in &line.pauses {
            if (from..to).contains(&pause.start) {
                result.pauses.push(PauseResult {
                    seconds: pause.seconds,
                    before: pause.before,
                    start_ms: at(pause.start),
                });
            }
        }
        if to == line.samples.len() {
            result.ended_ms = Some(at(to));
            self.phase = match self.acts.pop_front() {
                Some(Act::Interrupt(after, line)) => Phase::AwaitOnset { since: now, after, line },
                other => {
                    // Not an interruption: put it back for when the agent has answered.
                    if let Some(act) = other {
                        self.acts.push_front(act);
                    }
                    Phase::AwaitAgent { since: now }
                }
            };
        }
        frame
    }

    /// Times the agent against the resident's latest line: when its reply began, and for a
    /// line said over it, when it stopped.
    fn note_agent(&mut self, now: Instant) {
        let (onset, last_loud) = (self.ear.onset, self.ear.last_loud);
        let quiet = self.ear.quiet_for(now);
        let t0 = (self.t0, self.t0_ms);
        let unix = |at: Instant| t0.1 + (at - t0.0).as_secs_f64() * 1000.0;
        let Some(line) = self.lines.last_mut() else { return };
        if let (Some(ended), None, Some(onset)) = (line.ended_ms, line.reply_ms, onset)
            && !line.interrupt
            && unix(onset) >= ended
        {
            line.reply_ms = Some(unix(onset) - ended);
        }
        if line.interrupt
            && line.agent_stopped_ms.is_none()
            && let (Some(last), Some(quiet)) = (last_loud, quiet)
            && quiet >= QUIET_GAP
            && unix(last) >= line.started_ms
        {
            line.agent_stopped_ms = Some(unix(last) - line.started_ms);
        }
    }
}

/// Places one call to the agent at `addr` and plays the resident's part until the call ends.
/// `transfers` receives what the agent's Call control recorded.
pub async fn run(
    addr: SocketAddr,
    uuid: Uuid,
    acts: Vec<Act>,
    fallback: Vec<Rendered>,
    mut transfers: mpsc::UnboundedReceiver<String>,
) -> Result<Outcome, Error> {
    let stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    let (mut from_agent, mut to_agent) = stream.into_split();
    to_agent.write_all(&encode_uuid(&uuid)).await?;
    let t0 = Instant::now();
    let t0_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64() * 1000.0;
    let mut resident = Resident {
        acts: acts.into(),
        fallback: fallback.into(),
        phase: Phase::AwaitAgent { since: t0 },
        ear: Ear::default(),
        lines: Vec::new(),
        t0,
        t0_ms,
    };
    let mut decoder = Decoder::from_agent();
    let mut buf = [0u8; 4096];
    let mut clock = tokio::time::interval(TICK);
    clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut transfer = None;

    let ended = 'call: loop {
        tokio::select! {
            biased;
            Some(extension) = transfers.recv() => {
                transfer = Some(extension);
                break Ended::Transferred;
            }
            read = from_agent.read(&mut buf) => {
                let n = read?;
                if n == 0 {
                    break Ended::AgentClosed;
                }
                decoder.push(&buf[..n]);
                while let Some(message) = decoder.next_message()? {
                    match message {
                        Message::Audio { pcm, .. } => resident.ear.hear(&pcm, Instant::now()),
                        Message::Hangup => break 'call Ended::AgentHungUp,
                        _ => {}
                    }
                }
            }
            _ = clock.tick() => {
                if t0.elapsed() >= MAX_CALL {
                    break Ended::TimedOut;
                }
                let frame = resident.next_frame(Instant::now());
                let message = encode_audio(LINE_RATE_HZ, &samples_to_le_bytes(&frame))?;
                if to_agent.write_all(&message).await.is_err() {
                    break Ended::AgentClosed;
                }
            }
        }
    };
    // Dropping both halves closes the socket, which is how Asterisk ends a call.
    drop((from_agent, to_agent));
    Ok(Outcome { lines: resident.lines, ended, transfer, duration: t0.elapsed() })
}
