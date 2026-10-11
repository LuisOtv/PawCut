#![allow(dead_code)]

use std::path::Path;

use pawcut_core::*;
use pawcut_media::{MediaLibrary, RgbaImage, probe};
use pawcut_render::Renderer;

pub const W: u32 = 320;
pub const H: u32 = 180;
pub const RED: Rgba = Rgba::new(255, 0, 0, 255);
pub const BLUE: Rgba = Rgba::new(0, 0, 255, 255);
pub const WHITE: Rgba = Rgba::new(255, 255, 255, 255);

pub fn editor() -> Editor {
    Editor::new(Project::new(ProjectSettings {
        width: W,
        height: H,
        ..ProjectSettings::default()
    }))
}

pub fn renderer() -> Renderer {
    Renderer::new(MediaLibrary::new((W, H), 48_000, None))
}

pub fn track(ed: &mut Editor, kind: TrackKind, n: usize) -> TrackId {
    while ed.project().tracks(kind).len() <= n {
        ed.apply(Edit::AddTrack(kind)).unwrap();
    }
    ed.project().tracks(kind)[n].id
}

pub fn put(
    ed: &mut Editor,
    kind: TrackKind,
    n: usize,
    source: ClipSource,
    at: Frame,
    start: f64,
    dur: Frame,
) -> ClipId {
    let t = track(ed, kind, n);
    ed.apply(Edit::Overwrite {
        at,
        items: vec![Placement {
            track: t,
            draft: ClipDraft {
                source,
                source_start: start,
                duration: dur,
            },
        }],
    })
    .unwrap()
    .clips[0]
}

pub fn solid(ed: &mut Editor, n: usize, color: Rgba, at: Frame, dur: Frame) -> ClipId {
    put(
        ed,
        TrackKind::Video,
        n,
        ClipSource::SolidColor(color),
        at,
        0.0,
        dur,
    )
}

pub fn media(ed: &mut Editor, path: &Path) -> MediaId {
    let info = probe(path, ed.project().settings.fps).unwrap();
    ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap()
}

pub fn set(ed: &mut Editor, clip: ClipId, change: ClipChange) {
    ed.apply(Edit::SetClip { clip, change }).unwrap();
}

pub fn px(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    img.get_pixel(x, y).0
}

pub fn near(p: [u8; 4], rgb: [u8; 3], tol: u8) -> bool {
    (0..3).all(|i| p[i].abs_diff(rgb[i]) <= tol)
}
