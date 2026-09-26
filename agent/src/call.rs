//! One call to extension 3100, from the moment Asterisk connects until the call ends, and the
//! services every call shares. The live agent (`main.rs`) and the eval (issue #23) both answer
//! calls through [`answer`], so the eval runs the path a real call takes.
//!
//! Settings come from the environment; `main.rs` reads `.env` into it first.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use turn::smart_turn::SmartTurn;
use turn::vad::Silero;

use crate::conversation::{Lines, Services, check_in};
use crate::end_of_turn::EndOfTurn;
use crate::llm::{Llm, Message, Ollama, Role, SYSTEM_PROMPT};
use crate::schedule::{Outbound, Policy, Schedule};
use crate::stt::Stt;
use crate::telephony::CallControl;
use crate::telephony::ami::check_extension;
use crate::telephony::audiosocket::Uuid;
use crate::telephony::line::{self, LineStats};
use crate::tts::{Tts, Voice};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Where `make models` puts them. `tiny.en` is the fastest of `tiny.en`, `base.en` and
/// `small.en` on this laptop's CPU, and the only one inside the 1 s budget (issue #17).
/// All three misheard "fall" on a live call; large-v3-turbo heard it, but takes 14-17 s a clip
/// on the CPU, so only the GPU build uses it (issue #52).
#[cfg(not(feature = "cuda"))]
pub const DEFAULT_WHISPER_MODEL: &str = "models/ggml-tiny.en.bin";
#[cfg(feature = "cuda")]
pub const DEFAULT_WHISPER_MODEL: &str = "models/ggml-large-v3-turbo-q5_0.bin";
pub const DEFAULT_VAD_MODEL: &str = "models/silero_vad.onnx";
pub const DEFAULT_VOICE: &str = "models/en_US-hfc_female-medium.onnx";
pub const DEFAULT_SMART_TURN_MODEL: &str = "models/smart-turn-v3.2-cpu.onnx";

/// The same defaults as `.env.example`.
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3:4b-instruct-2507-q4_K_M";
const DEFAULT_DISPATCHER: &str = "2000";
const DEFAULT_CALLS_DIR: &str = "calls";
const DEFAULT_RESIDENTS: &str = "1001";

/// A setting from the environment. Empty counts as unset.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A switch that is on unless the environment turns it off.
fn switched_off(name: &str) -> bool {
    env(name).is_some_and(|v| matches!(v.to_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

/// A switch that is off unless the environment turns it on.
fn switched_on(name: &str) -> bool {
    env(name).is_some_and(|v| matches!(v.to_lowercase().as_str(), "on" | "1" | "true" | "yes"))
}

/// The LLM the environment names: Ollama at `OLLAMA_URL`, running `OLLAMA_MODEL`. Returns it
/// with a name for the log.
pub fn ollama_from_env() -> (Ollama, String) {
    let url = env("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());
    let model = env("OLLAMA_MODEL").unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());
    (Ollama::new(&url, &model), format!("{model} at {url}"))
}

/// When and whom outbound check-ins call (issue #22), from `RESIDENTS` and the `CHECKIN_*`
/// settings, as `.env.example` describes them. Every resident's extension passes the same check
/// as the dispatcher's, before any call is placed.
pub fn schedule_from_env() -> Result<Schedule, Error> {
    let residents: Vec<String> = env("RESIDENTS")
        .unwrap_or_else(|| DEFAULT_RESIDENTS.to_string())
        .split(',')
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    for resident in &residents {
        check_extension(resident).map_err(|e| format!("RESIDENTS: {e}"))?;
    }
    let defaults = Policy::default();
    let policy = Policy {
        attempts: number("CHECKIN_ATTEMPTS", defaults.attempts.into())? as u32,
        retry_after: Duration::from_secs(number("CHECKIN_RETRY_AFTER_S", defaults.retry_after.as_secs())?),
        ring: Duration::from_secs(number("CHECKIN_RING_S", defaults.ring.as_secs())?),
    };
    let every = match env("CHECKIN_EVERY_MIN") {
        Some(_) => Some(Duration::from_secs(60 * number("CHECKIN_EVERY_MIN", 0)?)),
        None => None,
    };
    Ok(Schedule { residents, now: switched_on("CHECKIN_NOW"), every, policy })
}

/// A whole number from the environment, at least 1, or `default` when unset.
fn number(name: &str, default: u64) -> Result<u64, Error> {
    let Some(value) = env(name) else { return Ok(default) };
    match value.parse::<u64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!("{name}={value:?}: expected a whole number of at least 1").into()),
    }
}

/// Loads the voice and the fixed lines, and checks the LLM answers, before any call.
pub async fn start_services<L: Llm>(stt: Stt, llm: L, llm_name: &str) -> Result<Services<L>, Error> {
    let voice_model = env("PIPER_VOICE").unwrap_or_else(|| DEFAULT_VOICE.to_string());
    let voice = Voice::load(&voice_model).map_err(|e| format!("{voice_model}: {e}"))?;
    let tts = Tts::start(voice);
    let started = Instant::now();
    let lines = Lines::synthesise(&tts).await?;
    eprintln!(
        "agent: speaking with {voice_model} (fixed lines in {} ms)",
        started.elapsed().as_millis()
    );

    // Also loads the model into memory, so the first call's first turn doesn't wait for it.
    let probe = [
        Message::new(Role::System, SYSTEM_PROMPT),
        Message::new(Role::User, "I'm fine, thank you."),
    ];
    let reply = llm.turn(&probe).await.map_err(|e| format!("{llm_name}: {e}"))?;
    eprintln!("agent: thinking with {llm_name} (warm-up turn in {} ms)", reply.took.as_millis());

    // A transfer only ever goes to an internal extension (the safety rules), checked before
    // any call rather than at the moment of an emergency.
    let dispatcher = env("DISPATCHER_EXTENSION").unwrap_or_else(|| DEFAULT_DISPATCHER.to_string());
    check_extension(&dispatcher).map_err(|e| format!("DISPATCHER_EXTENSION: {e}"))?;
    eprintln!("agent: escalations transfer to extension {dispatcher}");
    let calls_dir = PathBuf::from(env("CALLS_DIR").unwrap_or_else(|| DEFAULT_CALLS_DIR.into()));

    // Turn-taking (issue #19). Both are on unless `.env` turns them off, as a fallback for a
    // line whose echo pauses the agent on its own voice.
    let barge_in = !switched_off("BARGE_IN");
    let end_of_turn = if switched_off("SMART_TURN") {
        None
    } else {
        let model = env("SMART_TURN_MODEL").unwrap_or_else(|| DEFAULT_SMART_TURN_MODEL.to_string());
        let smart_turn = SmartTurn::new(&model).map_err(|e| format!("{model}: {e}"))?;
        eprintln!("agent: Smart Turn from {model} can hold the floor at a pause");
        Some(EndOfTurn::start(smart_turn))
    };
    eprintln!(
        "agent: barge-in {}, Smart Turn {}",
        if barge_in { "on" } else { "off (BARGE_IN)" },
        if end_of_turn.is_some() { "on" } else { "off (SMART_TURN)" }
    );

    // Off unless `.env` turns it on: a recording holds a real voice, the maintainer's on a test
    // call, and stays in the gitignored calls/ directory.
    let record = switched_on("RECORD_CALLS");
    if record {
        eprintln!("agent: recording each call to {}/<call>.wav", calls_dir.display());
    }

    Ok(Services {
        stt,
        tts,
        llm: Arc::new(llm),
        lines: Arc::new(lines),
        dispatcher,
        calls_dir,
        barge_in,
        end_of_turn,
        record,
    })
}

/// Answers one AudioSocket connection with a check-in: the resident calling 3100, or answering
/// a call the agent placed, which `outbound` knows by its UUID. Returns once the call has ended
/// and its log is written. `pbx` gives the call's Call control its PBX side once the UUID is
/// known: AMI on a live call, a recorder in the eval.
pub async fn answer<L: Llm + 'static>(
    stream: TcpStream,
    vad_model: &str,
    services: Services<L>,
    outbound: &Outbound,
    pbx: impl FnOnce(CallControl, &Uuid) -> CallControl,
) -> Result<LineStats, Error> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;
    // Each call gets its own VAD: Silero carries state from one window to the next.
    let vad = Silero::new(vad_model)?;
    let (uuid, media, control, line) = line::accept(stream, services.record).await?;
    let outbound_to = outbound.take(&uuid.to_string());
    match &outbound_to {
        Some(resident) => eprintln!("agent: call started, UUID {uuid}, placed to {resident}"),
        None => eprintln!("agent: call started, UUID {uuid}"),
    }
    let control = pbx(control, &uuid);

    let started = Instant::now();
    let calls_dir = services.calls_dir.clone();
    check_in(uuid.to_string(), outbound_to, media, control, vad, services).await;
    let stats = line.await??;
    log_stats(&uuid, started.elapsed(), &stats);
    if let Some(tape) = &stats.tape {
        let path = calls_dir.join(format!("{uuid}.wav"));
        match tape.write(&path) {
            Ok(()) => eprintln!("agent: call recorded to {}", path.display()),
            Err(e) => eprintln!("agent: couldn't write the recording {}: {e}", path.display()),
        }
    }
    Ok(stats)
}

/// The line's counts for one call, as one log line.
pub fn log_stats(uuid: &Uuid, took: Duration, stats: &LineStats) {
    let gap = |g: Option<Duration>| g.map_or("n/a".into(), |g| format!("{g:.1?}"));
    eprintln!(
        "agent: call ended, UUID {uuid}, {:.1} s: {} frames in ({} filled with silence), {} out \
         (gaps between writes {} to {}), {} dropped",
        took.as_secs_f64(),
        stats.frames_in,
        stats.frames_filled,
        stats.frames_out,
        gap(stats.min_write_gap),
        gap(stats.max_write_gap),
        stats.frames_dropped + stats.dropped_messages,
    );
}
