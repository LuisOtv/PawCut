use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffmpeg_next as ff;
use image::RgbaImage;

use crate::MediaError;

/// O contexto do swscale não é `Send` no ffmpeg-next.
struct Scaler(ff::software::scaling::Context);

// SAFETY: o SwsContext não depende da thread que o criou. Ele vive dentro de um
// `VideoSource`, que tem dono único e só é usado por `&mut`, então nunca há acesso simultâneo.
unsafe impl Send for Scaler {}

/// Decodifica frames de um vídeo, já em RGBA e reduzidos para caber em `fit`.
/// Leitura sequencial é barata; pedir um frame para trás ou muito à frente faz seek.
pub struct VideoSource {
    path: PathBuf,
    input: ff::format::context::Input,
    decoder: ff::decoder::Video,
    scaler: Scaler,
    stream: usize,
    time_base: f64,
    start: i64,
    fps: f64,
    size: (u32, u32),
    /// Último frame entregue.
    last: Option<(i64, Arc<RgbaImage>)>,
    /// Frame já decodificado que passou do último pedido.
    ahead: Option<(i64, ff::frame::Video)>,
    /// O decodificador recebeu fim de arquivo.
    draining: bool,
    /// Não há mais frames.
    eof: bool,
}

/// Tamanho que cabe em `fit` mantendo a proporção, sem aumentar.
pub(crate) fn fit_within(native: (u32, u32), fit: (u32, u32)) -> (u32, u32) {
    let (w, h) = (native.0.max(1) as f64, native.1.max(1) as f64);
    let scale = (fit.0 as f64 / w).min(fit.1 as f64 / h).min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

impl VideoSource {
    pub fn open(path: &Path, fit: (u32, u32)) -> Result<Self, MediaError> {
        crate::init();
        let err = |e: ff::Error| MediaError::ffmpeg(path, e);
        let input = ff::format::input(path).map_err(err)?;
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or_else(|| MediaError::Unsupported {
                path: path.to_path_buf(),
            })?;
        let stream_index = stream.index();
        let time_base = f64::from(stream.time_base());
        // AV_NOPTS_VALUE = i64::MIN quando o arquivo não informa o início.
        let start = if stream.start_time() == i64::MIN {
            0
        } else {
            stream.start_time()
        };
        let valid = |r: ff::Rational| r.numerator() > 0 && r.denominator() > 0;
        let fps = [stream.avg_frame_rate(), stream.rate()]
            .into_iter()
            .find(|r| valid(*r))
            .map_or(30.0, f64::from);
        let decoder = ff::codec::context::Context::from_parameters(stream.parameters())
            .and_then(|c| c.decoder().video())
            .map_err(err)?;
        let size = fit_within((decoder.width(), decoder.height()), fit);
        let scaler = ff::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ff::format::Pixel::RGBA,
            size.0,
            size.1,
            ff::software::scaling::Flags::BILINEAR,
        )
        .map_err(err)?;
        Ok(Self {
            path: path.to_path_buf(),
            input,
            decoder,
            scaler: Scaler(scaler),
            stream: stream_index,
            time_base,
            start,
            fps,
            size,
            last: None,
            ahead: None,
            draining: false,
            eof: false,
        })
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Tamanho dos frames entregues.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Índice do frame mostrado em `seconds`.
    pub fn index_at(&self, seconds: f64) -> i64 {
        (seconds * self.fps + 1e-6).floor() as i64
    }

    /// Frame `index` (a partir de 0). Depois do fim, devolve o último frame.
    pub fn frame(&mut self, index: i64) -> Result<Arc<RgbaImage>, MediaError> {
        let index = index.max(0);
        if let Some((i, img)) = &self.last {
            let next = self.ahead.as_ref().map(|a| a.0);
            // O último frame entregue ainda vale até o próximo começar (ou até o fim do vídeo).
            if index == *i
                || (index > *i && (next.is_some_and(|n| index < n) || (self.eof && next.is_none())))
            {
                return Ok(img.clone());
            }
        }
        let pos = self
            .ahead
            .as_ref()
            .map(|a| a.0)
            .or(self.last.as_ref().map(|l| l.0));
        let far = (self.fps * 2.0) as i64;
        let backward = self.last.as_ref().is_some_and(|l| index < l.0);
        let too_far = pos.map_or(index > far, |p| index > p + far);
        if backward || too_far {
            self.seek(index)?;
        }

        let mut best: Option<(i64, ff::frame::Video)> = None;
        loop {
            if let Some((ai, f)) = self.ahead.take() {
                if ai <= index {
                    best = Some((ai, f));
                    continue;
                }
                self.ahead = Some((ai, f));
                break;
            }
            if self.eof {
                break;
            }
            match self.decode_next()? {
                Some(next) => self.ahead = Some(next),
                None => self.eof = true,
            }
        }

        let chosen = match best {
            Some(b) => Some(b),
            // Nada novo até o alvo: o último entregue continua valendo.
            None if self.last.is_some() => None,
            // O alvo fica antes do primeiro frame: entrega o primeiro.
            None => self.ahead.take(),
        };
        match chosen {
            Some((i, frame)) => {
                let img = Arc::new(self.convert(&frame)?);
                self.last = Some((i, img.clone()));
                Ok(img)
            }
            None => self
                .last
                .as_ref()
                .map(|(_, img)| img.clone())
                .ok_or_else(|| MediaError::Unsupported {
                    path: self.path.clone(),
                }),
        }
    }

    fn seek(&mut self, index: i64) -> Result<(), MediaError> {
        let seconds = index as f64 / self.fps + self.start as f64 * self.time_base;
        let ts = (seconds * 1_000_000.0) as i64;
        self.input
            .seek(ts, ..ts)
            .map_err(|e| MediaError::ffmpeg(&self.path, e))?;
        self.decoder.flush();
        self.last = None;
        self.ahead = None;
        self.draining = false;
        self.eof = false;
        Ok(())
    }

    /// Próximo frame decodificado com o seu índice, ou `None` no fim do arquivo.
    fn decode_next(&mut self) -> Result<Option<(i64, ff::frame::Video)>, MediaError> {
        loop {
            let mut frame = ff::frame::Video::empty();
            if self.decoder.receive_frame(&mut frame).is_ok() {
                let ts = frame.timestamp().or(frame.pts()).unwrap_or(0);
                let secs = (ts - self.start) as f64 * self.time_base;
                let index = (secs * self.fps).round() as i64;
                return Ok(Some((index, frame)));
            }
            if self.draining {
                return Ok(None);
            }
            let mut sent = false;
            for (stream, packet) in self.input.packets() {
                if stream.index() == self.stream {
                    // Pacote corrompido: pula e tenta o próximo.
                    if self.decoder.send_packet(&packet).is_ok() {
                        sent = true;
                        break;
                    }
                }
            }
            if !sent {
                self.decoder
                    .send_eof()
                    .map_err(|e| MediaError::ffmpeg(&self.path, e))?;
                self.draining = true;
            }
        }
    }

    fn convert(&mut self, frame: &ff::frame::Video) -> Result<RgbaImage, MediaError> {
        let mut out = ff::frame::Video::empty();
        self.scaler
            .0
            .run(frame, &mut out)
            .map_err(|e| MediaError::ffmpeg(&self.path, e))?;
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        let stride = out.stride(0);
        let data = out.data(0);
        let mut buf = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            buf.extend_from_slice(&data[row * stride..row * stride + w * 4]);
        }
        Ok(RgbaImage::from_raw(w as u32, h as u32, buf).expect("buffer do tamanho da imagem"))
    }
}
