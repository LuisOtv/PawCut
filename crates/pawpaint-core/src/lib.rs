//! PawPaint: motor de pintura e edição de imagem (camadas de pixels em blocos, modos de mistura,
//! pincéis, seleções, filtros e transformações), sem nada de interface.

pub mod blend;
pub mod brush;
pub mod composite;
pub mod doc;
pub mod editor;
pub mod file;
pub mod fill;
pub mod filters;
pub mod selection;
pub mod text;
pub mod tile;
pub mod transform;

pub use blend::BlendMode;
pub use doc::{Doc, Layer, LayerId};

pub use editor::Editor;
pub use editor::{Background, Clip, Dirty, EditError, Floating, MoveTo};
pub use selection::Selection;
pub use tile::{TILE, TiledImage};
