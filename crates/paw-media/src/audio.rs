use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ffmpeg_next as ff;

use crate::MediaError;

/// Áudio decodificado: estéreo intercalado (L, R, L, R...) em f32.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl Pcm {
    /// Número de frames estéreo.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Frame `i` como `[L, R]`; silêncio fora do intervalo.
    pub fn frame(&self, i: i64) -> [f32; 2] {
        if i < 0 {
            return [0.0, 0.0];
        }
        let i = i as usize * 2;
        match self.samples.get(i..i + 2) {
            Some(s) => [s[0], s[1]],
            None => [0.0, 0.0],
        }
    }
}

/// Cadeia de `atempo` para a velocidade (cada atempo só aceita de 0.5 a 100).
pub(crate) fn atempo_chain(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-9 {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut s = speed;
    while s < 0.5 {
        parts.push("atempo=0.5".to_string());
        s /= 0.5;
    }
    parts.push(format!("atempo={s}"));
    parts.join(",")
}

/// Canais do stream. Alguns decoders (WAV de 16 bits no ffmpeg 6/7) deixam o layout vazio e só
/// informam a contagem; sem este reserva o filtro recebia `channel_layout=0c` e falhava.
fn channel_count(decoder: &ff::codec::decoder::Audio) -> u16 {
    match decoder.channel_layout().channels() {
        n if n > 0 => n as u16,
        _ => decoder.channels().max(1),
    }
}

fn layout_name(channels: u16) -> String {
    match channels {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}c"),
    }
}

/// Decodifica o melhor stream de áudio para estéreo em `sample_rate`, já na velocidade `speed`
/// (sem mudar o tom).
pub fn decode_audio(path: &Path, sample_rate: u32, speed: f64) -> Result<Pcm, MediaError> {
    crate::init();
    let err = |e: ff::Error| MediaError::ffmpeg(path, e);
    let mut input = ff::format::input(path).map_err(err)?;
    let stream = input
        .streams()
        .best(ff::media::Type::Audio)
        .ok_or_else(|| MediaError::Unsupported {
            path: path.to_path_buf(),
        })?;
    let index = stream.index();
    let tb = stream.time_base();
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|c| c.decoder().audio())
        .map_err(err)?;

    let src = format!(
        "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
        tb.numerator(),
        tb.denominator(),
        decoder.rate(),
        decoder.format().name(),
        layout_name(channel_count(&decoder)),
    );
    let mut chain =
        format!("aresample={sample_rate},aformat=sample_fmts=flt:channel_layouts=stereo");
    let tempo = atempo_chain(speed);
    if !tempo.is_empty() {
        chain.push(',');
        chain.push_str(&tempo);
    }
    let mut graph = ff::filter::Graph::new();
    let abuffer = ff::filter::find("abuffer").expect("ffmpeg sem o filtro abuffer");
    let abuffersink = ff::filter::find("abuffersink").expect("ffmpeg sem o filtro abuffersink");
    graph.add(&abuffer, "in", &src).map_err(err)?;
    graph.add(&abuffersink, "out", "").map_err(err)?;
    graph
        .output("in", 0)
        .and_then(|p| p.input("out", 0))
        .and_then(|p| p.parse(&chain))
        .map_err(err)?;
    graph.validate().map_err(err)?;

    let mut samples = Vec::new();
    let mut frame = ff::frame::Audio::empty();
    let push = |graph: &mut ff::filter::Graph, frame: &ff::frame::Audio, samples: &mut Vec<f32>| {
        let _ = graph.get("in").expect("filtro in").source().add(frame);
        drain(graph, samples);
    };
    for (s, packet) in input.packets() {
        if s.index() != index || decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut frame).is_ok() {
            push(&mut graph, &frame, &mut samples);
        }
    }
    let _ = decoder.send_eof();
    while decoder.receive_frame(&mut frame).is_ok() {
        push(&mut graph, &frame, &mut samples);
    }
    let _ = graph.get("in").expect("filtro in").source().flush();
    drain(&mut graph, &mut samples);
    Ok(Pcm {
        sample_rate,
        samples,
    })
}

fn drain(graph: &mut ff::filter::Graph, samples: &mut Vec<f32>) {
    let mut out = ff::frame::Audio::empty();
    while graph
        .get("out")
        .expect("filtro out")
        .sink()
        .frame(&mut out)
        .is_ok()
    {
        let n = out.samples() * 2;
        samples.extend(
            out.data(0)[..n * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
    }
}

fn cache_file(dir: &Path, path: &Path, sample_rate: u32, speed: f64) -> Option<PathBuf> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let mut h = DefaultHasher::new();
    (path, meta.len(), mtime, sample_rate, speed.to_bits()).hash(&mut h);
    Some(dir.join(format!("{:016x}.f32", h.finish())))
}

/// Igual a `decode_audio`, mas guarda o resultado em `cache_dir` para abrir rápido da próxima vez.
pub fn load_audio(
    path: &Path,
    sample_rate: u32,
    speed: f64,
    cache_dir: Option<&Path>,
) -> Result<Pcm, MediaError> {
    let file = cache_dir.and_then(|d| cache_file(d, path, sample_rate, speed));
    if let Some(file) = &file
        && let Some(samples) = read_cache(file)
    {
        return Ok(Pcm {
            sample_rate,
            samples,
        });
    }
    let pcm = decode_audio(path, sample_rate, speed)?;
    if let Some(file) = file {
        write_cache(&file, &pcm.samples).map_err(|source| MediaError::Cache {
            path: file.clone(),
            source,
        })?;
    }
    Ok(pcm)
}

/// Lê o cache em pedaços, sem guardar uma cópia inteira em bytes.
fn read_cache(file: &Path) -> Option<Vec<f32>> {
    use std::io::Read;
    let mut f = fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    let mut samples = Vec::with_capacity(len / 4);
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        // Os pedaços de 1 MiB são múltiplos de 4, exceto um final truncado (cache corrompido).
        if n % 4 != 0 {
            return None;
        }
        samples.extend(
            chunk[..n]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
    }
    Some(samples)
}

fn write_cache(file: &Path, samples: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("tmp");
    let mut out = std::io::BufWriter::new(fs::File::create(&tmp)?);
    for v in samples {
        out.write_all(&v.to_le_bytes())?;
    }
    out.flush()?;
    drop(out);
    fs::rename(&tmp, file)
}

/// Mínimo e máximo (mono) de cada um dos `buckets` pedaços iguais: a waveform da timeline.
pub fn peaks(pcm: &Pcm, buckets: usize) -> Vec<(f32, f32)> {
    let frames = pcm.frames();
    if buckets == 0 || frames == 0 {
        return Vec::new();
    }
    (0..buckets)
        .map(|b| {
            let from = b * frames / buckets;
            let to = ((b + 1) * frames / buckets).max(from + 1).min(frames);
            (from..to).fold((f32::MAX, f32::MIN), |(lo, hi), i| {
                let [l, r] = pcm.frame(i as i64);
                let m = (l + r) * 0.5;
                (lo.min(m), hi.max(m))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::atempo_chain;

    #[test]
    fn atempo_chain_splits_slow_speeds() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(4.0), "atempo=4");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5");
    }
}
