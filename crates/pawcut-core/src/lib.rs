//! Núcleo do PawCut: modelo do projeto, edições da timeline e desfazer/refazer.
//! Não depende de mídia nem de interface.

mod edit;
mod editor;
mod file;
mod ids;
mod model;
mod time;
mod track_ops;

pub use edit::{
    BladeTarget, ClipChange, ClipDraft, Edge, Edit, EditError, Outcome, Placement, TrackFlags,
};
pub use editor::Editor;
pub use file::{ProjectFileError, from_json, load_project, save_project, to_json};
pub use ids::{ClipId, LinkId, MediaId, TrackId};
pub use model::{
    Clip, ClipSource, MediaInfo, MediaKind, MediaRef, PROJECT_VERSION, Project, ProjectSettings,
    Rgba, TextStyle, Track, TrackKind, Transform,
};
pub use time::{Frame, FrameRate, format_timecode};
