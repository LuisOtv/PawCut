//! Exporta um `.pawcut` para MP4 pela linha de comando.
//! Uso: cargo run --release -p pawcut-export --example render_project -- projeto.pawcut saida.mp4

use std::sync::atomic::AtomicBool;

use pawcut_export::{ExportSettings, export};
use pawcut_media::{MediaLibrary, default_cache_dir};
use pawcut_render::Renderer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("uso: render_project <projeto.pawcut> <saida.mp4>");
        std::process::exit(2);
    };
    let project = pawcut_core::load_project(input.as_ref())?;
    let s = &project.settings;
    let mut renderer = Renderer::new(MediaLibrary::new(
        (s.width, s.height),
        s.sample_rate,
        default_cache_dir(),
    ));
    let settings = ExportSettings {
        output: output.into(),
        crf: 18,
    };
    export(
        &project,
        &mut renderer,
        &settings,
        &AtomicBool::new(false),
        |p| {
            if p.done % 30 == 0 || p.done == p.total {
                eprint!("\r{}/{} frames", p.done, p.total);
            }
        },
    )?;
    eprintln!("\npronto: {output}");
    Ok(())
}
