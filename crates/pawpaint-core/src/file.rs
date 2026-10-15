//! Projeto no disco: `nome.pawpaint` (JSON) e as imagens em `nome.pawpaint-assets/` (PNG).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;

use crate::editor::Editor;
use crate::model::{DOC_VERSION, Doc, Kind};

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("não deu para ler/escrever o arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("arquivo de projeto inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("imagem inválida: {0}")]
    Image(#[from] image::ImageError),
    #[error("projeto de uma versão mais nova do PawPaint")]
    TooNew,
}

pub fn assets_dir(project: &Path) -> PathBuf {
    let mut name = project.file_name().unwrap_or_default().to_os_string();
    name.push("-assets");
    project.with_file_name(name)
}

pub fn save_project(ed: &Editor, path: &Path) -> Result<(), FileError> {
    let dir = assets_dir(path);
    std::fs::create_dir_all(&dir)?;
    for (name, img) in ed.assets() {
        let file = dir.join(name);
        // Imagens importadas nunca mudam: se já estão no disco, não regrava.
        if !file.exists() {
            img.save(&file)?;
        }
    }
    // Primeiro num arquivo temporário, para um erro no meio não estragar o projeto antigo.
    let tmp = path.with_extension("pawpaint.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(ed.doc())?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Abre o projeto. Devolve também os nomes das imagens que não foram encontradas.
pub fn load_project(path: &Path) -> Result<(Editor, Vec<String>), FileError> {
    let doc: Doc = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if doc.version > DOC_VERSION {
        return Err(FileError::TooNew);
    }
    let dir = assets_dir(path);
    let mut assets: BTreeMap<String, Arc<RgbaImage>> = BTreeMap::new();
    let mut missing = Vec::new();
    for l in &doc.layers {
        if let Kind::Image { asset } = &l.kind
            && !assets.contains_key(asset)
        {
            match image::open(dir.join(asset)) {
                Ok(img) => {
                    assets.insert(asset.clone(), Arc::new(img.to_rgba8()));
                }
                Err(_) => missing.push(asset.clone()),
            }
        }
    }
    Ok((Editor::from_parts(doc, assets), missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ShapeKind, THUMB_H, THUMB_W};

    #[test]
    fn save_and_load_round_trip_with_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thumb.pawpaint");
        let mut e = Editor::new(THUMB_W, THUMB_H);
        e.add_text("OI");
        e.add_shape(ShapeKind::Ellipse);
        let mut img = RgbaImage::new(8, 8);
        img.put_pixel(1, 2, image::Rgba([10, 20, 30, 255]));
        e.add_image("foto", img);
        save_project(&e, &path).unwrap();
        let (back, missing) = load_project(&path).unwrap();
        assert!(missing.is_empty());
        assert_eq!(back.doc(), e.doc());
        let asset = back.assets().values().next().unwrap();
        assert_eq!(asset.get_pixel(1, 2).0, [10, 20, 30, 255]);
    }

    #[test]
    fn a_missing_image_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pawpaint");
        let mut e = Editor::new(100, 100);
        e.add_image("foto", RgbaImage::new(4, 4));
        save_project(&e, &path).unwrap();
        std::fs::remove_dir_all(assets_dir(&path)).unwrap();
        let (back, missing) = load_project(&path).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(back.doc().layers.len(), 1);
    }
}
