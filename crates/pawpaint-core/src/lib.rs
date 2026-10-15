//! PawPaint: o documento de uma thumbnail (tela, camadas e efeitos) e as edições sobre ele.

mod editor;
mod file;
mod model;

pub use editor::{Editor, MoveTo};
pub use file::{FileError, load_project, save_project};
pub use model::*;
