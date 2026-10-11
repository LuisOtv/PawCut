//! Compositor do PawCut: transforma a timeline em imagem e som.
//! É o mesmo código para o preview e para a exportação, então o MP4 sai igual ao que se vê.

mod compose;
mod layout;
mod mix;
mod text;

use std::sync::Arc;

use pawcut_core::{Clip, ClipId, ClipSource, Frame, MediaKind, Project, Rgba};
use pawcut_media::{MediaLibrary, RgbaImage};

use crate::layout::{Fit, base_size, dest_rect, fade_factor};
use crate::text::TextRasterizer;

/// Cor da camada quando o arquivo não abre ("Media Offline").
pub const OFFLINE_COLOR: Rgba = Rgba::new(150, 20, 30, 255);

pub struct RenderedFrame {
    pub image: RgbaImage,
    /// Clipes visíveis cujo arquivo não abriu.
    pub offline: Vec<ClipId>,
}

pub struct Renderer {
    media: MediaLibrary,
    text: TextRasterizer,
}

impl Renderer {
    pub fn new(media: MediaLibrary) -> Self {
        Self {
            media,
            text: TextRasterizer::new(),
        }
    }

    pub fn media(&mut self) -> &mut MediaLibrary {
        &mut self.media
    }

    /// Imagem do frame `frame` em `width`x`height` (o preview pode pedir menor que o projeto).
    pub fn render_frame(
        &mut self,
        project: &Project,
        frame: Frame,
        width: u32,
        height: u32,
    ) -> RenderedFrame {
        let s = &project.settings;
        let pj = (s.width as f32, s.height as f32);
        let out = (width as f32, height as f32);
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
        let mut offline = Vec::new();
        for track in project.video_tracks.iter().filter(|t| t.enabled) {
            let Some(clip) = track.clip_at(frame) else {
                continue;
            };
            let opacity = clip.transform.opacity * fade_factor(clip, frame);
            if opacity <= 0.0 {
                continue;
            }
            let t = &clip.transform;
            match &clip.source {
                ClipSource::SolidColor(color) => {
                    let r = dest_rect(base_size(Fit::Fill, pj, pj), t, pj, out);
                    compose::fill_rect(&mut image, r, *color, opacity);
                }
                ClipSource::Text(style) => {
                    let k = width as f32 / pj.0 * t.scale.max(0.01);
                    let img = self.text.rasterize(style, k);
                    let base = (img.width() as f32 / k, img.height() as f32 / k);
                    compose::draw_image(&mut image, &img, dest_rect(base, t, pj, out), opacity);
                }
                ClipSource::Media(_) => match self.media_layer(project, clip, frame) {
                    Some((img, base)) => {
                        compose::draw_image(&mut image, &img, dest_rect(base, t, pj, out), opacity)
                    }
                    None => {
                        offline.push(clip.id);
                        let r = dest_rect(base_size(Fit::Fill, pj, pj), t, pj, out);
                        compose::fill_rect(&mut image, r, OFFLINE_COLOR, 1.0);
                    }
                },
            }
        }
        RenderedFrame { image, offline }
    }

    /// Pixels e tamanho-base de um clipe de mídia; `None` se o arquivo não abre.
    fn media_layer(
        &mut self,
        project: &Project,
        clip: &Clip,
        frame: Frame,
    ) -> Option<(Arc<RgbaImage>, (f32, f32))> {
        let ClipSource::Media(id) = &clip.source else {
            return None;
        };
        let m = project.media(*id)?;
        let s = &project.settings;
        let pj = (s.width as f32, s.height as f32);
        let native = (m.info.width as f32, m.info.height as f32);
        match m.info.kind {
            MediaKind::Video { .. } => {
                let seconds = clip.source_time_at(frame) / s.fps.as_f64();
                let img = self.media.video_frame(&m.info.path, seconds).ok()?;
                Some((img, base_size(Fit::Contain, native, pj)))
            }
            MediaKind::Image => {
                let img = self.media.image(&m.info.path).ok()?;
                Some((img, base_size(Fit::Shrink, native, pj)))
            }
            MediaKind::Audio => None,
        }
    }

    /// Áudio mixado: `frames` frames estéreo a partir da amostra `start`, intercalados L/R.
    pub fn mix_audio(&mut self, project: &Project, start: i64, frames: usize) -> Vec<f32> {
        mix::mix(project, &mut self.media, start, frames)
    }
}
