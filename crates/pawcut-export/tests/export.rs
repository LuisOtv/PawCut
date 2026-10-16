use std::sync::atomic::AtomicBool;

use paw_media::{MediaLibrary, probe};
use paw_testkit as kit;
use pawcut_core::*;
use pawcut_export::{ExportError, ExportSettings, export};
use pawcut_render::Renderer;

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
            ..Default::default()
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
    put(
        &mut ed,
        TrackKind::Video,
        ClipSource::SolidColor(Rgba::new(0, 0, 255, 255)),
        0,
        30,
    );
    let info = MediaInfo {
        path: "/nao/existe.wav".into(),
        kind: MediaKind::Audio,
        duration: Some(60),
        width: 0,
        height: 0,
    };
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

#[test]
fn offline_media_in_a_disabled_clip_does_not_stop_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    put(
        &mut ed,
        TrackKind::Video,
        ClipSource::SolidColor(Rgba::new(0, 0, 255, 255)),
        0,
        30,
    );
    let info = MediaInfo {
        path: "/nao/existe.wav".into(),
        kind: MediaKind::Audio,
        duration: Some(60),
        width: 0,
        height: 0,
    };
    let m = ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
    let clip = put(&mut ed, TrackKind::Audio, ClipSource::Media(m), 5, 20);
    ed.apply(Edit::SetClip {
        clip,
        change: ClipChange::Enabled(false),
    })
    .unwrap();
    let out = dir.path().join("desligado.mp4");
    run(&ed, &out, false).expect("clipe desligado não conta como mídia offline");
    assert!(out.exists());
}

fn run_with(ed: &Editor, settings: ExportSettings) -> Result<(), ExportError> {
    export(
        ed.project(),
        &mut renderer(),
        &settings,
        &AtomicBool::new(false),
        |_| {},
    )
}

fn two_second_project(dir: &std::path::Path) -> Editor {
    let mut ed = editor();
    put(
        &mut ed,
        TrackKind::Video,
        ClipSource::SolidColor(Rgba::new(255, 0, 0, 255)),
        0,
        60,
    );
    let tone = add_media(&mut ed, &kit::dc_audio(dir, "a.wav", 0.3, 2.0));
    put(&mut ed, TrackKind::Audio, ClipSource::Media(tone), 0, 60);
    ed
}

fn duration(path: &std::path::Path) -> f64 {
    kit::ffprobe(path)["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn exports_only_the_in_out_range_at_another_size_in_h265() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let out = dir.path().join("trecho.mp4");
    run_with(
        &ed,
        ExportSettings {
            output: out.clone(),
            range: Some((15, 45)),
            size: Some((160, 90)),
            codec: pawcut_export::VideoCodec::H265,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        (duration(&out) - 1.0).abs() < 0.1,
        "duração {}",
        duration(&out)
    );
    let info = kit::ffprobe(&out);
    let video = info["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "video")
        .unwrap()
        .clone();
    assert_eq!(video["codec_name"], "hevc");
    assert_eq!(
        (video["width"].as_u64(), video["height"].as_u64()),
        (Some(160), Some(90))
    );
}

#[test]
fn exports_audio_only_as_mp3_and_wav() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    for (name, format, codec) in [
        ("a.mp3", pawcut_export::AudioFormat::Mp3, "mp3"),
        ("a.wav", pawcut_export::AudioFormat::Wav, "pcm_s16le"),
    ] {
        let out = dir.path().join(name);
        run_with(
            &ed,
            ExportSettings {
                output: out.clone(),
                audio_only: Some(format),
                ..Default::default()
            },
        )
        .unwrap();
        let info = kit::ffprobe(&out);
        let streams = info["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 1, "só áudio");
        assert_eq!(streams[0]["codec_name"], codec);
        assert!((duration(&out) - 2.0).abs() < 0.1);
    }
}

#[test]
fn saves_the_current_frame_as_png() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let out = dir.path().join("frame.png");
    pawcut_export::export_frame(ed.project(), &mut renderer(), 10, &out).unwrap();
    let img = image::open(&out).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (W, H));
    assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
}

fn video_stream(path: &std::path::Path) -> serde_json::Value {
    kit::ffprobe(path)["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["codec_type"] == "video")
        .unwrap()
        .clone()
}

#[test]
fn exports_at_another_frame_rate_keeping_the_duration() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let out = dir.path().join("60.mp4");
    let settings = ExportSettings {
        output: out.clone(),
        fps: Some(FrameRate::new(60, 1)),
        ..Default::default()
    };
    assert_eq!(pawcut_export::frame_count(ed.project(), &settings), 120);
    run_with(&ed, settings).unwrap();
    let v = video_stream(&out);
    assert_eq!(v["r_frame_rate"], "60/1");
    assert!(
        (duration(&out) - 2.0).abs() < 0.1,
        "duração {}",
        duration(&out)
    );
}

#[test]
fn vertical_output_from_a_wide_project_gets_black_bars() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let out = dir.path().join("vertical.mp4");
    run_with(
        &ed,
        ExportSettings {
            output: out.clone(),
            size: Some((180, 320)),
            bitrate_kbps: Some(800),
            speed: pawcut_export::EncoderSpeed::Ultrafast,
            ..Default::default()
        },
    )
    .unwrap();
    let v = video_stream(&out);
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(180), Some(320))
    );
    let img = kit::grab_frame(&out, 0.5, 180, 320);
    let top = kit::pixel(&img, 180, 90, 10);
    let mid = kit::pixel(&img, 180, 90, 160);
    assert!(top[0] < 30 && top[1] < 30, "barra preta em cima: {top:?}");
    assert!(
        mid[0] > 200 && mid[1] < 60,
        "imagem vermelha no meio: {mid:?}"
    );
}

#[test]
fn exports_prores_mov_and_files_without_audio() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let mov = dir.path().join("master.mov");
    run_with(
        &ed,
        ExportSettings {
            output: mov.clone(),
            codec: pawcut_export::VideoCodec::ProRes,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(video_stream(&mov)["codec_name"], "prores");

    let mute = dir.path().join("mudo.mkv");
    run_with(
        &ed,
        ExportSettings {
            output: mute.clone(),
            audio_kbps: None,
            ..Default::default()
        },
    )
    .unwrap();
    let streams = kit::ffprobe(&mute)["streams"].as_array().unwrap().clone();
    assert_eq!(streams.len(), 1, "sem trilha de áudio");
}

#[test]
fn vertical_output_can_use_a_blurred_background() {
    let dir = tempfile::tempdir().unwrap();
    let ed = two_second_project(dir.path());
    let out = dir.path().join("vertical-blur.mp4");
    run_with(
        &ed,
        ExportSettings {
            output: out.clone(),
            size: Some((180, 320)),
            blur_bars: true,
            speed: pawcut_export::EncoderSpeed::Ultrafast,
            ..Default::default()
        },
    )
    .unwrap();
    let img = kit::grab_frame(&out, 0.5, 180, 320);
    let top = kit::pixel(&img, 180, 90, 10);
    assert!(
        top[0] > 100 && top[1] < 60,
        "em cima, vermelho desfocado: {top:?}"
    );
}
