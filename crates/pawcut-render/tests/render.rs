mod common;

use common::*;
use pawcut_core::*;
use pawcut_testkit as kit;

#[test]
fn empty_timeline_is_black() {
    let ed = editor();
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert_eq!(out.image.dimensions(), (W, H));
    assert_eq!(px(&out.image, 160, 90), [0, 0, 0, 255]);
}

#[test]
fn upper_tracks_cover_lower_ones() {
    let mut ed = editor();
    solid(&mut ed, 0, RED, 0, 30);
    let top = solid(&mut ed, 1, BLUE, 0, 30);
    set(
        &mut ed,
        top,
        ClipChange::Transform(Transform {
            scale: 0.5,
            ..Transform::default()
        }),
    );
    let out = renderer().render_frame(ed.project(), 10, W, H);
    assert!(near(px(&out.image, 160, 90), [0, 0, 255], 0));
    assert!(near(px(&out.image, 5, 5), [255, 0, 0], 0));
}

#[test]
fn disabled_track_is_skipped() {
    let mut ed = editor();
    solid(&mut ed, 0, RED, 0, 30);
    solid(&mut ed, 1, BLUE, 0, 30);
    let v2 = ed.project().video_tracks[1].id;
    ed.apply(Edit::SetTrackFlags {
        track: v2,
        flags: TrackFlags {
            locked: false,
            enabled: false,
            muted: false,
            solo: false,
        },
    })
    .unwrap();
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert!(near(px(&out.image, 160, 90), [255, 0, 0], 0));
}

#[test]
fn opacity_blends_with_what_is_below() {
    let mut ed = editor();
    let c = solid(&mut ed, 0, WHITE, 0, 30);
    set(
        &mut ed,
        c,
        ClipChange::Transform(Transform {
            opacity: 0.5,
            ..Transform::default()
        }),
    );
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert!(near(px(&out.image, 160, 90), [128, 128, 128], 1));
}

#[test]
fn fades_go_from_and_to_black() {
    let mut ed = editor();
    let c = solid(&mut ed, 0, WHITE, 0, 30);
    set(&mut ed, c, ClipChange::FadeIn(10));
    set(&mut ed, c, ClipChange::FadeOut(10));
    let mut r = renderer();
    assert!(near(
        px(&r.render_frame(ed.project(), 0, W, H).image, 160, 90),
        [0, 0, 0],
        0
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 5, W, H).image, 160, 90),
        [128, 128, 128],
        1
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 15, W, H).image, 160, 90),
        [255, 255, 255],
        0
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 29, W, H).image, 160, 90),
        [0, 0, 0],
        0
    ));
}

#[test]
fn position_is_from_center_with_y_up() {
    let mut ed = editor();
    let c = solid(&mut ed, 0, BLUE, 0, 30);
    set(
        &mut ed,
        c,
        ClipChange::Transform(Transform {
            x: 80.0,
            y: 45.0,
            scale: 0.25,
            opacity: 1.0,
            ..Default::default()
        }),
    );
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert!(near(px(&out.image, 240, 45), [0, 0, 255], 0));
    assert!(near(px(&out.image, 160, 90), [0, 0, 0], 0));
    assert!(near(px(&out.image, 240, 135), [0, 0, 0], 0));
}

#[test]
fn smaller_preview_keeps_the_layout() {
    let mut ed = editor();
    solid(&mut ed, 0, RED, 0, 30);
    let top = solid(&mut ed, 1, BLUE, 0, 30);
    set(
        &mut ed,
        top,
        ClipChange::Transform(Transform {
            scale: 0.5,
            ..Transform::default()
        }),
    );
    let out = renderer().render_frame(ed.project(), 0, W / 2, H / 2);
    assert_eq!(out.image.dimensions(), (W / 2, H / 2));
    assert!(near(px(&out.image, 80, 45), [0, 0, 255], 0));
    assert!(near(px(&out.image, 2, 2), [255, 0, 0], 0));
}

#[test]
fn video_follows_source_time_and_speed() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let m = media(&mut ed, &kit::gray_ramp_video(dir.path(), 2, false));
    let c = put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Media(m),
        0,
        15.0,
        30,
    );
    let mut r = renderer();
    let gray = |r: &mut pawcut_render::Renderer, ed: &Editor, f| {
        px(&r.render_frame(ed.project(), f, W, H).image, 160, 90)[0]
    };
    assert!(gray(&mut r, &ed, 0).abs_diff(kit::expected_gray(15)) <= 4);
    assert!(gray(&mut r, &ed, 4).abs_diff(kit::expected_gray(19)) <= 4);
    ed.apply(Edit::SetSpeed {
        clips: vec![c],
        speed: 2.0,
    })
    .unwrap();
    assert!(gray(&mut r, &ed, 3).abs_diff(kit::expected_gray(21)) <= 4);
}

#[test]
fn images_keep_their_native_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let m = media(&mut ed, &kit::png(dir.path(), "logo.png", "red", 32, 32));
    put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Media(m),
        0,
        0.0,
        30,
    );
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert!(near(px(&out.image, 160, 90), [255, 0, 0], 4));
    assert!(near(px(&out.image, 190, 90), [0, 0, 0], 0));
}

#[test]
fn missing_media_is_reported_offline() {
    let mut ed = editor();
    let info = MediaInfo {
        path: "/nao/existe.mp4".into(),
        kind: MediaKind::Video { has_audio: false },
        duration: Some(100),
        width: 320,
        height: 180,
    };
    let m = ed.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
    let c = put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Media(m),
        0,
        0.0,
        30,
    );
    let out = renderer().render_frame(ed.project(), 0, W, H);
    assert_eq!(out.offline, vec![c]);
    let p = px(&out.image, 160, 90);
    assert!(
        p[0] > 100 && p[1] < 60,
        "esperava vermelho de offline, veio {p:?}"
    );
}

#[test]
fn text_is_drawn_around_the_center() {
    let mut ed = editor();
    let style = TextStyle {
        text: "PawCut".into(),
        font: "Sans".into(),
        size: 40.0,
        color: WHITE,
        ..Default::default()
    };
    put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Text(style),
        0,
        0.0,
        30,
    );
    let out = renderer().render_frame(ed.project(), 0, W, H);
    let bright = (60..120)
        .flat_map(|y| (60..260).map(move |x| (x, y)))
        .filter(|&(x, y)| px(&out.image, x, y)[0] > 200)
        .count();
    assert!(bright > 50, "poucos pixels de texto: {bright}");
    assert_eq!(px(&out.image, 2, 2), [0, 0, 0, 255]);
}

#[test]
fn renderer_can_move_to_a_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<pawcut_render::Renderer>();
}

#[test]
fn dissolve_blends_the_two_clips() {
    let mut ed = editor();
    solid(&mut ed, 0, RED, 0, 30);
    let b = solid(&mut ed, 0, BLUE, 30, 30);
    ed.apply(Edit::SetDissolve {
        clip: b,
        frames: 10,
    })
    .unwrap();
    let mut r = renderer();
    let mid = px(&r.render_frame(ed.project(), 30, W, H).image, 160, 90);
    assert!(
        mid[0] > 90 && mid[0] < 160 && mid[2] > 90 && mid[2] < 160,
        "meio do dissolve: {mid:?}"
    );
    assert!(near(
        px(&r.render_frame(ed.project(), 24, W, H).image, 160, 90),
        [255, 0, 0],
        0
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 36, W, H).image, 160, 90),
        [0, 0, 255],
        0
    ));
}

#[test]
fn opacity_keyframes_animate() {
    let mut ed = editor();
    let c = solid(&mut ed, 0, WHITE, 0, 100);
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
        at: 60.0,
        value: Some(1.0),
    })
    .unwrap();
    let mut r = renderer();
    assert!(near(
        px(&r.render_frame(ed.project(), 0, W, H).image, 160, 90),
        [0, 0, 0],
        0
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 30, W, H).image, 160, 90),
        [128, 128, 128],
        2
    ));
    assert!(near(
        px(&r.render_frame(ed.project(), 80, W, H).image, 160, 90),
        [255, 255, 255],
        0
    ));
}

#[test]
fn hue_shift_turns_red_into_green() {
    let mut ed = editor();
    let c = solid(&mut ed, 0, RED, 0, 30);
    set(
        &mut ed,
        c,
        ClipChange::Color(ColorAdjust {
            hue: 120.0,
            ..Default::default()
        }),
    );
    let p = px(
        &renderer().render_frame(ed.project(), 0, W, H).image,
        160,
        90,
    );
    assert!(p[1] > 240 && p[0] < 10, "{p:?}");
}

#[test]
fn crop_and_flip_on_media() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = editor();
    let m = media(
        &mut ed,
        &pawcut_testkit::png(dir.path(), "a.png", "red", 40, 40),
    );
    let c = put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Media(m),
        0,
        0.0,
        30,
    );
    let t = Transform {
        crop: Crop {
            left: 0.5,
            ..Default::default()
        },
        ..Transform::default()
    };
    set(&mut ed, c, ClipChange::Transform(t));
    let img = renderer().render_frame(ed.project(), 0, W, H).image;
    assert!(
        near(px(&img, 150, 90), [0, 0, 0], 0),
        "metade esquerda recortada"
    );
    assert!(near(px(&img, 170, 90), [255, 0, 0], 4));
}

#[test]
fn text_outline_draws_around_the_letters() {
    let mut ed = editor();
    let style = TextStyle {
        text: "I".into(),
        size: 80.0,
        outline: 6.0,
        outline_color: Rgba::new(255, 0, 0, 255),
        ..TextStyle::default()
    };
    put(
        &mut ed,
        TrackKind::Video,
        0,
        ClipSource::Text(style),
        0,
        0.0,
        30,
    );
    let img = renderer().render_frame(ed.project(), 0, W, H).image;
    let reds = (40..140)
        .flat_map(|y| (120..200).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = px(&img, x, y);
            p[0] > 200 && p[1] < 60
        });
    assert!(reds.count() > 30, "sem contorno vermelho");
}
