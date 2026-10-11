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
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x80, 0x80, 0x88);
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
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.spacing.interact_size.y = 20.0;
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

/// Faixa de cabeçalho (barra de ferramentas) no topo de um painel; devolve o resto da área.
pub fn header_strip(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
    ui.painter().rect_filled(rect, 0.0, BAR);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, BORDER),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        add,
    );
    ui.advance_cursor_after_rect(rect);
}
