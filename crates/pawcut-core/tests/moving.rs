mod common;

use common::*;
use pawcut_core::*;

fn mv(clips: Vec<ClipId>, delta: Frame, vtd: i32, atd: i32) -> Edit {
    Edit::Move {
        clips,
        delta,
        video_track_delta: vtd,
        audio_track_delta: atd,
    }
}

#[test]
fn move_shifts_in_time_and_overwrites_destination() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 50);
    let c2 = place(&mut ed, v1, ClipSource::Media(m), 100, 100.0, 50);
    ed.apply(mv(vec![c2], -70, 0, 0)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 30), (30, 80)]);
    assert_eq!(clip(&ed, c2).source_start, 100.0);
}

#[test]
fn move_never_goes_before_zero() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 20, 0.0, 10);
    ed.apply(mv(vec![c], -100, 0, 0)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10)]);
}

#[test]
fn move_between_video_tracks_with_linked_audio_staying_on_its_track() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed
        .apply(Edit::AddTrack(TrackKind::Video))
        .unwrap()
        .track
        .unwrap();
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 50);
    ed.apply(mv(vec![vc, ac], 10, 1, 0)).unwrap();
    assert!(spans(&ed, v(&ed, 0)).is_empty());
    assert_eq!(spans(&ed, v2), vec![(10, 60)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(10, 60)]);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn move_to_missing_track_fails() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    assert!(matches!(
        ed.apply(mv(vec![c], 0, 5, 0)),
        Err(EditError::InvalidValue(_))
    ));
}

#[test]
fn zero_move_is_no_change() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    assert_eq!(ed.apply(mv(vec![c], 0, 0, 0)), Err(EditError::NoChange));
    assert_eq!(ed.apply(mv(vec![c], -5, 0, 0)), Err(EditError::NoChange));
}
