//! A janela do PawPaint: camadas à esquerda, tela no meio, propriedades à direita.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use egui::{
    Align2, Color32, CursorIcon, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2,
    vec2,
};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawpaint_core::{
    Adjust, Background, Chroma, Editor, Kind, Layer, LayerId, MoveTo, Outline, Rgba, Shadow,
    ShapeKind, THUMB_H, THUMB_W, TextAlign, load_project, save_project,
};
use pawpaint_render::Renderer;

/// Limite de tamanho da thumbnail no YouTube.
const YOUTUBE_MAX: usize = 2_000_000;
const SNAP_PX: f32 = 6.0;
const HANDLE: f32 = 6.0;

fn to_color(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn from_color(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba::new(r, g, b, a)
}

/// Algo a fazer na janela depois que um diálogo fecha.
type Action = Box<dyn FnOnce(&mut PaintApp)>;

enum Purpose {
    Open,
    SaveAs,
    Import,
    ExportPng,
    ExportJpg,
}

enum Dialog {
    New { w: u32, h: u32 },
    ConfirmClose,
}

/// O que o mouse está fazendo na tela.
#[derive(Clone)]
enum Drag {
    Pan,
    Move { grab: Vec2 },
    Scale { start_dist: f32, start_scale: f32 },
    Rotate,
}

pub struct PaintApp {
    ed: Editor,
    path: Option<PathBuf>,
    selected: Option<LayerId>,
    renderer: Renderer,
    /// Imagem do último desenho e o `revision` dele.
    rendered: Option<(u64, egui::TextureHandle)>,
    previews: Vec<egui::TextureHandle>,
    preview_rev: u64,
    zoom: f32,
    fit: bool,
    pan: Vec2,
    drag: Option<Drag>,
    guides: bool,
    snap_lines: (bool, bool),
    toasts: Toasts,
    dialog: Option<Dialog>,
    file: Option<(Purpose, Receiver<Vec<PathBuf>>)>,
    fonts: Vec<String>,
    renaming: Option<(LayerId, String)>,
    focus_text: bool,
    allow_close: bool,
    last_title: String,
    screenshot: Option<PathBuf>,
    frames: u64,
}

impl PaintApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWPAINT_SCREENSHOT").map(PathBuf::from);
        // Só para testes visuais: `PAWPAINT_SELECT=n` já abre com a camada n selecionada.
        let select = std::env::var("PAWPAINT_SELECT")
            .ok()
            .and_then(|v| v.parse().ok());
        let mut images = Vec::new();
        for p in args {
            match p.extension().and_then(|e| e.to_str()) {
                Some("pawpaint") => app.open_project(&p),
                _ => images.push(p),
            }
        }
        if !images.is_empty() {
            app.import(images);
        }
        if let Some(id) = select {
            app.selected = Some(id);
        }
        app
    }

    fn empty() -> Self {
        Self {
            ed: Editor::new(THUMB_W, THUMB_H),
            path: None,
            selected: None,
            renderer: Renderer::new(),
            rendered: None,
            previews: Vec::new(),
            preview_rev: u64::MAX,
            zoom: 1.0,
            fit: true,
            pan: Vec2::ZERO,
            drag: None,
            guides: false,
            snap_lines: (false, false),
            toasts: Toasts::default(),
            dialog: None,
            file: None,
            fonts: Vec::new(),
            renaming: None,
            focus_text: false,
            allow_close: false,
            last_title: String::new(),
            screenshot: None,
            frames: 0,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.toasts.push(text, false);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.toasts.push(text, true);
    }

    fn layer(&self) -> Option<&Layer> {
        self.ed.doc().layer(self.selected?)
    }

    // ---------------------------------------------------------------- arquivos

    fn ask(&mut self, purpose: Purpose) {
        if self.file.is_some() {
            return;
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("thumbnail".to_string(), |s| {
                s.to_string_lossy().into_owned()
            });
        let (tx, rx) = channel();
        let kind = match purpose {
            Purpose::Open => 0,
            Purpose::SaveAs => 1,
            Purpose::Import => 2,
            Purpose::ExportPng => 3,
            Purpose::ExportJpg => 4,
        };
        std::thread::spawn(move || {
            let d = rfd::FileDialog::new();
            let picked: Vec<PathBuf> = match kind {
                0 => d
                    .set_title("Abrir projeto")
                    .add_filter("PawPaint", &["pawpaint"])
                    .pick_file()
                    .into_iter()
                    .collect(),
                1 => d
                    .set_title("Salvar projeto")
                    .add_filter("PawPaint", &["pawpaint"])
                    .set_file_name(format!("{name}.pawpaint"))
                    .save_file()
                    .into_iter()
                    .collect(),
                2 => d
                    .set_title("Importar imagens")
                    .add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_files()
                    .unwrap_or_default(),
                3 => d
                    .set_title("Exportar PNG")
                    .add_filter("PNG", &["png"])
                    .set_file_name(format!("{name}.png"))
                    .save_file()
                    .into_iter()
                    .collect(),
                _ => d
                    .set_title("Exportar JPG para o YouTube")
                    .add_filter("JPG", &["jpg"])
                    .set_file_name(format!("{name}.jpg"))
                    .save_file()
                    .into_iter()
                    .collect(),
            };
            let _ = tx.send(picked);
        });
        self.file = Some((purpose, rx));
    }

    fn poll_file(&mut self) {
        let Some(paths) = self.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
            return;
        };
        let (purpose, _) = self.file.take().expect("janela aberta");
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match purpose {
            Purpose::Open => self.open_project(&first),
            Purpose::SaveAs => {
                let mut p = first;
                if p.extension().is_none_or(|e| e != "pawpaint") {
                    p.set_extension("pawpaint");
                }
                self.save_to(p);
            }
            Purpose::Import => self.import(paths),
            Purpose::ExportPng => self.export_png(first),
            Purpose::ExportJpg => self.export_jpg(first),
        }
    }

    fn open_project(&mut self, path: &Path) {
        match load_project(path) {
            Ok((ed, missing)) => {
                self.ed = ed;
                self.path = Some(path.to_path_buf());
                self.selected = None;
                self.fit = true;
                self.rendered = None;
                if !missing.is_empty() {
                    self.warn(format!(
                        "{} imagem(ns) do projeto não foram encontradas",
                        missing.len()
                    ));
                }
            }
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match save_project(&self.ed, &path) {
            Ok(()) => {
                self.ed.mark_saved();
                self.path = Some(path);
                self.say("Projeto salvo");
            }
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.ask(Purpose::SaveAs),
        }
    }

    fn import(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            let name = p
                .file_stem()
                .map_or("Imagem".into(), |s| s.to_string_lossy().into_owned());
            match image::open(&p) {
                Ok(img) => {
                    let id = self.ed.add_image(&name, img.to_rgba8());
                    self.selected = Some(id);
                }
                Err(e) => self.warn(format!("Não deu para abrir {name}: {e}")),
            }
        }
    }

    /// Ctrl+V: imagem da área de transferência (print de tela), se wl-paste ou xclip existirem.
    fn paste_image(&mut self) {
        let attempts: [(&str, &[&str]); 2] = [
            ("wl-paste", &["--type", "image/png"]),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
        ];
        for (cmd, args) in attempts {
            if let Ok(out) = std::process::Command::new(cmd).args(args).output()
                && out.status.success()
                && let Ok(img) = image::load_from_memory(&out.stdout)
            {
                let id = self.ed.add_image("Colada", img.to_rgba8());
                self.selected = Some(id);
                return;
            }
        }
        self.warn("Não achei imagem na área de transferência (precisa de wl-clipboard ou xclip)");
    }

    fn flat(&mut self) -> image::RgbaImage {
        self.renderer.render(self.ed.doc(), self.ed.assets())
    }

    fn export_png(&mut self, mut path: PathBuf) {
        if path.extension().is_none() {
            path.set_extension("png");
        }
        let img = self.flat();
        match img.save(&path) {
            Ok(()) => {
                let kb = std::fs::metadata(&path).map_or(0, |m| m.len() / 1000);
                if kb * 1000 > YOUTUBE_MAX as u64 {
                    self.warn(format!(
                        "PNG exportado ({kb} KB), mas o YouTube só aceita até 2 MB: use o JPG"
                    ));
                } else {
                    self.say(format!("PNG exportado ({kb} KB)"));
                }
            }
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn export_jpg(&mut self, mut path: PathBuf) {
        if path.extension().is_none() {
            path.set_extension("jpg");
        }
        let img = self.flat();
        let (bytes, quality) = pawpaint_render::jpeg_under(&img, YOUTUBE_MAX);
        match std::fs::write(&path, &bytes) {
            Ok(()) => self.say(format!(
                "JPG exportado: {} KB, qualidade {quality} (limite do YouTube: 2 MB)",
                bytes.len() / 1000
            )),
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn guard_new(&mut self) {
        self.dialog = Some(Dialog::New {
            w: self.ed.doc().width,
            h: self.ed.doc().height,
        });
    }

    fn new_doc(&mut self, w: u32, h: u32) {
        self.ed = Editor::new(w, h);
        self.path = None;
        self.selected = None;
        self.fit = true;
        self.rendered = None;
    }

    // ---------------------------------------------------------------- edição

    fn nudge(&mut self, dx: f32, dy: f32) {
        if let Some(id) = self.selected {
            self.ed.edit(id, "nudge", "Mover", |l| {
                l.x += dx;
                l.y += dy;
            });
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.ed.delete(id);
        }
    }

    fn add_text(&mut self) {
        let id = self.ed.add_text("TEXTO");
        self.selected = Some(id);
        self.focus_text = true;
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let ctrl = Modifiers::COMMAND;
        let hit = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let typing = ctx.egui_wants_keyboard_input();
        if hit(ctrl | Modifiers::SHIFT, Key::S) {
            self.ask(Purpose::SaveAs);
        } else if hit(ctrl, Key::S) {
            self.save();
        }
        if hit(ctrl, Key::N) {
            self.guard_new();
        }
        if hit(ctrl, Key::O) {
            self.ask(Purpose::Open);
        }
        if hit(ctrl, Key::I) {
            self.ask(Purpose::Import);
        }
        if hit(ctrl, Key::E) {
            self.ask(Purpose::ExportJpg);
        }
        if hit(ctrl | Modifiers::SHIFT, Key::E) {
            self.ask(Purpose::ExportPng);
        }
        if hit(ctrl, Key::Num0) {
            self.fit = true;
            self.pan = Vec2::ZERO;
        }
        if hit(ctrl, Key::Num1) {
            self.fit = false;
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        if typing {
            return;
        }
        if hit(ctrl | Modifiers::SHIFT, Key::Z) || hit(ctrl, Key::Y) {
            self.ed.redo();
        } else if hit(ctrl, Key::Z) {
            self.ed.undo();
        }
        if hit(ctrl, Key::D)
            && let Some(id) = self.selected
        {
            self.selected = self.ed.duplicate(id);
        }
        if hit(ctrl, Key::V) {
            self.paste_image();
        }
        if hit(Modifiers::NONE, Key::T) {
            self.add_text();
        }
        if hit(Modifiers::NONE, Key::Delete) || hit(Modifiers::NONE, Key::Backspace) {
            self.delete_selected();
        }
        if hit(Modifiers::NONE, Key::Escape) {
            self.selected = None;
        }
        if hit(Modifiers::NONE, Key::G) {
            self.guides = !self.guides;
        }
        for (key, dx, dy) in [
            (Key::ArrowLeft, -1.0, 0.0),
            (Key::ArrowRight, 1.0, 0.0),
            (Key::ArrowUp, 0.0, -1.0),
            (Key::ArrowDown, 0.0, 1.0),
        ] {
            if hit(Modifiers::NONE, key) {
                self.nudge(dx, dy);
            }
            if hit(Modifiers::SHIFT, key) {
                self.nudge(dx * 10.0, dy * 10.0);
            }
        }
        if let Some(id) = self.selected {
            if hit(Modifiers::NONE, Key::CloseBracket) {
                self.ed.reorder(id, MoveTo::Up);
            }
            if hit(Modifiers::NONE, Key::OpenBracket) {
                self.ed.reorder(id, MoveTo::Down);
            }
        }
    }

    // ---------------------------------------------------------------- menus

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::Button::new(label).shortcut_text(keys.to_string()),
            )
            .clicked()
        };
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Nova…", "Ctrl+N", true) {
                    ui.close();
                    self.guard_new();
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    self.ask(Purpose::Open);
                }
                if item(ui, "Salvar", "Ctrl+S", true) {
                    ui.close();
                    self.save();
                }
                if item(ui, "Salvar como…", "Ctrl+Shift+S", true) {
                    ui.close();
                    self.ask(Purpose::SaveAs);
                }
                ui.separator();
                if item(ui, "Importar imagem…", "Ctrl+I", true) {
                    ui.close();
                    self.ask(Purpose::Import);
                }
                if item(ui, "Colar imagem", "Ctrl+V", true) {
                    ui.close();
                    self.paste_image();
                }
                ui.separator();
                if item(
                    ui,
                    "Exportar JPG para o YouTube (até 2 MB)…",
                    "Ctrl+E",
                    true,
                ) {
                    ui.close();
                    self.ask(Purpose::ExportJpg);
                }
                if item(ui, "Exportar PNG…", "Ctrl+Shift+E", true) {
                    ui.close();
                    self.ask(Purpose::ExportPng);
                }
                ui.separator();
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = self
                    .ed
                    .undo_label()
                    .map_or("Desfazer".to_string(), |l| format!("Desfazer {l}"));
                if item(ui, &undo, "Ctrl+Z", self.ed.undo_label().is_some()) {
                    ui.close();
                    self.ed.undo();
                }
                let redo = self
                    .ed
                    .redo_label()
                    .map_or("Refazer".to_string(), |l| format!("Refazer {l}"));
                if item(ui, &redo, "Ctrl+Y", self.ed.redo_label().is_some()) {
                    ui.close();
                    self.ed.redo();
                }
                ui.separator();
                let sel = self.selected;
                if item(ui, "Duplicar camada", "Ctrl+D", sel.is_some()) {
                    ui.close();
                    if let Some(id) = sel {
                        self.selected = self.ed.duplicate(id);
                    }
                }
                if item(ui, "Apagar camada", "Delete", sel.is_some()) {
                    ui.close();
                    self.delete_selected();
                }
            });
            ui.menu_button("Camada", |ui| {
                if item(ui, "Novo texto", "T", true) {
                    ui.close();
                    self.add_text();
                }
                if item(ui, "Novo retângulo", "", true) {
                    ui.close();
                    self.selected = Some(self.ed.add_shape(ShapeKind::Rect));
                }
                if item(ui, "Nova elipse", "", true) {
                    ui.close();
                    self.selected = Some(self.ed.add_shape(ShapeKind::Ellipse));
                }
                ui.separator();
                let sel = self.selected;
                for (label, keys, to) in [
                    ("Trazer para frente", "]", MoveTo::Up),
                    ("Enviar para trás", "[", MoveTo::Down),
                    ("Para o topo", "", MoveTo::Top),
                    ("Para o fundo", "", MoveTo::Bottom),
                ] {
                    if item(ui, label, keys, sel.is_some()) {
                        ui.close();
                        if let Some(id) = sel {
                            self.ed.reorder(id, to);
                        }
                    }
                }
                if item(ui, "Espelhar na horizontal", "", sel.is_some())
                    && let Some(id) = sel
                {
                    ui.close();
                    self.ed
                        .edit(id, "flip", "Espelhar", |l| l.flip_h = !l.flip_h);
                    self.ed.end_change();
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Ajustar à janela", "Ctrl+0", true) {
                    ui.close();
                    self.fit = true;
                    self.pan = Vec2::ZERO;
                }
                if item(ui, "Tamanho real (100%)", "Ctrl+1", true) {
                    ui.close();
                    self.fit = false;
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                ui.separator();
                let label = if self.guides {
                    "Esconder guias de segurança"
                } else {
                    "Mostrar guias de segurança"
                };
                if item(ui, label, "G", true) {
                    ui.close();
                    self.guides = !self.guides;
                }
            });
        });
    }

    // ---------------------------------------------------------------- camadas

    fn layers_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("Camadas").strong());
        ui.add_space(4.0);
        if self.ed.doc().layers.is_empty() {
            ui.label(
                RichText::new("Importe uma imagem (Ctrl+I), arraste um arquivo para cá ou aperte T para pôr um texto.")
                    .weak()
                    .small(),
            );
        }
        let ids: Vec<LayerId> = self.ed.doc().layers.iter().rev().map(|l| l.id).collect();
        let list_h = (ui.available_height() - 330.0).max(100.0);
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink(false)
            .show(ui, |ui| {
                for id in ids {
                    self.layer_row(ui, id);
                }
            });
        ui.add_space(8.0);
        ui.separator();
        self.previews_ui(ui);
    }

    fn layer_row(&mut self, ui: &mut egui::Ui, id: LayerId) {
        let Some(layer) = self.ed.doc().layer(id).cloned() else {
            return;
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        let selected = self.selected == Some(id);
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, 3.0, theme::ACCENT.gamma_multiply(0.35));
        } else if resp.hovered() {
            p.rect_filled(rect, 3.0, theme::BAR_LIGHT);
        }
        // Olho (visível) e cadeado.
        let eye = Rect::from_min_size(rect.min + vec2(4.0, 3.0), vec2(20.0, 20.0));
        let lock = Rect::from_min_size(rect.right_top() + vec2(-24.0, 3.0), vec2(20.0, 20.0));
        let eye_resp = ui.interact(eye, ui.id().with(("eye", id)), Sense::click());
        let lock_resp = ui.interact(lock, ui.id().with(("lock", id)), Sense::click());
        let p = ui.painter();
        // Olho: cheio = visível, só contorno = escondido. Cadeado: desenhado, só aparece travado.
        let ec = eye.center();
        if layer.visible {
            p.circle_filled(ec, 5.0, theme::TEXT);
        } else {
            p.circle_stroke(ec, 5.0, Stroke::new(1.5, theme::TEXT_DIM));
        }
        if layer.locked {
            let lc = lock.center();
            let body = Rect::from_center_size(lc + vec2(0.0, 2.0), vec2(10.0, 7.0));
            p.rect_filled(body, 1.5, theme::TEXT);
            let st = Stroke::new(1.5, theme::TEXT);
            p.line_segment(
                [pos2(lc.x - 3.0, lc.y - 1.0), pos2(lc.x - 3.0, lc.y - 4.0)],
                st,
            );
            p.line_segment(
                [pos2(lc.x + 3.0, lc.y - 1.0), pos2(lc.x + 3.0, lc.y - 4.0)],
                st,
            );
            p.line_segment(
                [pos2(lc.x - 3.0, lc.y - 4.0), pos2(lc.x + 3.0, lc.y - 4.0)],
                st,
            );
        }
        // Ícone do tipo.
        let icon = Rect::from_min_size(rect.min + vec2(30.0, 5.0), vec2(16.0, 16.0));
        match &layer.kind {
            Kind::Text(_) => {
                p.text(
                    icon.center(),
                    Align2::CENTER_CENTER,
                    "T",
                    theme::text(13.0),
                    theme::TEXT,
                );
            }
            Kind::Shape(s) => match s.kind {
                ShapeKind::Rect => {
                    p.rect_filled(icon.shrink(1.0), 2.0, to_color(s.fill));
                }
                ShapeKind::Ellipse => {
                    p.circle_filled(icon.center(), 7.0, to_color(s.fill));
                }
            },
            Kind::Image { .. } => {
                p.rect_stroke(
                    icon.shrink(1.0),
                    2.0,
                    Stroke::new(1.5, theme::TEXT),
                    egui::StrokeKind::Inside,
                );
                p.circle_filled(icon.center() - vec2(2.0, 2.0), 2.0, theme::TEXT);
            }
        }
        let name_pos = rect.min + vec2(54.0, 13.0);
        let renaming = matches!(&self.renaming, Some((rid, _) ) if *rid == id);
        if renaming {
            let mut done = false;
            if let Some((_, text)) = &mut self.renaming {
                let r = ui.put(
                    Rect::from_min_size(
                        rect.min + vec2(52.0, 2.0),
                        vec2(rect.width() - 86.0, 22.0),
                    ),
                    egui::TextEdit::singleline(text),
                );
                r.request_focus();
                done = r.lost_focus();
            }
            if done && let Some((rid, text)) = self.renaming.take() {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    self.ed
                        .edit(rid, "name", "Renomear camada", |l| l.name = name);
                    self.ed.end_change();
                }
            }
        } else {
            ui.painter()
                .with_clip_rect(rect.shrink2(vec2(54.0, 0.0)).intersect(rect))
                .text(
                    name_pos,
                    Align2::LEFT_CENTER,
                    &layer.name,
                    theme::text(12.5),
                    if layer.visible {
                        theme::TEXT
                    } else {
                        theme::TEXT_DIM
                    },
                );
        }
        if eye_resp.clicked() {
            self.ed
                .edit(id, "vis", "Mostrar/esconder", |l| l.visible = !l.visible);
            self.ed.end_change();
        } else if lock_resp.clicked() {
            self.ed.edit(id, "lock", "Travar", |l| l.locked = !l.locked);
            self.ed.end_change();
        } else if resp.clicked() {
            self.selected = Some(id);
        }
        if resp.double_clicked() {
            self.renaming = Some((id, layer.name.clone()));
        }
        resp.context_menu(|ui| {
            self.selected = Some(id);
            if ui.button("Renomear").clicked() {
                ui.close();
                self.renaming = Some((id, layer.name.clone()));
            }
            if ui.button("Duplicar").clicked() {
                ui.close();
                self.selected = self.ed.duplicate(id);
            }
            if ui.button("Trazer para frente").clicked() {
                ui.close();
                self.ed.reorder(id, MoveTo::Up);
            }
            if ui.button("Enviar para trás").clicked() {
                ui.close();
                self.ed.reorder(id, MoveTo::Down);
            }
            if ui.button("Apagar").clicked() {
                ui.close();
                self.selected = None;
                self.ed.delete(id);
            }
        });
    }

    /// A thumbnail como o espectador vê: grande na busca, pequena na lista, com a hora do vídeo
    /// por cima do canto (onde o texto não pode ficar).
    fn previews_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(RichText::new("Como vai aparecer").strong());
        ui.add_space(4.0);
        let Some(big) = self.previews.first().cloned() else {
            return;
        };
        let small = self.previews.get(1).cloned();
        let aspect = self.ed.doc().width as f32 / self.ed.doc().height as f32;
        let width = ui.available_width() - 8.0;
        let draw = |ui: &mut egui::Ui, tex: &egui::TextureHandle, w: f32| {
            let size = vec2(w, w / aspect);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            // Selo de duração, como no YouTube.
            let badge = Rect::from_min_size(
                rect.right_bottom() - vec2((w * 0.17).max(26.0) + 3.0, (w * 0.075).max(12.0) + 3.0),
                vec2((w * 0.17).max(26.0), (w * 0.075).max(12.0)),
            );
            ui.painter()
                .rect_filled(badge, 2.0, Color32::from_black_alpha(210));
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                "12:34",
                theme::text((w * 0.05).clamp(8.0, 11.0)),
                Color32::WHITE,
            );
        };
        draw(ui, &big, width.min(260.0));
        if let Some(small) = small {
            ui.add_space(4.0);
            draw(ui, &small, 120.0);
        }
    }

    // ---------------------------------------------------------------- propriedades

    fn properties_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let Some(layer) = self.layer().cloned() else {
            self.background_ui(ui);
            return;
        };
        let id = layer.id;
        ui.label(RichText::new(&layer.name).strong());
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Posição e tamanho")
                        .small()
                        .color(theme::TEXT_DIM),
                );
                let mut l = layer.clone();
                let mut changed = false;
                egui::Grid::new("xform").num_columns(2).show(ui, |ui| {
                    ui.label("X");
                    changed |= ui
                        .add(egui::DragValue::new(&mut l.x).speed(1.0).suffix(" px"))
                        .changed();
                    ui.end_row();
                    ui.label("Y");
                    changed |= ui
                        .add(egui::DragValue::new(&mut l.y).speed(1.0).suffix(" px"))
                        .changed();
                    ui.end_row();
                    ui.label("Escala");
                    let mut pct = l.scale * 100.0;
                    if ui
                        .add(
                            egui::DragValue::new(&mut pct)
                                .speed(0.5)
                                .range(2.0..=1600.0)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        l.scale = pct / 100.0;
                        changed = true;
                    }
                    ui.end_row();
                    ui.label("Giro");
                    changed |= ui
                        .add(egui::DragValue::new(&mut l.rotation).speed(0.5).suffix("°"))
                        .changed();
                    ui.end_row();
                    ui.label("Opacidade");
                    let mut pct = l.opacity * 100.0;
                    if ui
                        .add(
                            egui::DragValue::new(&mut pct)
                                .speed(0.5)
                                .range(0.0..=100.0)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        l.opacity = pct / 100.0;
                        changed = true;
                    }
                    ui.end_row();
                });
                ui.add_space(8.0);
                match &mut l.kind {
                    Kind::Text(t) => {
                        ui.label(RichText::new("Texto").small().color(theme::TEXT_DIM));
                        let te = ui.add(
                            egui::TextEdit::multiline(&mut t.text)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                        if self.focus_text {
                            te.request_focus();
                            self.focus_text = false;
                        }
                        changed |= te.changed();
                        if te.lost_focus() {
                            self.ed.end_change();
                        }
                        egui::Grid::new("textprops").num_columns(2).show(ui, |ui| {
                            ui.label("Fonte");
                            if self.fonts.is_empty() {
                                self.fonts = pawpaint_render::font_families();
                            }
                            egui::ComboBox::from_id_salt("font")
                                .selected_text(&t.font)
                                .width(170.0)
                                .show_ui(ui, |ui| {
                                    for f in ["sans-serif", "serif", "monospace"] {
                                        changed |= ui
                                            .selectable_value(&mut t.font, f.to_string(), f)
                                            .changed();
                                    }
                                    ui.separator();
                                    for f in &self.fonts {
                                        changed |= ui
                                            .selectable_value(&mut t.font, f.clone(), f)
                                            .changed();
                                    }
                                });
                            ui.end_row();
                            ui.label("Tamanho");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut t.size)
                                        .speed(1.0)
                                        .range(8.0..=800.0),
                                )
                                .changed();
                            ui.end_row();
                            ui.label("Estilo");
                            ui.horizontal(|ui| {
                                changed |= ui.toggle_value(&mut t.bold, "Negrito").changed();
                                changed |= ui.toggle_value(&mut t.italic, "Itálico").changed();
                            });
                            ui.end_row();
                            ui.label("Alinhar");
                            ui.horizontal(|ui| {
                                for (a, name) in [
                                    (TextAlign::Left, "Esq."),
                                    (TextAlign::Center, "Centro"),
                                    (TextAlign::Right, "Dir."),
                                ] {
                                    changed |= ui.selectable_value(&mut t.align, a, name).changed();
                                }
                            });
                            ui.end_row();
                            ui.label("Cor");
                            changed |= color_button(ui, &mut t.color, false);
                            ui.end_row();
                            ui.label("Entrelinha");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut t.line_spacing)
                                        .speed(0.01)
                                        .range(0.5..=3.0),
                                )
                                .changed();
                            ui.end_row();
                        });
                    }
                    Kind::Shape(s) => {
                        ui.label(RichText::new("Forma").small().color(theme::TEXT_DIM));
                        egui::Grid::new("shapeprops").num_columns(2).show(ui, |ui| {
                            ui.label("Largura");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.w)
                                        .speed(1.0)
                                        .range(4.0..=4000.0),
                                )
                                .changed();
                            ui.end_row();
                            ui.label("Altura");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.h)
                                        .speed(1.0)
                                        .range(4.0..=4000.0),
                                )
                                .changed();
                            ui.end_row();
                            ui.label("Preenchimento");
                            changed |= color_button(ui, &mut s.fill, true);
                            ui.end_row();
                            ui.label("Borda");
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut s.stroke_width)
                                        .speed(0.5)
                                        .range(0.0..=200.0),
                                )
                                .changed();
                            ui.end_row();
                            if s.stroke_width > 0.0 {
                                ui.label("Cor da borda");
                                changed |= color_button(ui, &mut s.stroke, true);
                                ui.end_row();
                            }
                            if s.kind == ShapeKind::Rect {
                                ui.label("Cantos");
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut s.radius)
                                            .speed(0.5)
                                            .range(0.0..=1000.0),
                                    )
                                    .changed();
                                ui.end_row();
                            }
                        });
                    }
                    Kind::Image { .. } => {
                        ui.label(RichText::new("Imagem").small().color(theme::TEXT_DIM));
                        let mut remove = l.adjust.chroma.is_some();
                        if ui
                            .checkbox(&mut remove, "Tirar uma cor (fundo verde, branco…)")
                            .changed()
                        {
                            l.adjust.chroma = remove.then(|| Chroma {
                                color: Rgba::rgb(0, 255, 0),
                                tolerance: 0.35,
                                softness: 0.2,
                            });
                            changed = true;
                        }
                        if let Some(ch) = &mut l.adjust.chroma {
                            egui::Grid::new("chroma").num_columns(2).show(ui, |ui| {
                                ui.label("Cor");
                                changed |= color_button(ui, &mut ch.color, false);
                                ui.end_row();
                                ui.label("Tolerância");
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ch.tolerance, 0.0..=1.0)
                                            .show_value(false),
                                    )
                                    .changed();
                                ui.end_row();
                                ui.label("Suavizar");
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ch.softness, 0.0..=1.0)
                                            .show_value(false),
                                    )
                                    .changed();
                                ui.end_row();
                            });
                        }
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Contorno").small().color(theme::TEXT_DIM));
                let mut has = l.outline.is_some();
                if ui.checkbox(&mut has, "Contorno de adesivo").changed() {
                    l.outline = has.then_some(Outline {
                        width: 10.0,
                        color: Rgba::BLACK,
                    });
                    changed = true;
                }
                if let Some(o) = &mut l.outline {
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut o.width)
                                    .speed(0.5)
                                    .range(0.0..=200.0)
                                    .suffix(" px"),
                            )
                            .changed();
                        changed |= color_button(ui, &mut o.color, false);
                    });
                }
                let mut has = l.shadow.is_some();
                if ui.checkbox(&mut has, "Sombra").changed() {
                    l.shadow = has.then_some(Shadow {
                        dx: 6.0,
                        dy: 8.0,
                        blur: 8.0,
                        color: Rgba::new(0, 0, 0, 160),
                    });
                    changed = true;
                }
                if let Some(s) = &mut l.shadow {
                    egui::Grid::new("shadow").num_columns(2).show(ui, |ui| {
                        ui.label("Deslocar");
                        ui.horizontal(|ui| {
                            changed |= ui.add(egui::DragValue::new(&mut s.dx).speed(0.5)).changed();
                            changed |= ui.add(egui::DragValue::new(&mut s.dy).speed(0.5)).changed();
                        });
                        ui.end_row();
                        ui.label("Desfoque");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.blur)
                                    .speed(0.2)
                                    .range(0.0..=100.0),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Cor");
                        changed |= color_button(ui, &mut s.color, true);
                        ui.end_row();
                    });
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Cor").small().color(theme::TEXT_DIM));
                egui::Grid::new("adjust").num_columns(2).show(ui, |ui| {
                    for (label, v) in [
                        ("Brilho", &mut l.adjust.brightness),
                        ("Contraste", &mut l.adjust.contrast),
                        ("Saturação", &mut l.adjust.saturation),
                    ] {
                        ui.label(label);
                        let r = ui.add(egui::Slider::new(v, -1.0..=1.0).show_value(false));
                        if r.double_clicked() {
                            *v = 0.0;
                            changed = true;
                        }
                        changed |= r.changed();
                        ui.end_row();
                    }
                });
                if l.adjust != Adjust::default() && ui.small_button("Zerar ajustes").clicked() {
                    l.adjust = Adjust {
                        chroma: l.adjust.chroma,
                        ..Adjust::default()
                    };
                    changed = true;
                }
                if changed {
                    self.ed.edit(id, "props", "Propriedades", |dst| *dst = l);
                }
            });
    }

    fn background_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Fundo").strong());
        ui.add_space(4.0);
        let bg = self.ed.doc().background.clone();
        let (mut solid, mut from, mut to, mut angle) = match &bg {
            Background::Solid(c) => (true, *c, *c, 135.0),
            Background::Gradient { from, to, angle } => (false, *from, *to, *angle),
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui.selectable_value(&mut solid, true, "Cor").changed();
            changed |= ui.selectable_value(&mut solid, false, "Degradê").changed();
        });
        egui::Grid::new("bg").num_columns(2).show(ui, |ui| {
            ui.label(if solid { "Cor" } else { "De" });
            changed |= color_button(ui, &mut from, false);
            ui.end_row();
            if !solid {
                ui.label("Até");
                changed |= color_button(ui, &mut to, false);
                ui.end_row();
                ui.label("Ângulo");
                changed |= ui
                    .add(egui::DragValue::new(&mut angle).speed(1.0).suffix("°"))
                    .changed();
                ui.end_row();
            }
        });
        if changed {
            self.ed.set_background(if solid {
                Background::Solid(from)
            } else {
                Background::Gradient { from, to, angle }
            });
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("Clique numa camada para editar. Atalhos: T texto · Ctrl+I imagem · [ e ] ordem · setas movem · G guias")
                .weak()
                .small(),
        );
    }

    // ---------------------------------------------------------------- tela

    fn render_texture(&mut self, ctx: &egui::Context) {
        let rev = self.ed.revision();
        if self.rendered.as_ref().is_some_and(|(r, _)| *r == rev) {
            return;
        }
        let img = self.flat();
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [img.width() as usize, img.height() as usize],
            img.as_raw(),
        );
        match &mut self.rendered {
            Some((r, tex)) => {
                tex.set(color, egui::TextureOptions::LINEAR);
                *r = rev;
            }
            None => {
                self.rendered = Some((
                    rev,
                    ctx.load_texture("canvas", color, egui::TextureOptions::LINEAR),
                ));
            }
        }
        if self.preview_rev != rev {
            self.preview_rev = rev;
            let mut previews = Vec::new();
            for (i, w) in [260u32, 120].into_iter().enumerate() {
                let h = (w as f32 * img.height() as f32 / img.width() as f32)
                    .round()
                    .max(1.0) as u32;
                let small = image::imageops::resize(
                    &img,
                    w * 2,
                    h * 2,
                    image::imageops::FilterType::Triangle,
                );
                let c = egui::ColorImage::from_rgba_unmultiplied(
                    [small.width() as usize, small.height() as usize],
                    small.as_raw(),
                );
                match self.previews.get_mut(i) {
                    Some(t) => {
                        t.set(c, egui::TextureOptions::LINEAR);
                        previews.push(t.clone());
                    }
                    None => previews.push(ctx.load_texture(
                        format!("preview{i}"),
                        c,
                        egui::TextureOptions::LINEAR,
                    )),
                }
            }
            self.previews = previews;
        }
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, theme::BG_DARK);
        let (cw, ch) = (self.ed.doc().width as f32, self.ed.doc().height as f32);
        if self.fit {
            self.zoom = ((area.width() - 48.0) / cw)
                .min((area.height() - 48.0) / ch)
                .max(0.05);
        }
        let resp = ui.interact(area, ui.id().with("canvas"), Sense::click_and_drag());
        // Roda do mouse: zoom em volta do cursor.
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 300.0);
            if scroll.abs() > 0.1 {
                let old = self.zoom;
                let new = (old * (1.0 + scroll * 0.002)).clamp(0.05, 8.0);
                if let Some(p) = resp.hover_pos() {
                    let center = area.center() + self.pan;
                    let before = (p - center) / old;
                    self.pan += (before * old) - (before * new);
                }
                self.zoom = new;
                self.fit = false;
            }
        }
        let size = vec2(cw, ch) * self.zoom;
        let origin = area.center() + self.pan - size / 2.0;
        let canvas = Rect::from_min_size(origin, size);
        let to_doc = |p: Pos2| ((p - origin) / self.zoom).to_pos2();
        let to_screen = |p: Pos2| origin + p.to_vec2() * self.zoom;
        // Fundo xadrez por trás (mostra transparência) e a imagem.
        ui.painter()
            .rect_filled(canvas, 0.0, Color32::from_gray(40));
        if let Some((_, tex)) = &self.rendered {
            ui.painter().image(
                tex.id(),
                canvas,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        ui.painter().rect_stroke(
            canvas,
            0.0,
            Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Outside,
        );
        let clip = ui.painter().with_clip_rect(area);
        if self.guides {
            // O selo de duração do YouTube cobre o canto de baixo à direita.
            let danger = Rect::from_min_max(
                pos2(
                    canvas.right() - cw * 0.17 * self.zoom,
                    canvas.bottom() - ch * 0.12 * self.zoom,
                ),
                canvas.right_bottom(),
            );
            clip.rect_filled(
                danger,
                0.0,
                Color32::from_rgba_unmultiplied(230, 60, 60, 70),
            );
            clip.text(
                danger.center(),
                Align2::CENTER_CENTER,
                "selo",
                theme::text(10.0),
                Color32::WHITE,
            );
            let safe = canvas.shrink2(vec2(cw * 0.03, ch * 0.05) * self.zoom);
            clip.rect_stroke(
                safe,
                0.0,
                Stroke::new(1.0, Color32::from_white_alpha(70)),
                egui::StrokeKind::Inside,
            );
            for f in [1.0 / 3.0, 2.0 / 3.0] {
                clip.line_segment(
                    [
                        pos2(canvas.left() + canvas.width() * f, canvas.top()),
                        pos2(canvas.left() + canvas.width() * f, canvas.bottom()),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(30)),
                );
                clip.line_segment(
                    [
                        pos2(canvas.left(), canvas.top() + canvas.height() * f),
                        pos2(canvas.right(), canvas.top() + canvas.height() * f),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(30)),
                );
            }
        }
        // Seleção: caixa girada, cantos (escala) e alça de giro.
        let mut handles: Option<(Pos2, [Pos2; 4], Pos2)> = None;
        if let Some(layer) = self.layer().cloned()
            && let Some((w, h)) = self.renderer.extent(&layer, self.ed.assets())
        {
            let (c, s) = (
                layer.rotation.to_radians().cos(),
                layer.rotation.to_radians().sin(),
            );
            let rot = |dx: f32, dy: f32| {
                to_screen(pos2(layer.x + dx * c - dy * s, layer.y + dx * s + dy * c))
            };
            let corners = [
                rot(-w / 2.0, -h / 2.0),
                rot(w / 2.0, -h / 2.0),
                rot(w / 2.0, h / 2.0),
                rot(-w / 2.0, h / 2.0),
            ];
            let top = rot(0.0, -h / 2.0 - 28.0 / self.zoom);
            let center = to_screen(pos2(layer.x, layer.y));
            let stroke = Stroke::new(1.5, theme::ACCENT);
            for i in 0..4 {
                clip.line_segment([corners[i], corners[(i + 1) % 4]], stroke);
            }
            clip.line_segment([rot(0.0, -h / 2.0), top], stroke);
            for c in corners {
                clip.rect_filled(
                    Rect::from_center_size(c, Vec2::splat(HANDLE * 2.0)),
                    2.0,
                    Color32::WHITE,
                );
                clip.rect_stroke(
                    Rect::from_center_size(c, Vec2::splat(HANDLE * 2.0)),
                    2.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            clip.circle_filled(top, HANDLE, Color32::WHITE);
            clip.circle_stroke(top, HANDLE, stroke);
            handles = Some((center, corners, top));
        }
        // Guias de ímã mostradas enquanto arrasta.
        if matches!(self.drag, Some(Drag::Move { .. })) {
            let s = Stroke::new(1.0, Color32::from_rgb(255, 80, 200));
            if self.snap_lines.0 {
                clip.line_segment(
                    [
                        pos2(canvas.center().x, canvas.top()),
                        pos2(canvas.center().x, canvas.bottom()),
                    ],
                    s,
                );
            }
            if self.snap_lines.1 {
                clip.line_segment(
                    [
                        pos2(canvas.left(), canvas.center().y),
                        pos2(canvas.right(), canvas.center().y),
                    ],
                    s,
                );
            }
        }
        // ---- interação
        let pointer = resp.interact_pointer_pos();
        if resp.drag_started() {
            let space = ui.input(|i| i.key_down(Key::Space));
            let middle = ui.input(|i| i.pointer.middle_down());
            self.drag = None;
            if let Some(p) = pointer {
                if space || middle {
                    self.drag = Some(Drag::Pan);
                } else if let Some((center, corners, top)) = handles
                    && p.distance(top) <= HANDLE + 4.0
                {
                    let _ = (center, corners);
                    self.drag = Some(Drag::Rotate);
                } else if let Some((center, corners, _)) = handles
                    && corners.iter().any(|c| p.distance(*c) <= HANDLE + 4.0)
                {
                    let l = self.layer().cloned();
                    if let Some(l) = l {
                        self.drag = Some(Drag::Scale {
                            start_dist: p.distance(center).max(1.0),
                            start_scale: l.scale,
                        });
                    }
                } else {
                    let d = to_doc(p);
                    let hit = self
                        .renderer
                        .layer_at(self.ed.doc(), self.ed.assets(), d.x, d.y);
                    // Clicar no vazio não perde a seleção ao começar a arrastar; só no clique.
                    if let Some(id) = hit {
                        self.selected = Some(id);
                    }
                    if let Some(l) = self.layer().filter(|l| !l.locked).cloned()
                        && hit.is_some()
                    {
                        self.drag = Some(Drag::Move {
                            grab: vec2(d.x - l.x, d.y - l.y),
                        });
                    } else {
                        self.drag = Some(Drag::Pan);
                    }
                }
            }
        }
        if resp.dragged()
            && let (Some(drag), Some(p)) = (self.drag.clone(), pointer)
        {
            match drag {
                Drag::Pan => {
                    self.pan += resp.drag_delta();
                    self.fit = false;
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                Drag::Move { grab } => {
                    let d = to_doc(p);
                    let (mut x, mut y) = (d.x - grab.x, d.y - grab.y);
                    let snap = SNAP_PX / self.zoom;
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.snap_lines = (false, false);
                    if !shift {
                        if (x - cw / 2.0).abs() < snap {
                            x = cw / 2.0;
                            self.snap_lines.0 = true;
                        }
                        if (y - ch / 2.0).abs() < snap {
                            y = ch / 2.0;
                            self.snap_lines.1 = true;
                        }
                    }
                    if let Some(id) = self.selected {
                        self.ed.edit(id, "move", "Mover", |l| {
                            l.x = x.round();
                            l.y = y.round();
                        });
                    }
                }
                Drag::Scale {
                    start_dist,
                    start_scale,
                } => {
                    if let (Some(id), Some((center, _, _))) = (self.selected, handles) {
                        let k = (p.distance(center) / start_dist).max(0.01);
                        self.ed.edit(id, "scale", "Escalar", |l| {
                            l.scale = (start_scale * k).clamp(0.02, 16.0);
                        });
                    }
                }
                Drag::Rotate => {
                    if let (Some(id), Some((center, _, _))) = (self.selected, handles) {
                        let v = p - center;
                        let mut deg = v.y.atan2(v.x).to_degrees() + 90.0;
                        if ui.input(|i| i.modifiers.shift) {
                            deg = (deg / 15.0).round() * 15.0;
                        }
                        self.ed.edit(id, "rotate", "Girar", |l| l.rotation = deg);
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
            self.snap_lines = (false, false);
            self.ed.end_change();
        }
        if resp.clicked()
            && let Some(p) = pointer
        {
            let d = to_doc(p);
            self.selected = self
                .renderer
                .layer_at(self.ed.doc(), self.ed.assets(), d.x, d.y);
        }
        if resp.double_clicked()
            && let Some(l) = self.layer()
            && matches!(l.kind, Kind::Text(_))
        {
            self.focus_text = true;
        }
        if resp.hovered() && self.drag.is_none() {
            let over_handle = handles.is_some_and(|(_, corners, top)| {
                resp.hover_pos().is_some_and(|p| {
                    p.distance(top) <= HANDLE + 4.0
                        || corners.iter().any(|c| p.distance(*c) <= HANDLE + 4.0)
                })
            });
            ui.ctx().set_cursor_icon(if over_handle {
                CursorIcon::Crosshair
            } else {
                CursorIcon::Default
            });
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let doc = self.ed.doc();
        let text = format!(
            "{} × {} px · {:.0}% · {} camada(s)",
            doc.width,
            doc.height,
            self.zoom * 100.0,
            doc.layers.len()
        );
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(text).small());
            if self.ed.is_dirty() {
                ui.label(RichText::new("não salvo").small().color(theme::TEXT_DIM));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        let mut then: Option<Action> = None;
        let title = match &dialog {
            Dialog::New { .. } => "Nova thumbnail",
            Dialog::ConfirmClose => "Sair sem salvar?",
        };
        let mut dialog = dialog;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| match &mut dialog {
                Dialog::New { w, h } => {
                    ui.horizontal(|ui| {
                        for (label, (pw, ph)) in [
                            ("YouTube 1280×720", (1280, 720)),
                            ("Shorts 1080×1920", (1080, 1920)),
                            ("Quadrado 1080", (1080, 1080)),
                        ] {
                            if ui.button(label).clicked() {
                                (*w, *h) = (pw, ph);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(w).range(64..=8192).suffix(" px"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(64..=8192).suffix(" px"));
                    });
                    ui.label(
                        RichText::new("O projeto atual fecha (salve antes se precisar).")
                            .weak()
                            .small(),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Criar").clicked() {
                            keep = false;
                            let (w, h) = (*w, *h);
                            then = Some(Box::new(move |me| me.new_doc(w, h)));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::ConfirmClose => {
                    ui.label("O projeto tem mudanças que não foram salvas.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Salvar").clicked() {
                            keep = false;
                            then = Some(Box::new(|me| {
                                me.save();
                                if me.path.is_some() {
                                    me.allow_close = true;
                                }
                            }));
                        }
                        if ui.button("Descartar").clicked() {
                            keep = false;
                            then = Some(Box::new(|me| me.allow_close = true));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
            });
        if keep {
            self.dialog = Some(dialog);
        }
        if let Some(f) = then {
            f(self);
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_file();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            let (projects, images): (Vec<_>, Vec<_>) = dropped
                .into_iter()
                .partition(|p| p.extension().is_some_and(|e| e == "pawpaint"));
            if let Some(p) = projects.first() {
                self.open_project(p);
            }
            if !images.is_empty() {
                self.import(images);
            }
        }
        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }
        // Fim de um gesto (soltou o mouse ou uma tecla): o próximo vira outro passo de desfazer.
        if ctx.input(|i| {
            i.pointer.any_released()
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { pressed: false, .. }))
        }) {
            self.ed.end_change();
        }
        if self
            .selected
            .is_some_and(|id| self.ed.doc().layer(id).is_none())
        {
            self.selected = None;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.ed.is_dirty() && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::ConfirmClose);
        }
        if self.allow_close && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".into(), |s| s.to_string_lossy().into_owned());
        let title = format!(
            "{}{name} — PawPaint",
            if self.ed.is_dirty() { "• " } else { "" }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.render_texture(&ctx);
        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        let panel = egui::Frame::new()
            .fill(theme::BAR)
            .inner_margin(egui::Margin::same(8));
        egui::Panel::left("layers")
            .exact_size(280.0)
            .resizable(false)
            .frame(panel)
            .show(ui, |ui| self.layers_panel(ui));
        egui::Panel::right("props")
            .exact_size(310.0)
            .resizable(false)
            .frame(panel)
            .show(ui, |ui| self.properties_panel(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas_ui(ui));
        self.dialogs(&ctx);
        self.toasts.tick(&ctx);
        self.toasts.ui(&ctx);
    }

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        if self.frames == 40 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Some(buf) =
                image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
            {
                let _ = buf.save(&path);
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

/// Botão de cor; `alpha` deixa mexer na transparência também.
fn color_button(ui: &mut egui::Ui, c: &mut Rgba, alpha: bool) -> bool {
    let mut col = to_color(*c);
    let mode = if alpha {
        egui::color_picker::Alpha::OnlyBlend
    } else {
        egui::color_picker::Alpha::Opaque
    };
    let changed = egui::color_picker::color_edit_button_srgba(ui, &mut col, mode).changed();
    if changed {
        *c = from_color(col);
    }
    changed
}

impl eframe::App for PaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        let ctx = ui.ctx().clone();
        self.screenshot_hook(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desenha a janela inteira com texto, forma e imagem, e com o texto selecionado.
    #[test]
    fn the_whole_window_renders_with_a_selection() {
        let mut app = PaintApp::empty();
        let t = app.ed.add_text("TESTE");
        app.ed.add_shape(ShapeKind::Ellipse);
        app.ed.add_image(
            "foto",
            image::RgbaImage::from_pixel(50, 50, image::Rgba([200, 50, 50, 255])),
        );
        app.selected = Some(t);
        app.guides = true;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        for i in 0..3 {
            if i == 1 {
                app.selected = None;
            }
            if i == 2 {
                app.dialog = Some(Dialog::New { w: 1280, h: 720 });
            }
            let out = ctx.run_ui(Default::default(), |ui| app.frame(ui));
            out.drop_without_applying_deltas();
        }
        assert!(app.rendered.is_some());
        assert_eq!(app.previews.len(), 2);
    }

    #[test]
    fn exporting_jpg_respects_the_youtube_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = PaintApp::empty();
        app.ed.add_text("OI");
        let path = dir.path().join("t.jpg");
        app.export_jpg(path.clone());
        let size = std::fs::metadata(&path).unwrap().len() as usize;
        assert!(size > 1000 && size <= YOUTUBE_MAX, "{size}");
        let img = image::open(&path).unwrap();
        assert_eq!((img.width(), img.height()), (THUMB_W, THUMB_H));
    }

    #[test]
    fn save_and_open_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pawpaint");
        let mut app = PaintApp::empty();
        app.ed.add_text("OI");
        app.save_to(path.clone());
        assert!(!app.ed.is_dirty());
        let mut other = PaintApp::empty();
        other.open_project(&path);
        assert_eq!(other.ed.doc().layers.len(), 1);
    }
}
