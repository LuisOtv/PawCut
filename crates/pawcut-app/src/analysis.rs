//! Análise de áudio dos clipes: pico (para normalizar) e trechos de silêncio.

use pawcut_core::{Clip, ClipSource, Frame, Project};
use pawcut_media::{Pcm, default_cache_dir, load_audio};

/// PCM do clipe (já na velocidade dele) e o intervalo de amostras que aparece na timeline.
fn clip_pcm(project: &Project, clip: &Clip) -> Option<(Pcm, i64, i64)> {
    let ClipSource::Media(id) = &clip.source else {
        return None;
    };
    let media = project.media(*id)?;
    let s = &project.settings;
    let pcm = load_audio(
        &media.info.path,
        s.sample_rate,
        clip.speed,
        default_cache_dir().as_deref(),
    )
    .ok()?;
    let src0 =
        (clip.source_start / s.fps.as_f64() * s.sample_rate as f64 / clip.speed).round() as i64;
    let len = s.fps.frame_to_sample(clip.end(), s.sample_rate)
        - s.fps.frame_to_sample(clip.start, s.sample_rate);
    Some((pcm, src0, src0 + len))
}

/// Pico absoluto do trecho do clipe (0..1).
pub fn clip_peak(project: &Project, clip: &Clip) -> Option<f32> {
    let (pcm, a, b) = clip_pcm(project, clip)?;
    Some(
        (a..b)
            .map(|i| pcm.frame(i))
            .fold(0.0f32, |m, [l, r]| m.max(l.abs()).max(r.abs())),
    )
}

/// Filtro biquad (forma direta I).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn run(&mut self, v: f64) -> f64 {
        let out = self.b[0] * v + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [v, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

/// Ponderação K da ITU-R BS.1770 (shelf de agudos + passa-altas) para a taxa `fs`.
fn k_weighting(fs: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    [shelf, high_pass]
}

/// Loudness integrado (LUFS, EBU R128) de áudio estéreo intercalado: blocos de 400 ms
/// a cada 100 ms, porta absoluta em −70 LUFS e relativa 10 LU abaixo.
pub fn integrated_lufs(samples: &[f32], sample_rate: u32) -> Option<f32> {
    let frames = samples.len() / 2;
    let mut filters = [
        k_weighting(sample_rate as f64),
        k_weighting(sample_rate as f64),
    ];
    // Energia ponderada de cada janela de 100 ms (somando os dois canais).
    let hop = (sample_rate as usize / 10).max(1);
    let mut hops = Vec::with_capacity(frames / hop + 1);
    let mut acc = 0.0f64;
    for i in 0..frames {
        for (ch, f) in filters.iter_mut().enumerate() {
            let mut v = samples[2 * i + ch] as f64;
            for stage in f.iter_mut() {
                v = stage.run(v);
            }
            acc += v * v;
        }
        if (i + 1) % hop == 0 {
            hops.push(acc / hop as f64);
            acc = 0.0;
        }
    }
    let blocks: Vec<f64> = hops
        .windows(4)
        .map(|w| w.iter().sum::<f64>() / 4.0)
        .collect();
    let loudness = |z: f64| -0.691 + 10.0 * z.log10();
    let above = |gate: f64| -> Vec<f64> {
        blocks
            .iter()
            .copied()
            .filter(|z| *z > 0.0 && loudness(*z) > gate)
            .collect()
    };
    let abs = above(-70.0);
    if abs.is_empty() {
        return None;
    }
    let rel = loudness(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let gated = above(rel);
    let z = gated.iter().sum::<f64>() / gated.len().max(1) as f64;
    Some(loudness(z) as f32)
}

/// Loudness (LUFS) e pico (0..1) do trecho do clipe que aparece na timeline.
pub fn clip_loudness(project: &Project, clip: &Clip) -> Option<(f32, f32)> {
    let (pcm, a, b) = clip_pcm(project, clip)?;
    let mut buf = Vec::with_capacity(((b - a).max(0) * 2) as usize);
    let mut peak = 0.0f32;
    for i in a..b {
        let [l, r] = pcm.frame(i);
        peak = peak.max(l.abs()).max(r.abs());
        buf.extend([l, r]);
    }
    Some((integrated_lufs(&buf, project.settings.sample_rate)?, peak))
}

/// Ganho (dB) para chegar em `target` LUFS sem o pico passar de −1 dBFS.
pub fn loudness_gain(lufs: f32, peak: f32, target: f32) -> f32 {
    let want = target - lufs;
    let headroom = if peak > 1e-6 {
        -1.0 - 20.0 * peak.log10()
    } else {
        24.0
    };
    want.min(headroom).clamp(-96.0, 24.0)
}

/// Volume (dB) que leva o pico do clipe a `target_db` (ex.: −1 dBFS).
pub fn normalize_gain(peak: f32, target_db: f32) -> Option<f32> {
    (peak > 1e-6).then(|| (target_db - 20.0 * peak.log10()).clamp(-96.0, 24.0))
}

/// Trechos de silêncio de `samples` (estéreo intercalado) em índices de frame estéreo:
/// abaixo de `threshold_db` por pelo menos `min_len`, encolhidos `pad` em cada ponta.
pub fn silent_runs(
    samples: &[f32],
    sample_rate: u32,
    threshold_db: f32,
    min_len: f64,
    pad: f64,
) -> Vec<(usize, usize)> {
    let window = (sample_rate as usize / 100).max(1);
    let frames = samples.len() / 2;
    let threshold = 10f32.powf(threshold_db / 20.0);
    let mut runs = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut w = 0;
    while w < frames {
        let end = (w + window).min(frames);
        let energy: f32 = (w..end)
            .map(|i| {
                (samples[2 * i] * samples[2 * i] + samples[2 * i + 1] * samples[2 * i + 1]) / 2.0
            })
            .sum();
        let rms = (energy / (end - w) as f32).sqrt();
        match (rms < threshold, run_start) {
            (true, None) => run_start = Some(w),
            (false, Some(s)) => {
                runs.push((s, w));
                run_start = None;
            }
            _ => {}
        }
        w = end;
    }
    if let Some(s) = run_start {
        runs.push((s, frames));
    }
    let min = (min_len * sample_rate as f64) as usize;
    let pad = (pad * sample_rate as f64) as usize;
    runs.into_iter()
        .filter(|(a, b)| b - a >= min)
        .map(|(a, b)| {
            (
                a + if a == 0 { 0 } else { pad },
                b - if b == frames { 0 } else { pad },
            )
        })
        .filter(|(a, b)| a < b)
        .collect()
}

/// Silêncios do clipe em frames da timeline.
pub fn clip_silences(
    project: &Project,
    clip: &Clip,
    threshold_db: f32,
    min_len: f64,
    pad: f64,
) -> Vec<(Frame, Frame)> {
    let Some((pcm, a, b)) = clip_pcm(project, clip) else {
        return Vec::new();
    };
    let samples: Vec<f32> = (a..b).flat_map(|i| pcm.frame(i)).collect();
    let s = &project.settings;
    let to_frame = |i: usize| clip.start + s.fps.seconds_to_frames(i as f64 / s.sample_rate as f64);
    silent_runs(&samples, s.sample_rate, threshold_db, min_len, pad)
        .into_iter()
        .map(|(x, y)| (to_frame(x), to_frame(y)))
        .filter(|(x, y)| x < y)
        .collect()
}

#[cfg(test)]
mod tests {

    fn sine(amp: f32, secs: f32) -> Vec<f32> {
        let sr = 48_000.0;
        (0..(sr * secs) as usize)
            .flat_map(|i| {
                let v = amp * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn lufs_of_a_reference_sine() {
        // Seno de 1 kHz a −23 dBFS nos dois canais mede −23 LUFS (EBU R128).
        let amp = 10f32.powf(-23.0 / 20.0);
        let l = integrated_lufs(&sine(amp, 3.0), 48_000).unwrap();
        assert!((l + 23.0).abs() < 0.3, "{l}");
        // Silêncio fica abaixo da porta.
        assert_eq!(integrated_lufs(&vec![0.0; 96_000], 48_000), None);
        // O ganho respeita o pico: −23 → −14 LUFS pediria +9 dB, mas o pico −23 dBFS deixa +22.
        assert!((loudness_gain(-23.0, amp, -14.0) - 9.0).abs() < 1e-4);
        assert!((loudness_gain(-23.0, 0.9, -14.0) - (-1.0 - 20.0 * 0.9f32.log10())).abs() < 1e-4);
    }

    use super::*;

    fn tone(secs: f64, level: f32, sr: u32) -> Vec<f32> {
        (0..(secs * sr as f64) as usize)
            .flat_map(|i| {
                let v = level * (i as f32 * 0.07).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn finds_long_silences_and_keeps_short_pauses() {
        let sr = 48_000;
        let mut s = tone(1.0, 0.5, sr);
        s.extend(tone(1.0, 0.0, sr)); // silêncio de 1 s
        s.extend(tone(0.5, 0.5, sr));
        s.extend(tone(0.2, 0.0, sr)); // pausa curta: fica
        s.extend(tone(0.5, 0.5, sr));
        let runs = silent_runs(&s, sr, -40.0, 0.4, 0.1);
        assert_eq!(runs.len(), 1);
        let (a, b) = runs[0];
        assert!((a as f64 / sr as f64 - 1.1).abs() < 0.02, "início {a}");
        assert!((b as f64 / sr as f64 - 1.9).abs() < 0.02, "fim {b}");
    }

    #[test]
    fn normalize_brings_peak_to_target() {
        let g = normalize_gain(0.25, -1.0).unwrap();
        assert!((g - (-1.0 - 20.0 * 0.25f32.log10())).abs() < 1e-4);
        assert!(normalize_gain(0.0, -1.0).is_none());
    }
}
