#!/usr/bin/env bash
# r4a-cli installer (management + VPN client, no server/agent role).
# Usage: curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-cli.sh | sudo bash
set -euo pipefail

REPO="rockxi/rust4eska"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  T=x86_64-linux-musl ;;
  Darwin-x86_64) T=x86_64-macos ;;
  Darwin-arm64)  T=aarch64-macos ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Downloading r4a-cli (${T})..."
curl -4 -fL --retry 3 --retry-delay 2 --connect-timeout 10 -o "$TMP/r4a-cli" \
  "https://github.com/${REPO}/releases/latest/download/r4a-cli-${T}"
chmod +x "$TMP/r4a-cli"

echo "==> Installing binary to /usr/local/bin..."
install -m 755 "$TMP/r4a-cli" /usr/local/bin/

echo "==> Done. Try: r4a-cli --master http://<master-ip>:3501 --secret <admin-secret> nodes list"
