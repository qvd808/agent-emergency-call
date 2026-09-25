//! PROTOTYPE, throwaway: speaks each candidate acknowledgement several times, exactly as the
//! agent speaks its fixed lines (`Voice::speak` in `Tone::Warm`, conversation.rs), and writes
//! every take as the 8 kHz audio a softphone hears. It first prints the phonemes espeak gives
//! each one, which is how "Mm-hm." was found to be spelled out as letters. Transcribe the
//! takes with `PROTOTYPE_acks_transcribe.py` to count how many come out as the word.
//!
//! It also tries "mm-hm" written as phonemes, skipping espeak, to see whether the voice can
//! say it at all, and the two best words under other settings. Those go through Piper and the
//! pitch reshaping directly, as `Voice::speak` would (issue #45). The run behind the choice of
//! acknowledgements is in `PROTOTYPE_acks.txt`.
//!
//! PIPER_ESPEAKNG_DATA_DIRECTORY=models cargo run --release -p agent \
//!     --example PROTOTYPE_acks -- <out dir> models/en_US-hfc_female-medium.onnx <takes>

use std::path::Path;

use agent::audio::{CORE_RATE_HZ, LINE_RATE_HZ, resample_clip};
use agent::prosody::{Shape, reshape};
use agent::tts::{Tone, Voice};
use piper_rs::Piper;

const CANDIDATES: &[(&str, &str)] = &[
    ("mm-hm", "Mm-hm."),
    ("i-see", "I see."),
    ("okay", "Okay."),
    ("right", "Right."),
    ("got-it", "Got it."),
    ("uh-huh", "Uh-huh."),
    ("alright", "Alright."),
];

/// "mm-hm" as phonemes: syllabic m, a long m, and with a schwa or a strut vowel before each m.
const PHONEMES: &[(&str, &str)] = &[
    ("ph-syllabic", "m̩hˈm̩."),
    ("ph-long", "mːhˈmː."),
    ("ph-schwa", "əmhˈəm."),
    ("ph-strut", "ʌmhˈʌm."),
];

/// Warm's (length_scale, noise_scale, noise_w) and shape, copied from `tts.rs`.
const WARM: (f32, f32, f32) = (1.1, 0.8, 0.9);
const WARM_SHAPE: Shape = Shape { shift_semitones: 1.0, range: 1.5 };

/// Other ways to speak the two words that came out best, to see what makes a bad take:
/// Warm without its pitch reshaping, Steady as `tts.rs` has it, and the voice's own defaults
/// (`en_US-hfc_female-medium.onnx.json`) with no reshaping.
const SETTINGS: &[(&str, (f32, f32, f32), Shape)] = &[
    ("warm-flat", WARM, Shape::NONE),
    ("steady", (1.2, 0.6, 0.7), Shape { shift_semitones: -0.5, range: 1.1 }),
    ("default", (1.0, 0.667, 0.8), Shape::NONE),
];
const SETTINGS_WORDS: &[(&str, &str)] = &[("i-see", "aɪ sˈiː."), ("okay", "oʊkˈeɪ.")];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (out, model) = (&args[1], &args[2]);
    let takes: usize = args[3].parse().unwrap();
    std::fs::create_dir_all(out).unwrap();
    for (_, text) in CANDIDATES {
        let phonemes = espeak_rs::text_to_phonemes(text, "en-us", None).unwrap().join(" ");
        println!("{text:10} -> {phonemes}");
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: LINE_RATE_HZ,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let write = |path: String, core: &[i16]| {
        let line = resample_clip(core, CORE_RATE_HZ, LINE_RATE_HZ);
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for s in &line {
            writer.write_sample(*s).unwrap();
        }
        writer.finalize().unwrap();
    };

    let mut voice = Voice::load(model).unwrap();
    for (slug, text) in CANDIDATES {
        for take in 0..takes {
            write(format!("{out}/{slug}-{take:02}.wav"), &voice.speak(text, Tone::Warm).unwrap());
        }
    }

    let mut piper = Piper::new(Path::new(model), Path::new(&format!("{model}.json"))).unwrap();
    let mut speak = |phonemes: &str, (length, noise, noise_w): (f32, f32, f32), shape| {
        let (audio, rate) =
            piper.create(phonemes, true, None, Some(length), Some(noise), Some(noise_w)).unwrap();
        let samples: Vec<i16> =
            audio.iter().map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).collect();
        reshape(&resample_clip(&samples, rate, CORE_RATE_HZ), CORE_RATE_HZ, shape)
    };
    for (slug, phonemes) in PHONEMES {
        for take in 0..takes {
            write(format!("{out}/{slug}-{take:02}.wav"), &speak(phonemes, WARM, WARM_SHAPE));
        }
    }
    for (setting, scales, shape) in SETTINGS {
        for (slug, phonemes) in SETTINGS_WORDS {
            for take in 0..takes {
                let core = speak(phonemes, *scales, *shape);
                write(format!("{out}/{setting}_{slug}-{take:02}.wav"), &core);
            }
        }
    }
}
