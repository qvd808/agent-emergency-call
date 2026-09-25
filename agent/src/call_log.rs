//! The call log (issue #21): one JSON file per call in `calls/`, and the concern flag, which
//! reaches the dispatcher as a mock notice. The notice only prints and goes in the log; nothing
//! is sent anywhere.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::checklist::{Asking, Checklist};
use crate::escalation::Escalation;
use crate::llm::Status;

#[derive(Debug, Serialize)]
pub struct CallLog {
    /// The call's AudioSocket UUID, also the file's name.
    pub call: String,
    /// The caller's extension. There is no resident list yet, so this is who the resident is.
    pub resident: Option<String>,
    /// Wall-clock start, in milliseconds since the Unix epoch.
    pub started_unix_ms: u128,
    pub duration_s: f64,
    /// Everything said, by both sides, in order.
    pub transcript: Vec<Line>,
    pub turns: Vec<TurnLog>,
    /// When the agent decided a turn was over but speech to text had heard only noise in it,
    /// in seconds since the call started. Nothing was answered; the agent listened on.
    pub unheard_turns_at_s: Vec<f64>,
    /// Where each item of the check-in ended up (issue #35).
    pub checklist: Checklist,
    /// The highest status any turn reached: it only ever rises (issue #11).
    pub final_status: Status,
    /// The LLM's last summary of the call.
    pub summary: String,
    pub escalation: Option<Escalation>,
    pub concern_flag: Option<ConcernFlag>,
    pub ended_by: EndedBy,
    /// Every time the resident talked over the agent (issue #19).
    pub barge_ins: Vec<BargeInLog>,
    /// How much of the agent's own voice came back on the resident's line (issue #34).
    pub echo: EchoLog,
}

/// One time the resident talked over the agent and its audio paused (issue #19).
#[derive(Debug, Clone, Serialize)]
pub struct BargeInLog {
    /// Seconds since the call started.
    pub paused_at_s: f64,
    /// What the agent was saying.
    pub over: String,
    /// `confirmed: ...` (the resident took the turn, and why), `resumed` (the agent played on),
    /// or what else ended the pause.
    pub outcome: String,
    pub paused_ms: u64,
}

/// The resident's line while the agent talked, against its level while neither side did
/// (issue #34, narrowed to the demo setup). On a call where the resident stays silent, what the
/// line carries while the agent talks is the agent's own voice coming back, plus noise, and
/// every VAD fire then is a false one. Levels are RMS per VAD window, in dB below full scale.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EchoLog {
    /// Seconds of the agent's audio going out, not paused.
    pub agent_talking_s: f64,
    /// The agent's own speech, as sent.
    pub agent_level_dbfs: Option<f64>,
    pub line_while_talking_p50_dbfs: Option<f64>,
    pub line_while_talking_p95_dbfs: Option<f64>,
    /// VAD windows at or over the speech threshold while the agent talked, out of
    /// `windows_while_talking`. The resident's real barge-ins count too.
    pub vad_fires_while_talking: u32,
    pub windows_while_talking: u32,
    /// Neither side talking: the line's noise floor.
    pub line_quiet_p50_dbfs: Option<f64>,
}

/// Collects an [`EchoLog`] over a call.
#[derive(Debug, Default)]
pub struct EchoMeter {
    while_talking: Vec<f64>,
    quiet: Vec<f64>,
    fires: u32,
    agent_squares: f64,
    agent_samples: u64,
}

/// Quieter than any real line; stands in for exact digital silence.
const FLOOR_DBFS: f64 = -120.0;

fn dbfs(mean_square: f64) -> f64 {
    if mean_square <= 0.0 { FLOOR_DBFS } else { (10.0 * mean_square.log10()).max(FLOOR_DBFS) }
}

impl EchoMeter {
    /// One VAD window of the resident's line, in [-1, 1], with its speech probability.
    pub fn window(&mut self, samples: &[f32], p: f32, agent_talking: bool) {
        let mean_square =
            samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64;
        let level = dbfs(mean_square);
        if agent_talking {
            self.while_talking.push(level);
            self.fires += (p >= turn::VAD_ON) as u32;
        } else if p < turn::VAD_ON {
            self.quiet.push(level);
        }
    }

    /// Audio the agent queued to say.
    pub fn agent_audio(&mut self, audio: &[i16]) {
        self.agent_squares += audio.iter().map(|&s| (s as f64 / 32_768.0).powi(2)).sum::<f64>();
        self.agent_samples += audio.len() as u64;
    }

    pub fn log(&self) -> EchoLog {
        let percentile = |values: &[f64], p: f64| -> Option<f64> {
            let mut sorted = values.to_vec();
            sorted.sort_by(f64::total_cmp);
            let rank = ((p * sorted.len() as f64).ceil() as usize).max(1);
            sorted.get(rank - 1).map(|v| (v * 10.0).round() / 10.0)
        };
        EchoLog {
            agent_talking_s: self.while_talking.len() as f64 * turn::WINDOW_S,
            agent_level_dbfs: (self.agent_samples > 0).then(|| {
                (dbfs(self.agent_squares / self.agent_samples as f64) * 10.0).round() / 10.0
            }),
            line_while_talking_p50_dbfs: percentile(&self.while_talking, 0.5),
            line_while_talking_p95_dbfs: percentile(&self.while_talking, 0.95),
            vad_fires_while_talking: self.fires,
            windows_while_talking: self.while_talking.len() as u32,
            line_quiet_p50_dbfs: percentile(&self.quiet, 0.5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndedBy {
    /// The agent said goodbye and hung up.
    Goodbye,
    /// The line closed first: the resident hung up, or the call dropped.
    LineClosed,
    /// Escalated: the call was handed to the dispatcher, or the handoff was tried.
    Escalated,
    /// The agent itself failed and hung up.
    AgentError,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub speaker: Who,
    pub text: String,
    /// Seconds since the call started.
    pub at_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Agent,
    Resident,
}

/// One of the resident's turns and the agent's answer to it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TurnLog {
    /// Counting from 1.
    pub turn: usize,
    pub heard: String,
    pub reply: Option<String>,
    pub status: Option<Status>,
    pub reason: Option<String>,
    /// What the reply asked about.
    pub asking: Option<Asking>,
    pub end_call: bool,
    /// When the agent decided the resident's turn was over, in seconds since the call started.
    pub taken_at_s: f64,
    /// Earlier times the agent took this turn and gave it back because the resident went on
    /// before any reply had played (issue #19).
    pub taken_back_at_s: Vec<f64>,
    /// Smart Turn's P(complete) at each pause in the turn (issue #19).
    pub smart_turn_p: Vec<f32>,
    /// Whether Smart Turn held the floor at the pause that ended the turn, so the agent waited
    /// 3 s instead of 1.5 s.
    pub held: bool,
    /// From the end of the resident's speech to that decision: the end-of-turn wait, plus any
    /// wait for the last transcript.
    pub end_of_turn_ms: u64,
    /// From the end of the resident's speech to the first frame of the reply going out.
    pub latency_ms: Option<u64>,
    /// Speech to text for the turn's utterances, added up.
    pub stt_ms: u64,
    /// Until the model had written the reply, which is when speaking can start.
    pub llm_ms: Option<u64>,
    /// Until the model had written the whole turn, summary included.
    pub llm_total_ms: Option<u64>,
    pub tts_ms: Option<u64>,
    /// Whether the reply was slow enough that the acknowledgement played first (issue #45).
    pub acknowledged: bool,
    /// Whether the resident took the turn by talking over the reply (issue #19).
    pub interrupted: bool,
}

/// A notice to the dispatcher that something can wait for a callback (glossary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConcernFlag {
    pub reasons: Vec<String>,
    pub summary: String,
}

/// Why the check-in ended early, as a concern reason.
pub const ENDED_EARLY: &str = "the check-in ended early, before the agent's goodbye";

/// The concern flag a call raises, if any (issue #11): one per call, at the end, for a
/// `concern` status or a check-in that ended before the goodbye. An escalated call raises none;
/// its log holds everything.
pub fn concern_flag(
    status: Status,
    reasons: &[String],
    summary: &str,
    ended_by: EndedBy,
) -> Option<ConcernFlag> {
    let ended_early = matches!(ended_by, EndedBy::LineClosed | EndedBy::AgentError);
    if ended_by == EndedBy::Escalated || (status < Status::Concern && !ended_early) {
        return None;
    }
    let mut reasons = reasons.to_vec();
    if ended_early {
        reasons.push(ENDED_EARLY.to_string());
    }
    Some(ConcernFlag { reasons, summary: summary.to_string() })
}

/// The mock notice: printed only.
pub fn print_notice(call: &str, resident: Option<&str>, flag: &ConcernFlag) {
    let who = match resident {
        Some(extension) => format!("the resident on extension {extension}"),
        None => "a resident on an unknown extension".to_string(),
    };
    eprintln!(
        "agent: {call}: MOCK NOTICE TO THE DISPATCHER (not sent anywhere): concern flag for {who}. \
         Why: {}. Summary: {}",
        flag.reasons.join("; "),
        if flag.summary.is_empty() { "none" } else { &flag.summary },
    );
}

/// Writes `<dir>/<call>.json`.
pub fn write(dir: &Path, log: &CallLog) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", log.call));
    let json = serde_json::to_string_pretty(log).map_err(std::io::Error::other)?;
    std::fs::write(&path, json + "\n")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(status: Status, ended_by: EndedBy) -> Option<ConcernFlag> {
        concern_flag(status, &["hasn't eaten today".to_string()], "Summary.", ended_by)
    }

    #[test]
    fn a_fine_call_that_said_goodbye_raises_no_flag() {
        assert_eq!(flag(Status::Ok, EndedBy::Goodbye), None);
    }

    #[test]
    fn a_concern_raises_one_flag_with_its_reasons() {
        let flag = flag(Status::Concern, EndedBy::Goodbye).unwrap();
        assert_eq!(flag.reasons, ["hasn't eaten today"]);
        assert_eq!(flag.summary, "Summary.");
    }

    #[test]
    fn a_check_in_that_ends_early_raises_a_flag_even_when_fine() {
        let flag = flag(Status::Ok, EndedBy::LineClosed).unwrap();
        assert_eq!(flag.reasons.last().map(String::as_str), Some(ENDED_EARLY));
    }

    #[test]
    fn an_escalated_call_raises_none() {
        assert_eq!(flag(Status::Emergency, EndedBy::Escalated), None);
        assert_eq!(flag(Status::Concern, EndedBy::Escalated), None);
    }

    #[test]
    fn echo_levels_are_in_db_below_full_scale() {
        let mut meter = EchoMeter::default();
        // A full-scale square wave is 0 dBFS; one at a tenth of it is -20 dBFS.
        meter.window(&[1.0, -1.0, 1.0, -1.0], 0.1, false);
        meter.window(&[0.1, -0.1, 0.1, -0.1], 0.9, true);
        meter.window(&[0.0; 4], 0.1, true);
        let log = meter.log();
        assert_eq!(log.line_quiet_p50_dbfs, Some(0.0));
        assert_eq!(log.line_while_talking_p50_dbfs, Some(FLOOR_DBFS));
        assert_eq!(log.line_while_talking_p95_dbfs, Some(-20.0));
        assert_eq!((log.vad_fires_while_talking, log.windows_while_talking), (1, 2));
        assert_eq!(log.agent_level_dbfs, None);
    }

    #[test]
    fn statuses_order_from_ok_to_emergency() {
        assert!(Status::Ok < Status::Concern && Status::Concern < Status::Emergency);
    }
}
