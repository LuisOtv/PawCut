//! Núcleo do PawCut: modelo do projeto, edições da timeline e desfazer/refazer.
//! Não depende de mídia nem de interface.

mod time;

pub use time::{Frame, FrameRate, format_timecode};
