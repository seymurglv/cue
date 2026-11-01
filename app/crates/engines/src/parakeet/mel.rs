//! The model's input: 128-band log-mel features of 16 kHz audio, computed the way NeMo's featurizer computes them for
//! Parakeet (ported from parakeet-rs): pre-emphasis, a centered STFT with a Hann window, Slaney mel bands, a log with
//! an additive guard, then every band normalized over the recording. A wrong spectrogram makes the model write
//! nothing at all, so each step follows NeMo exactly.
use std::f32::consts::PI;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// Mel bands: the features the encoder takes per frame.
pub const N_MELS: usize = 128;
const SAMPLE_RATE: usize = 16_000;
const N_FFT: usize = 512;
const WIN: usize = 400;
/// 10 ms: one feature frame.
pub const HOP: usize = 160;
const BINS: usize = N_FFT / 2 + 1;
const PREEMPHASIS: f32 = 0.97;
/// NeMo's `log_zero_guard_value` (type "add"): 2^-24.
const LOG_GUARD: f32 = 1.0 / 16_777_216.0;
/// Added to a band's standard deviation, as NeMo does, so a silent band does not divide by zero.
const STD_GUARD: f32 = 1e-5;

/// Features of one recording, band-major: `data[band * frames + frame]`, the `(1, 128, frames)` layout the encoder
/// takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Mel {
    pub frames: usize,
    pub data: Vec<f32>,
}

/// The FFT plan, the window and the mel filterbank, built once with the model.
pub struct Features {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// `N_MELS × BINS`, row-major.
    bank: Vec<f32>,
}

impl Default for Features {
    fn default() -> Self {
        Self::new()
    }
}

impl Features {
    pub fn new() -> Self {
        Self { fft: RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT), window: hann(WIN), bank: filterbank() }
    }

    /// Normalized log-mel of `pcm` (16 kHz mono, −1..1): `len / HOP + 1` frames, as the export's own preprocessor
    /// (`nemo128.onnx`) gives them and normalizes over.
    pub fn log_mel(&self, pcm: &[f32]) -> Mel {
        let mut mel = self.power_mel(&preemphasis(pcm));
        normalize(&mut mel);
        mel
    }

    /// log(mel power + guard), not normalized.
    fn power_mel(&self, audio: &[f32]) -> Mel {
        if audio.is_empty() {
            return Mel { frames: 0, data: Vec::new() };
        }
        let frames = audio.len() / HOP + 1;
        let padded = reflect_pad(audio, N_FFT / 2);
        // torch.stft centers a window shorter than the FFT in the FFT's buffer.
        let offset = (N_FFT - WIN) / 2;
        let mut input = vec![0.0f32; N_FFT];
        let mut output = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        let mut power = vec![0.0f32; BINS];
        let mut data = vec![0.0f32; N_MELS * frames];
        for frame in 0..frames {
            let start = frame * HOP;
            input.fill(0.0);
            for (i, w) in self.window.iter().enumerate() {
                input[offset + i] = padded[start + offset + i] * w;
            }
            // The lengths are the plan's own: the FFT cannot fail on them.
            if self.fft.process_with_scratch(&mut input, &mut output, &mut scratch).is_err() {
                continue;
            }
            for (p, c) in power.iter_mut().zip(&output) {
                *p = c.norm_sqr();
            }
            for band in 0..N_MELS {
                let row = &self.bank[band * BINS..(band + 1) * BINS];
                let energy: f32 = row.iter().zip(&power).map(|(w, p)| w * p).sum();
                data[band * frames + frame] = (energy + LOG_GUARD).ln();
            }
        }
        Mel { frames, data }
    }
}

/// `audio` with `pad` samples mirrored on each side (the edge sample not repeated), as torch's centered STFT pads;
/// audio too short to mirror is padded with silence.
fn reflect_pad(audio: &[f32], pad: usize) -> Vec<f32> {
    let n = audio.len();
    let mut out = Vec::with_capacity(n + 2 * pad);
    if n > pad {
        out.extend(audio[1..=pad].iter().rev());
        out.extend_from_slice(audio);
        out.extend(audio[n - 1 - pad..n - 1].iter().rev());
    } else {
        out.resize(pad, 0.0);
        out.extend_from_slice(audio);
        out.resize(n + 2 * pad, 0.0);
    }
    out
}

fn preemphasis(pcm: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(pcm.len());
    let mut last = None;
    for &x in pcm {
        out.push(last.map_or(x, |prev| x - PREEMPHASIS * prev));
        last = Some(x);
    }
    out
}

/// The symmetric Hann window NeMo uses (`periodic=False`).
fn hann(len: usize) -> Vec<f32> {
    (0..len).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (len as f32 - 1.0)).cos()).collect()
}

/// Every band to mean 0 and standard deviation 1 over the recording's frames (Bessel's correction, as NeMo). One frame
/// or none is left as it is.
fn normalize(mel: &mut Mel) {
    let n = mel.frames;
    if n <= 1 {
        return;
    }
    for band in mel.data.chunks_mut(n) {
        let mean = band.iter().sum::<f32>() / n as f32;
        let variance = band.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (n as f32 - 1.0);
        let std = variance.sqrt() + STD_GUARD;
        for x in band.iter_mut() {
            *x = (*x - mean) / std;
        }
    }
}

// The Slaney mel scale, as librosa computes it.
const F_SP: f64 = 200.0 / 3.0;
const MIN_LOG_HZ: f64 = 1_000.0;
const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;
const LOG_STEP: f64 = 0.068_751_777_420_949_12;

fn hz_to_mel(hz: f64) -> f64 {
    if hz < MIN_LOG_HZ { hz / F_SP } else { MIN_LOG_MEL + (hz / MIN_LOG_HZ).ln() / LOG_STEP }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel < MIN_LOG_MEL { mel * F_SP } else { MIN_LOG_HZ * ((mel - MIN_LOG_MEL) * LOG_STEP).exp() }
}

/// Triangular Slaney-normalized filters from 0 Hz to Nyquist.
fn filterbank() -> Vec<f32> {
    let top = hz_to_mel(SAMPLE_RATE as f64 / 2.0);
    let points: Vec<f64> = (0..=N_MELS + 1).map(|i| mel_to_hz(top * i as f64 / (N_MELS + 1) as f64)).collect();
    let freq = |bin: usize| bin as f64 * SAMPLE_RATE as f64 / N_FFT as f64;
    let mut bank = vec![0.0f32; N_MELS * BINS];
    for band in 0..N_MELS {
        let (lo, mid, hi) = (points[band], points[band + 1], points[band + 2]);
        let norm = 2.0 / (hi - lo);
        for bin in 0..BINS {
            let f = freq(bin);
            let ramp = ((f - lo) / (mid - lo)).min((hi - f) / (hi - mid)).max(0.0);
            bank[band * BINS + bin] = (ramp * norm) as f32;
        }
    }
    bank
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        (0..n).map(|i| 0.5 * (2.0 * PI * hz * i as f32 / SAMPLE_RATE as f32).sin()).collect()
    }

    /// The band whose filter peaks closest to `hz`.
    fn band_of(hz: f64) -> usize {
        let top = hz_to_mel(SAMPLE_RATE as f64 / 2.0);
        (0..N_MELS)
            .min_by(|&a, &b| {
                let centre = |band: usize| mel_to_hz(top * (band + 1) as f64 / (N_MELS + 1) as f64);
                (centre(a) - hz).abs().total_cmp(&(centre(b) - hz).abs())
            })
            .unwrap()
    }

    #[test]
    fn frames_are_ten_milliseconds_and_band_major() {
        let f = Features::new();
        let mel = f.log_mel(&vec![0.0; 16_000]);
        assert_eq!(mel.frames, 101);
        assert_eq!(mel.data.len(), N_MELS * 101);
        assert!(mel.data.iter().all(|x| x.is_finite()), "silence stays finite");
        assert_eq!(f.log_mel(&[0.1; 159]).frames, 1);
        assert_eq!(f.log_mel(&[]).frames, 0);
    }

    #[test]
    fn a_tone_lands_in_its_band() {
        let f = Features::new();
        for hz in [300.0, 1_000.0, 3_000.0] {
            let mel = f.power_mel(&sine(hz, 0.5));
            let frame = mel.frames / 2;
            let loudest =
                (0..N_MELS).max_by(|&a, &b| mel.data[a * mel.frames + frame].total_cmp(&mel.data[b * mel.frames + frame]));
            let expected = band_of(f64::from(hz)) as i64;
            assert!((loudest.unwrap() as i64 - expected).abs() <= 1, "{hz} Hz: band {loudest:?}, expected {expected}");
        }
    }

    #[test]
    fn every_band_is_normalized_over_the_recording() {
        let f = Features::new();
        let audio: Vec<f32> = sine(440.0, 1.0).iter().zip(sine(2_500.0, 1.0)).map(|(a, b)| a + b * 0.3).collect();
        let mel = f.log_mel(&audio);
        for band in mel.data.chunks(mel.frames) {
            let mean = band.iter().sum::<f32>() / mel.frames as f32;
            assert!(mean.abs() < 1e-3, "mean {mean}");
        }
    }

    #[test]
    fn filters_cover_the_spectrum_with_slaney_area() {
        let bank = filterbank();
        for band in 0..N_MELS {
            let row = &bank[band * BINS..(band + 1) * BINS];
            assert!(row.iter().any(|w| *w > 0.0), "band {band} is empty");
            assert!(row.iter().all(|w| *w >= 0.0));
        }
        assert_eq!(band_of(0.0), 0);
    }

    #[test]
    fn edges_are_mirrored() {
        assert_eq!(reflect_pad(&[1.0, 2.0, 3.0, 4.0], 2), [3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 2.0]);
        assert_eq!(reflect_pad(&[1.0, 2.0], 2), [0.0, 0.0, 1.0, 2.0, 0.0, 0.0], "too short to mirror");
    }

    #[test]
    fn preemphasis_keeps_the_first_sample() {
        assert_eq!(preemphasis(&[1.0, 1.0, 0.0]), [1.0, 1.0 - 0.97, -0.97]);
        assert!(preemphasis(&[]).is_empty());
    }
}
