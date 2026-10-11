use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::ids::{ClipId, LinkId, MediaId, TrackId};
use crate::model::{Clip, ClipSource, MediaInfo, MediaKind, MediaRef, Project, Track, TrackKind, Transform};
use crate::time::Frame;
use crate::track_ops::{clear_range, insert_sorted};

/// Clipe a ser criado: o que mostrar, de onde a fonte começa e quanto dura.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipDraft {
    pub source: ClipSource,
    pub source_start: f64,
    pub duration: Frame,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub track: TrackId,
    pub draft: ClipDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackFlags {
    pub locked: bool,
    pub enabled: bool,
    pub muted: bool,
    pub solo: bool,
}

/// Uma alteração no projeto. Aplicada pelo `Editor`, que guarda o histórico.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    AddMedia(MediaInfo),
    AddTrack(TrackKind),
    RemoveTrack(TrackId),
    SetTrackFlags { track: TrackId, flags: TrackFlags },
    /// Coloca os itens em `at`, por cima do que houver (F10 no Resolve). Vários itens ficam ligados.
    Overwrite { at: Frame, items: Vec<Placement> },
    /// Overwrite no fim da timeline (Shift+F12).
    Append { items: Vec<Placement> },
}

/// O que a edição criou, para a interface selecionar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub media: Option<MediaId>,
    pub track: Option<TrackId>,
    pub clips: Vec<ClipId>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum EditError {
    #[error("o clipe {0:?} não existe")]
    ClipNotFound(ClipId),
    #[error("a faixa {0:?} não existe")]
    TrackNotFound(TrackId),
    #[error("a mídia {0:?} não existe")]
    MediaNotFound(MediaId),
    #[error("a faixa está travada")]
    TrackLocked,
    #[error("esse tipo de clipe não pode ir nessa faixa")]
    WrongTrackKind,
    #[error("valor inválido: {0}")]
    InvalidValue(&'static str),
    #[error("nada mudou")]
    NoChange,
}

pub(crate) fn apply(p: &mut Project, edit: Edit) -> Result<Outcome, EditError> {
    match edit {
        Edit::AddMedia(info) => {
            let id = MediaId(p.ids.fresh());
            p.media.push(MediaRef { id, info });
            Ok(Outcome { media: Some(id), ..Outcome::default() })
        }
        Edit::AddTrack(kind) => Ok(Outcome { track: Some(p.add_track(kind)), ..Outcome::default() }),
        Edit::RemoveTrack(id) => remove_track(p, id),
        Edit::SetTrackFlags { track, flags } => set_track_flags(p, track, flags),
        Edit::Overwrite { at, items } => {
            validate_items(p, at, &items)?;
            place(p, at, items)
        }
        Edit::Append { items } => {
            let at = p.duration();
            validate_items(p, at, &items)?;
            place(p, at, items)
        }
    }
}

/// Tira o link de grupos que ficaram com um clipe só (ex.: depois de apagar metade do par).
pub(crate) fn normalize_links(p: &mut Project) {
    let mut count: HashMap<LinkId, usize> = HashMap::new();
    for c in p.all_tracks().flat_map(|t| &t.clips) {
        if let Some(l) = c.link {
            *count.entry(l).or_default() += 1;
        }
    }
    for t in p.video_tracks.iter_mut().chain(p.audio_tracks.iter_mut()) {
        for c in &mut t.clips {
            if c.link.is_some_and(|l| count[&l] < 2) {
                c.link = None;
            }
        }
    }
}

fn unlocked_track(p: &Project, id: TrackId) -> Result<&Track, EditError> {
    let t = p.track(id).ok_or(EditError::TrackNotFound(id))?;
    if t.locked {
        return Err(EditError::TrackLocked);
    }
    Ok(t)
}

fn remove_track(p: &mut Project, id: TrackId) -> Result<Outcome, EditError> {
    let kind = p.track(id).ok_or(EditError::TrackNotFound(id))?.kind;
    if p.tracks(kind).len() <= 1 {
        return Err(EditError::InvalidValue("a timeline precisa de pelo menos uma faixa de cada tipo"));
    }
    p.tracks_mut(kind).retain(|t| t.id != id);
    Ok(Outcome::default())
}

fn set_track_flags(p: &mut Project, id: TrackId, flags: TrackFlags) -> Result<Outcome, EditError> {
    let t = p.track_mut(id).ok_or(EditError::TrackNotFound(id))?;
    t.locked = flags.locked;
    t.enabled = flags.enabled;
    t.muted = flags.muted;
    t.solo = flags.solo;
    Ok(Outcome::default())
}

fn validate_items(p: &Project, at: Frame, items: &[Placement]) -> Result<(), EditError> {
    if items.is_empty() {
        return Err(EditError::InvalidValue("nada para colocar"));
    }
    if at < 0 {
        return Err(EditError::InvalidValue("posição antes do início da timeline"));
    }
    let mut seen = HashSet::new();
    for pl in items {
        if !seen.insert(pl.track) {
            return Err(EditError::InvalidValue("dois itens na mesma faixa"));
        }
        validate_draft(p, pl)?;
    }
    Ok(())
}

fn validate_draft(p: &Project, pl: &Placement) -> Result<(), EditError> {
    let track = unlocked_track(p, pl.track)?;
    let d = &pl.draft;
    if d.duration < 1 {
        return Err(EditError::InvalidValue("a duração precisa ser de pelo menos 1 frame"));
    }
    if d.source_start < 0.0 {
        return Err(EditError::InvalidValue("início da fonte negativo"));
    }
    let fits = match &d.source {
        ClipSource::Media(id) => {
            let m = p.media(*id).ok_or(EditError::MediaNotFound(*id))?;
            matches!(
                (m.info.kind, track.kind),
                (MediaKind::Video { .. } | MediaKind::Image, TrackKind::Video)
                    | (MediaKind::Video { has_audio: true } | MediaKind::Audio, TrackKind::Audio)
            )
        }
        ClipSource::SolidColor(_) | ClipSource::Text(_) => track.kind == TrackKind::Video,
    };
    if !fits {
        return Err(EditError::WrongTrackKind);
    }
    if let Some(len) = p.source_length(&d.source)
        && d.source_start + d.duration as f64 > len as f64 + 1e-6
    {
        return Err(EditError::InvalidValue("o trecho passa do fim da mídia"));
    }
    Ok(())
}

/// Cria os clipes (já validados) por cima do que houver em `[at, at + duração)`.
fn place(p: &mut Project, at: Frame, items: Vec<Placement>) -> Result<Outcome, EditError> {
    let link = (items.len() > 1).then(|| LinkId(p.ids.fresh()));
    let mut out = Outcome::default();
    for pl in items {
        let id = ClipId(p.ids.fresh());
        let clip = Clip {
            id,
            source: pl.draft.source,
            start: at,
            duration: pl.draft.duration,
            source_start: pl.draft.source_start,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link,
        };
        let (track, ids) = p.track_and_ids(pl.track).ok_or(EditError::TrackNotFound(pl.track))?;
        clear_range(track, at, clip.end(), ids);
        insert_sorted(track, clip);
        out.clips.push(id);
    }
    Ok(out)
}
