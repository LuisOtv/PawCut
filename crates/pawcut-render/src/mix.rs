//! Mixagem das faixas de áudio da timeline.

use pawcut_core::{Clip, ClipSource, Frame, Param, Project};
use pawcut_media::MediaLibrary;

/// Janela `[início, fim)` em frames.
type Window = Option<(Frame, Frame)>;

/// Trecho em que o clipe soa, já contando os dissolves: `(início, fim)` em frames,
/// mais a janela do dissolve de entrada e a do de saída (do clipe seguinte).
fn audible(
    clips: &[Clip],
    i: usize,
) -> (Frame, Frame, Window, Window) {
    let c = &clips[i];
    let into = c
        .dissolve_window()
        .filter(|_| i > 0 && clips[i - 1].end() == c.start);
    let out_of = clips
        .get(i + 1)
        .filter(|n| n.start == c.end())
        .and_then(|n| n.dissolve_window());
    let start = into.map_or(c.start, |w| w.0);
    let end = out_of.map_or(c.end(), |w| w.1);
    (start, end, into, out_of)
}

/// `frames` frames estéreo a partir da amostra `start` (na taxa do projeto), intercalados L/R.
pub(crate) fn mix(
    project: &Project,
    media: &mut MediaLibrary,
    start: i64,
    frames: usize,
) -> Vec<f32> {
    let sr = project.settings.sample_rate;
    let fps = project.settings.fps;
    let mut out = vec![0.0f32; frames * 2];
    let end = start + frames as i64;
    let any_solo = project.audio_tracks.iter().any(|t| t.solo);
    let to_sample = |f: Frame| fps.frame_to_sample(f, sr);
    for track in &project.audio_tracks {
        if (any_solo && !track.solo) || (!any_solo && track.muted) {
            continue;
        }
        for (i, clip) in track.clips.iter().enumerate() {
            let (f0, f1, into, out_of) = audible(&track.clips, i);
            let (a, b) = (to_sample(f0).max(start), to_sample(f1).min(end));
            if a >= b {
                continue;
            }
            let ClipSource::Media(id) = &clip.source else {
                continue;
            };
            let Some(m) = project.media(*id) else {
                continue;
            };
            let Ok(pcm) = media.audio(&m.info.path, clip.speed) else {
                continue;
            };
            let s0 = to_sample(clip.start);
            let s1 = to_sample(clip.end());
            // O PCM já está na velocidade do clipe: o ponto de entrada também encolhe/estica.
            let src0 = (clip.source_start / fps.as_f64() * sr as f64 / clip.speed).round() as i64;
            let fade_in = to_sample(clip.fade_in);
            let fade_out = to_sample(clip.fade_out);
            let animated = clip
                .keyframes
                .get(&Param::VolumeDb)
                .is_some_and(|k| !k.is_empty());
            let fixed_gain = 10f32.powf(clip.volume_db / 20.0);
            let samples_per_frame = sr as f64 / fps.as_f64();
            let window = |w: Window| w.map(|(x, y)| (to_sample(x), to_sample(y)));
            let (into, out_of) = (window(into), window(out_of));
            for n in a..b {
                let mut g = if animated {
                    let source =
                        clip.source_start + (n - s0) as f64 / samples_per_frame * clip.speed;
                    10f32.powf(clip.param_at_source(Param::VolumeDb, source) / 20.0)
                } else {
                    fixed_gain
                };
                if fade_in > 0 {
                    g *= ((n - s0) as f32 / fade_in as f32).clamp(0.0, 1.0);
                }
                if fade_out > 0 {
                    g *= ((s1 - 1 - n) as f32 / fade_out as f32).clamp(0.0, 1.0);
                }
                // Crossfade de potência constante nos dissolves.
                if let Some((x, y)) = into
                    && n < y
                {
                    let t = ((n - x) as f32 + 0.5) / (y - x) as f32;
                    g *= (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin();
                }
                if let Some((x, y)) = out_of
                    && n >= x
                {
                    let t = ((n - x) as f32 + 0.5) / (y - x) as f32;
                    g *= (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).cos();
                }
                let [l, r] = pcm.frame(src0 + (n - s0));
                let o = ((n - start) * 2) as usize;
                out[o] += l * g;
                out[o + 1] += r * g;
            }
        }
    }
    for v in &mut out {
        *v = v.clamp(-1.0, 1.0);
    }
    out
}
