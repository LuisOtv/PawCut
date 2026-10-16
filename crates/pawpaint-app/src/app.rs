//! A janela do PawPaint: menus, barra de opções do pincel, dockers (egui_dock) e diálogos.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use egui::{Align2, Key, Modifiers, RichText, vec2};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use paw_ui::theme;
use paw_ui::toasts::Toasts;
use pawpaint_core::brush::Brush;
use pawpaint_core::filters::Filter;
use pawpaint_core::text::TextRaster;
use pawpaint_core::{Background, file};
use pawpaint_core::{BlendMode, Editor, MoveTo};
use serde::{Deserialize, Serialize};

use crate::canvas;
use crate::docks;
use crate::state::*;

const YOUTUBE_MAX: usize = 2_000_000;

/// Algo a fazer no estado depois que um diálogo fecha.
type StateAction = Box<dyn FnOnce(&mut State)>;

// ---------------------------------------------------------------- preferências

fn config_dir() -> PathBuf {
    // Os testes não mexem nas configurações de verdade.
    if cfg!(test) {
        return std::env::temp_dir().join("pawpaint-test-config");
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawpaint")
}

#[derive(Serialize, Deserialize, Default)]
struct PrefsFile {
    recent: Vec<[u8; 3]>,
    brushes: Vec<Brush>,
    #[serde(default)]
    files: Vec<PathBuf>,
}

pub fn save_prefs(st: &State) {
    let user: Vec<Brush> = st.presets[st.presets.len() - st.user_presets..].to_vec();
    let file = PrefsFile {
        recent: st.prefs.recent.clone(),
        brushes: user,
        files: st.prefs.files.clone(),
    };
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(t) = serde_json::to_string(&file) {
        let _ = std::fs::write(config_dir().join("prefs.json"), t);
    }
}

pub fn save_brush_preset(st: &mut State, ctx: &egui::Context) {
    let name = st.brush_name_edit.trim().to_string();
    if name.is_empty() {
        st.warn("Dê um nome ao pincel");
        return;
    }
    let mut b = st.brush.clone();
    b.name = name.clone();
    st.preset_thumbs.push(docks::brush_thumb(ctx, &b, &name));
    st.presets.push(b.clone());
    st.user_presets += 1;
    st.brush = b;
    st.brush_name_edit.clear();
    save_prefs(st);
    st.say("Pincel salvo");
}

// ---------------------------------------------------------------- recuperação automática

fn autosave_dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join("pawpaint-test-autosave");
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pawpaint")
        .join("autosave")
}

fn autosave_file(uid: u64) -> PathBuf {
    autosave_dir().join(format!("{}-{uid}.pawpaint", std::process::id()))
}

/// Guarda uma cópia dos documentos com mudanças, para recuperar se o app fechar de repente.
fn autosave(st: &mut State) {
    st.last_autosave = std::time::Instant::now();
    let _ = std::fs::create_dir_all(autosave_dir());
    for d in &st.docs {
        let f = autosave_file(d.uid);
        if d.ed.is_dirty() && d.float.is_none() {
            let _ = file::save_project(&d.ed, &f);
        } else if !d.ed.is_dirty() {
            let _ = std::fs::remove_file(&f);
        }
    }
}

/// Cópias de um app que não está mais rodando (as de instâncias vivas não são mexidas).
fn leftovers() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(autosave_dir()) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "pawpaint"))
        .filter(|p| {
            let pid = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse::<u32>().ok());
            pid.is_none_or(|pid| !Path::new(&format!("/proc/{pid}")).exists())
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- layout dos dockers

pub fn default_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Canvas]);
    let tree = dock.main_surface_mut();
    let [canvas, right] =
        tree.split_right(NodeIndex::root(), 0.79, vec![Panel::Color, Panel::Palette]);
    let [_, layers] = tree.split_below(right, 0.42, vec![Panel::Layers]);
    tree.split_below(
        layers,
        0.45,
        vec![
            Panel::BrushPresets,
            Panel::BrushEditor,
            Panel::ToolOptions,
            Panel::History,
            Panel::Overview,
        ],
    );
    tree.split_left(canvas, 0.07, vec![Panel::Toolbox]);
    dock
}

struct Viewer<'a> {
    st: &'a mut State,
}

impl TabViewer for Viewer<'_> {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(("pawpaint-tab", *tab))
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn is_closeable(&self, tab: &Panel) -> bool {
        *tab != Panel::Canvas
    }

    fn allowed_in_windows(&self, _tab: &mut Panel) -> bool {
        true
    }

    fn scroll_bars(&self, _tab: &Panel) -> [bool; 2] {
        [false, false]
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        let st = &mut *self.st;
        match tab {
            Panel::Canvas => canvas::canvas_panel(st, ui),
            Panel::Toolbox => docks::toolbox(st, ui),
            Panel::Layers => docks::layers(st, ui),
            Panel::BrushPresets => docks::brush_presets(st, ui),
            Panel::BrushEditor => docks::brush_editor(st, ui),
            Panel::ToolOptions => docks::tool_options(st, ui),
            Panel::Color => docks::color(st, ui),
            Panel::Palette => docks::palette(st, ui),
            Panel::Overview => docks::overview(st, ui),
            Panel::History => docks::history(st, ui),
        }
    }
}

// ---------------------------------------------------------------- app

pub struct PaintApp {
    dock: DockState<Panel>,
    st: State,
    last_title: String,
    last_layout: String,
    screenshot: Option<PathBuf>,
    frames: u64,
}

impl PaintApp {
    pub fn new(ctx: &egui::Context, args: Vec<PathBuf>) -> Self {
        theme::apply(ctx);
        let mut app = Self::empty();
        app.screenshot = std::env::var_os("PAWPAINT_SCREENSHOT").map(PathBuf::from);
        if let Ok(t) = std::fs::read_to_string(config_dir().join("layout.json"))
            && let Ok(d) = serde_json::from_str::<DockState<Panel>>(&t)
            && d.find_tab(&Panel::Canvas).is_some()
        {
            app.dock = d;
        }
        if let Ok(t) = std::fs::read_to_string(config_dir().join("prefs.json"))
            && let Ok(p) = serde_json::from_str::<PrefsFile>(&t)
        {
            app.st.prefs.recent = p.recent;
            app.st.prefs.files = p.files;
            app.st.user_presets = p.brushes.len();
            app.st.presets.extend(p.brushes);
        }
        app.st.preset_thumbs = app
            .st
            .presets
            .iter()
            .map(|b| docks::brush_thumb(ctx, b, &b.name))
            .collect();
        for p in args {
            open_path(&mut app.st, &p);
        }
        let left = leftovers();
        if !left.is_empty() {
            app.st.dialog = Some(Dialog::Recover(left));
        }
        app
    }

    fn empty() -> Self {
        let presets = Brush::presets();
        let brush = presets[0].clone();
        Self {
            dock: default_layout(),
            st: State {
                docs: Vec::new(),
                cur: 0,
                next_uid: 1,
                tool: Tool::Brush,
                brush,
                presets,
                user_presets: 0,
                preset_thumbs: Vec::new(),
                fg: [0, 0, 0],
                bg: [255, 255, 255],
                hsv: (0.0, 0.0, 0.0),
                blend: BlendMode::Normal,
                mirror: (false, false),
                opts: ToolOpts::default(),
                clipboard: None,
                toasts: Toasts::default(),
                dialog: None,
                file: None,
                text_raster: TextRaster::new(),
                fonts: Vec::new(),
                prefs: Prefs {
                    recent: Vec::new(),
                    files: Vec::new(),
                },
                interaction: None,
                text_input: None,
                renaming: None,
                brush_name_edit: String::new(),
                dock_commands: Vec::new(),
                allow_close: false,
                tool_changed: false,
                last_autosave: std::time::Instant::now(),
            },
            last_title: String::new(),
            last_layout: String::new(),
            screenshot: None,
            frames: 0,
        }
    }
}

// ---------------------------------------------------------------- operações de estado

pub fn set_tool(st: &mut State, tool: Tool) {
    if st.tool == tool {
        return;
    }
    if let Some(d) = st.doc_mut() {
        if d.float.is_some() {
            canvas::commit_float(d);
        }
        d.crop = None;
    }
    st.text_input = None;
    st.interaction = None;
    st.tool = tool;
    st.tool_changed = true;
}

pub fn swap_colors(st: &mut State) {
    std::mem::swap(&mut st.fg, &mut st.bg);
    st.hsv = rgb_to_hsv(st.fg);
}

pub fn request_close_doc(st: &mut State, i: usize) {
    if st.docs.get(i).is_some_and(|d| d.ed.is_dirty()) {
        st.dialog = Some(Dialog::ConfirmClose(i));
    } else {
        close_doc(st, i);
    }
}

fn close_doc(st: &mut State, i: usize) {
    if i < st.docs.len() {
        let removed = st.docs.remove(i);
        let _ = std::fs::remove_file(autosave_file(removed.uid));
        st.cur = st.cur.min(st.docs.len().saturating_sub(1));
        st.interaction = None;
    }
}

fn remember_file(st: &mut State, p: &Path) {
    st.prefs.files.retain(|f| f != p);
    st.prefs.files.insert(0, p.to_path_buf());
    st.prefs.files.truncate(10);
    save_prefs(st);
}

fn add_doc(st: &mut State, ed: Editor, path: Option<PathBuf>) {
    let uid = st.next_uid;
    st.next_uid += 1;
    st.docs.push(DocView::new(uid, ed, path));
    st.cur = st.docs.len() - 1;
    st.interaction = None;
}

fn new_from_clipboard(st: &mut State) {
    match system_clipboard_image() {
        Some(img) if img.width() <= 8192 && img.height() <= 8192 => {
            add_doc(st, Editor::from_image(&img, "Colada"), None);
            if let Some(d) = st.doc_mut() {
                d.name = "Da área de transferência".into();
            }
        }
        Some(_) => st.warn("Imagem grande demais (limite 8192 px)"),
        None => st.warn("Não há imagem na área de transferência"),
    }
}

fn open_path(st: &mut State, p: &Path) {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    if ext.as_deref() == Some("pawpaint") {
        match file::load_project(p) {
            Ok(ed) => {
                add_doc(st, ed, Some(p.to_path_buf()));
                remember_file(st, p);
            }
            Err(e) => st.warn(e.to_string()),
        }
        return;
    }
    match file::open_image(p) {
        Ok(img) => {
            let name = p
                .file_stem()
                .map_or("Imagem".into(), |s| s.to_string_lossy().into_owned());
            if img.width() > 8192 || img.height() > 8192 {
                st.warn("Imagem grande demais (o limite é 8192 px de lado)");
                return;
            }
            add_doc(st, Editor::from_image(&img, &name), None);
            if let Some(d) = st.doc_mut() {
                d.name = name;
            }
        }
        Err(e) => st.warn(format!("Não deu para abrir {}: {e}", p.display())),
    }
}

fn ask(st: &mut State, purpose: Purpose) {
    if st.file.is_some() {
        return;
    }
    let name = st
        .doc()
        .and_then(|d| {
            d.path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "imagem".to_string());
    let (tx, rx): (_, Receiver<Vec<PathBuf>>) = channel();
    std::thread::spawn(move || {
        let d = rfd::FileDialog::new();
        let picked: Vec<PathBuf> = match purpose {
            Purpose::Open => d
                .set_title("Abrir")
                .add_filter(
                    "Imagens e projetos",
                    &["pawpaint", "png", "jpg", "jpeg", "webp", "bmp"],
                )
                .pick_files()
                .unwrap_or_default(),
            Purpose::SaveAs => d
                .set_title("Salvar projeto")
                .add_filter("PawPaint", &["pawpaint"])
                .set_file_name(format!("{name}.pawpaint"))
                .save_file()
                .into_iter()
                .collect(),
            Purpose::ImportLayer => d
                .set_title("Importar como camada")
                .add_filter("Imagens", &["png", "jpg", "jpeg", "webp", "bmp"])
                .pick_files()
                .unwrap_or_default(),
            Purpose::Export => d
                .set_title("Exportar imagem")
                .add_filter("PNG", &["png"])
                .add_filter("JPG", &["jpg"])
                .add_filter("WebP", &["webp"])
                .add_filter("BMP", &["bmp"])
                .set_file_name(format!("{name}.png"))
                .save_file()
                .into_iter()
                .collect(),
            Purpose::ExportJpg => d
                .set_title("Exportar JPG com até 2 MB")
                .add_filter("JPG", &["jpg"])
                .set_file_name(format!("{name}.jpg"))
                .save_file()
                .into_iter()
                .collect(),
        };
        let _ = tx.send(picked);
    });
    st.file = Some((purpose, rx));
}

fn poll_file(st: &mut State) {
    let Some(paths) = st.file.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) else {
        return;
    };
    let (purpose, _) = st.file.take().expect("janela aberta");
    if paths.is_empty() {
        return;
    }
    match purpose {
        Purpose::Open => {
            for p in paths {
                open_path(st, &p);
            }
        }
        Purpose::SaveAs => {
            let mut p = paths[0].clone();
            if p.extension().is_none_or(|e| e != "pawpaint") {
                p.set_extension("pawpaint");
            }
            save_to(st, p);
        }
        Purpose::ImportLayer => {
            for p in paths {
                match file::open_image(&p) {
                    Ok(img) => {
                        let name = p
                            .file_stem()
                            .map_or("Imagem".into(), |s| s.to_string_lossy().into_owned());
                        if let Some(d) = st.doc_mut() {
                            d.active = d.ed.import_layer(&img, &name);
                        }
                    }
                    Err(e) => st.warn(e.to_string()),
                }
            }
        }
        Purpose::Export => {
            let Some(d) = st.doc() else { return };
            match file::export_image(d.ed.doc(), &paths[0], 92) {
                Ok(()) => st.say("Imagem exportada"),
                Err(e) => st.warn(e.to_string()),
            }
        }
        Purpose::ExportJpg => {
            let Some(d) = st.doc() else { return };
            let img = pawpaint_core::composite::flatten(d.ed.doc());
            let (bytes, q) = file::jpeg_under(&img, YOUTUBE_MAX);
            let mut p = paths[0].clone();
            if p.extension().is_none() {
                p.set_extension("jpg");
            }
            match std::fs::write(&p, &bytes) {
                Ok(()) => st.say(format!(
                    "JPG exportado: {} KB, qualidade {q} (limite 2 MB)",
                    bytes.len() / 1000
                )),
                Err(e) => st.warn(e.to_string()),
            }
        }
    }
}

fn save_to(st: &mut State, path: PathBuf) {
    let cur = st.cur;
    if let Some(d) = st.docs.get_mut(cur) {
        if d.float.is_some() {
            canvas::commit_float(d);
        }
        match file::save_project(&d.ed, &path) {
            Ok(()) => {
                d.ed.mark_saved();
                d.path = Some(path.clone());
                remember_file(st, &path);
                st.say("Projeto salvo");
            }
            Err(e) => st.warn(e.to_string()),
        }
    }
}

fn save(st: &mut State) {
    match st.doc().map(|d| d.path.clone()) {
        Some(Some(p)) => save_to(st, p),
        Some(None) => ask(st, Purpose::SaveAs),
        None => {}
    }
}

fn open_filter(st: &mut State, kind: FilterKind) {
    let (a, b, c) = match kind {
        FilterKind::BrightnessContrast => (0.0, 0.0, 0.0),
        FilterKind::HueSaturation => (0.0, 0.0, 0.0),
        FilterKind::Levels => (0.0, 1.0, 1.0),
        FilterKind::Threshold => (0.5, 0.0, 0.0),
        FilterKind::Posterize => (4.0, 0.0, 0.0),
        FilterKind::Blur => (4.0, 0.0, 0.0),
        FilterKind::Sharpen => (1.0, 1.5, 0.0),
        FilterKind::Pixelate => (8.0, 0.0, 0.0),
        FilterKind::Noise => (0.2, 0.0, 0.0),
    };
    st.dialog = Some(Dialog::Filter {
        kind,
        a,
        b,
        c,
        preview: true,
        applied: false,
    });
}

fn filter_of(kind: &FilterKind, a: f32, b: f32, c: f32) -> (Filter, &'static str) {
    match kind {
        FilterKind::BrightnessContrast => (
            Filter::BrightnessContrast {
                brightness: a,
                contrast: b,
            },
            "Brilho e contraste",
        ),
        FilterKind::HueSaturation => (
            Filter::HueSaturation {
                hue: a,
                saturation: b,
                lightness: c,
            },
            "Matiz e saturação",
        ),
        FilterKind::Levels => (
            Filter::Levels {
                black: a,
                white: b,
                gamma: c,
            },
            "Níveis",
        ),
        FilterKind::Threshold => (Filter::Threshold(a), "Limiar"),
        FilterKind::Posterize => (Filter::Posterize(a as u32), "Posterizar"),
        FilterKind::Blur => (Filter::GaussianBlur(a), "Desfoque"),
        FilterKind::Sharpen => (
            Filter::Sharpen {
                amount: a,
                radius: b,
            },
            "Nitidez",
        ),
        FilterKind::Pixelate => (Filter::Pixelate(a as u32), "Pixelar"),
        FilterKind::Noise => (Filter::Noise(a), "Ruído"),
    }
}

fn quick_filter(st: &mut State, f: Filter, label: &'static str) {
    if let Some(d) = st.doc_mut() {
        let id = d.active;
        if let Err(e) = d.ed.apply_filter(id, &f, label) {
            st.warn(e.to_string());
        }
    }
}

// ---------------------------------------------------------------- menus

impl PaintApp {
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, keys: &str, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::Button::new(label).shortcut_text(keys.to_string()),
            )
            .clicked()
        };
        let st = &mut self.st;
        let has_doc = !st.docs.is_empty();
        let has_sel = st.doc().is_some_and(|d| d.ed.doc().selection.is_some());
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Arquivo", |ui| {
                if item(ui, "Novo…", "Ctrl+N", true) {
                    ui.close();
                    st.dialog = Some(Dialog::New {
                        w: 1920,
                        h: 1080,
                        bg: 0,
                        color: [255, 255, 255],
                    });
                }
                if item(
                    ui,
                    "Nova a partir da área de transferência",
                    "Ctrl+Shift+V",
                    true,
                ) {
                    ui.close();
                    new_from_clipboard(st);
                }
                if item(ui, "Abrir…", "Ctrl+O", true) {
                    ui.close();
                    ask(st, Purpose::Open);
                }
                ui.add_enabled_ui(!st.prefs.files.is_empty(), |ui| {
                    ui.menu_button("Abrir recente", |ui| {
                        for f in st.prefs.files.clone() {
                            let name = f
                                .file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                            if ui
                                .button(name)
                                .on_hover_text(f.display().to_string())
                                .clicked()
                            {
                                ui.close();
                                open_path(st, &f);
                            }
                        }
                    });
                });
                if item(ui, "Importar como camada…", "", has_doc) {
                    ui.close();
                    ask(st, Purpose::ImportLayer);
                }
                ui.separator();
                if item(ui, "Salvar", "Ctrl+S", has_doc) {
                    ui.close();
                    save(st);
                }
                if item(ui, "Salvar como…", "Ctrl+Shift+S", has_doc) {
                    ui.close();
                    ask(st, Purpose::SaveAs);
                }
                ui.separator();
                if item(
                    ui,
                    "Exportar imagem… (PNG, JPG, WebP)",
                    "Ctrl+Shift+E",
                    has_doc,
                ) {
                    ui.close();
                    ask(st, Purpose::Export);
                }
                if item(ui, "Exportar JPG com até 2 MB…", "", has_doc) {
                    ui.close();
                    ask(st, Purpose::ExportJpg);
                }
                ui.separator();
                if item(ui, "Fechar documento", "Ctrl+W", has_doc) {
                    ui.close();
                    request_close_doc(st, st.cur);
                }
                if item(ui, "Sair", "", true) {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Editar", |ui| {
                let (undo, redo) = st
                    .doc()
                    .map_or((None, None), |d| (d.ed.undo_label(), d.ed.redo_label()));
                if item(
                    ui,
                    &undo.map_or("Desfazer".to_string(), |l| format!("Desfazer {l}")),
                    "Ctrl+Z",
                    undo.is_some(),
                ) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.undo();
                    }
                }
                if item(
                    ui,
                    &redo.map_or("Refazer".to_string(), |l| format!("Refazer {l}")),
                    "Ctrl+Shift+Z",
                    redo.is_some(),
                ) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.redo();
                    }
                }
                ui.separator();
                if item(ui, "Recortar", "Ctrl+X", has_doc) {
                    ui.close();
                    edit_cut(st);
                }
                if item(ui, "Copiar", "Ctrl+C", has_doc) {
                    ui.close();
                    edit_copy(st, false);
                }
                if item(ui, "Copiar tudo (mesclado)", "Ctrl+Shift+C", has_doc) {
                    ui.close();
                    edit_copy(st, true);
                }
                if item(
                    ui,
                    "Colar como camada nova",
                    "Ctrl+V",
                    st.clipboard.is_some() && has_doc,
                ) {
                    ui.close();
                    edit_paste(st);
                }
                ui.separator();
                if item(ui, "Preencher com a cor da frente", "Backspace", has_doc) {
                    ui.close();
                    edit_fill(st, false);
                }
                if item(
                    ui,
                    "Preencher com a cor de fundo",
                    "Shift+Backspace",
                    has_doc,
                ) {
                    ui.close();
                    edit_fill(st, true);
                }
                if item(ui, "Limpar", "Delete", has_doc) {
                    ui.close();
                    edit_clear(st);
                }
            });
            ui.menu_button("Selecionar", |ui| {
                if item(ui, "Tudo", "Ctrl+A", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.select_all();
                    }
                }
                if item(ui, "Nenhum", "Ctrl+Shift+A", has_sel) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.deselect();
                    }
                }
                if item(ui, "Inverter", "Ctrl+Shift+I", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.ed.invert_selection();
                    }
                }
                ui.separator();
                if item(ui, "Suavizar borda…", "", has_sel) {
                    ui.close();
                    st.dialog = Some(Dialog::Feather(4));
                }
                if item(ui, "Expandir ou contrair…", "", has_sel) {
                    ui.close();
                    st.dialog = Some(Dialog::Grow(3));
                }
            });
            ui.menu_button("Imagem", |ui| {
                if item(ui, "Tamanho da tela…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        st.dialog = Some(Dialog::CanvasSize {
                            w: d.ed.doc().width,
                            h: d.ed.doc().height,
                            anchor: 4,
                        });
                    }
                }
                if item(ui, "Escalar imagem…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        let (w, h) = (d.ed.doc().width, d.ed.doc().height);
                        st.dialog = Some(Dialog::ScaleImage {
                            w,
                            h,
                            keep: true,
                            ratio: w as f32 / h as f32,
                        });
                    }
                }
                if item(ui, "Cortar ao conteúdo", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        commit_if_floating(d);
                        if d.ed.crop_to_content() {
                            d.fit = true;
                        }
                    }
                }
                if item(ui, "Cortar para a seleção", "", has_sel) {
                    ui.close();
                    crop_to_selection(st);
                }
                ui.separator();
                for (label, cw, horizontal, is_flip) in [
                    ("Girar 90° horário", true, false, false),
                    ("Girar 90° anti-horário", false, false, false),
                    ("Espelhar na horizontal", false, true, true),
                    ("Espelhar na vertical", false, false, true),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        if let Some(d) = st.doc_mut() {
                            commit_if_floating(d);
                            if is_flip {
                                d.ed.flip_image(horizontal);
                            } else {
                                d.ed.rotate_image(cw);
                            }
                            d.fit = true;
                        }
                    }
                }
                ui.separator();
                if item(ui, "Achatar imagem", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        commit_if_floating(d);
                        d.ed.flatten();
                        d.active = d.ed.doc().layers.last().map_or(0, |l| l.id);
                    }
                }
            });
            ui.menu_button("Camada", |ui| {
                if item(ui, "Nova camada", "Ctrl+Shift+N", has_doc) {
                    ui.close();
                    new_layer(st);
                }
                if item(ui, "Duplicar", "Ctrl+J", has_doc) {
                    ui.close();
                    dup_layer(st);
                }
                if item(ui, "Unir com a de baixo", "Ctrl+E", has_doc) {
                    ui.close();
                    merge_down(st);
                }
                if item(ui, "Espelhar camada", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        let id = d.active;
                        let _ = d.ed.flip_layer(id, true);
                    }
                }
                if item(ui, "Propriedades…", "", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc() {
                        st.dialog = Some(Dialog::LayerProps(d.active));
                    }
                }
                ui.separator();
                for (label, to) in [
                    ("Trazer para frente", MoveTo::Up),
                    ("Enviar para trás", MoveTo::Down),
                    ("Para o topo", MoveTo::Top),
                    ("Para o fundo", MoveTo::Bottom),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        if let Some(d) = st.doc_mut() {
                            let id = d.active;
                            d.ed.move_layer(id, to);
                        }
                    }
                }
            });
            ui.menu_button("Filtro", |ui| {
                ui.label(RichText::new("Ajustar").small().weak());
                for (label, kind) in [
                    ("Brilho e contraste…", FilterKind::BrightnessContrast),
                    (
                        "Matiz, saturação e luminosidade…",
                        FilterKind::HueSaturation,
                    ),
                    ("Níveis…", FilterKind::Levels),
                    ("Limiar…", FilterKind::Threshold),
                    ("Posterizar…", FilterKind::Posterize),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        open_filter(st, kind);
                    }
                }
                if item(ui, "Curvas…", "", has_doc) {
                    ui.close();
                    st.dialog = Some(Dialog::Curves {
                        points: vec![(0.0, 0.0), (1.0, 1.0)],
                        drag: None,
                        applied: false,
                    });
                }
                if item(ui, "Inverter cores", "Ctrl+I", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::Invert, "Inverter cores");
                }
                if item(ui, "Dessaturar", "Ctrl+Shift+U", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::Desaturate, "Dessaturar");
                }
                ui.separator();
                ui.label(RichText::new("Efeitos").small().weak());
                for (label, kind) in [
                    ("Desfoque gaussiano…", FilterKind::Blur),
                    ("Nitidez…", FilterKind::Sharpen),
                    ("Pixelar…", FilterKind::Pixelate),
                    ("Ruído…", FilterKind::Noise),
                ] {
                    if item(ui, label, "", has_doc) {
                        ui.close();
                        open_filter(st, kind);
                    }
                }
                if item(ui, "Detectar bordas", "", has_doc) {
                    ui.close();
                    quick_filter(st, Filter::EdgeDetect, "Detectar bordas");
                }
            });
            ui.menu_button("Ver", |ui| {
                if item(ui, "Ajustar à janela", "Ctrl+0", has_doc) {
                    ui.close();
                    if let Some(d) = st.doc_mut() {
                        d.fit = true;
                        d.pan = egui::Vec2::ZERO;
                    }
                }
                if item(ui, "Tamanho real (100%)", "Ctrl+1", has_doc) {
                    ui.close();
                    zoom_to(st, 1.0);
                }
                if item(ui, "Ampliar", "+", has_doc) {
                    ui.close();
                    zoom_by(st, 1.25);
                }
                if item(ui, "Reduzir", "-", has_doc) {
                    ui.close();
                    zoom_by(st, 0.8);
                }
            });
            ui.menu_button("Configurações", |ui| {
                ui.menu_button("Dockers", |ui| {
                    for p in Panel::ALL.into_iter().filter(|p| *p != Panel::Canvas) {
                        let on = self.dock.find_tab(&p).is_some();
                        let mut v = on;
                        if ui.checkbox(&mut v, p.title()).changed() {
                            st.dock_commands.push(DockCommand::Toggle(p));
                        }
                    }
                });
                if ui.button("Redefinir layout dos dockers").clicked() {
                    ui.close();
                    st.dock_commands.push(DockCommand::Reset);
                }
            });
        });
    }

    /// Barra de cima com as opções do pincel (como a do Krita).
    fn brush_bar(&mut self, ui: &mut egui::Ui) {
        let st = &mut self.st;
        ui.horizontal_centered(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(st.tool.label()).strong());
            ui.separator();
            if st.tool.paints() || st.tool == Tool::Text {
                let name = st.brush.name.clone();
                egui::ComboBox::from_id_salt("bar-preset")
                    .selected_text(name)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        let mut pick = None;
                        for (i, b) in st.presets.iter().enumerate() {
                            if ui
                                .selectable_label(b.name == st.brush.name, &b.name)
                                .clicked()
                            {
                                pick = Some(i);
                            }
                        }
                        if let Some(i) = pick {
                            st.brush = st.presets[i].clone();
                        }
                    });
                let mut mode = st.blend;
                egui::ComboBox::from_id_salt("bar-blend")
                    .selected_text(mode.label())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for m in BlendMode::ALL {
                            ui.selectable_value(&mut mode, m, m.label());
                        }
                    });
                st.blend = mode;
                ui.separator();
                let mut o = st.brush.opacity * 100.0;
                ui.label("Opacidade");
                ui.add(
                    egui::Slider::new(&mut o, 1.0..=100.0)
                        .suffix("%")
                        .trailing_fill(true),
                );
                st.brush.opacity = o / 100.0;
                ui.label("Tamanho");
                ui.add(
                    egui::Slider::new(&mut st.brush.size, 1.0..=500.0)
                        .suffix(" px")
                        .logarithmic(true)
                        .trailing_fill(true),
                );
                let mut f = st.brush.flow * 100.0;
                ui.label("Fluxo");
                ui.add(
                    egui::Slider::new(&mut f, 1.0..=100.0)
                        .suffix("%")
                        .trailing_fill(true),
                );
                st.brush.flow = f / 100.0;
                ui.separator();
                ui.separator();
                ui.toggle_value(&mut st.mirror.0, "Espelho ↔")
                    .on_hover_text("Pinta também refletido na vertical (eixo no meio)");
                ui.toggle_value(&mut st.mirror.1, "Espelho ↕")
                    .on_hover_text("Pinta também refletido na horizontal (eixo no meio)");
                let mut erasing = st.tool == Tool::Eraser;
                if ui
                    .toggle_value(&mut erasing, "Borracha")
                    .on_hover_text("Alterna borracha (E)")
                    .changed()
                {
                    set_tool(st, if erasing { Tool::Eraser } else { Tool::Brush });
                }
            } else {
                ui.label(
                    RichText::new("Opções no docker \"Opções da ferramenta\"")
                        .weak()
                        .small(),
                );
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let st = &self.st;
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            if let Some(d) = st.doc() {
                let doc = d.ed.doc();
                ui.label(
                    RichText::new(format!(
                        "{} × {} px · {:.0}%",
                        doc.width,
                        doc.height,
                        d.zoom * 100.0
                    ))
                    .small(),
                );
                if let Some((x, y)) = d.cursor
                    && x >= 0.0
                    && y >= 0.0
                    && (x as usize) < doc.width
                    && (y as usize) < doc.height
                {
                    ui.label(
                        RichText::new(format!("{:.0}, {:.0}", x, y))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
                if let Some(sel) = &doc.selection
                    && let Some((x0, y0, x1, y1)) = sel.bounds()
                {
                    ui.label(
                        RichText::new(format!("seleção {} × {}", x1 - x0, y1 - y0))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
            } else {
                ui.label(
                    RichText::new("Sem documento")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
        });
    }
}

fn commit_if_floating(d: &mut DocView) {
    if d.float.is_some() {
        canvas::commit_float(d);
    }
}

fn zoom_to(st: &mut State, z: f32) {
    if let Some(d) = st.doc_mut() {
        d.fit = false;
        d.zoom = z;
        d.pan = egui::Vec2::ZERO;
    }
}

fn zoom_by(st: &mut State, k: f32) {
    if let Some(d) = st.doc_mut() {
        d.fit = false;
        d.zoom = (d.zoom * k).clamp(0.02, 32.0);
    }
}

fn new_layer(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        let a = d.active;
        d.active = d.ed.add_layer(None, Some(a));
    }
}

fn dup_layer(st: &mut State) {
    if let Some(d) = st.doc_mut() {
        let a = d.active;
        if let Ok(n) = d.ed.duplicate_layer(a) {
            d.active = n;
        }
    }
}

fn merge_down(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let a = d.active;
    match d.ed.merge_down(a) {
        Ok(n) => d.active = n,
        Err(e) => st.warn(e.to_string()),
    }
}

fn edit_copy(st: &mut State, merged: bool) {
    let Some(d) = st.doc() else { return };
    st.clipboard = d.ed.copy(d.active, merged);
    if let Some(c) = &st.clipboard {
        set_system_clipboard(c);
        st.say("Copiado");
    }
}

/// Também põe a imagem na área de transferência do sistema (para colar em outros programas).
fn set_system_clipboard(c: &pawpaint_core::Clip) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_image(arboard::ImageData {
            width: c.image.width() as usize,
            height: c.image.height() as usize,
            bytes: std::borrow::Cow::Borrowed(c.image.as_raw()),
        });
    }
}

/// Imagem da área de transferência do sistema (um print, por exemplo), se houver.
fn system_clipboard_image() -> Option<image::RgbaImage> {
    let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
    image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
}

fn edit_cut(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    match d.ed.cut(id) {
        Ok(c) => st.clipboard = c.or(st.clipboard.take()),
        Err(e) => st.warn(e.to_string()),
    }
}

fn edit_paste(st: &mut State) {
    let system = system_clipboard_image();
    // Se a imagem do sistema é a que copiamos daqui, cola no lugar de origem.
    let same = match (&system, &st.clipboard) {
        (Some(s), Some(c)) => {
            s.dimensions() == c.image.dimensions() && s.as_raw() == c.image.as_raw()
        }
        _ => false,
    };
    let Some(d) = st.doc_mut() else { return };
    if let (Some(img), false) = (system, same) {
        d.active = d.ed.import_layer(&img, "Colada");
        return;
    }
    if let Some(c) = st.clipboard.clone()
        && let Some(d) = st.doc_mut()
    {
        d.active = d.ed.paste(&c, None);
    }
}

fn edit_fill(st: &mut State, bg: bool) {
    let c = if bg { st.bg } else { st.fg };
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    if let Err(e) = d.ed.fill_selection(id, c) {
        st.warn(e.to_string());
    }
}

fn edit_clear(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    let id = d.active;
    if let Err(e) = d.ed.clear_layer(id) {
        st.warn(e.to_string());
    }
}

fn crop_to_selection(st: &mut State) {
    let Some(d) = st.doc_mut() else { return };
    if let Some((x0, y0, x1, y1)) = d.ed.doc().selection.as_ref().and_then(|s| s.bounds()) {
        commit_if_floating(d);
        d.ed.crop(x0, y0, x1, y1);
        d.fit = true;
    }
}

// ---------------------------------------------------------------- atalhos

impl PaintApp {
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let st = &mut self.st;
        let ctrl = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let hit = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if ctx.egui_wants_keyboard_input() || st.dialog.is_some() || st.text_input.is_some() {
            return;
        }
        if hit(ctrl, Key::N) {
            st.dialog = Some(Dialog::New {
                w: 1920,
                h: 1080,
                bg: 0,
                color: [255, 255, 255],
            });
        }
        if hit(ctrl, Key::O) {
            ask(st, Purpose::Open);
        }
        if hit(cs, Key::V) {
            new_from_clipboard(st);
        }
        if hit(cs, Key::S) {
            ask(st, Purpose::SaveAs);
        } else if hit(ctrl, Key::S) {
            save(st);
        }
        if hit(cs, Key::E) {
            ask(st, Purpose::Export);
        }
        if hit(ctrl, Key::W) {
            request_close_doc(st, st.cur);
        }
        if st.docs.is_empty() {
            return;
        }
        if hit(cs, Key::Z) || hit(ctrl, Key::Y) {
            if let Some(d) = st.doc_mut() {
                commit_if_floating(d);
                d.ed.redo();
            }
        } else if hit(ctrl, Key::Z)
            && let Some(d) = st.doc_mut()
        {
            if d.float.is_some() {
                canvas::abort_float(d);
            } else {
                d.ed.undo();
            }
        }
        if hit(cs, Key::C) {
            edit_copy(st, true);
        } else if hit(ctrl, Key::C) {
            edit_copy(st, false);
        }
        if hit(ctrl, Key::X) {
            edit_cut(st);
        }
        if hit(ctrl, Key::V) {
            edit_paste(st);
        }
        if hit(cs, Key::A) {
            if let Some(d) = st.doc_mut() {
                d.ed.deselect();
            }
        } else if hit(ctrl, Key::A)
            && let Some(d) = st.doc_mut()
        {
            d.ed.select_all();
        }
        if hit(cs, Key::I) {
            if let Some(d) = st.doc_mut() {
                d.ed.invert_selection();
            }
        } else if hit(ctrl, Key::I) {
            quick_filter(st, Filter::Invert, "Inverter cores");
        }
        if hit(ctrl, Key::J) {
            dup_layer(st);
        }
        if hit(ctrl, Key::E) {
            merge_down(st);
        }
        if hit(cs, Key::N) {
            new_layer(st);
        }
        if hit(ctrl, Key::Num0)
            && let Some(d) = st.doc_mut()
        {
            d.fit = true;
            d.pan = egui::Vec2::ZERO;
        }
        if hit(ctrl, Key::Num1) {
            zoom_to(st, 1.0);
        }
        if hit(Modifiers::NONE, Key::Plus) || hit(Modifiers::NONE, Key::Equals) {
            zoom_by(st, 1.25);
        }
        if hit(Modifiers::NONE, Key::Minus) {
            zoom_by(st, 0.8);
        }
        if hit(ctrl, Key::T) {
            set_tool(st, Tool::Transform);
        }
        if hit(Modifiers::NONE, Key::Enter) {
            canvas::confirm(st, true);
        }
        if hit(Modifiers::NONE, Key::Escape) {
            canvas::confirm(st, false);
            if let Some(d) = st.doc_mut() {
                d.ed.end_change();
            }
        }
        if hit(Modifiers::NONE, Key::Delete) {
            edit_clear(st);
        }
        if hit(Modifiers::NONE, Key::Backspace) {
            edit_fill(st, false);
        }
        if hit(Modifiers::SHIFT, Key::Backspace) {
            edit_fill(st, true);
        }
        if hit(Modifiers::NONE, Key::X) {
            swap_colors(st);
        }
        if hit(Modifiers::NONE, Key::D) {
            st.fg = [0, 0, 0];
            st.bg = [255, 255, 255];
            st.hsv = (0.0, 0.0, 0.0);
        }
        if hit(Modifiers::NONE, Key::OpenBracket) {
            st.brush.size = (st.brush.size * 0.88 - 1.0).max(1.0);
        }
        if hit(Modifiers::NONE, Key::CloseBracket) {
            st.brush.size = (st.brush.size * 1.14 + 1.0).min(500.0);
        }
        for tool in Tool::ALL {
            let key = match tool.key() {
                "B" => Key::B,
                "E" => Key::E,
                "S" => Key::S,
                "F" => Key::F,
                "G" => Key::G,
                "P" => Key::P,
                "J" => Key::J,
                "K" => Key::K,
                "L" => Key::L,
                "W" => Key::W,
                "T" => Key::T,
                "C" => Key::C,
                "I" => Key::I,
                "U" => Key::U,
                "O" => Key::O,
                "Q" => Key::Q,
                _ => continue,
            };
            if hit(Modifiers::NONE, key) {
                set_tool(st, tool);
            }
        }
    }
}

// ---------------------------------------------------------------- diálogos

impl PaintApp {
    fn dialogs(&mut self, ctx: &egui::Context) {
        let st = &mut self.st;
        let Some(mut dialog) = st.dialog.take() else {
            return;
        };
        let mut keep = true;
        let mut then: Option<StateAction> = None;
        let title = match &dialog {
            Dialog::New { .. } => "Nova imagem",
            Dialog::Filter { .. } => "Filtro",
            Dialog::Curves { .. } => "Curvas",
            Dialog::CanvasSize { .. } => "Tamanho da tela",
            Dialog::ScaleImage { .. } => "Escalar imagem",
            Dialog::Feather(_) => "Suavizar borda da seleção",
            Dialog::Grow(_) => "Expandir ou contrair seleção",
            Dialog::LayerProps(_) => "Propriedades da camada",
            Dialog::ConfirmClose(_) | Dialog::ConfirmQuit => "Mudanças não salvas",
            Dialog::Recover(_) => "Recuperação automática",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| match &mut dialog {
                Dialog::New { w, h, bg, color } => {
                    ui.horizontal_wrapped(|ui| {
                        for (label, (pw, ph)) in [
                            ("1920×1080", (1920, 1080)),
                            ("1280×720 (thumbnail)", (1280, 720)),
                            ("1080×1080", (1080, 1080)),
                            ("1080×1920", (1080, 1920)),
                            ("A4 300 dpi", (2480, 3508)),
                        ] {
                            if ui.button(label).clicked() {
                                (*w, *h) = (pw, ph);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Tamanho");
                        ui.add(egui::DragValue::new(w).range(1..=8192).suffix(" px"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(1..=8192).suffix(" px"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Fundo");
                        ui.selectable_value(bg, 0, "Branco");
                        ui.selectable_value(bg, 1, "Transparente");
                        ui.selectable_value(bg, 2, "Cor");
                        if *bg == 2 {
                            ui.color_edit_button_srgb(color);
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Criar").clicked() {
                            keep = false;
                            let (w, h, bg, color) = (*w, *h, *bg, *color);
                            then = Some(Box::new(move |st| {
                                let background = match bg {
                                    0 => Background::White,
                                    1 => Background::Transparent,
                                    _ => Background::Color(color),
                                };
                                add_doc(st, Editor::new(w, h, background), None);
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Filter {
                    kind,
                    a,
                    b,
                    c,
                    preview,
                    applied,
                } => {
                    let mut changed = false;
                    let mut slider =
                        |ui: &mut egui::Ui,
                         name: &str,
                         v: &mut f32,
                         r: std::ops::RangeInclusive<f32>| {
                            ui.horizontal(|ui| {
                                ui.label(name);
                                changed |= ui.add(egui::Slider::new(v, r)).changed();
                            });
                        };
                    match kind {
                        FilterKind::BrightnessContrast => {
                            slider(ui, "Brilho", a, -1.0..=1.0);
                            slider(ui, "Contraste", b, -1.0..=1.0);
                        }
                        FilterKind::HueSaturation => {
                            slider(ui, "Matiz", a, -180.0..=180.0);
                            slider(ui, "Saturação", b, -1.0..=1.0);
                            slider(ui, "Luminosidade", c, -1.0..=1.0);
                        }
                        FilterKind::Levels => {
                            slider(ui, "Pretos", a, 0.0..=1.0);
                            slider(ui, "Brancos", b, 0.0..=1.0);
                            slider(ui, "Gama", c, 0.1..=4.0);
                        }
                        FilterKind::Threshold => slider(ui, "Limiar", a, 0.0..=1.0),
                        FilterKind::Posterize => slider(ui, "Níveis", a, 2.0..=32.0),
                        FilterKind::Blur => slider(ui, "Raio", a, 0.5..=60.0),
                        FilterKind::Sharpen => {
                            slider(ui, "Quantidade", a, 0.0..=3.0);
                            slider(ui, "Raio", b, 0.3..=10.0);
                        }
                        FilterKind::Pixelate => slider(ui, "Tamanho do bloco", a, 2.0..=80.0),
                        FilterKind::Noise => slider(ui, "Quantidade", a, 0.0..=1.0),
                    }
                    ui.checkbox(preview, "Prévia na tela");
                    let (f, label) = filter_of(kind, *a, *b, *c);
                    let cur = st.cur;
                    if (changed || (*preview && !*applied))
                        && *preview
                        && let Some(d) = st.docs.get_mut(cur)
                    {
                        if *applied {
                            d.ed.undo();
                        }
                        let id = d.active;
                        *applied = d.ed.apply_filter(id, &f, label).is_ok();
                    } else if changed
                        && !*preview
                        && *applied
                        && let Some(d) = st.docs.get_mut(cur)
                    {
                        d.ed.undo();
                        *applied = false;
                    }
                    ui.add_space(8.0);
                    let applied_now = *applied;
                    ui.horizontal(|ui| {
                        if ui.button("Aplicar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| {
                                if !applied_now && let Some(d) = st.doc_mut() {
                                    let id = d.active;
                                    if let Err(e) = d.ed.apply_filter(id, &f, label) {
                                        st.warn(e.to_string());
                                    }
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| {
                                if applied_now && let Some(d) = st.doc_mut() {
                                    d.ed.undo();
                                }
                            }));
                        }
                    });
                }
                Dialog::Curves {
                    points,
                    drag,
                    applied,
                } => {
                    let side = 240.0;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(side, side), egui::Sense::click_and_drag());
                    let p = ui.painter();
                    p.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                    for i in 1..4 {
                        let t = i as f32 / 4.0;
                        let g = egui::Stroke::new(1.0, egui::Color32::from_gray(60));
                        p.line_segment(
                            [
                                rect.lerp_inside(vec2(t, 0.0)),
                                rect.lerp_inside(vec2(t, 1.0)),
                            ],
                            g,
                        );
                        p.line_segment(
                            [
                                rect.lerp_inside(vec2(0.0, t)),
                                rect.lerp_inside(vec2(1.0, t)),
                            ],
                            g,
                        );
                    }
                    let to_screen = |x: f32, y: f32| {
                        egui::pos2(
                            rect.left() + x * rect.width(),
                            rect.bottom() - y * rect.height(),
                        )
                    };
                    let lut = pawpaint_core::filters::curve_lut(points);
                    let line: Vec<egui::Pos2> = (0..256)
                        .map(|k| to_screen(k as f32 / 255.0, lut[k]))
                        .collect();
                    p.add(egui::Shape::line(
                        line,
                        egui::Stroke::new(2.0, theme::ACCENT),
                    ));
                    for (x, y) in points.iter() {
                        p.circle_filled(to_screen(*x, *y), 4.5, egui::Color32::WHITE);
                    }
                    let norm = |pos: egui::Pos2| {
                        (
                            ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                            ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
                        )
                    };
                    let mut changed = false;
                    if let Some(pos) = resp.interact_pointer_pos() {
                        if resp.drag_started() || resp.clicked() {
                            let near = points
                                .iter()
                                .position(|(x, y)| to_screen(*x, *y).distance(pos) < 10.0);
                            *drag = Some(near.unwrap_or_else(|| {
                                points.push(norm(pos));
                                points.len() - 1
                            }));
                            changed = true;
                        }
                        if resp.dragged()
                            && let Some(i) = *drag
                            && i < points.len()
                        {
                            points[i] = norm(pos);
                            changed = true;
                        }
                    }
                    if resp.secondary_clicked()
                        && let Some(pos) = resp.interact_pointer_pos()
                        && let Some(i) = points
                            .iter()
                            .position(|(x, y)| to_screen(*x, *y).distance(pos) < 10.0)
                    {
                        points.remove(i);
                        changed = true;
                    }
                    if resp.drag_stopped() {
                        *drag = None;
                    }
                    ui.label(
                        RichText::new("Clique cria um ponto, arraste move, botão direito apaga.")
                            .small()
                            .weak(),
                    );
                    let cur = st.cur;
                    if changed && let Some(d) = st.docs.get_mut(cur) {
                        if *applied {
                            d.ed.undo();
                        }
                        let id = d.active;
                        *applied =
                            d.ed.apply_filter(id, &Filter::Curves(points.clone()), "Curvas")
                                .is_ok();
                    }
                    let applied_now = *applied;
                    let pts = points.clone();
                    ui.horizontal(|ui| {
                        if ui.button("Aplicar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| {
                                if !applied_now && let Some(d) = st.doc_mut() {
                                    let id = d.active;
                                    let _ = d.ed.apply_filter(id, &Filter::Curves(pts), "Curvas");
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| {
                                if applied_now && let Some(d) = st.doc_mut() {
                                    d.ed.undo();
                                }
                            }));
                        }
                    });
                }
                Dialog::CanvasSize { w, h, anchor } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(w).range(1..=8192).suffix(" px"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(1..=8192).suffix(" px"));
                    });
                    ui.label(
                        RichText::new("Onde a imagem fica na tela nova:")
                            .small()
                            .weak(),
                    );
                    egui::Grid::new("anchor").show(ui, |ui| {
                        for i in 0..9u8 {
                            if ui.selectable_label(*anchor == i, "●").clicked() {
                                *anchor = i;
                            }
                            if i % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Aplicar").clicked() {
                            keep = false;
                            let (nw, nh, anchor) = (*w, *h, *anchor);
                            then = Some(Box::new(move |st| {
                                if let Some(d) = st.doc_mut() {
                                    commit_if_floating(d);
                                    let (ow, oh) =
                                        (d.ed.doc().width as i64, d.ed.doc().height as i64);
                                    let (ax, ay) = ((anchor % 3) as i64, (anchor / 3) as i64);
                                    let dx = (nw as i64 - ow) * ax / 2;
                                    let dy = (nh as i64 - oh) * ay / 2;
                                    d.ed.resize_canvas(nw, nh, dx, dy);
                                    d.fit = true;
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::ScaleImage {
                    w,
                    h,
                    keep: lock,
                    ratio,
                } => {
                    let before = (*w, *h);
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(w).range(1..=8192).suffix(" px"));
                        ui.label("×");
                        ui.add(egui::DragValue::new(h).range(1..=8192).suffix(" px"));
                    });
                    ui.checkbox(lock, "Manter proporção");
                    if *lock {
                        if *w != before.0 {
                            *h = ((*w as f32 / *ratio).round() as usize).clamp(1, 8192);
                        } else if *h != before.1 {
                            *w = ((*h as f32 * *ratio).round() as usize).clamp(1, 8192);
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Escalar").clicked() {
                            keep = false;
                            let (nw, nh) = (*w, *h);
                            then = Some(Box::new(move |st| {
                                if let Some(d) = st.doc_mut() {
                                    commit_if_floating(d);
                                    d.ed.scale_image(nw, nh);
                                    d.fit = true;
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Feather(r) => {
                    ui.add(egui::Slider::new(r, 1..=100).suffix(" px"));
                    ui.horizontal(|ui| {
                        if ui.button("Aplicar").clicked() {
                            keep = false;
                            let r = *r;
                            then = Some(Box::new(move |st| {
                                if let Some(d) = st.doc_mut() {
                                    d.ed.feather_selection(r);
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Grow(px) => {
                    ui.add(egui::Slider::new(px, -100..=100).suffix(" px"));
                    ui.label(
                        RichText::new("Positivo expande, negativo contrai.")
                            .small()
                            .weak(),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Aplicar").clicked() {
                            keep = false;
                            let px = *px;
                            then = Some(Box::new(move |st| {
                                if let Some(d) = st.doc_mut() {
                                    d.ed.grow_selection(px);
                                }
                            }));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::LayerProps(id) => {
                    let id = *id;
                    let cur = st.cur;
                    if let Some(l) = st.docs.get(cur).and_then(|d| d.ed.doc().layer(id).cloned()) {
                        let mut name = l.name.clone();
                        if ui.add(egui::TextEdit::singleline(&mut name)).changed() {
                            st.docs[cur]
                                .ed
                                .edit_layer(id, "name", "Renomear camada", |x| x.name = name);
                        }
                        let mut o = l.opacity * 100.0;
                        if ui
                            .add(
                                egui::Slider::new(&mut o, 0.0..=100.0)
                                    .suffix("%")
                                    .text("Opacidade"),
                            )
                            .changed()
                        {
                            st.docs[cur]
                                .ed
                                .edit_layer(id, "opacity", "Opacidade da camada", |x| {
                                    x.opacity = o / 100.0
                                });
                        }
                        let mut mode = l.mode;
                        egui::ComboBox::from_id_salt("props-blend")
                            .selected_text(mode.label())
                            .show_ui(ui, |ui| {
                                for m in BlendMode::ALL {
                                    ui.selectable_value(&mut mode, m, m.label());
                                }
                            });
                        if mode != l.mode {
                            st.docs[cur]
                                .ed
                                .edit_layer(id, "mode", "Modo de mistura", |x| x.mode = mode);
                        }
                    }
                    if ui.button("Fechar").clicked() {
                        keep = false;
                        st.docs[cur].ed.end_change();
                    }
                }
                Dialog::ConfirmClose(i) => {
                    let i = *i;
                    ui.label("Este documento tem mudanças que não foram salvas.");
                    ui.horizontal(|ui| {
                        if ui.button("Salvar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| {
                                st.cur = i.min(st.docs.len().saturating_sub(1));
                                save(st);
                                if st.docs.get(i).is_some_and(|d| !d.ed.is_dirty()) {
                                    close_doc(st, i);
                                }
                            }));
                        }
                        if ui.button("Descartar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |st| close_doc(st, i)));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Recover(files) => {
                    ui.label(format!(
                        "{} documento(s) não foram salvos da última vez que o app fechou.",
                        files.len()
                    ));
                    let files = files.clone();
                    ui.horizontal(|ui| {
                        if ui.button("Recuperar").clicked() {
                            keep = false;
                            let files = files.clone();
                            then = Some(Box::new(move |st| {
                                for f in files {
                                    if let Ok(ed) = file::load_project(&f) {
                                        add_doc(st, ed, None);
                                        if let Some(d) = st.doc_mut() {
                                            d.name = "Recuperado".into();
                                            d.ed.end_change();
                                        }
                                    }
                                    let _ = std::fs::remove_file(&f);
                                }
                                st.say("Documentos recuperados. Salve com Ctrl+S.");
                            }));
                        }
                        if ui.button("Descartar").clicked() {
                            keep = false;
                            then = Some(Box::new(move |_| {
                                for f in &files {
                                    let _ = std::fs::remove_file(f);
                                }
                            }));
                        }
                    });
                }
                Dialog::ConfirmQuit => {
                    ui.label("Há documentos com mudanças que não foram salvas.");
                    ui.horizontal(|ui| {
                        if ui.button("Sair sem salvar").clicked() {
                            keep = false;
                            then = Some(Box::new(|st| st.allow_close = true));
                        }
                        if ui.button("Cancelar").clicked() {
                            keep = false;
                        }
                    });
                }
            });
        if keep {
            st.dialog = Some(dialog);
        }
        if let Some(f) = then {
            f(st);
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        poll_file(&mut self.st);
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        for p in dropped {
            open_path(&mut self.st, &p);
        }
        self.shortcuts(&ctx);
        // Fim de um gesto: o próximo vira outro passo de desfazer.
        if ctx.input(|i| i.pointer.any_released()) {
            for d in &mut self.st.docs {
                d.ed.end_change();
            }
        }
        // Cores recentes: guarda a cor da frente depois de pintar.
        if self.st.interaction.is_none() {
            let fg = self.st.fg;
            if self.st.tool.paints()
                && self.st.prefs.recent.first() != Some(&fg)
                && ctx.input(|i| i.pointer.any_released())
            {
                self.st.prefs.recent.retain(|c| *c != fg);
                self.st.prefs.recent.insert(0, fg);
                self.st.prefs.recent.truncate(20);
                save_prefs(&self.st);
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && (self.st.allow_close || !self.st.docs.iter().any(|d| d.ed.is_dirty()))
        {
            for d in &self.st.docs {
                let _ = std::fs::remove_file(autosave_file(d.uid));
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.st.allow_close
            && self.st.docs.iter().any(|d| d.ed.is_dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.st.dialog = Some(Dialog::ConfirmQuit);
        }
        if self.st.allow_close && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let title = match self.st.doc() {
            Some(d) => format!("{} — PawPaint", d.title()),
            None => "PawPaint".to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        let bar = |fill| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 0))
        };
        egui::Panel::top("menu")
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("brushbar")
            .exact_size(34.0)
            .frame(bar(theme::BAR))
            .show(ui, |ui| self.brush_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(22.0)
            .frame(bar(theme::BG_DARK))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let style = theme::dock_style(ui.style().as_ref());
            DockArea::new(&mut self.dock)
                .style(style)
                .show_leaf_collapse_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_inside(ui, &mut Viewer { st: &mut self.st });
        });
        self.apply_dock_commands();
        if self.st.last_autosave.elapsed().as_secs() >= 45 {
            autosave(&mut self.st);
        }
        self.dialogs(&ctx);
        self.st.toasts.tick(&ctx);
        self.st.toasts.ui(&ctx);
        self.save_layout();
    }

    fn apply_dock_commands(&mut self) {
        for c in std::mem::take(&mut self.st.dock_commands) {
            match c {
                DockCommand::Reset => self.dock = default_layout(),
                DockCommand::Toggle(p) => {
                    if let Some(loc) = self.dock.find_tab(&p) {
                        self.dock.remove_tab(loc);
                    } else {
                        self.dock.push_to_focused_leaf(p);
                    }
                }
            }
        }
    }

    fn save_layout(&mut self) {
        if !self.frames.is_multiple_of(30) {
            return;
        }
        if let Ok(t) = serde_json::to_string(&self.dock)
            && t != self.last_layout
        {
            let _ = std::fs::create_dir_all(config_dir());
            let _ = std::fs::write(config_dir().join("layout.json"), &t);
            self.last_layout = t;
        }
    }

    fn screenshot_hook(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        let Some(path) = self.screenshot.clone() else {
            return;
        };
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
            self.st.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
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
    use egui::{Event, PointerButton, Pos2, RawInput, Rect, pos2};

    struct Harness {
        ctx: egui::Context,
        app: PaintApp,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = PaintApp::empty();
            app.st.preset_thumbs = app
                .st
                .presets
                .iter()
                .map(|b| docks::brush_thumb(&ctx, b, &b.name))
                .collect();
            add_doc(&mut app.st, Editor::new(300, 200, Background::White), None);
            let mut h = Self {
                ctx,
                app,
                time: 0.0,
            };
            for _ in 0..3 {
                h.frame(vec![]);
            }
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.016;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1480.0, 880.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(raw, |ui| self.app.frame(ui));
            out.drop_without_applying_deltas();
        }

        fn screen(&self, x: f32, y: f32) -> Pos2 {
            let d = self.app.st.doc().unwrap();
            d.view_origin + vec2(x, y) * d.zoom
        }

        fn mv(&mut self, x: f32, y: f32) {
            let p = self.screen(x, y);
            self.frame(vec![Event::PointerMoved(p)]);
        }

        fn button(&mut self, x: f32, y: f32, pressed: bool, mods: Modifiers) {
            let p = self.screen(x, y);
            self.frame(vec![Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: mods,
            }]);
        }

        fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
            self.mv(from.0, from.1);
            self.button(from.0, from.1, true, Modifiers::NONE);
            for i in 1..=4 {
                let t = i as f32 / 4.0;
                self.mv(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            }
            self.button(to.0, to.1, false, Modifiers::NONE);
            self.frame(vec![]);
        }

        fn px(&self, x: usize, y: usize) -> [u8; 4] {
            let d = self.app.st.doc().unwrap();
            d.ed.doc().layer(d.active).unwrap().pixels.pixel(x, y)
        }
    }

    #[test]
    fn dragging_with_the_brush_paints_and_undoes_as_one_step() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 12.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.fg = [255, 0, 0];
        h.app.st.tool = Tool::Brush;
        h.drag((40.0, 100.0), (240.0, 100.0));
        assert_eq!(h.px(140, 100), [255, 0, 0, 255]);
        assert_eq!(h.px(140, 130), [255, 255, 255, 255]);
        let d = h.app.st.doc_mut().unwrap();
        assert_eq!(d.ed.undo_label(), Some("Pincel"));
        d.ed.undo();
        assert_eq!(h.px(140, 100), [255, 255, 255, 255]);
    }

    #[test]
    fn eraser_selection_fill_and_bucket_follow_the_mouse() {
        let mut h = Harness::new();
        h.app.st.brush = Brush {
            size: 20.0,
            hardness: 1.0,
            ..Brush::basic()
        };
        h.app.st.tool = Tool::Eraser;
        h.drag((40.0, 50.0), (200.0, 50.0));
        assert_eq!(h.px(100, 50)[3], 0);
        // seleção retangular e preenchimento com Backspace
        set_tool(&mut h.app.st, Tool::SelRect);
        h.drag((20.0, 120.0), (120.0, 180.0));
        let sel = h
            .app
            .st
            .doc()
            .unwrap()
            .ed
            .doc()
            .selection
            .clone()
            .expect("seleção");
        assert_eq!(sel.bounds().map(|b| (b.0 / 10, b.1 / 10)), Some((2, 12)));
        h.app.st.fg = [0, 0, 255];
        edit_fill(&mut h.app.st, false);
        assert_eq!(h.px(60, 150), [0, 0, 255, 255]);
        assert_eq!(h.px(200, 150), [255, 255, 255, 255]);
        // balde de tinta: clica na parte branca de fora da seleção (a seleção segura o resto)
        h.app.st.doc_mut().unwrap().ed.deselect();
        set_tool(&mut h.app.st, Tool::Fill);
        h.app.st.fg = [0, 200, 0];
        h.mv(250.0, 20.0);
        h.button(250.0, 20.0, true, Modifiers::NONE);
        h.button(250.0, 20.0, false, Modifiers::NONE);
        assert_eq!(h.px(250, 20), [0, 200, 0, 255]);
        assert_eq!(h.px(60, 150), [0, 0, 255, 255]);
    }

    #[test]
    fn smudge_tool_drags_paint_with_the_mouse() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.active;
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 0, 0, 100, 200),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [255, 0, 0]).unwrap();
            d.ed.deselect();
        }
        h.app.st.brush = Brush {
            size: 24.0,
            hardness: 0.8,
            flow: 0.9,
            spacing: 0.15,
            ..Brush::basic()
        };
        set_tool(&mut h.app.st, Tool::Smudge);
        h.drag((80.0, 100.0), (200.0, 100.0));
        let p = h.px(130, 100);
        assert!(
            p[1] < 250 && p[0] == 255,
            "o vermelho devia ter sido arrastado: {p:?}"
        );
        assert_eq!(h.app.st.doc().unwrap().ed.undo_label(), Some("Borrão"));
    }

    #[test]
    fn move_tool_drags_the_layer_and_transform_waits_for_enter() {
        let mut h = Harness::new();
        {
            let d = h.app.st.doc_mut().unwrap();
            let id = d.ed.add_layer(None, None);
            d.active = id;
            d.ed.fill_selection(id, [9, 9, 9]).unwrap();
            d.ed.select_all();
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 10, 10, 30, 30),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [200, 0, 0]).unwrap();
            d.ed.deselect();
        }
        let id = h.app.st.doc().unwrap().active;
        h.app.st.doc_mut().unwrap().ed.clear_layer(id).unwrap();
        {
            let d = h.app.st.doc_mut().unwrap();
            d.ed.combine_selection(
                pawpaint_core::Selection::rect(300, 200, 10, 10, 30, 30),
                pawpaint_core::selection::SelOp::Replace,
                "s",
            );
            d.ed.fill_selection(id, [200, 0, 0]).unwrap();
            d.ed.deselect();
        }
        set_tool(&mut h.app.st, Tool::Move);
        h.drag((100.0, 100.0), (150.0, 130.0));
        assert!(
            h.app.st.doc().unwrap().float.is_none(),
            "Mover comita ao soltar"
        );
        assert_eq!(h.px(20, 20), [0; 4]);
        assert_eq!(h.px(70, 50), [200, 0, 0, 255]);
        // Transformar: solta a camada e espera o Enter
        set_tool(&mut h.app.st, Tool::Transform);
        h.frame(vec![]);
        assert!(h.app.st.doc().unwrap().float.is_some());
        canvas::confirm(&mut h.app.st, false);
        assert!(h.app.st.doc().unwrap().float.is_none());
        assert_eq!(h.px(70, 50), [200, 0, 0, 255]);
    }

    #[test]
    fn autosave_keeps_dirty_documents_and_leaves_live_instances_alone() {
        let mut h = Harness::new();
        let uid = h.app.st.doc().unwrap().uid;
        let d = h.app.st.doc_mut().unwrap();
        let id = d.active;
        d.ed.fill_selection(id, [1, 2, 3]).unwrap();
        autosave(&mut h.app.st);
        let mine = autosave_file(uid);
        assert!(mine.exists());
        assert!(
            leftovers().iter().all(|p| p != &mine),
            "instância viva não é sobra"
        );
        let dead = autosave_dir().join("4000000-7.pawpaint");
        std::fs::copy(&mine, &dead).unwrap();
        assert!(leftovers().contains(&dead));
        let ed = file::load_project(&dead).unwrap();
        assert_eq!(ed.doc().layers[0].pixels.pixel(5, 5), [1, 2, 3, 255]);
        let _ = std::fs::remove_file(&dead);
        h.app.st.doc_mut().unwrap().ed.mark_saved();
        autosave(&mut h.app.st);
        assert!(!mine.exists());
    }

    #[test]
    fn docks_render_with_content_in_every_panel() {
        let mut h = Harness::new();
        h.app.st.dialog = Some(Dialog::New {
            w: 100,
            h: 100,
            bg: 0,
            color: [1, 2, 3],
        });
        h.frame(vec![]);
        h.app.st.dialog = Some(Dialog::Filter {
            kind: FilterKind::Blur,
            a: 3.0,
            b: 0.0,
            c: 0.0,
            preview: true,
            applied: false,
        });
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.app.st.doc().unwrap().ed.undo_label().is_some());
        h.app.st.dialog = Some(Dialog::Curves {
            points: vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)],
            drag: None,
            applied: false,
        });
        h.frame(vec![]);
        h.frame(vec![]);
        h.app.st.dialog = None;
        for p in Panel::ALL {
            let mut layout = DockState::new(vec![Panel::Canvas]);
            layout.main_surface_mut().push_to_focused_leaf(p);
            h.app.dock = layout;
            h.frame(vec![]);
        }
        h.app.dock = default_layout();
        h.frame(vec![]);
        let _ = pos2(0.0, 0.0);
    }
}
