//! Interface compartilhada dos apps Paw: tema escuro no estilo do Resolve, ícones desenhados
//! à mão, avisos (toasts) e backups versionados.

pub mod backups;
pub mod icons;
pub mod radial;
pub mod slider;
pub mod theme;
pub mod toasts;

pub use slider::Slider;
