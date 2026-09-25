//! PROTOTYPE, throwaway: speaks the same check-in lines in several Piper voices and settings,
//! and writes each as the 8 kHz audio a softphone hears, to pick the agent's voice by ear.
//!
//! cargo run --release -p agent --example PROTOTYPE_voice_samples -- <out dir> <voice.onnx>...

use std::path::Path;

use agent::audio::{CORE_RATE_HZ, LINE_RATE_HZ, resample_clip};
use piper_rs::Piper;

const LINES: &[&str] = &[
    "Hello, this is the automated check-in assistant. How are you feeling today?",
    "Oh, that's good to hear. Have you had any falls lately?",
    "Oh no, I'm sorry to hear that. Were you able to get up by yourself?",
    "I'm connecting you to a person now. Please stay on the line. If you are in danger, call nine one one yourself as soon as you can.",
];

/// (name, length_scale, noise_scale, noise_w). `None` keeps the voice's own default.
const SETTINGS: &[(&str, Option<f32>, Option<f32>, Option<f32>)] = &[
    ("default", None, None, None),
    ("warm", Some(1.1), Some(0.8), Some(0.9)),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = Path::new(&args[1]);
    std::fs::create_dir_all(out).unwrap();
    for model in &args[2..] {
        let mut piper = Piper::new(Path::new(model), Path::new(&format!("{model}.json"))).unwrap();
        let voice = Path::new(model).file_stem().unwrap().to_str().unwrap().replace("en_US-", "");
        for (name, length, noise, noise_w) in SETTINGS {
            let mut line_audio = Vec::new();
            let mut took = 0.0;
            for text in LINES {
                let started = std::time::Instant::now();
                let (samples, rate) = piper.create(text, false, None, *length, *noise, *noise_w).unwrap();
                took += started.elapsed().as_secs_f64();
                let samples: Vec<i16> = samples
                    .iter()
                    .map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
                    .collect();
                // The agent's path: Piper's rate to 16 kHz, then down to the line's 8 kHz.
                let core = resample_clip(&samples, rate, CORE_RATE_HZ);
                line_audio.extend(resample_clip(&core, CORE_RATE_HZ, LINE_RATE_HZ));
                line_audio.extend(std::iter::repeat_n(0i16, LINE_RATE_HZ as usize * 7 / 10));
            }
            let path = out.join(format!("{voice}--{name}.wav"));
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: LINE_RATE_HZ,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for s in &line_audio {
                writer.write_sample(*s).unwrap();
            }
            writer.finalize().unwrap();
            println!("{} ({:.0} ms to synthesise four lines)", path.display(), took * 1000.0);
        }
    }
}
