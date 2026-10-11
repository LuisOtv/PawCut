//! A janela do PawCut: menus, layout em abas (egui_dock) e a barra Edit/Deliver.

use std::path::PathBuf;

use egui::{Color32, RichText};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};

use crate::config;
use crate::keymap::Action;
use crate::panels::{DragPayload, Panel};
use crate::state::{AppState, Tool};
use crate::workspace::{Pending, Workspace};

pub fn default_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::TimelineViewer]);
    let tree = dock.main_surface_mut();
    let [top, _] = tree.split_below(NodeIndex::root(), 0.56, vec![Panel::Timeline]);
    let [center, _] = tree.split_right(top, 0.79, vec![Panel::Inspector]);
    // No split_left/split_above a fração vai para o nó novo (o da esquerda/de cima).
    let [center, pool] = tree.split_left(center, 0.24, vec![Panel::MediaPool]);
    tree.split_below(pool, 0.62, vec![Panel::Effects]);
    tree.split_left(center, 0.5, vec![Panel::SourceViewer]);
    dock
}

fn load_layout() -> DockState<Panel> {
    std::fs::read_to_string(config::layout_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(default_layout)
}

impl TabViewer for Workspace {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("pawcut-tab", *tab))
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        // Uma aba anterior pode ter editado o projeto neste mesmo frame.
        self.refresh_project_arc();
        match tab {
            Panel::MediaPool => self.media_pool_ui(ui),
            Panel::Effects => self.effects_ui(ui),
            Panel::SourceViewer => self.source_viewer_ui(ui),
            Panel::TimelineViewer => self.timeline_viewer_ui(ui),
            Panel::Inspector => self.inspector_ui(ui),
            Panel::Timeline => self.timeline_ui(ui),
        }
    }
}

pub struct PawCutApp {
    dock: DockState<Panel>,
    ws: Workspace,
    last_title: String,
    /// Para testes visuais: `PAWCUT_SCREENSHOT=arquivo.png` salva a janela e fecha.
    screenshot: Option<PathBuf>,
    frames: u64,
}

impl PawCutApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_gray(27);
        visuals.window_fill = Color32::from_gray(32);
        visuals.selection.bg_fill = Color32::from_rgb(200, 120, 30);
        ctx.set_visuals(visuals);
        let mut ws = Workspace::new(
            ctx,
            AppState::new(pawcut_core::Project::new(Default::default())),
        );
        if !args.is_empty() {
            ws.open_paths(args);
        }
        Self {
            dock: load_layout(),
            ws,
            last_title: String::new(),
            screenshot: std::env::var_os("PAWCUT_SCREENSHOT").map(PathBuf::from),
            frames: 0,
        }
    }

    #[cfg(test)]
    pub fn workspace(&self) -> &Workspace {
        &self.ws
    }

    fn save_layout(&self) {
        if let Ok(text) = serde_json::to_string(&self.dock) {
            let _ = std::fs::create_dir_all(config::config_dir());
            let _ = std::fs::write(config::layout_path(), text);
        }
    }

    fn item(&mut self, ui: &mut egui::Ui, label: &str, action: Action) {
        let shortcut = self
            .ws
            .keymap
            .chords(action)
            .first()
            .map(|c| c.display())
            .unwrap_or_default();
        if ui
            .add(egui::Button::new(label).shortcut_text(shortcut))
            .clicked()
        {
            ui.close();
            self.ws.run_action(action);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                self.item(ui, "Novo projeto…", Action::NewProject);
                self.item(ui, "Abrir…", Action::Open);
                self.item(ui, "Salvar", Action::Save);
                self.item(ui, "Salvar como…", Action::SaveAs);
                ui.separator();
                self.item(ui, "Importar mídia…", Action::Import);
                ui.separator();
                self.item(ui, "Exportar (Deliver)…", Action::Deliver);
                ui.separator();
                if ui.button("Sair").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                self.item(ui, "Desfazer", Action::Undo);
                self.item(ui, "Refazer", Action::Redo);
                ui.separator();
                self.item(ui, "Cortar no cursor", Action::BladeAtPlayhead);
                self.item(ui, "Apagar", Action::Delete);
                self.item(ui, "Apagar com ripple", Action::RippleDelete);
                self.item(ui, "Ligar / separar áudio e vídeo", Action::LinkUnlink);
                ui.separator();
                self.item(ui, "Insert do Source", Action::InsertEdit);
                self.item(ui, "Overwrite do Source", Action::OverwriteEdit);
                self.item(ui, "Append do Source", Action::AppendEdit);
                ui.separator();
                self.item(ui, "Selecionar tudo", Action::SelectAll);
                self.item(ui, "Tirar seleção", Action::Deselect);
                ui.separator();
                if ui.button("Preferences — atalhos de teclado…").clicked() {
                    ui.close();
                    self.ws.prefs_open = true;
                }
            });
            ui.menu_button("Timeline", |ui| {
                self.item(ui, "Ferramenta de seleção", Action::SelectionTool);
                self.item(ui, "Ferramenta lâmina", Action::BladeTool);
                self.item(ui, "Snapping", Action::ToggleSnapping);
                self.item(ui, "Seleção ligada", Action::ToggleLinkedSelection);
                ui.separator();
                if ui.button("Adicionar faixa de vídeo").clicked() {
                    ui.close();
                    self.ws
                        .st
                        .apply(pawcut_core::Edit::AddTrack(pawcut_core::TrackKind::Video));
                }
                if ui.button("Adicionar faixa de áudio").clicked() {
                    ui.close();
                    self.ws
                        .st
                        .apply(pawcut_core::Edit::AddTrack(pawcut_core::TrackKind::Audio));
                }
                ui.separator();
                self.item(ui, "Zoom para caber", Action::ZoomToFit);
            });
            ui.menu_button("Reprodução", |ui| {
                self.item(ui, "Tocar / pausar", Action::PlayPause);
                self.item(ui, "Para trás", Action::PlayReverse);
                self.item(ui, "Parar", Action::Stop);
                self.item(ui, "Para frente", Action::PlayForward);
                ui.separator();
                self.item(ui, "Marcar In", Action::MarkIn);
                self.item(ui, "Marcar Out", Action::MarkOut);
                self.item(ui, "Limpar In/Out", Action::ClearMarks);
            });
            ui.menu_button("Workspace", |ui| {
                for panel in Panel::ALL {
                    let mut shown = self.dock.find_tab(&panel).is_some();
                    if ui.checkbox(&mut shown, panel.title()).changed() {
                        match self.dock.find_tab(&panel) {
                            Some(path) => {
                                self.dock.remove_tab(path);
                            }
                            None => self.dock.push_to_first_leaf(panel),
                        }
                    }
                }
                ui.separator();
                if ui.button("Reset UI Layout").clicked() {
                    ui.close();
                    self.dock = default_layout();
                }
            });
        });
    }

    fn page_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let _ =
                ui.selectable_label(self.ws.deliver.is_none(), RichText::new("✂ Edit").strong());
            if ui
                .selectable_label(
                    self.ws.deliver.is_some(),
                    RichText::new("🚀 Deliver").strong(),
                )
                .clicked()
            {
                let running = self.ws.deliver.as_ref().is_some_and(|d| d.job.is_some());
                if running {
                    // Exportação em andamento: a janela fica aberta (feche pelo X dela para cancelar).
                } else if self.ws.deliver.is_some() {
                    self.ws.deliver = None
                } else {
                    self.ws.open_deliver()
                }
            }
            ui.separator();
            let tool = match self.ws.st.tool {
                Tool::Selection => "Seleção",
                Tool::Blade => "Lâmina",
            };
            ui.label(RichText::new(format!("Ferramenta: {tool}")).weak());
            if let Some(msg) = &self.ws.st.status {
                ui.separator();
                ui.label(RichText::new(msg).color(Color32::from_rgb(230, 200, 120)));
            }
            if !self
                .ws
                .audio
                .has_device
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                ui.separator();
                ui.label(RichText::new("sem saída de áudio").weak());
            }
        });
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            let exporting = self.ws.deliver.as_ref().is_some_and(|d| d.job.is_some());
            if exporting && !self.ws.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ws.export_quit_prompt = true;
            } else if self.ws.st.is_dirty() && !self.ws.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ws.confirm = Some(Pending::Quit);
            } else {
                self.save_layout();
                self.ws.stop();
                if !self.ws.st.is_dirty() {
                    self.ws.discard_autosave();
                }
            }
        }
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        self.frames += 1;
        // PAWCUT_ACTIONS="SelectAll,BladeTool": ações executadas antes da captura (depuração).
        if self.frames == 30
            && let Ok(list) = std::env::var("PAWCUT_ACTIONS")
        {
            for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                if let Ok(action) = serde_json::from_str::<Action>(&format!("\"{name}\"")) {
                    self.ws.run_action(action);
                }
            }
            if let Ok(f) = std::env::var("PAWCUT_PLAYHEAD")
                && let Ok(f) = f.parse()
            {
                self.ws.st.set_playhead(f);
            }
        }
        if self.frames == 90 {
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
            self.ws.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for PawCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ws.begin_frame(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("pages").show(ui, |ui| self.page_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let style = Style::from_egui(ui.style().as_ref());
            DockArea::new(&mut self.dock)
                .style(style)
                .show_leaf_collapse_buttons(false)
                .show_inside(ui, &mut self.ws);
        });
        self.ws.dialogs_ui(&ctx);

        // Rótulo que acompanha o ponteiro ao arrastar do Media Pool / Toolbox.
        if let Some(payload) = egui::DragAndDrop::payload::<DragPayload>(&ctx)
            && let Some(pos) = ctx.pointer_hover_pos()
        {
            let text = match *payload {
                DragPayload::Media(m) => self
                    .ws
                    .project_arc
                    .media(m)
                    .and_then(|m| {
                        m.info
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default(),
                DragPayload::Generator(g) => format!("{g:?}"),
            };
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("dnd-label"),
            ));
            let r = painter.text(
                pos + egui::vec2(14.0, 8.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
            painter.rect_filled(r.expand(4.0), 3.0, Color32::from_black_alpha(160));
            painter.text(
                pos + egui::vec2(14.0, 8.0),
                egui::Align2::LEFT_TOP,
                "",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }

        let title = self.ws.st.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.handle_close(&ctx);
        self.screenshot(&ctx);
    }
}
