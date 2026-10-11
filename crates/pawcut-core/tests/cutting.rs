mod common;

use common::*;
use pawcut_core::*;

#[test]
fn blade_cuts_linked_pair_and_keeps_halves_linked() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let out = ed
        .apply(Edit::Blade {
            at: 40,
            target: BladeTarget::Clips(vec![vc, ac]),
        })
        .unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 40), (40, 100)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 40), (40, 100)]);
    let (rv, ra) = (out.clips[0], out.clips[1]);
    assert_eq!(clip(&ed, rv).source_start, 40.0);
    assert!(clip(&ed, rv).link.is_some());
    assert_eq!(clip(&ed, rv).link, clip(&ed, ra).link);
    assert_ne!(clip(&ed, rv).link, clip(&ed, vc).link);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn blade_all_tracks_skips_locked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let a1 = a(&ed, 0);
    lock(&mut ed, a1);
    let out = ed
        .apply(Edit::Blade {
            at: 50,
            target: BladeTarget::AllTracks,
        })
        .unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 50), (50, 100)]);
    assert_eq!(spans(&ed, a1), vec![(0, 100)]);
    assert_eq!(clip(&ed, out.clips[0]).link, None);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn blade_on_edge_does_nothing() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    assert_eq!(
        ed.apply(Edit::Blade {
            at: 0,
            target: BladeTarget::Clips(vec![c])
        }),
        Err(EditError::NoChange)
    );
    assert_eq!(
        ed.apply(Edit::Blade {
            at: 100,
            target: BladeTarget::Clips(vec![c])
        }),
        Err(EditError::NoChange)
    );
    assert_eq!(spans(&ed, v1), vec![(0, 100)]);
}

#[test]
fn delete_leaves_gap() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    let mid = place(&mut ed, v1, solid(), 10, 0.0, 20);
    place(&mut ed, v1, solid(), 30, 0.0, 10);
    ed.apply(Edit::Delete { clips: vec![mid] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10), (30, 40)]);
}

#[test]
fn ripple_delete_closes_gap_on_same_tracks_only() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed
        .apply(Edit::AddTrack(TrackKind::Video))
        .unwrap()
        .track
        .unwrap();
    place_av(&mut ed, m, 0, 0.0, 10);
    let (mid_v, _) = place_av(&mut ed, m, 10, 10.0, 20);
    place_av(&mut ed, m, 30, 30.0, 10);
    place(&mut ed, v2, solid(), 35, 0.0, 10);
    let pair = ed.project().expand_linked(&[mid_v]);
    ed.apply(Edit::RippleDelete { clips: pair }).unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 10), (10, 20)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 10), (10, 20)]);
    assert_eq!(spans(&ed, v2), vec![(35, 45)]);
}

#[test]
fn deleting_one_half_of_a_pair_unlinks_the_other() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    ed.apply(Edit::Delete { clips: vec![vc] }).unwrap();
    assert_eq!(clip(&ed, ac).link, None);
}

#[test]
fn cannot_delete_on_locked_track() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    lock(&mut ed, v1);
    assert_eq!(
        ed.apply(Edit::Delete { clips: vec![c] }),
        Err(EditError::TrackLocked)
    );
}

#[test]
fn duplicate_ids_in_selection_are_harmless() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let d = place(&mut ed, v1, solid(), 10, 0.0, 10);
    ed.apply(Edit::RippleDelete { clips: vec![c, c] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10)]);
    ed.apply(Edit::Blade {
        at: 5,
        target: BladeTarget::Clips(vec![d, d]),
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 5), (5, 10)]);
}
