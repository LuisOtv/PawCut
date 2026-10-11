mod common;

use std::collections::HashSet;

use common::*;
use pawcut_core::*;

fn sample_editor() -> Editor {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, _) = place_av(&mut ed, m, 0, 12.5, 100);
    ed.apply(Edit::SetSpeed {
        clips: vec![vc],
        speed: 1.5,
    })
    .unwrap();
    let v2 = ed
        .apply(Edit::AddTrack(TrackKind::Video))
        .unwrap()
        .track
        .unwrap();
    let text = ClipSource::Text(TextStyle {
        text: "Título".into(),
        font: "Sans".into(),
        size: 72.0,
        color: Rgba::new(255, 255, 0, 255),
    });
    place(&mut ed, v2, text, 10, 0.0, 60);
    ed
}

#[test]
fn save_and_load_round_trip() {
    let ed = sample_editor();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projeto.pawcut");
    save_project(ed.project(), &path).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(&loaded, ed.project());
    assert!(!dir.path().join("projeto.pawcut.tmp").exists());
}

#[test]
fn newer_version_is_refused() {
    let ed = sample_editor();
    let mut value: serde_json::Value =
        serde_json::from_str(&to_json(ed.project()).unwrap()).unwrap();
    value["version"] = 99.into();
    let err = from_json(&value.to_string()).unwrap_err();
    assert!(matches!(
        err,
        ProjectFileError::UnsupportedVersion { found: 99, .. }
    ));
}

#[test]
fn garbage_is_a_parse_error() {
    assert!(matches!(
        from_json("{ isso não é json"),
        Err(ProjectFileError::Parse(_))
    ));
    assert!(matches!(
        from_json("{\"version\": 1}"),
        Err(ProjectFileError::Parse(_))
    ));
}

#[test]
fn missing_file_is_io_error() {
    let err = load_project(std::path::Path::new("/caminho/que/nao/existe.pawcut")).unwrap_err();
    assert!(matches!(err, ProjectFileError::Io { .. }));
    assert!(err.to_string().contains("/caminho/que/nao/existe.pawcut"));
}

#[test]
fn editing_after_load_does_not_reuse_ids() {
    let ed = sample_editor();
    let json = to_json(ed.project()).unwrap();
    let mut ed = Editor::new(from_json(&json).unwrap());
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 200, 0.0, 10);
    let ids: Vec<ClipId> = ed
        .project()
        .all_tracks()
        .flat_map(|t| t.clips.iter().map(|c| c.id))
        .collect();
    let unique: HashSet<ClipId> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len());
}

fn tampered(edit: impl Fn(&mut serde_json::Value)) -> Result<Project, ProjectFileError> {
    let ed = sample_editor();
    let mut value: serde_json::Value =
        serde_json::from_str(&to_json(ed.project()).unwrap()).unwrap();
    edit(&mut value);
    from_json(&value.to_string())
}

#[test]
fn files_with_impossible_values_are_refused() {
    let cases: Vec<(&str, Box<dyn Fn(&mut serde_json::Value)>)> = vec![
        (
            "fps zero",
            Box::new(|v| v["settings"]["fps"]["num"] = 0.into()),
        ),
        (
            "sample rate zero",
            Box::new(|v| v["settings"]["sample_rate"] = 0.into()),
        ),
        (
            "duração zero",
            Box::new(|v| v["video_tracks"][0]["clips"][0]["duration"] = 0.into()),
        ),
        (
            "velocidade negativa",
            Box::new(|v| v["video_tracks"][0]["clips"][0]["speed"] = (-3.0).into()),
        ),
        (
            "source_start null",
            Box::new(|v| {
                v["video_tracks"][0]["clips"][0]["source_start"] = serde_json::Value::Null
            }),
        ),
        (
            "sobreposição",
            Box::new(|v| {
                let c = v["video_tracks"][0]["clips"][0].clone();
                let mut d = c.clone();
                d["id"] = 9999.into();
                d["start"] = (c["start"].as_i64().unwrap() + 1).into();
                v["video_tracks"][0]["clips"]
                    .as_array_mut()
                    .unwrap()
                    .push(d);
            }),
        ),
        (
            "id repetido",
            Box::new(|v| {
                let id = v["video_tracks"][0]["clips"][0]["id"].clone();
                v["audio_tracks"][0]["clips"][0]["id"] = id;
            }),
        ),
    ];
    for (name, edit) in cases {
        assert!(tampered(edit).is_err(), "deveria recusar: {name}");
    }
}

#[test]
fn stale_id_counter_is_repaired_on_load() {
    let project = tampered(|v| v["ids"]["last"] = 0.into()).unwrap();
    let mut ed = Editor::new(project);
    let t = ed
        .apply(Edit::AddTrack(TrackKind::Video))
        .unwrap()
        .track
        .unwrap();
    let same: Vec<_> = ed.project().all_tracks().filter(|tr| tr.id == t).collect();
    assert_eq!(same.len(), 1);
    let clip_ids: HashSet<u64> = ed
        .project()
        .all_tracks()
        .flat_map(|tr| tr.clips.iter().map(|c| c.id.0))
        .collect();
    assert!(!clip_ids.contains(&t.0));
}
