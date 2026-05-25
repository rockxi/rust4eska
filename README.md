# rust4eska (r4a)

A lightweight, high-performance cluster management system written in Rust. Features internal WireGuard VPN, distributed state via Raft/Sled, and edge routing with Pingora.

## Features

- **Built-in VPN**: Automatic WireGuard mesh setup between master and agents.
- **Distributed Store**: Consensus-backed configuration and manifest storage.
- **Edge Routing**: High-performance HTTP/gRPC proxying using Pingora.
- **Git & Registry**: Built-in bare Git repository hosting and OCI-compliant registry.
- **Unified Dashboards**: Manage nodes, manifests, and observability via TUI or a modern Web UI.
- **Zero External Dependencies**: No Nginx, Docker Registry, or external VPN software required.

## Quick Start (Development)

To test the system locally using Docker Compose (simulating a cluster with 1 master and 2 agents):

1. **Prerequisites**:
   - Rust (latest stable)
   - Node.js (for frontend build)
   - Docker & Docker Compose
   - `musl-tools` (for static compilation)

2. **Launch Cluster**:
   ```bash
   make dev-up
   ```

3. **Deploy Code**:
   Whenever you make changes to the Rust or Frontend code, run:
   ```bash
   make dev-deploy
   ```
   This rebuilds the assets, compiles the binaries for Linux (musl), and syncs them into the running containers.

4. **Access Dashboards**:
   
   **Web UI (Recommended)**:
   - URL: `http://localhost:8081`
   - Default Secret: `test_secret_for_cluster_123`

   **TUI**:
   - From host: `R4A_MASTER=http://localhost:8080 r4a-tui`
   - Or inside a container: `docker exec -it node-master r4a-tui`

## Installation (Production)

1. **Build Binaries**:
   ```bash
   make build
   ```

2. **Setup Master**:
   ```bash
   sudo ./r4a-server service enable
   ```

3. **Setup Agent**:
   ```bash
   sudo ./r4a-agent service enable --master http://<master-ip>:8080 --name <node-name>
   ```

## Updates

To update all components (server, agent, tui) in the cluster:
1. Open `r4a-tui`.
2. Go to the **Update** tab.
3. Press `u` to trigger a cluster-wide update.

## Project Structure

- `binaries/`: Entry points for `r4a-server`, `r4a-agent`, and `r4a-tui`.
- `crates/`: Modular logic for VPN, storage, ingress, and workers.
- `.memory.md/`: Project documentation and memory for AI assistants.

## License

MIT / Apache-2.0
