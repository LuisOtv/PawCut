//! Rotação de exibição: vídeo de celular gravado em pé e fotos com orientação EXIF.

use ffmpeg_next as ff;
use image::RgbaImage;

/// Quantos graus (sentido horário: 0, 90, 180 ou 270) girar os frames para exibir em pé,
/// segundo a matriz de exibição do stream.
pub(crate) fn rotation_of(stream: &ff::format::stream::Stream) -> u32 {
    for sd in stream.side_data() {
        if sd.kind() != ff::codec::packet::side_data::Type::DisplayMatrix || sd.data().len() < 36 {
            continue;
        }
        let mut m = [0i32; 9];
        for (i, b) in sd.data()[..36].as_chunks::<4>().0.iter().enumerate() {
            m[i] = i32::from_ne_bytes(*b);
        }
        // SAFETY: `m` tem as 9 posições que a função lê.
        let ccw = unsafe { ff::ffi::av_display_rotation_get(m.as_ptr()) };
        if !ccw.is_finite() {
            return 0;
        }
        let cw = (-ccw).round() as i64;
        return ((cw + 45).rem_euclid(360) / 90 * 90) as u32;
    }
    0
}

pub(crate) fn rotate(img: RgbaImage, degrees: u32) -> RgbaImage {
    match degrees {
        90 => image::imageops::rotate90(&img),
        180 => image::imageops::rotate180(&img),
        270 => image::imageops::rotate270(&img),
        _ => img,
    }
}

/// Largura e altura como aparecem depois de girar.
pub(crate) fn displayed(size: (u32, u32), degrees: u32) -> (u32, u32) {
    if degrees % 180 == 90 {
        (size.1, size.0)
    } else {
        size
    }
}
