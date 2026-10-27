//! Audio for transcription: any input stream → 16 kHz mono i16; the level for the Drop's bars;
//! "microphone is silent" means 4 s of digital silence (below −70 dBFS), not a quiet room.

pub const OUT_RATE: u32 = 16_000;
/// Filter length up to and including 48 kHz; above that it grows with the rate so the transition band in hertz does not widen.
const TAPS: usize = 63;
/// Cap on the filter half-width (384 kHz) and on the number of precomputed phases: memory stays bounded at any rate.
const MAX_HALF: usize = 248;
const MAX_PHASES: u64 = 1_024;
const CUTOFF_HZ: f64 = 7_200.0;
const DEAD_DBFS: f32 = -70.0;
const DEAD_AFTER_MS: u64 = 4_000;

/// Any rate → 16 kHz. The filter is a Blackman-windowed sinc (at 48 kHz: 63 taps, 7.2 kHz cutoff), precomputed
/// for every fractional position of an output sample between input samples. It both removes everything above 8 kHz
/// before decimation and interpolates (no linear interpolation, which distorts at 44.1 kHz). The output sample
/// position is computed in integers, so length and timing never drift on an arbitrarily long stream. State lives
/// between calls: chunks of any length give the same output as one chunk. Latency is the filter half-width:
/// 0.65 ms at 48 kHz and above, 0.7 ms at 44.1 kHz, up to 3.9 ms at 8 kHz.
pub struct Resampler {
    in_rate: u64,
    passthrough: bool,
    /// Filter length (2·half-width + 1) and number of phases.
    len: usize,
    phases: u64,
    /// `phases` rows of `len` coefficients; each row sums to 1 (a constant level passes unchanged).
    rows: Vec<f32>,
    /// The last `len` input samples, written twice in a row: the filter window is always a contiguous slice.
    hist: Vec<f32>,
    head: usize,
    /// How many input samples have been taken, and the index of the next output sample.
    seen: u64,
    next_out: u64,
}

impl Resampler {
    /// # Panics
    /// If `in_rate == 0`: no such device exists, and with zero the stream would never advance.
    pub fn new(in_rate: u32) -> Self {
        assert!(in_rate > 0, "input rate cannot be zero");
        let rate = in_rate as f64;
        let scaled = ((TAPS / 2) as u64 * in_rate as u64).div_ceil(48_000);
        let half = (scaled as usize).clamp(TAPS / 2, MAX_HALF);
        let len = 2 * half + 1;
        // Cutoff no higher than 0.45 of the input rate: at 8–11 kHz the filter works as an interpolator and removes images.
        let fc = CUTOFF_HZ.min(0.45 * rate) / rate;
        let phases = (OUT_RATE as u64 / gcd(in_rate as u64, OUT_RATE as u64)).min(MAX_PHASES);
        let mut rows = Vec::with_capacity(phases as usize * len);
        for p in 0..phases {
            let frac = p as f64 / phases as f64;
            // The window runs from the oldest sample to the newest; the output sample sits `frac` after the newest minus the half-width.
            let row: Vec<f64> = (0..len).map(|m| kernel(frac + half as f64 - m as f64, fc, half as f64)).collect();
            let sum: f64 = row.iter().sum();
            rows.extend(row.iter().map(|c| (c / sum) as f32));
        }
        Self {
            in_rate: in_rate as u64,
            passthrough: in_rate == OUT_RATE,
            len,
            phases,
            rows,
            hist: vec![0.0; 2 * len],
            head: 0,
            seen: 0,
            next_out: 0,
        }
    }

    /// Samples outside ±1 are clamped to full scale; NaN and infinities count as silence.
    pub fn process(&mut self, input: &[f32]) -> Vec<i16> {
        if self.passthrough {
            return input.iter().map(|&v| to_i16(clean(v))).collect();
        }
        let out_rate = OUT_RATE as u64;
        let mut out = Vec::with_capacity((input.len() as u64 * out_rate / self.in_rate) as usize + 2);
        for &x in input {
            let x = clean(x);
            self.hist[self.head] = x;
            self.hist[self.head + self.len] = x;
            self.head = (self.head + 1) % self.len;
            // Output sample n lies at input position n·in/16000; it is ready as soon as the input sample
            // with the index of that position's integer part has arrived.
            loop {
                let pos = self.next_out * self.in_rate;
                if pos / out_rate > self.seen {
                    break;
                }
                let p = (pos % out_rate * self.phases / out_rate) as usize;
                let row = &self.rows[p * self.len..(p + 1) * self.len];
                let window = &self.hist[self.head..self.head + self.len];
                out.push(to_i16(row.iter().zip(window).map(|(c, s)| c * s).sum()));
                self.next_out += 1;
            }
            self.seen += 1;
        }
        out
    }
}

/// Sinc with cutoff `fc` (a fraction of the input rate) under a Blackman window spanning ±`half` input samples.
fn kernel(t: f64, fc: f64, half: f64) -> f64 {
    use std::f64::consts::PI;
    if t.abs() >= half {
        return 0.0;
    }
    let sinc = if t == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * t).sin() / (PI * t) };
    sinc * (0.42 + 0.5 * (PI * t / half).cos() + 0.08 * (2.0 * PI * t / half).cos())
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn clean(v: f32) -> f32 {
    if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

pub fn peak_dbfs(pcm: &[i16]) -> f32 {
    let peak = pcm.iter().map(|s| (*s as i32).unsigned_abs()).max().unwrap_or(0);
    if peak == 0 { -120.0 } else { 20.0 * (peak as f32 / 32768.0).log10() }
}

/// 0…1 for the Drop's level bars: −60 dBFS → 0, −10 dBFS → 1.
pub fn level(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let mean = pcm.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum::<f32>() / pcm.len() as f32;
    if mean <= 0.0 {
        return 0.0;
    }
    let db = 10.0 * mean.log10();
    ((db + 60.0) / 50.0).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    Pending,
    Alive,
    Dead,
}

pub struct SilenceDetector {
    elapsed_ms: u64,
    verdict: Silence,
}

impl SilenceDetector {
    pub fn new() -> Self {
        Self { elapsed_ms: 0, verdict: Silence::Pending }
    }

    /// `Dead` is returned once; after that the result is `Alive`/`Pending` and no longer changes.
    pub fn push(&mut self, peak_dbfs: f32, ms: u64) -> Silence {
        match self.verdict {
            Silence::Pending => {
                // Silence is strictly below −70 dBFS; exactly −70 already counts as sound.
                if peak_dbfs >= DEAD_DBFS {
                    self.verdict = Silence::Alive;
                    return Silence::Alive;
                }
                self.elapsed_ms = self.elapsed_ms.saturating_add(ms);
                if self.elapsed_ms >= DEAD_AFTER_MS {
                    self.verdict = Silence::Dead;
                    return Silence::Dead;
                }
                Silence::Pending
            }
            Silence::Alive => Silence::Alive,
            Silence::Dead => Silence::Pending,
        }
    }
}

impl Default for SilenceDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn sine(rate: u32, hz: f64, secs: f64, amp: f32) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| amp * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin() as f32)
            .collect()
    }

    fn rms(x: &[i16]) -> f64 {
        (x.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn passthrough_at_16k() {
        let mut r = Resampler::new(16_000);
        assert_eq!(r.process(&[0.0, 0.5, -0.5]), vec![0, 16_383, -16_383]);
    }

    #[test]
    fn length_follows_rate() {
        for rate in [48_000u32, 44_100, 32_000] {
            let mut r = Resampler::new(rate);
            let out = r.process(&vec![0.0; rate as usize]);
            assert!((out.len() as i64 - 16_000).abs() <= 2, "{rate}: {}", out.len());
        }
    }

    #[test]
    fn speech_band_kept_and_high_band_cut() {
        let mut r = Resampler::new(48_000);
        let out = r.process(&sine(48_000, 1_000.0, 1.0, 0.5));
        assert!((rms(&out[400..]) - 0.5 / 2f64.sqrt()).abs() < 0.03);
        let mut r = Resampler::new(48_000);
        let out = r.process(&sine(48_000, 12_000.0, 1.0, 0.5));
        assert!(rms(&out[400..]) < 0.03, "12 kHz must be removed by the filter");
    }

    #[test]
    fn chunks_equal_one_shot() {
        let input = sine(44_100, 440.0, 0.5, 0.3);
        let whole = Resampler::new(44_100).process(&input);
        let mut r = Resampler::new(44_100);
        let parts: Vec<i16> = input.chunks(441).flat_map(|c| r.process(c)).collect();
        assert_eq!(whole, parts);
    }

    #[test]
    fn levels() {
        assert_eq!(peak_dbfs(&[0, 0, 0]), -120.0);
        assert!((peak_dbfs(&[16_384]) - (-6.02)).abs() < 0.05);
        assert_eq!(level(&[0; 320]), 0.0);
        assert_eq!(level(&[32_767; 320]), 1.0);
    }

    #[test]
    fn dead_mic_after_four_seconds_of_digital_silence() {
        let mut d = SilenceDetector::new();
        for _ in 0..199 {
            assert_eq!(d.push(-120.0, 20), Silence::Pending);
        }
        assert_eq!(d.push(-120.0, 20), Silence::Dead);
        assert_eq!(d.push(-120.0, 20), Silence::Pending, "Dead is reported once");
        let mut d = SilenceDetector::new();
        d.push(-90.0, 2_000);
        assert_eq!(d.push(-55.0, 20), Silence::Alive, "a quiet room means the microphone is alive");
        assert_eq!(d.push(-120.0, 10_000), Silence::Alive);
    }

    // ---- Below: extra quality and edge-case checks. ----

    /// Rates at which Windows delivers a microphone in shared mode: Bluetooth headsets (8 / 16 kHz),
    /// built-in and USB microphones (44.1 / 48 kHz), audio interfaces (up to 192 kHz).
    const DEVICE_RATES: [u32; 11] =
        [8_000, 11_025, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

    /// Least-squares fit of a `hz` sine to the 16 kHz output: (amplitude, phase, residual RMS).
    /// `n0` is the index of the slice's first sample in the whole stream, so phases of different slices are comparable.
    fn fit(out: &[i16], hz: f64, n0: usize) -> (f64, f64, f64) {
        let w = 2.0 * PI * hz / OUT_RATE as f64;
        let sc = |i: usize| (w * (n0 + i) as f64).sin_cos();
        let (mut ss, mut cc, mut sc_, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (i, &v) in out.iter().enumerate() {
            let (s, c) = sc(i);
            let y = v as f64 / 32768.0;
            ss += s * s;
            cc += c * c;
            sc_ += s * c;
            ys += y * s;
            yc += y * c;
        }
        let det = ss * cc - sc_ * sc_;
        let a = (ys * cc - yc * sc_) / det;
        let b = (yc * ss - ys * sc_) / det;
        let resid = out
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let (s, c) = sc(i);
                (v as f64 / 32768.0 - a * s - b * c).powi(2)
            })
            .sum::<f64>()
            / out.len() as f64;
        (a.hypot(b), b.atan2(a), resid.sqrt())
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn speech_band_is_flat_and_clean_at_every_device_rate() {
        for rate in DEVICE_RATES {
            for hz in [300.0, 1_000.0, 3_000.0] {
                let out = Resampler::new(rate).process(&sine(rate, hz, 0.5, 0.5));
                let (amp, _, resid) = fit(&out[800..], hz, 800);
                let gain = db(amp / 0.5);
                let snr = db(amp / 2f64.sqrt() / resid);
                assert!(gain.abs() < 0.1, "{rate} Hz, tone {hz}: gain {gain:.3} dB");
                assert!(snr > 60.0, "{rate} Hz, tone {hz}: signal/distortion {snr:.1} dB");
            }
        }
    }

    #[test]
    fn content_above_8k_is_cut_before_it_folds_into_speech() {
        for rate in DEVICE_RATES {
            for hz in [10_000.0, 12_000.0, 14_000.0, 20_000.0] {
                if hz > rate as f64 / 2.0 - 500.0 {
                    continue;
                }
                let out = Resampler::new(rate).process(&sine(rate, hz, 0.5, 0.5));
                let down = -db(rms(&out[800..]) / (0.5 / 2f64.sqrt()));
                assert!(down >= 60.0, "{rate} Hz, tone {hz}: attenuation only {down:.1} dB");
            }
        }
    }

    #[test]
    fn length_and_level_follow_at_every_device_rate() {
        for rate in DEVICE_RATES.into_iter().chain([16_000]) {
            let out = Resampler::new(rate).process(&vec![0.1; rate as usize]);
            assert_eq!(out.len(), 16_000, "{rate}");
            // A constant level passes unchanged: 0.1 → 3276 (0.1 · 32767 with the fraction dropped).
            assert!(out[200..].iter().all(|&s| (s - 3_276).abs() <= 1), "{rate}");
        }
    }

    #[test]
    fn random_chunks_equal_one_shot_at_every_device_rate() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |max: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (max + 1)) as usize
        };
        for rate in DEVICE_RATES.into_iter().chain([16_000]) {
            let input = sine(rate, 700.0, 0.3, 0.6);
            let whole = Resampler::new(rate).process(&input);
            let mut r = Resampler::new(rate);
            let mut parts = Vec::new();
            let mut rest = &input[..];
            while !rest.is_empty() {
                let n = next(1_500).min(rest.len());
                parts.extend(r.process(&rest[..n]));
                rest = &rest[n..];
            }
            assert_eq!(whole, parts, "{rate}");
        }
    }

    #[test]
    fn minute_at_44k1_in_10ms_chunks_keeps_length_and_phase() {
        const SECS: usize = 60;
        let input = sine(44_100, 1_000.0, SECS as f64, 0.5);
        let mut r = Resampler::new(44_100);
        let out: Vec<i16> = input.chunks(441).flat_map(|c| r.process(c)).collect();
        assert!((out.len() as i64 - (SECS * 16_000) as i64).abs() <= 1, "{}", out.len());
        // Timing does not drift: the 1 kHz phase is the same in the second and in the last second.
        let (_, first, _) = fit(&out[16_000..32_000], 1_000.0, 16_000);
        let n0 = out.len() - 16_000;
        let (_, last, _) = fit(&out[n0..], 1_000.0, n0);
        let drift = (first - last + PI).rem_euclid(2.0 * PI) - PI;
        assert!(drift.abs() < 1e-3, "phase drift {drift} rad");
    }

    #[test]
    fn odd_and_extreme_rates_stay_sane() {
        // A rate that shares no common divisor with 16 kHz: phases are rounded to 1/1024 of a sample.
        let rate = 37_831;
        let out = Resampler::new(rate).process(&sine(rate, 3_000.0, 0.5, 0.5));
        assert_eq!(out.len(), 8_000);
        let (amp, _, resid) = fit(&out[800..], 3_000.0, 800);
        assert!(db(amp / 0.5).abs() < 0.1 && db(amp / 2f64.sqrt() / resid) > 60.0);
        // Extreme values neither crash nor blow up memory.
        assert_eq!(Resampler::new(1).process(&[0.5; 4]).len(), 64_000);
        assert!(Resampler::new(u32::MAX).process(&[0.5; 1_000]).len() <= 1);
    }

    #[test]
    fn out_of_range_nan_and_inf_never_panic_or_wrap() {
        let mut r = Resampler::new(16_000);
        assert_eq!(
            r.process(&[1.5, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX, f32::MIN, 1.0, -1.0]),
            vec![32_767, -32_767, 0, 0, 0, 32_767, -32_767, 32_767, -32_767]
        );
        for rate in [44_100u32, 48_000, 96_000] {
            // A flat 0.25 with rare NaN/inf: the output stays near 0.25, with no drops to zero and no clicks.
            let mut input = vec![0.25f32; rate as usize / 2];
            for (i, v) in input.iter_mut().enumerate().skip(1_000).step_by(997) {
                *v = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY][i % 3];
            }
            let out = Resampler::new(rate).process(&input);
            let (lo, hi) = (out[100..].iter().min().unwrap(), out[100..].iter().max().unwrap());
            assert!(*lo >= 4_000 && *hi <= 9_000, "{rate}: {lo}…{hi}");
            // Overload hits full scale and does not wrap around to the other sign.
            let out = Resampler::new(rate).process(&vec![1e9; rate as usize / 10]);
            assert!(out[100..].iter().all(|&s| s >= 32_766), "{rate}");
            let square: Vec<f32> = (0..rate as usize / 2).map(|i| if i / 200 % 2 == 0 { 3.0 } else { -3.0 }).collect();
            let out = Resampler::new(rate).process(&square);
            assert_eq!((out.iter().min(), out.iter().max()), (Some(&-32_767), Some(&32_767)), "{rate}");
        }
    }

    #[test]
    fn peak_and_level_edges() {
        assert_eq!(peak_dbfs(&[]), -120.0);
        assert_eq!(peak_dbfs(&[-32_768]), 0.0);
        // 10 units is still digital silence (−70.3 dBFS), 11 is already sound (−69.5 dBFS).
        assert!(peak_dbfs(&[0, -10, 3]) < DEAD_DBFS && peak_dbfs(&[0, 11, 3]) > DEAD_DBFS);
        assert_eq!(level(&[]), 0.0);
        let at = |dbfs: f32| vec![(32_768.0 * 10f32.powf(dbfs / 20.0)).round() as i16; 320];
        assert_eq!(level(&at(-80.0)), 0.0);
        assert!(level(&at(-60.0)) < 0.01);
        assert!((level(&at(-35.0)) - 0.5).abs() < 0.01);
        assert!(level(&at(-10.0)) > 0.99);
    }

    #[test]
    fn dead_mic_boundaries() {
        // 3999 ms of silence: still waiting; 4000 ms: silent.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-120.0, 3_999), Silence::Pending);
        assert_eq!(d.push(-120.0, 1), Silence::Dead);
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-120.0, 4_000), Silence::Dead);
        // Silence is "below −70 dBFS": exactly −70 is already sound, just below is still silence.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-70.0, 20), Silence::Alive);
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-70.01, 3_999), Silence::Pending);
        assert_eq!(d.push(-70.01, 1), Silence::Dead);
        // An empty frame does not advance the count; a huge one does not overflow it.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-120.0, 0), Silence::Pending);
        assert_eq!(d.push(-120.0, 3_999), Silence::Pending);
        assert_eq!(d.push(-120.0, u64::MAX), Silence::Dead);
        assert_eq!(d.push(-120.0, u64::MAX), Silence::Pending);
    }

    #[test]
    fn dead_only_from_continuous_silence_at_start() {
        // 3.9 s of silence, then sound: alive; after that a quiet room or a pause of any length does not change it.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-120.0, 3_900), Silence::Pending);
        assert_eq!(d.push(-40.0, 20), Silence::Alive);
        for _ in 0..1_000 {
            assert_eq!(d.push(-120.0, 20), Silence::Alive);
        }
        // Sound in the very first frame: alive at once.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-20.0, 20), Silence::Alive);
        assert_eq!(d.push(-120.0, 60_000), Silence::Alive);
        // A quiet room (−65 dBFS) is not digital silence.
        let mut d = SilenceDetector::new();
        assert_eq!(d.push(-65.0, 20), Silence::Alive);
    }
}
