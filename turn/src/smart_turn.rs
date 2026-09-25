//! Smart Turn v3.2 (ONNX), audio end-of-turn model.
//!
//! Input contract from pipecat-ai/smart-turn `inference.py` and `audio_utils.py`: the last
//! 8 s of 16 kHz audio, zero-padded at the start; `WhisperFeatureExtractor(chunk_length=8)`
//! with `do_normalize=True`; the model returns P(complete) for `input_features` [1, 80, 800].
//!
//! The feature extractor below is a port of the Whisper log-mel front end. Its parity with
//! the Python reference was checked on the prototype branch (`prototype/turn-detection`,
//! `turn/prototype/results/parity.txt`: features within 4.05e-05, the same P(complete) as the
//! reference on the same ONNX Runtime). That check ran on ort 2.0.0-rc.13; the workspace pins
//! rc.12, whose output on the same audio is untested.
//!
//! The agent uses it only to hold the floor (issue #12): 1.5 s of silence ends a turn, unless
//! Smart Turn gives P(complete) < 0.05, and then 3 s does.

use ndarray::Array3;
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::TensorRef,
};
use rustfft::{FftPlanner, num_complex::Complex};

const SR: usize = 16000;
const N_FFT: usize = 400;
const HOP: usize = 160;
const N_MELS: usize = 80;
pub const SECONDS: usize = 8;
const FRAMES: usize = SECONDS * SR / HOP; // 800

pub struct SmartTurn {
    session: Session,
    filters: Vec<f32>, // N_MELS x (N_FFT/2+1), row-major
    window: Vec<f32>,
}

impl SmartTurn {
    pub fn new(model: &str) -> ort::Result<Self> {
        // ENABLE_ALL, as the reference `inference.py`. The int8 model's output moves with the
        // optimisation level (0.099 vs 0.118 on the same input, parity check), so match it.
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::All)?
            .with_intra_threads(1)?
            .commit_from_file(model)?;
        // Periodic Hann, as numpy/transformers `window_function(n, "hann")`.
        let window = (0..N_FFT)
            .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos())
            .map(|w| w as f32)
            .collect();
        Ok(SmartTurn { session, filters: mel_filters(), window })
    }

    /// P(turn complete) for 16 kHz audio of any length (the last 8 s are used).
    pub fn predict(&mut self, audio: &[f32]) -> ort::Result<f32> {
        let feats = self.features(audio);
        let out = self.session.run(ort::inputs!["input_features" => TensorRef::from_array_view(&feats)?])?;
        let (_, v) = out[0].try_extract_tensor::<f32>()?;
        Ok(v[0])
    }

    /// Whisper log-mel features, [1, 80, 800].
    pub fn features(&self, audio: &[f32]) -> Array3<f32> {
        let n = SECONDS * SR;
        let mut x = vec![0f32; n];
        let take = audio.len().min(n);
        x[n - take..].copy_from_slice(&audio[audio.len() - take..]);

        // zero_mean_unit_var_norm over the whole (padded) 8 s
        let mean = x.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
        let var = x.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n as f64;
        let sd = (var + 1e-7).sqrt();
        for v in &mut x {
            *v = ((*v as f64 - mean) / sd) as f32;
        }

        // center=True: reflect-pad n_fft/2 on each side
        let pad = N_FFT / 2;
        let mut p = Vec::with_capacity(n + 2 * pad);
        p.extend((1..=pad).rev().map(|i| x[i]));
        p.extend_from_slice(&x);
        p.extend((n - 1 - pad..n - 1).rev().map(|i| x[i]));

        let bins = N_FFT / 2 + 1;
        let fft = FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
        let mut logmel = vec![0f32; N_MELS * FRAMES];
        let mut buf = vec![Complex::new(0f32, 0f32); N_FFT];
        let mut power = vec![0f32; bins];
        for f in 0..FRAMES {
            // frame FRAMES (the 801st) is dropped, as `log_spec[:, :-1]`
            for i in 0..N_FFT {
                buf[i] = Complex::new(p[f * HOP + i] * self.window[i], 0.0);
            }
            fft.process(&mut buf);
            for k in 0..bins {
                power[k] = buf[k].norm_sqr();
            }
            for m in 0..N_MELS {
                let row = &self.filters[m * bins..(m + 1) * bins];
                let e: f32 = row.iter().zip(&power).map(|(a, b)| a * b).sum();
                logmel[m * FRAMES + f] = e.max(1e-10).log10();
            }
        }
        let max = logmel.iter().cloned().fold(f32::MIN, f32::max);
        for v in &mut logmel {
            *v = ((*v).max(max - 8.0) + 4.0) / 4.0;
        }
        Array3::from_shape_vec((1, N_MELS, FRAMES), logmel).unwrap()
    }
}

/// Slaney-style mel filter bank with Slaney area normalisation (librosa / transformers
/// `mel_filter_bank(..., norm="slaney", mel_scale="slaney")`), 0-8000 Hz.
fn mel_filters() -> Vec<f32> {
    fn hz_to_mel(f: f64) -> f64 {
        let (f_sp, min_log_hz, min_log_mel, logstep) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
        if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
    }
    fn mel_to_hz(m: f64) -> f64 {
        let (f_sp, min_log_hz, min_log_mel, logstep) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
        if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { m * f_sp }
    }
    let bins = N_FFT / 2 + 1;
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(SR as f64 / 2.0));
    let pts: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64))
        .collect();
    let fft_f: Vec<f64> = (0..bins).map(|k| k as f64 * (SR as f64 / 2.0) / (bins - 1) as f64).collect();
    let mut w = vec![0f32; N_MELS * bins];
    for m in 0..N_MELS {
        let enorm = 2.0 / (pts[m + 2] - pts[m]);
        for k in 0..bins {
            let down = (fft_f[k] - pts[m]) / (pts[m + 1] - pts[m]);
            let up = (pts[m + 2] - fft_f[k]) / (pts[m + 2] - pts[m + 1]);
            w[m * bins + k] = (down.min(up).max(0.0) * enorm) as f32;
        }
    }
    w
}
