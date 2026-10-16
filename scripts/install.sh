#!/usr/bin/env bash
# Compila os apps da suíte Paw em modo release e instala para o usuário atual:
#   ~/.local/bin/<app>, atalho no menu de aplicativos e ícone.
# Uso: scripts/install.sh            (todos)
#      scripts/install.sh pawvoice   (só um)
#      scripts/install.sh --uninstall
set -euo pipefail
cd "$(dirname "$0")/.."

# app|pacote|nome|descrição|categorias|mime
APPS=(
  "pawcut|pawcut-app|PawCut|Editor de vídeo simples|AudioVideo;Video;AudioVideoEditing;|video/mp4;video/quicktime;video/x-matroska;video/webm;"
  "pawvoice|pawvoice-app|PawVoice|Gravador e editor de voz|AudioVideo;Audio;Recorder;|audio/x-wav;audio/mpeg;audio/flac;"
  "pawwrite|pawwrite-app|PawWrite|Editor de roteiros|Office;TextEditor;|text/markdown;text/plain;"
  "pawpaint|pawpaint-app|PawPaint|Editor de imagens e pintura|Graphics;2DGraphics;RasterGraphics;|image/png;image/jpeg;image/webp;"
)

BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"

if [[ "${1:-}" == "--uninstall" ]]; then
    for entry in "${APPS[@]}"; do
        IFS='|' read -r app _ name _ _ _ <<<"$entry"
        rm -f "$BIN_DIR/$app" "$DESKTOP_DIR/$app.desktop" "$ICON_DIR/$app.svg"
        echo "$name removido."
    done
    exit 0
fi

command -v cargo >/dev/null || source "$HOME/.cargo/env"
mkdir -p "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"
for entry in "${APPS[@]}"; do
    IFS='|' read -r app pkg name comment categories mime <<<"$entry"
    if [[ -n "${1:-}" && "$1" != "$app" ]]; then
        continue
    fi
    cargo build --release -p "$pkg"
    install -m 755 "target/release/$app" "$BIN_DIR/$app"
    install -m 644 "assets/$app.svg" "$ICON_DIR/$app.svg"
    cat > "$DESKTOP_DIR/$app.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$name
Comment=$comment
Exec=$BIN_DIR/$app %F
Icon=$app
Terminal=false
Categories=$categories
MimeType=$mime
StartupWMClass=$app
DESKTOP
    echo "$name instalado: $BIN_DIR/$app (também aparece no menu de aplicativos)."
done
update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
