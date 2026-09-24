//! For now, an AudioSocket echo server (issue #15): every audio frame a caller sends is written
//! straight back, so dialling 3100 plays the caller's own voice back to them. It proves the
//! path from the phone through Asterisk to this process and back.
//!
//! Asterisk connects to us: the dialplan's `AudioSocket()` dials `host.docker.internal:9092`,
//! which reaches this process on `127.0.0.1` (issue #27). Only loopback, so nothing on the
//! Wi-Fi can reach the agent.

use agent::telephony::audiosocket::{Decoder, Message, encode_audio};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Must match the port in `asterisk/config/extensions.conf`, extension 3100.
const DEFAULT_LISTEN: &str = "127.0.0.1:9092";

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listen = std::env::var("AUDIOSOCKET_LISTEN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let listener = TcpListener::bind(&listen).await?;
    eprintln!("agent: listening for AudioSocket on {listen}");

    loop {
        let (stream, peer) = listener.accept().await?;
        tokio::spawn(async move {
            match echo(stream).await {
                Ok(summary) => eprintln!("agent: {peer}: call ended, {summary}"),
                Err(e) => eprintln!("agent: {peer}: call failed: {e}"),
            }
        });
    }
}

/// Echoes one call until Asterisk closes the socket. Returns a one-line summary for the log.
async fn echo(mut stream: TcpStream) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Asterisk sets TCP_NODELAY on its side (issue #4). Setting it here too sends each
    // 323-byte frame at once instead of letting the kernel hold it back to batch it.
    stream.set_nodelay(true)?;

    let mut decoder = Decoder::new();
    let mut buf = [0u8; 4096];
    let mut uuid = None;
    let mut frames = 0u64;

    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            decoder.finish()?;
            let uuid = uuid.map_or("no UUID".to_string(), |u| format!("UUID {u}"));
            return Ok(format!("{uuid}, {frames} frames echoed"));
        }
        decoder.push(&buf[..n]);

        while let Some(message) = decoder.next_message()? {
            match message {
                Message::Uuid(u) => {
                    eprintln!("agent: call started, UUID {u}");
                    uuid = Some(u);
                }
                Message::Audio { rate_hz, pcm } => {
                    // One write per message: Asterisk wants the payload within 5 ms of the
                    // header (issue #4). Incoming frames arrive every 20 ms, so echoing each
                    // as it arrives is already paced in real time.
                    stream.write_all(&encode_audio(rate_hz, &pcm)?).await?;
                    frames += 1;
                }
                Message::Dtmf(digit) => eprintln!("agent: DTMF {}", digit as char),
                other => eprintln!("agent: ignored {other:?}"),
            }
        }
    }
}
