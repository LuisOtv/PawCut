mod common;

use common::*;
use pawcut_core::*;

#[test]
fn ripple_trim_to_playhead_removes_head_or_tail_and_closes_gap() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 100, 0.0, 50);
    ed.apply(Edit::RippleTrimToPlayhead {
        clips: vec![vc, ac],
        edge: Edge::Start,
        at: 30,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 70), (70, 120)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 70)]);
    assert_eq!(ed.project().video_tracks[0].clips[0].source_start, 30.0);
    let first = ed.project().video_tracks[0].clips[0].id;
    let pair = ed.project().expand_linked(&[first]);
    ed.apply(Edit::RippleTrimToPlayhead {
        clips: pair,
        edge: Edge::End,
        at: 50,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 50), (50, 100)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 50)]);
}

#[test]
fn markers_are_added_replaced_and_removed() {
    let mut ed = new_editor();
    let mk = |note: &str| Marker {
        frame: 0,
        color: Rgba::new(255, 0, 0, 255),
        note: note.into(),
    };
    ed.apply(Edit::SetMarker {
        frame: 40,
        marker: Some(mk("a")),
    })
    .unwrap();
    ed.apply(Edit::SetMarker {
        frame: 10,
        marker: Some(mk("b")),
    })
    .unwrap();
    ed.apply(Edit::SetMarker {
        frame: 40,
        marker: Some(mk("c")),
    })
    .unwrap();
    let frames: Vec<(Frame, String)> = ed
        .project()
        .markers
        .iter()
        .map(|m| (m.frame, m.note.clone()))
        .collect();
    assert_eq!(frames, vec![(10, "b".to_string()), (40, "c".to_string())]);
    ed.apply(Edit::SetMarker {
        frame: 10,
        marker: None,
    })
    .unwrap();
    assert_eq!(ed.project().markers.len(), 1);
}

#[test]
fn dissolve_needs_an_adjacent_clip_and_is_limited_by_durations() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let a = place(&mut ed, v1, solid(), 0, 0.0, 20);
    let b = place(&mut ed, v1, solid(), 20, 0.0, 100);
    let lone = place(&mut ed, v1, solid(), 200, 0.0, 10);
    assert!(
        ed.apply(Edit::SetDissolve {
            clip: a,
            frames: 10
        })
        .is_err()
    );
    assert!(
        ed.apply(Edit::SetDissolve {
            clip: lone,
            frames: 10
        })
        .is_err()
    );
    ed.apply(Edit::SetDissolve {
        clip: b,
        frames: 50,
    })
    .unwrap();
    assert_eq!(clip(&ed, b).dissolve_in, 20);
    let track = &ed.project().video_tracks[0];
    let layers = track.layers_at(20);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].0.id, a);
    assert!((layers[0].1 + layers[1].1 - 1.0).abs() < 1e-6);
    assert_eq!(track.layers_at(5).len(), 1);
}

#[test]
fn keyframes_interpolate_in_source_time() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 10, 0.0, 100);
    ed.apply(Edit::SetKeyframe {
        clip: c,
        param: Param::Opacity,
        at: 0.0,
        value: Some(0.0),
    })
    .unwrap();
    ed.apply(Edit::SetKeyframe {
        clip: c,
        param: Param::Opacity,
        at: 50.0,
        value: Some(1.0),
    })
    .unwrap();
    let k = clip(&ed, c);
    assert_eq!(k.param(Param::Opacity, 10), 0.0);
    assert!(
        (k.param(Param::Opacity, 35) - 0.5).abs() < 1e-6,
        "meio do caminho com ease"
    );
    assert_eq!(k.param(Param::Opacity, 90), 1.0);
    assert_eq!(k.param(Param::X, 50), 0.0, "sem keyframes usa o valor fixo");
    // mover o clipe leva a animação junto
    ed.apply(Edit::Move {
        clips: vec![c],
        delta: 100,
        video_track_delta: 0,
        audio_track_delta: 0,
    })
    .unwrap();
    assert_eq!(clip(&ed, c).param(Param::Opacity, 110), 0.0);
    assert!(
        ed.apply(Edit::SetKeyframe {
            clip: c,
            param: Param::Opacity,
            at: 10.0,
            value: Some(3.0)
        })
        .is_err()
    );
    ed.apply(Edit::SetKeyframe {
        clip: c,
        param: Param::Opacity,
        at: 0.2,
        value: None,
    })
    .unwrap();
    assert_eq!(clip(&ed, c).keyframes[&Param::Opacity].len(), 1);
    ed.apply(Edit::ClearKeyframes {
        clip: c,
        param: Param::Opacity,
    })
    .unwrap();
    assert!(clip(&ed, c).keyframes.is_empty());
}

#[test]
fn color_crop_and_resolution_are_validated() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    ed.apply(Edit::SetClip {
        clip: c,
        change: ClipChange::Color(ColorAdjust {
            hue: 90.0,
            saturation: -100.0,
            lightness: 10.0,
        }),
    })
    .unwrap();
    assert!(
        ed.apply(Edit::SetClip {
            clip: c,
            change: ClipChange::Color(ColorAdjust {
                hue: 500.0,
                ..Default::default()
            })
        })
        .is_err()
    );
    let crop = Transform {
        crop: Crop {
            left: 0.6,
            right: 0.5,
            top: 0.0,
            bottom: 0.0,
        },
        ..Transform::default()
    };
    assert!(
        ed.apply(Edit::SetClip {
            clip: c,
            change: ClipChange::Transform(crop)
        })
        .is_err()
    );
    ed.apply(Edit::SetResolution {
        width: 1080,
        height: 1920,
    })
    .unwrap();
    assert_eq!(
        (ed.project().settings.width, ed.project().settings.height),
        (1080, 1920)
    );
}

#[test]
fn version_1_projects_still_open() {
    let ed = new_editor();
    let mut value: serde_json::Value =
        serde_json::from_str(&to_json(ed.project()).unwrap()).unwrap();
    value["version"] = 1.into();
    value.as_object_mut().unwrap().remove("markers");
    let loaded = from_json(&value.to_string()).unwrap();
    assert!(loaded.markers.is_empty());
}

#[test]
fn paste_places_full_copies_with_new_ids_and_links() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 50);
    let t = Transform {
        x: 40.0,
        ..Transform::default()
    };
    ed.apply(Edit::SetClip {
        clip: vc,
        change: ClipChange::Transform(t),
    })
    .unwrap();
    let p = ed.project().clone();
    let copy = |id: ClipId| {
        let (tid, _) = p.locate(id).unwrap();
        let mut c = p.clip(id).unwrap().clone();
        c.start -= 0; // deslocamento relativo ao primeiro clipe copiado
        (tid, c)
    };
    let out = ed
        .apply(Edit::Paste {
            at: 100,
            clips: vec![copy(vc), copy(ac)],
        })
        .unwrap();
    assert_eq!(out.clips.len(), 2);
    let (nv, na) = (clip(&ed, out.clips[0]), clip(&ed, out.clips[1]));
    assert_eq!((nv.start, nv.end()), (100, 150));
    assert_eq!(nv.transform.x, 40.0);
    assert!(nv.link.is_some() && nv.link == na.link && nv.link != clip(&ed, vc).link);
    assert_ne!(nv.id, vc);
}

#[test]
fn ripple_delete_range_cuts_time_out_of_several_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    place_av(&mut ed, m, 0, 0.0, 100);
    let (v1, a1) = (v(&ed, 0), a(&ed, 0));
    ed.apply(Edit::RippleDeleteRange {
        tracks: vec![v1, a1],
        start: 30,
        end: 50,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 30), (30, 80)]);
    assert_eq!(spans(&ed, a1), vec![(0, 30), (30, 80)]);
    let right = &ed.project().video_tracks[0].clips[1];
    assert_eq!(right.source_start, 50.0);
    assert_eq!(right.link, ed.project().audio_tracks[0].clips[1].link);
}

#[test]
fn close_gap_and_close_all_gaps() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 10, 0.0, 10);
    place(&mut ed, v1, solid(), 30, 0.0, 10);
    place(&mut ed, v1, solid(), 60, 0.0, 10);
    ed.apply(Edit::CloseGap { track: v1, at: 25 }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(10, 20), (20, 30), (50, 60)]);
    assert_eq!(
        ed.apply(Edit::CloseGap { track: v1, at: 15 }),
        Err(EditError::NoChange)
    );
    ed.apply(Edit::CloseAllGaps { track: v1 }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(10, 20), (20, 30), (30, 40)]);
    ed.apply(Edit::CloseGap { track: v1, at: 0 }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10), (10, 20), (20, 30)]);
}

#[test]
fn undo_and_redo_have_names() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    assert_eq!(ed.undo_label(), Some("Overwrite"));
    ed.undo();
    assert_eq!(ed.redo_label(), Some("Overwrite"));
    assert_eq!(ed.undo_label(), None);
}

#[test]
fn delete_range_leaves_a_gap() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 100);
    ed.apply(Edit::DeleteRange {
        tracks: vec![v1],
        start: 30,
        end: 50,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 30), (50, 100)]);
}

#[test]
fn ripple_trim_moves_the_rest_of_the_track() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let first = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 100);
    place(&mut ed, v1, solid(), 100, 0.0, 50);
    // Fim: estica 20 e empurra o resto.
    ed.apply(Edit::RippleTrim {
        clips: vec![first],
        edge: Edge::End,
        delta: 20,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 120), (120, 170)]);
    // Início: o clipe fica no lugar, o conteúdo encurta e o resto puxa.
    ed.apply(Edit::RippleTrim {
        clips: vec![first],
        edge: Edge::Start,
        delta: 30,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 90), (90, 140)]);
    assert_eq!(clip(&ed, first).source_start, 30.0);
    // Não passa do fim da fonte (300 frames).
    ed.apply(Edit::RippleTrim {
        clips: vec![first],
        edge: Edge::End,
        delta: 1000,
    })
    .unwrap();
    assert_eq!(clip(&ed, first).duration, 270);
}

#[test]
fn roll_moves_the_cut_and_slip_changes_the_content() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let left = place(&mut ed, v1, ClipSource::Media(m), 0, 50.0, 100);
    let right = place(&mut ed, v1, ClipSource::Media(m), 100, 100.0, 100);
    ed.apply(Edit::Roll {
        cuts: vec![(left, right)],
        delta: -20,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 80), (80, 200)]);
    assert_eq!(clip(&ed, right).source_start, 80.0);
    // O direito fica com pelo menos 1 frame.
    ed.apply(Edit::Roll {
        cuts: vec![(left, right)],
        delta: 500,
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 199), (199, 200)]);
    // Slip: mesma posição e duração, outro trecho.
    ed.apply(Edit::Slip {
        clips: vec![left],
        delta: 10,
    })
    .unwrap();
    assert_eq!(clip(&ed, left).source_start, 60.0);
    ed.apply(Edit::Slip {
        clips: vec![left],
        delta: -1000,
    })
    .unwrap();
    assert_eq!(clip(&ed, left).source_start, 0.0);
}

#[test]
fn clips_can_be_disabled_frozen_and_eased() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 10.0, 100);
    ed.apply(Edit::SetClip {
        clip: c,
        change: ClipChange::Enabled(false),
    })
    .unwrap();
    assert!(!clip(&ed, c).enabled);
    ed.apply(Edit::SetClip {
        clip: c,
        change: ClipChange::Freeze(Some(42.0)),
    })
    .unwrap();
    assert_eq!(clip(&ed, c).source_time_at(0), 42.0);
    assert_eq!(clip(&ed, c).source_time_at(99), 42.0);
    // Ease num keyframe.
    for (at, value) in [(10.0, 0.0), (110.0, 90.0)] {
        ed.apply(Edit::SetKeyframe {
            clip: c,
            param: Param::Rotation,
            at,
            value: Some(value),
        })
        .unwrap();
    }
    ed.apply(Edit::SetClip {
        clip: c,
        change: ClipChange::Freeze(None),
    })
    .unwrap();
    ed.apply(Edit::SetKeyframeEase {
        clip: c,
        param: Param::Rotation,
        at: 110.0,
        ease: Ease::Linear,
    })
    .unwrap();
    assert!((clip(&ed, c).param(Param::Rotation, 25) - 22.5).abs() < 1e-3);
    ed.apply(Edit::SetKeyframeEase {
        clip: c,
        param: Param::Rotation,
        at: 110.0,
        ease: Ease::EaseIn,
    })
    .unwrap();
    assert!(clip(&ed, c).param(Param::Rotation, 25) < 10.0);
    // Projetos antigos abrem com o clipe ligado.
    let json = to_json(ed.project())
        .unwrap()
        .replace("\"enabled\": false,", "");
    let back = from_json(&json).unwrap();
    assert!(back.video_tracks[0].clips[0].enabled);
}
