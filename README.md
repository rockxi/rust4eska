# rust4eska (r4a)

[Русская версия](README.ru.md)

A lightweight, self-contained cluster management system written in Rust. One master, any number of agents, connected by a built-in WireGuard mesh VPN — no Nginx, no external VPN, no external database, no Docker Registry. Everything ships as static binaries.

## Features

- **Built-in VPN** — automatic WireGuard mesh (`10.42.0.0/16`) between master and agents, with direct P2P links between agents when NAT allows and automatic relay through the master when it doesn't.
- **Built-in DNS** — the master serves `*.r4a.local` names over the VPN, no `/etc/hosts` editing.
- **Edge routing** — Pingora-based ingress routes `<app>.<node>.r4a.local` to containers on any node.
- **Workloads** — declarative TOML manifests reconciled to Docker containers on agents.
- **Git & Registry** — built-in bare Git hosting and OCI registry.
- **Vault & RBAC** — encrypted secret store (`vault://` refs in manifests), tokens and policies.
- **Dashboards** — terminal UI (`r4a-tui`) and a React Web UI (`r4a-web`).
- **Cluster updates** — one keypress in the TUI updates signed binaries across the whole cluster.

## Installing binaries

Every role below starts by downloading the binaries it needs from [GitHub Releases](https://github.com/rockxi/rust4eska/releases), for Linux (x86_64, static musl) and macOS (x86_64 / arm64):

```bash
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  T=x86_64-linux-musl ;;
  Darwin-x86_64) T=x86_64-macos ;;
  Darwin-arm64)  T=aarch64-macos ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
for bin in r4a-server r4a-agent r4a-cli r4a-tui; do
  curl -fL -o "$bin" "https://github.com/rockxi/rust4eska/releases/latest/download/${bin}-${T}"
  chmod +x "$bin"
done
sudo install -m 755 r4a-server r4a-agent r4a-cli r4a-tui /usr/local/bin/
```

You don't need all four binaries on every machine — see which ones each role below actually uses.

General requirements (Linux and macOS): WireGuard support (any modern Linux kernel, or `wireguard-go` on macOS) + `wireguard-tools`, `iproute2`, `iptables` on Linux; root access (VPN interface setup); Docker only on nodes that will run workloads.

Package installation:

```bash
# Ubuntu/Debian
sudo apt update && sudo apt install -y wireguard-tools iproute2 iptables

# macOS (Homebrew)
brew install wireguard-tools wireguard-go
```

## Setting up the master node

The master runs `r4a-server`: it holds the WireGuard mesh (`10.42.0.0/16`), the control API, DNS for `*.r4a.local`, ingress and cluster state. Needs `r4a-server` (plus `r4a-cli` / `r4a-tui` for management) from [Installing binaries](#installing-binaries).

These ports must be reachable from outside the master:

| Port | Protocol | Purpose |
|------|----------|---------|
| `51820` | UDP | WireGuard (must be open / port-forwarded — critical) |
| `3501` | TCP | Control API (only `/` and `/api/join` are served to non-VPN IPs) |

If the master is behind a home router, forward `51820/udp` (and `3501/tcp`) to it. Allow the same ports through `iptables`/`ufw`/`firewalld` on Linux, or through Firewall in System Settings on macOS, if enabled.

One-liner install (downloads the binaries, installs WireGuard deps via apt/brew, generates secrets, runs as a service):

```bash
curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-server.sh | sudo bash
```

Secrets are printed once at the end — save them. Or step by step, with full control:

```bash
sudo -E r4a-server install
```

Or fully manual:

```bash
export R4A_SECRET=$(openssl rand -hex 16)         # cluster join secret — share it with agents/clients
export R4A_ADMIN_SECRET=$(openssl rand -hex 16)   # admin secret — for CLI/TUI/Web UI management (keep private)
echo "cluster secret: $R4A_SECRET"; echo "admin secret: $R4A_ADMIN_SECRET"

# If behind NAT, tell agents your public endpoint:
export R4A_PUBLIC_ENDPOINT=<your-public-ip>:51820

sudo -E r4a-server init          # foreground, good for the first test
# or install as a systemd (Linux) / launchd (macOS) service, using the same env vars:
sudo -E r4a-server service enable
```

The master takes VPN IP `10.42.0.1`. State lives in `~/.r4a-server/`.

Verify:

```bash
r4a-cli --master http://10.42.0.1:3501 --secret <admin-secret> nodes list
R4A_MASTER=http://10.42.0.1:3501 R4A_SECRET=<admin-secret> r4a-tui   # dashboard; "P2P" column shows direct links
```

## Setting up an agent node

An agent joins an existing master and can run workloads (Docker containers reconciled from TOML manifests). Needs `r4a-agent` (plus Docker) from [Installing binaries](#installing-binaries), and a master already running.

One-liner install (downloads the binary, installs WireGuard deps via apt/brew, joins the master as a service):

```bash
curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-agent.sh | sudo bash -s -- \
  --master http://<master-public-ip>:3501 --secret <cluster-secret> --name friend1
```

Or fully manual:

```bash
sudo r4a-agent connect \
  --master http://<master-public-ip>:3501 \
  --secret <cluster-secret> \
  --name friend1
# permanent (systemd on Linux / launchd on macOS):
sudo r4a-agent service enable --master http://<master-public-ip>:3501 --secret <cluster-secret> --name friend1
```

Verify (from the agent, or from any machine already on the VPN):

```bash
ping 10.42.0.1                      # master over VPN
r4a-cli --master http://10.42.0.1:3501 --secret <admin-secret> nodes list   # agent should be listed
```

## Connecting as a VPN client only

Use this if you just want access to an existing cluster (ping nodes, reach `*.r4a.local`, use the API/TUI) without running any workloads. Needs only `r4a-cli` (add `r4a-tui` for the dashboard) from [Installing binaries](#installing-binaries), and a master already running.

```bash
export R4A_MASTER=http://<master-public-ip>:3501
export R4A_SECRET=<cluster-secret>
sudo -E r4a-cli connect up --label my-laptop
r4a-cli connect status
```

Verify:

```bash
ping 10.42.0.1                      # master over VPN
r4a-cli --master http://10.42.0.1:3501 --secret <admin-secret> nodes list
```

Disconnect: `r4a-cli connect down` (or `r4a-cli connect cleanup` if something is left over after a failed disconnect).

If something breaks, see [Troubleshooting](#troubleshooting).

## Deploying the frontend and workloads

### Web UI

Run on the master (Linux or macOS):

```bash
r4a-web --port 3502
```

Open `http://10.42.0.1:3502` and log in with the admin secret. Use it to create/edit workload manifests visually, or manage nodes, secrets and RBAC.

### Deploying a workload via the API

Workloads are described by TOML manifests (see `postgres.toml` for an example) and reconciled into Docker containers on the agents.

```bash
# exchange the admin secret for a bearer token, then POST the manifest:
TOKEN=$(curl -s -X POST http://10.42.0.1:3501/api/tokens/exchange \
  -H "X-R4A-Secret: <admin-secret>" | jq -r .id)
curl -X POST http://10.42.0.1:3501/api/manifests \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" -d @manifest.json
r4a-cli manifests list        # view deployed manifests
```

The app becomes reachable at `<app>.<node>.r4a.local` through the built-in ingress and DNS (VPN members only).

## Development cluster (Docker Compose)

Simulates 1 master + 2 agents locally:

```bash
make dev-up        # build & start
make dev-deploy    # recompile and sync binaries into running containers
make dev-down
```

- Web UI: `http://localhost:3502` — login with admin secret `test_admin_secret_456`
- API: `http://localhost:3501`, ingress: `http://localhost:3500`
- TUI: `R4A_MASTER=http://localhost:3501 R4A_SECRET=test_admin_secret_456 r4a-tui` or `docker exec -it node-master r4a-tui`

Requirements: Rust stable, Node.js (frontend), Docker, musl target (`rustup target add x86_64-unknown-linux-musl`).

## Cluster updates

1. Open `r4a-tui` → **Update** tab → press `u` for a cluster-wide update.
2. Binaries are ed25519-signature checked. Self-built binaries won't pass the official key — set `R4A_SKIP_SIGNATURE_VERIFY=1` on agents (dev/test only).

## Ports & environment reference

| Port | Where | Purpose |
|------|-------|---------|
| 51820/udp | master & agents | WireGuard |
| 3501 | master | Control API (VPN-only except `/api/join`) |
| 3500 | master | Ingress (Pingora) |
| 3502 | master | Web UI (`r4a-web`) |
| 443 | master VPN IP | HTTPS proxy (VPN-only) |
| 53 | master VPN IP | DNS for `*.r4a.local` (VPN-only) |
| 8082 | agent VPN IP | Agent API (VPN-only) |

| Variable | Purpose |
|----------|---------|
| `R4A_SECRET` | Cluster join secret (required to join; auto-generated on master if unset — see `~/.r4a-server/identity.json`) |
| `R4A_ADMIN_SECRET` | Admin secret — exchanged for a management token (CLI/TUI/Web UI) |
| `R4A_PUBLIC_ENDPOINT` | Publicly reachable `host:51820` — required behind NAT (master and optionally agents) |
| `R4A_MASTER` | Master API URL for CLI/TUI (default `http://master.r4a.local:3501`) |
| `R4A_TOKEN` | RBAC bearer token (alternative to secret) |
| `R4A_SKIP_SIGNATURE_VERIFY` | `1` = skip release signature check (dev only) |

## Troubleshooting

- **Agent joins but no ping over VPN** — `51820/udp` is not reachable. Check the port-forward on the master's router and set `R4A_PUBLIC_ENDPOINT` before starting the master.
- **P2P column shows relay, not direct** — both peers are behind restrictive NATs; traffic falls back to relaying through the master automatically. Direct P2P across two different NATs is known to be unreliable ([known issue](#known-limitations)).
- **`*.r4a.local` doesn't resolve** — DNS is served only over the VPN (`10.42.0.1:53`). Use the VPN IP directly (`http://10.42.0.1:3501`) if your OS didn't pick up the resolver.
- **API returns 403 from outside** — by design: everything except `/` and `/api/join` is VPN-only.
- **Leftover interfaces/DNS after a failed disconnect** — `r4a-cli connect cleanup`.

## Known limitations (MVP)

- Direct P2P between two agents that are each behind a different NAT may not establish; relay via master is used instead.
- Multi-master sync is HTTP push based, not Raft consensus yet.
- The release signing key is a placeholder; signature verification matters only for the built-in update flow.

## Project structure

- `binaries/` — `r4a-server` (master), `r4a-agent`, `r4a-cli`, `r4a-tui`, `r4a-web` (embedded React SPA)
- `crates/` — `r4a-core` (types/crypto), `r4a-vpn` (WireGuard+DNS), `r4a-store` (Sled+sync+vault+RBAC), `r4a-ingress` (Pingora), `r4a-git-registry`, `r4a-worker` (Docker reconciler), `r4a-service`, `r4a-telemetry`, `r4a-client`

## License

MIT / Apache-2.0
