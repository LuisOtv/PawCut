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
    let row = h.state().tl_point(TrackKind::Video, 0, 0.0);
    let area = h.state().tl_area;
    // Botão "L" do cabeçalho: canto inferior esquerdo da linha de V1.
    let lock = egui::pos2(area.left() + 17.0, row.y + 23.0 - 12.0);
    click(&mut h, lock);
    assert!(h.state().st.project().video_tracks[0].locked);
}
