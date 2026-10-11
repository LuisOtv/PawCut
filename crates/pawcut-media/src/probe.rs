use std::path::Path;

use ff::format::stream::Disposition;
use ffmpeg_next as ff;
use pawcut_core::{FrameRate, MediaInfo, MediaKind};

use crate::MediaError;

const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

pub(crate) fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Lê os metadados de um arquivo para o Media Pool. A duração sai em frames do projeto.
pub fn probe(path: &Path, fps: FrameRate) -> Result<MediaInfo, MediaError> {
    if is_image(path) {
        let (width, height) =
            image::image_dimensions(path).map_err(|source| MediaError::Image {
                path: path.to_path_buf(),
                source,
            })?;
        return Ok(MediaInfo {
            path: path.to_path_buf(),
            kind: MediaKind::Image,
            duration: None,
            width,
            height,
        });
    }
    crate::init();
    let input = ff::format::input(path).map_err(|e| MediaError::ffmpeg(path, e))?;
    let has_audio = input.streams().best(ff::media::Type::Audio).is_some();
    let seconds = if input.duration() > 0 {
        input.duration() as f64 / 1_000_000.0
    } else {
        0.0
    };
    let duration = Some(fps.seconds_to_frames(seconds).max(1));
    // Capa de álbum aparece como stream de vídeo "attached_pic": não conta como vídeo.
    let video = input.streams().find(|s| {
        s.parameters().medium() == ff::media::Type::Video
            && !s.disposition().contains(Disposition::ATTACHED_PIC)
    });
    match video {
        Some(stream) => {
            let decoder = ff::codec::context::Context::from_parameters(stream.parameters())
                .and_then(|c| c.decoder().video())
                .map_err(|e| MediaError::ffmpeg(path, e))?;
            Ok(MediaInfo {
                path: path.to_path_buf(),
                kind: MediaKind::Video { has_audio },
                duration,
                width: decoder.width(),
                height: decoder.height(),
            })
        }
        None if has_audio => Ok(MediaInfo {
            path: path.to_path_buf(),
            kind: MediaKind::Audio,
            duration,
            width: 0,
            height: 0,
        }),
        None => Err(MediaError::Unsupported {
            path: path.to_path_buf(),
        }),
    }
}
