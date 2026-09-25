//! PROTOTYPE, throwaway: transcribes a 16 kHz mono WAV with timestamps, to read a recorded
//! call. cargo run --release -p agent --example PROTOTYPE_transcribe -- <model> <wav>

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ctx = WhisperContext::new_with_params(&args[1], WhisperContextParameters::default()).unwrap();
    let mut reader = hound::WavReader::open(&args[2]).unwrap();
    let audio: Vec<f32> =
        reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32_768.0).collect();
    let mut state = ctx.create_state().unwrap();
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_n_threads(8);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    state.full(params, &audio).unwrap();
    for segment in state.as_iter() {
        let (t0, t1) = (segment.start_timestamp() as f64 / 100.0, segment.end_timestamp() as f64 / 100.0);
        println!("{t0:6.2}-{t1:6.2} {}", segment.to_str_lossy().unwrap());
    }
}
