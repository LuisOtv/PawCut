//! Estado do editor e as operações que a interface dispara. Sem nada de egui: tudo testável.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pawcut_core::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Selection,
    Blade,
}

/// Geradores da Toolbox (aba Effects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generator {
    Text,
    SolidColor,
    /// Lower third: nome embaixo à esquerda com caixa.
    LowerThird,
    /// Legenda: texto embaixo, centralizado, com contorno.
    Caption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceMode {
    Insert,
    Overwrite,
    Append,
}

/// Qual viewer recebe In/Out e o transporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Monitor {
    Source,
    Timeline,
}

#[derive(Debug, Clone, Default)]
pub struct SourceMonitor {
    pub media: Option<MediaId>,
    pub playhead: Frame,
    pub mark_in: Option<Frame>,
    pub mark_out: Option<Frame>,
}

pub struct AppState {
    pub editor: Editor,
    pub path: Option<PathBuf>,
    pub saved_revision: u64,
    pub playhead: Frame,
    pub selection: Vec<ClipId>,
    pub tool: Tool,
    pub snapping: bool,
    pub linked_selection: bool,
    pub source: SourceMonitor,
    pub active_monitor: Monitor,
    /// In/Out da timeline (para exportar só um trecho no futuro e para o Resolve-feel).
    pub mark_in: Option<Frame>,
    pub mark_out: Option<Frame>,
    /// Mídias cujo arquivo não existe mais (Media Offline).
    pub offline: HashSet<MediaId>,
    /// Mensagem para a barra de status.
    pub status: Option<String>,
    /// Clipes copiados: faixa de origem e o clipe com `start` relativo ao primeiro.
    pub clipboard: Vec<(TrackKind, usize, Clip)>,
}

/// Duração padrão de geradores e imagens soltos na timeline: 5 segundos.
pub fn default_still_frames(project: &Project) -> Frame {
    project.settings.fps.seconds_to_frames(5.0).max(1)
}

impl AppState {
    pub fn new(project: Project) -> Self {
        let mut s = Self {
            editor: Editor::new(project),
            path: None,
            saved_revision: 0,
            playhead: 0,
            selection: Vec::new(),
            tool: Tool::Selection,
            snapping: true,
            linked_selection: true,
            source: SourceMonitor::default(),
            active_monitor: Monitor::Timeline,
            mark_in: None,
            mark_out: None,
            offline: HashSet::new(),
            status: None,
            clipboard: Vec::new(),
        };
        s.saved_revision = s.editor.revision();
        s.refresh_offline();
        s
    }

    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    pub fn is_dirty(&self) -> bool {
        self.editor.revision() != self.saved_revision
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Sem título".to_string(), |s| {
                s.to_string_lossy().into_owned()
            });
        format!("{}{name} — PawCut", if self.is_dirty() { "• " } else { "" })
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// Aplica uma edição; erros que não são "nada mudou" vão para a barra de status.
    pub fn apply(&mut self, edit: Edit) -> Option<Outcome> {
        match self.editor.apply(edit) {
            Ok(o) => Some(o),
            Err(EditError::NoChange) => None,
            Err(e) => {
                self.say(e.to_string());
                None
            }
        }
    }

    pub fn apply_coalescing(&mut self, edit: Edit, key: &str) {
        match self.editor.apply_coalescing(edit, key) {
            Ok(_) | Err(EditError::NoChange) => {}
            Err(e) => self.say(e.to_string()),
        }
    }

    /// Tira da seleção o que não existe mais (depois de desfazer, apagar...).
    pub fn prune_selection(&mut self) {
        let p = self.editor.project();
        self.selection.retain(|c| p.clip(*c).is_some());
    }

    pub fn undo(&mut self) {
        if self.editor.undo() {
            self.prune_selection();
        }
    }

    pub fn redo(&mut self) {
        if self.editor.redo() {
            self.prune_selection();
        }
    }

    /// Seleção com os parceiros ligados quando Linked Selection está ligado.
    pub fn expanded(&self, clips: &[ClipId]) -> Vec<ClipId> {
        if self.linked_selection {
            self.project().expand_linked(clips)
        } else {
            clips.to_vec()
        }
    }

    pub fn select(&mut self, clip: ClipId, additive: bool) {
        let group = self.expanded(&[clip]);
        if additive {
            if self.selection.contains(&clip) {
                self.selection.retain(|c| !group.contains(c));
            } else {
                for c in group {
                    if !self.selection.contains(&c) {
                        self.selection.push(c);
                    }
                }
            }
        } else {
            self.selection = group;
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self
            .project()
            .all_tracks()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect();
    }

    // ---------- navegação ----------

    pub fn set_playhead(&mut self, frame: Frame) {
        self.playhead = frame.max(0);
    }

    pub fn step(&mut self, delta: Frame) {
        match self.active_monitor {
            Monitor::Timeline => self.set_playhead(self.playhead + delta),
            Monitor::Source => {
                let max = self
                    .source_length()
                    .unwrap_or(Frame::MAX)
                    .saturating_sub(1)
                    .max(0);
                self.source.playhead = (self.source.playhead + delta).clamp(0, max);
            }
        }
    }

    /// Pula para o próximo (`forward`) ou o anterior ponto de corte da timeline.
    pub fn go_to_edit(&mut self, forward: bool) {
        let pts = self.project().edit_points();
        let target = if forward {
            pts.into_iter().find(|p| *p > self.playhead)
        } else {
            pts.into_iter().rev().find(|p| *p < self.playhead)
        };
        if let Some(t) = target {
            self.playhead = t;
        }
    }

    /// Encaixa `frame` no ponto de corte mais próximo (ou no cursor) dentro de `tolerance` frames.
    /// Clipes em `exclude` não servem de ímã (são os que estão sendo arrastados).
    pub fn snap(&self, frame: Frame, exclude: &[ClipId], tolerance: Frame) -> Frame {
        if !self.snapping {
            return frame;
        }
        let p = self.project();
        let mut best: Option<Frame> = None;
        let candidates = p
            .all_tracks()
            .flat_map(|t| t.clips.iter())
            .filter(|c| !exclude.contains(&c.id))
            .flat_map(|c| [c.start, c.end()])
            .chain([0, self.playhead]);
        for c in candidates {
            let d = (c - frame).abs();
            if d <= tolerance && best.is_none_or(|b| d < (b - frame).abs()) {
                best = Some(c);
            }
        }
        best.unwrap_or(frame)
    }

    // ---------- edição ----------

    pub fn blade_at_playhead(&mut self) {
        let target = if self.selection.is_empty() {
            BladeTarget::AllTracks
        } else {
            BladeTarget::Clips(self.expanded(&self.selection.clone()))
        };
        self.apply(Edit::Blade {
            at: self.playhead,
            target,
        });
    }

    /// Lâmina numa posição: corta o clipe (e os ligados, se Linked Selection).
    pub fn blade_clip(&mut self, clip: ClipId, at: Frame) {
        let clips = self.expanded(&[clip]);
        self.apply(Edit::Blade {
            at,
            target: BladeTarget::Clips(clips),
        });
    }

    pub fn delete_selection(&mut self, ripple: bool) {
        if self.selection.is_empty() {
            return;
        }
        let clips = self.expanded(&self.selection.clone());
        let edit = if ripple {
            Edit::RippleDelete { clips }
        } else {
            Edit::Delete { clips }
        };
        if self.apply(edit).is_some() {
            self.selection.clear();
        }
    }

    pub fn toggle_link(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let p = self.project();
        let any_linked = self
            .selection
            .iter()
            .filter_map(|c| p.clip(*c))
            .any(|c| c.link.is_some());
        let clips = if any_linked {
            p.expand_linked(&self.selection)
        } else {
            self.selection.clone()
        };
        if any_linked {
            self.apply(Edit::Unlink { clips });
            self.say("Áudio e vídeo separados");
        } else {
            self.apply(Edit::Link { clips });
        }
    }

    /// Garante que exista a faixa `index` do tipo `kind` e devolve o id dela.
    pub fn ensure_track(&mut self, kind: TrackKind, index: usize) -> TrackId {
        while self.project().tracks(kind).len() <= index {
            self.apply(Edit::AddTrack(kind));
        }
        self.project().tracks(kind)[index].id
    }

    /// Itens a colocar para uma mídia: vídeo em V`index` e o áudio dele em A`index` (ligados).
    fn media_items(
        &mut self,
        media: MediaId,
        track_index: usize,
        kind: TrackKind,
        from: f64,
        len: Frame,
    ) -> Vec<Placement> {
        let Some(m) = self.project().media(media).cloned() else {
            return Vec::new();
        };
        let draft = || ClipDraft {
            source: ClipSource::Media(media),
            source_start: from,
            duration: len,
        };
        let mut items = Vec::new();
        match (m.info.kind, kind) {
            (MediaKind::Video { has_audio }, TrackKind::Video) => {
                items.push(Placement {
                    track: self.ensure_track(TrackKind::Video, track_index),
                    draft: draft(),
                });
                if has_audio {
                    items.push(Placement {
                        track: self.ensure_track(TrackKind::Audio, track_index),
                        draft: draft(),
                    });
                }
            }
            (MediaKind::Image, TrackKind::Video) => items.push(Placement {
                track: self.ensure_track(TrackKind::Video, track_index),
                draft: draft(),
            }),
            (MediaKind::Video { has_audio: true } | MediaKind::Audio, TrackKind::Audio) => items
                .push(Placement {
                    track: self.ensure_track(TrackKind::Audio, track_index),
                    draft: draft(),
                }),
            _ => self.say("Esse tipo de mídia não vai nessa faixa"),
        }
        items
    }

    /// Solta uma mídia do Media Pool na faixa `track_index` do tipo `kind`, em `at` (overwrite).
    pub fn drop_media(&mut self, media: MediaId, kind: TrackKind, track_index: usize, at: Frame) {
        let Some(m) = self.project().media(media).cloned() else {
            return;
        };
        let len = m
            .info
            .duration
            .unwrap_or_else(|| default_still_frames(self.project()));
        let items = self.media_items(media, track_index, kind, 0.0, len);
        if items.is_empty() {
            return;
        }
        if let Some(o) = self.apply(Edit::Overwrite {
            at: at.max(0),
            items,
        }) {
            self.selection = o.clips;
        }
    }

    pub fn drop_generator(&mut self, generator: Generator, track_index: usize, at: Frame) {
        let (w, h) = (
            self.project().settings.width as f32,
            self.project().settings.height as f32,
        );
        let (source, placement) = match generator {
            Generator::Text => (
                ClipSource::Text(TextStyle {
                    bold: true,
                    shadow: true,
                    ..TextStyle::default()
                }),
                None,
            ),
            Generator::SolidColor => (ClipSource::SolidColor(Rgba::new(40, 40, 40, 255)), None),
            Generator::LowerThird => (
                ClipSource::Text(TextStyle {
                    text: "Nome Sobrenome\nCargo ou descrição".into(),
                    size: 44.0,
                    align: TextAlign::Left,
                    background: true,
                    ..TextStyle::default()
                }),
                Some(Transform {
                    x: -w * 0.25,
                    y: -h * 0.32,
                    ..Transform::default()
                }),
            ),
            Generator::Caption => (
                ClipSource::Text(TextStyle {
                    text: "Legenda".into(),
                    size: 52.0,
                    bold: true,
                    outline: 4.0,
                    ..TextStyle::default()
                }),
                Some(Transform {
                    y: -h * 0.38,
                    ..Transform::default()
                }),
            ),
        };
        let track = self.ensure_track(TrackKind::Video, track_index);
        let duration = default_still_frames(self.project());
        let draft = ClipDraft {
            source,
            source_start: 0.0,
            duration,
        };
        if let Some(o) = self.apply(Edit::Overwrite {
            at: at.max(0),
            items: vec![Placement { track, draft }],
        }) {
            if let (Some(t), Some(&c)) = (placement, o.clips.first()) {
                self.apply(Edit::SetClip {
                    clip: c,
                    change: ClipChange::Transform(t),
                });
            }
            self.selection = o.clips;
        }
    }

    /// Shift+[ / Shift+]: corta do início até o cursor (ou do cursor até o fim) e fecha o buraco.
    /// Vale para a seleção, ou para tudo sob o cursor se nada estiver selecionado.
    pub fn ripple_trim(&mut self, edge: Edge) {
        let at = self.playhead;
        let p = self.project();
        let clips: Vec<ClipId> = if self.selection.is_empty() {
            p.all_tracks()
                .filter(|t| !t.locked)
                .filter_map(|t| t.clip_at(at))
                .map(|c| c.id)
                .collect()
        } else {
            self.expanded(&self.selection.clone())
        };
        let cut_start = clips
            .iter()
            .filter_map(|c| p.clip(*c))
            .filter(|c| c.contains(at))
            .map(|c| c.start)
            .min();
        if self
            .apply(Edit::RippleTrimToPlayhead { clips, edge, at })
            .is_some()
            && edge == Edge::Start
            && let Some(s) = cut_start
        {
            // O que vinha depois do cursor foi puxado para o ponto do corte.
            self.playhead = s;
        }
    }

    pub fn copy_selection(&mut self) {
        let p = self.project().clone();
        let ids = self.expanded(&self.selection.clone());
        let mut items: Vec<(TrackKind, usize, Clip)> = ids
            .iter()
            .filter_map(|id| {
                let (tid, _) = p.locate(*id)?;
                let t = p.track(tid)?;
                let index = p.tracks(t.kind).iter().position(|x| x.id == tid)?;
                Some((t.kind, index, p.clip(*id)?.clone()))
            })
            .collect();
        let Some(first) = items.iter().map(|(_, _, c)| c.start).min() else {
            self.say("Selecione clipes para copiar");
            return;
        };
        for (_, _, c) in &mut items {
            c.start -= first;
        }
        let n = items.len();
        self.clipboard = items;
        self.say(format!("{n} clipe(s) copiado(s)"));
    }

    /// Cola os clipes copiados no cursor, nas mesmas faixas (criando-as se preciso).
    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let items = self.clipboard.clone();
        let mut clips = Vec::new();
        for (kind, index, c) in items {
            clips.push((self.ensure_track(kind, index), c));
        }
        let len = clips.iter().map(|(_, c)| c.end()).max().unwrap_or(0);
        if let Some(o) = self.apply(Edit::Paste {
            at: self.playhead,
            clips,
        }) {
            self.selection = o.clips;
            self.playhead += len;
        }
    }

    pub fn add_marker(&mut self) {
        let marker = Marker {
            frame: self.playhead,
            color: Rgba::new(70, 140, 230, 255),
            note: String::new(),
        };
        self.apply(Edit::SetMarker {
            frame: self.playhead,
            marker: Some(marker),
        });
    }

    pub fn go_to_marker(&mut self, forward: bool) {
        let frames = self.project().markers.iter().map(|m| m.frame);
        let target = if forward {
            frames.filter(|f| *f > self.playhead).min()
        } else {
            frames.filter(|f| *f < self.playhead).max()
        };
        if let Some(f) = target {
            self.playhead = f;
        }
    }

    /// Corte mais próximo do cursor (até 1 s de distância) com clipes encostados.
    fn nearest_cut(&self) -> Option<Frame> {
        let p = self.project();
        let window = p.settings.fps.nominal();
        p.all_tracks()
            .filter(|t| !t.locked)
            .flat_map(|t| {
                t.clips
                    .windows(2)
                    .filter(|w| w[0].end() == w[1].start)
                    .map(|w| w[1].start)
            })
            .filter(|f| (f - self.playhead).abs() <= window)
            .min_by_key(|f| (f - self.playhead).abs())
    }

    /// Ctrl+T: cross dissolve de 1 segundo no corte mais perto do cursor (vídeo e áudio).
    pub fn add_dissolve(&mut self) {
        let Some(cut) = self.nearest_cut() else {
            self.say("Nenhum corte perto do cursor");
            return;
        };
        let frames = self.project().settings.fps.nominal();
        let targets: Vec<ClipId> = self
            .project()
            .all_tracks()
            .filter(|t| !t.locked)
            .flat_map(|t| {
                t.clips
                    .windows(2)
                    .filter(|w| w[0].end() == cut && w[1].start == cut)
                    .map(|w| w[1].id)
            })
            .collect();
        for id in targets {
            self.apply_coalescing(
                Edit::SetDissolve { clip: id, frames },
                &format!("dissolve-{cut}"),
            );
        }
        self.editor.end_coalescing();
    }

    /// Ajusta o volume dos clipes de áudio selecionados para o pico ficar em −1 dBFS.
    pub fn normalize_selection(&mut self) {
        let p = self.project().clone();
        let mut done = 0;
        for id in self.expanded(&self.selection.clone()) {
            let Some((tid, _)) = p.locate(id) else {
                continue;
            };
            if p.track(tid).is_none_or(|t| t.kind != TrackKind::Audio) {
                continue;
            }
            let Some(clip) = p.clip(id) else { continue };
            if let Some(gain) = crate::analysis::clip_peak(&p, clip)
                .and_then(|peak| crate::analysis::normalize_gain(peak, -1.0))
            {
                let db = (clip.volume_db + gain).clamp(-96.0, 24.0);
                self.apply_coalescing(
                    Edit::SetClip {
                        clip: id,
                        change: ClipChange::VolumeDb(db),
                    },
                    "normalize",
                );
                done += 1;
            }
        }
        self.editor.end_coalescing();
        self.say(if done > 0 {
            format!("{done} clipe(s) normalizado(s) para −1 dB de pico")
        } else {
            "Selecione clipes de áudio".into()
        });
    }

    /// Corta os silêncios do clipe de áudio selecionado (e do vídeo ligado), fechando os buracos.
    pub fn remove_silences(&mut self, threshold_db: f32, min_len: f64) {
        let p = self.project().clone();
        let Some(clip) = self
            .expanded(&self.selection.clone())
            .into_iter()
            .filter_map(|id| p.clip(id).map(|c| (id, c)))
            .find(|(id, _)| {
                p.locate(*id)
                    .and_then(|(t, _)| p.track(t))
                    .is_some_and(|t| t.kind == TrackKind::Audio)
            })
            .map(|(_, c)| c.clone())
        else {
            self.say("Selecione um clipe com áudio");
            return;
        };
        let runs = crate::analysis::clip_silences(&p, &clip, threshold_db, min_len, 0.12);
        if runs.is_empty() {
            self.say("Nenhum silêncio encontrado");
            return;
        }
        let tracks: Vec<TrackId> = p
            .expand_linked(&[clip.id])
            .iter()
            .filter_map(|c| p.locate(*c).map(|(t, _)| t))
            .collect();
        // Do fim para o começo, para os tempos ainda valerem depois de cada corte.
        for (a, b) in runs.iter().rev() {
            self.apply_coalescing(
                Edit::RippleDeleteRange {
                    tracks: tracks.clone(),
                    start: *a,
                    end: *b,
                },
                "silences",
            );
        }
        self.editor.end_coalescing();
        self.selection.clear();
        let total: Frame = runs.iter().map(|(a, b)| b - a).sum();
        let secs = p.settings.fps.frames_to_seconds(total);
        self.say(format!(
            "{} silêncio(s) removido(s), {secs:.1} s a menos",
            runs.len()
        ));
    }

    // ---------- Source Viewer ----------

    pub fn source_length(&self) -> Option<Frame> {
        let m = self.project().media(self.source.media?)?;
        Some(
            m.info
                .duration
                .unwrap_or_else(|| default_still_frames(self.project())),
        )
    }

    pub fn open_in_source(&mut self, media: MediaId) {
        self.source = SourceMonitor {
            media: Some(media),
            ..SourceMonitor::default()
        };
        self.active_monitor = Monitor::Source;
    }

    pub fn mark(&mut self, is_in: bool) {
        match self.active_monitor {
            Monitor::Source => {
                let f = self.source.playhead;
                if is_in {
                    self.source.mark_in = Some(f)
                } else {
                    self.source.mark_out = Some(f + 1)
                }
            }
            Monitor::Timeline => {
                let f = self.playhead;
                if is_in {
                    self.mark_in = Some(f)
                } else {
                    self.mark_out = Some(f + 1)
                }
            }
        }
    }

    pub fn clear_marks(&mut self) {
        match self.active_monitor {
            Monitor::Source => (self.source.mark_in, self.source.mark_out) = (None, None),
            Monitor::Timeline => (self.mark_in, self.mark_out) = (None, None),
        }
    }

    /// Insert (F9), Overwrite (F10) ou Append (Shift+F12) do trecho In/Out do Source Viewer em V1/A1.
    pub fn place_from_source(&mut self, mode: PlaceMode) {
        let Some(media) = self.source.media else {
            self.say("Abra uma mídia no Source Viewer (duplo clique no Media Pool)");
            return;
        };
        let Some(len) = self.source_length() else {
            return;
        };
        let from = self.source.mark_in.unwrap_or(0).clamp(0, len - 1);
        let to = self.source.mark_out.unwrap_or(len).clamp(from + 1, len);
        let Some(kind) = self.project().media(media).map(|m| m.info.kind) else {
            return;
        };
        let track_kind = if kind == MediaKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Video
        };
        let items = self.media_items(media, 0, track_kind, from as f64, to - from);
        if items.is_empty() {
            return;
        }
        let at = self.playhead;
        let edit = match mode {
            PlaceMode::Insert => Edit::Insert { at, items },
            PlaceMode::Overwrite => Edit::Overwrite { at, items },
            PlaceMode::Append => Edit::Append { items },
        };
        if let Some(o) = self.apply(edit) {
            if let Some(c) = o.clips.first().and_then(|c| self.project().clip(*c)) {
                self.playhead = c.end();
            }
            self.selection = o.clips;
        }
    }

    // ---------- mídia ----------

    /// Lê e adiciona arquivos ao Media Pool; devolve quantos entraram.
    pub fn import(&mut self, paths: &[PathBuf]) -> usize {
        let fps = self.project().settings.fps;
        let mut ok = 0;
        let mut errors = Vec::new();
        for path in paths {
            if self.project().media.iter().any(|m| m.info.path == *path) {
                continue;
            }
            match pawcut_media::probe(path, fps) {
                Ok(info) => {
                    self.apply(Edit::AddMedia(info));
                    ok += 1;
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        if let Some(e) = errors.first() {
            self.say(if errors.len() > 1 {
                format!("{e} (e mais {} erros)", errors.len() - 1)
            } else {
                e.clone()
            });
        } else if ok > 0 {
            self.say(format!("{ok} arquivo(s) importado(s)"));
        }
        ok
    }

    pub fn relink(&mut self, media: MediaId, path: &Path) {
        match pawcut_media::probe(path, self.project().settings.fps) {
            Ok(info) => {
                self.apply(Edit::RelinkMedia { media, info });
                self.refresh_offline();
                self.say("Mídia reconectada");
            }
            Err(e) => self.say(e.to_string()),
        }
    }

    pub fn refresh_offline(&mut self) {
        self.offline = self
            .project()
            .media
            .iter()
            .filter(|m| !m.info.path.exists())
            .map(|m| m.id)
            .collect();
    }

    // ---------- arquivo ----------

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved_revision = self.editor.revision();
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), ProjectFileError> {
        save_project(self.project(), path)?;
        self.mark_saved(path.to_path_buf());
        Ok(())
    }

    pub fn open(path: &Path) -> Result<Self, ProjectFileError> {
        let mut s = Self::new(load_project(path)?);
        s.mark_saved(path.to_path_buf());
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new(Project::new(ProjectSettings::default()))
    }

    fn add_video(s: &mut AppState, frames: Frame, has_audio: bool) -> MediaId {
        let info = MediaInfo {
            path: "/videos/a.mp4".into(),
            kind: MediaKind::Video { has_audio },
            duration: Some(frames),
            width: 1920,
            height: 1080,
        };
        s.apply(Edit::AddMedia(info)).unwrap().media.unwrap()
    }

    fn spans(s: &AppState, kind: TrackKind, i: usize) -> Vec<(Frame, Frame)> {
        s.project().tracks(kind)[i]
            .clips
            .iter()
            .map(|c| (c.start, c.end()))
            .collect()
    }

    #[test]
    fn dropping_a_video_places_linked_video_and_audio() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 10);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(10, 100)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(10, 100)]);
        assert_eq!(s.selection.len(), 2);
    }

    #[test]
    fn dropping_on_a_higher_track_creates_it() {
        let mut s = state();
        s.drop_generator(Generator::Text, 2, 0);
        assert_eq!(s.project().video_tracks.len(), 3);
        assert_eq!(spans(&s, TrackKind::Video, 2), vec![(0, 150)]);
        assert!(s.is_dirty());
    }

    #[test]
    fn blade_at_playhead_cuts_selection_or_everything() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.selection.clear();
        s.set_playhead(30);
        s.blade_at_playhead();
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30), (30, 90)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 30), (30, 90)]);
        // com seleção e linked ligado, corta o par selecionado
        let v = s.project().video_tracks[0].clips[1].id;
        s.select(v, false);
        s.set_playhead(60);
        s.blade_at_playhead();
        assert_eq!(
            spans(&s, TrackKind::Audio, 0),
            vec![(0, 30), (30, 60), (60, 90)]
        );
    }

    #[test]
    fn ripple_delete_of_selection_closes_the_gap() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.set_playhead(30);
        s.selection.clear();
        s.blade_at_playhead();
        let first = s.project().video_tracks[0].clips[0].id;
        s.select(first, false);
        s.delete_selection(true);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 60)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 60)]);
        assert!(s.selection.is_empty());
    }

    #[test]
    fn linked_selection_off_selects_only_one_side() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        let v = s.project().video_tracks[0].clips[0].id;
        s.linked_selection = false;
        s.select(v, false);
        assert_eq!(s.selection, vec![v]);
        s.linked_selection = true;
        s.select(v, false);
        assert_eq!(s.selection.len(), 2);
        s.select(v, true);
        assert!(
            s.selection.is_empty(),
            "clique com shift num selecionado tira o grupo"
        );
    }

    #[test]
    fn snap_pulls_to_nearby_edges_and_playhead() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        let g = s.selection[0];
        s.set_playhead(200);
        assert_eq!(s.snap(153, &[], 5), 150);
        assert_eq!(s.snap(197, &[], 5), 200);
        assert_eq!(s.snap(160, &[], 5), 160);
        assert_eq!(s.snap(148, &[g], 5), 148, "o próprio clipe não é ímã");
        s.snapping = false;
        assert_eq!(s.snap(153, &[], 5), 153);
    }

    #[test]
    fn navigation_between_edits() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 30);
        s.set_playhead(0);
        s.go_to_edit(true);
        assert_eq!(s.playhead, 30);
        s.go_to_edit(true);
        assert_eq!(s.playhead, 180);
        s.go_to_edit(false);
        assert_eq!(s.playhead, 30);
        s.step(-100);
        assert_eq!(s.playhead, 0);
    }

    #[test]
    fn source_monitor_in_out_overwrite_and_insert() {
        let mut s = state();
        let m = add_video(&mut s, 100, true);
        s.open_in_source(m);
        s.source.playhead = 10;
        s.mark(true);
        s.source.playhead = 39;
        s.mark(false);
        s.place_from_source(PlaceMode::Overwrite);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 30)]);
        assert_eq!(s.project().video_tracks[0].clips[0].source_start, 10.0);
        assert_eq!(s.playhead, 30, "o cursor vai para o fim do que entrou");
        s.set_playhead(15);
        s.place_from_source(PlaceMode::Insert);
        assert_eq!(
            spans(&s, TrackKind::Video, 0),
            vec![(0, 15), (15, 45), (45, 60)]
        );
        s.place_from_source(PlaceMode::Append);
        assert_eq!(spans(&s, TrackKind::Video, 0).last(), Some(&(60, 90)));
    }

    #[test]
    fn toggle_link_separates_and_rejoins() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.toggle_link();
        assert!(s.project().video_tracks[0].clips[0].link.is_none());
        s.toggle_link();
        let v = &s.project().video_tracks[0].clips[0];
        assert!(v.link.is_some());
    }

    #[test]
    fn undo_drops_vanished_clips_from_selection() {
        let mut s = state();
        s.drop_generator(Generator::Text, 0, 0);
        assert_eq!(s.selection.len(), 1);
        s.undo();
        assert!(s.selection.is_empty());
        s.redo();
    }

    #[test]
    fn save_and_open_track_dirty_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.pawcut");
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        assert!(s.title().starts_with("• Sem título"));
        s.save_to(&path).unwrap();
        assert!(!s.is_dirty());
        assert_eq!(s.title(), "p — PawCut");
        let opened = AppState::open(&path).unwrap();
        assert_eq!(opened.project(), s.project());
        assert!(!opened.is_dirty());
    }

    #[test]
    fn missing_media_is_marked_offline() {
        let mut s = state();
        let m = add_video(&mut s, 90, false);
        s.refresh_offline();
        assert!(s.offline.contains(&m));
    }

    #[test]
    fn ripple_trim_start_pulls_the_rest_and_moves_playhead_to_the_cut() {
        let mut s = state();
        let m = add_video(&mut s, 300, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.selection.clear();
        s.set_playhead(40);
        s.ripple_trim(Edge::Start);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 260)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 260)]);
        assert_eq!(s.playhead, 0);
        s.set_playhead(100);
        s.ripple_trim(Edge::End);
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 100)]);
    }

    #[test]
    fn copy_paste_keeps_tracks_and_link() {
        let mut s = state();
        let m = add_video(&mut s, 90, true);
        s.drop_media(m, TrackKind::Video, 0, 0);
        s.copy_selection();
        s.set_playhead(200);
        s.paste();
        assert_eq!(spans(&s, TrackKind::Video, 0), vec![(0, 90), (200, 290)]);
        assert_eq!(spans(&s, TrackKind::Audio, 0), vec![(0, 90), (200, 290)]);
        let p = s.project();
        assert_eq!(
            p.video_tracks[0].clips[1].link,
            p.audio_tracks[0].clips[1].link
        );
        assert_eq!(s.playhead, 290);
    }

    #[test]
    fn markers_and_dissolve_from_the_keyboard() {
        let mut s = state();
        s.drop_generator(Generator::SolidColor, 0, 0);
        s.drop_generator(Generator::SolidColor, 0, 150);
        s.set_playhead(20);
        s.add_marker();
        s.set_playhead(100);
        s.add_marker();
        s.go_to_marker(false);
        assert_eq!(s.playhead, 20);
        s.go_to_marker(true);
        assert_eq!(s.playhead, 100);
        s.set_playhead(140);
        s.add_dissolve();
        assert_eq!(s.project().video_tracks[0].clips[1].dissolve_in, 30);
    }

    #[test]
    fn remove_silences_cuts_the_pause_out() {
        let dir = tempfile::tempdir().unwrap();
        let wav = pawcut_testkit::speech_with_pause(dir.path());
        let mut s = state();
        assert_eq!(s.import(&[wav]), 1);
        let m = s.project().media[0].id;
        s.drop_media(m, TrackKind::Audio, 0, 0);
        let before = s.project().duration();
        s.remove_silences(-40.0, 0.5);
        let after = s.project().duration();
        let removed = s.project().settings.fps.frames_to_seconds(before - after);
        assert!((removed - 0.76).abs() < 0.1, "removeu {removed:.2} s");
        assert_eq!(s.project().audio_tracks[0].clips.len(), 2);
        s.undo();
        assert_eq!(s.project().duration(), before, "um desfazer volta tudo");
    }
}
