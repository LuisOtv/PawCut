//! Mede as operações pesadas do PawPaint numa foto grande, para achar o que trava a interface.
//! Uso: cargo run --release -p pawpaint-core --example bench_ops

use image::RgbaImage;
use pawpaint_core::brush::Brush;
use pawpaint_core::filters::Filter;
use pawpaint_core::{BlendMode, Editor, composite};
use std::time::Instant;

fn time<T>(nome: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{nome:<44} {:>8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    r
}

fn main() {
    let (w, h) = (4000u32, 3000u32);
    let mut seed = 12345u32;
    let img = RgbaImage::from_fn(w, h, |x, y| {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = (seed >> 28) as u8;
        image::Rgba([
            (x * 255 / w) as u8 ^ n,
            (y * 255 / h) as u8,
            ((x + y) % 256) as u8 ^ n,
            255,
        ])
    });
    let mut ed = Editor::from_image(&img, "Fundo");
    let top = ed.import_layer(&img, "Camada");
    let id = ed.doc().layers[0].id;
    println!("foto {w}x{h}, 2 camadas");

    time("achatar para exportar (composite::flatten)", || {
        composite::flatten(ed.doc())
    });

    // Um traço longo: 1500 pontos de um pincel de 60 px, como uma pincelada rápida.
    let mut brush = Brush::basic();
    brush.size = 60.0;
    time("traço de 1500 pontos (pincel 60 px)", || {
        ed.begin_stroke(top, brush.clone(), [200, 20, 20], false, BlendMode::Normal)
            .unwrap();
        for i in 0..1500 {
            let t = i as f32 / 1500.0;
            ed.stroke_point(200.0 + t * 3500.0, 1500.0 + (t * 25.0).sin() * 900.0);
            let _ = ed.take_dirty();
        }
        ed.end_stroke(3700.0, 1500.0);
    });
    time("desfazer o traço", || ed.undo());
    time("refazer o traço", || ed.redo());

    time("balde de tinta (camada, tolerância 30%)", || {
        ed.bucket_fill(id, (10, 10), [10, 200, 10], 0.3, true, 1.0, false)
            .unwrap()
    });
    time("balde de tinta (imagem toda visível)", || {
        ed.bucket_fill(id, (10, 10), [10, 10, 200], 0.3, true, 1.0, true)
            .unwrap()
    });
    time("preencher tudo", || {
        ed.fill_selection(id, [30, 30, 30]).unwrap()
    });
    time("inverter cores", || {
        ed.apply_filter(id, &Filter::Invert, "Inverter").unwrap()
    });
    time("brilho e contraste", || {
        ed.apply_filter(
            id,
            &Filter::BrightnessContrast {
                brightness: 0.2,
                contrast: 0.2,
            },
            "x",
        )
        .unwrap()
    });
    time("desfoque gaussiano raio 4", || {
        ed.apply_filter(id, &Filter::GaussianBlur(4.0), "x")
            .unwrap()
    });
    time("desfoque gaussiano raio 40", || {
        ed.apply_filter(id, &Filter::GaussianBlur(40.0), "x")
            .unwrap()
    });
    time("nitidez", || {
        ed.apply_filter(
            id,
            &Filter::Sharpen {
                amount: 1.0,
                radius: 2.0,
            },
            "x",
        )
        .unwrap()
    });
    time("pixelizar 16", || {
        ed.apply_filter(id, &Filter::Pixelate(16), "x").unwrap()
    });
    time("mesclar para baixo", || ed.merge_down(top).unwrap());
    time("girar a imagem 90 graus", || ed.rotate_image(true));
    time("escalar a imagem para 50%", || ed.scale_image(1500, 2000));
    time("selecionar tudo + inverter + suavizar 8", || {
        ed.select_all();
        ed.invert_selection();
        ed.feather_selection(8);
    });
}
