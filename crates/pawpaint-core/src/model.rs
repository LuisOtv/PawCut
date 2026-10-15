use serde::{Deserialize, Serialize};

/// Tamanho padrão de thumbnail do YouTube.
pub const THUMB_W: u32 = 1280;
pub const THUMB_H: u32 = 720;
pub const DOC_VERSION: u32 = 1;

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

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);

    pub fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Background {
    Solid(Rgba),
    /// Degradê linear; `angle` em graus (0 = de cima para baixo, 90 = da esquerda para a direita).
    Gradient {
        from: Rgba,
        to: Rgba,
        angle: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    /// Altura da fonte em pixels da tela, com a camada em escala 1.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: Rgba,
    pub align: TextAlign,
    /// Espaço entre linhas (1.0 = padrão da fonte).
    pub line_spacing: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rect,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeStyle {
    pub kind: ShapeKind,
    pub w: f32,
    pub h: f32,
    pub fill: Rgba,
    pub stroke_width: f32,
    pub stroke: Rgba,
    /// Cantos arredondados do retângulo (em pixels).
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// Imagem guardada nos recursos do projeto, pelo nome.
    Image {
        asset: String,
    },
    Text(TextStyle),
    Shape(ShapeStyle),
}

/// Contorno de adesivo em volta de tudo que a camada desenha.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub width: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    /// Cor e força (alfa) da sombra.
    pub color: Rgba,
}

/// Tira uma cor da imagem (fundo verde, fundo branco…).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Chroma {
    pub color: Rgba,
    /// 0..1: quanto de diferença ainda conta como a cor.
    pub tolerance: f32,
    /// 0..1: suavidade da borda.
    pub softness: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adjust {
    /// −1..1
    pub brightness: f32,
    /// −1..1
    pub contrast: f32,
    /// −1..1 (−1 = preto e branco)
    pub saturation: f32,
    pub chroma: Option<Chroma>,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            chroma: None,
        }
    }
}

impl Adjust {
    pub fn is_neutral(&self) -> bool {
        *self == Adjust::default()
    }
}

pub type LayerId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// 0..1
    pub opacity: f32,
    /// Centro da camada na tela, em pixels.
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    /// Graus, sentido horário.
    pub rotation: f32,
    pub flip_h: bool,
    pub kind: Kind,
    pub outline: Option<Outline>,
    pub shadow: Option<Shadow>,
    pub adjust: Adjust,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>, kind: Kind, x: f32, y: f32) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            x,
            y,
            scale: 1.0,
            rotation: 0.0,
            flip_h: false,
            kind,
            outline: None,
            shadow: None,
            adjust: Adjust::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub background: Background,
    /// De baixo para cima (a última camada fica por cima).
    pub layers: Vec<Layer>,
    pub(crate) next_id: u64,
}

impl Doc {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            version: DOC_VERSION,
            width,
            height,
            background: Background::Gradient {
                from: Rgba::rgb(0x1d, 0x1d, 0x2e),
                to: Rgba::rgb(0x4a, 0x2a, 0x6a),
                angle: 135.0,
            },
            layers: Vec::new(),
            next_id: 1,
        }
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub(crate) fn fresh(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            text: "TEXTO".into(),
            font: "sans-serif".into(),
            size: 150.0,
            bold: true,
            italic: false,
            color: Rgba::WHITE,
            align: TextAlign::Center,
            line_spacing: 1.0,
        }
    }
}

impl ShapeStyle {
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            kind,
            w: 400.0,
            h: 240.0,
            fill: Rgba::rgb(0xe8, 0x6a, 0x35),
            stroke_width: 0.0,
            stroke: Rgba::WHITE,
            radius: 0.0,
        }
    }
}
