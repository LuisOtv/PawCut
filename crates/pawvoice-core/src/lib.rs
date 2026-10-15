//! PawVoice: o modelo de uma sessão de gravação de voz.
//!
//! Cada tomada é uma lista de trechos que apontam para fontes de áudio imutáveis (mono, f32).
//! Cortar e apagar só mexem na lista (barato, e o desfazer guarda só ela). Efeitos geram uma
//! fonte nova para o trecho processado.

mod editor;
mod file;
mod wav;

pub use editor::{EditError, Editor};
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub id: u64,
    pub name: String,
    pub segments: Vec<Segment>,
}

impl Take {
    pub fn len(&self) -> usize {
        self.segments.iter().map(Segment::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub sample_rate: u32,
    pub takes: Vec<Take>,
    /// Roteiro (texto do teleprompter).
    #[serde(default)]
    pub script: String,
    pub(crate) next_id: u64,
}

impl Session {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            version: SESSION_VERSION,
            sample_rate,
            takes: Vec::new(),
            script: String::new(),
            next_id: 1,
        }
    }

    pub fn take(&self, id: u64) -> Option<&Take> {
        self.takes.iter().find(|t| t.id == id)
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
