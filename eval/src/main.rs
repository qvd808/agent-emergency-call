//! `make eval` (issue #23): scripted residents call the agent one at a time, in real time,
//! through the same path as a live call, and the gates of issue #29 are checked on the result.
//!
//! The agent runs in this process, answering through `agent::call::answer` on a loopback port,
//! exactly as `make agent` answers Asterisk. Its Call control has a recorder in place of AMI;
//! on a transfer the fake caller closes the socket, as Asterisk does after a `Redirect`. The
//! residents' voice is Piper's `lessac`, a different voice from the agent's, rendered at 8 kHz
//! before the first call.
//!
//! Settings, from the environment or `.env`:
//! - `REPEAT`: runs of each persona; the gates must hold on every one (default 1).
//! - `PERSONA`: a comma-separated list of personas to run (default all).
//! - `EVAL_TEMPERATURE`: the LLM's temperature (default 0).
//! - `EVAL_VOICE`: the residents' Piper voice (default lessac).
//! - The agent's own settings, as for `make agent`: `OLLAMA_*`, `WHISPER_MODEL`, `VAD_MODEL`,
//!   `PIPER_VOICE`, `DISPATCHER_EXTENSION`.
//!
//! Call logs and `report.md` go in `calls/eval/<run>/`. Exits 1 if a gate fails.

mod caller;
mod personas;
mod report;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent::audio::{CORE_RATE_HZ, LINE_RATE_HZ, resample_clip};
use agent::call::{
    DEFAULT_VAD_MODEL, DEFAULT_WHISPER_MODEL, answer, env, ollama_from_env, start_services,
};
use agent::conversation::{Services, TURN_END};
use agent::llm::{Head, Llm, Message, Reply};
use agent::stt::{Stt, Whisper};
use agent::telephony::Recorder;
use agent::telephony::audiosocket::Uuid;
use agent::tts::{Tone, Tts, Voice};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use caller::{Act, PauseAt, Rendered};
use personas::{Part, Persona, Step};

type Error = Box<dyn std::error::Error + Send + Sync>;

const DEFAULT_RESIDENT_VOICE: &str = "models/en_US-lessac-medium.onnx";

/// The extension every scripted resident calls from, as AMI would report it.
const RESIDENT_EXTENSION: &str = "1001";

/// An LLM that always fails, for the agent-fault persona (issue #29).
struct Failing;

impl Llm for Failing {
    async fn turn(&self, _: &[Message]) -> Result<Reply, agent::llm::Error> {
        Err("the eval's failing LLM stub".into())
    }

    async fn turn_streaming(
        &self,
        _: &[Message],
        _: oneshot::Sender<(Head, Duration)>,
    ) -> Result<Reply, agent::llm::Error> {
        Err("the eval's failing LLM stub".into())
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    let repeat: usize = env("REPEAT").map_or(Ok(1), |r| r.parse())?;
    let temperature: f32 = env("EVAL_TEMPERATURE").map_or(Ok(0.0), |t| t.parse())?;
    let only: Option<Vec<String>> =
        env("PERSONA").map(|p| p.split(',').map(|s| s.trim().to_string()).collect());
    let personas: Vec<Persona> = personas::all()
        .into_iter()
        .filter(|p| only.as_ref().is_none_or(|only| only.iter().any(|n| n == p.name)))
        .collect();
    if personas.is_empty() {
        return Err(format!("PERSONA matches none of: {}", names(&personas::all())).into());
    }
    let run = timestamp(SystemTime::now());
    let dir = PathBuf::from("calls/eval").join(&run);

    // The agent, set up as `make agent` sets it up.
    let whisper_model = env("WHISPER_MODEL").unwrap_or_else(|| DEFAULT_WHISPER_MODEL.to_string());
    let vad_model = env("VAD_MODEL").unwrap_or_else(|| DEFAULT_VAD_MODEL.to_string());
    let stt = Stt::start(Whisper::load(&whisper_model).map_err(|e| format!("{whisper_model}: {e}"))?);
    let (llm, model) = ollama_from_env();
    let mut services = start_services(stt, llm.with_temperature(temperature), &model).await?;
    services.calls_dir = dir.clone();
    let failing = Services {
        stt: services.stt.clone(),
        tts: services.tts.clone(),
        llm: Arc::new(Failing),
        lines: services.lines.clone(),
        dispatcher: services.dispatcher.clone(),
        calls_dir: services.calls_dir.clone(),
        barge_in: services.barge_in,
        end_of_turn: services.end_of_turn.clone(),
        record: services.record,
    };

    // The residents.
    let voice = env("EVAL_VOICE").unwrap_or_else(|| DEFAULT_RESIDENT_VOICE.to_string());
    let residents = Tts::start(Voice::load(&voice).map_err(|e| format!("{voice}: {e}"))?);
    let mut scripts = Vec::new();
    for persona in &personas {
        scripts.push(render_script(&residents, persona).await?);
    }
    let mut fallback = Vec::new();
    for line in personas::FALLBACK {
        fallback.push(render(&residents, line).await?);
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let total = personas.len() * repeat;
    println!("eval: run {run}, {total} calls in real time; the agent's log goes to stderr");
    let mut runs = Vec::new();
    for rep in 1..=repeat {
        for (persona, script) in personas.iter().zip(&scripts) {
            let n = runs.len() + 1;
            println!("eval: [{n}/{total}] {} (run {rep})", persona.name);
            let uuid = new_uuid(n);
            let (transfers_tx, transfers) = mpsc::unbounded_channel();
            let recorder = Recorder { caller: RESIDENT_EXTENSION.to_string(), transfers: transfers_tx };
            let call = caller::run(addr, uuid, acts(script), fallback.clone(), transfers);
            let agent = async {
                let (stream, _) = listener.accept().await?;
                let pbx = |control: agent::telephony::CallControl, _: &Uuid| control.with_recorder(recorder);
                if persona.failing_llm {
                    answer(stream, &vad_model, failing.clone(), pbx).await
                } else {
                    answer(stream, &vad_model, services.clone(), pbx).await
                }
            };
            let (outcome, answered) = tokio::join!(call, agent);
            let outcome = outcome?;
            if let Err(e) = answered {
                eprintln!("eval: the agent's side of the call failed: {e}");
            }
            let log_path = dir.join(format!("{uuid}.json"));
            let log = std::fs::read_to_string(&log_path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            println!(
                "eval:   {:.1} s, {}; {}",
                outcome.duration.as_secs_f64(),
                match &log {
                    Some(log) => match log["escalation"]["trigger"].as_str() {
                        Some(trigger) => format!(
                            "escalated ({trigger}) on turn {}",
                            log["escalation"]["turn"].as_u64().unwrap_or(0)
                        ),
                        None => format!("no escalation, status {}", log["final_status"].as_str().unwrap_or("?")),
                    },
                    None => "no call log".to_string(),
                },
                log_path.display()
            );
            runs.push(report::Run { persona, repeat: rep, outcome, log });
        }
    }

    let meta = report::Meta {
        run: run.clone(),
        model: format!("{model}"),
        temperature,
        repeat,
        dispatcher: services.dispatcher.clone(),
        keyword_limit_ms: (TURN_END + 1.0) * 1000.0,
    };
    let (text, pass) = report::render(&runs, &meta);
    std::fs::create_dir_all(&dir)?;
    let report_path = dir.join("report.md");
    std::fs::write(&report_path, &text)?;
    println!("\n{text}\neval: report written to {}", report_path.display());
    std::process::exit(if pass { 0 } else { 1 });
}

fn names(personas: &[Persona]) -> String {
    personas.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")
}

/// A persona's script, rendered.
enum RenderedStep {
    Say(Rendered),
    Interrupt(f64, Rendered),
    Silent,
}

async fn render_script(tts: &Tts, persona: &Persona) -> Result<Vec<RenderedStep>, Error> {
    let mut script = Vec::new();
    for step in &persona.steps {
        script.push(match step {
            Step::Say(line) => RenderedStep::Say(render(tts, line).await?),
            Step::Interrupt(after, line) => RenderedStep::Interrupt(*after, render(tts, line).await?),
            Step::Silent => RenderedStep::Silent,
        });
    }
    Ok(script)
}

fn acts(script: &[RenderedStep]) -> Vec<Act> {
    script
        .iter()
        .map(|step| match step {
            RenderedStep::Say(line) => Act::Say(line.clone()),
            RenderedStep::Interrupt(after, line) => Act::Interrupt(*after, line.clone()),
            RenderedStep::Silent => Act::Silent,
        })
        .collect()
}

/// A scripted line as 8 kHz audio, with its pauses as exact silence.
async fn render(tts: &Tts, line: &str) -> Result<Rendered, Error> {
    let mut samples: Vec<i16> = Vec::new();
    let mut pauses = Vec::new();
    let mut last_words = String::new();
    for part in personas::parts(line) {
        match part {
            Part::Words(words) => {
                let speech = tts.speak(&words, Tone::Steady).await?;
                let line_rate = resample_clip(&speech.audio, CORE_RATE_HZ, LINE_RATE_HZ);
                samples.extend_from_slice(trim(&line_rate));
                last_words = words;
            }
            Part::Pause(seconds) => {
                let start = samples.len();
                samples.resize(start + (seconds * LINE_RATE_HZ as f64) as usize, 0);
                let before = personas::before_pause(&last_words);
                pauses.push(PauseAt { start, seconds, before });
            }
        }
    }
    Ok(Rendered { text: line.to_string(), samples, pauses })
}

/// Piper's audio without its quiet lead-in and tail, so a line's end is its last word and a
/// pause is exactly as long as scripted. Keeps 10 ms either side.
fn trim(samples: &[i16]) -> &[i16] {
    const QUIET: u16 = 300;
    let margin = LINE_RATE_HZ as usize / 100;
    let first = samples.iter().position(|s| s.unsigned_abs() >= QUIET).unwrap_or(0);
    let end = samples.iter().rposition(|s| s.unsigned_abs() >= QUIET).map_or(samples.len(), |i| i + 1);
    &samples[first.saturating_sub(margin)..(end + margin).min(samples.len())]
}

/// Distinct per call and per run, which is all the call log's file name needs.
fn new_uuid(n: usize) -> Uuid {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let mut bytes = nanos.to_be_bytes();
    bytes[..2].copy_from_slice(&(n as u16).to_be_bytes());
    Uuid(bytes)
}

/// `2026-09-24T21-30-05Z`, in UTC, for the run's directory.
fn timestamp(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's `civil_from_days`).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}-{:02}-{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00-00-00Z");
        // 2026-09-24 21:30:05 UTC.
        let at = UNIX_EPOCH + Duration::from_secs(1_790_285_405);
        assert_eq!(timestamp(at), "2026-09-24T21-30-05Z");
    }

    #[test]
    fn trimming_keeps_the_words_and_10_ms_either_side() {
        let mut clip = vec![0i16; 800];
        clip.extend(vec![5000i16; 400]);
        clip.extend(vec![0i16; 800]);
        let trimmed = trim(&clip);
        assert_eq!(trimmed.len(), 400 + 2 * 80);
    }
}
