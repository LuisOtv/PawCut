//! Análise de áudio dos clipes: pico (para normalizar) e trechos de silêncio.

pub use paw_audio::dsp::{integrated_lufs, loudness_gain, normalize_gain, silent_runs};
use paw_media::{Pcm, default_cache_dir, load_audio};
use pawcut_core::{Clip, ClipSource, Frame, Project};

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
