#!/usr/bin/env bash
# Instala o Nemux no Linux: os binários em /opt/nemux, um atalho no menu de
# aplicativos e o comando `nemux` no PATH.
#
# Uso (da raiz do repositório, sem sudo: a senha é pedida só para copiar):
#   scripts/install-linux.sh              compila em release e instala
#   scripts/install-linux.sh --no-build   instala o que já está em target/release
#   scripts/install-linux.sh --uninstall  remove o que este script instalou
#
# Os destinos podem ser trocados (útil para testar sem root):
#   NEMUX_INSTALL_DIR (padrão /opt/nemux), NEMUX_BIN_DIR (/usr/local/bin),
#   NEMUX_SHARE_DIR (/usr/share: atalho em applications/, ícone em icons/).
set -euo pipefail

INSTALL_DIR="${NEMUX_INSTALL_DIR:-/opt/nemux}"
BIN_DIR="${NEMUX_BIN_DIR:-/usr/local/bin}"
SHARE_DIR="${NEMUX_SHARE_DIR:-/usr/share}"
DESKTOP_FILE="$SHARE_DIR/applications/nemux.desktop"
ICON_FILE="$SHARE_DIR/icons/hicolor/scalable/apps/nemux.svg"
LINK="$BIN_DIR/nemux"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE="$REPO/target/release"

build=1
uninstall=0
for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        --uninstall) uninstall=1 ;;
        -h | --help)
            sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "Opção desconhecida: $arg (veja --help)" >&2
            exit 2
            ;;
    esac
done

# Roda como root só quando o destino não é gravável pelo usuário.
as_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        sudo "$@"
    fi
}
writable() { # o diretório (ou o primeiro pai que existe) é gravável?
    local dir="$1"
    while [ ! -d "$dir" ]; do dir="$(dirname "$dir")"; done
    [ -w "$dir" ]
}
run_in() { # run_in <diretório de destino> <comando...>
    local dir="$1"
    shift
    if writable "$dir"; then "$@"; else as_root "$@"; fi
}

refresh_menu() {
    if command -v update-desktop-database >/dev/null; then
        run_in "$SHARE_DIR/applications" update-desktop-database -q "$SHARE_DIR/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null && [ -f "$SHARE_DIR/icons/hicolor/index.theme" ]; then
        run_in "$SHARE_DIR/icons/hicolor" gtk-update-icon-cache -q -t "$SHARE_DIR/icons/hicolor" || true
    fi
}

if [ "$uninstall" -eq 1 ]; then
    # Só apaga a pasta se ela for mesmo a do Nemux.
    if [ -x "$INSTALL_DIR/nemux-app" ]; then
        run_in "$(dirname "$INSTALL_DIR")" rm -rf "$INSTALL_DIR"
        echo "Removido: $INSTALL_DIR"
    fi
    if [ -L "$LINK" ] && [ "$(readlink "$LINK")" = "$INSTALL_DIR/nemux-app" ]; then
        run_in "$BIN_DIR" rm -f "$LINK"
        echo "Removido: $LINK"
    fi
    for file in "$DESKTOP_FILE" "$ICON_FILE"; do
        if [ -f "$file" ]; then
            run_in "$(dirname "$file")" rm -f "$file"
            echo "Removido: $file"
        fi
    done
    refresh_menu
    echo "Os seus dados ficam onde estavam: ~/.config/nemux e ~/.local/share/nemux."
    exit 0
fi

if [ "$build" -eq 1 ]; then
    if [ "$(id -u)" -eq 0 ]; then
        echo "Rode sem sudo: compilar como root deixaria target/ com dono root." >&2
        echo "O script pede a senha só na hora de copiar." >&2
        exit 1
    fi
    echo "Compilando em release..."
    cargo build --release --manifest-path "$REPO/Cargo.toml" -p nemux-app -p nemux-mcp
fi

for exe in nemux-app nemux-mcp; do
    if [ ! -x "$RELEASE/$exe" ]; then
        echo "Falta $RELEASE/$exe: rode sem --no-build." >&2
        exit 1
    fi
done

# Os binários vão para /opt/nemux lado a lado: "Uso do Claude na barra" procura o
# nemux-mcp junto do nemux-app. Copia para um arquivo novo e troca de nome, para
# não esbarrar num nemux-mcp em uso por alguma sessão do Claude.
echo "Instalando em $INSTALL_DIR..."
run_in "$INSTALL_DIR" mkdir -p "$INSTALL_DIR"
for exe in nemux-app nemux-mcp; do
    run_in "$INSTALL_DIR" install -m 755 "$RELEASE/$exe" "$INSTALL_DIR/.$exe.novo"
    run_in "$INSTALL_DIR" mv -f "$INSTALL_DIR/.$exe.novo" "$INSTALL_DIR/$exe"
done
run_in "$INSTALL_DIR" install -m 644 "$REPO/app/ui/icons/nemux.svg" "$INSTALL_DIR/nemux.svg"

run_in "$BIN_DIR" mkdir -p "$BIN_DIR"
run_in "$BIN_DIR" ln -sfn "$INSTALL_DIR/nemux-app" "$LINK"

run_in "$(dirname "$ICON_FILE")" mkdir -p "$(dirname "$ICON_FILE")"
run_in "$(dirname "$ICON_FILE")" install -m 644 "$REPO/app/ui/icons/nemux.svg" "$ICON_FILE"

# StartupWMClass é o WM_CLASS da janela: liga a janela aberta ao atalho (ícone
# certo na barra de tarefas).
desktop="$(mktemp --suffix=.desktop)" # o validador exige a extensão
trap 'rm -f "$desktop"' EXIT
cat >"$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Nemux
GenericName=Terminal
Comment=Terminais em abas com uma memória que as sessões do Claude Code compartilham
Exec=$INSTALL_DIR/nemux-app
Icon=nemux
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;claude;
StartupWMClass=nemux-app
StartupNotify=true
EOF
if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$desktop"
fi
run_in "$(dirname "$DESKTOP_FILE")" mkdir -p "$(dirname "$DESKTOP_FILE")"
run_in "$(dirname "$DESKTOP_FILE")" install -m 644 "$desktop" "$DESKTOP_FILE"
refresh_menu

cat <<EOF

Nemux instalado:
  binários   $INSTALL_DIR
  comando    $LINK
  menu       $DESKTOP_FILE

Para a memória no Claude Code (uma vez, vale para todos os projetos):
  claude mcp add --scope user nemux -- $INSTALL_DIR/nemux-mcp
"Uso do Claude na barra" (engrenagem) já acha o nemux-mcp de $INSTALL_DIR; se
ligou antes por outro caminho, desmarque e marque de novo.
EOF
