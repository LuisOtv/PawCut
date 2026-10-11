mod common;

use common::*;
use pawcut_core::*;

fn trim(clips: Vec<ClipId>, edge: Edge, delta: Frame) -> Edit {
    Edit::Trim { clips, edge, delta }
}

#[test]
fn trim_end_extends_until_next_clip() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c1 = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 50);
    place(&mut ed, v1, ClipSource::Media(m), 80, 0.0, 20);
    ed.apply(trim(vec![c1], Edge::End, 100)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 80), (80, 100)]);
}

#[test]
fn trim_end_stops_at_end_of_media() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 30.0, 50);
    ed.apply(trim(vec![c], Edge::End, 100)).unwrap();
    assert_eq!(clip(&ed, c).duration, 70);
}

#[test]
fn trim_start_moves_source_in_point() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 20, 20.0, 50);
    ed.apply(trim(vec![c], Edge::Start, 10)).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.start, c.duration, c.source_start), (30, 40, 30.0));
}

#[test]
fn trim_start_cannot_go_before_source_start_or_frame_zero() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 20, 5.0, 50);
    ed.apply(trim(vec![c], Edge::Start, -100)).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.start, c.duration, c.source_start), (15, 55, 0.0));
}

#[test]
fn generator_start_trim_stops_at_frame_zero() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 20, 1000.0, 10);
    ed.apply(trim(vec![c], Edge::Start, -100)).unwrap();
    assert_eq!(clip(&ed, c).start, 0);
}

#[test]
fn trim_never_leaves_less_than_one_frame() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    ed.apply(trim(vec![c], Edge::End, -50)).unwrap();
    assert_eq!(clip(&ed, c).duration, 1);
    assert_eq!(
        ed.apply(trim(vec![c], Edge::End, -50)),
        Err(EditError::NoChange)
    );
}

#[test]
fn trim_linked_pair_moves_both_by_same_amount() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let music = add_audio(&mut ed, 300);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 50);
    let a1 = a(&ed, 0);
    place(&mut ed, a1, ClipSource::Media(music), 60, 0.0, 30);
    ed.apply(trim(vec![vc, ac], Edge::End, 100)).unwrap();
    assert_eq!(clip(&ed, vc).end(), 60);
    assert_eq!(clip(&ed, ac).end(), 60);
}
