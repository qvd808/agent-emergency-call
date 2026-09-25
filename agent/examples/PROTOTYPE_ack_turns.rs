//! PROTOTYPE, throwaway: a listening set for the acknowledgements (issue #45), spoken exactly
//! as the agent speaks them (`Voice::speak`, conversation.rs) and written as the 8 kHz audio a
//! softphone hears.
//!
//! - `ack-<word>-<take>.wav`: each acknowledgement alone, three takes, the old two included.
//!   The agent renders one take at startup and plays it all session.
//! - `turn-<word>.wav`: the acknowledgement, the wait, then a reply in the tone its status
//!   gets (`tone` in conversation.rs). The wait is 1.2 s, about what the 19:36 call of
//!   2026-09-24 left between the acknowledgement's end and the reply's start (0.8-1.4 s over
//!   five turns, read off Silero VAD segments of the recording, so approximate).
//!
//! PIPER_ESPEAKNG_DATA_DIRECTORY=models cargo run --release -p agent \
//!     --example PROTOTYPE_ack_turns -- <out dir> models/en_US-hfc_female-medium.onnx

use agent::audio::{CORE_RATE_HZ, LINE_RATE_HZ, resample_clip};
use agent::tts::{Tone, Voice};

const ACKS: &[(&str, &str)] =
    &[("okay", "Okay."), ("right", "Right."), ("old-mm-hm", "Mm-hm."), ("old-i-see", "I see.")];

/// A reply after good news (status `ok`, Warm) and after bad (status `concern`, Steady).
const TURNS: &[(&str, &str, &str, Tone)] = &[
    ("okay", "Okay.", "That's good to hear. Have you had any falls lately?", Tone::Warm),
    ("right", "Right.", "I'm sorry your back hurts. Have you had anything to eat today?", Tone::Steady),
    ("old-mm-hm", "Mm-hm.", "That's good to hear. Have you had any falls lately?", Tone::Warm),
];

const WAIT_S: f64 = 1.2;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (out, model) = (&args[1], &args[2]);
    std::fs::create_dir_all(out).unwrap();
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
    for (slug, text) in ACKS {
        for take in 1..=3 {
            write(format!("{out}/ack-{slug}-{take}.wav"), &voice.speak(text, Tone::Warm).unwrap());
        }
    }
    for (slug, ack, reply, tone) in TURNS {
        let mut core = voice.speak(ack, Tone::Warm).unwrap();
        core.extend(std::iter::repeat_n(0, (WAIT_S * CORE_RATE_HZ as f64) as usize));
        core.extend(voice.speak(reply, *tone).unwrap());
        write(format!("{out}/turn-{slug}.wav"), &core);
    }
}
