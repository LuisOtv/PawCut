use pawcut_media::VideoSource;
use pawcut_testkit as kit;

fn gray(src: &mut VideoSource, index: i64) -> u8 {
    let img = src.frame(index).unwrap();
    img.get_pixel(img.width() / 2, img.height() / 2).0[0]
}

fn assert_frame(src: &mut VideoSource, index: i64, expected: i64) {
    let got = gray(src, index);
    let want = kit::expected_gray(expected);
    assert!(
        got.abs_diff(want) <= 3,
        "frame {index}: cinza {got}, esperado {want} (frame {expected})"
    );
}

#[test]
fn decodes_requested_frame_and_fits_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = VideoSource::open(&kit::gray_ramp_video(dir.path(), 2, false), (32, 32)).unwrap();
    assert_eq!(src.size(), (32, 18));
    assert_eq!(src.fps(), 30.0);
    assert_eq!(src.index_at(0.5), 15);
    assert_frame(&mut src, 15, 15);
}

#[test]
fn sequential_frames_come_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = VideoSource::open(&kit::gray_ramp_video(dir.path(), 2, false), (64, 36)).unwrap();
    for n in 0..45 {
        assert_frame(&mut src, n, n);
    }
}

#[test]
fn seeks_backward_and_far_forward() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = VideoSource::open(&kit::gray_ramp_video(dir.path(), 2, false), (64, 36)).unwrap();
    assert_frame(&mut src, 50, 50);
    assert_frame(&mut src, 3, 3);
    assert_frame(&mut src, 57, 57);
    assert_frame(&mut src, 12, 12);
}

#[test]
fn past_end_returns_last_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = VideoSource::open(&kit::gray_ramp_video(dir.path(), 2, false), (64, 36)).unwrap();
    assert_frame(&mut src, 500, 59);
    assert_frame(&mut src, 10, 10);
}

#[test]
fn can_move_to_another_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<VideoSource>();
}
