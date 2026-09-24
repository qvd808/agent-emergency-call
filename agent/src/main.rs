//! For now, a test harness for the Media half of the telephony seam (issue #16). On each call
//! it plays the WAV named by `TEST_WAV` into the call at real-time pace, then echoes the
//! caller's voice back. Without `TEST_WAV` it only echoes, as issue #15 did. Both directions
//! go through the same path the agent will use: 8 kHz on the line, 16 kHz inside.
//!
//! Asterisk connects to us: the dialplan's `AudioSocket()` dials `host.docker.internal:9092`,
//! which reaches this process on `127.0.0.1` (issue #27). Only loopback, so nothing on the
//! Wi-Fi can reach the agent.

use std::path::Path;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, resample_clip};
use agent::telephony::{core_duration, line};
use tokio::net::{TcpListener, TcpStream};

/// Must match the port in `asterisk/config/extensions.conf`, extension 3100.
const DEFAULT_LISTEN: &str = "127.0.0.1:9092";

/// The mark placed after the test WAV.
const END_OF_CLIP: u64 = 1;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let listen = env("AUDIOSOCKET_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let clip = match env("TEST_WAV") {
        Some(path) => {
            let clip = load_wav(Path::new(&path))?;
            let secs = core_duration(clip.len()).as_secs_f64();
            eprintln!("agent: each call plays {path} ({secs:.2} s), then echoes");
            Some(clip)
        }
        None => {
            eprintln!("agent: TEST_WAV not set; each call only echoes");
            None
        }
    };
    let listener = TcpListener::bind(&listen).await?;
    eprintln!("agent: listening for AudioSocket on {listen}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let clip = clip.clone();
        tokio::spawn(async move {
            if let Err(e) = call(stream, clip).await {
                eprintln!("agent: {peer}: call failed: {e}");
            }
        });
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

async fn call(stream: TcpStream, clip: Option<Vec<i16>>) -> Result<(), Error> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;
    let (uuid, mut media, line) = line::accept(stream).await?;
    eprintln!("agent: call started, UUID {uuid}");

    let started = Instant::now();
    let mut echoing = true;
    if let Some(clip) = clip {
        media.speaker.play(clip);
        media.speaker.mark(END_OF_CLIP);
        echoing = false;
    }

    loop {
        tokio::select! {
            frame = media.frames.recv() => match frame {
                // Echo only once the clip is over; before that, the echo would queue up
                // behind the clip and come back late.
                Some(frame) if echoing => media.speaker.play(frame.to_vec()),
                Some(_) => {}
                None => break,
            },
            Some(END_OF_CLIP) = media.played.recv() => {
                eprintln!(
                    "agent: test WAV written to the line {:.3} s after the call started; echoing",
                    started.elapsed().as_secs_f64()
                );
                echoing = true;
            }
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
