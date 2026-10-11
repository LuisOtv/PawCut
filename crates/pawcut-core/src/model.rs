use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::{ClipId, LinkId, MediaId, TrackId};
use crate::time::{Frame, FrameRate};

/// Versão do formato `.pawcut` que esta build lê e grava.
pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub sample_rate: u32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: FrameRate::new(30, 1), sample_rate: 48_000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video { has_audio: bool },
    Audio,
    Image,
}

/// Metadados de um arquivo importado. `duration` em frames do projeto; `None` para imagens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub duration: Option<Frame>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: MediaId,
    pub info: MediaInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipSource {
    Media(MediaId),
    SolidColor(Rgba),
    Text(TextStyle),
}

impl ClipSource {
    pub fn is_generator(&self) -> bool {
        !matches!(self, ClipSource::Media(_))
    }
}

/// Transform fixo: deslocamento em pixels a partir do centro, escala e opacidade (0..=1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub opacity: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, scale: 1.0, opacity: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source: ClipSource,
    /// Início na timeline.
    pub start: Frame,
    /// Duração na timeline.
    pub duration: Frame,
    /// Ponto da fonte (em frames do projeto, a 1x) mostrado em `start`.
    pub source_start: f64,
    pub speed: f64,
    pub volume_db: f32,
    pub fade_in: Frame,
    pub fade_out: Frame,
    pub transform: Transform,
    pub link: Option<LinkId>,
}

impl Clip {
    pub fn end(&self) -> Frame {
        self.start + self.duration
    }

    pub fn contains(&self, frame: Frame) -> bool {
        frame >= self.start && frame < self.end()
    }

    /// Tempo da fonte mostrado no frame `frame` da timeline.
    pub fn source_time_at(&self, frame: Frame) -> f64 {
        self.source_start + (frame - self.start) as f64 * self.speed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub locked: bool,
    /// Só vídeo: faixa desligada não aparece no render.
    pub enabled: bool,
    /// Só áudio.
    pub muted: bool,
    pub solo: bool,
    /// Ordenados por `start`, sem sobreposição.
    pub clips: Vec<Clip>,
}

impl Track {
    pub(crate) fn new(id: TrackId, kind: TrackKind) -> Self {
        Self { id, kind, locked: false, enabled: true, muted: false, solo: false, clips: Vec::new() }
    }

    pub fn clip_at(&self, frame: Frame) -> Option<&Clip> {
        self.clips.iter().find(|c| c.contains(frame))
    }

    pub fn end(&self) -> Frame {
        self.clips.last().map_or(0, Clip::end)
    }
}

/// Gerador de IDs únicos do projeto (salvo junto, para não repetir IDs depois de reabrir).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct IdGen {
    last: u64,
}

impl IdGen {
    pub(crate) fn fresh(&mut self) -> u64 {
        self.last += 1;
        self.last
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub settings: ProjectSettings,
    pub media: Vec<MediaRef>,
    /// Índice 0 = V1 (a de baixo). Faixas de cima cobrem as de baixo.
    pub video_tracks: Vec<Track>,
    pub audio_tracks: Vec<Track>,
    pub(crate) ids: IdGen,
}

impl Project {
    /// Projeto vazio com V1 e A1, como uma timeline nova no Resolve.
    pub fn new(settings: ProjectSettings) -> Self {
        let mut p = Self {
            version: PROJECT_VERSION,
            settings,
            media: Vec::new(),
            video_tracks: Vec::new(),
            audio_tracks: Vec::new(),
            ids: IdGen::default(),
        };
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Audio);
        p
    }

    pub(crate) fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = TrackId(self.ids.fresh());
        self.tracks_mut(kind).push(Track::new(id, kind));
        id
    }

    pub fn tracks(&self, kind: TrackKind) -> &[Track] {
        match kind {
            TrackKind::Video => &self.video_tracks,
            TrackKind::Audio => &self.audio_tracks,
        }
    }

    pub(crate) fn tracks_mut(&mut self, kind: TrackKind) -> &mut Vec<Track> {
        match kind {
            TrackKind::Video => &mut self.video_tracks,
            TrackKind::Audio => &mut self.audio_tracks,
        }
    }

    pub fn all_tracks(&self) -> impl Iterator<Item = &Track> {
        self.video_tracks.iter().chain(self.audio_tracks.iter())
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.all_tracks().find(|t| t.id == id)
    }

    pub(crate) fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.video_tracks.iter_mut().chain(self.audio_tracks.iter_mut()).find(|t| t.id == id)
    }

    /// Faixa mutável e o gerador de IDs ao mesmo tempo (para cortes que criam clipes).
    pub(crate) fn track_and_ids(&mut self, id: TrackId) -> Option<(&mut Track, &mut IdGen)> {
        let track = self.video_tracks.iter_mut().chain(self.audio_tracks.iter_mut()).find(|t| t.id == id)?;
        Some((track, &mut self.ids))
    }

    /// Rótulo no estilo Resolve: "V1", "A2".
    pub fn track_label(&self, id: TrackId) -> Option<String> {
        for (prefix, tracks) in [("V", &self.video_tracks), ("A", &self.audio_tracks)] {
            if let Some(i) = tracks.iter().position(|t| t.id == id) {
                return Some(format!("{prefix}{}", i + 1));
            }
        }
        None
    }

    pub fn media(&self, id: MediaId) -> Option<&MediaRef> {
        self.media.iter().find(|m| m.id == id)
    }

    /// Faixa e índice do clipe dentro dela.
    pub fn locate(&self, clip: ClipId) -> Option<(TrackId, usize)> {
        self.all_tracks().find_map(|t| t.clips.iter().position(|c| c.id == clip).map(|i| (t.id, i)))
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.all_tracks().flat_map(|t| t.clips.iter()).find(|c| c.id == id)
    }

    pub(crate) fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.video_tracks
            .iter_mut()
            .chain(self.audio_tracks.iter_mut())
            .flat_map(|t| t.clips.iter_mut())
            .find(|c| c.id == id)
    }

    /// Fim do último clipe da timeline.
    pub fn duration(&self) -> Frame {
        self.all_tracks().map(Track::end).max().unwrap_or(0)
    }

    /// Acrescenta os parceiros ligados de cada clipe (Linked Selection). Sem repetir, na ordem.
    pub fn expand_linked(&self, clips: &[ClipId]) -> Vec<ClipId> {
        let mut out: Vec<ClipId> = Vec::new();
        for &id in clips {
            let Some(clip) = self.clip(id) else { continue };
            let group: Vec<ClipId> = match clip.link {
                Some(link) => self
                    .all_tracks()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| c.link == Some(link))
                    .map(|c| c.id)
                    .collect(),
                None => vec![id],
            };
            for g in group {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        out
    }

    /// Todos os inícios e fins de clipe (e o 0), ordenados: usados no snapping e em ↑/↓.
    pub fn edit_points(&self) -> Vec<Frame> {
        let mut pts: Vec<Frame> =
            self.all_tracks().flat_map(|t| t.clips.iter()).flat_map(|c| [c.start, c.end()]).collect();
        pts.push(0);
        pts.sort_unstable();
        pts.dedup();
        pts
    }

    /// Quanto a fonte tem, em frames do projeto a 1x. `None` = sem limite (imagens e geradores).
    pub fn source_length(&self, source: &ClipSource) -> Option<Frame> {
        match source {
            ClipSource::Media(id) => self.media(*id).and_then(|m| m.info.duration),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u64, start: Frame, duration: Frame, link: Option<u64>) -> Clip {
        Clip {
            id: ClipId(id),
            source: ClipSource::SolidColor(Rgba::new(0, 0, 0, 255)),
            start,
            duration,
            source_start: 0.0,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link: link.map(LinkId),
        }
    }

    #[test]
    fn new_project_has_v1_and_a1() {
        let p = Project::new(ProjectSettings::default());
        assert_eq!(p.video_tracks.len(), 1);
        assert_eq!(p.audio_tracks.len(), 1);
        assert_eq!(p.track_label(p.video_tracks[0].id).as_deref(), Some("V1"));
        assert_eq!(p.track_label(p.audio_tracks[0].id).as_deref(), Some("A1"));
        assert_ne!(p.video_tracks[0].id, p.audio_tracks[0].id);
        assert_eq!(p.duration(), 0);
        assert_eq!(p.version, PROJECT_VERSION);
    }

    #[test]
    fn clip_maps_timeline_to_source_time_with_speed() {
        let mut c = clip(1, 100, 50, None);
        c.source_start = 10.0;
        c.speed = 2.0;
        assert_eq!(c.end(), 150);
        assert!(c.contains(100));
        assert!(!c.contains(150));
        assert_eq!(c.source_time_at(100), 10.0);
        assert_eq!(c.source_time_at(110), 30.0);
    }

    #[test]
    fn queries_find_clips_links_and_edit_points() {
        let mut p = Project::new(ProjectSettings::default());
        p.video_tracks[0].clips = vec![clip(10, 0, 30, Some(7)), clip(11, 40, 20, None)];
        p.audio_tracks[0].clips = vec![clip(12, 0, 30, Some(7))];
        assert_eq!(p.duration(), 60);
        assert_eq!(p.locate(ClipId(11)), Some((p.video_tracks[0].id, 1)));
        assert_eq!(p.expand_linked(&[ClipId(10)]), vec![ClipId(10), ClipId(12)]);
        assert_eq!(p.expand_linked(&[ClipId(11)]), vec![ClipId(11)]);
        assert_eq!(p.edit_points(), vec![0, 30, 40, 60]);
        assert_eq!(p.video_tracks[0].clip_at(45).map(|c| c.id), Some(ClipId(11)));
        assert!(p.video_tracks[0].clip_at(35).is_none());
    }

    #[test]
    fn source_length_is_none_for_generators_and_images() {
        let mut p = Project::new(ProjectSettings::default());
        p.media.push(MediaRef {
            id: MediaId(50),
            info: MediaInfo {
                path: "a.mp4".into(),
                kind: MediaKind::Video { has_audio: true },
                duration: Some(300),
                width: 1920,
                height: 1080,
            },
        });
        p.media.push(MediaRef {
            id: MediaId(51),
            info: MediaInfo {
                path: "logo.png".into(),
                kind: MediaKind::Image,
                duration: None,
                width: 200,
                height: 200,
            },
        });
        assert_eq!(p.source_length(&ClipSource::Media(MediaId(50))), Some(300));
        assert_eq!(p.source_length(&ClipSource::Media(MediaId(51))), None);
        assert_eq!(p.source_length(&ClipSource::SolidColor(Rgba::new(255, 0, 0, 255))), None);
    }
}
