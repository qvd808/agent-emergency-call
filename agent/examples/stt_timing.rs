//! Times whisper on the turns of the synthetic calls from the turn-detection prototype
//! (issue #17). Each call is an 8 kHz WAV with a JSON file giving each turn's text and
//! times; every turn is cut out, converted to 16 kHz as the line would, and transcribed.
//!
//! cargo run --release -p agent --example stt_timing -- <model.bin> <dir>

use std::path::Path;
use std::time::Instant;

use agent::audio::{CORE_RATE_HZ, resample_clip};
use agent::stt::Whisper;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, model, dir] = &args[..] else {
        panic!("usage: stt_timing <model.bin> <dir>");
    };
    let whisper = Whisper::load(model).unwrap();
    let mut times = vec![];
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for json in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let truth: serde_json::Value = serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
        let (audio, rate) = read_wav(&json.with_extension("wav"));
        let audio = resample_clip(&audio, rate, CORE_RATE_HZ);
        for turn in truth["turns"].as_array().unwrap() {
            let at = |key: &str| (turn[key].as_f64().unwrap() * CORE_RATE_HZ as f64) as usize;
            let started = Instant::now();
            let text = whisper.transcribe(&audio[at("start")..at("end").min(audio.len())]).unwrap();
            let ms = started.elapsed().as_secs_f64() * 1e3;
            times.push(ms);
            println!("{ms:6.0} ms  {:<50} | {text}", turn["text"].as_str().unwrap());
        }
    }
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    println!("n={} p50 {:.0} ms p95 {:.0} ms max {:.0} ms", times.len(), pct(0.5), pct(0.95), pct(1.0));
}

fn read_wav(path: &Path) -> (Vec<i16>, u32) {
    let mut reader = hound::WavReader::open(path).unwrap();
    let rate = reader.spec().sample_rate;
    (reader.samples::<i16>().map(Result::unwrap).collect(), rate)
}
