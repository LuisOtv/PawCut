//! Testes da interface com mouse e teclado simulados (egui_kittest).

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use pawcut_core::*;

use crate::panels::DragPayload;
use crate::state::{AppState, Generator, Tool};
use crate::workspace::Workspace;

fn harness(setup: impl FnOnce(&mut AppState)) -> Harness<'static, Workspace> {
    let mut st = AppState::new(Project::new(ProjectSettings::default()));
    setup(&mut st);
    st.snapping = false;
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    ws.tl.ppf = 2.0;
    ws.tl.scroll = 0.0;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 520.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.timeline_ui(ui);
            },
            ws,
        );
    h.run_steps(3);
    h
}

fn press(h: &mut Harness<'static, Workspace>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn drag(h: &mut Harness<'static, Workspace>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    press(h, from, true);
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.step();
    }
    press(h, to, false);
    h.run_steps(2);
}

fn click(h: &mut Harness<'static, Workspace>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    press(h, pos, true);
    press(h, pos, false);
    h.run_steps(2);
}

fn v1_spans(h: &Harness<'static, Workspace>) -> Vec<(Frame, Frame)> {
    h.state().st.project().video_tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect()
}

fn with_solid(st: &mut AppState) {
    st.drop_generator(Generator::SolidColor, 0, 0);
    st.selection.clear();
}

#[test]
fn dragging_a_clip_moves_it_in_time() {
    let mut h = harness(with_solid);
    let from = h.state().tl_point(TrackKind::Video, 0, 75.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 135.0);
    drag(&mut h, from, to);
    assert_eq!(v1_spans(&h), vec![(60, 210)]);
}

#[test]
fn dragging_into_the_empty_zone_creates_a_track() {
    let mut h = harness(with_solid);
    let from = h.state().tl_point(TrackKind::Video, 0, 75.0);
    let to = h.state().tl_point(TrackKind::Video, 1, 75.0);
    drag(&mut h, from, to);
    let p = h.state().st.project();
    assert_eq!(p.video_tracks.len(), 2);
    assert!(p.video_tracks[0].clips.is_empty());
    assert_eq!(p.video_tracks[1].clips.len(), 1);
}

#[test]
fn dragging_the_end_edge_trims() {
    let mut h = harness(with_solid);
    let edge = h.state().tl_point(TrackKind::Video, 0, 150.0) - egui::vec2(2.0, 0.0);
    let to = h.state().tl_point(TrackKind::Video, 0, 200.0);
    drag(&mut h, edge, to);
    // a borda anda o mesmo que o ponteiro (pegou 2 px antes do fim)
    assert_eq!(v1_spans(&h), vec![(0, 201)]);
}

#[test]
fn blade_tool_cuts_where_clicked() {
    let mut h = harness(with_solid);
    h.state_mut().st.tool = Tool::Blade;
    let at = h.state().tl_point(TrackKind::Video, 0, 50.0);
    click(&mut h, at);
    assert_eq!(v1_spans(&h), vec![(0, 50), (50, 150)]);
}

#[test]
fn clicking_selects_and_clicking_empty_space_moves_playhead() {
    let mut h = harness(with_solid);
    let on_clip = h.state().tl_point(TrackKind::Video, 0, 20.0);
    click(&mut h, on_clip);
    assert_eq!(h.state().st.selection.len(), 1);
    let empty = h.state().tl_point(TrackKind::Video, 0, 300.0);
    click(&mut h, empty);
    assert!(h.state().st.selection.is_empty());
    assert_eq!(h.state().st.playhead, 300);
}

#[test]
fn dropping_media_from_the_pool_places_it() {
    let mut h = harness(|st| {
        st.apply(Edit::AddMedia(MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: true },
            duration: Some(90),
            width: 1920,
            height: 1080,
        }));
    });
    let media = h.state().st.project().media[0].id;
    let start = h.state().tl_point(TrackKind::Video, 0, 400.0);
    let target = h.state().tl_point(TrackKind::Video, 0, 30.0);
    h.hover_at(start);
    h.step();
    press(&mut h, start, true);
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Media(media));
    h.hover_at(target);
    h.step();
    press(&mut h, target, false);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(30, 120)]);
    let a1: Vec<_> = h.state().st.project().audio_tracks[0]
        .clips
        .iter()
        .map(|c| (c.start, c.end()))
        .collect();
    assert_eq!(a1, vec![(30, 120)]);
}

#[test]
fn keyboard_shortcut_blades_at_playhead() {
    let mut h = harness(with_solid);
    h.state_mut().st.set_playhead(40);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::B);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 40), (40, 150)]);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 150)]);
}

#[test]
fn lock_button_locks_the_track() {
    let mut h = harness(with_solid);
    let lock = h.state().tl_lock_button(TrackKind::Video, 0);
    click(&mut h, lock);
    assert!(h.state().st.project().video_tracks[0].locked);
}

mod whole_app {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use pawcut_core::*;
    use pawcut_testkit as kit;

    use crate::app::PawCutApp;
    use crate::state::{AppState, Generator};
    use crate::workspace::Workspace;

    fn small_project(st: &mut AppState) {
        st.drop_generator(Generator::SolidColor, 0, 0);
        st.drop_generator(Generator::Text, 1, 0);
        let g = st.project().video_tracks[0].clips[0].id;
        st.apply(Edit::Trim {
            clips: vec![g],
            edge: Edge::End,
            delta: -120,
        });
        let t = st.project().video_tracks[1].clips[0].id;
        st.apply(Edit::Trim {
            clips: vec![t],
            edge: Edge::End,
            delta: -120,
        });
    }

    #[test]
    fn deliver_button_exports_an_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("saida.mp4");
        let mut st = AppState::new(Project::new(ProjectSettings {
            width: 320,
            height: 180,
            ..ProjectSettings::default()
        }));
        small_project(&mut st);
        let ctx = egui::Context::default();
        let mut ws = Workspace::new(&ctx, st);
        ws.recovery.clear();
        ws.open_deliver();
        ws.deliver.as_mut().unwrap().output = out.display().to_string();
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui_state(
                |ui, ws: &mut Workspace| {
                    let ctx = ui.ctx().clone();
                    ws.begin_frame(&ctx);
                    ws.dialogs_ui(&ctx);
                },
                ws,
            );
        h.run_steps(2);
        h.get_by_label("⏺ Exportar").click();
        for _ in 0..400 {
            h.step();
            if h.state()
                .deliver
                .as_ref()
                .is_some_and(|d| d.message.is_some())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let msg = h.state().deliver.as_ref().unwrap().message.clone();
        assert!(matches!(msg, Some(Ok(_))), "exportação: {msg:?}");
        let info = kit::ffprobe(&out);
        let duration: f64 = info["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 1.0).abs() < 0.1, "duração {duration}");
    }

    #[test]
    fn whole_window_opens_a_real_project_and_survives_interaction() {
        let dir = tempfile::tempdir().unwrap();
        let video = kit::gray_ramp_video(dir.path(), 2, true);
        let logo = kit::png(dir.path(), "logo.png", "orange", 64, 64);
        let music = kit::dc_audio(dir.path(), "musica.wav", 0.2, 2.0);
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        assert_eq!(st.import(&[video, logo, music]), 3);
        let ids: Vec<MediaId> = st.project().media.iter().map(|m| m.id).collect();
        st.drop_media(ids[0], TrackKind::Video, 0, 0);
        st.drop_media(ids[1], TrackKind::Video, 1, 10);
        st.drop_media(ids[2], TrackKind::Audio, 1, 0);
        st.drop_generator(Generator::Text, 2, 0);
        let path = dir.path().join("projeto.pawcut");
        st.save_to(&path).unwrap();

        let config = tempfile::tempdir().unwrap();
        // SAFETY: os testes deste módulo não leem essas variáveis em paralelo de outro jeito.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", config.path());
            std::env::set_var("XDG_DATA_HOME", config.path());
        }
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 950.0))
            .build_eframe(|cc| PawCutApp::new(cc, vec![path.clone()]));
        h.run_steps(10);
        assert_eq!(h.state().workspace().st.project().video_tracks.len(), 3);
        h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::A);
        h.run_steps(5);
        assert!(h.state().workspace().st.selection.len() >= 5);
        h.key_press(egui::Key::B);
        h.run_steps(3);
        assert_eq!(h.state().workspace().st.tool, crate::state::Tool::Blade);
        h.get_by_label("Workspace").click();
        h.run_steps(3);
        h.get_by_label("Reset UI Layout").click();
        h.run_steps(5);
    }
}

mod fixes {
    use std::time::Duration;

    use pawcut_core::*;

    use crate::state::{AppState, Monitor};
    use crate::workspace::Workspace;

    fn workspace(st: AppState) -> (Workspace, tempfile::TempDir) {
        let ctx = egui::Context::default();
        let mut ws = Workspace::new(&ctx, st);
        let dir = tempfile::tempdir().unwrap();
        ws.autosave_dir = dir.path().to_path_buf();
        ws.recovery.clear();
        ws.refresh_autosave_path();
        (ws, dir)
    }

    #[test]
    fn autosave_never_touches_files_from_other_sessions() {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        st.drop_generator(crate::state::Generator::Text, 0, 0);
        let (mut ws, dir) = workspace(st);
        let foreign = dir.path().join("sem-titulo-de-outra-sessao.pawcut");
        std::fs::write(&foreign, "{}").unwrap();
        ws.autosave_now();
        let own = ws.autosave_path.clone();
        assert!(own.exists());
        assert_ne!(own, foreign);
        ws.discard_autosave();
        assert!(!own.exists());
        assert!(foreign.exists(), "autosave de outra sessão continua lá");
    }

    #[test]
    fn autosave_names_do_not_collide_for_same_file_name() {
        let a = crate::config::autosave_name(Some(std::path::Path::new("/a/video.pawcut")), "s1");
        let b = crate::config::autosave_name(Some(std::path::Path::new("/b/video.pawcut")), "s1");
        assert_ne!(a, b);
        let u1 = crate::config::autosave_name(None, "s1");
        let u2 = crate::config::autosave_name(None, "s2");
        assert_ne!(u1, u2);
    }

    #[test]
    fn source_viewer_plays_from_the_first_frame() {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        let info = MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: false },
            duration: Some(300),
            width: 1920,
            height: 1080,
        };
        let m = st.apply(Edit::AddMedia(info)).unwrap().media.unwrap();
        st.open_in_source(m);
        let (mut ws, _dir) = workspace(st);
        assert_eq!(ws.st.active_monitor, Monitor::Source);
        ws.play(1.0);
        std::thread::sleep(Duration::from_millis(60));
        let ctx = ws.ctx.clone();
        ws.begin_frame(&ctx);
        assert!(ws.playback.is_playing(), "parou logo no começo");
        assert!(ws.st.source.playhead >= 1);
    }
}

#[test]
fn wheel_over_track_headers_scrolls_tracks_vertically() {
    let mut h = harness(|st| {
        for i in 0..10 {
            st.drop_generator(Generator::SolidColor, i, 0);
        }
    });
    let area = h.state().tl_area;
    let over_headers = egui::pos2(area.left() + 40.0, area.center().y);
    h.hover_at(over_headers);
    h.step();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -120.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(8);
    assert!(h.state().tl.v_scroll > 0.0, "a rolagem vertical não andou");
    let scroll_before = h.state().tl.scroll;
    assert_eq!(scroll_before, 0.0, "rolar nos cabeçalhos não mexe no tempo");
}

#[test]
fn hovering_a_media_previews_it_at_the_mouse_position() {
    let st = {
        let mut st = AppState::new(Project::new(ProjectSettings::default()));
        st.apply(Edit::AddMedia(MediaInfo {
            path: "/videos/x.mp4".into(),
            kind: MediaKind::Video { has_audio: false },
            duration: Some(300),
            width: 1920,
            height: 1080,
        }));
        st
    };
    let ctx = egui::Context::default();
    let mut ws = Workspace::new(&ctx, st);
    ws.recovery.clear();
    let mut h = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui_state(
            |ui, ws: &mut Workspace| {
                let ctx = ui.ctx().clone();
                ws.begin_frame(&ctx);
                ws.media_pool_ui(ui);
            },
            ws,
        );
    h.run_steps(2);
    // A primeira miniatura começa logo abaixo da barra (30 px) com 4 px de margem.
    h.hover_at(egui::pos2(10.0 + 124.0 * 0.75, 30.0 + 6.0 + 4.0 + 35.0));
    h.run_steps(3);
    let (media, frac) = h
        .state()
        .hover_preview
        .expect("o viewer devia estar mostrando a mídia");
    assert_eq!(media, h.state().st.project().media[0].id);
    assert!((frac - 0.75).abs() < 0.1, "posição {frac}");
    h.hover_at(egui::pos2(590.0, 390.0));
    h.run_steps(3);
    assert!(
        h.state().hover_preview.is_none(),
        "tirando o mouse volta para a timeline"
    );
}

#[test]
fn dragging_the_fade_handle_sets_the_fade() {
    let mut h = harness(with_solid);
    let center = h.state().tl_point(TrackKind::Video, 0, 0.0);
    // alça do fade in: canto de cima à esquerda (4 px para dentro), metade da altura da linha acima do centro
    let handle = egui::pos2(center.x + 4.0, center.y - 25.0 + 4.0);
    let to = egui::pos2(h.state().tl_point(TrackKind::Video, 0, 30.0).x, handle.y);
    drag(&mut h, handle, to);
    let c = &h.state().st.project().video_tracks[0].clips[0];
    assert!((28..=32).contains(&c.fade_in), "fade in {}", c.fade_in);
    assert_eq!(
        v1_spans(&h),
        vec![(0, 150)],
        "a alça não move nem apara o clipe"
    );
}

#[test]
fn keyboard_markers_ripple_trim_and_paste() {
    let mut h = harness(with_solid);
    h.state_mut().st.set_playhead(40);
    h.key_press(egui::Key::M);
    h.run_steps(2);
    assert_eq!(h.state().st.project().markers.len(), 1);
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 110)]);
    assert_eq!(h.state().st.playhead, 0);
    let c = h.state().st.project().video_tracks[0].clips[0].id;
    h.state_mut().st.select(c, false);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::C);
    h.state_mut().st.set_playhead(200);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::V);
    h.run_steps(2);
    assert_eq!(v1_spans(&h), vec![(0, 110), (200, 310)]);
}
