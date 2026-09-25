//! The check-in conversation: the fixed greeting, then turn after turn of the resident
//! speaking, the LLM answering with a [`Turn`](crate::llm::Turn), and Piper speaking the reply,
//! until the agent says goodbye and hangs up (issue #18), or a trigger escalates the call to
//! the dispatcher (issue #20). Every call ends with a JSON log and, when one is due, a concern
//! flag (issue #21).
//!
//! The five escalation triggers (issue #11), none able to suppress another:
//! - the keyword rule, on every transcript as it arrives, never waiting on the LLM;
//! - the LLM's `emergency` status, before that turn's reply is spoken;
//! - silence: the third prompt in a row left unanswered for [`SILENCE_WAIT`];
//! - the resident asking for a person (the keyword rule's own category);
//! - an agent fault: the LLM or TTS failing twice, speech to text failing twice, or a turn
//!   still unanswered [`TURN_DEADLINE`] after the resident stopped.
//!
//! On any of them: stop playback, drop the turn in flight, speak the fixed escalation script,
//! then transfer the call through Call control.
//!
//! What the check-in covers is a [`Checklist`] the code keeps (issue #35): each turn the model
//! is told where it stands and what to ask next, and marks what the resident's words answered.
//! A medical question gets the offer of a person; a yes to it escalates as asking for a person.
//!
//! To keep the wait short, the reply is spoken as soon as the model has written it, while the
//! model is still writing the turn's summary ([`Head`]). If it still hasn't been queued
//! [`ACK_AFTER`] after the resident stopped, a fixed acknowledgement ("Okay.") fills the wait.
//!
//! Turn-taking (issue #19), the first iteration settled in issue #12:
//! - **End of turn.** The resident's turn ends after [`TURN_END`] of silence. Smart Turn, asked
//!   at 0.2 s of each pause, can only hold the floor: below p = 0.05 the agent waits up to
//!   3 s instead ([`end_of_turn`](crate::end_of_turn)).
//! - **Taking the turn back.** If the resident starts again while the agent is still thinking,
//!   before any reply has played, the agent drops the answer in progress and listens on: the
//!   decision was premature, and nothing the resident could hear has happened yet.
//! - **Barge-in, BI-2** ([`turn::barge_in`]). While the agent talks, 2 VAD windows of speech
//!   pause its audio. 0.8 s of speech, 3 words or a keyword gives the resident the turn and
//!   drops the rest of the agent's speech; otherwise it plays on after 1 s of silence, and
//!   what was said over it is dropped. Only the resident's speech during a pause is
//!   transcribed, so the agent's own voice is never heard as the resident's unless it pauses
//!   the agent first, and the escalation script, which says "call nine one one", is never
//!   paused or heard. The keyword rule checks every transcript, so a "help" said over the
//!   agent always escalates.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep_until;
use turn::barge_in::{self, BargeIn, Confirmed};
use turn::vad::{Silero, WINDOW};

use crate::audio::CORE_RATE_HZ;
use crate::call_log::{self, BargeInLog, CallLog, EchoMeter, EndedBy, Line, TurnLog, Who};
use crate::end_of_turn::{self, ASK_AT, EndOfTurn, HOLD_BELOW, HOLD_TO, PRE_SPEECH};
use crate::checklist::{self, Asking, Checklist};
use crate::escalation::{self, Escalation, Trigger};
use crate::listen::Segmenter;
use crate::llm::{Head, Llm, Message, Reply, Role, SYSTEM_PROMPT, Status};
use crate::stt::{self, Stt, Transcript};
use crate::telephony::{CallControl, MarkId, Media, Speaker};
use crate::tts::{Speech, Tone, Tts};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Silence that ends the resident's turn: the base wait in issue #12's first iteration.
pub const TURN_END: f64 = 1.5;

/// Times one turn may be taken back before the agent answers regardless. Bounds a loop where
/// something other than the resident keeps restarting the turn, such as the echo of the
/// holding line, which would otherwise keep an agent fault from ever escalating (inferred
/// risk; not seen).
const MAX_TAKE_BACKS: usize = 2;

/// Resident audio kept for Smart Turn, which looks at 8 s at most.
const RECENT_SAMPLES: usize = (end_of_turn::WINDOW_S * CORE_RATE_HZ as f64) as usize;

/// How long the agent waits for an answer once it has finished speaking (issue #11).
const SILENCE_WAIT: Duration = Duration::from_secs(10);

/// The unanswered prompt, counting the agent's own question, that escalates (issue #11).
const UNANSWERED_ESCALATES: u32 = 3;

/// A turn still unanswered this long after the resident stopped is an agent fault (issue #11).
const TURN_DEADLINE: Duration = Duration::from_secs(10);

/// A reply not yet queued this long after the resident stopped, half the turn's deadline, gets
/// an acknowledgement first (the maintainer's choice, 2026-09-25). Played on every turn, it came
/// just before replies that were about to start anyway, and sounded out of place. On live calls
/// replies started 2.6-3.9 s after the resident stopped (issue #18), so on turns like those it
/// doesn't play.
const ACK_AFTER: Duration = Duration::from_secs(TURN_DEADLINE.as_secs() / 2);

/// Silence after the goodbye or the escalation script, before hanging up or transferring, so
/// the phone has played the last word by the time the call leaves the agent. The mark only
/// says the audio reached Asterisk; how much the phone still holds in its jitter buffer is
/// unknown (inferred size).
const TAIL: f64 = 0.5;

/// Fixed lines, spoken the same way on every call and never written by the LLM (issue #10).
pub const GREETING_INBOUND: &str =
    "Hello. This is the automated check-in assistant. How are you feeling today?";
pub const HOLDING: &str = "Sorry, give me a moment.";
pub const ESCALATION: &str = "I'm connecting you to a person now. Please stay on the line. \
                              If you are in danger, call nine one one yourself as soon as you can.";
/// Played when a reply is slow ([`ACK_AFTER`]), taking turns, so the model's thinking time
/// isn't dead air (the maintainer's choice, 2026-09-24). Both fit good news and bad.
///
/// They must be words the voice says clearly on every take, since each is synthesised once
/// and then played all session (issue #45). espeak spells "Mm-hm." out as letters,
/// `ˌɛmˈɛmˌeɪtʃˈɛm`, which a live call heard as "and Mummy Chem". Of 40 takes each, spoken as
/// below and heard at 8 kHz by Whisper small.en, "Okay." and "Right." came back as the word 40
/// times, "I see." 35 and "Mm-hm." 5 (`PROTOTYPE_acks`, 2026-09-25).
pub const ACKS: [&str; 2] = ["Okay.", "Right."];
/// The first and second re-prompt after an unanswered prompt.
pub const STILL_THERE: [&str; 2] = [
    "Are you still there? Please say something if you can hear me.",
    "I can't hear you. If you don't answer, I'll get a person on the line.",
];

/// The fixed lines, synthesised once at startup, so a slow or failed TTS call can never delay
/// or reword them.
pub struct Lines {
    pub greeting: Vec<i16>,
    pub holding: Vec<i16>,
    pub escalation: Vec<i16>,
    pub still_there: [Vec<i16>; 2],
    pub acks: [Vec<i16>; 2],
}

impl Lines {
    pub async fn synthesise(tts: &Tts) -> Result<Self, Error> {
        Ok(Lines {
            greeting: tts.speak(GREETING_INBOUND, Tone::Warm).await?.audio,
            holding: tts.speak(HOLDING, Tone::Warm).await?.audio,
            escalation: tts.speak(ESCALATION, Tone::Steady).await?.audio,
            still_there: [
                tts.speak(STILL_THERE[0], Tone::Steady).await?.audio,
                tts.speak(STILL_THERE[1], Tone::Steady).await?.audio,
            ],
            acks: [
                tts.speak(ACKS[0], Tone::Warm).await?.audio,
                tts.speak(ACKS[1], Tone::Warm).await?.audio,
            ],
        })
    }
}

/// What every call shares.
pub struct Services<L> {
    pub stt: Stt,
    pub tts: Tts,
    pub llm: Arc<L>,
    pub lines: Arc<Lines>,
    /// Where escalations go: an internal extension, checked at startup.
    pub dispatcher: String,
    /// Where call logs go.
    pub calls_dir: PathBuf,
    /// Whether the resident can talk over the agent. Off, the agent is half duplex: it doesn't
    /// listen while it talks.
    pub barge_in: bool,
    /// Smart Turn, to hold the floor at a pause. `None`: every turn ends after [`TURN_END`].
    pub end_of_turn: Option<EndOfTurn>,
}

impl<L> Clone for Services<L> {
    fn clone(&self) -> Self {
        Services {
            stt: self.stt.clone(),
            tts: self.tts.clone(),
            llm: self.llm.clone(),
            lines: self.lines.clone(),
            dispatcher: self.dispatcher.clone(),
            calls_dir: self.calls_dir.clone(),
            barge_in: self.barge_in,
            end_of_turn: self.end_of_turn.clone(),
        }
    }
}

/// A reply that promises something for later. Nothing may be promised: stage-1 notices to
/// the dispatcher are mocks, so no one will come or call (issue #10). The prompt forbids it,
/// and the model still wrote "I'll make sure someone comes with a sandwich" (text run of
/// 2026-09-24).
pub fn promises(reply: &str) -> bool {
    const PHRASES: &[&str] = &[
        "someone will", "somebody will", "someone comes", "someone to come", "ill send",
        "ill get someone", "ill make sure", "will come", "will call you", "ill call you",
        "call you back", "check in again", "check on you", "keep an eye", "see you soon",
        "see you later", "ill arrange", "ill have someone",
    ];
    let words = format!(" {} ", escalation::normalise(reply).join(" "));
    PHRASES.iter().any(|phrase| words.contains(&format!(" {phrase} ")))
}

/// Sent back with a reply that [`promises`] something, to have it written again.
const NO_PROMISES: &str = "[System: your reply promised something for later. Nothing can be \
    promised: no one will come, call, bring anything or check again. Write the whole turn again \
    without any promise; you may say you'll pass it on.]";

/// A reply the LLM wrote as a goodbye but without `end_call`, the miss seen on live calls
/// (issue #35): it says goodbye or bye and asks nothing. Such a reply ends the call anyway.
pub fn says_goodbye(reply: &str) -> bool {
    !reply.contains('?')
        && reply
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == "goodbye" || word == "bye")
}

/// One utterance's transcript, back from the worker.
struct Heard {
    result: Result<Transcript, stt::Error>,
    /// When the resident stopped saying it, by the wall clock.
    stopped: Instant,
    /// Said over the agent: the pause of its audio that the utterance was cut in.
    over: Option<u64>,
}

/// Smart Turn's verdict on one pause.
struct Verdict {
    /// Which pause ([`TurnSoFar::asked`]).
    pause: u64,
    result: Result<end_of_turn::Verdict, end_of_turn::Error>,
}

/// How one of the resident's turns was answered.
enum Answer {
    /// `speech` is `None` for an `emergency` turn, whose words are never spoken. `early`: the
    /// reply was already handed over as an [`Early`], so only the summary is new.
    Reply { reply: Reply, speech: Option<Speech>, early: bool },
    /// The LLM or TTS failed twice.
    Fault(Error),
}

/// A turn to act on before the model has finished it: everything but the summary.
struct Early {
    /// The turn it answers ([`Call::generation`]); a turn given back makes it stale.
    generation: u64,
    /// The turn with an empty summary; `took` is the time until the head was written.
    reply: Reply,
    speech: Option<Speech>,
}

type Thinking = Option<JoinHandle<(Vec<Message>, Answer)>>;

/// What the agent's current speech leads to once it has played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// Listen again, and start the silence timer.
    Listen,
    HangUp,
    Transfer,
}

/// The resident's turn in progress.
#[derive(Default, Clone)]
struct TurnSoFar {
    /// Utterances cut so far.
    utterances: usize,
    /// Of those, the ones whose transcript hasn't come back.
    outstanding: usize,
    words: Vec<String>,
    stt_ms: u64,
    /// When the resident last stopped, by the wall clock.
    stopped: Option<Instant>,
    /// When the resident's speech in this turn began, in media time.
    start: Option<f64>,
    /// The pause Smart Turn was asked about, if it has been asked about this one.
    asked: Option<u64>,
    /// Smart Turn holds the floor at this pause.
    hold: bool,
    smart_turn_p: Vec<f32>,
    taken_back_at_s: Vec<f64>,
}

impl TurnSoFar {
    /// The turn is over and every transcript is in.
    fn ready(&self, segmenter: &Segmenter) -> bool {
        let wait = if self.hold { HOLD_TO } else { TURN_END };
        self.utterances > 0 && self.outstanding == 0 && segmenter.quiet_for() >= wait
    }

    /// Adds what the resident said after this turn was taken, when it is given back.
    fn merge(&mut self, later: TurnSoFar) {
        self.utterances += later.utterances;
        self.outstanding += later.outstanding;
        self.words.extend(later.words);
        self.stt_ms += later.stt_ms;
        self.stopped = later.stopped.or(self.stopped);
        self.start = self.start.or(later.start);
        self.asked = later.asked;
        self.hold = later.hold;
        self.smart_turn_p.extend(later.smart_turn_p);
    }
}

/// One call's state apart from the channels it waits on.
struct Call<L> {
    id: String,
    started: Instant,
    speaker: Speaker,
    control: CallControl,
    services: Services<L>,
    marks: MarkId,
    /// The mark after the agent's current speech, and what follows it.
    speaking: Option<(MarkId, After)>,
    /// The mark before a reply's first audio, when the resident stopped, and the turn's index.
    first_audio: Option<(MarkId, Instant, usize)>,
    /// When the resident stopped before the turn being answered.
    stopped: Instant,
    /// When the turn being answered becomes an agent fault.
    turn_deadline: Option<Instant>,
    /// When the acknowledgement plays, unless the reply has been queued by then.
    ack_at: Option<Instant>,
    /// Acknowledgements played so far, so they take turns.
    acks: usize,
    /// When the current prompt counts as unanswered.
    silence: Option<Instant>,
    unanswered: u32,
    /// The escalation, and when the speech (or timeout) that triggered it ended.
    escalation: Option<(Escalation, Instant)>,
    transcript: Vec<Line>,
    turns: Vec<TurnLog>,
    unheard_turns_at_s: Vec<f64>,
    checklist: Checklist,
    status: Status,
    reasons: Vec<String>,
    summary: String,
    /// Counts the turns answered, so a reply to a turn given back is known to be stale.
    generation: u64,
    /// The turn being answered, kept so it can be given back if the resident goes on.
    answering: Option<TurnSoFar>,
    /// The acknowledgement has played for the turn being answered.
    acked: bool,
    /// Counts the pauses Smart Turn has been asked about.
    pauses: u64,
    barge: BargeIn,
    /// While the agent's audio is paused for the resident: since when, in seconds since the
    /// call started.
    paused: Option<f64>,
    /// Counts the pauses, and lists the ones the agent played on from: what was said in those
    /// was a backchannel, not the resident's turn.
    pause_id: u64,
    resumed: Vec<u64>,
    /// What the agent's current speech says, and the turn it answers if it is a reply.
    speaking_text: String,
    speaking_turn: Option<usize>,
    barge_ins: Vec<BargeInLog>,
    echo: EchoMeter,
}

impl<L: Llm + 'static> Call<L> {
    fn at(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Queues speech between two marks and notes it in the transcript. Returns the first mark.
    fn say(&mut self, audio: Vec<i16>, text: &str, after: After) -> MarkId {
        self.silence = None;
        self.marks += 1;
        let first = self.marks;
        self.speaker.mark(first);
        self.echo.agent_audio(&audio);
        self.speaker.play(audio);
        self.speaking_text = text.to_string();
        self.speaking_turn = None;
        if self.paused.is_some() {
            // Not reached today: nothing new is said while the agent is paused, apart from the
            // escalation script, which ends the pause first.
            self.speaker.resume();
            self.end_pause("replaced by new speech".to_string());
        }
        self.barge.reset();
        if after != After::Listen {
            self.speaker.play(vec![0; (TAIL * CORE_RATE_HZ as f64) as usize]);
        }
        self.marks += 1;
        self.speaker.mark(self.marks);
        self.speaking = Some((self.marks, after));
        self.transcript.push(Line { speaker: Who::Agent, text: text.to_string(), at_s: self.at() });
        first
    }

    /// The resident said something with words in it: that answers the prompt.
    fn heard(&mut self, text: &str) {
        eprintln!("agent: {}: heard {text:?}", self.id);
        self.transcript.push(Line { speaker: Who::Resident, text: text.to_string(), at_s: self.at() });
        self.unanswered = 0;
        self.silence = None;
    }

    /// Starts answering the resident's turn, unless it held no words.
    fn respond(
        &mut self,
        turn: &mut TurnSoFar,
        history: &[Message],
        early: &mpsc::UnboundedSender<Early>,
    ) -> Thinking {
        let turn = std::mem::take(turn);
        if turn.words.is_empty() {
            if turn.utterances > 0 {
                eprintln!("agent: {}: the turn held nothing intelligible; listening on", self.id);
                self.unheard_turns_at_s.push(self.at());
            }
            return None;
        }
        let heard = turn.words.join(" ");
        self.stopped = turn.stopped.unwrap_or_else(Instant::now);
        self.turns.push(TurnLog {
            turn: self.turns.len() + 1,
            heard: heard.clone(),
            taken_at_s: self.at(),
            taken_back_at_s: turn.taken_back_at_s.clone(),
            smart_turn_p: turn.smart_turn_p.clone(),
            held: turn.hold,
            end_of_turn_ms: self.stopped.elapsed().as_millis() as u64,
            stt_ms: turn.stt_ms,
            ..TurnLog::default()
        });
        self.turn_deadline = Some(self.stopped + TURN_DEADLINE);
        self.ack_at = Some(self.stopped + ACK_AFTER);
        self.silence = None;
        self.generation += 1;
        self.acked = false;
        self.answering = Some(turn);
        let note = self.checklist.note(checklist::wants_to_end(&heard));
        let (services, speaker, early) =
            (self.services.clone(), self.speaker.clone(), early.clone());
        let generation = self.generation;
        Some(tokio::spawn(respond(history.to_vec(), heard, note, services, speaker, early, generation)))
    }

    /// The agent's audio is going out: queued or playing, and not paused.
    fn talking(&self) -> bool {
        self.speaking.is_some() && self.paused.is_none()
    }

    /// The resident started talking over the agent: holds its audio where it is.
    fn pause(&mut self) {
        self.speaker.pause();
        self.paused = Some(self.at());
        self.pause_id += 1;
        eprintln!("agent: {}: the resident is talking over the agent; paused", self.id);
    }

    /// It was only a "yeah": plays on from where it paused.
    fn resume(&mut self) {
        self.speaker.resume();
        self.resumed.push(self.pause_id);
        self.end_pause("resumed".to_string());
    }

    /// Notes how a pause ended.
    fn end_pause(&mut self, outcome: String) {
        let Some(since) = self.paused.take() else { return };
        let paused_ms = ((self.at() - since) * 1000.0) as u64;
        eprintln!("agent: {}: barge-in {outcome}, after {paused_ms} ms paused", self.id);
        self.barge_ins.push(BargeInLog {
            paused_at_s: since,
            over: self.speaking_text.clone(),
            outcome,
            paused_ms,
        });
    }

    /// The resident has the turn: drops the rest of the agent's speech, and listens.
    async fn yield_floor(&mut self, why: Confirmed) {
        // A late transcript can confirm after the agent had played on.
        self.paused.get_or_insert_with(|| self.started.elapsed().as_secs_f64());
        self.speaker.clear().await;
        self.barge.reset();
        self.end_pause(match why {
            Confirmed::Speech(s) => format!("confirmed: {s:.1} s of speech"),
            Confirmed::Words(n) => format!("confirmed: {n} words"),
            Confirmed::Keyword(k) => format!("confirmed: the word {k:?}"),
        });
        if let Some(index) = self.speaking_turn.take() {
            self.turns[index].interrupted = true;
        }
        self.speaking = None;
        self.first_audio = None;
        self.silence = None;
    }

    /// Whether the turn being answered can still be given back: nothing has been said since,
    /// not even the acknowledgement.
    fn can_take_back(&self, thinking: &Thinking, turn: &TurnSoFar) -> bool {
        thinking.is_some()
            && self.speaking.is_none()
            && !self.acked
            && self.escalation.is_none()
            && self.answering.as_ref().is_some_and(|a| a.taken_back_at_s.len() < MAX_TAKE_BACKS)
            && turn.taken_back_at_s.len() < MAX_TAKE_BACKS
    }

    /// The resident went on after the agent had taken the turn, before any reply played:
    /// drops the answer in progress and gives the turn back.
    fn take_back(&mut self, thinking: &mut Thinking, turn: &mut TurnSoFar) {
        let Some(task) = thinking.take() else { return };
        task.abort();
        // Any reply already sent from it is stale.
        self.generation += 1;
        self.turn_deadline = None;
        self.ack_at = None;
        let taken_at = self.turns.pop().map_or_else(|| self.at(), |log| log.taken_at_s);
        let mut given_back = self.answering.take().unwrap_or_default();
        given_back.taken_back_at_s.push(taken_at);
        given_back.merge(std::mem::take(turn));
        *turn = given_back;
        eprintln!("agent: {}: the resident went on; gave the turn back", self.id);
    }

    /// Asks Smart Turn about the pause the resident is in, once per pause.
    fn consult(
        &mut self,
        turn: &mut TurnSoFar,
        segmenter: &Segmenter,
        recent: &VecDeque<f32>,
        verdicts: &mpsc::UnboundedSender<Verdict>,
    ) {
        let Some(start) = turn.start else { return };
        if segmenter.quiet_for() == 0.0 {
            // Talking again: the next pause gets its own verdict.
            turn.asked = None;
            turn.hold = false;
            return;
        }
        let Some(model) = &self.services.end_of_turn else { return };
        if turn.asked.is_some() || segmenter.quiet_for() < ASK_AT {
            return;
        }
        self.pauses += 1;
        turn.asked = Some(self.pauses);
        // The whole turn and a little before it, at most the model's 8 s.
        let from = (start - PRE_SPEECH).max(segmenter.now() - end_of_turn::WINDOW_S);
        let n = (((segmenter.now() - from) * CORE_RATE_HZ as f64) as usize).min(recent.len());
        let audio: Vec<f32> = recent.iter().skip(recent.len() - n).copied().collect();
        let (model, verdicts, pause) = (model.clone(), verdicts.clone(), self.pauses);
        tokio::spawn(async move {
            let result = model.ask(audio).await;
            let _ = verdicts.send(Verdict { pause, result });
        });
    }

    /// The reply is slow: fills the wait with an acknowledgement. The reply, once queued, plays
    /// after it.
    fn acknowledge(&mut self) {
        let ack = self.acks % ACKS.len();
        self.acks += 1;
        self.acked = true;
        self.speaker.play(self.services.lines.acks[ack].clone());
        self.transcript.push(Line { speaker: Who::Agent, text: ACKS[ack].into(), at_s: self.at() });
        if let Some(log) = self.turns.last_mut() {
            log.acknowledged = true;
        }
        eprintln!(
            "agent: {}: no reply {} ms after the resident stopped, said {:?}",
            self.id,
            self.stopped.elapsed().as_millis(),
            ACKS[ack]
        );
    }

    /// The summary of a turn already acted on as an [`Early`].
    fn finish(&mut self, reply: Reply) {
        self.summary = reply.turn.summary;
        eprintln!("agent: {}: summary {:?}", self.id, self.summary);
        if let Some(log) = self.turns.last_mut() {
            log.llm_total_ms = Some(reply.took.as_millis() as u64);
        }
    }

    /// Speaks the LLM's reply, or escalates if it says `emergency`.
    async fn answer(&mut self, mut reply: Reply, speech: Option<Speech>, thinking: &mut Thinking) {
        // The reply is here: no acknowledgement needed.
        self.ack_at = None;
        let turn = &mut reply.turn;
        // Two backstops for a goodbye without `end_call` (issue #35).
        let backstop =
            !turn.end_call && (turn.asking == Asking::Goodbye || says_goodbye(&turn.reply));
        turn.end_call |= backstop;
        let heard = self.turns.last().map_or("", |t| t.heard.as_str());
        let recorded = self.checklist.record(&turn.checklist, turn.asking, heard);
        eprintln!(
            "agent: {}: {} {:?} [asking {:?}, {:?}{}{}] (LLM {} ms, TTS {} ms)\n\
             agent: {}: marked {:?}{}",
            self.id,
            // An `emergency` turn's words are never spoken.
            if speech.is_some() { "said" } else { "the LLM wrote (not spoken)" },
            turn.reply,
            turn.asking,
            turn.status,
            turn.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default(),
            match (turn.end_call, backstop) {
                (true, true) => ", end_call (added: the reply is a goodbye)",
                (true, false) => ", end_call",
                _ => "",
            },
            reply.took.as_millis(),
            speech.as_ref().map_or(0, |s| s.took.as_millis()),
            self.id,
            turn.checklist,
            match (&recorded.closed[..], &recorded.ungrounded[..]) {
                ([], []) => String::new(),
                (closed, ungrounded) => format!(
                    "; closed without a clear answer: {closed:?}; dropped as not in their words: \
                     {ungrounded:?}"
                ),
            },
        );
        self.status = self.status.max(turn.status);
        if turn.status > Status::Ok
            && let Some(reason) = &turn.reason
            && !self.reasons.contains(reason)
        {
            self.reasons.push(reason.clone());
        }
        // Empty on an early turn; its summary comes with `finish`.
        if !turn.summary.is_empty() {
            self.summary = turn.summary.clone();
            eprintln!("agent: {}: summary {:?}", self.id, self.summary);
        }
        if let Some(log) = self.turns.last_mut() {
            log.reply = Some(turn.reply.clone());
            log.status = Some(turn.status);
            log.reason = turn.reason.clone();
            log.asking = Some(turn.asking);
            log.end_call = turn.end_call;
            log.llm_ms = Some(reply.took.as_millis() as u64);
            log.tts_ms = speech.as_ref().map(|s| s.took.as_millis() as u64);
        }

        let Some(speech) = speech else {
            let reason = turn.reason.clone().unwrap_or_else(|| "status emergency".into());
            let (turn, stopped) = (self.turns.len(), self.stopped);
            self.escalate(Trigger::Llm, reason, turn, stopped, thinking).await;
            return;
        };
        let after = if turn.end_call { After::HangUp } else { After::Listen };
        let first = self.say(speech.audio, &reply.turn.reply, after);
        self.speaking_turn = Some(self.turns.len() - 1);
        self.first_audio = Some((first, self.stopped, self.turns.len() - 1));
    }

    /// Escalates, once per call: stops playback, drops the turn in flight, and speaks the
    /// fixed script. The transfer follows once the script has played.
    async fn escalate(
        &mut self,
        trigger: Trigger,
        evidence: String,
        turn: usize,
        since: Instant,
        thinking: &mut Thinking,
    ) {
        if self.escalation.is_some() {
            return;
        }
        if let Some(task) = thinking.take() {
            task.abort();
        }
        self.turn_deadline = None;
        self.ack_at = None;
        self.first_audio = None;
        // The script always plays through: no barge-in from here on.
        self.speaker.clear().await;
        self.end_pause("ended by the escalation".to_string());
        self.barge.reset();
        eprintln!("agent: {}: ESCALATING on turn {turn}, trigger {trigger:?}: {evidence}", self.id);
        let detected_ms = since.elapsed().as_millis() as u64;
        let escalation =
            Escalation { trigger, evidence, turn, detected_ms, latency_ms: None, transfer: None };
        self.escalation = Some((escalation, since));
        // A keyword phrase is an emergency by definition (issue #11); the call's status says so
        // even when no LLM turn ran.
        if trigger == Trigger::Keyword {
            self.status = Status::Emergency;
        }
        let script = self.services.lines.escalation.clone();
        self.say(script, ESCALATION, After::Transfer);
        eprintln!("agent: {}: said {ESCALATION:?}", self.id);
    }
}

/// Runs one check-in until either side hangs up or the call is transferred. `call` names the
/// call in the log.
pub async fn check_in<L: Llm + 'static>(
    call: String,
    media: Media,
    control: CallControl,
    mut vad: Silero,
    services: Services<L>,
) {
    let Media { mut frames, mut played, speaker } = media;
    let started_unix_ms =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    // Who is calling, for the log and the concern flag. Looked up while the greeting plays.
    let lookup = {
        let control = control.clone();
        tokio::spawn(async move { control.channel().await })
    };
    let mut c = Call {
        id: call,
        started: Instant::now(),
        speaker,
        control,
        services,
        marks: 0,
        speaking: None,
        first_audio: None,
        stopped: Instant::now(),
        turn_deadline: None,
        ack_at: None,
        acks: 0,
        silence: None,
        unanswered: 0,
        escalation: None,
        transcript: Vec::new(),
        turns: Vec::new(),
        unheard_turns_at_s: Vec::new(),
        checklist: Checklist::default(),
        status: Status::Ok,
        reasons: Vec::new(),
        summary: String::new(),
        generation: 0,
        answering: None,
        acked: false,
        pauses: 0,
        barge: BargeIn::default(),
        paused: None,
        pause_id: 0,
        resumed: Vec::new(),
        speaking_text: String::new(),
        speaking_turn: None,
        barge_ins: Vec::new(),
        echo: EchoMeter::default(),
    };
    let mut history = vec![Message::new(Role::System, SYSTEM_PROMPT)];
    let mut segmenter = Segmenter::default();
    let mut pending: Vec<i16> = Vec::with_capacity(WINDOW * 2);
    let (heard_tx, mut heard_rx) = mpsc::unbounded_channel::<Heard>();
    let (early_tx, mut early_rx) = mpsc::unbounded_channel::<Early>();
    let mut turn = TurnSoFar::default();
    let mut thinking: Thinking = None;
    let mut transfer: Option<JoinHandle<Result<String, Error>>> = None;
    let (verdict_tx, mut verdict_rx) = mpsc::unbounded_channel::<Verdict>();
    // The resident's latest audio, ending at the segmenter's clock, for Smart Turn.
    let mut recent: VecDeque<f32> = VecDeque::with_capacity(RECENT_SAMPLES + WINDOW);

    let greeting = c.services.lines.greeting.clone();
    c.say(greeting, GREETING_INBOUND, After::Listen);
    eprintln!("agent: {}: said {GREETING_INBOUND:?}", c.id);

    let ended_by = 'call: loop {
        // Copied out, so the timers below don't borrow `c`.
        let (silence, deadline, ack_at) = (c.silence, c.turn_deadline, c.ack_at);
        tokio::select! {
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    if c.escalation.is_some() {
                        eprintln!("agent: {}: the call has left the agent", c.id);
                        break EndedBy::Escalated;
                    }
                    eprintln!("agent: {}: the resident hung up", c.id);
                    break EndedBy::LineClosed;
                };
                pending.extend_from_slice(&frame);
                while pending.len() >= WINDOW {
                    let window: Vec<i16> = pending.drain(..WINDOW).collect();
                    let samples: Vec<f32> = window.iter().map(|&s| s as f32 / 32_768.0).collect();
                    // Runs on every window, heard or not, so Silero's state stays continuous.
                    let p = match vad.prob(&samples) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("agent: {}: VAD failed, hanging up: {e}", c.id);
                            c.control.hangup();
                            break 'call EndedBy::AgentError;
                        }
                    };
                    c.echo.window(&samples, p, c.talking());
                    recent.extend(samples.iter().copied());
                    let excess = recent.len().saturating_sub(RECENT_SAMPLES);
                    recent.drain(..excess);
                    // The escalation script always plays to the end, unheard.
                    if c.escalation.is_some() {
                        segmenter.skip(&window);
                        continue;
                    }
                    if c.speaking.is_some() {
                        if !c.services.barge_in {
                            segmenter.skip(&window);
                            continue;
                        }
                        match c.barge.push(segmenter.now(), p) {
                            Some(barge_in::Action::Pause) => c.pause(),
                            Some(barge_in::Action::Confirm(why)) => c.yield_floor(why).await,
                            Some(barge_in::Action::Resume) => {
                                c.resume();
                                // What was said over the agent isn't the resident's turn.
                                segmenter.restart();
                                turn = TurnSoFar { outstanding: turn.outstanding, ..TurnSoFar::default() };
                            }
                            None => {}
                        }
                        if c.talking() {
                            segmenter.skip(&window);
                            continue;
                        }
                    }
                    // Listening, or paused for the resident: cut their speech into utterances.
                    let was_open = segmenter.in_utterance();
                    let utterance = segmenter.push(&window, p);
                    if !was_open && segmenter.in_utterance() {
                        turn.start = turn.start.or(segmenter.utterance_start());
                        if c.can_take_back(&thinking, &turn) {
                            c.take_back(&mut thinking, &mut turn);
                        }
                    }
                    if let Some(utterance) = utterance {
                        // Frames arrive in real time, so the resident stopped this long before
                        // now (inferred, as in listen.rs).
                        let stopped = Instant::now()
                            - Duration::from_secs_f64(segmenter.now() - utterance.end);
                        turn.utterances += 1;
                        turn.outstanding += 1;
                        let over = c.paused.map(|_| c.pause_id);
                        let (stt, heard) = (c.services.stt.clone(), heard_tx.clone());
                        tokio::spawn(async move {
                            // An agent fault only on the second failure (issue #11).
                            let result = match stt.transcribe(utterance.audio.clone()).await {
                                Err(e) => {
                                    eprintln!("agent: transcription failed, retrying once: {e}");
                                    stt.transcribe(utterance.audio).await
                                }
                                ok => ok,
                            };
                            let _ = heard.send(Heard { result, stopped, over });
                        });
                    }
                    c.consult(&mut turn, &segmenter, &recent, &verdict_tx);
                    if turn.ready(&segmenter) && thinking.is_none() && c.speaking.is_none() {
                        thinking = c.respond(&mut turn, &history, &early_tx);
                    }
                }
            }
            Some(heard) = heard_rx.recv() => {
                turn.outstanding = turn.outstanding.saturating_sub(1);
                if c.escalation.is_some() {
                    continue;
                }
                match heard.result {
                    Ok(transcript) => {
                        turn.stt_ms += transcript.took.as_millis() as u64;
                        if is_noise(&transcript.text) {
                            eprintln!("agent: {}: heard only noise: {:?}", c.id, transcript.text);
                        } else {
                            c.heard(&transcript.text);
                            // Said over the agent: enough words, or a keyword, take the turn,
                            // even when they arrive after it played on.
                            let confirms = heard.over.and(barge_in::confirms(&transcript.text));
                            if let Some(why) = confirms.clone()
                                && c.speaking.is_some()
                            {
                                c.yield_floor(why).await;
                            }
                            // Said in a pause the agent played on from: a backchannel, not the
                            // resident's turn. The keyword rule still hears it.
                            let own_turn = match heard.over {
                                Some(pause) => confirms.is_some() || !c.resumed.contains(&pause),
                                None => true,
                            };
                            if own_turn {
                                turn.words.push(transcript.text.clone());
                                turn.stopped = Some(heard.stopped);
                                // A turn given words counts as spoken, even if the utterance
                                // itself was cut before a resume reset the turn.
                                turn.utterances = turn.utterances.max(1);
                            }
                            let so_far =
                                if own_turn { turn.words.join(" ") } else { transcript.text.clone() };
                            if let Some(hit) = escalation::check(&transcript.text, &so_far) {
                                let evidence = format!("{:?} in {so_far:?}", hit.phrase);
                                let number = c.turns.len() + 1;
                                c.escalate(hit.trigger, evidence, number, heard.stopped, &mut thinking)
                                    .await;
                                continue;
                            }
                            // A yes to the offer of a person, checked here rather than left to
                            // the model, so it never waits on an LLM call.
                            if own_turn
                                && c.checklist.last() == Some(Asking::OfferPerson)
                                && checklist::accepts_offer(&transcript.text)
                            {
                                let evidence =
                                    format!("took the offer of a person: {:?}", transcript.text);
                                let number = c.turns.len() + 1;
                                let trigger = Trigger::AskedForPerson;
                                c.escalate(trigger, evidence, number, heard.stopped, &mut thinking)
                                    .await;
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        let number = c.turns.len() + 1;
                        let evidence = format!("speech to text failed twice: {e}");
                        c.escalate(Trigger::AgentFault, evidence, number, heard.stopped, &mut thinking)
                            .await;
                        continue;
                    }
                }
                if turn.ready(&segmenter) && thinking.is_none() && c.speaking.is_none() {
                    thinking = c.respond(&mut turn, &history, &early_tx);
                }
            }
            Some(verdict) = verdict_rx.recv() => {
                // A verdict on a pause that has since ended is stale.
                if turn.asked != Some(verdict.pause) {
                    continue;
                }
                match verdict.result {
                    Ok(v) => {
                        turn.smart_turn_p.push(v.p);
                        turn.hold = v.p < HOLD_BELOW;
                        if turn.hold {
                            eprintln!(
                                "agent: {}: Smart Turn p = {:.3} ({} ms): holding the floor for up \
                                 to {HOLD_TO} s",
                                c.id,
                                v.p,
                                v.took.as_millis()
                            );
                        }
                    }
                    Err(e) => eprintln!("agent: {}: Smart Turn failed, not holding: {e}", c.id),
                }
            }
            Some(early) = early_rx.recv() => {
                // One from a turn given back, or dropped by an escalation, is stale.
                if thinking.is_some() && c.escalation.is_none() && early.generation == c.generation {
                    c.answer(early.reply, early.speech, &mut thinking).await;
                }
            }
            answer = async { thinking.as_mut().unwrap().await }, if thinking.is_some() => {
                thinking = None;
                c.turn_deadline = None;
                c.ack_at = None;
                c.answering = None;
                let answer = match answer {
                    Ok((h, answer)) => {
                        history = h;
                        answer
                    }
                    Err(e) => {
                        eprintln!("agent: {}: the turn task failed, hanging up: {e}", c.id);
                        c.control.hangup();
                        break EndedBy::AgentError;
                    }
                };
                match answer {
                    Answer::Fault(error) => {
                        let (number, stopped) = (c.turns.len(), c.stopped);
                        let evidence = format!("the turn failed twice: {error}");
                        c.escalate(Trigger::AgentFault, evidence, number, stopped, &mut thinking).await;
                    }
                    Answer::Reply { reply, early: true, .. } => {
                        // `select!` may pick the finished task before its early message.
                        while let Ok(early) = early_rx.try_recv() {
                            if early.generation == c.generation {
                                let mut none = None;
                                c.answer(early.reply, early.speech, &mut none).await;
                            }
                        }
                        c.finish(reply);
                    }
                    Answer::Reply { reply, speech, early: false } => {
                        c.answer(reply, speech, &mut thinking).await
                    }
                }
            }
            Some(mark) = played.recv() => {
                if let Some((first, since, index)) = c.first_audio
                    && mark == first
                {
                    c.first_audio = None;
                    let ms = since.elapsed().as_millis() as u64;
                    c.turns[index].latency_ms = Some(ms);
                    eprintln!("agent: {}: reply started {ms} ms after the resident stopped", c.id);
                }
                if let Some((end, after)) = c.speaking
                    && mark == end
                {
                    c.speaking = None;
                    c.speaking_turn = None;
                    c.barge.reset();
                    if c.paused.is_some() {
                        // It ran out while paused: the resident is talking, so listen on.
                        c.speaker.resume();
                        c.end_pause("the agent had finished anyway".to_string());
                    } else {
                        segmenter.restart();
                    }
                    match after {
                        After::Listen => c.silence = Some(Instant::now() + SILENCE_WAIT),
                        After::HangUp => {
                            eprintln!("agent: {}: goodbye played, hanging up", c.id);
                            c.control.hangup();
                            break EndedBy::Goodbye;
                        }
                        After::Transfer => {
                            let (control, dispatcher) =
                                (c.control.clone(), c.services.dispatcher.clone());
                            if let Some((escalation, since)) = &mut c.escalation {
                                escalation.latency_ms = Some(since.elapsed().as_millis() as u64);
                            }
                            eprintln!("agent: {}: transferring to {dispatcher}", c.id);
                            transfer = Some(tokio::spawn(async move {
                                control.transfer(&dispatcher).await
                            }));
                        }
                    }
                }
            }
            _ = sleep_until(silence.unwrap_or_else(Instant::now).into()), if silence.is_some() => {
                // Speech in progress, or a turn not yet answered, isn't silence; look again soon.
                if segmenter.in_utterance() || turn.utterances > 0 || thinking.is_some() {
                    c.silence = Some(Instant::now() + Duration::from_secs(1));
                    continue;
                }
                c.unanswered += 1;
                eprintln!("agent: {}: prompt unanswered ({} in a row)", c.id, c.unanswered);
                if c.unanswered >= UNANSWERED_ESCALATES {
                    let evidence = format!(
                        "{} prompts in a row unanswered for {} s each",
                        c.unanswered,
                        SILENCE_WAIT.as_secs()
                    );
                    let number = c.turns.len();
                    c.escalate(Trigger::Silence, evidence, number, Instant::now(), &mut thinking).await;
                } else {
                    let i = c.unanswered as usize - 1;
                    let line = c.services.lines.still_there[i].clone();
                    c.say(line, STILL_THERE[i], After::Listen);
                    eprintln!("agent: {}: said {:?}", c.id, STILL_THERE[i]);
                }
            }
            _ = sleep_until(deadline.unwrap_or_else(Instant::now).into()), if deadline.is_some() => {
                c.turn_deadline = None;
                if thinking.is_some() {
                    let (number, stopped) = (c.turns.len(), c.stopped);
                    let evidence = format!(
                        "no reply within {} s of the resident stopping",
                        TURN_DEADLINE.as_secs()
                    );
                    c.escalate(Trigger::AgentFault, evidence, number, stopped, &mut thinking).await;
                }
            }
            _ = sleep_until(ack_at.unwrap_or_else(Instant::now).into()), if ack_at.is_some() => {
                c.ack_at = None;
                if thinking.is_some() && c.escalation.is_none() {
                    c.acknowledge();
                }
            }
            result = async { transfer.as_mut().unwrap().await }, if transfer.is_some() => {
                transfer = None;
                record_transfer(&mut c, result);
            }
        }
    };

    // The socket can close before AMI answers: the channel leaves AudioSocket at the Redirect.
    if let Some(task) = transfer {
        record_transfer(&mut c, task.await);
    }
    let resident = match lookup.await {
        Ok(Ok(channel)) => channel.caller,
        Ok(Err(e)) => {
            eprintln!("agent: {}: couldn't look up the caller: {e}", c.id);
            None
        }
        Err(_) => None,
    };
    let concern_flag = call_log::concern_flag(c.status, &c.reasons, &c.summary, ended_by);
    if let Some(flag) = &concern_flag {
        call_log::print_notice(&c.id, resident.as_deref(), flag);
    }
    let log = CallLog {
        call: c.id.clone(),
        resident,
        started_unix_ms,
        duration_s: c.at(),
        transcript: c.transcript,
        turns: c.turns,
        unheard_turns_at_s: c.unheard_turns_at_s,
        checklist: c.checklist,
        final_status: c.status,
        summary: c.summary,
        escalation: c.escalation.map(|(e, _)| e),
        concern_flag,
        ended_by,
        barge_ins: c.barge_ins,
        echo: c.echo.log(),
    };
    let echo = &log.echo;
    eprintln!(
        "agent: {}: echo: the agent talked {:.1} s at {} dBFS; the line meanwhile {} dBFS \
         (p50), {} (p95), VAD over threshold on {} of {} windows; quiet line {} dBFS",
        log.call,
        echo.agent_talking_s,
        db(echo.agent_level_dbfs),
        db(echo.line_while_talking_p50_dbfs),
        db(echo.line_while_talking_p95_dbfs),
        echo.vad_fires_while_talking,
        echo.windows_while_talking,
        db(echo.line_quiet_p50_dbfs),
    );
    match call_log::write(&c.services.calls_dir, &log) {
        Ok(path) => eprintln!("agent: {}: call log written to {}", c.id, path.display()),
        Err(e) => eprintln!("agent: {}: couldn't write the call log: {e}", c.id),
    }
}

fn db(level: Option<f64>) -> String {
    level.map_or("n/a".to_string(), |l| format!("{l:.1}"))
}

fn record_transfer<L>(c: &mut Call<L>, result: Result<Result<String, Error>, tokio::task::JoinError>) {
    let outcome = match result {
        Ok(Ok(message)) => format!("sent: {message}"),
        Ok(Err(e)) => format!("failed: {e}"),
        Err(e) => format!("failed: {e}"),
    };
    eprintln!("agent: {}: transfer to {} {outcome}", c.id, c.services.dispatcher);
    if let Some((escalation, _)) = &mut c.escalation {
        escalation.transfer = Some(outcome);
    }
}

/// Answers one of the resident's turns: asks the LLM and synthesises the reply. On a failure
/// it says the holding line and tries once more.
///
/// The model sees the check-in list's `note` after the resident's words. The history keeps
/// only the words, so the model never reads an old note as the current state.
async fn respond<L: Llm>(
    mut history: Vec<Message>,
    heard: String,
    note: String,
    services: Services<L>,
    speaker: Speaker,
    early: mpsc::UnboundedSender<Early>,
    generation: u64,
) -> (Vec<Message>, Answer) {
    let mut messages = history.clone();
    messages.push(Message::new(Role::User, format!("{heard}\n\n{note}")));
    let mut attempt = 0;
    loop {
        attempt += 1;
        match answer(&messages, &services, &early, generation).await {
            Ok((reply, speech, was_early)) => {
                history.push(Message::new(Role::User, heard));
                history.push(Message::new(Role::Assistant, reply.raw.clone()));
                return (history, Answer::Reply { reply, speech, early: was_early });
            }
            Err(error) if attempt >= 2 => return (history, Answer::Fault(error)),
            Err(error) => {
                eprintln!("agent: the turn failed, retrying once: {error}");
                speaker.play(services.lines.holding.clone());
            }
        }
    }
}

/// The LLM's turn, and the reply spoken in the tone its status calls for. An `emergency` turn
/// isn't synthesised: it escalates, and its words are never said.
///
/// The reply is synthesised and sent to `early` as soon as the model has written it, ahead of
/// the summary; the result then says `true`. A reply that [`promises`] something waits for the
/// whole turn and is written again.
async fn answer<L: Llm>(
    history: &[Message],
    services: &Services<L>,
    early: &mpsc::UnboundedSender<Early>,
    generation: u64,
) -> Result<(Reply, Option<Speech>, bool), Error> {
    let (head_tx, head_rx) = oneshot::channel::<(Head, Duration)>();
    let whole = services.llm.turn_streaming(history, head_tx);
    tokio::pin!(whole);
    let head = tokio::select! {
        head = head_rx => head.ok(),
        // The turn ended (or failed) with no head: nothing to act on early.
        reply = &mut whole => return finish_whole(reply?, history, services).await,
    };
    let Some((head, took)) = head.filter(|(head, _)| !promises(&head.reply)) else {
        let reply = whole.await?;
        return finish_whole(reply, history, services).await;
    };
    let speech = match tone(head.status) {
        Some(tone) => Some(services.tts.speak(&head.reply, tone).await?),
        None => None,
    };
    let early_turn = head.clone().into_turn(String::new());
    let _ = early.send(Early {
        generation,
        reply: Reply { turn: early_turn.clone(), raw: String::new(), took },
        speech,
    });
    // The reply is out; a failure writing the summary costs only the summary.
    let reply = match whole.await {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("agent: the turn failed after its reply was written: {e}");
            let raw = serde_json::to_string(&early_turn).unwrap_or_default();
            Reply { turn: early_turn, raw, took }
        }
    };
    Ok((reply, None, true))
}

/// The tone for a reply, or `None` for an `emergency` turn, which is never spoken.
fn tone(status: Status) -> Option<Tone> {
    match status {
        Status::Emergency => None,
        Status::Concern => Some(Tone::Steady),
        Status::Ok => Some(Tone::Warm),
    }
}

/// A whole turn not acted on early: rewritten once if it promises something, then spoken.
async fn finish_whole<L: Llm>(
    mut reply: Reply,
    history: &[Message],
    services: &Services<L>,
) -> Result<(Reply, Option<Speech>, bool), Error> {
    if promises(&reply.turn.reply) {
        eprintln!("agent: the reply promised something, asking once more: {:?}", reply.turn.reply);
        let mut again = history.to_vec();
        again.push(Message::new(Role::Assistant, reply.raw.clone()));
        again.push(Message::new(Role::User, NO_PROMISES));
        let first = reply.took;
        reply = services.llm.turn(&again).await?;
        reply.took += first;
    }
    let Some(tone) = tone(reply.turn.status) else { return Ok((reply, None, false)) };
    let speech = services.tts.speak(&reply.turn.reply, tone).await?;
    Ok((reply, Some(speech), false))
}

/// A tag such as `[BLANK_AUDIO]` or `(wind blowing)`, which whisper writes for a clip
/// without words (inferred; not yet seen in this project's calls), isn't something the
/// resident said.
fn is_noise(text: &str) -> bool {
    let text = text.trim();
    text.is_empty()
        || (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_tags_are_noise_and_words_are_not() {
        assert!(is_noise(""));
        assert!(is_noise(" [BLANK_AUDIO]"));
        assert!(is_noise("(wind blowing)"));
        assert!(!is_noise("I'm fine, thank you."));
    }

    #[test]
    fn a_promise_for_later_is_caught() {
        assert!(promises("I'll make sure someone comes with a sandwich. Goodbye."));
        assert!(promises("I'll check in again later on the pain, just to be safe."));
        assert!(promises("I'll keep an eye on that. Have you eaten?"));
        assert!(promises("Thanks for letting me know. I'll see you soon."));
        assert!(!promises("I'm sorry to hear that. I'll pass that on. Have you eaten today?"));
        assert!(!promises("That's good to hear. Do you need anything right now?"));
    }

    #[test]
    fn a_goodbye_that_asks_nothing_ends_the_call() {
        // The live miss from issue #35.
        assert!(says_goodbye(
            "Thank you for telling me. I'll pass on that you haven't eaten today. Goodbye."
        ));
        assert!(says_goodbye("Take care now. Bye-bye."));
        assert!(says_goodbye("Okay, bye for now!"));
    }

    #[test]
    fn a_reply_that_asks_something_does_not_end_the_call() {
        assert!(!says_goodbye("Before I say goodbye, is there anything you need?"));
        assert!(!says_goodbye("Have you eaten anything today?"));
        assert!(!says_goodbye("That's good to hear. Anything else on your mind?"));
        assert!(!says_goodbye("I'll pass that on."));
    }
}
