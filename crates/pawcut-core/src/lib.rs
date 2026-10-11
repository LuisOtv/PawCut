//! Núcleo do PawCut: modelo do projeto, edições da timeline e desfazer/refazer.
//! Não depende de mídia nem de interface.

mod edit;
mod editor;
mod ids;
mod model;
mod time;
mod track_ops;

pub use time::{Frame, FrameRate, format_timecode};
pub use edit::{BladeTarget, ClipDraft, Edit, EditError, Outcome, Placement, TrackFlags};
pub use editor::Editor;
pub use ids::{ClipId, LinkId, MediaId, TrackId};
pub use model::{
    Clip, ClipSource, MediaInfo, MediaKind, MediaRef, PROJECT_VERSION, Project, ProjectSettings, Rgba,
    TextStyle, Track, TrackKind, Transform,
};
