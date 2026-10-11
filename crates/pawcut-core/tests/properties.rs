mod common;

use common::*;
use pawcut_core::*;

fn set(clip: ClipId, change: ClipChange) -> Edit {
    Edit::SetClip { clip, change }
}

#[test]
fn set_transform_volume_and_fades() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    let t = Transform { x: 10.0, y: -5.0, scale: 0.5, opacity: 0.8 };
    ed.apply(set(c, ClipChange::Transform(t))).unwrap();
    ed.apply(set(c, ClipChange::VolumeDb(-6.0))).unwrap();
    ed.apply(set(c, ClipChange::FadeIn(30))).unwrap();
    ed.apply(set(c, ClipChange::FadeOut(90))).unwrap();
    let c = clip(&ed, c);
    assert_eq!(c.transform, t);
    assert_eq!(c.volume_db, -6.0);
    assert_eq!((c.fade_in, c.fade_out), (30, 70));
}

#[test]
fn rejects_bad_property_values() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    let bad_scale = Transform { scale: 0.0, ..Transform::default() };
    let bad_opacity = Transform { opacity: 1.5, ..Transform::default() };
    for change in [
        ClipChange::Transform(bad_scale),
        ClipChange::Transform(bad_opacity),
        ClipChange::VolumeDb(f32::NAN),
        ClipChange::VolumeDb(40.0),
        ClipChange::FadeIn(-1),
    ] {
        assert!(matches!(ed.apply(set(c, change)), Err(EditError::InvalidValue(_))));
    }
}

#[test]
fn only_generators_can_change_source() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let g = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let text = ClipSource::Text(TextStyle {
        text: "Olá".into(),
        font: "Sans".into(),
        size: 64.0,
        color: Rgba::new(255, 255, 255, 255),
    });
    ed.apply(set(g, ClipChange::Source(text.clone()))).unwrap();
    assert_eq!(clip(&ed, g).source, text);
    let mc = place(&mut ed, v1, ClipSource::Media(m), 20, 0.0, 10);
    assert!(matches!(ed.apply(set(mc, ClipChange::Source(solid()))), Err(EditError::InvalidValue(_))));
}

#[test]
fn speed_2x_halves_duration_and_0_5x_doubles_until_next_clip() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 100);
    place(&mut ed, v1, solid(), 150, 0.0, 10);
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 2.0 }).unwrap();
    assert_eq!((clip(&ed, c).duration, clip(&ed, c).speed), (50, 2.0));
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 0.5 }).unwrap();
    assert_eq!((clip(&ed, c).duration, clip(&ed, c).speed), (150, 0.5));
    assert_eq!(spans(&ed, v1), vec![(0, 150), (150, 160)]);
}

#[test]
fn rejects_speed_out_of_range() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    for speed in [0.0, 0.05, 20.0, f64::NAN] {
        assert!(matches!(ed.apply(Edit::SetSpeed { clips: vec![c], speed }), Err(EditError::InvalidValue(_))));
    }
}

#[test]
fn trim_with_speed_respects_media_end() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 20);
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 2.0 }).unwrap();
    assert_eq!(clip(&ed, c).duration, 10);
    ed.apply(Edit::Trim { clips: vec![c], edge: Edge::End, delta: 1000 }).unwrap();
    assert_eq!(clip(&ed, c).duration, 50);
}

#[test]
fn fades_shrink_with_clip() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    ed.apply(set(c, ClipChange::FadeIn(40))).unwrap();
    ed.apply(set(c, ClipChange::FadeOut(40))).unwrap();
    ed.apply(Edit::Trim { clips: vec![c], edge: Edge::End, delta: -70 }).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.duration, c.fade_in, c.fade_out), (30, 30, 0));
}

#[test]
fn link_and_unlink() {
    let mut ed = new_editor();
    let music = add_audio(&mut ed, 300);
    let (v1, a1) = (v(&ed, 0), a(&ed, 0));
    let s = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let au = place(&mut ed, a1, ClipSource::Media(music), 0, 0.0, 10);
    assert!(matches!(ed.apply(Edit::Link { clips: vec![s] }), Err(EditError::InvalidValue(_))));
    ed.apply(Edit::Link { clips: vec![s, au] }).unwrap();
    assert!(clip(&ed, s).link.is_some());
    assert_eq!(clip(&ed, s).link, clip(&ed, au).link);
    ed.apply(Edit::Unlink { clips: vec![s] }).unwrap();
    assert_eq!(clip(&ed, s).link, None);
    assert_eq!(clip(&ed, au).link, None);
}
