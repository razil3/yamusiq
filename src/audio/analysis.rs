//! Spectrum and waveform analysis of the samples the listener is hearing.

use std::sync::Arc;

use rustfft::{num_complex::Complex, Fft, FftPlanner};

use crate::player::Analysis;

/// Analysis window in samples (≈ 43 ms at 48 kHz).
pub const WINDOW: usize = 2048;
/// FFT length: the window is zero-padded 4× so low bands, which are narrower
/// than one raw bin, are resolved by interpolation instead of smearing.
const FFT_LEN: usize = WINDOW * 4;
/// Samples handed to the oscilloscope.
pub const WAVE_LEN: usize = 640;

const LOW_HZ: f32 = 30.0;
const HIGH_HZ: f32 = 16_000.0;
/// dB range mapped onto 0..=1.
const FLOOR_DB: f32 = -54.0;
const RANGE_DB: f32 = 40.0;
/// Spectral tilt so music (which falls ~3 dB/octave) reads evenly.
const TILT_DB_PER_OCTAVE: f32 = 3.0;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
}

impl Default for Analyzer {
    fn default() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_LEN);
        let window = (0..WINDOW)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32).cos())
            .collect();
        Self { fft, window }
    }
}

impl Analyzer {
    /// `samples` are the most recent mono samples, oldest first (at least
    /// `WINDOW` of them for a full-resolution spectrum).
    pub fn analyze(&self, samples: &[f32], rate: u32, bands: usize) -> Analysis {
        let n = WINDOW;
        let tail = &samples[samples.len().saturating_sub(n)..];
        // Left-pad with silence when fewer than WINDOW samples are available;
        // everything past the window is zero padding.
        let pad = n - tail.len();
        let mut buf: Vec<Complex<f32>> = (0..FFT_LEN)
            .map(|i| {
                let s = if i < pad || i >= n { 0.0 } else { tail[i - pad] };
                Complex::new(if i < n { s * self.window[i] } else { 0.0 }, 0.0)
            })
            .collect();
        self.fft.process(&mut buf);

        // Amplitude of a full-scale sine with a Hann window ≈ window/4.
        let mags: Vec<f32> = buf[..FFT_LEN / 2].iter().map(|c| c.norm() * 4.0 / n as f32).collect();
        let bin_hz = rate as f32 / FFT_LEN as f32;
        let high = HIGH_HZ.min(rate as f32 / 2.0 - bin_hz);
        let ratio = high / LOW_HZ;
        let band_levels = (0..bands)
            .map(|b| {
                let lo = LOW_HZ * ratio.powf(b as f32 / bands as f32);
                let hi = LOW_HZ * ratio.powf((b + 1) as f32 / bands as f32);
                let (k0, k1) = ((lo / bin_hz).floor() as usize, (hi / bin_hz).ceil() as usize);
                let k1 = k1.clamp(k0 + 1, mags.len());
                // Peak within the band; narrow low bands use the nearest bin.
                let mag = mags[k0.min(mags.len() - 1)..k1].iter().cloned().fold(0.0, f32::max);
                let centre = (lo * hi).sqrt();
                let db = 20.0 * (mag + 1e-9).log10() + TILT_DB_PER_OCTAVE * (centre / 1000.0).log2();
                ((db - FLOOR_DB) / RANGE_DB).clamp(0.0, 1.0)
            })
            .collect();

        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len().max(1) as f32).sqrt();
        let level = ((20.0 * (rms + 1e-9).log10() + 60.0) / 60.0).clamp(0.0, 1.0);

        Analysis { bands: band_levels, wave: triggered(samples, WAVE_LEN), level }
    }
}

/// Latest `len` samples starting at a rising zero crossing, so a periodic
/// signal stands still on screen like on a triggered scope.
pub fn triggered(samples: &[f32], len: usize) -> Vec<f32> {
    if samples.len() <= len {
        return samples.to_vec();
    }
    let latest_start = samples.len() - len;
    // Search backwards over at most one window for a rising edge.
    let earliest = latest_start.saturating_sub(len);
    let start = (earliest.max(1)..=latest_start)
        .rev()
        .find(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .unwrap_or(latest_start);
    samples[start..start + len].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, len: usize, amp: f32) -> Vec<f32> {
        (0..len).map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin()).collect()
    }

    #[test]
    fn sine_peaks_in_the_right_band() {
        let a = Analyzer::default();
        for freq in [60.0, 440.0, 5000.0] {
            let r = a.analyze(&sine(freq, 48_000, 4096, 0.5), 48_000, 48);
            let peak = r.bands.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
            let ratio = (HIGH_HZ.min(24_000.0 - 48_000.0 / FFT_LEN as f32)) / LOW_HZ;
            let lo = LOW_HZ * ratio.powf(peak as f32 / 48.0);
            let hi = LOW_HZ * ratio.powf((peak + 1) as f32 / 48.0);
            assert!(lo * 0.8 <= freq && freq <= hi * 1.25, "{freq} Hz landed in band {peak} ({lo:.0}–{hi:.0} Hz)");
            assert!(r.bands[peak] > 0.6, "a −6 dBFS tone should read loud");
        }
    }

    #[test]
    fn silence_is_flat_and_quiet() {
        let r = Analyzer::default().analyze(&vec![0.0; 4096], 48_000, 48);
        assert!(r.bands.iter().all(|&b| b == 0.0));
        assert_eq!(r.level, 0.0);
    }

    #[test]
    fn short_input_is_padded() {
        let r = Analyzer::default().analyze(&sine(440.0, 48_000, 300, 0.5), 48_000, 16);
        assert_eq!(r.bands.len(), 16);
        assert!(r.level > 0.5);
    }

    #[test]
    fn trigger_starts_on_rising_zero_crossing() {
        let s = sine(100.0, 48_000, 6000, 1.0);
        let w = triggered(&s, WAVE_LEN);
        assert_eq!(w.len(), WAVE_LEN);
        assert!(w[0] >= 0.0 && w[0] < 0.05, "starts at the crossing");
        assert!(w[1] > w[0], "rising");
    }
}
