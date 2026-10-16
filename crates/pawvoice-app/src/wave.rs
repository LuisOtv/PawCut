//! Forma de onda: picos pré-calculados para desenhar rápido em qualquer zoom.

/// Amostras por balde de pico.
pub const BUCKET: usize = 256;

/// (mín, máx) de cada balde de `BUCKET` amostras.
pub fn peaks(s: &[f32]) -> Vec<(f32, f32)> {
    s.chunks(BUCKET)
        .map(|c| {
            c.iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
        })
        .collect()
}

/// (mín, máx) do intervalo de amostras `[a, b)`, usando os baldes quando o intervalo é grande.
pub fn range_minmax(samples: &[f32], peaks: &[(f32, f32)], a: usize, b: usize) -> (f32, f32) {
    let b = b.min(samples.len());
    if a >= b {
        return (0.0, 0.0);
    }
    if b - a >= BUCKET * 2 {
        let (pa, pb) = (a / BUCKET, (b / BUCKET).min(peaks.len()));
        peaks[pa..pb.max(pa + 1).min(peaks.len())]
            .iter()
            .fold((0.0, 0.0), |(lo, hi), (l, h)| {
                (f32::min(lo, *l), f32::max(hi, *h))
            })
    } else {
        samples[a..b]
            .iter()
            .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minmax_matches_brute_force() {
        let s: Vec<f32> = (0..5000)
            .map(|i| ((i * 37) % 101) as f32 / 50.0 - 1.0)
            .collect();
        let p = peaks(&s);
        for (a, b) in [(0, 10), (100, 3000), (4000, 5000), (1, 2)] {
            let brute = s[a..b]
                .iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            let fast = range_minmax(&s, &p, a, b);
            assert!(fast.0 <= brute.0 + 1e-6 && fast.1 >= brute.1 - 1e-6);
        }
    }
}

/// Espectrograma de um canal: `data[coluna * bins + faixa]`, 0 = silêncio, 255 = 0 dBFS.
pub struct Spec {
    pub cols: usize,
    pub bins: usize,
    /// Amostras entre uma coluna e a próxima.
    pub hop: usize,
    pub data: Vec<u8>,
}

const FFT_N: usize = 1024;
/// Limite de colunas (cabe numa textura e em memória mesmo para gravações longas).
const MAX_COLS: usize = 8192;
/// Até onde vai o eixo de frequência: a voz mora abaixo disso.
pub const SPEC_MAX_HZ: f32 = 8000.0;

/// Calcula o espectrograma de `s` (janela de Hann, 0 Hz até `SPEC_MAX_HZ`, −80 a 0 dB).
pub fn spectrogram(s: &[f32], rate: u32) -> Spec {
    use rustfft::{FftPlanner, num_complex::Complex};
    let bins = ((SPEC_MAX_HZ * FFT_N as f32 / rate as f32) as usize).clamp(8, FFT_N / 2);
    let hop = (FFT_N / 2).max(s.len().div_ceil(MAX_COLS));
    let cols = s.len().div_ceil(hop).max(1);
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_N);
    let window: Vec<f32> = (0..FFT_N)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_N as f32).cos())
        .collect();
    let mut data = Vec::with_capacity(cols * bins);
    let mut buf = vec![Complex::new(0.0, 0.0); FFT_N];
    for col in 0..cols {
        // Janela centrada no meio do intervalo da coluna.
        let start = (col * hop + hop / 2) as isize - (FFT_N / 2) as isize;
        for (i, b) in buf.iter_mut().enumerate() {
            let v = usize::try_from(start + i as isize)
                .ok()
                .and_then(|k| s.get(k))
                .copied()
                .unwrap_or(0.0);
            *b = Complex::new(v * window[i], 0.0);
        }
        fft.process(&mut buf);
        data.extend(buf[..bins].iter().map(|c| {
            // Senoide de amplitude 1 dá |X| = N/4 com janela de Hann.
            let mag = c.norm() / (FFT_N as f32 / 4.0);
            let db = 20.0 * mag.max(1e-6).log10();
            ((db + 80.0) / 80.0 * 255.0).clamp(0.0, 255.0) as u8
        }));
    }
    Spec {
        cols,
        bins,
        hop,
        data,
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    #[test]
    fn a_tone_lights_up_its_own_band() {
        let rate = 48_000;
        let s: Vec<f32> = (0..rate)
            .map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / rate as f32).sin())
            .collect();
        let spec = spectrogram(&s, rate);
        assert_eq!(spec.bins, 170);
        let col = &spec.data[spec.cols / 2 * spec.bins..][..spec.bins];
        let (peak, v) = col.iter().enumerate().max_by_key(|(_, v)| **v).unwrap();
        // 1 kHz cai no bin 1000 · 1024 / 48000 ≈ 21
        assert!((20..=22).contains(&peak), "pico no bin {peak}");
        assert!(*v > 230, "tom cheio devia ficar perto de 0 dB: {v}");
        assert!(col[100] < 60, "longe do tom devia ficar escuro");
    }

    #[test]
    fn long_audio_is_capped_and_silence_is_black() {
        let spec = spectrogram(&vec![0.0; 48_000 * 600], 48_000);
        assert!(spec.cols <= 8192);
        assert!(spec.data.iter().all(|v| *v == 0));
        assert_eq!(spec.data.len(), spec.cols * spec.bins);
    }
}
