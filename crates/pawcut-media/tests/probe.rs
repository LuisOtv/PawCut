use pawcut_core::{FrameRate, MediaKind};
use pawcut_media::{MediaError, probe};
use pawcut_testkit as kit;

const FPS: FrameRate = FrameRate::new(30, 1);

#[test]
fn video_with_audio() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&kit::gray_ramp_video(dir.path(), 2, true), FPS).unwrap();
    assert_eq!(info.kind, MediaKind::Video { has_audio: true });
    assert_eq!((info.width, info.height), (64, 36));
    assert_eq!(info.duration, Some(60));
}

#[test]
fn video_without_audio() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&kit::gray_ramp_video(dir.path(), 1, false), FPS).unwrap();
    assert_eq!(info.kind, MediaKind::Video { has_audio: false });
    assert_eq!(info.duration, Some(30));
}

#[test]
fn audio_file() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0), FPS).unwrap();
    assert_eq!(info.kind, MediaKind::Audio);
    assert_eq!(info.duration, Some(30));
}

#[test]
fn mp3_cover_art_is_not_a_video() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&kit::mp3_with_cover(dir.path()), FPS).unwrap();
    assert_eq!(info.kind, MediaKind::Audio);
}

#[test]
fn image_file() {
    let dir = tempfile::tempdir().unwrap();
    let info = probe(&kit::png(dir.path(), "logo.png", "red", 40, 20), FPS).unwrap();
    assert_eq!(info.kind, MediaKind::Image);
    assert_eq!((info.width, info.height), (40, 20));
    assert_eq!(info.duration, None);
}

#[test]
fn unsupported_and_missing_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notas.txt");
    std::fs::write(&txt, "isso não é vídeo").unwrap();
    assert!(probe(&txt, FPS).is_err());
    let missing = probe(&dir.path().join("sumiu.mp4"), FPS).unwrap_err();
    assert!(matches!(missing, MediaError::Ffmpeg { .. }));
    assert!(missing.to_string().contains("sumiu.mp4"));
}
