use std::sync::Arc;

use pawcut_media::{MediaLibrary, thumbnail};
use pawcut_testkit as kit;

#[test]
fn video_frames_are_cached_and_follow_time() {
    let dir = tempfile::tempdir().unwrap();
    let video = kit::gray_ramp_video(dir.path(), 2, false);
    let mut lib = MediaLibrary::new((64, 36), 48_000, None);
    let a = lib.video_frame(&video, 0.5).unwrap();
    let b = lib.video_frame(&video, 0.5).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let g = a.get_pixel(32, 18).0[0];
    assert!(g.abs_diff(kit::expected_gray(15)) <= 3);
}

#[test]
fn images_are_shrunk_to_fit_but_never_enlarged() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = MediaLibrary::new((320, 180), 48_000, None);
    let big = lib
        .image(&kit::png(dir.path(), "big.png", "red", 400, 400))
        .unwrap();
    assert_eq!(big.dimensions(), (180, 180));
    let small = lib
        .image(&kit::png(dir.path(), "small.png", "red", 40, 20))
        .unwrap();
    assert_eq!(small.dimensions(), (40, 20));
    let [r, g, b, a] = small.get_pixel(0, 0).0;
    assert!(r >= 250 && g < 5 && b < 5 && a == 255);
}

#[test]
fn audio_is_cached_per_speed() {
    let dir = tempfile::tempdir().unwrap();
    let wav = kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0);
    let mut lib = MediaLibrary::new((64, 36), 48_000, None);
    let a = lib.audio(&wav, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a, &lib.audio(&wav, 1.0).unwrap()));
    let fast = lib.audio(&wav, 2.0).unwrap();
    assert!(fast.frames() < a.frames());
}

#[test]
fn missing_files_fail_fast_until_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = MediaLibrary::new((64, 36), 48_000, None);
    let path = dir.path().join("depois.mp4");
    assert!(lib.video_frame(&path, 0.0).is_err());
    std::fs::copy(kit::gray_ramp_video(dir.path(), 1, false), &path).unwrap();
    assert!(lib.video_frame(&path, 0.0).is_err(), "falha fica em cache");
    lib.forget(&path);
    assert!(lib.video_frame(&path, 0.0).is_ok());
}

#[test]
fn thumbnails_fit_the_box() {
    let dir = tempfile::tempdir().unwrap();
    let t = thumbnail(&kit::gray_ramp_video(dir.path(), 1, false), (32, 32)).unwrap();
    assert_eq!(t.dimensions(), (32, 18));
    let t = thumbnail(&kit::png(dir.path(), "p.png", "blue", 100, 50), (40, 40)).unwrap();
    assert_eq!(t.dimensions(), (40, 20));
}

#[test]
fn library_can_move_to_the_render_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<MediaLibrary>();
}

#[test]
fn audio_cache_stays_within_its_budget() {
    let dir = tempfile::tempdir().unwrap();
    let a = kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0);
    let b = kit::dc_audio(dir.path(), "b.wav", 0.25, 1.0);
    let mut lib = MediaLibrary::new((64, 36), 48_000, None);
    let first = lib.audio(&a, 1.0).unwrap();
    lib.set_audio_budget(first.samples.len() * 4 + 1000);
    lib.audio(&b, 1.0).unwrap();
    assert!(lib.audio_bytes() <= first.samples.len() * 4 + 1000);
    let again = lib.audio(&a, 1.0).unwrap();
    assert!(
        !Arc::ptr_eq(&first, &again),
        "o primeiro foi despejado e decodificado de novo"
    );
    assert_eq!(*first, *again);
}
