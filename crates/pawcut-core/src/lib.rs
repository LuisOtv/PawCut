//! Núcleo do PawCut: modelo do projeto, edições da timeline e desfazer/refazer.
//! Não depende de mídia nem de interface.

mod ids;
mod model;
mod time;
mod track_ops;

pub use time::{Frame, FrameRate, format_timecode};
pub use ids::{ClipId, LinkId, MediaId, TrackId};
pub use model::{
    Clip, ClipSource, MediaInfo, MediaKind, MediaRef, PROJECT_VERSION, Project, ProjectSettings, Rgba,
    TextStyle, Track, TrackKind, Transform,
};
