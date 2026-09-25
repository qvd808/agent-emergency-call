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

use std::path::Path;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, resample_clip};
use agent::call::{
    DEFAULT_VAD_MODEL, DEFAULT_WHISPER_MODEL, answer, env, log_stats, ollama_from_env,
    start_services,
};
use agent::conversation::Services;
use agent::listen::listen;
use agent::llm::Ollama;
use agent::stt::{Stt, Whisper};
use agent::telephony::ami::Ami;
use agent::telephony::{core_duration, line};
use tokio::net::{TcpListener, TcpStream};
use turn::vad::Silero;

/// Must match the port in `asterisk/config/extensions.conf`, extension 3100.
const DEFAULT_LISTEN: &str = "127.0.0.1:9092";

const DEFAULT_AMI_PORT: &str = "5038";

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
        None => {
            let (llm, name) = ollama_from_env();
            Some(start_services(stt.clone(), llm, &name).await?)
        }
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

async fn call(
    stream: TcpStream,
    clip: Option<Vec<i16>>,
    stt: Stt,
    services: Option<Services<Ollama>>,
    vad_model: &str,
    ami: Option<Ami>,
) -> Result<(), Error> {
    if let Some(services) = services {
        answer(stream, vad_model, services, |control, uuid| match ami {
            Some(ami) => control.with_ami(ami, uuid.to_string()),
            None => control,
        })
        .await?;
        return Ok(());
    }

    // The audio harness: play the clip, then transcribe whatever the resident says.
    stream.set_nodelay(true)?;
    let vad = Silero::new(vad_model)?;
    let (uuid, media, _control, line) = line::accept(stream).await?;
    eprintln!("agent: call started, UUID {uuid}");
    let started = Instant::now();
    if let Some(clip) = clip {
        media.speaker.play(clip);
    }
    // Keeps the speaker alive, so the line goes on sending silence, until the call ends.
    let _speaker = media.speaker;
    listen(uuid.to_string(), media.frames, vad, stt).await;
    let stats = line.await??;
    log_stats(&uuid, started.elapsed(), &stats);
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
