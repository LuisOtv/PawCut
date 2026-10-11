//! Mixagem das faixas de áudio da timeline.

use pawcut_core::{ClipSource, Project};
use pawcut_media::MediaLibrary;

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
    for track in &project.audio_tracks {
        if (any_solo && !track.solo) || (!any_solo && track.muted) {
            continue;
        }
        for clip in &track.clips {
            let s0 = fps.frame_to_sample(clip.start, sr);
            let s1 = fps.frame_to_sample(clip.end(), sr);
            let (a, b) = (s0.max(start), s1.min(end));
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
            // O PCM já está na velocidade do clipe: o ponto de entrada também encolhe/estica.
            let src0 = (clip.source_start / fps.as_f64() * sr as f64 / clip.speed).round() as i64;
            let gain = 10f32.powf(clip.volume_db / 20.0);
            let fade_in = fps.frame_to_sample(clip.fade_in, sr);
            let fade_out = fps.frame_to_sample(clip.fade_out, sr);
            for n in a..b {
                let mut g = gain;
                if fade_in > 0 {
                    g *= ((n - s0) as f32 / fade_in as f32).min(1.0);
                }
                if fade_out > 0 {
                    g *= ((s1 - 1 - n) as f32 / fade_out as f32).min(1.0);
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
