//! Stands in for Asterisk on one call (issue #17): connects to the agent, sends a UUID, then
//! streams an 8 kHz WAV as audio messages, one 20 ms frame every 20 ms, and hangs up after a
//! few seconds of trailing silence. What the agent sends back is read and discarded.
//!
//! cargo run --release -p agent --example fake_call -- <8 kHz WAV> [127.0.0.1:9092]

use std::time::Duration;

use agent::audio::{LINE_FRAME, LINE_RATE_HZ, samples_to_le_bytes};
use agent::telephony::audiosocket::{Uuid, encode_audio, encode_hangup, encode_uuid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Silence after the WAV, so the agent's last utterance closes and is transcribed.
const TAIL: Duration = Duration::from_secs(3);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let wav = args.get(1).ok_or("usage: fake_call <8 kHz WAV> [addr]")?;
    let addr = args.get(2).map_or("127.0.0.1:9092", String::as_str);
    let mut reader = hound::WavReader::open(wav)?;
    if reader.spec().sample_rate != LINE_RATE_HZ || reader.spec().channels != 1 {
        return Err(format!("{wav}: need 8 kHz mono, got {:?}", reader.spec()).into());
    }
    let mut samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
    samples.resize(samples.len() + (TAIL.as_millis() as usize / 20) * LINE_FRAME, 0);

    let stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    let (mut from_agent, mut to_agent) = stream.into_split();
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        while matches!(from_agent.read(&mut buf).await, Ok(n) if n > 0) {}
    });
    to_agent.write_all(&encode_uuid(&Uuid(rand_uuid()))).await?;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for frame in samples.chunks(LINE_FRAME) {
        tick.tick().await;
        to_agent.write_all(&encode_audio(LINE_RATE_HZ, &samples_to_le_bytes(frame))?).await?;
    }
    to_agent.write_all(&encode_hangup()).await?;
    Ok(())
}

/// Not a real random UUID: distinct per run is all the agent's log needs.
fn rand_uuid() -> [u8; 16] {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    nanos.as_nanos().to_be_bytes()
}
