use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(
    /// Arquivo importado no Media Pool.
    MediaId
);
id_type!(
    /// Faixa de vídeo ou áudio.
    TrackId
);
id_type!(
    /// Clipe na timeline.
    ClipId
);
id_type!(
    /// Grupo de clipes ligados (ex.: vídeo + o seu áudio).
    LinkId
);
