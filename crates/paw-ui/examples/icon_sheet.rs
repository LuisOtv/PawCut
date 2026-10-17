//! Folha com todos os ícones e um slider, para conferir o estilo da suíte.
//! Uso: cargo run -p paw-ui --example icon_sheet

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ícones Paw")
            .with_inner_size([900.0, 620.0]),
        ..Default::default()
    };
    let mut a = 40.0f32;
    let mut b = -6.0f32;
    let mut c = 24i32;
    eframe::run_ui_native("Ícones Paw", options, move |ui, _frame| {
        paw_ui::theme::apply(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            for size in [16.0, 24.0] {
                ui.label(format!("{size} px"));
                paw_ui::icons::sheet(ui, size);
                ui.add_space(8.0);
            }
            ui.add(
                paw_ui::Slider::new(&mut a, 0.0..=100.0)
                    .suffix("%")
                    .text("Opacidade"),
            );
            ui.add(
                paw_ui::Slider::new(&mut b, -24.0..=24.0)
                    .suffix(" dB")
                    .text("Ganho"),
            );
            ui.add(
                paw_ui::Slider::new(&mut c, 1..=500)
                    .suffix(" px")
                    .logarithmic(true)
                    .text("Tamanho"),
            );
        });
    })
}
