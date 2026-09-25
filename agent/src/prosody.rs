//! Prosody (issue #37): reshapes the pitch of Piper's speech, so the agent's voice rises and
//! falls more than Piper alone makes it, and sits higher or lower to suit the
//! [`Tone`](crate::tts::Tone).
//!
//! Piper takes no pitch setting. Its three settings change timing and how much the voice
//! varies at random, not where the pitch goes. So the pitch is moved after synthesis, in
//! two steps:
//!
//! 1. **Track the pitch** (F0) every 10 ms with YIN: A. de Cheveigné and H. Kawahara, "YIN, a
//!    fundamental frequency estimator for speech and music", J. Acoust. Soc. Am. 111(4),
//!    1917-1930, April 2002, doi:10.1121/1.1458024, fetched from
//!    <http://recherche.ircam.fr/equipes/pcm/cheveign/pss/2002_JASA_YIN.pdf>.
//! 2. **Move it** with TD-PSOLA: E. Moulines and F. Charpentier, "Pitch-synchronous waveform
//!    processing techniques for text-to-speech synthesis using diphones", Speech
//!    Communication 9 (1990) 453-467, fetched from
//!    <https://courses.physics.illinois.edu/ece420/sp2019/5_PSOLA.pdf>. The voice is cut into
//!    grains two pitch periods long, one centred on each glottal pulse ("pitch-mark"), and the
//!    grains are added back together at a new spacing. Closer together is a higher pitch. Each
//!    grain keeps its own shape, which carries the vowel and the speaker, so the voice still
//!    sounds like the same person rather than sped up.
//!
//! The new pitch contour is the old one stretched around its median: `range` 1.5 makes every
//! rise and fall half as large again, and `shift_semitones` moves the whole line up or down.

/// How to reshape a line's pitch. [`Shape::NONE`] leaves it as Piper spoke it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Moves the whole line up (positive) or down, in semitones.
    pub shift_semitones: f32,
    /// Stretches the pitch contour around its median, measured in semitones: 1 keeps it, 1.5
    /// makes each rise and fall half as large again, below 1 flattens it.
    pub range: f32,
}

impl Shape {
    pub const NONE: Shape = Shape { shift_semitones: 0.0, range: 1.0 };
}

/// Pitch is tracked every 10 ms.
const HOP_S: f64 = 0.010;
/// YIN's integration window: the paper's statistics use 25 ms (p. 1922).
const WINDOW_S: f64 = 0.025;
/// The pitch range searched. Piper's voices speak well inside it (inferred from the voices
/// heard so far, not measured for every voice).
const MIN_F0: f64 = 70.0;
const MAX_F0: f64 = 500.0;
/// YIN step 4: the first dip of the normalised difference below this is the period (the
/// paper's value, p. 1920).
const YIN_THRESHOLD: f64 = 0.1;
/// A frame is voiced when its best dip is below this. Not from the paper, which always
/// returns an estimate; chosen so that Praat and this tracker agree on Piper's speech.
const VOICED_BELOW: f64 = 0.25;
/// Quieter frames than this (RMS, full scale 1.0) are silence, whatever YIN says.
const SILENCE_RMS: f64 = 0.01;
/// A voiced stretch shorter than this is too short to track pitch-marks through, and is
/// left as it is.
const MIN_VOICED_FRAMES: usize = 3;
/// Pitch-marks in unvoiced speech are this far apart. The paper sets them "at a constant
/// rate" (p. 455); the rate itself is chosen.
const UNVOICED_STEP_S: f64 = 0.005;
/// No period is moved by more than these factors, to keep the grains overlapping.
const MIN_RATIO: f64 = 0.75;
const MAX_RATIO: f64 = 1.4;

/// Reshapes the pitch of `samples` at `rate` Hz. The result is the same length.
pub fn reshape(samples: &[i16], rate: u32, shape: Shape) -> Vec<i16> {
    if shape == Shape::NONE || samples.is_empty() {
        return samples.to_vec();
    }
    let x: Vec<f64> = samples.iter().map(|&s| s as f64 / 32_768.0).collect();
    let rate = rate as f64;
    let track = Track::of(&x, rate);
    let Some(reference) = track.median() else {
        // Nothing voiced: no pitch to move.
        return samples.to_vec();
    };
    let marks = pitch_marks(&x, rate, &track);
    let target = |f0: f64| {
        let semitones = 12.0 * (f0 / reference).log2();
        reference * 2f64.powf((semitones * shape.range as f64 + shape.shift_semitones as f64) / 12.0)
    };
    let y = overlap_add(&x, &marks, |t| {
        track.f0_at(t, rate).map(|f0| (target(f0) / f0).clamp(MIN_RATIO, MAX_RATIO))
    });
    y.iter().map(|&s| (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).collect()
}

/// The pitch every [`HOP_S`], smoothed; `None` where the frame is unvoiced.
pub struct Track {
    f0: Vec<Option<f64>>,
}

impl Track {
    pub fn of(x: &[f64], rate: f64) -> Self {
        let hop = (HOP_S * rate).round() as usize;
        let window = (WINDOW_S * rate).round() as usize;
        let max_lag = (rate / MIN_F0).ceil() as usize;
        let min_lag = (rate / MAX_F0).floor() as usize;
        let smooth = low_pass(x, rate);
        let frames = x.len() / hop + 1;
        let mut f0: Vec<Option<f64>> = (0..frames)
            .map(|i| {
                // The frame is centred on its time, i * hop.
                let start = (i * hop).saturating_sub(window / 2);
                let end = start + window + max_lag;
                if end > smooth.len() {
                    return None;
                }
                let frame = &smooth[start..end];
                let rms = (frame[..window].iter().map(|s| s * s).sum::<f64>() / window as f64).sqrt();
                if rms < SILENCE_RMS {
                    return None;
                }
                yin(frame, window, min_lag, max_lag).map(|period| rate / period)
            })
            .collect();
        drop_short_runs(&mut f0);
        Track { f0: median_filter(&f0) }
    }

    /// The pitch at `t` samples, interpolated between frames; `None` if unvoiced there.
    pub fn f0_at(&self, t: f64, rate: f64) -> Option<f64> {
        let at = t / (HOP_S * rate);
        let i = at.floor() as usize;
        let frac = at - i as f64;
        match (self.f0.get(i).copied().flatten(), self.f0.get(i + 1).copied().flatten()) {
            (Some(a), Some(b)) => Some(a + (b - a) * frac),
            (Some(a), None) => Some(a),
            (None, Some(b)) if frac > 0.5 => Some(b),
            _ => None,
        }
    }

    /// The median voiced pitch, the centre the contour is stretched around.
    pub fn median(&self) -> Option<f64> {
        let mut voiced: Vec<f64> = self.f0.iter().flatten().copied().collect();
        if voiced.is_empty() {
            return None;
        }
        voiced.sort_by(f64::total_cmp);
        Some(voiced[voiced.len() / 2])
    }
}

/// YIN steps 2 to 5 on one frame: the period in samples, or `None` if the frame is not
/// periodic enough to be voiced.
fn yin(frame: &[f64], window: usize, min_lag: usize, max_lag: usize) -> Option<f64> {
    // Step 2, the difference function: d(tau) = sum over the window of (x_j - x_{j+tau})^2.
    let d: Vec<f64> = (0..=max_lag)
        .map(|tau| (0..window).map(|j| (frame[j] - frame[j + tau]).powi(2)).sum())
        .collect();
    // Step 3, cumulative mean normalisation: d'(tau) = d(tau) / ((1/tau) sum_{j=1..tau} d(j)),
    // with d'(0) = 1.
    let mut norm = vec![1.0; max_lag + 1];
    let mut running = 0.0;
    for tau in 1..=max_lag {
        running += d[tau];
        norm[tau] = if running > 0.0 { d[tau] * tau as f64 / running } else { 1.0 };
    }
    // Step 4, absolute threshold: the first dip below it, followed down to its bottom; if
    // none, the global minimum.
    let mut tau = (min_lag..max_lag).find(|&t| norm[t] < YIN_THRESHOLD).unwrap_or_else(|| {
        (min_lag..max_lag).min_by(|&a, &b| norm[a].total_cmp(&norm[b])).unwrap_or(min_lag)
    });
    while tau + 1 < max_lag && norm[tau + 1] < norm[tau] {
        tau += 1;
    }
    if norm[tau] >= VOICED_BELOW || tau <= min_lag.max(1) {
        return None;
    }
    // Step 5, parabolic interpolation around the dip for a period between samples.
    let (a, b, c) = (norm[tau - 1], norm[tau], norm[tau + 1]);
    let denominator = a - 2.0 * b + c;
    let offset = if denominator.abs() > f64::EPSILON { 0.5 * (a - c) / denominator } else { 0.0 };
    Some(tau as f64 + offset.clamp(-0.5, 0.5))
}

/// The paper's prefilter: a 1 ms moving average, which removes energy above about 1 kHz
/// (p. 1922, "convolution with a 1-ms square window"). Used for tracking and for placing
/// pitch-marks; the grains themselves are cut from the unfiltered voice.
fn low_pass(x: &[f64], rate: f64) -> Vec<f64> {
    let width = ((0.001 * rate).round() as usize).max(1);
    let mut out = Vec::with_capacity(x.len());
    let mut sum = 0.0;
    for i in 0..x.len() {
        sum += x[i];
        if i >= width {
            sum -= x[i - width];
        }
        out.push(sum / width.min(i + 1) as f64);
    }
    // Centre the average, so a peak stays where it was.
    out.rotate_left(width / 2);
    out
}

/// Voiced stretches shorter than [`MIN_VOICED_FRAMES`] become unvoiced.
fn drop_short_runs(f0: &mut [Option<f64>]) {
    let mut i = 0;
    while i < f0.len() {
        if f0[i].is_none() {
            i += 1;
            continue;
        }
        let start = i;
        while i < f0.len() && f0[i].is_some() {
            i += 1;
        }
        if i - start < MIN_VOICED_FRAMES {
            f0[start..i].fill(None);
        }
    }
}

/// A five-frame median over the voiced frames, which removes single-frame octave slips
/// without moving the contour's rises and falls.
fn median_filter(f0: &[Option<f64>]) -> Vec<Option<f64>> {
    (0..f0.len())
        .map(|i| {
            f0[i]?;
            let mut near: Vec<f64> =
                f0[i.saturating_sub(2)..(i + 3).min(f0.len())].iter().flatten().copied().collect();
            near.sort_by(f64::total_cmp);
            Some(near[near.len() / 2])
        })
        .collect()
}

/// Pitch-marks, in samples: one on each pulse in voiced speech, one every
/// [`UNVOICED_STEP_S`] elsewhere, plus the first and last sample.
fn pitch_marks(x: &[f64], rate: f64, track: &Track) -> Vec<Mark> {
    let smooth = low_pass(x, rate);
    let unvoiced_step = (UNVOICED_STEP_S * rate).round() as usize;
    let mut marks = vec![Mark { at: 0, voiced: false }];
    let mut t = 0;
    while t < x.len() {
        let Some(f0) = track.f0_at(t as f64, rate) else {
            t += unvoiced_step;
            if t < x.len() {
                marks.push(Mark { at: t, voiced: false });
            }
            continue;
        };
        // A voiced stretch: find its end, and which way its pulses point.
        let start = t;
        let mut end = t;
        while end < x.len() && track.f0_at(end as f64, rate).is_some() {
            end += 1;
        }
        let peak = smooth[start..end].iter().fold(0f64, |m, &s| m.max(s));
        let trough = smooth[start..end].iter().fold(0f64, |m, &s| m.min(s));
        let sign = if peak >= -trough { 1.0 } else { -1.0 };
        let pulse = |from: usize, to: usize| {
            (from..to.min(end)).max_by(|&a, &b| (sign * smooth[a]).total_cmp(&(sign * smooth[b])))
        };
        // The first pulse is the strongest point in the first period after the last mark; each
        // next one is the strongest point within a quarter period of where the pitch says it
        // should be.
        let from = marks.last().map_or(start, |last| start.max(last.at + 1));
        let mut mark = pulse(from, from + (rate / f0) as usize);
        while let Some(m) = mark {
            if marks.last().is_some_and(|last| m <= last.at) {
                break;
            }
            marks.push(Mark { at: m, voiced: true });
            let period = rate / track.f0_at(m as f64, rate).unwrap_or(f0);
            let guess = m + period.round() as usize;
            let slack = (period / 4.0).round() as usize;
            if guess + slack >= end {
                break;
            }
            mark = pulse(guess - slack, guess + slack + 1);
        }
        t = end.max(marks.last().map_or(end, |m| m.at + 1));
        if t < x.len() && marks.last().is_some_and(|m| t - m.at >= unvoiced_step) {
            marks.push(Mark { at: t, voiced: false });
        }
    }
    if marks.last().is_some_and(|m| m.at != x.len() - 1) {
        marks.push(Mark { at: x.len() - 1, voiced: false });
    }
    marks
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    at: usize,
    voiced: bool,
}

/// TD-PSOLA synthesis. Walks new marks through time and places at each the grain of the
/// nearest old mark: a Hann window from the old mark before it to the one after it, two
/// periods long in voiced speech (p. 455). Between two voiced old marks the next new mark
/// comes after their gap divided by `ratio(t)`; anywhere else it is simply the next old mark,
/// so unvoiced sounds are copied as they are and each voiced stretch starts in step with the
/// old one. With no change the windows add to exactly one; where the spacing changes they
/// don't, so the sum is divided by the windows' own sum, the paper's normalisation for "the
/// variable overlap between the successive windows" (p. 455).
fn overlap_add(x: &[f64], marks: &[Mark], ratio: impl Fn(f64) -> Option<f64>) -> Vec<f64> {
    let n = x.len();
    let mut out = vec![0.0; n];
    let mut weight = vec![0.0; n];
    let mut t = 0.0;
    while t < n as f64 {
        let k = nearest(marks, t);
        let centre = marks[k].at as isize;
        let left = if k > 0 { centre - marks[k - 1].at as isize } else { 0 };
        let right = if k + 1 < marks.len() { marks[k + 1].at as isize - centre } else { 0 };
        let placed = t.round() as isize;
        for j in -left..=right {
            let w = match j {
                0 => 1.0,
                j if j < 0 => 0.5 - 0.5 * (std::f64::consts::PI * (j + left) as f64 / left as f64).cos(),
                j => 0.5 + 0.5 * (std::f64::consts::PI * j as f64 / right as f64).cos(),
            };
            let (from, to) = (centre + j, placed + j);
            if (0..n as isize).contains(&from) && (0..n as isize).contains(&to) {
                out[to as usize] += w * x[from as usize];
                weight[to as usize] += w;
            }
        }
        // Voiced: the old gap after the grain's mark, scaled. Otherwise: the next old mark.
        t = match (marks.get(k + 1), ratio(t)) {
            (Some(next), Some(r)) if marks[k].voiced && next.voiced => t + (right as f64 / r).max(1.0),
            _ => match marks.get(marks.partition_point(|m| m.at as f64 <= t)) {
                Some(next) => next.at as f64,
                None => break,
            },
        };
    }
    // Where the windows add to less than a half, only the edges of grains reach: those are
    // left quiet rather than boosted.
    out.iter().zip(&weight).map(|(s, w)| s / w.max(0.5)).collect()
}

/// The index of the mark nearest to `t`.
fn nearest(marks: &[Mark], t: f64) -> usize {
    let i = marks.partition_point(|m| (m.at as f64) < t);
    if i == 0 {
        return 0;
    }
    if i == marks.len() {
        return marks.len() - 1;
    }
    if t - marks[i - 1].at as f64 <= marks[i].at as f64 - t { i - 1 } else { i }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// A voice-like buzz: pulses at a pitch gliding from `from` to `to` Hz, each rung like a
    /// vowel with a decaying 700 Hz resonance, for `seconds`.
    fn buzz(from: f64, to: f64, seconds: f64) -> Vec<i16> {
        let n = (seconds * RATE as f64) as usize;
        let mut out = vec![0.0; n];
        let mut next = 0.0;
        while (next as usize) < n {
            let at = next as usize;
            for (i, s) in out[at..].iter_mut().enumerate().take(400) {
                let t = i as f64 / RATE as f64;
                *s += (-t * 300.0).exp() * (2.0 * std::f64::consts::PI * 700.0 * t).sin();
            }
            let f0 = from + (to - from) * at as f64 / n as f64;
            next += RATE as f64 / f0;
        }
        out.iter().map(|&s| (s * 8_000.0) as i16).collect()
    }

    fn pitch(samples: &[i16]) -> Track {
        let x: Vec<f64> = samples.iter().map(|&s| s as f64 / 32_768.0).collect();
        Track::of(&x, RATE as f64)
    }

    /// Semitones between the 10th and 90th percentile of the voiced pitch.
    fn span(track: &Track) -> f64 {
        let mut f: Vec<f64> = track.f0.iter().flatten().copied().collect();
        f.sort_by(f64::total_cmp);
        12.0 * (f[f.len() * 9 / 10] / f[f.len() / 10]).log2()
    }

    #[test]
    fn yin_finds_a_steady_pitch() {
        let median = pitch(&buzz(200.0, 200.0, 1.0)).median().unwrap();
        assert!((median - 200.0).abs() < 1.0, "{median}");
    }

    #[test]
    fn silence_has_no_pitch() {
        assert_eq!(pitch(&vec![0; RATE as usize]).median(), None);
    }

    #[test]
    fn reshaping_keeps_the_length() {
        let voice = buzz(150.0, 250.0, 1.0);
        let shaped = reshape(&voice, RATE, Shape { shift_semitones: 2.0, range: 1.5 });
        assert_eq!(shaped.len(), voice.len());
    }

    #[test]
    fn a_shift_moves_the_pitch_by_that_many_semitones() {
        let shaped = reshape(&buzz(180.0, 180.0, 1.0), RATE, Shape { shift_semitones: 3.0, range: 1.0 });
        let median = pitch(&shaped).median().unwrap();
        let expected = 180.0 * 2f64.powf(3.0 / 12.0);
        assert!((median - expected).abs() < 3.0, "{median} vs {expected}");
    }

    #[test]
    fn a_wider_range_widens_the_contour() {
        let voice = buzz(160.0, 240.0, 1.5);
        let before = span(&pitch(&voice));
        let after = span(&pitch(&reshape(&voice, RATE, Shape { shift_semitones: 0.0, range: 1.5 })));
        assert!(after > before * 1.3, "{before:.2} st -> {after:.2} st");
    }

    #[test]
    fn no_change_is_almost_the_same_audio() {
        let voice = buzz(150.0, 250.0, 1.0);
        // A range of exactly 1 would skip the reshaping, so this runs the full path.
        let shaped = reshape(&voice, RATE, Shape { shift_semitones: 0.0, range: 1.0001 });
        let error: f64 = voice.iter().zip(&shaped).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum();
        let energy: f64 = voice.iter().map(|&a| (a as f64).powi(2)).sum();
        let snr_db = 10.0 * (energy / error).log10();
        assert!(snr_db > 20.0, "{snr_db:.1} dB");
    }
}
