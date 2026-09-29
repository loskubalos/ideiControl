#!/usr/bin/env bash
# Install ideiMx HID udev rule (AppImage / source builds — .deb installs this automatically).
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RULE_SRC="${SCRIPT_DIR}/99-ideimx.rules"
RULE_DST="/etc/udev/rules.d/99-ideimx.rules"

if [[ ! -f "$RULE_SRC" ]]; then
  echo "Missing $RULE_SRC" >&2
  exit 1
fi

echo "Installing $RULE_DST (requires sudo)…"
sudo install -m 644 "$RULE_SRC" "$RULE_DST"
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=hidraw
echo "Done. Unplug and replug the ideiMx device if it was already connected."
