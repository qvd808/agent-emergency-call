//! The check-in conversation (issue #18): the fixed greeting, then turn after turn of the
//! resident speaking, the LLM answering with a [`Turn`], and Piper speaking the reply, until
//! the LLM says goodbye with `end_call` and the agent hangs up.
//!
//! A first version, to be built on:
//! - The resident's turn ends after [`TURN_END`] of silence, issue #12's base. Smart Turn's
//!   hold, barge-in and per-turn latency in the call log land with issue #19.
//! - Half duplex: while the agent thinks or speaks, it doesn't listen. Its own voice can't be
//!   mistaken for the resident's, but anything the resident says then is lost.
//! - An `emergency` status, and an agent fault after the one retry, are logged only.
//!   Escalation lands with issue #20.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use turn::vad::{Silero, WINDOW};

use crate::audio::CORE_RATE_HZ;
use crate::listen::Segmenter;
use crate::llm::{Llm, Message, Reply, Role, SYSTEM_PROMPT, Status};
use crate::stt::{self, Stt, Transcript};
use crate::telephony::{CallControl, MarkId, Media, Speaker};
use crate::tts::{Speech, Tts};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Silence that ends the resident's turn: the base wait in issue #12's first iteration.
pub const TURN_END: f64 = 1.5;

/// Silence after the goodbye, before hanging up, so the phone has played the last word by the
/// time the call ends. The mark only says the audio reached Asterisk; how much the phone still
/// holds in its jitter buffer is unknown (inferred size).
const HANGUP_TAIL: f64 = 0.5;

/// Fixed lines, spoken the same way on every call and never written by the LLM (issue #10).
pub const GREETING_INBOUND: &str =
    "Hello. This is the automated check-in assistant. How are you feeling today?";
pub const HOLDING: &str = "Sorry, give me a moment.";

/// The fixed lines, synthesised once at startup.
pub struct Lines {
    pub greeting: Vec<i16>,
    pub holding: Vec<i16>,
}

impl Lines {
    pub async fn synthesise(tts: &Tts) -> Result<Self, Error> {
        Ok(Lines {
            greeting: tts.speak(GREETING_INBOUND).await?.audio,
            holding: tts.speak(HOLDING).await?.audio,
        })
    }
}

/// What every call shares.
pub struct Services<L> {
    pub stt: Stt,
    pub tts: Tts,
    pub llm: Arc<L>,
    pub lines: Arc<Lines>,
}

impl<L> Clone for Services<L> {
    fn clone(&self) -> Self {
        Services {
            stt: self.stt.clone(),
            tts: self.tts.clone(),
            llm: self.llm.clone(),
            lines: self.lines.clone(),
        }
    }
}

/// How one of the resident's turns was answered.
enum Answer {
    /// Nothing but noise was heard.
    Nothing,
    Reply { heard: String, reply: Reply, speech: Speech },
    /// The LLM or TTS failed twice.
    Fault { heard: String, error: Error },
}

/// Runs one check-in until either side hangs up. `call` names the call in the log.
pub async fn check_in<L: Llm + 'static>(
    call: String,
    media: Media,
    control: CallControl,
    mut vad: Silero,
    services: Services<L>,
) {
    let Media { mut frames, mut played, speaker } = media;
    let mut marks: MarkId = 0;
    let mut history = vec![Message::new(Role::System, SYSTEM_PROMPT)];
    let mut segmenter = Segmenter::default();
    let mut pending: Vec<i16> = Vec::with_capacity(WINDOW * 2);
    // Transcripts of the current turn's utterances, in order.
    let mut heard: Vec<JoinHandle<Result<Transcript, stt::Error>>> = Vec::new();
    // When the resident last stopped speaking, by the wall clock.
    let mut stopped = Instant::now();
    let mut thinking: Option<JoinHandle<(Vec<Message>, Answer)>> = None;
    // The mark before a reply's first audio, and when the resident stopped before it.
    let mut first_audio: Option<(MarkId, Instant)> = None;

    speaker.play(services.lines.greeting.clone());
    marks += 1;
    speaker.mark(marks);
    // The mark after the agent's current speech, and whether to hang up once it has played.
    let mut speaking: Option<(MarkId, bool)> = Some((marks, false));
    eprintln!("agent: {call}: said {GREETING_INBOUND:?}");

    loop {
        tokio::select! {
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    eprintln!("agent: {call}: the resident hung up");
                    break;
                };
                pending.extend_from_slice(&frame);
                while pending.len() >= WINDOW {
                    let window: Vec<i16> = pending.drain(..WINDOW).collect();
                    let samples: Vec<f32> = window.iter().map(|&s| s as f32 / 32_768.0).collect();
                    // Runs on every window, heard or not, so Silero's state stays continuous.
                    let p = match vad.prob(&samples) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("agent: {call}: VAD failed, hanging up: {e}");
                            control.hangup();
                            return;
                        }
                    };
                    if thinking.is_some() || speaking.is_some() {
                        continue;
                    }
                    if let Some(utterance) = segmenter.push(&window, p) {
                        // Frames arrive in real time, so the resident stopped this long before
                        // now (inferred, as in listen.rs).
                        stopped = Instant::now()
                            - Duration::from_secs_f64(segmenter.now() - utterance.end);
                        let stt = services.stt.clone();
                        heard.push(tokio::spawn(async move { stt.transcribe(utterance.audio).await }));
                    }
                    if !heard.is_empty() && segmenter.quiet_for() >= TURN_END {
                        let turn = std::mem::take(&mut heard);
                        let history = std::mem::take(&mut history);
                        let (services, speaker) = (services.clone(), speaker.clone());
                        thinking = Some(tokio::spawn(respond(turn, history, services, speaker)));
                    }
                }
            }
            answer = async { thinking.as_mut().unwrap().await }, if thinking.is_some() => {
                thinking = None;
                segmenter.restart();
                let answer = match answer {
                    Ok((h, answer)) => {
                        history = h;
                        answer
                    }
                    Err(e) => {
                        eprintln!("agent: {call}: the turn task failed, hanging up: {e}");
                        control.hangup();
                        return;
                    }
                };
                match answer {
                    Answer::Nothing => {}
                    Answer::Fault { heard, error } => {
                        eprintln!(
                            "agent: {call}: heard {heard:?}; agent fault after one retry: \
                             {error}. Escalation isn't built yet (issue #20)"
                        );
                    }
                    Answer::Reply { heard, reply, speech } => {
                        let turn = &reply.turn;
                        eprintln!(
                            "agent: {call}: heard {heard:?}\nagent: {call}: said {:?} [{:?}{}{}] \
                             (LLM {} ms, TTS {} ms)\nagent: {call}: summary {:?}",
                            turn.reply,
                            turn.status,
                            turn.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default(),
                            if turn.end_call { ", end_call" } else { "" },
                            reply.took.as_millis(),
                            speech.took.as_millis(),
                            turn.summary,
                        );
                        if turn.status == Status::Emergency {
                            eprintln!(
                                "agent: {call}: EMERGENCY: escalation isn't built yet (issue #20)"
                            );
                        }
                        marks += 1;
                        speaker.mark(marks);
                        first_audio = Some((marks, stopped));
                        speaker.play(speech.audio);
                        if turn.end_call {
                            let tail = (HANGUP_TAIL * CORE_RATE_HZ as f64) as usize;
                            speaker.play(vec![0; tail]);
                        }
                        marks += 1;
                        speaker.mark(marks);
                        speaking = Some((marks, turn.end_call));
                    }
                }
            }
            Some(mark) = played.recv() => {
                if let Some((first, since)) = first_audio
                    && mark == first
                {
                    first_audio = None;
                    eprintln!(
                        "agent: {call}: reply started {} ms after the resident stopped",
                        since.elapsed().as_millis()
                    );
                }
                if let Some((end, hang_up)) = speaking
                    && mark == end
                {
                    speaking = None;
                    segmenter.restart();
                    if hang_up {
                        eprintln!("agent: {call}: goodbye played, hanging up");
                        control.hangup();
                        break;
                    }
                }
            }
        }
    }
}

/// Answers one of the resident's turns: waits for its transcripts, asks the LLM, and
/// synthesises the reply. On a failure it says the holding line and tries once more.
async fn respond<L: Llm>(
    heard: Vec<JoinHandle<Result<Transcript, stt::Error>>>,
    mut history: Vec<Message>,
    services: Services<L>,
    speaker: Speaker,
) -> (Vec<Message>, Answer) {
    let mut words = Vec::new();
    for transcript in heard {
        match transcript.await {
            Ok(Ok(t)) if !is_noise(&t.text) => words.push(t.text),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("agent: transcription failed: {e}"),
            Err(e) => eprintln!("agent: transcription task failed: {e}"),
        }
    }
    let heard = words.join(" ");
    if heard.is_empty() {
        return (history, Answer::Nothing);
    }
    history.push(Message::new(Role::User, heard.clone()));

    let mut attempt = 0;
    loop {
        attempt += 1;
        match answer(&history, &services).await {
            Ok((reply, speech)) => {
                history.push(Message::new(Role::Assistant, reply.raw.clone()));
                return (history, Answer::Reply { heard, reply, speech });
            }
            Err(error) if attempt >= 2 => return (history, Answer::Fault { heard, error }),
            Err(error) => {
                eprintln!("agent: the turn failed, retrying once: {error}");
                speaker.play(services.lines.holding.clone());
            }
        }
    }
}

async fn answer<L: Llm>(history: &[Message], services: &Services<L>) -> Result<(Reply, Speech), Error> {
    let reply = services.llm.turn(history).await?;
    let speech = services.tts.speak(&reply.turn.reply).await?;
    Ok((reply, speech))
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
}
