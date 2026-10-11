//! Atalhos de teclado: ações nomeadas e o mapa tecla → ação, editável e salvo em disco.

use std::collections::HashMap;
use std::path::Path;

use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    PlayPause,
    PlayForward,
    Stop,
    PlayReverse,
    StepBack,
    StepForward,
    PrevEdit,
    NextEdit,
    GoToStart,
    GoToEnd,
    MarkIn,
    MarkOut,
    ClearMarks,
    SelectionTool,
    BladeTool,
    ToggleSnapping,
    ToggleLinkedSelection,
    BladeAtPlayhead,
    Delete,
    RippleDelete,
    InsertEdit,
    OverwriteEdit,
    AppendEdit,
    LinkUnlink,
    SelectAll,
    Deselect,
    Undo,
    Redo,
    NewProject,
    Open,
    Save,
    SaveAs,
    Import,
    Deliver,
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    RippleTrimStart,
    RippleTrimEnd,
    Copy,
    Paste,
    AddMarker,
    PrevMarker,
    NextMarker,
    AddDissolve,
}

impl Action {
    pub const ALL: [Action; 45] = [
        Action::PlayPause,
        Action::PlayForward,
        Action::Stop,
        Action::PlayReverse,
        Action::StepBack,
        Action::StepForward,
        Action::PrevEdit,
        Action::NextEdit,
        Action::GoToStart,
        Action::GoToEnd,
        Action::MarkIn,
        Action::MarkOut,
        Action::ClearMarks,
        Action::SelectionTool,
        Action::BladeTool,
        Action::ToggleSnapping,
        Action::ToggleLinkedSelection,
        Action::BladeAtPlayhead,
        Action::Delete,
        Action::RippleDelete,
        Action::InsertEdit,
        Action::OverwriteEdit,
        Action::AppendEdit,
        Action::LinkUnlink,
        Action::SelectAll,
        Action::Deselect,
        Action::Undo,
        Action::Redo,
        Action::NewProject,
        Action::Open,
        Action::Save,
        Action::SaveAs,
        Action::Import,
        Action::Deliver,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomToFit,
        Action::RippleTrimStart,
        Action::RippleTrimEnd,
        Action::Copy,
        Action::Paste,
        Action::AddMarker,
        Action::PrevMarker,
        Action::NextMarker,
        Action::AddDissolve,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::PlayPause => "Tocar / pausar",
            Action::PlayForward => "Tocar para frente (L)",
            Action::Stop => "Parar (K)",
            Action::PlayReverse => "Tocar para trás (J)",
            Action::StepBack => "Voltar um frame",
            Action::StepForward => "Avançar um frame",
            Action::PrevEdit => "Corte anterior",
            Action::NextEdit => "Próximo corte",
            Action::GoToStart => "Ir para o início",
            Action::GoToEnd => "Ir para o fim",
            Action::MarkIn => "Marcar entrada (In)",
            Action::MarkOut => "Marcar saída (Out)",
            Action::ClearMarks => "Limpar In/Out",
            Action::SelectionTool => "Ferramenta de seleção",
            Action::BladeTool => "Ferramenta lâmina (Blade)",
            Action::ToggleSnapping => "Ligar/desligar snapping",
            Action::ToggleLinkedSelection => "Ligar/desligar seleção ligada",
            Action::BladeAtPlayhead => "Cortar no cursor",
            Action::Delete => "Apagar",
            Action::RippleDelete => "Apagar com ripple",
            Action::InsertEdit => "Insert",
            Action::OverwriteEdit => "Overwrite",
            Action::AppendEdit => "Append no fim",
            Action::LinkUnlink => "Ligar/separar áudio e vídeo",
            Action::SelectAll => "Selecionar tudo",
            Action::Deselect => "Tirar seleção",
            Action::Undo => "Desfazer",
            Action::Redo => "Refazer",
            Action::NewProject => "Novo projeto",
            Action::Open => "Abrir projeto",
            Action::Save => "Salvar",
            Action::SaveAs => "Salvar como",
            Action::Import => "Importar mídia",
            Action::Deliver => "Exportar (Deliver)",
            Action::ZoomIn => "Zoom +",
            Action::ZoomOut => "Zoom −",
            Action::ZoomToFit => "Zoom para caber",
            Action::RippleTrimStart => "Cortar do início até o cursor (ripple)",
            Action::RippleTrimEnd => "Cortar do cursor até o fim (ripple)",
            Action::Copy => "Copiar clipes",
            Action::Paste => "Colar no cursor",
            Action::AddMarker => "Adicionar marcador",
            Action::PrevMarker => "Marcador anterior",
            Action::NextMarker => "Próximo marcador",
            Action::AddDissolve => "Cross dissolve no corte",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyChord {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
    pub const fn ctrl(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }
    pub const fn shift(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    pub fn from_event(key: Key, m: Modifiers) -> Self {
        Self {
            key,
            ctrl: m.ctrl || m.command,
            shift: m.shift,
            alt: m.alt,
        }
    }

    /// Texto como "Ctrl+Shift+B".
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.name());
        s
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    bindings: Vec<(Action, Vec<KeyChord>)>,
}

impl Keymap {
    /// Atalhos parecidos com os do DaVinci Resolve.
    pub fn defaults() -> Self {
        use Action as A;
        use KeyChord as K;
        let b: Vec<(Action, Vec<KeyChord>)> = vec![
            (A::PlayPause, vec![K::new(Key::Space)]),
            (A::PlayForward, vec![K::new(Key::L)]),
            (A::Stop, vec![K::new(Key::K)]),
            (A::PlayReverse, vec![K::new(Key::J)]),
            (A::StepBack, vec![K::new(Key::ArrowLeft)]),
            (A::StepForward, vec![K::new(Key::ArrowRight)]),
            (A::PrevEdit, vec![K::new(Key::ArrowUp)]),
            (A::NextEdit, vec![K::new(Key::ArrowDown)]),
            (A::GoToStart, vec![K::new(Key::Home)]),
            (A::GoToEnd, vec![K::new(Key::End)]),
            (A::MarkIn, vec![K::new(Key::I)]),
            (A::MarkOut, vec![K::new(Key::O)]),
            (
                A::ClearMarks,
                vec![KeyChord {
                    key: Key::X,
                    ctrl: false,
                    shift: false,
                    alt: true,
                }],
            ),
            (A::SelectionTool, vec![K::new(Key::A)]),
            (A::BladeTool, vec![K::new(Key::B)]),
            (A::ToggleSnapping, vec![K::new(Key::N)]),
            (A::ToggleLinkedSelection, vec![K::ctrl_shift(Key::L)]),
            (
                A::BladeAtPlayhead,
                vec![K::ctrl(Key::B), K::ctrl(Key::Backslash)],
            ),
            (A::Delete, vec![K::new(Key::Backspace), K::new(Key::Delete)]),
            (
                A::RippleDelete,
                vec![K::shift(Key::Backspace), K::shift(Key::Delete)],
            ),
            (A::InsertEdit, vec![K::new(Key::F9)]),
            (A::OverwriteEdit, vec![K::new(Key::F10)]),
            (A::AppendEdit, vec![K::shift(Key::F12)]),
            (
                A::LinkUnlink,
                vec![KeyChord {
                    key: Key::L,
                    ctrl: true,
                    shift: false,
                    alt: true,
                }],
            ),
            (A::SelectAll, vec![K::ctrl(Key::A)]),
            (A::Deselect, vec![K::ctrl_shift(Key::A)]),
            (A::Undo, vec![K::ctrl(Key::Z)]),
            (A::Redo, vec![K::ctrl_shift(Key::Z), K::ctrl(Key::Y)]),
            (A::NewProject, vec![K::ctrl(Key::N)]),
            (A::Open, vec![K::ctrl(Key::O)]),
            (A::Save, vec![K::ctrl(Key::S)]),
            (A::SaveAs, vec![K::ctrl_shift(Key::S)]),
            (A::Import, vec![K::ctrl(Key::I)]),
            (A::Deliver, vec![K::ctrl_shift(Key::E)]),
            (A::ZoomIn, vec![K::ctrl(Key::Equals), K::ctrl(Key::Plus)]),
            (A::ZoomOut, vec![K::ctrl(Key::Minus)]),
            (A::ZoomToFit, vec![K::shift(Key::Z)]),
            (A::RippleTrimStart, vec![K::shift(Key::OpenBracket)]),
            (A::RippleTrimEnd, vec![K::shift(Key::CloseBracket)]),
            (A::Copy, vec![K::ctrl(Key::C)]),
            (A::Paste, vec![K::ctrl(Key::V)]),
            (A::AddMarker, vec![K::new(Key::M)]),
            (A::PrevMarker, vec![K::shift(Key::ArrowUp)]),
            (A::NextMarker, vec![K::shift(Key::ArrowDown)]),
            (A::AddDissolve, vec![K::ctrl(Key::T)]),
        ];
        Self { bindings: b }
    }

    pub fn chords(&self, action: Action) -> &[KeyChord] {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map_or(&[], |(_, c)| c.as_slice())
    }

    pub fn action_for(&self, chord: &KeyChord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, cs)| cs.contains(chord))
            .map(|(a, _)| *a)
    }

    /// Ação (diferente de `action`) que já usa `chord`.
    pub fn conflict(&self, chord: &KeyChord, action: Action) -> Option<Action> {
        self.action_for(chord).filter(|a| *a != action)
    }

    /// Define `chord` como o único atalho de `action`, tirando-o de quem o usava antes.
    pub fn set(&mut self, action: Action, chord: KeyChord) {
        for (_, cs) in &mut self.bindings {
            cs.retain(|c| *c != chord);
        }
        match self.bindings.iter_mut().find(|(a, _)| *a == action) {
            Some((_, cs)) => *cs = vec![chord],
            None => self.bindings.push((action, vec![chord])),
        }
    }

    pub fn clear(&mut self, action: Action) {
        if let Some((_, cs)) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
            cs.clear();
        }
    }

    /// Lê do disco; ações novas (que o arquivo não conhece) ficam com o padrão.
    pub fn load(path: &Path) -> Self {
        let mut map = Self::defaults();
        let Ok(text) = std::fs::read_to_string(path) else {
            return map;
        };
        let Ok(saved) = serde_json::from_str::<Keymap>(&text) else {
            return map;
        };
        let saved: HashMap<Action, Vec<KeyChord>> = saved.bindings.into_iter().collect();
        for (action, chords) in &mut map.bindings {
            if let Some(s) = saved.get(action) {
                chords.clone_from(s);
            }
        }
        map
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_default_and_defaults_do_not_clash() {
        let k = Keymap::defaults();
        let mut seen = std::collections::HashSet::new();
        for a in Action::ALL {
            assert!(!k.chords(a).is_empty(), "{a:?} sem atalho");
            for c in k.chords(a) {
                assert!(seen.insert(*c), "{} repetido", c.display());
            }
        }
    }

    #[test]
    fn resolve_style_defaults() {
        let k = Keymap::defaults();
        assert_eq!(
            k.action_for(&KeyChord::ctrl(Key::B)),
            Some(Action::BladeAtPlayhead)
        );
        assert_eq!(
            k.action_for(&KeyChord::shift(Key::Backspace)),
            Some(Action::RippleDelete)
        );
        assert_eq!(
            k.action_for(&KeyChord::new(Key::B)),
            Some(Action::BladeTool)
        );
        assert_eq!(
            k.action_for(&KeyChord::new(Key::F10)),
            Some(Action::OverwriteEdit)
        );
    }

    #[test]
    fn remapping_steals_the_key_and_reports_conflicts() {
        let mut k = Keymap::defaults();
        let space = KeyChord::new(Key::Space);
        assert_eq!(k.conflict(&space, Action::Stop), Some(Action::PlayPause));
        assert_eq!(k.conflict(&space, Action::PlayPause), None);
        k.set(Action::Stop, space);
        assert_eq!(k.action_for(&space), Some(Action::Stop));
        assert!(k.chords(Action::PlayPause).is_empty());
    }

    #[test]
    fn chord_display_and_modifiers() {
        let m = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        let c = KeyChord::from_event(Key::Z, m);
        assert_eq!(c, KeyChord::ctrl_shift(Key::Z));
        assert_eq!(c.display(), "Ctrl+Shift+Z");
    }

    #[test]
    fn save_and_load_keep_custom_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.json");
        let mut k = Keymap::defaults();
        k.set(Action::BladeAtPlayhead, KeyChord::new(Key::C));
        k.save(&path).unwrap();
        let loaded = Keymap::load(&path);
        assert_eq!(
            loaded.action_for(&KeyChord::new(Key::C)),
            Some(Action::BladeAtPlayhead)
        );
        assert_eq!(
            Keymap::load(&dir.path().join("nao-existe.json")),
            Keymap::defaults()
        );
    }
}
