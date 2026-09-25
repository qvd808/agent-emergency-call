//! The check-in agent (issue #18). Each call to extension 3100 gets the spoken check-in in
//! `conversation.rs`: speech to text, the LLM's turn, and Piper's voice back down the line.
//!
//! With `TEST_WAV` set it is instead the audio harness from issues #16 and #17: each call
//! plays that WAV at real-time pace, then transcribes and logs what the resident says.
//!
//! Asterisk connects to us: the dialplan's `AudioSocket()` dials `host.docker.internal:9092`,
//! which reaches this process on `127.0.0.1` (issue #27). Only loopback, so nothing on the
//! Wi-Fi can reach the agent.
//!
//! Settings come from the environment, and from `.env` for anything the environment doesn't set.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, resample_clip};
use agent::conversation::{Lines, Services, check_in};
use agent::listen::listen;
use agent::llm::{Llm, Message, Ollama, Role, SYSTEM_PROMPT};
use agent::stt::{Stt, Whisper};
use agent::telephony::ami::{Ami, check_extension};
use agent::telephony::{core_duration, line};
use agent::tts::{Tts, Voice};
use tokio::net::{TcpListener, TcpStream};
use turn::vad::Silero;

/// Must match the port in `asterisk/config/extensions.conf`, extension 3100.
const DEFAULT_LISTEN: &str = "127.0.0.1:9092";

/// Where `make models` puts them. `tiny.en` is the fastest of `tiny.en`, `base.en` and
/// `small.en` on this laptop's CPU, and the only one inside the 1 s budget (issue #17).
const DEFAULT_WHISPER_MODEL: &str = "models/ggml-tiny.en.bin";
const DEFAULT_VAD_MODEL: &str = "models/silero_vad.onnx";
const DEFAULT_VOICE: &str = "models/en_US-hfc_female-medium.onnx";

/// The same defaults as `.env.example`.
const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen3:4b-instruct-2507-q4_K_M";
const DEFAULT_AMI_PORT: &str = "5038";
const DEFAULT_DISPATCHER: &str = "2000";
const DEFAULT_CALLS_DIR: &str = "calls";

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    if dotenvy::dotenv().is_ok() {
        eprintln!("agent: read settings from .env");
    }
    let listen = env("AUDIOSOCKET_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let clip = match env("TEST_WAV") {
        Some(path) => {
            let clip = load_wav(Path::new(&path))?;
            let secs = core_duration(clip.len()).as_secs_f64();
            eprintln!("agent: each call first plays {path} ({secs:.2} s), then listens");
            Some(clip)
        }
        None => None,
    };
    let whisper_model = env("WHISPER_MODEL").unwrap_or_else(|| DEFAULT_WHISPER_MODEL.to_string());
    let vad_model = env("VAD_MODEL").unwrap_or_else(|| DEFAULT_VAD_MODEL.to_string());
    let whisper = Whisper::load(&whisper_model).map_err(|e| format!("{whisper_model}: {e}"))?;
    // Fails now rather than on the first call if the VAD model is missing.
    Silero::new(&vad_model).map_err(|e| format!("{vad_model}: {e}"))?;
    let stt = Stt::start(whisper);
    eprintln!("agent: transcribing with {whisper_model}");

    let ami = ami_from_env().await;
    let services = match clip {
        Some(_) => None,
        None => Some(start_services(stt.clone()).await?),
    };

    let listener = TcpListener::bind(&listen).await?;
    eprintln!("agent: listening for AudioSocket on {listen}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let (clip, stt, services, vad_model, ami) =
            (clip.clone(), stt.clone(), services.clone(), vad_model.clone(), ami.clone());
        tokio::spawn(async move {
            if let Err(e) = call(stream, clip, stt, services, &vad_model, ami).await {
                eprintln!("agent: {peer}: call failed: {e}");
            }
        });
    }
}

/// Loads the voice and the fixed lines, and checks the LLM answers, before any call.
async fn start_services(stt: Stt) -> Result<Services<Ollama>, Error> {
    let voice_model = env("PIPER_VOICE").unwrap_or_else(|| DEFAULT_VOICE.to_string());
    let voice = Voice::load(&voice_model).map_err(|e| format!("{voice_model}: {e}"))?;
    let tts = Tts::start(voice);
    let started = Instant::now();
    let lines = Lines::synthesise(&tts).await?;
    eprintln!(
        "agent: speaking with {voice_model} (fixed lines in {} ms)",
        started.elapsed().as_millis()
    );

    let url = env("OLLAMA_URL").unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_string());
    let model = env("OLLAMA_MODEL").unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string());
    let llm = Ollama::new(&url, &model);
    // Also loads the model into memory, so the first call's first turn doesn't wait for it.
    let probe = [
        Message::new(Role::System, SYSTEM_PROMPT),
        Message::new(Role::User, "I'm fine, thank you."),
    ];
    let reply = llm.turn(&probe).await.map_err(|e| format!("{model} at {url}: {e}"))?;
    eprintln!("agent: thinking with {model} (warm-up turn in {} ms)", reply.took.as_millis());

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

/// AMI, if `.env` configures it, after checking it logs in. Without it the check-in still runs,
/// but an escalation can only speak its script: the transfer fails.
async fn ami_from_env() -> Option<Ami> {
    let (Some(host), Some(username), Some(secret)) =
        (env("AMI_HOST"), env("AMI_USERNAME"), env("AMI_SECRET"))
    else {
        eprintln!(
            "agent: WARNING: AMI is not configured (AMI_HOST, AMI_USERNAME, AMI_SECRET in .env), \
             so escalations cannot transfer calls. `make asterisk-up` writes them."
        );
        return None;
    };
    let port = env("AMI_PORT").unwrap_or_else(|| DEFAULT_AMI_PORT.to_string());
    let ami = Ami { addr: format!("{host}:{port}"), username, secret };
    match ami.check().await {
        Ok(()) => eprintln!("agent: AMI logged in at {}", ami.addr),
        Err(e) => eprintln!(
            "agent: WARNING: AMI at {} failed ({e}), so escalations cannot transfer calls \
             until it works. Is Asterisk up (`make asterisk-up`)?",
            ami.addr
        ),
    }
    Some(ami)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

async fn call(
    stream: TcpStream,
    clip: Option<Vec<i16>>,
    stt: Stt,
    services: Option<Services<Ollama>>,
    vad_model: &str,
    ami: Option<Ami>,
) -> Result<(), Error> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;
    // Each call gets its own VAD: Silero carries state from one window to the next.
    let vad = Silero::new(vad_model)?;
    let (uuid, media, control, line) = line::accept(stream).await?;
    eprintln!("agent: call started, UUID {uuid}");
    let control = match ami {
        Some(ami) => control.with_ami(ami, uuid.to_string()),
        None => control,
    };

    let started = Instant::now();
    match (clip, services) {
        (_, Some(services)) => check_in(uuid.to_string(), media, control, vad, services).await,
        (clip, None) => {
            if let Some(clip) = clip {
                media.speaker.play(clip);
            }
            // Keeps the speaker alive, so the line goes on sending silence, until the call ends.
            let _speaker = media.speaker;
            listen(uuid.to_string(), media.frames, vad, stt).await;
        }
    }

    let stats = line.await??;
    let gap = |g: Option<std::time::Duration>| g.map_or("n/a".into(), |g| format!("{g:.1?}"));
    eprintln!(
        "agent: call ended, UUID {uuid}, {:.1} s: {} frames in ({} filled with silence), {} out \
         (gaps between writes {} to {}), {} dropped",
        started.elapsed().as_secs_f64(),
        stats.frames_in,
        stats.frames_filled,
        stats.frames_out,
        gap(stats.min_write_gap),
        gap(stats.max_write_gap),
        stats.frames_dropped + stats.dropped_messages,
    );
    Ok(())
}

/// Reads a 16-bit WAV of any rate, keeps the first channel, and converts it to 16 kHz.
fn load_wav(path: &Path) -> Result<Vec<i16>, Error> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(format!("{}: need 16-bit PCM, got {spec:?}", path.display()).into());
    }
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .step_by(spec.channels as usize)
        .collect::<Result<_, _>>()?;
    Ok(resample_clip(&samples, spec.sample_rate, CORE_RATE_HZ))
}
