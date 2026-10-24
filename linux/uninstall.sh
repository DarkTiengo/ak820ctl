#!/usr/bin/env bash
# Remove o que o install.sh colocou. A configuração (~/.config/ak820ctl) fica, a não ser com --all.
set -uo pipefail
systemctl --user disable --now ak820-hyperhdr.service ak820-time.timer 2>/dev/null
rm -f "$HOME/.config/systemd/user/"ak820-{hyperhdr.service,time.service,time.timer}
systemctl --user daemon-reload
rm -f "$HOME/.local/bin/ak820ctl" "$HOME/.local/bin/ak820gui" \
      "$HOME/.local/share/applications/ak820gui.desktop"
rm -rf "$HOME/.local/share/doc/ak820ctl"
[ "${1:-}" = "--all" ] && rm -rf "$HOME/.config/ak820ctl"
sudo rm -f /etc/udev/rules.d/70-ak820.rules && sudo udevadm control --reload-rules
echo "ak820ctl removido"
