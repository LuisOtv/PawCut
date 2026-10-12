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

/// Lê os metadados de um arquivo para o Media Pool. A duração sai em frames do projeto
/// e o tamanho é o de exibição (vídeo de celular em pé já vem com largura e altura trocadas).
pub fn probe(path: &Path, fps: FrameRate) -> Result<MediaInfo, MediaError> {
    if is_image(path) {
        let (width, height) = crate::library::image_size(path)?;
        return Ok(MediaInfo {
            path: path.to_path_buf(),
            kind: MediaKind::Image,
            duration: None,
            width,
            height,
        });
    }
    crate::init();
    let mut input = ff::format::input(path).map_err(|e| MediaError::ffmpeg(path, e))?;
    let has_audio = input.streams().best(ff::media::Type::Audio).is_some();
    let seconds = if input.duration() > 0 {
        input.duration() as f64 / 1_000_000.0
    } else {
        scan_duration(&mut input)
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
            let (width, height) = crate::orient::displayed(
                (decoder.width(), decoder.height()),
                crate::orient::rotation_of(&stream),
            );
            Ok(MediaInfo {
                path: path.to_path_buf(),
                kind: MediaKind::Video { has_audio },
                duration,
                width,
                height,
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

/// Duração de arquivos sem duração no container (ex.: WebM gravado ao vivo pelo navegador):
/// a duração do stream, ou o fim do último pacote.
fn scan_duration(input: &mut ff::format::context::Input) -> f64 {
    let streams: Vec<(usize, f64, i64, i64)> = input
        .streams()
        .map(|s| {
            let start = if s.start_time() == i64::MIN {
                0
            } else {
                s.start_time()
            };
            (s.index(), f64::from(s.time_base()), s.duration(), start)
        })
        .collect();
    let from_streams = streams
        .iter()
        .filter(|s| s.2 > 0)
        .map(|s| s.2 as f64 * s.1)
        .fold(0.0, f64::max);
    if from_streams > 0.0 {
        return from_streams;
    }
    let mut end = 0.0f64;
    for (stream, packet) in input.packets() {
        let Some(&(_, tb, _, start)) = streams.iter().find(|s| s.0 == stream.index()) else {
            continue;
        };
        if let Some(pts) = packet.pts() {
            end = end.max((pts - start + packet.duration().max(0)) as f64 * tb);
        }
    }
    end
}
