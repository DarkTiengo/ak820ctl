#!/usr/bin/env bash
# Compila o ak820ctl e o ak820gui no Bazzite (ou em qualquer distro com distrobox/podman) sem
# mexer no sistema: a compilação roda num container Fedora e só os binários saem dele.
# O resultado vai para dist-linux/ e para ak820ctl-linux-x86_64.tar.gz.
# Uso, na pasta ak820ctl:   bash linux/build-bazzite.sh
set -euo pipefail
cd "$(dirname "$0")/.."
box=ak820-build

if ! command -v distrobox >/dev/null; then
  echo "distrobox não encontrado (no Bazzite ele já vem instalado)"; exit 1
fi

if ! distrobox list | grep -qw "$box"; then
  echo "== criando o container $box (Fedora), só na primeira vez"
  distrobox create --yes --name "$box" --image registry.fedoraproject.org/fedora-toolbox:latest
fi

echo "== compilando dentro do container"
distrobox enter "$box" -- bash -euo pipefail -c '
  if ! command -v cargo >/dev/null || ! rpm -q systemd-devel >/dev/null 2>&1; then
    sudo dnf install -y cargo rust gcc pkgconf-pkg-config systemd-devel \
      libxkbcommon-devel wayland-devel mesa-libGL-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel
  fi
  cargo test --release
  cargo build --release
'

out=dist-linux
rm -rf "$out"
mkdir -p "$out/systemd"
cp target/release/ak820ctl target/release/ak820gui README.md "$out/"
cp linux/70-ak820.rules linux/install.sh linux/uninstall.sh linux/ak820gui.desktop linux/ak820.env "$out/"
cp linux/systemd/* "$out/systemd/"
tar -czf ak820ctl-linux-x86_64.tar.gz -C "$out" .
echo "== pronto: $out/ e ak820ctl-linux-x86_64.tar.gz"
echo "Para instalar (fora do container):  bash $out/install.sh            (ou --hyperhdr)"
