//! Diálogos iguais nos quatro apps: onde abrem, que largura têm, como rolam e como se fecham.
//!
//! - [`window`]: janela de opções (Exportar, Novo, Filtro…). Abre centralizada na horizontal e
//!   perto do topo, sem seta de recolher e sem "×": fecha por Cancelar ou Esc.
//! - [`modal`]: decisão que bloqueia o app (sair sem salvar, recuperação). Escurece o fundo.
//! - [`scroll`]: dentro de um diálogo alto, só as opções rolam e os botões ficam à vista.
//!
//! Todos terminam com [`theme::dialog_actions`] e a ação principal em [`theme::primary_button`].

use egui::{Align2, Context, Id, Key, Modifiers, Ui};

use crate::theme;

/// Margem mínima entre o diálogo e a borda da janela do app.
const EDGE: f32 = 16.0;

/// Distância do topo da janela do app: um décimo da altura, entre 16 e 80 px.
fn top_offset(ctx: &Context) -> f32 {
    let h = ctx.content_rect().height();
    (h * 0.1).clamp(16.0, 80.0)
}

/// A largura pedida, mas nunca maior que a janela do app.
pub fn fit_width(ctx: &Context, wanted: f32) -> f32 {
    wanted.min((ctx.content_rect().width() - EDGE * 2.0).max(240.0))
}

/// Altura máxima do conteúdo que rola: o que sobra da janela menos título e botões.
pub fn body_max_height(ctx: &Context) -> f32 {
    (ctx.content_rect().height() - top_offset(ctx) - 150.0).max(120.0)
}

/// Janela de opções padrão. `id` precisa ser único por diálogo.
pub fn window<R>(
    ctx: &Context,
    id: &str,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let width = fit_width(ctx, width);
    egui::Window::new(title)
        .id(Id::new(("paw-dialog", id)))
        .collapsible(false)
        .resizable(false)
        .pivot(Align2::CENTER_TOP)
        .default_pos(egui::pos2(
            ctx.content_rect().center().x,
            ctx.content_rect().top() + top_offset(ctx),
        ))
        .show(ctx, |ui| {
            ui.set_width(width);
            add(ui)
        })
        .and_then(|r| r.inner)
}

/// Decisão que bloqueia o app. Título em destaque, largura fixa, fundo escurecido.
pub fn modal<R>(
    ctx: &Context,
    id: &str,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let width = fit_width(ctx, width);
    egui::Modal::new(Id::new(("paw-modal", id)))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.heading(title);
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

/// Conteúdo alto de um diálogo: rola sozinho, sem esconder os botões do pé.
pub fn scroll<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let max = body_max_height(ui.ctx());
    egui::ScrollArea::vertical()
        .max_height(max)
        .auto_shrink([false, true])
        .show(ui, add)
        .inner
}

/// Teclas de um diálogo: Esc cancela, Enter confirma. Com um campo de texto em foco as teclas
/// são do campo e nada é consumido.
#[derive(Clone, Copy, Default)]
pub struct Keys {
    pub esc: bool,
    pub enter: bool,
}

pub fn keys(ctx: &Context) -> Keys {
    if ctx.text_edit_focused() {
        return Keys::default();
    }
    ctx.input_mut(|i| Keys {
        esc: i.consume_key(Modifiers::NONE, Key::Escape),
        enter: i.consume_key(Modifiers::NONE, Key::Enter),
    })
}

/// Tamanho mínimo do pé: reaproveita o do tema para quem só quer Cancelar/Aplicar.
pub fn cancel_apply(ui: &mut Ui, apply: &str) -> (bool, bool) {
    theme::dialog_actions(ui, |ui| {
        let ok = theme::primary_button(ui, apply).clicked();
        let cancel = ui.button("Cancelar").clicked();
        (ok, cancel)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &Context, w: f32, h: f32, rows: usize, footer_y: &mut f32) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            window(&ctx, "t", "Teste", 560.0, |ui| {
                scroll(ui, |ui| {
                    for i in 0..rows {
                        ui.label(format!("opção {i}"));
                    }
                });
                theme::dialog_actions(ui, |ui| {
                    *footer_y = ui.cursor().max.y;
                    theme::primary_button(ui, "Aplicar");
                });
            });
        });
        out.drop_without_applying_deltas();
    }

    /// Em janela baixa ou estreita o diálogo fica inteiro na tela e o pé (com o botão principal)
    /// nunca fica abaixo da borda: só as opções rolam.
    #[test]
    fn stays_on_screen_with_footer_visible_in_small_windows() {
        for (w, h, rows) in [
            (960.0, 600.0, 80),
            (700.0, 380.0, 80),
            (400.0, 300.0, 5),
            (3840.0, 2160.0, 200),
        ] {
            let ctx = Context::default();
            let mut footer = 0.0;
            // dois frames: o primeiro mede, o segundo posiciona
            run(&ctx, w, h, rows, &mut footer);
            run(&ctx, w, h, rows, &mut footer);
            let rect = ctx
                .memory(|m| m.area_rect(Id::new(("paw-dialog", "t"))))
                .unwrap();
            assert!(
                rect.left() >= 0.0 && rect.right() <= w,
                "{w}x{h}: horizontal {rect:?}"
            );
            assert!(
                rect.top() >= 0.0 && rect.bottom() <= h,
                "{w}x{h}: vertical {rect:?}"
            );
            assert!(footer <= h, "{w}x{h}: pé em {footer}");
        }
    }
}
