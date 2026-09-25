//! PROTOTYPE, throwaway: speaks check-in lines once per tone, then writes each line as Piper
//! spoke it and after pitch reshaping, as the 8 kHz audio a softphone hears. Every shape is
//! applied to the same Piper audio, so the files compare like with like. Measure them with
//! `PROTOTYPE_prosody_measure.py`.
//!
//! PIPER_ESPEAKNG_DATA_DIRECTORY=models cargo run --release -p agent \
//!     --example PROTOTYPE_prosody_samples -- <out dir> models/en_US-hfc_female-medium.onnx

use std::path::Path;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, LINE_RATE_HZ, resample_clip};
use agent::prosody::{Shape, reshape};
use piper_rs::Piper;

const LINES: &[(&str, &str)] = &[
    ("greeting", "Hello. This is the automated check-in assistant. How are you feeling today?"),
    ("good", "Oh, that's good to hear. Have you had any falls lately?"),
    ("good-excl", "Oh, that's good to hear! Have you had any falls lately?"),
    ("sorry", "Oh no, I'm sorry to hear that. Were you able to get up by yourself?"),
    ("goodbye", "Thank you for talking with me today. Take care, and goodbye!"),
    ("escalate", "I'm connecting you to a person now. Please stay on the line. If you are in danger, call nine one one yourself as soon as you can."),
];

/// Piper's (length_scale, noise_scale, noise_w) per tone, as in `tts.rs`, and the shapes to
/// compare. `x-nochange` runs the full reshaping path asking for no change, to check it gives
/// Piper's audio back.
fn groups() -> Vec<(&'static str, (f32, f32, f32), Vec<(&'static str, Shape)>)> {
    let s = |shift_semitones, range| Shape { shift_semitones, range };
    vec![
        ("warm", (1.1, 0.8, 0.9), vec![
            ("0-as-piper", Shape::NONE),
            ("1-r1.3-up1", s(1.0, 1.3)),
            ("2-r1.5-up1-chosen", s(1.0, 1.5)),
            ("3-r1.8-up1.5", s(1.5, 1.8)),
            ("x-nochange", s(0.0, 1.0001)),
        ]),
        ("steady", (1.2, 0.6, 0.7), vec![
            ("0-as-piper", Shape::NONE),
            ("1-r1.1-down0.5-chosen", s(-0.5, 1.1)),
        ]),
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = Path::new(&args[1]);
    std::fs::create_dir_all(out).unwrap();
    let model = &args[2];
    let mut piper = Piper::new(Path::new(model), Path::new(&format!("{model}.json"))).unwrap();
    for (tone, (length, noise, noise_w), shapes) in groups() {
        let mut all: Vec<Vec<i16>> = vec![Vec::new(); shapes.len()];
        let mut took = vec![0.0; shapes.len()];
        let mut speech_s = 0.0;
        for (line, text) in LINES {
            let (samples, rate) =
                piper.create(text, false, None, Some(length), Some(noise), Some(noise_w)).unwrap();
            let samples: Vec<i16> = samples
                .iter()
                .map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
                .collect();
            let core = resample_clip(&samples, rate, CORE_RATE_HZ);
            speech_s += core.len() as f64 / CORE_RATE_HZ as f64;
            for (i, (name, shape)) in shapes.iter().enumerate() {
                let started = Instant::now();
                let shaped = reshape(&core, CORE_RATE_HZ, *shape);
                took[i] += started.elapsed().as_secs_f64() * 1000.0;
                if name.starts_with('x') {
                    let error: f64 =
                        core.iter().zip(&shaped).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum();
                    let energy: f64 = core.iter().map(|&a| (a as f64).powi(2)).sum();
                    println!("{tone} {line}: no-change SNR {:.1} dB", 10.0 * (energy / error).log10());
                }
                let line_audio = resample_clip(&shaped, CORE_RATE_HZ, LINE_RATE_HZ);
                write(&out.join(format!("{tone}-{name}--{line}.wav")), &line_audio);
                all[i].extend(&line_audio);
                all[i].extend(std::iter::repeat_n(0i16, LINE_RATE_HZ as usize * 7 / 10));
            }
        }
        for (i, (name, _)) in shapes.iter().enumerate() {
            write(&out.join(format!("{tone}-{name}.wav")), &all[i]);
            println!("{tone}-{name}: reshaping took {:.1} ms for {speech_s:.1} s of speech", took[i]);
        }
    }
}

fn write(path: &Path, samples: &[i16]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: LINE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for s in samples {
        writer.write_sample(*s).unwrap();
    }
    writer.finalize().unwrap();
}
