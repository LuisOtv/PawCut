//! Onde o PawCut guarda configurações (atalhos, layout) e autosaves.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `$XDG_CONFIG_HOME/pawcut` ou `~/.config/pawcut`.
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("pawcut")
}

/// `$XDG_DATA_HOME/pawcut` ou `~/.local/share/pawcut`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("pawcut")
}

pub fn keymap_path() -> PathBuf {
    config_dir().join("keymap.json")
}

pub fn layout_path() -> PathBuf {
    config_dir().join("layout.json")
}

pub fn autosave_dir() -> PathBuf {
    data_dir().join("autosave")
}

/// Autosaves que sobraram de uma sessão que não fechou direito.
pub fn leftover_autosaves() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(autosave_dir()) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pawcut"))
        .collect();
    files.sort();
    files
}
