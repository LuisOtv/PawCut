mod common;

use common::*;
use pawcut_core::*;

#[test]
fn overwrite_places_linked_video_and_audio() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 10, 0.0, 100);
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(10, 110)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(10, 110)]);
    assert!(clip(&ed, vc).link.is_some());
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
    assert_eq!(ed.project().expand_linked(&[vc]), vec![vc, ac]);
}

#[test]
fn overwrite_replaces_what_is_underneath() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 100);
    place(&mut ed, v1, ClipSource::Media(m), 40, 200.0, 20);
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 60), (60, 100)]);
}

#[test]
fn append_goes_to_end_of_timeline() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 50);
    ed.apply(Edit::Append { items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 30) }] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 50), (50, 80)]);
}

#[test]
fn rejects_clip_past_end_of_media() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let r = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement { track: v1, draft: draft(ClipSource::Media(m), 50.0, 60) }],
    });
    assert!(matches!(r, Err(EditError::InvalidValue(_))));
}

#[test]
fn rejects_wrong_track_kinds() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let a1 = a(&ed, 0);
    let silent = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement { track: a1, draft: draft(ClipSource::Media(m), 0.0, 10) }],
    });
    assert_eq!(silent, Err(EditError::WrongTrackKind));
    let gen_on_audio =
        ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: a1, draft: draft(solid(), 0.0, 10) }] });
    assert_eq!(gen_on_audio, Err(EditError::WrongTrackKind));
}

#[test]
fn rejects_locked_track_zero_duration_and_negative_position() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let zero = ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 0) }] });
    assert!(matches!(zero, Err(EditError::InvalidValue(_))));
    let neg = ed.apply(Edit::Overwrite { at: -5, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 10) }] });
    assert!(matches!(neg, Err(EditError::InvalidValue(_))));
    lock(&mut ed, v1);
    let locked = ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 10) }] });
    assert_eq!(locked, Err(EditError::TrackLocked));
}

#[test]
fn generators_have_no_length_limit() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 100_000);
    assert_eq!(spans(&ed, v1), vec![(0, 100_000)]);
}

#[test]
fn add_and_remove_tracks() {
    let mut ed = new_editor();
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    assert_eq!(ed.project().track_label(v2).as_deref(), Some("V2"));
    let only_audio = a(&ed, 0);
    assert!(matches!(ed.apply(Edit::RemoveTrack(only_audio)), Err(EditError::InvalidValue(_))));
    ed.apply(Edit::RemoveTrack(v2)).unwrap();
    assert_eq!(ed.project().video_tracks.len(), 1);
}
