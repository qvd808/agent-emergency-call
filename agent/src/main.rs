//! For now, a harness for hearing the resident (issue #17). On each call it transcribes what
//! the resident says and logs each utterance with how soon its text was ready. With
//! `TEST_WAV` set, it first plays that WAV into the call at real-time pace, as issue #16 did.
//! The audio goes through the same path the agent will use: 8 kHz on the line, 16 kHz inside.
//!
//! Asterisk connects to us: the dialplan's `AudioSocket()` dials `host.docker.internal:9092`,
//! which reaches this process on `127.0.0.1` (issue #27). Only loopback, so nothing on the
//! Wi-Fi can reach the agent.

use std::path::Path;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, resample_clip};
use agent::listen::listen;
use agent::stt::{Stt, Whisper};
use agent::telephony::{core_duration, line};
use turn::vad::Silero;
use tokio::net::{TcpListener, TcpStream};

/// Must match the port in `asterisk/config/extensions.conf`, extension 3100.
const DEFAULT_LISTEN: &str = "127.0.0.1:9092";

/// Where `make models` puts them. `tiny.en` is the fastest of `tiny.en`, `base.en` and
/// `small.en` on this laptop's CPU, and the only one inside the 1 s budget (issue #17).
const DEFAULT_WHISPER_MODEL: &str = "models/ggml-tiny.en.bin";
const DEFAULT_VAD_MODEL: &str = "models/silero_vad.onnx";

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let listen = env("AUDIOSOCKET_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let clip = match env("TEST_WAV") {
        Some(path) => {
            let clip = load_wav(Path::new(&path))?;
            let secs = core_duration(clip.len()).as_secs_f64();
            eprintln!("agent: each call first plays {path} ({secs:.2} s)");
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
    let listener = TcpListener::bind(&listen).await?;
    eprintln!("agent: listening for AudioSocket on {listen}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let (clip, stt, vad_model) = (clip.clone(), stt.clone(), vad_model.clone());
        tokio::spawn(async move {
            if let Err(e) = call(stream, clip, stt, &vad_model).await {
                eprintln!("agent: {peer}: call failed: {e}");
            }
        });
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

async fn call(
    stream: TcpStream,
    clip: Option<Vec<i16>>,
    stt: Stt,
    vad_model: &str,
) -> Result<(), Error> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;
    // Each call gets its own VAD: Silero carries state from one window to the next.
    let vad = Silero::new(vad_model)?;
    let (uuid, media, line) = line::accept(stream).await?;
    eprintln!("agent: call started, UUID {uuid}");

    let started = Instant::now();
    if let Some(clip) = clip {
        media.speaker.play(clip);
    }
    // Keeps the speaker alive, so the line goes on sending silence, until the call ends.
    let _speaker = media.speaker;
    listen(uuid.to_string(), media.frames, vad, stt).await;

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
