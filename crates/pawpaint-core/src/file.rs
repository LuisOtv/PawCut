//! Arquivo `.pawpaint` (um zip com `doc.json` e um PNG por camada), importação e exportação.

use std::io::{Read, Write};
use std::path::Path;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::blend::BlendMode;
use crate::composite;
use crate::doc::{Doc, Layer};
use crate::editor::Editor;
use crate::tile::TiledImage;

const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("não deu para ler/escrever o arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("arquivo de projeto inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("arquivo zip inválido: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("imagem inválida: {0}")]
    Image(#[from] image::ImageError),
    #[error("projeto de uma versão mais nova do PawPaint")]
    TooNew,
}

#[derive(Serialize, Deserialize)]
struct LayerMeta {
    id: u64,
    name: String,
    visible: bool,
    locked: bool,
    alpha_lock: bool,
    opacity: f32,
    mode: BlendMode,
    file: String,
    #[serde(default)]
    mask: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct DocMeta {
    version: u32,
    width: usize,
    height: usize,
    next_id: u64,
    layers: Vec<LayerMeta>,
}

fn png_bytes(img: &RgbaImage) -> Result<Vec<u8>, FileError> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

pub fn save_project(ed: &Editor, path: &Path) -> Result<(), FileError> {
    let doc = ed.doc();
    // Primeiro num arquivo temporário: um erro no meio não estraga o projeto antigo.
    let tmp = path.with_extension("pawpaint.tmp");
    {
        let file = std::fs::File::create(&tmp)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut metas = Vec::new();
        for l in &doc.layers {
            let name = format!("layers/{}.png", l.id);
            zip.start_file(&name, opts)?;
            zip.write_all(&png_bytes(&l.pixels.to_rgba())?)?;
            let mut mask_name = None;
            if let Some(m) = &l.mask {
                let mn = format!("layers/{}-mask.png", l.id);
                zip.start_file(&mn, opts)?;
                zip.write_all(&png_bytes(&m.to_rgba())?)?;
                mask_name = Some(mn);
            }
            metas.push(LayerMeta {
                id: l.id,
                name: l.name.clone(),
                visible: l.visible,
                locked: l.locked,
                alpha_lock: l.alpha_lock,
                opacity: l.opacity,
                mode: l.mode,
                file: name,
                mask: mask_name,
            });
        }
        let meta = DocMeta {
            version: VERSION,
            width: doc.width,
            height: doc.height,
            next_id: doc.next_id(),
            layers: metas,
        };
        zip.start_file("doc.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&meta)?.as_bytes())?;
        // Imagem final, para miniaturas em gerenciadores de arquivos.
        zip.start_file("merged.png", opts)?;
        zip.write_all(&png_bytes(&composite::flatten(doc))?)?;
        zip.finish()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Editor, FileError> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let meta: DocMeta = {
        let mut s = String::new();
        zip.by_name("doc.json")?.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    if meta.version > VERSION {
        return Err(FileError::TooNew);
    }
    let mut doc = Doc::new(meta.width, meta.height);
    for m in meta.layers {
        let mut bytes = Vec::new();
        zip.by_name(&m.file)?.read_to_end(&mut bytes)?;
        let img = image::load_from_memory(&bytes)?.to_rgba8();
        let mut l = Layer::new(m.id, m.name, meta.width, meta.height);
        l.visible = m.visible;
        l.locked = m.locked;
        l.alpha_lock = m.alpha_lock;
        l.opacity = m.opacity;
        l.mode = m.mode;
        l.pixels = TiledImage::from_rgba(&img);
        if let Some(mn) = &m.mask {
            let mut mb = Vec::new();
            zip.by_name(mn)?.read_to_end(&mut mb)?;
            // Na máscara só o alfa importa (quanto está escondido).
            l.mask = Some(TiledImage::from_rgba(
                &image::load_from_memory(&mb)?.to_rgba8(),
            ));
        }
        doc.layers.push(l);
    }
    doc.set_next_id(meta.next_id);
    Ok(Editor::from_doc(doc))
}

/// Abre uma imagem comum (PNG, JPG, WebP, BMP…).
pub fn open_image(path: &Path) -> Result<RgbaImage, FileError> {
    Ok(image::open(path)?.to_rgba8())
}

/// Exporta a imagem final em PNG, JPG, WebP ou BMP (pelo tipo do arquivo). JPG vai sobre branco.
pub fn export_image(doc: &Doc, path: &Path, jpeg_quality: u8) -> Result<(), FileError> {
    let img = composite::flatten(doc);
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => {
            std::fs::write(path, encode_jpeg(&img, jpeg_quality))?;
        }
        _ => img.save(path)?,
    }
    Ok(())
}

pub fn encode_jpeg(img: &RgbaImage, quality: u8) -> Vec<u8> {
    // Transparência vira branco (JPG não tem alfa).
    let mut flat =
        RgbaImage::from_pixel(img.width(), img.height(), image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut flat, img, 0, 0);
    let rgb = image::DynamicImage::ImageRgba8(flat).to_rgb8();
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    let _ = rgb.write_with_encoder(enc);
    out
}

/// O JPG de maior qualidade que cabe em `max_bytes`. Devolve os bytes e a qualidade (mínimo 30).
pub fn jpeg_under(img: &RgbaImage, max_bytes: usize) -> (Vec<u8>, u8) {
    let mut best = (encode_jpeg(img, 30), 30);
    let (mut lo, mut hi) = (30u8, 98u8);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let bytes = encode_jpeg(img, mid);
        if bytes.len() <= max_bytes {
            best = (bytes, mid);
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Background;

    #[test]
    fn project_round_trip_keeps_layers_and_properties() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pawpaint");
        let mut e = Editor::new(130, 70, Background::White);
        let id = e.add_layer(Some("Tinta".into()), None);
        e.edit_layer(id, "p", "p", |l| {
            l.opacity = 0.4;
            l.mode = BlendMode::Multiply;
        });
        e.fill_selection(id, [10, 20, 30]).unwrap();
        e.edit_layer(id, "lock", "l", |l| l.alpha_lock = true);
        save_project(&e, &path).unwrap();
        let back = load_project(&path).unwrap();
        assert_eq!(back.doc().layers.len(), 2);
        let l = back.doc().layer(id).unwrap();
        assert_eq!(
            (l.name.as_str(), l.opacity, l.mode, l.alpha_lock),
            ("Tinta", 0.4, BlendMode::Multiply, true)
        );
        assert_eq!(l.pixels.pixel(100, 60), [10, 20, 30, 255]);
        assert_eq!(back.doc().next_id(), e.doc().next_id());
        assert!(!path.with_extension("pawpaint.tmp").exists());
    }

    #[test]
    fn export_png_jpg_and_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Editor::new(200, 100, Background::Transparent);
        let id = e.doc().layers[0].id;
        e.fill_selection(id, [200, 30, 30]).unwrap();
        export_image(e.doc(), &dir.path().join("o.png"), 90).unwrap();
        assert_eq!(
            image::open(dir.path().join("o.png"))
                .unwrap()
                .to_rgba8()
                .get_pixel(5, 5)
                .0,
            [200, 30, 30, 255]
        );
        export_image(e.doc(), &dir.path().join("o.jpg"), 90).unwrap();
        assert!(dir.path().join("o.jpg").exists());
        let mut noisy = RgbaImage::new(400, 300);
        let mut s = 7u32;
        for p in noisy.pixels_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *p = image::Rgba([(s >> 8) as u8, (s >> 16) as u8, (s >> 24) as u8, 255]);
        }
        let (bytes, q) = jpeg_under(&noisy, 90_000);
        assert!(bytes.len() <= 90_000 && q >= 30, "{} q{q}", bytes.len());
    }
}
