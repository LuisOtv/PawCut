#!/usr/bin/env bash
# Compila o PawCut em modo release e instala para o usuário atual:
#   ~/.local/bin/pawcut, atalho no menu de aplicativos e ícone.
# Para desinstalar: scripts/install.sh --uninstall
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="$HOME/.local/bin/pawcut"
DESKTOP="$HOME/.local/share/applications/pawcut.desktop"
ICON="$HOME/.local/share/icons/hicolor/scalable/apps/pawcut.svg"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    echo "PawCut removido."
    exit 0
fi

command -v cargo >/dev/null || source "$HOME/.cargo/env"
cargo build --release -p pawcut-app
mkdir -p "$(dirname "$BIN")" "$(dirname "$DESKTOP")" "$(dirname "$ICON")"
install -m 755 target/release/pawcut "$BIN"
install -m 644 assets/pawcut.svg "$ICON"
cat > "$DESKTOP" <<DESKTOP
[Desktop Entry]
Type=Application
Name=PawCut
Comment=Editor de vídeo simples
Exec=$BIN %F
Icon=pawcut
Terminal=false
Categories=AudioVideo;Video;AudioVideoEditing;
MimeType=video/mp4;video/quicktime;video/x-matroska;video/webm;
StartupWMClass=pawcut
DESKTOP
update-desktop-database "$(dirname "$DESKTOP")" 2>/dev/null || true
echo "PawCut instalado: $BIN (também aparece no menu de aplicativos)."
