//! One call to extension 3100, from the moment Asterisk connects until the call ends, and the
//! services every call shares. The live agent (`main.rs`) and the eval (issue #23) both answer
//! calls through [`answer`], so the eval runs the path a real call takes.
//!
//! Settings come from the environment; `main.rs` reads `.env` into it first.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use turn::vad::Silero;

use crate::conversation::{Lines, Services, check_in};
use crate::llm::{Llm, Message, Ollama, Role, SYSTEM_PROMPT};
use crate::stt::Stt;
use crate::telephony::CallControl;
use crate::telephony::ami::check_extension;
use crate::telephony::audiosocket::Uuid;
use crate::telephony::line::{self, LineStats};
use crate::tts::{Tts, Voice};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Where `make models` puts them. `tiny.en` is the fastest of `tiny.en`, `base.en` and
/// `small.en` on this laptop's CPU, and the only one inside the 1 s budget (issue #17).
pub const DEFAULT_WHISPER_MODEL: &str = "models/ggml-tiny.en.bin";
pub const DEFAULT_VAD_MODEL: &str = "models/silero_vad.onnx";
pub const DEFAULT_VOICE: &str = "models/en_US-hfc_female-medium.onnx";

/// The same defaults as `.env.example`.
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3:4b-instruct-2507-q4_K_M";
const DEFAULT_DISPATCHER: &str = "2000";
const DEFAULT_CALLS_DIR: &str = "calls";

/// A setting from the environment. Empty counts as unset.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The LLM the environment names: Ollama at `OLLAMA_URL`, running `OLLAMA_MODEL`. Returns it
/// with a name for the log.
pub fn ollama_from_env() -> (Ollama, String) {
    let url = env("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());
    let model = env("OLLAMA_MODEL").unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());
    (Ollama::new(&url, &model), format!("{model} at {url}"))
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

    Ok(Services {
        stt,
        tts,
        llm: Arc::new(llm),
        lines: Arc::new(lines),
        dispatcher,
        calls_dir,
    })
}

/// Answers one AudioSocket connection with a check-in. Returns once the call has ended and its
/// log is written. `pbx` gives the call's Call control its PBX side once the UUID is known:
/// AMI on a live call, a recorder in the eval.
pub async fn answer<L: Llm + 'static>(
    stream: TcpStream,
    vad_model: &str,
    services: Services<L>,
    pbx: impl FnOnce(CallControl, &Uuid) -> CallControl,
) -> Result<LineStats, Error> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;
    // Each call gets its own VAD: Silero carries state from one window to the next.
    let vad = Silero::new(vad_model)?;
    let (uuid, media, control, line) = line::accept(stream).await?;
    eprintln!("agent: call started, UUID {uuid}");
    let control = pbx(control, &uuid);

    let started = Instant::now();
    check_in(uuid.to_string(), media, control, vad, services).await;
    let stats = line.await??;
    log_stats(&uuid, started.elapsed(), &stats);
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
