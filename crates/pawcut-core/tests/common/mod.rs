#![allow(dead_code)]

use pawcut_core::*;

pub fn new_editor() -> Editor {
    Editor::new(Project::new(ProjectSettings::default()))
}

pub fn add_video(ed: &mut Editor, frames: Frame, has_audio: bool) -> MediaId {
    ed.apply(Edit::AddMedia(MediaInfo {
        path: "/videos/teste.mp4".into(),
        kind: MediaKind::Video { has_audio },
        duration: Some(frames),
        width: 1920,
        height: 1080,
    }))
    .unwrap()
    .media
    .unwrap()
}

pub fn add_audio(ed: &mut Editor, frames: Frame) -> MediaId {
    ed.apply(Edit::AddMedia(MediaInfo {
        path: "/musicas/teste.mp3".into(),
        kind: MediaKind::Audio,
        duration: Some(frames),
        width: 0,
        height: 0,
    }))
    .unwrap()
    .media
    .unwrap()
}

pub fn v(ed: &Editor, n: usize) -> TrackId {
    ed.project().video_tracks[n].id
}

pub fn a(ed: &Editor, n: usize) -> TrackId {
    ed.project().audio_tracks[n].id
}

pub fn solid() -> ClipSource {
    ClipSource::SolidColor(Rgba::new(255, 0, 0, 255))
}

pub fn draft(source: ClipSource, source_start: f64, duration: Frame) -> ClipDraft {
    ClipDraft {
        source,
        source_start,
        duration,
    }
}

/// Overwrite de um único clipe. Devolve o id criado.
pub fn place(
    ed: &mut Editor,
    track: TrackId,
    source: ClipSource,
    at: Frame,
    source_start: f64,
    duration: Frame,
) -> ClipId {
    ed.apply(Edit::Overwrite {
        at,
        items: vec![Placement {
            track,
            draft: draft(source, source_start, duration),
        }],
    })
    .unwrap()
    .clips[0]
}

/// Coloca o vídeo em V1 e o áudio em A1, ligados. Devolve (vídeo, áudio).
pub fn place_av(
    ed: &mut Editor,
    media: MediaId,
    at: Frame,
    source_start: f64,
    duration: Frame,
) -> (ClipId, ClipId) {
    let (vt, at_) = (v(ed, 0), a(ed, 0));
    let out = ed
        .apply(Edit::Overwrite {
            at,
            items: vec![
                Placement {
                    track: vt,
                    draft: draft(ClipSource::Media(media), source_start, duration),
                },
                Placement {
                    track: at_,
                    draft: draft(ClipSource::Media(media), source_start, duration),
                },
            ],
        })
        .unwrap();
    (out.clips[0], out.clips[1])
}

pub fn spans(ed: &Editor, track: TrackId) -> Vec<(Frame, Frame)> {
    ed.project()
        .track(track)
        .unwrap()
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect()
}

pub fn clip(ed: &Editor, id: ClipId) -> Clip {
    ed.project().clip(id).unwrap().clone()
}

pub fn lock(ed: &mut Editor, track: TrackId) {
    ed.apply(Edit::SetTrackFlags {
        track,
        flags: TrackFlags {
            locked: true,
            enabled: true,
            muted: false,
            solo: false,
        },
    })
    .unwrap();
}
