//! PawVoice: o modelo de uma sessão de áudio no estilo do Audacity.
//!
//! A sessão tem faixas empilhadas. Cada faixa é uma sequência contínua de trechos que apontam
//! para fontes de áudio imutáveis (mono, f32), começando no tempo 0; os buracos entre os
//! clipes são trechos de silêncio. Cortar, colar e mover só mexem nessas listas (barato, e o
//! desfazer guarda só elas). Efeitos geram uma fonte nova para o trecho processado.

mod editor;
mod file;
mod wav;

pub use editor::{Audio, EditError, Editor, Mixer, MoveTo, Planar, ProcessJob};
pub use file::{FileError, export_audio, load_session, save_session};
pub use wav::{read_wav_f32, write_wav_f32};

use serde::{Deserialize, Serialize};

/// Fonte 0 é silêncio (não ocupa memória).
pub type SourceId = u64;
pub const SILENCE: SourceId = 0;

/// Um pedaço `[start, end)` de uma fonte (em amostras).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl Segment {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn silence(len: usize) -> Self {
        Self {
            source: SILENCE,
            start: 0,
            end: len,
        }
    }
}

/// Ponto do envelope de volume (ferramenta Envelope): ganho linear em `at` (0 = mudo,
/// 1 = original, até 2 = +6 dB).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvPoint {
    pub at: usize,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub segments: Vec<Segment>,
    /// 1 = mono, 2 = estéreo.
    #[serde(default = "one")]
    pub channels: u8,
    /// Envelope de volume (vazio = sem mudança). Pontos em ordem de tempo.
    #[serde(default)]
    pub envelope: Vec<EnvPoint>,
    #[serde(default)]
    pub gain_db: f32,
    /// −1 esquerda … 1 direita.
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
    /// Cor da onda (índice numa paleta do app; 0 = azul padrão).
    #[serde(default)]
    pub color: u8,
}

fn one() -> u8 {
    1
}

impl Track {
    pub fn new(id: u64, name: String, segments: Vec<Segment>) -> Self {
        Self {
            id,
            name,
            segments,
            channels: 1,
            envelope: Vec::new(),
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            color: 0,
        }
    }

    /// Ganho do envelope em `t` (linear entre os pontos; antes do primeiro e depois do último
    /// vale o ponto da ponta, como no Audacity).
    pub fn env_at(&self, t: usize) -> f32 {
        let e = &self.envelope;
        match e.len() {
            0 => 1.0,
            _ => {
                let i = e.partition_point(|p| p.at <= t);
                if i == 0 {
                    e[0].value
                } else if i == e.len() {
                    e[i - 1].value
                } else {
                    let (a, b) = (e[i - 1], e[i]);
                    let k = (t - a.at) as f32 / (b.at - a.at).max(1) as f32;
                    a.value + (b.value - a.value) * k
                }
            }
        }
    }

    /// Fim do último áudio (silêncio no fim não conta).
    pub fn len(&self) -> usize {
        self.segments.iter().map(Segment::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clipes: trechos seguidos que não são silêncio, como `(início, fim)` na timeline.
    pub fn clips(&self) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0;
        for s in &self.segments {
            let end = pos + s.len();
            if s.source != SILENCE {
                match out.last_mut() {
                    Some(last) if last.1 == pos => last.1 = end,
                    _ => out.push((pos, end)),
                }
            }
            pos = end;
        }
        out
    }
}

/// Marca de texto na linha do tempo: um ponto (`end == at`) ou um trecho.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub at: usize,
    pub end: usize,
    pub text: String,
}

/// Faixa de rótulos (capítulos, marcações), sempre em ordem de tempo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelTrack {
    pub id: u64,
    pub name: String,
    pub labels: Vec<Label>,
}

pub const SESSION_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub sample_rate: u32,
    /// (A versão 1 chamava as faixas de "tomadas".)
    #[serde(alias = "takes")]
    pub tracks: Vec<Track>,
    /// Roteiro (texto do teleprompter).
    #[serde(default)]
    pub script: String,
    /// Faixas de rótulos (projetos antigos não têm).
    #[serde(default)]
    pub label_tracks: Vec<LabelTrack>,
    pub(crate) next_id: u64,
}

impl Session {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            version: SESSION_VERSION,
            sample_rate,
            tracks: Vec::new(),
            script: String::new(),
            label_tracks: Vec::new(),
            next_id: 1,
        }
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn label_track(&self, id: u64) -> Option<&LabelTrack> {
        self.label_tracks.iter().find(|t| t.id == id)
    }

    /// Todos os rótulos com texto, em ordem de tempo.
    fn named_labels(&self) -> Vec<&Label> {
        let mut all: Vec<&Label> = self
            .label_tracks
            .iter()
            .flat_map(|t| &t.labels)
            .filter(|l| !l.text.trim().is_empty())
            .collect();
        all.sort_by_key(|l| l.at);
        all
    }

    /// Capítulos para a descrição do YouTube: uma linha "m:ss Título" por rótulo. O YouTube
    /// só aceita a lista se ela começar em 0:00 e tiver ao menos 3 capítulos.
    pub fn youtube_chapters(&self) -> String {
        self.named_labels()
            .iter()
            .map(|l| {
                let secs = l.at / self.sample_rate.max(1) as usize;
                let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
                let t = if h > 0 {
                    format!("{h}:{m:02}:{s:02}")
                } else {
                    format!("{m}:{s:02}")
                };
                format!("{t} {}", l.text.trim())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Por que a lista de capítulos não serve no YouTube, se não servir.
    pub fn chapters_problem(&self) -> Option<&'static str> {
        let all = self.named_labels();
        if all.len() < 3 {
            Some("O YouTube pede pelo menos 3 capítulos")
        } else if all[0].at >= self.sample_rate.max(1) as usize {
            Some("O primeiro capítulo precisa começar em 0:00")
        } else {
            None
        }
    }

    /// Rótulos no formato do Audacity: "início<TAB>fim<TAB>texto", em segundos.
    pub fn labels_text(&self) -> String {
        let sr = self.sample_rate.max(1) as f64;
        self.label_tracks
            .iter()
            .flat_map(|t| &t.labels)
            .map(|l| {
                format!(
                    "{:.6}\t{:.6}\t{}",
                    l.at as f64 / sr,
                    l.end as f64 / sr,
                    l.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Duração do projeto: o fim da faixa mais longa.
    pub fn len(&self) -> usize {
        self.tracks.iter().map(Track::len).max().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Segundos em "m:ss.mmm" (ou "h:mm:ss.mmm").
pub fn format_time(samples: usize, sample_rate: u32) -> String {
    let ms = (samples as u128 * 1000 / sample_rate.max(1) as u128) as u64;
    let (h, m, s, r) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{r:03}")
    } else {
        format!("{m}:{s:02}.{r:03}")
    }
}

/// Lê "1:02.5", "62.5", "1:02:03.250" em amostras.
pub fn parse_time(text: &str, sample_rate: u32) -> Option<usize> {
    let mut secs = 0.0f64;
    for part in text.trim().replace(',', ".").split(':') {
        secs = secs * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    (secs >= 0.0).then(|| (secs * sample_rate as f64).round() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        assert_eq!(format_time(48_000 * 62 + 24_000, 48_000), "1:02.500");
        assert_eq!(parse_time("1:02.5", 48_000), Some(48_000 * 62 + 24_000));
        assert_eq!(parse_time("62,5", 48_000), Some(48_000 * 62 + 24_000));
        assert_eq!(parse_time("x", 48_000), None);
    }

    #[test]
    fn clips_are_the_non_silent_runs() {
        let t = Track::new(
            1,
            "a".into(),
            vec![
                Segment::silence(10),
                Segment {
                    source: 5,
                    start: 0,
                    end: 20,
                },
                Segment {
                    source: 6,
                    start: 0,
                    end: 5,
                },
                Segment::silence(3),
                Segment {
                    source: 5,
                    start: 20,
                    end: 30,
                },
            ],
        );
        assert_eq!(t.clips(), vec![(10, 35), (38, 48)]);
    }

    #[test]
    fn version_1_sessions_open_as_tracks() {
        let json = r#"{"version":1,"sample_rate":48000,"takes":[{"id":2,"name":"Tomada 1","segments":[{"source":1,"start":0,"end":10}]}],"next_id":3}"#;
        let s: Session = serde_json::from_str(json).unwrap();
        assert_eq!(s.tracks[0].name, "Tomada 1");
        assert_eq!(s.tracks[0].gain_db, 0.0);
    }
}
