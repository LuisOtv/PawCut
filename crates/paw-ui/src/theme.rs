//! Visual no estilo do DaVinci Resolve: cinzas escuros, pouco contraste, laranja nos destaques.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle,
};

pub const BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x21);
pub const BG_DARK: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);
pub const BAR: Color32 = Color32::from_rgb(0x27, 0x27, 0x2b);
pub const BAR_LIGHT: Color32 = Color32::from_rgb(0x30, 0x30, 0x35);
pub const BORDER: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x11);
pub const TEXT: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xcc);
/// Texto secundário: ≥ 4,5:1 sobre BG e BAR (o cinza anterior ficava em ~4,2:1).
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x96);
pub const ACCENT: Color32 = Color32::from_rgb(0xe8, 0x6a, 0x35);
pub const PLAYHEAD: Color32 = Color32::from_rgb(0xe8, 0x45, 0x40);
pub const TIMECODE: Color32 = Color32::from_rgb(0xf0, 0x7a, 0x4a);

pub const CLIP_VIDEO: Color32 = Color32::from_rgb(0x4a, 0x6b, 0x9c);
pub const CLIP_VIDEO_LABEL: Color32 = Color32::from_rgb(0x37, 0x52, 0x7c);
pub const CLIP_AUDIO: Color32 = Color32::from_rgb(0x2f, 0x6e, 0x50);
pub const CLIP_AUDIO_LABEL: Color32 = Color32::from_rgb(0x24, 0x56, 0x3e);
pub const WAVE: Color32 = Color32::from_rgb(0x8e, 0xd8, 0xae);
pub const CLIP_TITLE: Color32 = Color32::from_rgb(0x7a, 0x55, 0xa3);
pub const CLIP_GENERATOR: Color32 = Color32::from_rgb(0x66, 0x66, 0x78);
pub const CLIP_IMAGE: Color32 = Color32::from_rgb(0x8a, 0x6c, 0x42);
pub const OFFLINE: Color32 = Color32::from_rgb(0x9a, 0x22, 0x2c);

/// Grade da interface: tudo alinhado em múltiplos de 4 px.
pub const TOOLBAR_H: f32 = 28.0;
pub const PAD: f32 = 8.0;
pub const ICON: f32 = 16.0;
pub const ROW_H: f32 = 24.0;
/// Colunas do Inspector.
pub const LABEL_W: f32 = 104.0;
/// Menor largura da coluna de rótulos quando o painel fica estreito.
pub const LABEL_MIN_W: f32 = 60.0;
/// Largura mínima que um painel de propriedades aguenta sem cortar campos.
pub const PROPS_MIN_W: f32 = 260.0;
pub const VALUE_W: f32 = 84.0;
/// Largura de um campo numérico solto (fora do Inspector), com ou sem sufixo.
pub const NUM_W: f32 = 64.0;

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn text(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Fonte da interface: Noto Sans se existir (bem perto da Open Sans do Resolve).
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, path) in [
        (
            "noto",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        ),
        ("dejavu", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.into(), FontData::from_owned(bytes).into());
            if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
                list.insert(0, name.into());
            }
            break;
        }
    }
    fonts
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, text(10.5)),
            (TextStyle::Body, text(12.5)),
            (TextStyle::Button, text(12.5)),
            (TextStyle::Heading, text(16.0)),
            (TextStyle::Monospace, mono(12.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size = egui::vec2(40.0, 22.0);
        style.spacing.slider_width = crate::slider::RAIL_W;
        style.spacing.window_margin = Margin::same(12);
        style.spacing.menu_margin = Margin::same(6);
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = BAR;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.extreme_bg_color = BG_DARK;
        v.faint_bg_color = BAR;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.8);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = ACCENT;
        v.window_corner_radius = CornerRadius::same(4);
        v.menu_corner_radius = CornerRadius::same(3);
        let w = &mut v.widgets;
        for (state, fill) in [
            (&mut w.noninteractive, BG),
            (&mut w.inactive, BAR_LIGHT),
            (&mut w.hovered, Color32::from_rgb(0x3c, 0x3c, 0x43)),
            (&mut w.active, Color32::from_rgb(0x48, 0x48, 0x50)),
            (&mut w.open, BAR_LIGHT),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.corner_radius = CornerRadius::same(3);
            state.bg_stroke = Stroke::new(1.0, BORDER);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    });
}

/// Abas planas e escuras, como os painéis do Resolve.
pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut s = egui_dock::Style::from_egui(style);
    s.main_surface_border_stroke = Stroke::NONE;
    s.separator.width = 3.0;
    s.separator.color_idle = BORDER;
    s.separator.color_hovered = ACCENT.gamma_multiply(0.7);
    s.separator.color_dragged = ACCENT;
    s.tab_bar.bg_fill = BAR;
    s.tab_bar.height = 26.0;
    s.tab_bar.hline_color = BORDER;
    s.tab_bar.corner_radius = CornerRadius::ZERO;
    s.tab.tab_body.bg_fill = BG;
    s.tab.tab_body.stroke = Stroke::NONE;
    s.tab.tab_body.inner_margin = Margin::same(0);
    s.tab.tab_body.corner_radius = CornerRadius::ZERO;
    s.tab.hline_below_active_tab_name = false;
    for t in [
        &mut s.tab.active,
        &mut s.tab.focused,
        &mut s.tab.active_with_kb_focus,
        &mut s.tab.focused_with_kb_focus,
    ] {
        t.bg_fill = BAR;
        t.text_color = Color32::from_rgb(0xea, 0xea, 0xee);
        t.outline_color = Color32::TRANSPARENT;
        t.corner_radius = CornerRadius::ZERO;
    }
    for t in [
        &mut s.tab.inactive,
        &mut s.tab.inactive_with_kb_focus,
        &mut s.tab.hovered,
    ] {
        t.bg_fill = BAR;
        t.text_color = TEXT_DIM;
        t.outline_color = Color32::TRANSPARENT;
        t.corner_radius = CornerRadius::ZERO;
    }
    s.tab.hovered.text_color = TEXT;
    s.buttons.close_tab_color = TEXT_DIM;
    s.buttons.close_tab_active_color = Color32::WHITE;
    s.buttons.close_tab_bg_fill = BAR_LIGHT;
    s.overlay.selection_color = ACCENT.gamma_multiply(0.4);
    s
}

/// Barra de ferramentas no topo de um painel: altura, margem e alinhamento iguais em todos.
pub fn toolbar(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let rect =
        egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), TOOLBAR_H));
    ui.painter().rect_filled(rect, 0.0, BAR);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, BORDER),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(PAD, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            add(ui)
        },
    );
    ui.advance_cursor_after_rect(rect);
}

/// Seção recolhível no estilo do Resolve (cabeçalho com setinha); o estado fica na memória do egui.
pub fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.id().with(("section", title));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    let fill = if resp.hovered() { BAR_LIGHT } else { BAR };
    ui.painter().rect_filled(rect, 0.0, fill);
    let c = rect.left_center() + egui::vec2(PAD + 4.0, 0.0);
    let tri = if open {
        vec![
            c + egui::vec2(-4.0, -2.0),
            c + egui::vec2(4.0, -2.0),
            c + egui::vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + egui::vec2(-2.0, -4.0),
            c + egui::vec2(3.0, 0.0),
            c + egui::vec2(-2.0, 4.0),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(tri, TEXT, Stroke::NONE));
    ui.painter().text(
        rect.left_center() + egui::vec2(PAD + 14.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        text(12.0),
        Color32::WHITE,
    );
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    if open {
        ui.add_space(4.0);
        add(ui);
        ui.add_space(6.0);
    }
}

/// Linha do Inspector com colunas fixas: rótulo (à direita), valor, e até dois botões no fim
/// (◆ e ↺) sempre na mesma coluna. `value` recebe a área do valor.
pub fn row(
    ui: &mut egui::Ui,
    label: &str,
    value: impl FnOnce(&mut egui::Ui),
    extras: impl FnOnce(&mut egui::Ui),
) {
    let full = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), ROW_H));
    // Painel estreito: quem cede primeiro é a coluna do rótulo, nunca o campo de valor.
    let extras_w = 48.0;
    let label_w = (full.width() - PAD - VALUE_W - extras_w - 4.0).clamp(LABEL_MIN_W, LABEL_W);
    let label_rect = egui::Rect::from_min_size(full.min, egui::vec2(label_w, ROW_H));
    // Rótulo desenhado à mão; o widget invisível deixa ele acessível (leitor de tela, testes).
    if !label.is_empty() {
        ui.interact(
            label_rect,
            ui.id().with(("row", label)),
            egui::Sense::hover(),
        )
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    }
    // Recortado na própria coluna: rótulo comprido não invade o painel vizinho.
    ui.painter_at(label_rect).text(
        label_rect.right_center() - egui::vec2(4.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        label,
        text(12.0),
        TEXT_DIM,
    );
    let value_rect = egui::Rect::from_min_max(
        egui::pos2(label_rect.right() + PAD, full.top()),
        egui::pos2(
            (full.right() - extras_w - 4.0).max(label_rect.right() + PAD),
            full.bottom(),
        ),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(value_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().interact_size.x = VALUE_W;
            value(ui)
        },
    );
    let extras_rect =
        egui::Rect::from_min_max(egui::pos2(full.right() - extras_w, full.top()), full.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(extras_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            extras(ui)
        },
    );
    ui.advance_cursor_after_rect(full);
    ui.add_space(2.0);
}

/// Roda `add` com largura mínima fixa para campos (`DragValue`, `TextEdit`, `ComboBox`).
/// Sem isso o `DragValue` cresce e encolhe conforme os dígitos e o sufixo, e o layout pula
/// enquanto o valor é arrastado ou digitado.
pub fn fixed_width<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.x = width;
        add(ui)
    })
    .inner
}

/// Largura para um campo de texto/combo: a desejada, mas nunca mais do que cabe no espaço
/// que sobrou (em painéis estreitos o campo encolhe em vez de vazar para fora da linha).
pub fn fit_width(ui: &egui::Ui, wanted: f32) -> f32 {
    wanted.min(ui.available_width()).max(24.0)
}
