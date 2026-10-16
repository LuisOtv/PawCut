use paw_media::{Pcm, decode_audio, load_audio, peaks};
use paw_testkit as kit;

fn mid(p: &Pcm) -> [f32; 2] {
    p.frame(p.frames() as i64 / 2)
}

#[test]
fn decodes_wav_to_stereo_48k() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = decode_audio(&kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0), 48_000, 1.0).unwrap();
    assert_eq!(pcm.sample_rate, 48_000);
    assert!(
        pcm.frames().abs_diff(48_000) <= 10,
        "frames = {}",
        pcm.frames()
    );
    assert_eq!(mid(&pcm), [0.5, 0.5]);
}

#[test]
fn decodes_audio_track_of_a_video() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = decode_audio(&kit::gray_ramp_video(dir.path(), 2, true), 48_000, 1.0).unwrap();
    assert!(
        pcm.frames().abs_diff(96_000) <= 2048,
        "frames = {}",
        pcm.frames()
    );
    assert!((mid(&pcm)[0] - 0.5).abs() < 0.02);
}

#[test]
fn speed_changes_length_but_not_level() {
    let dir = tempfile::tempdir().unwrap();
    let wav = kit::dc_audio(dir.path(), "a.wav", 0.5, 1.0);
    let fast = decode_audio(&wav, 48_000, 2.0).unwrap();
    assert!(
        fast.frames().abs_diff(24_000) <= 600,
        "2x frames = {}",
        fast.frames()
    );
    assert!((mid(&fast)[0] - 0.5).abs() < 0.05);
    let slow = decode_audio(&wav, 48_000, 0.25).unwrap();
    assert!(
        slow.frames().abs_diff(192_000) <= 4000,
        "0.25x frames = {}",
        slow.frames()
    );
}

#[test]
fn out_of_range_frames_are_silent() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = decode_audio(&kit::dc_audio(dir.path(), "a.wav", 0.5, 0.1), 48_000, 1.0).unwrap();
    assert_eq!(pcm.frame(-1), [0.0, 0.0]);
    assert_eq!(pcm.frame(1_000_000), [0.0, 0.0]);
}

#[test]
fn files_without_audio_fail() {
    let dir = tempfile::tempdir().unwrap();
    assert!(decode_audio(&kit::gray_ramp_video(dir.path(), 1, false), 48_000, 1.0).is_err());
    assert!(decode_audio(&dir.path().join("sumiu.wav"), 48_000, 1.0).is_err());
}

#[test]
fn disk_cache_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let wav = kit::dc_audio(dir.path(), "a.wav", 0.25, 0.5);
    let first = load_audio(&wav, 48_000, 1.0, Some(cache.path())).unwrap();
    let files: Vec<_> = std::fs::read_dir(cache.path()).unwrap().collect();
    assert_eq!(files.len(), 1);
    let second = load_audio(&wav, 48_000, 1.0, Some(cache.path())).unwrap();
    assert_eq!(first, second);
}

#[test]
fn peaks_find_min_and_max_per_bucket() {
    let pcm = Pcm {
        sample_rate: 48_000,
        samples: vec![1.0, 1.0, -1.0, -1.0, 0.5, 0.5, 0.0, 0.0],
    };
    assert_eq!(peaks(&pcm, 2), vec![(-1.0, 1.0), (0.0, 0.5)]);
    assert!(peaks(&pcm, 0).is_empty());
}

#[test]
fn decodes_16_bit_wav_whose_decoder_reports_no_channel_layout() {
    // O WAV de 16 bits (o padrão do ffmpeg) vinha sem layout no ffmpeg 6/7 e a decodificação falhava.
    let dir = tempfile::tempdir().unwrap();
    for channels in ["1", "2"] {
        let wav = dir.path().join(format!("s16-{channels}.wav"));
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("sine=frequency=1000:duration=1")
            .args(["-ac", channels, "-ar", "44100", "-c:a", "pcm_s16le"])
            .arg(&wav)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let pcm = decode_audio(&wav, 48_000, 1.0).unwrap();
        assert!(
            pcm.frames().abs_diff(48_000) <= 200,
            "frames = {}",
            pcm.frames()
        );
    }
}
