use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::ids::{ClipId, LinkId, MediaId, TrackId};
use crate::model::{Clip, ClipSource, MediaInfo, MediaKind, MediaRef, Project, Track, TrackKind, Transform};
use crate::time::Frame;
use crate::track_ops::{clear_range, insert_sorted, relink, ripple_open, split_clip};

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

#[derive(Debug, Clone, PartialEq)]
pub enum BladeTarget {
    /// Cortar só estes clipes (seleção, já expandida pelo Linked Selection).
    Clips(Vec<ClipId>),
    /// Cortar tudo sob o cursor nas faixas destravadas (Ctrl+B sem seleção).
    AllTracks,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
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
    /// Corta em `at`. Clipes em que `at` cai na borda são ignorados.
    Blade { at: Frame, target: BladeTarget },
    /// Apaga deixando buraco.
    Delete { clips: Vec<ClipId> },
    /// Apaga e puxa o resto da mesma faixa para fechar o buraco (Shift+Backspace).
    RippleDelete { clips: Vec<ClipId> },
    /// Arrasta clipes no tempo e entre faixas do mesmo tipo, sobrescrevendo o destino.
    Move { clips: Vec<ClipId>, delta: Frame, video_track_delta: i32, audio_track_delta: i32 },
    /// Arrasta a borda dos clipes. Positivo = para a direita.
    Trim { clips: Vec<ClipId>, edge: Edge, delta: Frame },
    /// Abre espaço em `at` em todas as faixas destravadas e coloca os itens (F9).
    Insert { at: Frame, items: Vec<Placement> },
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
        Edit::Blade { at, target } => blade(p, at, target),
        Edit::Delete { clips } => delete(p, clips, false),
        Edit::RippleDelete { clips } => delete(p, clips, true),
        Edit::Move { clips, delta, video_track_delta, audio_track_delta } => {
            move_clips(p, clips, delta, video_track_delta, audio_track_delta)
        }
        Edit::Trim { clips, edge, delta } => trim(p, clips, edge, delta),
        Edit::Insert { at, items } => {
            validate_items(p, at, &items)?;
            let len = items.iter().map(|pl| pl.draft.duration).max().unwrap_or(0);
            let mut links = HashMap::new();
            let Project { video_tracks, audio_tracks, ids, .. } = &mut *p;
            for track in video_tracks.iter_mut().chain(audio_tracks.iter_mut()).filter(|t| !t.locked) {
                ripple_open(track, at, len, &mut links, ids);
            }
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

/// Tira IDs repetidos, mantendo a ordem.
fn dedup(ids: Vec<ClipId>) -> Vec<ClipId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// Faixa e índice de um clipe, exigindo faixa destravada.
fn locate_unlocked(p: &Project, clip: ClipId) -> Result<(TrackId, usize), EditError> {
    let (tid, i) = p.locate(clip).ok_or(EditError::ClipNotFound(clip))?;
    unlocked_track(p, tid)?;
    Ok((tid, i))
}

fn blade(p: &mut Project, at: Frame, target: BladeTarget) -> Result<Outcome, EditError> {
    let targets: Vec<ClipId> = match target {
        BladeTarget::Clips(clips) => {
            let clips = dedup(clips);
            for &id in &clips {
                locate_unlocked(p, id)?;
            }
            clips
        }
        BladeTarget::AllTracks => {
            p.all_tracks().filter(|t| !t.locked).filter_map(|t| t.clip_at(at)).map(|c| c.id).collect()
        }
    };
    let targets: Vec<ClipId> = targets
        .into_iter()
        .filter(|id| p.clip(*id).is_some_and(|c| c.start < at && at < c.end()))
        .collect();
    if targets.is_empty() {
        return Err(EditError::NoChange);
    }
    let mut links = HashMap::new();
    let mut out = Outcome::default();
    for id in targets {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let (track, ids) = p.track_and_ids(tid).expect("faixa do clipe existe");
        let clip = track.clips.remove(i);
        let (left, mut right) = split_clip(&clip, at, ClipId(ids.fresh()));
        right.link = relink(clip.link, &mut links, ids);
        out.clips.push(right.id);
        track.clips.insert(i, left);
        track.clips.insert(i + 1, right);
    }
    Ok(out)
}

fn delete(p: &mut Project, clips: Vec<ClipId>, ripple: bool) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    let mut removed: Vec<(TrackId, Clip)> = Vec::new();
    for id in clips {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let track = p.track_mut(tid).expect("faixa do clipe existe");
        removed.push((tid, track.clips.remove(i)));
    }
    if ripple {
        // Do fim para o começo, para que cada deslocamento não atrapalhe o próximo.
        removed.sort_by_key(|(_, c)| std::cmp::Reverse(c.start));
        for (tid, gone) in &removed {
            let track = p.track_mut(*tid).expect("faixa do clipe existe");
            for c in track.clips.iter_mut().filter(|c| c.start >= gone.end()) {
                c.start -= gone.duration;
            }
        }
    }
    Ok(Outcome::default())
}

fn move_clips(p: &mut Project, clips: Vec<ClipId>, delta: Frame, vtd: i32, atd: i32) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    let mut plan: Vec<(ClipId, TrackId)> = Vec::new();
    let mut min_start = Frame::MAX;
    for &id in &clips {
        let (tid, i) = locate_unlocked(p, id)?;
        let track = p.track(tid).expect("faixa do clipe existe");
        let kind = track.kind;
        let idx = p.tracks(kind).iter().position(|t| t.id == tid).expect("faixa está na lista") as i64;
        let td = i64::from(match kind {
            TrackKind::Video => vtd,
            TrackKind::Audio => atd,
        });
        let dest = usize::try_from(idx + td)
            .ok()
            .and_then(|d| p.tracks(kind).get(d))
            .ok_or(EditError::InvalidValue("não existe faixa nessa direção"))?;
        if dest.locked {
            return Err(EditError::TrackLocked);
        }
        min_start = min_start.min(track.clips[i].start);
        plan.push((id, dest.id));
    }
    let delta = delta.max(-min_start);
    if delta == 0 && plan.iter().all(|(id, dest)| p.locate(*id).map(|(t, _)| t) == Some(*dest)) {
        return Err(EditError::NoChange);
    }
    let mut moving: Vec<(Clip, TrackId)> = Vec::new();
    for (id, dest) in plan {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let mut clip = p.track_mut(tid).expect("faixa do clipe existe").clips.remove(i);
        clip.start += delta;
        moving.push((clip, dest));
    }
    let mut out = Outcome::default();
    for (clip, dest) in moving {
        let (track, ids) = p.track_and_ids(dest).expect("faixa de destino validada");
        clear_range(track, clip.start, clip.end(), ids);
        out.clips.push(clip.id);
        insert_sorted(track, clip);
    }
    Ok(out)
}

fn trim(p: &mut Project, clips: Vec<ClipId>, edge: Edge, delta: Frame) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    let (mut lo, mut hi) = (Frame::MIN, Frame::MAX);
    for &id in &clips {
        let (tid, i) = locate_unlocked(p, id)?;
        let track = p.track(tid).expect("faixa do clipe existe");
        let c = &track.clips[i];
        let (l, h) = match edge {
            Edge::End => {
                let mut h = track.clips.get(i + 1).map_or(Frame::MAX, |n| n.start - c.end());
                if let Some(len) = p.source_length(&c.source) {
                    let max_dur = ((len as f64 - c.source_start) / c.speed + 1e-9).floor() as Frame;
                    h = h.min(max_dur - c.duration);
                }
                (1 - c.duration, h)
            }
            Edge::Start => {
                let prev_end = if i > 0 { track.clips[i - 1].end() } else { 0 };
                let mut l = prev_end - c.start;
                if p.source_length(&c.source).is_some() {
                    // a fonte não pode começar antes do 0
                    l = l.max(-(((c.source_start / c.speed) + 1e-9).floor() as Frame));
                }
                (l, c.duration - 1)
            }
        };
        lo = lo.max(l);
        hi = hi.min(h);
    }
    let d = if lo > hi { 0 } else { delta.clamp(lo, hi) };
    if d == 0 {
        return Err(EditError::NoChange);
    }
    for &id in &clips {
        let c = p.clip_mut(id).expect("clipe validado acima");
        match edge {
            Edge::End => c.duration += d,
            Edge::Start => {
                c.source_start = c.source_time_at(c.start + d);
                c.start += d;
                c.duration -= d;
            }
        }
        clamp_fades(c);
    }
    Ok(Outcome { clips, ..Outcome::default() })
}

/// Fades nunca passam da duração do clipe somados.
fn clamp_fades(c: &mut Clip) {
    c.fade_in = c.fade_in.clamp(0, c.duration);
    c.fade_out = c.fade_out.clamp(0, c.duration - c.fade_in);
}
