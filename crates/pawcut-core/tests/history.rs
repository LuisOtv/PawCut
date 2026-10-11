mod common;

use common::*;
use pawcut_core::*;

#[test]
fn undo_and_redo_walk_through_snapshots() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let before = ed.project().clone();
    place_av(&mut ed, m, 0, 0.0, 100);
    let after = ed.project().clone();
    assert!(ed.undo());
    assert_eq!(ed.project(), &before);
    assert!(ed.redo());
    assert_eq!(ed.project(), &after);
    assert!(!ed.redo());
}

#[test]
fn new_edit_clears_redo() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    ed.undo();
    assert!(ed.can_redo());
    place(&mut ed, v1, solid(), 20, 0.0, 10);
    assert!(!ed.can_redo());
}

#[test]
fn failed_or_empty_edit_does_not_touch_history() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, true);
    let v1 = v(&ed, 0);
    let bad = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement {
            track: v1,
            draft: draft(ClipSource::Media(m), 90.0, 50),
        }],
    });
    assert!(bad.is_err());
    let same_flags = ed.apply(Edit::SetTrackFlags {
        track: v1,
        flags: TrackFlags {
            locked: false,
            enabled: true,
            muted: false,
            solo: false,
        },
    });
    assert_eq!(same_flags, Err(EditError::NoChange));
    assert!(ed.undo()); // desfaz o AddMedia
    assert!(!ed.undo());
}

#[test]
fn history_is_capped_at_200_steps() {
    let mut ed = new_editor();
    for _ in 0..250 {
        ed.apply(Edit::AddTrack(TrackKind::Audio)).unwrap();
    }
    for _ in 0..200 {
        assert!(ed.undo());
    }
    assert!(!ed.undo());
}

#[test]
fn coalesced_edits_share_one_undo_step() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    for x in [1.0, 2.0, 3.0] {
        let t = Transform {
            x,
            ..Transform::default()
        };
        ed.apply_coalescing(
            Edit::SetClip {
                clip: c,
                change: ClipChange::Transform(t),
            },
            "drag-x",
        )
        .unwrap();
    }
    assert_eq!(clip(&ed, c).transform.x, 3.0);
    assert!(ed.undo());
    assert_eq!(
        clip(&ed, c).transform.x,
        0.0,
        "um undo volta antes do arrasto inteiro"
    );
    // um apply normal encerra o grupo
    ed.redo();
    ed.apply(Edit::SetClip {
        clip: c,
        change: ClipChange::VolumeDb(-3.0),
    })
    .unwrap();
    let t = Transform {
        x: 9.0,
        ..Transform::default()
    };
    ed.apply_coalescing(
        Edit::SetClip {
            clip: c,
            change: ClipChange::Transform(t),
        },
        "drag-x",
    )
    .unwrap();
    ed.undo();
    assert_eq!(clip(&ed, c).transform.x, 3.0);
}

#[test]
fn revision_changes_on_every_change() {
    let mut ed = new_editor();
    let r0 = ed.revision();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    let r1 = ed.revision();
    assert_ne!(r0, r1);
    ed.undo();
    assert_ne!(ed.revision(), r1);
    let _ = ed.apply(Edit::Delete { clips: vec![] });
    let before = ed.revision();
    let _ = ed.apply(Edit::Delete { clips: vec![] });
    assert_eq!(ed.revision(), before, "edição que falha não muda a revisão");
}
