//! PROTOTYPE, throwaway: speaks one resident line in a Piper voice as an 8 kHz WAV for
//! `fake_call`, with silence before (the agent doesn't listen while its greeting plays) and
//! after (so the agent's answer has time to happen before the fake call hangs up).
//!
//! cargo run --release -p agent --example PROTOTYPE_say -- <voice.onnx> <out.wav> <before s> <after s> <text>

use std::path::Path;

use agent::audio::{LINE_RATE_HZ, resample_clip};
use piper_rs::Piper;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (voice, out) = (&args[1], &args[2]);
    let before: f64 = args[3].parse().unwrap();
    let after: f64 = args[4].parse().unwrap();
    let text = args[5..].join(" ");
    let mut piper = Piper::new(Path::new(voice), Path::new(&format!("{voice}.json"))).unwrap();
    let (samples, rate) = piper.create(&text, false, None, None, None, None).unwrap();
    let samples: Vec<i16> =
        samples.iter().map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).collect();
    let speech = resample_clip(&samples, rate, LINE_RATE_HZ);
    let silence = |s: f64| vec![0i16; (s * LINE_RATE_HZ as f64) as usize];
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: LINE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec).unwrap();
    for s in silence(before).iter().chain(&speech).chain(&silence(after)) {
        writer.write_sample(*s).unwrap();
    }
    writer.finalize().unwrap();
}
