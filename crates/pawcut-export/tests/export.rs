use std::sync::atomic::AtomicBool;

use pawcut_core::*;
use pawcut_export::{ExportError, ExportSettings, export};
use pawcut_media::{MediaLibrary, probe};
use pawcut_render::Renderer;
use pawcut_testkit as kit;

const W: u32 = 320;
const H: u32 = 180;

fn editor() -> Editor {
    Editor::new(Project::new(ProjectSettings {
        width: W,
        height: H,
        ..ProjectSettings::default()
    }))
}

fn renderer() -> Renderer {
    Renderer::new(MediaLibrary::new((W, H), 48_000, None))
}

fn put(ed: &mut Editor, kind: TrackKind, source: ClipSource, at: Frame, dur: Frame) -> ClipId {
    let t = ed.project().tracks(kind)[0].id;
    ed.apply(Edit::Overwrite {
        at,
        items: vec![Placement {
            track: t,
            draft: ClipDraft {
                source,
                source_start: 0.0,
                duration: dur,
            },
        }],
    })
    .unwrap()
    .clips[0]
}

fn add_media(ed: &mut Editor, path: &std::path::Path) -> MediaId {
    let info = probe(path, ed.project().settings.fps).unwrap();
    ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap()
}

fn run(ed: &Editor, out: &std::path::Path, cancel: bool) -> Result<Vec<Frame>, ExportError> {
    let mut seen = Vec::new();
    let flag = AtomicBool::new(cancel);
    export(
        ed.project(),
        &mut renderer(),
        &ExportSettings {
            output: out.to_path_buf(),
            crf: 20,
        },
        &flag,
        |p| seen.push(p.done),
    )
    .map(|_| seen)
}

#[test]
fn exports_mp4_that_matches_the_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    put(
        &mut ed,
        TrackKind::Video,
        ClipSource::SolidColor(Rgba::new(255, 0, 0, 255)),
        0,
        30,
    );
    let blue = add_media(
        &mut ed,
        &kit::solid_video(dir.path(), "azul.mp4", "0x3366CC", 64, 36, 2),
    );
    put(&mut ed, TrackKind::Video, ClipSource::Media(blue), 30, 30);
    let tone = add_media(&mut ed, &kit::dc_audio(dir.path(), "a.wav", 0.3, 2.0));
    put(&mut ed, TrackKind::Audio, ClipSource::Media(tone), 0, 60);

    let out = dir.path().join("saida.mp4");
    let progress = run(&ed, &out, false).unwrap();
    assert_eq!(progress.last(), Some(&60));

    let info = kit::ffprobe(&out);
    let streams = info["streams"].as_array().unwrap();
    let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let audio = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(320), Some(180))
    );
    assert_eq!(video["r_frame_rate"], "30/1");
    assert_eq!(audio["codec_name"], "aac");
    let duration: f64 = info["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((duration - 2.0).abs() < 0.1, "duração {duration}");

    let first = kit::grab_frame(&out, 0.5, W, H);
    let [r, g, b, _] = kit::pixel(&first, W, 160, 90);
    assert!(
        r > 200 && g < 50 && b < 50,
        "esperava vermelho, veio {r},{g},{b}"
    );
    let second = kit::grab_frame(&out, 1.5, W, H);
    let [r, g, b, _] = kit::pixel(&second, W, 160, 90);
    assert!(
        r.abs_diff(0x33) < 25 && g.abs_diff(0x66) < 25 && b.abs_diff(0xCC) < 25,
        "esperava azul, veio {r},{g},{b}"
    );
}

#[test]
fn cancel_removes_the_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    put(
        &mut ed,
        TrackKind::Video,
        ClipSource::SolidColor(Rgba::new(0, 255, 0, 255)),
        0,
        30,
    );
    let out = dir.path().join("cancelado.mp4");
    assert!(matches!(run(&ed, &out, true), Err(ExportError::Cancelled)));
    assert!(!out.exists());
}

#[test]
fn empty_timeline_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        run(&editor(), &dir.path().join("x.mp4"), false),
        Err(ExportError::Empty)
    ));
}

#[test]
fn offline_media_stops_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let info = MediaInfo {
        path: "/nao/existe.mp4".into(),
        kind: MediaKind::Video { has_audio: false },
        duration: Some(100),
        width: 320,
        height: 180,
    };
    let m = ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
    put(&mut ed, TrackKind::Video, ClipSource::Media(m), 10, 20);
    let out = dir.path().join("offline.mp4");
    match run(&ed, &out, false) {
        Err(ExportError::Offline { frame, path }) => {
            assert_eq!(frame, 10);
            assert_eq!(path, std::path::PathBuf::from("/nao/existe.mp4"));
        }
        other => panic!("esperava Offline, veio {other:?}"),
    }
    assert!(!out.exists());
}

#[test]
fn offline_audio_also_stops_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    put(&mut ed, TrackKind::Video, ClipSource::SolidColor(Rgba::new(0, 0, 255, 255)), 0, 30);
    let info = MediaInfo { path: "/nao/existe.wav".into(), kind: MediaKind::Audio, duration: Some(60), width: 0, height: 0 };
    let m = ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
    put(&mut ed, TrackKind::Audio, ClipSource::Media(m), 5, 20);
    let out = dir.path().join("mudo.mp4");
    match run(&ed, &out, false) {
        Err(ExportError::Offline { frame, path }) => {
            assert_eq!(frame, 5);
            assert_eq!(path, std::path::PathBuf::from("/nao/existe.wav"));
        }
        other => panic!("esperava Offline, veio {other:?}"),
    }
    assert!(!out.exists());
}
