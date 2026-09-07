#!/usr/bin/env bash
# Gera o tarball portatil do AMS YouTube Downloader para Linux.
# Uso: packaging/build-tarball.sh [--skip-build]
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
NAME="AMS_YT_Downloader-${VERSION}-linux-${ARCH}"
STAGE="target/tarball/${NAME}"

if [[ "${1:-}" != "--skip-build" ]]; then
    cargo build --release
fi

rm -rf "$STAGE"
mkdir -p "$STAGE"

install -m 755 target/release/ams-yt-dw            "$STAGE/ams-yt-dw"
install -m 644 packaging/ams-yt-dw.desktop         "$STAGE/ams-yt-dw.desktop"
install -m 644 assets/icon_window.png              "$STAGE/ams-yt-dw.png"
install -m 644 README.md                           "$STAGE/README.md"

cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Instala em ~/.local (nao precisa de root). Desinstalar: ./install.sh --uninstall
set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$BIN/ams-yt-dw" "$APPS/ams-yt-dw.desktop" "$ICONS/ams-yt-dw.png"
    command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
    echo "Removido."
    exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS"
install -m 755 ams-yt-dw        "$BIN/ams-yt-dw"
install -m 644 ams-yt-dw.png    "$ICONS/ams-yt-dw.png"
# O Exec precisa do caminho absoluto: ~/.local/bin nem sempre esta no PATH das
# sessoes graficas, e o compositor nao le o seu .bashrc.
sed "s|^Exec=ams-yt-dw|Exec=$BIN/ams-yt-dw|" ams-yt-dw.desktop > "$APPS/ams-yt-dw.desktop"
chmod 644 "$APPS/ams-yt-dw.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache  >/dev/null && gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Instalado em $BIN/ams-yt-dw"
echo
MISSING=""
command -v yt-dlp >/dev/null || MISSING="$MISSING yt-dlp"
command -v ffmpeg >/dev/null || MISSING="$MISSING ffmpeg"
if [[ -n "$MISSING" ]]; then
    echo "ATENCAO: faltando no PATH:$MISSING"
    echo "  Fedora : sudo dnf install$MISSING"
    echo "  Debian : sudo apt install$MISSING"
    echo "  (yt-dlp atualizado: pipx install yt-dlp)"
else
    echo "yt-dlp e ffmpeg encontrados no PATH."
fi
INSTALL
chmod 755 "$STAGE/install.sh"

tar -czf "target/tarball/${NAME}.tar.gz" -C target/tarball "$NAME"
echo "target/tarball/${NAME}.tar.gz"
