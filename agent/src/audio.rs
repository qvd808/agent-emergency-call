//! Sample rates and resampling.
//!
//! Inside the agent, audio is 16 kHz, 16-bit, mono, in 20 ms frames (issue #13). A phone line
//! carries 8 kHz, so the AudioSocket adapter converts both ways, one frame at a time. Clips of
//! any rate, such as a WAV file or Piper's output, are converted to 16 kHz whole.
//!
//! Resampling is rubato's `Fft`, the pick for a fixed 1:2 ratio in issue #24.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// The agent's own sample rate: what VAD and speech-to-text take (issue #13).
pub const CORE_RATE_HZ: u32 = 16_000;

/// The phone line's rate over AudioSocket. The dialplan app always sends 8 kHz (issue #4).
pub const LINE_RATE_HZ: u32 = 8_000;

pub const FRAME_MS: u32 = 20;

/// Samples in one 20 ms frame at 16 kHz.
pub const CORE_FRAME: usize = (CORE_RATE_HZ * FRAME_MS / 1000) as usize;

/// Samples in one 20 ms frame at 8 kHz: 320 bytes on the wire.
pub const LINE_FRAME: usize = (LINE_RATE_HZ * FRAME_MS / 1000) as usize;

/// Little-endian 16-bit bytes, as AudioSocket carries them, to samples. A trailing odd byte is
/// ignored; the caller keeps it for the next read.
pub fn samples_from_le_bytes(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

pub fn samples_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32_768.0).collect()
}

/// Rounds and clips. The anti-aliasing filter can overshoot a full-scale input a little, and
/// without the clip that would wrap around to the opposite sign: a loud click.
fn to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).collect()
}

/// Resamples a continuous stream, one 20 ms frame in and one 20 ms frame out. It keeps the
/// filter's state between frames, so the frame edges are inaudible. Converting each frame on
/// its own would reset the filter every 20 ms, which sounds like a 50 Hz buzz (inferred, not
/// listened to).
///
/// The output lags the input by [`FrameResampler::delay`] samples at the output rate.
pub struct FrameResampler {
    inner: Fft<f32>,
    frame_in: usize,
    frame_out: usize,
    out: Vec<f32>,
}

impl FrameResampler {
    pub fn new(rate_in: u32, rate_out: u32) -> Self {
        let frame_in = (rate_in * FRAME_MS / 1000) as usize;
        let frame_out = (rate_out * FRAME_MS / 1000) as usize;
        // One sub-chunk: the FFT block is one frame, which keeps the delay to a fraction of a
        // frame. Both sides fixed, so every call takes one frame and returns one frame.
        let inner = Fft::new_custom(
            rate_in as usize,
            rate_out as usize,
            frame_in,
            1,
            1,
            rubato::WindowFunction::BlackmanHarris2,
            FixedSync::Both,
        )
        .expect("20 ms at both rates is a whole number of samples");
        assert_eq!(inner.input_frames_next(), frame_in);
        assert_eq!(inner.output_frames_next(), frame_out);
        Self { inner, frame_in, frame_out, out: vec![0.0; frame_out] }
    }

    /// Line (8 kHz) to core (16 kHz).
    pub fn upsampler() -> Self {
        Self::new(LINE_RATE_HZ, CORE_RATE_HZ)
    }

    /// Core (16 kHz) to line (8 kHz).
    pub fn downsampler() -> Self {
        Self::new(CORE_RATE_HZ, LINE_RATE_HZ)
    }

    /// How far the output lags the input, in output samples.
    pub fn delay(&self) -> usize {
        self.inner.output_delay()
    }

    /// `frame` must be exactly one 20 ms frame at the input rate.
    pub fn process(&mut self, frame: &[i16]) -> Vec<i16> {
        assert_eq!(frame.len(), self.frame_in, "one 20 ms frame at a time");
        let input = to_f32(frame);
        let input = InterleavedSlice::new(&input[..], 1, self.frame_in).expect("sized above");
        let mut output =
            InterleavedSlice::new_mut(&mut self.out[..], 1, self.frame_out).expect("sized above");
        let (_, written) = self
            .inner
            .process_into_buffer(&input, &mut output, None)
            .expect("buffers are sized from the resampler's own frame counts");
        debug_assert_eq!(written, self.frame_out);
        to_i16(&self.out)
    }
}

/// Resamples a whole clip, such as a WAV file or one of Piper's sentences, from any rate to
/// any other. The filter's delay is trimmed off, so the output starts on the clip's first
/// sample.
pub fn resample_clip(samples: &[i16], rate_in: u32, rate_out: u32) -> Vec<i16> {
    if rate_in == rate_out || samples.is_empty() {
        return samples.to_vec();
    }
    let mut resampler =
        Fft::<f32>::new(rate_in as usize, rate_out as usize, 1024, 1, FixedSync::Input)
            .expect("both rates are positive");
    let input = to_f32(samples);
    let input = InterleavedSlice::new(&input[..], 1, samples.len()).expect("sized from input");
    let output = resampler
        .process_all(&input, samples.len(), None)
        .expect("process_all sizes its own output");
    to_i16(&output.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `hz`, `n` samples at `rate`, amplitude 0.5 of full scale.
    fn sine(hz: f32, rate: u32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (0.5 * (2.0 * std::f32::consts::PI * hz * t).sin() * 32_767.0) as i16
            })
            .collect()
    }

    /// Root mean square, as a fraction of full scale.
    fn rms(samples: &[i16]) -> f32 {
        let sum: f64 = samples.iter().map(|&s| (s as f64 / 32_768.0).powi(2)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    /// Root mean square of the difference between two equal-length signals.
    fn rms_error(a: &[i16], b: &[i16]) -> f32 {
        let d: Vec<i16> = a.iter().zip(b).map(|(&x, &y)| x.saturating_sub(y)).collect();
        rms(&d)
    }

    fn stream(resampler: &mut FrameResampler, input: &[i16]) -> Vec<i16> {
        input.chunks_exact(resampler.frame_in).flat_map(|f| resampler.process(f)).collect()
    }

    #[test]
    fn frame_sizes_are_20_ms() {
        assert_eq!(CORE_FRAME, 320);
        assert_eq!(LINE_FRAME, 160);
        let mut up = FrameResampler::upsampler();
        assert_eq!(up.process(&[0; LINE_FRAME]).len(), CORE_FRAME);
        let mut down = FrameResampler::downsampler();
        assert_eq!(down.process(&[0; CORE_FRAME]).len(), LINE_FRAME);
    }

    #[test]
    fn byte_order_is_little_endian() {
        assert_eq!(samples_from_le_bytes(&[0x01, 0x02, 0xff, 0xff, 0x09]), vec![0x0201, -1]);
        assert_eq!(samples_to_le_bytes(&[0x0201, -1]), vec![0x01, 0x02, 0xff, 0xff]);
    }

    /// A tone in the phone band survives the round trip 8 → 16 → 8 kHz, frame by frame, with
    /// no clicks at the frame edges. A click would show up as error well above the filter's.
    #[test]
    fn streaming_round_trip_keeps_a_phone_band_tone() {
        let input = sine(1_000.0, LINE_RATE_HZ, LINE_FRAME * 100);
        let mut up = FrameResampler::upsampler();
        let mut down = FrameResampler::downsampler();
        let wide = stream(&mut up, &input);
        let back = stream(&mut down, &wide);
        assert_eq!(back.len(), input.len());

        // Line up the output with the input, skipping the start-up transient.
        let delay = up.delay() / 2 + down.delay();
        let skip = LINE_FRAME * 5;
        let n = input.len() - skip - delay;
        let err = rms_error(&back[skip + delay..skip + delay + n], &input[skip..skip + n]);
        assert!(err < 0.01, "round-trip error {err} of full scale, tone rms {}", rms(&input));
    }

    /// 16 → 8 kHz must filter out what 8 kHz cannot carry. Without the filter, a 6 kHz tone
    /// folds down to 2 kHz and stays loud.
    #[test]
    fn downsampling_removes_what_the_line_cannot_carry() {
        let input = sine(6_000.0, CORE_RATE_HZ, CORE_FRAME * 50);
        let mut down = FrameResampler::downsampler();
        let out = stream(&mut down, &input);
        let settled = &out[LINE_FRAME * 5..];
        assert!(rms(settled) < 0.01, "6 kHz leaked through at rms {}", rms(settled));
    }

    #[test]
    fn clip_resampling_keeps_length_and_start() {
        // Piper's usual rate to the core's.
        let input = sine(440.0, 22_050, 22_050);
        let out = resample_clip(&input, 22_050, CORE_RATE_HZ);
        assert!((out.len() as i64 - 16_000).abs() <= 1, "{} samples", out.len());
        // The delay is trimmed: the output lines up with a reference tone to within half a sample.
        // It doesn't match exactly: 22050 to 16000 isn't a whole-number ratio, and measured
        // on this test the output leads by about 0.3 of a sample, 20 µs.
        let reference = sine(440.0, CORE_RATE_HZ, out.len());
        let (a, b) = (&out[800..out.len() - 800], &reference[800..out.len() - 800]);
        assert!(rms_error(a, b) < 0.03, "clip error {}", rms_error(a, b));
        assert!((rms(a) - rms(b)).abs() < 0.005, "level {} against {}", rms(a), rms(b));
    }

    #[test]
    fn full_scale_does_not_wrap_around() {
        let square: Vec<i16> = (0..LINE_FRAME * 20)
            .map(|i| if (i / 20) % 2 == 0 { i16::MAX } else { i16::MIN })
            .collect();
        let mut up = FrameResampler::upsampler();
        let out = stream(&mut up, &square);
        // A wrapped sample jumps across the full range between neighbours.
        let worst = out.windows(2).map(|w| (w[0] as i32 - w[1] as i32).abs()).max().unwrap();
        assert!(worst < 60_000, "a jump of {worst} between neighbouring samples");
    }
}
