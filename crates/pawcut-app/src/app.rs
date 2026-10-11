//! A janela do PawCut: menus, layout em abas (egui_dock) e a barra Edit/Deliver.

use std::path::PathBuf;

use egui::Color32;
use egui_dock::{DockArea, DockState, TabViewer};

use crate::layout::PanelLayout;

use crate::config;
use crate::icons::{self, Icon};
use crate::keymap::Action;
use crate::panels::{DragPayload, Panel};
use crate::state::AppState;
use crate::theme;
use crate::workspace::{Pending, Workspace};

fn load_layout() -> DockState<Panel> {
    std::fs::read_to_string(config::layout_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(PanelLayout::default_layout)
}

impl TabViewer for Workspace {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("pawcut-tab", *tab))
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    // Como no Resolve: painéis não fecham pelo "X" nem viram janelas soltas;
    // liga e desliga pela barra de cima (assim o lugar deles é lembrado).
    fn is_closeable(&self, _tab: &Panel) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Panel) -> bool {
        false
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
    layout: PanelLayout,
    ws: Workspace,
    last_title: String,
    /// Para testes visuais: `PAWCUT_SCREENSHOT=arquivo.png` salva a janela e fecha.
    screenshot: Option<PathBuf>,
    frames: u64,
}

impl PawCutApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Vec<PathBuf>) -> Self {
        let ctx = &cc.egui_ctx;
        crate::theme::apply(ctx);
        let mut ws = Workspace::new(
            ctx,
            AppState::new(pawcut_core::Project::new(Default::default())),
        );
        if !args.is_empty() {
            ws.open_paths(args);
        }
        Self {
            layout: PanelLayout::new(load_layout()),
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
        if let Ok(text) = serde_json::to_string(&self.layout.dock) {
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
                    let mut shown = self.layout.is_visible(panel);
                    if ui.checkbox(&mut shown, panel.title()).changed() {
                        self.layout.toggle(panel);
                    }
                }
                ui.separator();
                if ui.button("Reset UI Layout").clicked() {
                    ui.close();
                    self.layout.reset();
                }
            });
        });
    }

    fn toggle_panel(&mut self, panel: Panel) {
        self.layout.toggle(panel);
    }

    /// Barra de interface do Resolve: liga/desliga painéis e mostra o nome do projeto.
    fn interface_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.horizontal_centered(|ui| {
            for (panel, icon, label) in [
                (Panel::MediaPool, Icon::MediaPool, "Media Pool"),
                (Panel::Effects, Icon::Effects, "Effects"),
            ] {
                let on = self.layout.is_visible(panel);
                if icons::button(ui, icon, 15.0, on, label).clicked() {
                    self.toggle_panel(panel);
                }
                ui.label(egui::RichText::new(label).size(11.5).color(if on {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }));
                ui.add_space(10.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let on = self.layout.is_visible(Panel::Inspector);
                ui.label(egui::RichText::new("Inspector").size(11.5).color(if on {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }));
                if icons::button(ui, Icon::Inspector, 15.0, on, "Inspector").clicked() {
                    self.toggle_panel(Panel::Inspector);
                }
            });
        });
        let title = self.ws.st.title().replace(" — PawCut", "");
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            title,
            theme::text(13.0),
            theme::TEXT,
        );
    }

    /// Barra de páginas do Resolve (embaixo, centralizada) e a linha de status.
    fn page_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().text(
            rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "PawCut",
            theme::text(14.0),
            theme::TEXT_DIM,
        );
        let mut status = String::new();
        if let Some(msg) = &self.ws.st.status {
            status = msg.clone();
        }
        if !self
            .ws
            .audio
            .has_device
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            status = format!("{status}  ·  sem saída de áudio");
        }
        ui.painter().text(
            rect.right_center() - egui::vec2(12.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            status,
            theme::text(11.5),
            Color32::from_rgb(230, 200, 130),
        );

        let pages = egui::Rect::from_center_size(rect.center(), egui::vec2(170.0, rect.height()));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(pages)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let deliver_open = self.ws.deliver.is_some();
                for (icon, label, is_deliver) in [
                    (Icon::EditPage, "Edit", false),
                    (Icon::DeliverPage, "Deliver", true),
                ] {
                    let on = deliver_open == is_deliver;
                    let (r, resp) = ui
                        .allocate_exact_size(egui::vec2(80.0, rect.height()), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(r.shrink(2.0), 4.0, theme::BAR_LIGHT);
                    }
                    let color = if on { Color32::WHITE } else { theme::TEXT_DIM };
                    icons::paint(
                        ui.painter(),
                        egui::Rect::from_center_size(
                            r.center() - egui::vec2(0.0, 7.0),
                            egui::vec2(18.0, 18.0),
                        ),
                        icon,
                        if on { theme::ACCENT } else { color },
                    );
                    ui.painter().text(
                        r.center() + egui::vec2(0.0, 12.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        theme::text(10.5),
                        color,
                    );
                    if resp.clicked() && !on {
                        if is_deliver {
                            self.ws.open_deliver();
                        } else if !self.ws.deliver.as_ref().is_some_and(|d| d.job.is_some()) {
                            // Exportação em andamento continua visível até terminar ou ser cancelada.
                            self.ws.deliver = None;
                        }
                    }
                }
            },
        );
    }

    /// Aviso por cima de tudo enquanto arquivos do sistema são arrastados sobre a janela.
    fn file_drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| i.raw.hovered_files.len());
        if hovering == 0 {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("file-drop"),
        ));
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha(150));
        let card = egui::Rect::from_center_size(screen.center(), egui::vec2(420.0, 120.0));
        painter.rect_filled(card, 8.0, theme::BAR);
        painter.rect_stroke(
            card,
            8.0,
            egui::Stroke::new(2.0, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
        icons::paint(
            &painter,
            egui::Rect::from_center_size(
                card.center() - egui::vec2(0.0, 22.0),
                egui::vec2(30.0, 30.0),
            ),
            Icon::Import,
            theme::ACCENT,
        );
        let text = if hovering == 1 {
            "Solte para importar no Media Pool".to_string()
        } else {
            format!("Solte para importar {hovering} arquivos")
        };
        painter.text(
            card.center() + egui::vec2(0.0, 22.0),
            egui::Align2::CENTER_CENTER,
            text,
            theme::text(15.0),
            Color32::WHITE,
        );
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
        self.ws.source_panel_visible = self.layout.is_visible(Panel::SourceViewer);
        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("interface")
            .exact_size(34.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.interface_bar(ui));
        egui::Panel::bottom("pages")
            .exact_size(44.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.page_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let style = theme::dock_style(ui.style().as_ref());
            DockArea::new(&mut self.layout.dock)
                .style(style)
                .show_leaf_collapse_buttons(false)
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_inside(ui, &mut self.ws);
        });
        self.ws.dialogs_ui(&ctx);

        self.file_drop_overlay(&ctx);
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
