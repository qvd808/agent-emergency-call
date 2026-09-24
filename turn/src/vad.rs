//! Silero VAD (ONNX), from the turn-detection prototype (issue #12). Mirrors
//! `OnnxWrapper.__call__` in Silero's `utils_vad.py`: each call takes exactly 512 samples at
//! 16 kHz, prefixed with the last 64 samples of the previous call, and carries a [2, 1, 128]
//! state.

use ndarray::{Array2, Array3, arr0};
use ort::{session::Session, value::TensorRef};

/// Samples per window at 16 kHz: 32 ms.
pub const WINDOW: usize = 512;
const CONTEXT: usize = 64;
const SAMPLE_RATE: i64 = 16_000;

pub struct Silero {
    session: Session,
    state: Array3<f32>,
    context: Vec<f32>,
}

impl Silero {
    pub fn new(model: &str) -> ort::Result<Self> {
        let session = Session::builder()?.with_intra_threads(1)?.commit_from_file(model)?;
        Ok(Silero { session, state: Array3::zeros((2, 1, 128)), context: vec![0.0; CONTEXT] })
    }

    /// Speech probability for one window of [`WINDOW`] samples in [-1, 1].
    pub fn prob(&mut self, window: &[f32]) -> ort::Result<f32> {
        assert_eq!(window.len(), WINDOW);
        let mut x = self.context.clone();
        x.extend_from_slice(window);
        self.context = x[x.len() - CONTEXT..].to_vec();
        let input = Array2::from_shape_vec((1, x.len()), x).expect("one row of CONTEXT + WINDOW");
        let sr = arr0(SAMPLE_RATE);
        let out = self.session.run(ort::inputs![
            "input" => TensorRef::from_array_view(&input)?,
            "state" => TensorRef::from_array_view(&self.state)?,
            "sr" => TensorRef::from_array_view(&sr)?,
        ])?;
        let p = out["output"].try_extract_array::<f32>()?[[0, 0]];
        let state = out["stateN"].try_extract_array::<f32>()?;
        self.state = state.to_shape((2, 1, 128)).expect("Silero's state is [2, 1, 128]").to_owned();
        Ok(p)
    }
}
