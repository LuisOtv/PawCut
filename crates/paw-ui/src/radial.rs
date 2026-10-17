//! Roda de atalhos (como a roda de comunicação dos jogos e o "pie menu" do Blender): segure o
//! botão do meio do mouse, arraste na direção da ação e solte. Um toque rápido (sem arrastar)
//! deixa a roda aberta para escolher com um clique; o centro abre a personalização.
//!
//! Igual nos quatro apps. Cada app fornece o catálogo de ações (id, nome, ícone) e executa o id
//! que `Radial::show` devolver. As oito posições ficam salvas em `radial.json` na pasta de
//! configuração do app.

use std::path::PathBuf;

use egui::{
    Align2, Color32, Context, Id, Key, Mesh, Order, PointerButton, Pos2, Rect, Sense, Stroke, pos2,
    vec2,
};
use serde::{Deserialize, Serialize};

use crate::icons::{self, Icon};
use crate::theme;

/// Posições na roda (começando em cima, no sentido horário).
pub const SLOTS: usize = 8;
/// Raio de fora e do miolo.
pub const OUTER: f32 = 118.0;
pub const INNER: f32 = 40.0;
/// Até aqui o ponteiro ainda está "no centro" (nada escolhido).
const DEAD: f32 = 26.0;
/// Soltar o botão antes disso, sem arrastar, deixa a roda aberta.
const TAP: f64 = 0.35;

/// Uma ação que pode ir para a roda.
#[derive(Debug, Clone)]
pub struct RadialAction {
    pub id: String,
    pub label: String,
    pub icon: Option<Icon>,
}

impl RadialAction {
    pub fn new(id: impl Into<String>, label: impl Into<String>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: Some(icon),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    slots: Vec<String>,
}

struct Open {
    center: Pos2,
    /// Ainda segurando o botão do meio (soltar escolhe); `false` = aberta com um toque.
    held: bool,
    since: f64,
    moved: bool,
}

pub struct Radial {
    path: Option<PathBuf>,
    /// Id da ação em cada posição ("" = vazia).
    pub slots: Vec<String>,
    defaults: Vec<String>,
    open: Option<Open>,
    /// Janela de personalização aberta.
    pub editing: bool,
}

/// Direção (para o editor) de cada posição.
pub const DIRECTIONS: [&str; SLOTS] = [
    "Cima",
    "Cima, direita",
    "Direita",
    "Baixo, direita",
    "Baixo",
    "Baixo, esquerda",
    "Esquerda",
    "Cima, esquerda",
];

/// Posição da roda na direção `d` (a partir do centro), ou `None` perto do centro.
pub fn slot_at(d: egui::Vec2) -> Option<usize> {
    if d.length() < DEAD {
        return None;
    }
    // 0 em cima, crescendo no sentido horário; cada posição cobre 45°.
    let a = d.x.atan2(-d.y).to_degrees();
    let a = (a + 360.0 + 22.5) % 360.0;
    Some((a / 45.0) as usize % SLOTS)
}

fn slot_angle(i: usize) -> f32 {
    (i as f32 * 45.0 - 90.0).to_radians()
}

impl Radial {
    /// Carrega as posições de `path` (ou usa `defaults`, completando até oito com vazias).
    pub fn load(path: Option<PathBuf>, defaults: &[&str]) -> Self {
        let mut defaults: Vec<String> = defaults.iter().map(|s| s.to_string()).collect();
        defaults.resize(SLOTS, String::new());
        let slots = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<File>(&t).ok())
            .map(|f| f.slots)
            .filter(|s| !s.is_empty())
            .map(|mut s| {
                s.resize(SLOTS, String::new());
                s
            })
            .unwrap_or_else(|| defaults.clone());
        Self {
            path,
            slots,
            defaults,
            open: None,
            editing: false,
        }
    }

    pub fn save(&self) {
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(t) = serde_json::to_string_pretty(&File {
                slots: self.slots.clone(),
            }) {
                let _ = std::fs::write(p, t);
            }
        }
    }

    pub fn reset(&mut self) {
        self.slots = self.defaults.clone();
        self.save();
    }

    /// A roda (ou a personalização) está na tela: o app não deve reagir a cliques e atalhos.
    pub fn is_open(&self) -> bool {
        self.open.is_some() || self.editing
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// Abre a roda em `at` como se o botão do meio tivesse sido apertado (testes e atalhos).
    pub fn open_at(&mut self, at: Pos2, time: f64, held: bool) {
        self.open = Some(Open {
            center: at,
            held,
            since: time,
            moved: false,
        });
    }

    /// Chame uma vez por quadro, depois do resto da interface (a roda fica por cima). Devolve o
    /// id da ação escolhida neste quadro.
    pub fn show(&mut self, ctx: &Context, catalog: &[RadialAction]) -> Option<String> {
        let mut chosen = None;
        let (pressed, released, pos, time, esc, primary, secondary) = ctx.input(|i| {
            (
                i.pointer.button_pressed(PointerButton::Middle),
                i.pointer.button_released(PointerButton::Middle),
                i.pointer.latest_pos(),
                i.time,
                i.key_pressed(Key::Escape),
                i.pointer.button_clicked(PointerButton::Primary),
                i.pointer.button_clicked(PointerButton::Secondary),
            )
        });
        if self.editing {
            self.editor(ctx, catalog);
            return None;
        }
        match &mut self.open {
            None => {
                if pressed && let Some(p) = pos {
                    self.open_at(p, time, true);
                }
            }
            Some(o) => {
                let d = pos.map_or(egui::Vec2::ZERO, |p| p - o.center);
                if d.length() >= DEAD {
                    o.moved = true;
                }
                let hit = slot_at(d).filter(|&i| !self.slots[i].is_empty());
                if esc || secondary || (!o.held && pressed) {
                    self.open = None;
                } else if o.held && released {
                    if let Some(i) = hit {
                        chosen = Some(self.slots[i].clone());
                        self.open = None;
                    } else if !o.moved && time - o.since < TAP {
                        o.held = false;
                    } else {
                        self.open = None;
                    }
                } else if !o.held && primary {
                    if let Some(i) = hit {
                        chosen = Some(self.slots[i].clone());
                    } else if d.length() < INNER {
                        self.editing = true;
                    }
                    self.open = None;
                }
            }
        }
        if let Some(o) = &self.open {
            let hover = pos.and_then(|p| slot_at(p - o.center));
            self.paint(ctx, o.center, o.held, hover, catalog);
        }
        chosen
    }

    fn paint(
        &self,
        ctx: &Context,
        at: Pos2,
        held: bool,
        hover: Option<usize>,
        catalog: &[RadialAction],
    ) {
        let screen = ctx.content_rect();
        // A roda inteira dentro da tela, mesmo aberta perto da borda.
        let m = OUTER + 8.0;
        let c = pos2(
            at.x.clamp(
                screen.left() + m,
                (screen.right() - m).max(screen.left() + m),
            ),
            at.y.clamp(
                screen.top() + m,
                (screen.bottom() - m).max(screen.top() + m),
            ),
        );
        egui::Area::new(Id::new("paw-radial"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                // Cobre a tela: o que está embaixo não recebe hover nem clique.
                let _ = ui.allocate_rect(screen, Sense::click_and_drag());
                let p = ui.painter();
                p.rect_filled(screen, 0.0, Color32::from_black_alpha(60));
                p.circle_filled(
                    c + vec2(0.0, 3.0),
                    OUTER + 6.0,
                    Color32::from_black_alpha(110),
                );
                p.circle(
                    c,
                    OUTER,
                    theme::BG_DARK.gamma_multiply(0.97),
                    Stroke::new(1.0, theme::BORDER),
                );
                for i in 0..SLOTS {
                    let id = &self.slots[i];
                    let action = catalog.iter().find(|a| &a.id == id);
                    let on = hover == Some(i) && action.is_some();
                    if on {
                        wedge(p, c, i, theme::ACCENT.gamma_multiply(0.85));
                    }
                    // Divisória entre as fatias.
                    let a = slot_angle(i) - 22.5f32.to_radians();
                    let dir = vec2(a.cos(), a.sin());
                    p.line_segment(
                        [c + dir * (INNER + 4.0), c + dir * (OUTER - 4.0)],
                        Stroke::new(1.0, theme::BAR_LIGHT),
                    );
                    let mid = c + vec2(slot_angle(i).cos(), slot_angle(i).sin())
                        * ((INNER + OUTER) / 2.0 + 4.0);
                    let color = if on { Color32::WHITE } else { theme::TEXT };
                    match action {
                        Some(a) => {
                            match a.icon {
                                Some(icon) => icons::paint(
                                    p,
                                    Rect::from_center_size(mid - vec2(0.0, 6.0), vec2(22.0, 22.0)),
                                    icon,
                                    color,
                                ),
                                None => {
                                    p.text(
                                        mid - vec2(0.0, 6.0),
                                        Align2::CENTER_CENTER,
                                        initials(&a.label),
                                        theme::text(14.0),
                                        color,
                                    );
                                }
                            }
                            p.text(
                                mid + vec2(0.0, 14.0),
                                Align2::CENTER_CENTER,
                                short(&a.label),
                                theme::text(10.0),
                                if on { Color32::WHITE } else { theme::TEXT_DIM },
                            );
                        }
                        None => {
                            p.text(
                                mid,
                                Align2::CENTER_CENTER,
                                "+",
                                theme::text(16.0),
                                theme::BAR_LIGHT,
                            );
                        }
                    }
                }
                // Miolo: nome completo da ação apontada, ou a dica.
                p.circle(c, INNER, theme::BAR, Stroke::new(1.0, theme::BORDER));
                let text = match hover.and_then(|i| catalog.iter().find(|a| a.id == self.slots[i]))
                {
                    Some(a) => a.label.clone(),
                    None if held => "Solte numa ação".to_string(),
                    None => "Personalizar".to_string(),
                };
                let galley = ui.painter().layout(
                    text,
                    theme::text(10.5),
                    Color32::WHITE,
                    INNER * 2.0 - 12.0,
                );
                let gp = c - galley.size() / 2.0;
                p.galley(gp, galley, Color32::WHITE);
                if !held && hover.is_none() {
                    icons::paint(
                        p,
                        Rect::from_center_size(c - vec2(0.0, 20.0), vec2(12.0, 12.0)),
                        Icon::Gear,
                        theme::TEXT_DIM,
                    );
                }
            });
    }

    /// Janela para escolher a ação de cada posição.
    fn editor(&mut self, ctx: &Context, catalog: &[RadialAction]) {
        let mut open = true;
        let mut changed = false;
        let mut done = false;
        egui::Window::new("Roda de atalhos")
            .id(Id::new("paw-radial-editor"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Segure o botão do meio do mouse, arraste até a ação e solte.\n\
                         Um toque rápido deixa a roda aberta para clicar.",
                    )
                    .color(theme::TEXT_DIM),
                );
                ui.add_space(8.0);
                egui::Grid::new("radial-slots")
                    .num_columns(2)
                    .spacing(vec2(12.0, 6.0))
                    .show(ui, |ui| {
                        for (i, dir) in DIRECTIONS.iter().enumerate() {
                            ui.label(egui::RichText::new(*dir).color(theme::TEXT_DIM));
                            let current = catalog
                                .iter()
                                .find(|a| a.id == self.slots[i])
                                .map_or("(vazio)".to_string(), |a| a.label.clone());
                            egui::ComboBox::from_id_salt(("radial-slot", i))
                                .selected_text(current)
                                .width(220.0)
                                .height(320.0)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(self.slots[i].is_empty(), "(vazio)")
                                        .clicked()
                                    {
                                        self.slots[i].clear();
                                        changed = true;
                                    }
                                    for a in catalog {
                                        if ui
                                            .selectable_label(self.slots[i] == a.id, &a.label)
                                            .clicked()
                                        {
                                            self.slots[i] = a.id.clone();
                                            changed = true;
                                        }
                                    }
                                });
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Restaurar padrão").clicked() {
                        self.slots = self.defaults.clone();
                        changed = true;
                    }
                    if ui.button("Fechar").clicked() {
                        done = true;
                    }
                });
            });
        if changed {
            self.save();
        }
        if !open || done || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.editing = false;
        }
    }
}

/// Fatia `i` da roda (anel entre o miolo e a borda) preenchida.
fn wedge(p: &egui::Painter, c: Pos2, i: usize, color: Color32) {
    let mut mesh = Mesh::default();
    let (a0, a1) = (
        slot_angle(i) - 22.5f32.to_radians(),
        slot_angle(i) + 22.5f32.to_radians(),
    );
    let n = 12;
    for k in 0..=n {
        let a = a0 + (a1 - a0) * k as f32 / n as f32;
        let d = vec2(a.cos(), a.sin());
        mesh.colored_vertex(c + d * (INNER + 2.0), color);
        mesh.colored_vertex(c + d * (OUTER - 2.0), color);
    }
    for k in 0..n as u32 {
        let b = k * 2;
        mesh.add_triangle(b, b + 1, b + 2);
        mesh.add_triangle(b + 1, b + 3, b + 2);
    }
    p.add(mesh);
}

/// Até duas palavras, para caber na fatia.
fn short(label: &str) -> String {
    let mut out = String::new();
    for w in label.split_whitespace() {
        if out.chars().count() + w.chars().count() > 13 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.is_empty() {
        label.chars().take(12).collect()
    } else {
        out
    }
}

fn initials(label: &str) -> String {
    label
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, RawInput};

    fn catalog() -> Vec<RadialAction> {
        vec![
            RadialAction::new("undo", "Desfazer", Icon::Undo),
            RadialAction::new("redo", "Refazer", Icon::Redo),
            RadialAction::new("save", "Salvar projeto", Icon::Save),
        ]
    }

    #[test]
    fn directions_map_to_slots_clockwise_from_the_top() {
        assert_eq!(slot_at(vec2(0.0, -80.0)), Some(0));
        assert_eq!(slot_at(vec2(60.0, -60.0)), Some(1));
        assert_eq!(slot_at(vec2(80.0, 0.0)), Some(2));
        assert_eq!(slot_at(vec2(0.0, 80.0)), Some(4));
        assert_eq!(slot_at(vec2(-80.0, 0.0)), Some(6));
        assert_eq!(slot_at(vec2(-60.0, -60.0)), Some(7));
        assert_eq!(slot_at(vec2(5.0, 5.0)), None);
        // Bem na fronteira entre cima e cima-direita continua estável.
        assert!(slot_at(vec2(30.6, -73.9)).is_some());
    }

    #[test]
    fn slots_persist_and_reset_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radial.json");
        let mut r = Radial::load(Some(path.clone()), &["undo", "redo"]);
        assert_eq!(r.slots.len(), SLOTS);
        assert_eq!(r.slots[0], "undo");
        r.slots[3] = "save".into();
        r.save();
        let back = Radial::load(Some(path.clone()), &["undo"]);
        assert_eq!(back.slots[3], "save");
        assert_eq!(back.slots[1], "redo");
        let mut back = back;
        back.reset();
        assert_eq!(Radial::load(Some(path), &["undo"]).slots[3], "");
    }

    struct H {
        ctx: Context,
        t: f64,
    }

    impl H {
        fn frame(&mut self, r: &mut Radial, events: Vec<Event>) -> Option<String> {
            self.t += 0.05;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.t),
                events,
                ..Default::default()
            };
            let mut got = None;
            let cat = catalog();
            self.ctx
                .run_ui(raw, |ui| got = r.show(ui.ctx(), &cat))
                .drop_without_applying_deltas();
            got
        }
    }

    fn btn(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn hold_drag_and_release_picks_the_action_in_that_direction() {
        let mut h = H {
            ctx: Context::default(),
            t: 0.0,
        };
        let mut r = Radial::load(None, &["undo", "redo", "save"]);
        let c = pos2(400.0, 300.0);
        h.frame(&mut r, vec![Event::PointerMoved(c)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, true)]);
        assert!(r.is_open());
        let right = c + vec2(80.0, 0.0);
        h.frame(&mut r, vec![Event::PointerMoved(right)]);
        let got = h.frame(&mut r, vec![btn(right, PointerButton::Middle, false)]);
        assert_eq!(got.as_deref(), Some("save"));
        assert!(!r.is_open());
    }

    #[test]
    fn a_quick_tap_keeps_it_open_and_a_click_picks_or_customizes() {
        let mut h = H {
            ctx: Context::default(),
            t: 0.0,
        };
        let mut r = Radial::load(None, &["undo", "redo"]);
        let c = pos2(400.0, 300.0);
        h.frame(&mut r, vec![Event::PointerMoved(c)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, true)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, false)]);
        assert!(r.is_open(), "toque rápido deixa aberta");
        // Clique numa fatia vazia não faz nada e fecha; clique numa ação escolhe.
        let up = c + vec2(0.0, -80.0);
        h.frame(&mut r, vec![Event::PointerMoved(up)]);
        h.frame(&mut r, vec![btn(up, PointerButton::Primary, true)]);
        let got = h.frame(&mut r, vec![btn(up, PointerButton::Primary, false)]);
        assert_eq!(got.as_deref(), Some("undo"));
        // De novo, e clique no miolo abre a personalização.
        h.frame(&mut r, vec![Event::PointerMoved(c)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, true)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, false)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Primary, true)]);
        h.frame(&mut r, vec![btn(c, PointerButton::Primary, false)]);
        assert!(r.editing && r.is_open());
        // Soltar longe do centro, numa fatia vazia, cancela.
        r.editing = false;
        h.frame(&mut r, vec![btn(c, PointerButton::Middle, true)]);
        let empty = c + vec2(0.0, 80.0);
        h.frame(&mut r, vec![Event::PointerMoved(empty)]);
        let got = h.frame(&mut r, vec![btn(empty, PointerButton::Middle, false)]);
        assert!(got.is_none() && !r.is_open());
    }

    #[test]
    fn labels_are_shortened_for_the_slice() {
        assert_eq!(short("Salvar projeto como"), "Salvar projeto");
        assert_eq!(short("Desfazer"), "Desfazer");
        assert_eq!(initials("nova camada"), "NC");
    }
}
