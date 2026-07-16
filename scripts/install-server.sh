#!/usr/bin/env bash
# r4a master node installer.
# Usage: curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-server.sh | sudo bash
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

echo "==> Downloading r4a-server, r4a-cli, r4a-tui (${T})..."
for bin in r4a-server r4a-cli r4a-tui; do
  curl -4 -fL --retry 3 --retry-delay 2 --connect-timeout 10 -o "$TMP/$bin" \
    "https://github.com/${REPO}/releases/latest/download/${bin}-${T}"
  chmod +x "$TMP/$bin"
done

echo "==> Installing binaries to /usr/local/bin..."
install -m 755 "$TMP/r4a-server" "$TMP/r4a-cli" "$TMP/r4a-tui" /usr/local/bin/

echo "==> Bootstrapping master node (WireGuard deps, secrets, service)..."
r4a-server install
