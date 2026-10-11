# PawCut Core: plano de implementação (plano 1 de 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Criar o crate `pawcut-core`, com o modelo do projeto, todas as edições da timeline, desfazer/refazer e salvar/abrir `.pawcut`, 100% testado e sem depender de mídia nem de interface.

**Architecture:** Workspace Cargo em `/home/luis/Documents/PawCut` com um crate por enquanto (`crates/pawcut-core`). As edições são valores de um enum (`Edit`) aplicados por um `Editor`. O desfazer guarda **retratos** do `Project`, que é pequeno (só metadados) e barato de clonar. Assim desfazer/refazer não precisa de lógica inversa por operação. Operações de baixo nível sobre uma faixa (`clear_range`, `ripple_open`, `split_clip`) ficam isoladas em `track_ops.rs`.

**Tech Stack:** Rust (edition 2024), `serde` + `serde_json`, `thiserror`, `tempfile` (só nos testes).

**Spec:** `docs/superpowers/specs/2026-10-11-pawcut-design.md`

**Roteiro dos planos:**
1. **Este plano:** `pawcut-core`.
2. `pawcut-media` + `pawcut-render` + `pawcut-export`: um projeto vira MP4 por um binário de teste.
3. `pawcut-app` básico: egui_dock, Media Pool, Timeline, Timeline Viewer, reprodução.
4. `pawcut-app` completo: Source Viewer, Inspector, geradores, keymap editável, Deliver, autosave, Relink.

## Global Constraints

- O Rust precisa estar instalado (`cargo --version` funcionando, Rust ≥ 1.85 por causa da edition 2024). Se não estiver, **parar** e pedir ao Luis para rodar `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.
- Workspace com `resolver = "3"` e `edition = "2024"`. Não usar o identificador `gen`, que é palavra reservada na edition 2024.
- O tempo da timeline é sempre `Frame = i64` (frames do projeto). Só `source_start` e `speed` são `f64`.
- Padrões do projeto: 1920×1080, 30/1 fps, 48000 Hz.
- O índice 0 de `video_tracks` é V1 (a faixa de baixo). Os rótulos são calculados ("V1", "A2"), nunca guardados.
- `PROJECT_VERSION = 1`. Arquivo `.pawcut` = JSON do `Project`.
- Mensagens de erro para o usuário em **português**. Identificadores de código em inglês.
- Nada de `unwrap`/`expect` em caminho de I/O fora dos testes. `expect` só é permitido para invariantes já validadas logo antes, com mensagem.
- A máquina não tem identidade git global. Commits usam `git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit ...`, a menos que o Luis já tenha configurado. As mensagens terminam com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- No fim: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` e `cargo fmt --check` passando.

## Review Focus

1. **Arrastar ou aparar um clipe para antes do frame 0**: deve parar em 0, sem start negativo. Testado em `move_never_goes_before_zero` (Task 6) e `trim_start_cannot_go_before_source_start_or_frame_zero` (Task 7).
2. **Mudar a velocidade para um valor que deixaria o clipe maior que o espaço livre**: o clipe é truncado no início do próximo, nunca se sobrepõe. Testado em `speed_2x_halves_duration_and_0_5x_doubles_until_next_clip` (Task 9).
3. **Abrir um `.pawcut` de versão mais nova, corrompido ou inexistente**: erro claro, nunca panic. Testes na Task 10.
4. **Cortar exatamente na borda de um clipe**: não cria clipe de 0 frames e devolve `NoChange`. Testado em `blade_on_edge_does_nothing` (Task 5).
5. **A interface mandar o mesmo clipe duas vezes numa seleção** (por exemplo, a expansão do linked duplicando): sem panic, cada clipe é processado uma vez. Testado em `duplicate_ids_in_selection_are_harmless` (Task 5).

---

## Estrutura de arquivos

```
Cargo.toml                         # workspace
crates/pawcut-core/
  Cargo.toml
  src/lib.rs                       # re-exports públicos
  src/time.rs                      # Frame, FrameRate, timecode
  src/ids.rs                       # MediaId, TrackId, ClipId, LinkId
  src/model.rs                     # Project, Track, Clip, MediaRef... + consultas
  src/track_ops.rs                 # split/clear_range/ripple_open/insert_sorted/relink (pub(crate))
  src/edit.rs                      # Edit, EditError, Outcome, apply()
  src/editor.rs                    # Editor (aplica + desfazer/refazer)
  src/file.rs                      # to_json/from_json/save_project/load_project
  tests/common/mod.rs              # helpers dos testes de integração
  tests/placement.rs  tests/history.rs  tests/cutting.rs  tests/moving.rs
  tests/trimming.rs   tests/insert.rs   tests/properties.rs tests/file.rs
```

---

### Task 1: Workspace e tempo

**Files:**
- Create: `Cargo.toml`, `crates/pawcut-core/Cargo.toml`, `crates/pawcut-core/src/lib.rs`, `crates/pawcut-core/src/time.rs`

**Interfaces:**
- Produces: `pub type Frame = i64`; `FrameRate { num: u32, den: u32 }` com `const fn new`, `as_f64`, `nominal`, `frames_to_seconds`, `seconds_to_frames`, `frame_to_sample(frame, sample_rate: u32) -> i64`; `format_timecode(Frame, FrameRate) -> String`.

- [ ] **Step 0: Conferir o Rust**

Run: `cargo --version && rustc --version`
Expected: as versões aparecem (≥ 1.85). Se der "command not found", parar e pedir ao Luis para instalar o Rust (ver Global Constraints).

- [ ] **Step 1: Criar o workspace e o crate**

`Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/pawcut-core"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tempfile = "3"
```

`crates/pawcut-core/Cargo.toml`:
```toml
[package]
name = "pawcut-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/pawcut-core/src/lib.rs`:
```rust
//! Núcleo do PawCut: modelo do projeto, edições da timeline e desfazer/refazer.
//! Não depende de mídia nem de interface.

mod time;

pub use time::{Frame, FrameRate, format_timecode};
```

- [ ] **Step 2: Escrever os testes de tempo (falhando)**

`crates/pawcut-core/src/time.rs`:
```rust
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;

    const FPS30: FrameRate = FrameRate::new(30, 1);
    const NTSC: FrameRate = FrameRate::new(30000, 1001);

    #[test]
    fn timecode_formats_hours_minutes_seconds_frames() {
        assert_eq!(format_timecode(0, FPS30), "00:00:00:00");
        assert_eq!(format_timecode(29, FPS30), "00:00:00:29");
        assert_eq!(format_timecode(30, FPS30), "00:00:01:00");
        assert_eq!(format_timecode(30 * 3661 + 5, FPS30), "01:01:01:05");
    }

    #[test]
    fn timecode_uses_nominal_rate_for_fractional_fps() {
        assert_eq!(format_timecode(30, NTSC), "00:00:01:00");
    }

    #[test]
    fn negative_timecode_has_sign() {
        assert_eq!(format_timecode(-31, FPS30), "-00:00:01:01");
    }

    #[test]
    fn converts_between_frames_seconds_and_samples() {
        assert_eq!(FPS30.seconds_to_frames(2.0), 60);
        assert!((FPS30.frames_to_seconds(45) - 1.5).abs() < 1e-9);
        assert_eq!(FPS30.frame_to_sample(30, 48_000), 48_000);
        assert_eq!(NTSC.frame_to_sample(30, 48_000), 48_048);
    }
}
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p pawcut-core`
Expected: erro de compilação (`FrameRate`/`format_timecode` não existem).

- [ ] **Step 4: Implementar** (acima do `#[cfg(test)]` em `time.rs`)

```rust
/// Posição ou duração na timeline, em frames do projeto.
pub type Frame = i64;

/// Taxa de quadros racional (ex.: 30/1, 30000/1001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Frames por segundo arredondado, usado no timecode (non-drop-frame).
    pub fn nominal(self) -> i64 {
        (self.as_f64().round() as i64).max(1)
    }

    pub fn frames_to_seconds(self, frames: Frame) -> f64 {
        frames as f64 / self.as_f64()
    }

    pub fn seconds_to_frames(self, seconds: f64) -> Frame {
        (seconds * self.as_f64()).round() as Frame
    }

    /// Primeira amostra de áudio que pertence ao frame `frame`.
    pub fn frame_to_sample(self, frame: Frame, sample_rate: u32) -> i64 {
        (i128::from(frame) * i128::from(sample_rate) * i128::from(self.den) / i128::from(self.num)) as i64
    }
}

/// Formata como `HH:MM:SS:FF`, igual à régua do Resolve.
pub fn format_timecode(frame: Frame, rate: FrameRate) -> String {
    let fps = rate.nominal();
    let sign = if frame < 0 { "-" } else { "" };
    let f = frame.abs();
    let secs = f / fps;
    format!(
        "{sign}{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        f % fps
    )
}
```

- [ ] **Step 5: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: 4 testes passando.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): workspace and frame/timecode math

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: IDs e modelo do projeto

**Files:**
- Create: `crates/pawcut-core/src/ids.rs`, `crates/pawcut-core/src/model.rs`
- Modify: `crates/pawcut-core/src/lib.rs`

**Interfaces:**
- Consumes: `Frame`, `FrameRate` (Task 1).
- Produces:
  - IDs `MediaId(pub u64)`, `TrackId(pub u64)`, `ClipId(pub u64)`, `LinkId(pub u64)` (Copy, Eq, Hash, Ord, serde transparent).
  - `PROJECT_VERSION: u32 = 1`, `Rgba { r, g, b, a: u8 }` + `Rgba::new`, `ProjectSettings { width, height, fps, sample_rate }` (Default 1920×1080 a 30/1, 48000).
  - `MediaKind { Video { has_audio: bool }, Audio, Image }`, `MediaInfo { path: PathBuf, kind, duration: Option<Frame>, width: u32, height: u32 }`, `MediaRef { id: MediaId, info: MediaInfo }`.
  - `TextStyle { text: String, font: String, size: f32, color: Rgba }`, `ClipSource { Media(MediaId), SolidColor(Rgba), Text(TextStyle) }` + `is_generator()`.
  - `Transform { x, y, scale, opacity: f32 }` (Default 0,0,1,1).
  - `Clip { id, source, start: Frame, duration: Frame, source_start: f64, speed: f64, volume_db: f32, fade_in: Frame, fade_out: Frame, transform, link: Option<LinkId> }` + `end()`, `contains(Frame)`, `source_time_at(Frame) -> f64`.
  - `TrackKind { Video, Audio }`, `Track { id, kind, locked, enabled, muted, solo, clips: Vec<Clip> }` + `clip_at(Frame) -> Option<&Clip>`, `end()`.
  - `Project { version, settings, media, video_tracks, audio_tracks, pub(crate) ids: IdGen }` com `new(settings)`, `tracks(kind) -> &[Track]`, `all_tracks()`, `track(id)`, `track_label(id) -> Option<String>`, `media(id)`, `locate(ClipId) -> Option<(TrackId, usize)>`, `clip(id)`, `duration()`, `expand_linked(&[ClipId]) -> Vec<ClipId>`, `edit_points() -> Vec<Frame>`, `source_length(&ClipSource) -> Option<Frame>`.
  - Crate-private: `IdGen::fresh() -> u64`, `Project::add_track(kind) -> TrackId`, `tracks_mut`, `track_mut`, `clip_mut`, `track_and_ids(TrackId) -> Option<(&mut Track, &mut IdGen)>`, `Track::new(id, kind)`.

- [ ] **Step 1: Criar `ids.rs`**

```rust
use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(/// Arquivo importado no Media Pool.
    MediaId);
id_type!(/// Faixa de vídeo ou áudio.
    TrackId);
id_type!(/// Clipe na timeline.
    ClipId);
id_type!(/// Grupo de clipes ligados (ex.: vídeo + o seu áudio).
    LinkId);
```

- [ ] **Step 2: Escrever os testes do modelo (falhando)**

`crates/pawcut-core/src/model.rs`, por enquanto só com os testes:
```rust
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
```

Em `lib.rs`, adicionar `mod ids;` e `mod model;`.

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p pawcut-core`
Expected: erro de compilação (tipos do modelo não existem).

- [ ] **Step 4: Implementar o modelo** (acima dos testes em `model.rs`)

```rust
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
```

Em `lib.rs`, acrescentar os re-exports:
```rust
pub use ids::{ClipId, LinkId, MediaId, TrackId};
pub use model::{
    Clip, ClipSource, MediaInfo, MediaKind, MediaRef, PROJECT_VERSION, Project, ProjectSettings, Rgba,
    TextStyle, Track, TrackKind, Transform,
};
```

- [ ] **Step 5: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: 8 testes passando (4 de tempo + 4 do modelo). Avisos de `dead_code` em itens `pub(crate)` ainda não usados são esperados.

- [ ] **Step 6: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): project model and queries

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Operações de faixa (`track_ops`)

**Files:**
- Create: `crates/pawcut-core/src/track_ops.rs`
- Modify: `crates/pawcut-core/src/lib.rs` (adicionar `mod track_ops;`)

**Interfaces:**
- Consumes: `Clip`, `Track`, `IdGen`, `ClipId`, `LinkId`, `Frame`.
- Produces (todas `pub(crate)`):
  - `split_clip(&Clip, at: Frame, right_id: ClipId) -> (Clip, Clip)`, com a pré-condição `start < at < end`. A esquerda perde o fade_out e a direita perde o fade_in.
  - `insert_sorted(&mut Track, Clip)`.
  - `clear_range(&mut Track, start, end, &mut IdGen)`: abre espaço vazio em `[start, end)`.
  - `ripple_open(&mut Track, at, len, &mut HashMap<LinkId, LinkId>, &mut IdGen)`: corta o clipe que atravessa `at` e empurra para a direita tudo que começa em `>= at`.
  - `relink(Option<LinkId>, &mut HashMap<LinkId, LinkId>, &mut IdGen) -> Option<LinkId>`.

- [ ] **Step 1: Escrever os testes (falhando)**

`crates/pawcut-core/src/track_ops.rs`:
```rust
use std::collections::HashMap;

use crate::ids::{ClipId, LinkId};
use crate::model::{Clip, IdGen, Track};
use crate::time::Frame;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TrackId;
    use crate::model::{ClipSource, Rgba, TrackKind, Transform};

    /// Clipe cuja fonte coincide com a timeline (source_start = start), para facilitar as contas.
    fn clip(id: u64, start: Frame, duration: Frame) -> Clip {
        Clip {
            id: ClipId(id),
            source: ClipSource::SolidColor(Rgba::new(0, 0, 0, 255)),
            start,
            duration,
            source_start: start as f64,
            speed: 1.0,
            volume_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            transform: Transform::default(),
            link: None,
        }
    }

    fn track(clips: Vec<Clip>) -> Track {
        let mut t = Track::new(TrackId(1), TrackKind::Video);
        t.clips = clips;
        t
    }

    fn spans(t: &Track) -> Vec<(Frame, Frame)> {
        t.clips.iter().map(|c| (c.start, c.end())).collect()
    }

    #[test]
    fn split_keeps_source_continuity_and_moves_fades() {
        let mut c = clip(1, 10, 20);
        c.source_start = 5.0;
        c.speed = 2.0;
        c.fade_in = 4;
        c.fade_out = 6;
        let (l, r) = split_clip(&c, 15, ClipId(2));
        assert_eq!((l.id, l.start, l.duration, l.fade_in, l.fade_out), (ClipId(1), 10, 5, 4, 0));
        assert_eq!((r.id, r.start, r.duration, r.fade_in, r.fade_out), (ClipId(2), 15, 15, 0, 6));
        assert_eq!(r.source_start, 15.0);
    }

    #[test]
    fn clear_range_handles_every_overlap_shape() {
        // antes, cruza o início, dentro, cruza o fim, depois
        let mut t = track(vec![clip(1, 0, 10), clip(2, 10, 20), clip(3, 35, 10), clip(4, 50, 20), clip(5, 80, 10)]);
        let mut ids = IdGen::default();
        clear_range(&mut t, 20, 60, &mut ids);
        assert_eq!(spans(&t), vec![(0, 10), (10, 20), (60, 70), (80, 90)]);
        assert_eq!(t.clips[2].id, ClipId(4));
        assert_eq!(t.clips[2].source_start, 60.0);
    }

    #[test]
    fn clear_range_inside_one_clip_splits_it_in_two() {
        let mut c = clip(1, 0, 100);
        c.link = Some(LinkId(9));
        let mut t = track(vec![c]);
        let mut ids = IdGen::default();
        clear_range(&mut t, 40, 60, &mut ids);
        assert_eq!(spans(&t), vec![(0, 40), (60, 100)]);
        assert_eq!(t.clips[0].id, ClipId(1));
        assert_ne!(t.clips[1].id, ClipId(1));
        assert_eq!(t.clips[1].source_start, 60.0);
        assert_eq!(t.clips[1].link, None);
    }

    #[test]
    fn ripple_open_splits_and_pushes_right() {
        let mut a = clip(1, 0, 20);
        a.link = Some(LinkId(9));
        let mut t = track(vec![a, clip(2, 30, 10)]);
        let mut ids = IdGen::default();
        let mut links = HashMap::new();
        ripple_open(&mut t, 10, 5, &mut links, &mut ids);
        assert_eq!(spans(&t), vec![(0, 10), (15, 25), (35, 45)]);
        assert_eq!(t.clips[1].source_start, 10.0);
        assert!(t.clips[1].link.is_some());
        assert_ne!(t.clips[1].link, Some(LinkId(9)));
    }

    #[test]
    fn insert_sorted_keeps_clips_ordered() {
        let mut t = track(vec![clip(1, 0, 10), clip(2, 50, 10)]);
        insert_sorted(&mut t, clip(3, 20, 10));
        assert_eq!(spans(&t), vec![(0, 10), (20, 30), (50, 60)]);
    }
}
```

Adicionar `mod track_ops;` em `lib.rs`.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core track_ops`
Expected: erro de compilação (funções não existem).

- [ ] **Step 3: Implementar** (acima dos testes)

```rust
/// Divide `clip` em `at` (precisa de `start < at < end`). A direita recebe `right_id`.
pub(crate) fn split_clip(clip: &Clip, at: Frame, right_id: ClipId) -> (Clip, Clip) {
    debug_assert!(clip.start < at && at < clip.end(), "corte fora do clipe");
    let mut left = clip.clone();
    left.duration = at - clip.start;
    left.fade_out = 0;
    left.fade_in = left.fade_in.min(left.duration);

    let mut right = clip.clone();
    right.id = right_id;
    right.start = at;
    right.duration = clip.end() - at;
    right.source_start = clip.source_time_at(at);
    right.fade_in = 0;
    right.fade_out = right.fade_out.min(right.duration);
    (left, right)
}

pub(crate) fn insert_sorted(track: &mut Track, clip: Clip) {
    let i = track.clips.partition_point(|c| c.start < clip.start);
    track.clips.insert(i, clip);
}

/// Deixa `[start, end)` vazio: apaga, apara ou divide o que estiver ali (overwrite).
pub(crate) fn clear_range(track: &mut Track, start: Frame, end: Frame, ids: &mut IdGen) {
    if start >= end {
        return;
    }
    for clip in std::mem::take(&mut track.clips) {
        let (cs, ce) = (clip.start, clip.end());
        if ce <= start || cs >= end {
            track.clips.push(clip);
        } else if cs < start && ce > end {
            let (left, _) = split_clip(&clip, start, clip.id);
            let (_, mut right) = split_clip(&clip, end, ClipId(ids.fresh()));
            right.link = None;
            track.clips.push(left);
            track.clips.push(right);
        } else if cs < start {
            track.clips.push(split_clip(&clip, start, clip.id).0);
        } else if ce > end {
            track.clips.push(split_clip(&clip, end, clip.id).1);
        }
        // senão: o clipe está todo dentro do intervalo e sai.
    }
    track.clips.sort_by_key(|c| c.start);
}

/// Mapeia o link de uma metade cortada para um link novo, compartilhado entre as faixas.
pub(crate) fn relink(link: Option<LinkId>, map: &mut HashMap<LinkId, LinkId>, ids: &mut IdGen) -> Option<LinkId> {
    link.map(|l| *map.entry(l).or_insert_with(|| LinkId(ids.fresh())))
}

/// Abre um buraco de `len` frames em `at`: divide o clipe que atravessa `at` e empurra o resto.
pub(crate) fn ripple_open(
    track: &mut Track,
    at: Frame,
    len: Frame,
    links: &mut HashMap<LinkId, LinkId>,
    ids: &mut IdGen,
) {
    if let Some(i) = track.clips.iter().position(|c| c.start < at && c.end() > at) {
        let clip = track.clips.remove(i);
        let (left, mut right) = split_clip(&clip, at, ClipId(ids.fresh()));
        right.link = relink(clip.link, links, ids);
        track.clips.insert(i, left);
        track.clips.insert(i + 1, right);
    }
    for c in track.clips.iter_mut().filter(|c| c.start >= at) {
        c.start += len;
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: 13 testes passando.

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): track range operations

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Editor, histórico e colocar clipes (Overwrite/Append/faixas)

**Files:**
- Create: `crates/pawcut-core/src/edit.rs`, `crates/pawcut-core/src/editor.rs`, `crates/pawcut-core/tests/common/mod.rs`, `crates/pawcut-core/tests/placement.rs`, `crates/pawcut-core/tests/history.rs`
- Modify: `crates/pawcut-core/src/lib.rs`

**Interfaces:**
- Consumes: modelo (Task 2) e `clear_range`/`insert_sorted` (Task 3).
- Produces:
  - `ClipDraft { source: ClipSource, source_start: f64, duration: Frame }`, `Placement { track: TrackId, draft: ClipDraft }`, `TrackFlags { locked, enabled, muted, solo: bool }`.
  - `Edit::{AddMedia(MediaInfo), AddTrack(TrackKind), RemoveTrack(TrackId), SetTrackFlags { track, flags }, Overwrite { at, items: Vec<Placement> }, Append { items }}`. As Tasks 5–9 acrescentam variantes.
  - `Outcome { media: Option<MediaId>, track: Option<TrackId>, clips: Vec<ClipId> }` (Default).
  - `EditError::{ClipNotFound(ClipId), TrackNotFound(TrackId), MediaNotFound(MediaId), TrackLocked, WrongTrackKind, InvalidValue(&'static str), NoChange}`.
  - `Editor::new(Project)`, `project() -> &Project`, `apply(Edit) -> Result<Outcome, EditError>`, `undo() -> bool`, `redo() -> bool`, `can_undo()`, `can_redo()`. O histórico guarda até 200 passos.
  - Crate-private: `edit::apply(&mut Project, Edit)`, `edit::normalize_links(&mut Project)`, que tira o link de grupos com menos de 2 clipes.
  - Regra: quando um `Overwrite`/`Append` (e, mais tarde, `Insert`) traz mais de um item, todos os clipes novos ficam ligados entre si. Não é permitido repetir a mesma faixa em dois itens.

- [ ] **Step 1: Helpers de teste**

`crates/pawcut-core/tests/common/mod.rs`:
```rust
#![allow(dead_code)]

use pawcut_core::*;

pub fn new_editor() -> Editor {
    Editor::new(Project::new(ProjectSettings::default()))
}

pub fn add_video(ed: &mut Editor, frames: Frame, has_audio: bool) -> MediaId {
    ed.apply(Edit::AddMedia(MediaInfo {
        path: "/videos/teste.mp4".into(),
        kind: MediaKind::Video { has_audio },
        duration: Some(frames),
        width: 1920,
        height: 1080,
    }))
    .unwrap()
    .media
    .unwrap()
}

pub fn add_audio(ed: &mut Editor, frames: Frame) -> MediaId {
    ed.apply(Edit::AddMedia(MediaInfo {
        path: "/musicas/teste.mp3".into(),
        kind: MediaKind::Audio,
        duration: Some(frames),
        width: 0,
        height: 0,
    }))
    .unwrap()
    .media
    .unwrap()
}

pub fn v(ed: &Editor, n: usize) -> TrackId {
    ed.project().video_tracks[n].id
}

pub fn a(ed: &Editor, n: usize) -> TrackId {
    ed.project().audio_tracks[n].id
}

pub fn solid() -> ClipSource {
    ClipSource::SolidColor(Rgba::new(255, 0, 0, 255))
}

pub fn draft(source: ClipSource, source_start: f64, duration: Frame) -> ClipDraft {
    ClipDraft { source, source_start, duration }
}

/// Overwrite de um único clipe. Devolve o id criado.
pub fn place(ed: &mut Editor, track: TrackId, source: ClipSource, at: Frame, source_start: f64, duration: Frame) -> ClipId {
    ed.apply(Edit::Overwrite { at, items: vec![Placement { track, draft: draft(source, source_start, duration) }] })
        .unwrap()
        .clips[0]
}

/// Coloca o vídeo em V1 e o áudio em A1, ligados. Devolve (vídeo, áudio).
pub fn place_av(ed: &mut Editor, media: MediaId, at: Frame, source_start: f64, duration: Frame) -> (ClipId, ClipId) {
    let (vt, at_) = (v(ed, 0), a(ed, 0));
    let out = ed
        .apply(Edit::Overwrite {
            at,
            items: vec![
                Placement { track: vt, draft: draft(ClipSource::Media(media), source_start, duration) },
                Placement { track: at_, draft: draft(ClipSource::Media(media), source_start, duration) },
            ],
        })
        .unwrap();
    (out.clips[0], out.clips[1])
}

pub fn spans(ed: &Editor, track: TrackId) -> Vec<(Frame, Frame)> {
    ed.project().track(track).unwrap().clips.iter().map(|c| (c.start, c.end())).collect()
}

pub fn clip(ed: &Editor, id: ClipId) -> Clip {
    ed.project().clip(id).unwrap().clone()
}

pub fn lock(ed: &mut Editor, track: TrackId) {
    ed.apply(Edit::SetTrackFlags {
        track,
        flags: TrackFlags { locked: true, enabled: true, muted: false, solo: false },
    })
    .unwrap();
}
```

- [ ] **Step 2: Testes de colocação (falhando)**

`crates/pawcut-core/tests/placement.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

#[test]
fn overwrite_places_linked_video_and_audio() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 10, 0.0, 100);
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(10, 110)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(10, 110)]);
    assert!(clip(&ed, vc).link.is_some());
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
    assert_eq!(ed.project().expand_linked(&[vc]), vec![vc, ac]);
}

#[test]
fn overwrite_replaces_what_is_underneath() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 100);
    place(&mut ed, v1, ClipSource::Media(m), 40, 200.0, 20);
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 60), (60, 100)]);
}

#[test]
fn append_goes_to_end_of_timeline() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 50);
    ed.apply(Edit::Append { items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 30) }] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 50), (50, 80)]);
}

#[test]
fn rejects_clip_past_end_of_media() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let r = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement { track: v1, draft: draft(ClipSource::Media(m), 50.0, 60) }],
    });
    assert!(matches!(r, Err(EditError::InvalidValue(_))));
}

#[test]
fn rejects_wrong_track_kinds() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let a1 = a(&ed, 0);
    let silent = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement { track: a1, draft: draft(ClipSource::Media(m), 0.0, 10) }],
    });
    assert_eq!(silent, Err(EditError::WrongTrackKind));
    let gen_on_audio =
        ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: a1, draft: draft(solid(), 0.0, 10) }] });
    assert_eq!(gen_on_audio, Err(EditError::WrongTrackKind));
}

#[test]
fn rejects_locked_track_zero_duration_and_negative_position() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let zero = ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 0) }] });
    assert!(matches!(zero, Err(EditError::InvalidValue(_))));
    let neg = ed.apply(Edit::Overwrite { at: -5, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 10) }] });
    assert!(matches!(neg, Err(EditError::InvalidValue(_))));
    lock(&mut ed, v1);
    let locked = ed.apply(Edit::Overwrite { at: 0, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 10) }] });
    assert_eq!(locked, Err(EditError::TrackLocked));
}

#[test]
fn generators_have_no_length_limit() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 100_000);
    assert_eq!(spans(&ed, v1), vec![(0, 100_000)]);
}

#[test]
fn add_and_remove_tracks() {
    let mut ed = new_editor();
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    assert_eq!(ed.project().track_label(v2).as_deref(), Some("V2"));
    let only_audio = a(&ed, 0);
    assert!(matches!(ed.apply(Edit::RemoveTrack(only_audio)), Err(EditError::InvalidValue(_))));
    ed.apply(Edit::RemoveTrack(v2)).unwrap();
    assert_eq!(ed.project().video_tracks.len(), 1);
}
```

`crates/pawcut-core/tests/history.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

#[test]
fn undo_and_redo_walk_through_snapshots() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let before = ed.project().clone();
    place_av(&mut ed, m, 0, 0.0, 100);
    let after = ed.project().clone();
    assert!(ed.undo());
    assert_eq!(ed.project(), &before);
    assert!(ed.redo());
    assert_eq!(ed.project(), &after);
    assert!(!ed.redo());
}

#[test]
fn new_edit_clears_redo() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    ed.undo();
    assert!(ed.can_redo());
    place(&mut ed, v1, solid(), 20, 0.0, 10);
    assert!(!ed.can_redo());
}

#[test]
fn failed_or_empty_edit_does_not_touch_history() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, true);
    let v1 = v(&ed, 0);
    let bad = ed.apply(Edit::Overwrite {
        at: 0,
        items: vec![Placement { track: v1, draft: draft(ClipSource::Media(m), 90.0, 50) }],
    });
    assert!(bad.is_err());
    let same_flags = ed.apply(Edit::SetTrackFlags {
        track: v1,
        flags: TrackFlags { locked: false, enabled: true, muted: false, solo: false },
    });
    assert_eq!(same_flags, Err(EditError::NoChange));
    assert!(ed.undo()); // desfaz o AddMedia
    assert!(!ed.undo());
}

#[test]
fn history_is_capped_at_200_steps() {
    let mut ed = new_editor();
    for _ in 0..250 {
        ed.apply(Edit::AddTrack(TrackKind::Audio)).unwrap();
    }
    for _ in 0..200 {
        assert!(ed.undo());
    }
    assert!(!ed.undo());
}
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p pawcut-core`
Expected: erro de compilação (`Editor`, `Edit` etc. não existem).

- [ ] **Step 4: Implementar `edit.rs`**

```rust
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
```

> Nota: `if let ... && ...` (let chains) é estável na edition 2024 a partir do Rust 1.88. Se o compilador for mais antigo, troque por um `if let` aninhado.

- [ ] **Step 5: Implementar `editor.rs`**

```rust
use crate::edit::{self, Edit, EditError, Outcome};
use crate::model::Project;

const HISTORY_LIMIT: usize = 200;

/// Dono do projeto aberto. Toda alteração passa por `apply`, que guarda um retrato para desfazer.
#[derive(Debug, Clone)]
pub struct Editor {
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Self { project, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Aplica a edição numa cópia. Em caso de erro ou edição vazia, o projeto e o histórico ficam intactos.
    pub fn apply(&mut self, edit: Edit) -> Result<Outcome, EditError> {
        let mut next = self.project.clone();
        let outcome = edit::apply(&mut next, edit)?;
        edit::normalize_links(&mut next);
        if next == self.project {
            return Err(EditError::NoChange);
        }
        self.undo.push(std::mem::replace(&mut self.project, next));
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        Ok(outcome)
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.project, prev));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.project, next));
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
```

Em `lib.rs`, adicionar:
```rust
mod edit;
mod editor;

pub use edit::{ClipDraft, Edit, EditError, Outcome, Placement, TrackFlags};
pub use editor::Editor;
```

- [ ] **Step 6: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (13 unitários + 8 de placement + 4 de history).

- [ ] **Step 7: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): editor with snapshot undo, overwrite/append and tracks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Cortar e apagar (Blade, Delete, Ripple Delete)

**Files:**
- Modify: `crates/pawcut-core/src/edit.rs`, `crates/pawcut-core/src/lib.rs`
- Create: `crates/pawcut-core/tests/cutting.rs`

**Interfaces:**
- Consumes: `split_clip`, `relink` (Task 3) e `Editor` (Task 4).
- Produces:
  - `BladeTarget::{Clips(Vec<ClipId>), AllTracks}`.
  - `Edit::Blade { at: Frame, target: BladeTarget }`: `Outcome.clips` = ids das metades da direita. As metades direitas de um par ligado continuam ligadas entre si, com um link novo.
  - `Edit::Delete { clips }` deixa um buraco. `Edit::RippleDelete { clips }` fecha o buraco **só nas faixas dos clipes apagados**.
  - Os helpers `dedup(Vec<ClipId>) -> Vec<ClipId>` e `locate_unlocked(&Project, ClipId) -> Result<(TrackId, usize), EditError>`, usados pelas tarefas seguintes.
  - A interface é quem expande o Linked Selection (`Project::expand_linked`) antes de chamar. O core nunca expande sozinho.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/cutting.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

#[test]
fn blade_cuts_linked_pair_and_keeps_halves_linked() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let out = ed.apply(Edit::Blade { at: 40, target: BladeTarget::Clips(vec![vc, ac]) }).unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 40), (40, 100)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 40), (40, 100)]);
    let (rv, ra) = (out.clips[0], out.clips[1]);
    assert_eq!(clip(&ed, rv).source_start, 40.0);
    assert!(clip(&ed, rv).link.is_some());
    assert_eq!(clip(&ed, rv).link, clip(&ed, ra).link);
    assert_ne!(clip(&ed, rv).link, clip(&ed, vc).link);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn blade_all_tracks_skips_locked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let a1 = a(&ed, 0);
    lock(&mut ed, a1);
    let out = ed.apply(Edit::Blade { at: 50, target: BladeTarget::AllTracks }).unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 50), (50, 100)]);
    assert_eq!(spans(&ed, a1), vec![(0, 100)]);
    assert_eq!(clip(&ed, out.clips[0]).link, None);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn blade_on_edge_does_nothing() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    assert_eq!(ed.apply(Edit::Blade { at: 0, target: BladeTarget::Clips(vec![c]) }), Err(EditError::NoChange));
    assert_eq!(ed.apply(Edit::Blade { at: 100, target: BladeTarget::Clips(vec![c]) }), Err(EditError::NoChange));
    assert_eq!(spans(&ed, v1), vec![(0, 100)]);
}

#[test]
fn delete_leaves_gap() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 0, 0.0, 10);
    let mid = place(&mut ed, v1, solid(), 10, 0.0, 20);
    place(&mut ed, v1, solid(), 30, 0.0, 10);
    ed.apply(Edit::Delete { clips: vec![mid] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10), (30, 40)]);
}

#[test]
fn ripple_delete_closes_gap_on_same_tracks_only() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    place_av(&mut ed, m, 0, 0.0, 10);
    let (mid_v, _) = place_av(&mut ed, m, 10, 10.0, 20);
    place_av(&mut ed, m, 30, 30.0, 10);
    place(&mut ed, v2, solid(), 35, 0.0, 10);
    let pair = ed.project().expand_linked(&[mid_v]);
    ed.apply(Edit::RippleDelete { clips: pair }).unwrap();
    assert_eq!(spans(&ed, v(&ed, 0)), vec![(0, 10), (10, 20)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(0, 10), (10, 20)]);
    assert_eq!(spans(&ed, v2), vec![(35, 45)]);
}

#[test]
fn deleting_one_half_of_a_pair_unlinks_the_other() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    ed.apply(Edit::Delete { clips: vec![vc] }).unwrap();
    assert_eq!(clip(&ed, ac).link, None);
}

#[test]
fn cannot_delete_on_locked_track() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    lock(&mut ed, v1);
    assert_eq!(ed.apply(Edit::Delete { clips: vec![c] }), Err(EditError::TrackLocked));
}

#[test]
fn duplicate_ids_in_selection_are_harmless() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let d = place(&mut ed, v1, solid(), 10, 0.0, 10);
    ed.apply(Edit::RippleDelete { clips: vec![c, c] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10)]);
    ed.apply(Edit::Blade { at: 5, target: BladeTarget::Clips(vec![d, d]) }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 5), (5, 10)]);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test cutting`
Expected: erro de compilação (`BladeTarget`, `Edit::Blade` etc. não existem).

- [ ] **Step 3: Implementar em `edit.rs`**

Trocar o import de `track_ops` por:
```rust
use crate::track_ops::{clear_range, insert_sorted, relink, split_clip};
```

Logo abaixo de `TrackFlags`, acrescentar:
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum BladeTarget {
    /// Cortar só estes clipes (seleção, já expandida pelo Linked Selection).
    Clips(Vec<ClipId>),
    /// Cortar tudo sob o cursor nas faixas destravadas (Ctrl+B sem seleção).
    AllTracks,
}
```

No `enum Edit`, acrescentar:
```rust
    /// Corta em `at`. Clipes em que `at` cai na borda são ignorados.
    Blade { at: Frame, target: BladeTarget },
    /// Apaga deixando buraco.
    Delete { clips: Vec<ClipId> },
    /// Apaga e puxa o resto da mesma faixa para fechar o buraco (Shift+Backspace).
    RippleDelete { clips: Vec<ClipId> },
```

No `match` de `apply`, acrescentar:
```rust
        Edit::Blade { at, target } => blade(p, at, target),
        Edit::Delete { clips } => delete(p, clips, false),
        Edit::RippleDelete { clips } => delete(p, clips, true),
```

Novas funções (no fim de `edit.rs`):
```rust
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
```

Em `lib.rs`, atualizar a linha:
```rust
pub use edit::{BladeTarget, ClipDraft, Edit, EditError, Outcome, Placement, TrackFlags};
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+8 de cutting).

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): blade, delete and ripple delete

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Mover clipes

**Files:**
- Modify: `crates/pawcut-core/src/edit.rs`
- Create: `crates/pawcut-core/tests/moving.rs`

**Interfaces:**
- Consumes: `dedup`, `locate_unlocked` (Task 5) e `clear_range`, `insert_sorted`.
- Produces: `Edit::Move { clips: Vec<ClipId>, delta: Frame, video_track_delta: i32, audio_track_delta: i32 }`. Todos os clipes andam o mesmo `delta`, limitado para nenhum começar antes do 0. Os clipes de vídeo trocam de faixa por `video_track_delta` e os de áudio por `audio_track_delta`. O destino é sobrescrito (overwrite). `Outcome.clips` = clipes movidos.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/moving.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

fn mv(clips: Vec<ClipId>, delta: Frame, vtd: i32, atd: i32) -> Edit {
    Edit::Move { clips, delta, video_track_delta: vtd, audio_track_delta: atd }
}

#[test]
fn move_shifts_in_time_and_overwrites_destination() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 50);
    let c2 = place(&mut ed, v1, ClipSource::Media(m), 100, 100.0, 50);
    ed.apply(mv(vec![c2], -70, 0, 0)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 30), (30, 80)]);
    assert_eq!(clip(&ed, c2).source_start, 100.0);
}

#[test]
fn move_never_goes_before_zero() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 20, 0.0, 10);
    ed.apply(mv(vec![c], -100, 0, 0)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 10)]);
}

#[test]
fn move_between_video_tracks_with_linked_audio_staying_on_its_track() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 50);
    ed.apply(mv(vec![vc, ac], 10, 1, 0)).unwrap();
    assert!(spans(&ed, v(&ed, 0)).is_empty());
    assert_eq!(spans(&ed, v2), vec![(10, 60)]);
    assert_eq!(spans(&ed, a(&ed, 0)), vec![(10, 60)]);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
}

#[test]
fn move_to_missing_track_fails() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    assert!(matches!(ed.apply(mv(vec![c], 0, 5, 0)), Err(EditError::InvalidValue(_))));
}

#[test]
fn zero_move_is_no_change() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    assert_eq!(ed.apply(mv(vec![c], 0, 0, 0)), Err(EditError::NoChange));
    assert_eq!(ed.apply(mv(vec![c], -5, 0, 0)), Err(EditError::NoChange));
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test moving`
Expected: erro de compilação (`Edit::Move` não existe).

- [ ] **Step 3: Implementar**

No `enum Edit`:
```rust
    /// Arrasta clipes no tempo e entre faixas do mesmo tipo, sobrescrevendo o destino.
    Move { clips: Vec<ClipId>, delta: Frame, video_track_delta: i32, audio_track_delta: i32 },
```

No `match`:
```rust
        Edit::Move { clips, delta, video_track_delta, audio_track_delta } => {
            move_clips(p, clips, delta, video_track_delta, audio_track_delta)
        }
```

Função nova:
```rust
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
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+5 de moving).

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): move clips in time and across tracks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Aparar (Trim)

**Files:**
- Modify: `crates/pawcut-core/src/edit.rs`, `crates/pawcut-core/src/lib.rs`
- Create: `crates/pawcut-core/tests/trimming.rs`

**Interfaces:**
- Consumes: `dedup`, `locate_unlocked`.
- Produces:
  - `Edge::{Start, End}`.
  - `Edit::Trim { clips: Vec<ClipId>, edge: Edge, delta: Frame }`. Todos os clipes recebem o **mesmo** delta, limitado pelo mais restrito: vizinho, frame 0, início e fim da fonte, mínimo de 1 frame.
  - O helper `clamp_fades(&mut Clip)`, também usado na Task 9.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/trimming.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

fn trim(clips: Vec<ClipId>, edge: Edge, delta: Frame) -> Edit {
    Edit::Trim { clips, edge, delta }
}

#[test]
fn trim_end_extends_until_next_clip() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c1 = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 50);
    place(&mut ed, v1, ClipSource::Media(m), 80, 0.0, 20);
    ed.apply(trim(vec![c1], Edge::End, 100)).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 80), (80, 100)]);
}

#[test]
fn trim_end_stops_at_end_of_media() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 30.0, 50);
    ed.apply(trim(vec![c], Edge::End, 100)).unwrap();
    assert_eq!(clip(&ed, c).duration, 70);
}

#[test]
fn trim_start_moves_source_in_point() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 20, 20.0, 50);
    ed.apply(trim(vec![c], Edge::Start, 10)).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.start, c.duration, c.source_start), (30, 40, 30.0));
}

#[test]
fn trim_start_cannot_go_before_source_start_or_frame_zero() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 20, 5.0, 50);
    ed.apply(trim(vec![c], Edge::Start, -100)).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.start, c.duration, c.source_start), (15, 55, 0.0));
}

#[test]
fn generator_start_trim_stops_at_frame_zero() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 20, 1000.0, 10);
    ed.apply(trim(vec![c], Edge::Start, -100)).unwrap();
    assert_eq!(clip(&ed, c).start, 0);
}

#[test]
fn trim_never_leaves_less_than_one_frame() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 10);
    ed.apply(trim(vec![c], Edge::End, -50)).unwrap();
    assert_eq!(clip(&ed, c).duration, 1);
    assert_eq!(ed.apply(trim(vec![c], Edge::End, -50)), Err(EditError::NoChange));
}

#[test]
fn trim_linked_pair_moves_both_by_same_amount() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let music = add_audio(&mut ed, 300);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 50);
    let a1 = a(&ed, 0);
    place(&mut ed, a1, ClipSource::Media(music), 60, 0.0, 30);
    ed.apply(trim(vec![vc, ac], Edge::End, 100)).unwrap();
    assert_eq!(clip(&ed, vc).end(), 60);
    assert_eq!(clip(&ed, ac).end(), 60);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test trimming`
Expected: erro de compilação (`Edge`, `Edit::Trim` não existem).

- [ ] **Step 3: Implementar**

Depois de `BladeTarget`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}
```

No `enum Edit`:
```rust
    /// Arrasta a borda dos clipes. Positivo = para a direita.
    Trim { clips: Vec<ClipId>, edge: Edge, delta: Frame },
```

No `match`:
```rust
        Edit::Trim { clips, edge, delta } => trim(p, clips, edge, delta),
```

Funções novas:
```rust
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
```

> Geradores e imagens não têm limite de fonte (`source_length` = `None`), então o trim de início só para no clipe anterior ou no frame 0. É o que o teste `generator_start_trim_stops_at_frame_zero` cobre.

Em `lib.rs`:
```rust
pub use edit::{BladeTarget, ClipDraft, Edge, Edit, EditError, Outcome, Placement, TrackFlags};
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+7 de trimming).

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): trim clip edges with source and neighbor limits

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Insert (com ripple)

**Files:**
- Modify: `crates/pawcut-core/src/edit.rs`
- Create: `crates/pawcut-core/tests/insert.rs`

**Interfaces:**
- Consumes: `ripple_open` (Task 3) e `validate_items`, `place` (Task 4).
- Produces: `Edit::Insert { at: Frame, items: Vec<Placement> }`. Abre um buraco do tamanho do maior item em **todas as faixas destravadas** e coloca os itens nele. As metades direitas de pares ligados continuam ligadas.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/insert.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

#[test]
fn insert_pushes_everything_after_on_all_unlocked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    place_av(&mut ed, m, 0, 0.0, 100);
    place(&mut ed, v2, solid(), 120, 0.0, 30);
    let v1 = v(&ed, 0);
    let a1 = a(&ed, 0);
    ed.apply(Edit::Insert {
        at: 40,
        items: vec![Placement { track: v1, draft: draft(ClipSource::Media(m), 200.0, 30) }],
    })
    .unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 70), (70, 130)]);
    assert_eq!(spans(&ed, a1), vec![(0, 40), (70, 130)]);
    assert_eq!(spans(&ed, v2), vec![(150, 180)]);
    let p = ed.project();
    let right_v = p.track(v1).unwrap().clip_at(70).unwrap();
    let right_a = p.track(a1).unwrap().clip_at(70).unwrap();
    assert!(right_v.link.is_some());
    assert_eq!(right_v.link, right_a.link);
    assert_eq!(right_v.source_start, 40.0);
}

#[test]
fn insert_skips_locked_tracks() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, ac) = place_av(&mut ed, m, 0, 0.0, 100);
    let (v1, a1) = (v(&ed, 0), a(&ed, 0));
    lock(&mut ed, a1);
    ed.apply(Edit::Insert { at: 40, items: vec![Placement { track: v1, draft: draft(solid(), 0.0, 30) }] }).unwrap();
    assert_eq!(spans(&ed, v1), vec![(0, 40), (40, 70), (70, 130)]);
    assert_eq!(spans(&ed, a1), vec![(0, 100)]);
    assert_eq!(clip(&ed, vc).link, clip(&ed, ac).link);
    assert_eq!(ed.project().track(v1).unwrap().clip_at(70).unwrap().link, None);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test insert`
Expected: erro de compilação (`Edit::Insert` não existe).

- [ ] **Step 3: Implementar**

Import de `track_ops`:
```rust
use crate::track_ops::{clear_range, insert_sorted, relink, ripple_open, split_clip};
```

No `enum Edit`:
```rust
    /// Abre espaço em `at` em todas as faixas destravadas e coloca os itens (F9).
    Insert { at: Frame, items: Vec<Placement> },
```

No `match`:
```rust
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
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+2 de insert).

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): insert edit with ripple

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Propriedades, velocidade e link

**Files:**
- Modify: `crates/pawcut-core/src/edit.rs`, `crates/pawcut-core/src/lib.rs`
- Create: `crates/pawcut-core/tests/properties.rs`

**Interfaces:**
- Consumes: `dedup`, `locate_unlocked`, `clamp_fades`.
- Produces:
  - `ClipChange::{Transform(Transform), VolumeDb(f32), FadeIn(Frame), FadeOut(Frame), Source(ClipSource)}`.
  - `Edit::SetClip { clip: ClipId, change: ClipChange }`. Limites: escala > 0 e finita; opacidade de 0 a 1; x/y finitos; volume de -96 a +24 dB; fade ≥ 0, cortado para caber no clipe; `Source` só troca gerador por gerador.
  - `Edit::SetSpeed { clips: Vec<ClipId>, speed: f64 }`, com speed de 0.1 a 10. A nova duração é `round(duração × velocidade antiga / velocidade nova)`, com mínimo de 1 e cortada no início do próximo clipe.
  - `Edit::Link { clips }` (≥ 2 clipes) e `Edit::Unlink { clips }`.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/properties.rs`:
```rust
mod common;

use common::*;
use pawcut_core::*;

fn set(clip: ClipId, change: ClipChange) -> Edit {
    Edit::SetClip { clip, change }
}

#[test]
fn set_transform_volume_and_fades() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    let t = Transform { x: 10.0, y: -5.0, scale: 0.5, opacity: 0.8 };
    ed.apply(set(c, ClipChange::Transform(t))).unwrap();
    ed.apply(set(c, ClipChange::VolumeDb(-6.0))).unwrap();
    ed.apply(set(c, ClipChange::FadeIn(30))).unwrap();
    ed.apply(set(c, ClipChange::FadeOut(90))).unwrap();
    let c = clip(&ed, c);
    assert_eq!(c.transform, t);
    assert_eq!(c.volume_db, -6.0);
    assert_eq!((c.fade_in, c.fade_out), (30, 70));
}

#[test]
fn rejects_bad_property_values() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    let bad_scale = Transform { scale: 0.0, ..Transform::default() };
    let bad_opacity = Transform { opacity: 1.5, ..Transform::default() };
    for change in [
        ClipChange::Transform(bad_scale),
        ClipChange::Transform(bad_opacity),
        ClipChange::VolumeDb(f32::NAN),
        ClipChange::VolumeDb(40.0),
        ClipChange::FadeIn(-1),
    ] {
        assert!(matches!(ed.apply(set(c, change)), Err(EditError::InvalidValue(_))));
    }
}

#[test]
fn only_generators_can_change_source() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let g = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let text = ClipSource::Text(TextStyle {
        text: "Olá".into(),
        font: "Sans".into(),
        size: 64.0,
        color: Rgba::new(255, 255, 255, 255),
    });
    ed.apply(set(g, ClipChange::Source(text.clone()))).unwrap();
    assert_eq!(clip(&ed, g).source, text);
    let mc = place(&mut ed, v1, ClipSource::Media(m), 20, 0.0, 10);
    assert!(matches!(ed.apply(set(mc, ClipChange::Source(solid()))), Err(EditError::InvalidValue(_))));
}

#[test]
fn speed_2x_halves_duration_and_0_5x_doubles_until_next_clip() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 100);
    place(&mut ed, v1, solid(), 150, 0.0, 10);
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 2.0 }).unwrap();
    assert_eq!((clip(&ed, c).duration, clip(&ed, c).speed), (50, 2.0));
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 0.5 }).unwrap();
    assert_eq!((clip(&ed, c).duration, clip(&ed, c).speed), (150, 0.5));
    assert_eq!(spans(&ed, v1), vec![(0, 150), (150, 160)]);
}

#[test]
fn rejects_speed_out_of_range() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    for speed in [0.0, 0.05, 20.0, f64::NAN] {
        assert!(matches!(ed.apply(Edit::SetSpeed { clips: vec![c], speed }), Err(EditError::InvalidValue(_))));
    }
}

#[test]
fn trim_with_speed_respects_media_end() {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 100, false);
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, ClipSource::Media(m), 0, 0.0, 20);
    ed.apply(Edit::SetSpeed { clips: vec![c], speed: 2.0 }).unwrap();
    assert_eq!(clip(&ed, c).duration, 10);
    ed.apply(Edit::Trim { clips: vec![c], edge: Edge::End, delta: 1000 }).unwrap();
    assert_eq!(clip(&ed, c).duration, 50);
}

#[test]
fn fades_shrink_with_clip() {
    let mut ed = new_editor();
    let v1 = v(&ed, 0);
    let c = place(&mut ed, v1, solid(), 0, 0.0, 100);
    ed.apply(set(c, ClipChange::FadeIn(40))).unwrap();
    ed.apply(set(c, ClipChange::FadeOut(40))).unwrap();
    ed.apply(Edit::Trim { clips: vec![c], edge: Edge::End, delta: -70 }).unwrap();
    let c = clip(&ed, c);
    assert_eq!((c.duration, c.fade_in, c.fade_out), (30, 30, 0));
}

#[test]
fn link_and_unlink() {
    let mut ed = new_editor();
    let music = add_audio(&mut ed, 300);
    let (v1, a1) = (v(&ed, 0), a(&ed, 0));
    let s = place(&mut ed, v1, solid(), 0, 0.0, 10);
    let au = place(&mut ed, a1, ClipSource::Media(music), 0, 0.0, 10);
    assert!(matches!(ed.apply(Edit::Link { clips: vec![s] }), Err(EditError::InvalidValue(_))));
    ed.apply(Edit::Link { clips: vec![s, au] }).unwrap();
    assert!(clip(&ed, s).link.is_some());
    assert_eq!(clip(&ed, s).link, clip(&ed, au).link);
    ed.apply(Edit::Unlink { clips: vec![s] }).unwrap();
    assert_eq!(clip(&ed, s).link, None);
    assert_eq!(clip(&ed, au).link, None);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test properties`
Expected: erro de compilação (`ClipChange`, `SetClip`, `SetSpeed`, `Link`, `Unlink` não existem).

- [ ] **Step 3: Implementar**

Depois de `Edge`:
```rust
/// Uma propriedade do Inspector.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipChange {
    Transform(Transform),
    VolumeDb(f32),
    FadeIn(Frame),
    FadeOut(Frame),
    /// Só para geradores (cor sólida/texto).
    Source(ClipSource),
}
```

No `enum Edit`:
```rust
    SetClip { clip: ClipId, change: ClipChange },
    SetSpeed { clips: Vec<ClipId>, speed: f64 },
    Link { clips: Vec<ClipId> },
    Unlink { clips: Vec<ClipId> },
```

No `match`:
```rust
        Edit::SetClip { clip, change } => set_clip(p, clip, change),
        Edit::SetSpeed { clips, speed } => set_speed(p, clips, speed),
        Edit::Link { clips } => set_link(p, clips, true),
        Edit::Unlink { clips } => set_link(p, clips, false),
```

Funções novas:
```rust
fn set_clip(p: &mut Project, id: ClipId, change: ClipChange) -> Result<Outcome, EditError> {
    locate_unlocked(p, id)?;
    let c = p.clip_mut(id).expect("clipe validado acima");
    match change {
        ClipChange::Transform(t) => {
            if !(t.scale.is_finite() && t.scale > 0.0) {
                return Err(EditError::InvalidValue("a escala precisa ser maior que zero"));
            }
            if !(0.0..=1.0).contains(&t.opacity) {
                return Err(EditError::InvalidValue("a opacidade vai de 0 a 1"));
            }
            if !(t.x.is_finite() && t.y.is_finite()) {
                return Err(EditError::InvalidValue("posição inválida"));
            }
            c.transform = t;
        }
        ClipChange::VolumeDb(db) => {
            if !(-96.0..=24.0).contains(&db) {
                return Err(EditError::InvalidValue("o volume vai de -96 dB a +24 dB"));
            }
            c.volume_db = db;
        }
        ClipChange::FadeIn(f) => {
            if f < 0 {
                return Err(EditError::InvalidValue("fade negativo"));
            }
            c.fade_in = f.min(c.duration - c.fade_out);
        }
        ClipChange::FadeOut(f) => {
            if f < 0 {
                return Err(EditError::InvalidValue("fade negativo"));
            }
            c.fade_out = f.min(c.duration - c.fade_in);
        }
        ClipChange::Source(s) => {
            if !(c.source.is_generator() && s.is_generator()) {
                return Err(EditError::InvalidValue("só geradores podem trocar de conteúdo"));
            }
            c.source = s;
        }
    }
    Ok(Outcome { clips: vec![id], ..Outcome::default() })
}

fn set_speed(p: &mut Project, clips: Vec<ClipId>, speed: f64) -> Result<Outcome, EditError> {
    if !(0.1..=10.0).contains(&speed) {
        return Err(EditError::InvalidValue("a velocidade vai de 0.1x a 10x"));
    }
    let clips = dedup(clips);
    if clips.is_empty() {
        return Err(EditError::InvalidValue("nenhum clipe selecionado"));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    for &id in &clips {
        let (tid, i) = p.locate(id).expect("clipe validado acima");
        let track = p.track_mut(tid).expect("faixa do clipe existe");
        let next_start = track.clips.get(i + 1).map(|n| n.start);
        let c = &mut track.clips[i];
        let span = c.duration as f64 * c.speed;
        let mut duration = ((span / speed).round() as Frame).max(1);
        if let Some(n) = next_start {
            duration = duration.min(n - c.start);
        }
        c.speed = speed;
        c.duration = duration;
        clamp_fades(c);
    }
    Ok(Outcome { clips, ..Outcome::default() })
}

fn set_link(p: &mut Project, clips: Vec<ClipId>, linked: bool) -> Result<Outcome, EditError> {
    let clips = dedup(clips);
    if linked && clips.len() < 2 {
        return Err(EditError::InvalidValue("selecione pelo menos dois clipes para ligar"));
    }
    for &id in &clips {
        locate_unlocked(p, id)?;
    }
    let link = linked.then(|| LinkId(p.ids.fresh()));
    for &id in &clips {
        p.clip_mut(id).expect("clipe validado acima").link = link;
    }
    Ok(Outcome { clips, ..Outcome::default() })
}
```

Em `lib.rs`:
```rust
pub use edit::{BladeTarget, ClipChange, ClipDraft, Edge, Edit, EditError, Outcome, Placement, TrackFlags};
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+8 de properties).

- [ ] **Step 5: Commit**

```bash
git add crates/pawcut-core
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): clip properties, speed and link/unlink

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Arquivo `.pawcut` e fechamento

**Files:**
- Create: `crates/pawcut-core/src/file.rs`, `crates/pawcut-core/tests/file.rs`
- Modify: `crates/pawcut-core/src/lib.rs`

**Interfaces:**
- Consumes: `Project` (Serialize/Deserialize), `PROJECT_VERSION`.
- Produces:
  - `to_json(&Project) -> Result<String, ProjectFileError>`, `from_json(&str) -> Result<Project, ProjectFileError>`.
  - `save_project(&Project, &Path)`: gravação atômica (grava `.pawcut.tmp` e depois faz `rename`). `load_project(&Path) -> Result<Project, ProjectFileError>`.
  - `ProjectFileError::{Io { path, source }, Parse(serde_json::Error), UnsupportedVersion { found: u64, supported: u32 }}`.

- [ ] **Step 1: Testes (falhando)**

`crates/pawcut-core/tests/file.rs`:
```rust
mod common;

use std::collections::HashSet;

use common::*;
use pawcut_core::*;

fn sample_editor() -> Editor {
    let mut ed = new_editor();
    let m = add_video(&mut ed, 300, true);
    let (vc, _) = place_av(&mut ed, m, 0, 12.5, 100);
    ed.apply(Edit::SetSpeed { clips: vec![vc], speed: 1.5 }).unwrap();
    let v2 = ed.apply(Edit::AddTrack(TrackKind::Video)).unwrap().track.unwrap();
    let text = ClipSource::Text(TextStyle {
        text: "Título".into(),
        font: "Sans".into(),
        size: 72.0,
        color: Rgba::new(255, 255, 0, 255),
    });
    place(&mut ed, v2, text, 10, 0.0, 60);
    ed
}

#[test]
fn save_and_load_round_trip() {
    let ed = sample_editor();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("projeto.pawcut");
    save_project(ed.project(), &path).unwrap();
    let loaded = load_project(&path).unwrap();
    assert_eq!(&loaded, ed.project());
    assert!(!dir.path().join("projeto.pawcut.tmp").exists());
}

#[test]
fn newer_version_is_refused() {
    let ed = sample_editor();
    let mut value: serde_json::Value = serde_json::from_str(&to_json(ed.project()).unwrap()).unwrap();
    value["version"] = 99.into();
    let err = from_json(&value.to_string()).unwrap_err();
    assert!(matches!(err, ProjectFileError::UnsupportedVersion { found: 99, .. }));
}

#[test]
fn garbage_is_a_parse_error() {
    assert!(matches!(from_json("{ isso não é json"), Err(ProjectFileError::Parse(_))));
    assert!(matches!(from_json("{\"version\": 1}"), Err(ProjectFileError::Parse(_))));
}

#[test]
fn missing_file_is_io_error() {
    let err = load_project(std::path::Path::new("/caminho/que/nao/existe.pawcut")).unwrap_err();
    assert!(matches!(err, ProjectFileError::Io { .. }));
    assert!(err.to_string().contains("/caminho/que/nao/existe.pawcut"));
}

#[test]
fn editing_after_load_does_not_reuse_ids() {
    let ed = sample_editor();
    let json = to_json(ed.project()).unwrap();
    let mut ed = Editor::new(from_json(&json).unwrap());
    let v1 = v(&ed, 0);
    place(&mut ed, v1, solid(), 200, 0.0, 10);
    let ids: Vec<ClipId> = ed.project().all_tracks().flat_map(|t| t.clips.iter().map(|c| c.id)).collect();
    let unique: HashSet<ClipId> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len());
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p pawcut-core --test file`
Expected: erro de compilação (`save_project` etc. não existem).

- [ ] **Step 3: Implementar `file.rs`**

```rust
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{PROJECT_VERSION, Project};

#[derive(Debug, Error)]
pub enum ProjectFileError {
    #[error("não foi possível acessar {}: {source}", .path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("arquivo de projeto inválido: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("este projeto é de uma versão mais nova do PawCut (formato {found}; esta versão lê até {supported})")]
    UnsupportedVersion { found: u64, supported: u32 },
}

pub fn to_json(project: &Project) -> Result<String, ProjectFileError> {
    Ok(serde_json::to_string_pretty(project)?)
}

pub fn from_json(text: &str) -> Result<Project, ProjectFileError> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(found) = value.get("version").and_then(serde_json::Value::as_u64)
        && found > u64::from(PROJECT_VERSION)
    {
        return Err(ProjectFileError::UnsupportedVersion { found, supported: PROJECT_VERSION });
    }
    let mut project: Project = serde_json::from_value(value)?;
    // Garante a invariante de ordem mesmo se o arquivo foi editado à mão.
    for t in project.video_tracks.iter_mut().chain(project.audio_tracks.iter_mut()) {
        t.clips.sort_by_key(|c| c.start);
    }
    Ok(project)
}

/// Grava num arquivo temporário e renomeia, para nunca deixar um `.pawcut` pela metade.
pub fn save_project(project: &Project, path: &Path) -> Result<(), ProjectFileError> {
    let text = to_json(project)?;
    let tmp = path.with_extension("pawcut.tmp");
    let io = |source: std::io::Error| ProjectFileError::Io { path: path.to_path_buf(), source };
    fs::write(&tmp, text).map_err(io)?;
    fs::rename(&tmp, path).map_err(io)?;
    Ok(())
}

pub fn load_project(path: &Path) -> Result<Project, ProjectFileError> {
    let text =
        fs::read_to_string(path).map_err(|source| ProjectFileError::Io { path: path.to_path_buf(), source })?;
    from_json(&text)
}
```

Em `lib.rs`:
```rust
mod file;

pub use file::{ProjectFileError, from_json, load_project, save_project, to_json};
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p pawcut-core`
Expected: todos passando (+5 de file).

- [ ] **Step 5: Verificação final do crate**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: sem avisos do clippy e todos os testes passando. Corrigir o que o clippy apontar antes de seguir. Se `lib.rs` tiver a ordem dos `mod`/`pub use` bagunçada, deixar em ordem alfabética.

- [ ] **Step 6: Commit**

```bash
git add -A
git -c user.name="Luis" -c user.email="luispim2005@gmail.com" commit -m "feat(core): .pawcut save/load with version check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Cobertura do spec por este plano

| Requisito do spec | Onde |
|---|---|
| Tempo em frames inteiros, fps/resolução do projeto, timecode `HH:MM:SS:FF` | Tasks 1–2 |
| Faixas V/A, lock, enabled, mute/solo, V1 embaixo | Tasks 2, 4 |
| Clipes de mídia, cor sólida, texto e imagem (sem limite de fonte) | Tasks 2, 4 |
| Overwrite / Insert / Append | Tasks 4, 8 |
| Blade, Delete, Ripple Delete | Task 5 |
| Mover no tempo e entre faixas | Task 6 |
| Trim nas bordas | Task 7 |
| Transform fixo, volume, fades, velocidade, link/unlink (separar áudio) | Task 9 |
| Desfazer/refazer | Task 4 |
| `.pawcut` com `version` | Task 10 |
| Snapping e ↑/↓ (dados) | `Project::edit_points` (Task 2); a interação fica para o plano 3 |
| Decode, render, export, interface, keymap, autosave, Relink | Planos 2–4 |
