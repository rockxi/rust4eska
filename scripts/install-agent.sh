#!/usr/bin/env bash
# r4a agent node installer.
# Usage: curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-agent.sh | sudo bash -s -- --master http://<master-ip>:3501 --secret <cluster-secret> [--name <node-name>]
set -euo pipefail

REPO="rockxi/rust4eska"
MASTER="${R4A_MASTER:-}"
SECRET="${R4A_SECRET:-}"
NAME="${R4A_NODE_NAME:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --master) MASTER="$2"; shift 2 ;;
    --secret) SECRET="$2"; shift 2 ;;
    --name)   NAME="$2";   shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$MASTER" || -z "$SECRET" ]]; then
  echo "usage: curl -fsSL .../install-agent.sh | sudo bash -s -- --master http://<master-ip>:3501 --secret <cluster-secret> [--name <node-name>]" >&2
  exit 1
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  T=x86_64-linux-musl ;;
  Darwin-x86_64) T=x86_64-macos ;;
  Darwin-arm64)  T=aarch64-macos ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Downloading r4a-agent (${T})..."
curl -fL -o "$TMP/r4a-agent" "https://github.com/${REPO}/releases/latest/download/r4a-agent-${T}"
chmod +x "$TMP/r4a-agent"

echo "==> Installing binary to /usr/local/bin..."
install -m 755 "$TMP/r4a-agent" /usr/local/bin/

if [[ "$(uname -s)" == "Linux" ]]; then
  echo "==> Installing WireGuard dependencies via apt-get..."
  apt-get update && apt-get install -y wireguard-tools iproute2 iptables || \
    echo "apt-get install failed — install wireguard-tools manually" >&2
elif [[ "$(uname -s)" == "Darwin" ]]; then
  echo "==> Installing WireGuard dependencies via brew..."
  brew install wireguard-tools wireguard-go || \
    echo "brew install failed — install wireguard-tools manually" >&2
fi

NAME_ARGS=()
[[ -n "$NAME" ]] && NAME_ARGS=(--name "$NAME")

echo "==> Starting r4a-agent as a service..."
r4a-agent service enable --master "$MASTER" --secret "$SECRET" "${NAME_ARGS[@]}"
