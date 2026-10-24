#!/usr/bin/env bash
# Instala o ak820ctl e a interface ak820gui para o usuário atual. Funciona no Bazzite e em outras
# distros atômicas: não usa rpm-ostree.
#   binários        -> ~/.local/bin
#   atalho no menu  -> ~/.local/share/applications
#   serviços        -> ~/.config/systemd/user   (systemd --user)
#   configuração    -> ~/.config/ak820ctl/ak820.env   (não é sobrescrita se já existir)
#   regra udev      -> /etc/udev/rules.d/70-ak820.rules   (pede a senha do sudo)
#
# Uso:  bash install.sh                 instala e liga o relógio automático
#       bash install.sh --hyperhdr      também liga a sincronização com o HyperHDR
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
with_hyperhdr=0
for a in "$@"; do
  case "$a" in
    --hyperhdr) with_hyperhdr=1 ;;
    *) echo "opção desconhecida: $a"; exit 1 ;;
  esac
done

for b in ak820ctl ak820gui; do
  if [ -f "$here/$b" ]; then
    install -Dm755 "$here/$b" "$HOME/.local/bin/$b"
    echo "instalado: ~/.local/bin/$b"
  fi
done
[ -x "$HOME/.local/bin/ak820ctl" ] || { echo "ak820ctl não encontrado ao lado deste script"; exit 1; }

if [ -x "$HOME/.local/bin/ak820gui" ]; then
  sed "s|^Exec=.*|Exec=$HOME/.local/bin/ak820gui|" "$here/ak820gui.desktop" \
    | install -Dm644 /dev/stdin "$HOME/.local/share/applications/ak820gui.desktop"
  echo "atalho no menu: AK820 Pro"
fi
[ -f "$here/README.md" ] && install -Dm644 "$here/README.md" "$HOME/.local/share/doc/ak820ctl/README.md"

conf="$HOME/.config/ak820ctl/ak820.env"
if [ ! -f "$conf" ]; then
  install -Dm644 "$here/ak820.env" "$conf"
  echo "configuração criada: $conf"
else
  echo "configuração mantida: $conf"
fi

units="$HOME/.config/systemd/user"
mkdir -p "$units"
for u in ak820-hyperhdr.service ak820-time.service ak820-time.timer; do
  install -Dm644 "$here/systemd/$u" "$units/$u"
done
systemctl --user daemon-reload
systemctl --user enable --now ak820-time.timer
echo "relógio automático ligado (ao entrar e a cada hora)"
if [ "$with_hyperhdr" = 1 ]; then
  systemctl --user enable --now ak820-hyperhdr.service
  echo "sincronização com o HyperHDR ligada (endereço em $conf)"
else
  echo "para ligar a sincronização com o HyperHDR:  systemctl --user enable --now ak820-hyperhdr.service"
fi

sudo install -Dm644 "$here/70-ak820.rules" /etc/udev/rules.d/70-ak820.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=hidraw
echo "regra udev instalada; se o teclado não aparecer, reconecte o dongle ou o cabo."

command -v ffmpeg >/dev/null || echo "aviso: ffmpeg não encontrado; vídeos na tela não vão funcionar até instalá-lo"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "adicione ~/.local/bin ao PATH";; esac
"$HOME/.local/bin/ak820ctl" list || true
