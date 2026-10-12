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
