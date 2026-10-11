mod common;

use common::*;
use pawcut_core::*;

#[test]
fn insert_pushes_everything_after_on_all_unlocked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    place_av(&mut ed, m, 0, 0.0, 100);
    place(&mut ed, v2, solid(), 120, 0.0, 30);
    let v1 = v(&ed, 0);
    let a1 = a(&ed, 0);
    ed.apply(Edit::Insert {
        at: 40,
        items: vec![Placement { track: v1, draft: draft(ClipSource::Media(m), 200.0, 30) }],
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 70), (70, 130)]);
    assert_eq!(spans(&ed, a1), vec![(0, 40), (70, 130)]);
    assert_eq!(spans(&ed, v2), vec![(150, 180)]);
    let p = ed.project();
    let right_v = p.track(v1).unwrap().clip_at(70).unwrap();
    let right_a = p.track(a1).unwrap().clip_at(70).unwrap();
    assert!(right_v.link.is_some());
    assert_eq!(right_v.link, right_a.link);
    assert_eq!(right_v.source_start, 40.0);
}

#[test]
fn insert_skips_locked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let (v1, a1) = (v(&ed, 0), a(&ed, 0));
    lock(&mut ed, a1);
    ed.apply(Edit::Insert { at: 40, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 30) }] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 70), (70, 130)]);
    assert_eq!(spans(&ed, a1), vec![(0, 100)]);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
    assert_eq!(ed.project().track(v1).unwrap().clip_at(70).unwrap().link, None);
}
