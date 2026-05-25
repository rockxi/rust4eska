# Work Plan: r4a Web Interface (r4a-web)

This plan covers the development of a lightweight, dark-themed web interface for the `r4a` cluster management system, packaged as a standalone binary with embedded frontend assets.

## 1. Objectives
- Create `r4a-web` binary that serves a React-based frontend.
- Ensure feature parity with `r4a-tui`.
- Maintain a strictly dark, minimalistic, and "non-overloaded" UI.
- Support zero-dependency deployment (embedded assets).
- Enable CORS in `r4a-server` to allow web UI API calls.

## 2. Architecture
- **Backend (r4a-web)**: Rust (Axum) + `rust-embed`.
- **Frontend (frontend)**: React + Vite + Tailwind CSS + Lucide React (icons).
- **Communication**: Frontend talks to `r4a-server` (port 8080) via REST.
- **Auth**: User provides Cluster Secret on login -> Frontend exchanges it for a Token -> Token stored in SessionStorage.

## 3. Tech Stack Decisions
- **Framework**: React 18+ with Vite.
- **Styling**: Tailwind CSS (Dark Mode by default).
- **State Management**: React Query (TanStack Query) for efficient API polling and caching.
- **Icons**: Lucide React.
- **HTTP Client**: Axios or Fetch API.

## 4. Phase 1: Infrastructure & CORS
- Add `tower-http` to `r4a-server` dependencies.
- Update `r4a-server/src/main.rs` to include `CorsLayer`.
- Create `binaries/r4a-web` with `Cargo.toml` and basic Axum server.
- Initialize Vite project in `binaries/r4a-web/frontend`.

## 5. Phase 2: Core UI Components
- Setup Tailwind with a deep dark palette (Slate/Zinc).
- Create Layout component (Sidebar/Navbar + Content area).
- Implement Login screen (Secret input).

## 6. Phase 3: Feature Implementation
- **Dashboard**: Card-based view of nodes, CPU/RAM/VRAM bars.
- **Git**: Table of repos, "New Repository" modal.
- **Vault**: Key-value list, reveal/edit/delete functionality.
- **RBAC**: Token management table, creation form.
- **Updates**: Cluster status, trigger update button.

## 7. Phase 4: Embedding & Finalization
- Build frontend production assets.
- Integrate `rust-embed` in `r4a-web` backend to serve `dist/` folder.
- Add `r4a-web` to workspace `Cargo.toml`.

---

## Task Breakdown - COMPLETED

### Infrastructure
- [x] **Task 1: Update r4a-server with CORS**
  - Add `tower-http = { version = "0.5", features = ["cors"] }` to `binaries/r4a-server/Cargo.toml`.
  - Edit `binaries/r4a-server/src/main.rs` to add `CorsLayer::permissive()` to the Router.
  - Verify: `curl -I -X OPTIONS http://localhost:8080/api/nodes` returns CORS headers.

- [x] **Task 2: Initialize binaries/r4a-web Workspace Member**
  - Create `binaries/r4a-web/Cargo.toml`.
  - Create `binaries/r4a-web/src/main.rs` with Axum skeleton.
  - Add `r4a-web` to root `Cargo.toml`.
  - Verify: `cargo check -p r4a-web` passes.

- [x] **Task 3: Initialize Frontend Project**
  - Run `npm create vite@latest frontend -- --template react-ts` in `binaries/r4a-web`.
  - Install Tailwind CSS and Lucide React.
  - Setup `vite.config.ts` for build output into `../dist`.
  - Verify: `cd binaries/r4a-web/frontend && npm run build` creates `../dist` folder.

### Frontend Development
- [x] **Task 4: Authentication & Layout**
  - Create Auth provider to handle Token storage.
  - Implement Login page.
  - Implement App Shell (Sidebar navigation matching TUI tabs).
  - Verify: Enter cluster secret on Login page -> Redirected to Dashboard -> `r4a_token` exists in browser `sessionStorage`.

- [x] **Task 5: Dashboard Screen**
  - Fetch `/api/nodes`.
  - Display nodes as cards with real-time progress bars for metrics.
  - Polling every 2s (matching TUI).
  - Verify: Dashboard cards match `r4a-cli nodes` output; metrics update dynamically.

- [x] **Task 6: Git & Vault Screens**
  - Implement Git repo listing and creation.
  - Implement Vault management (List -> Get value -> Set/Delete).
  - Use modals or inline forms for editing.
  - Verify: Creating a repo in Web UI -> `r4a-cli git list` shows it. Setting a Vault secret -> `r4a-cli vault get <key>` returns the new value.

- [x] **Task 7: RBAC & Updates Screens**
  - Token listing and deletion.
  - Cluster update status and trigger logic.
  - Verify: Tokens in UI match `r4a-cli token list`. Triggering "Update All" -> Master `update_pending` flag becomes true.

### Packaging
- [x] **Task 8: Assets Embedding**
  - Use `rust-embed` in `r4a-web/src/main.rs`.
  - Implement a fallback handler in Axum to serve `index.html` for SPA routing.
  - Verify: Running the compiled binary serves the UI on a new port (e.g., 8081).

---

## Final Verification Wave
- [x] UI is dark and lightweight (check bundle size and render speed).
- [x] All TUI features are functional in the browser.
- [x] Binary is standalone (no external files needed after build).
- [x] CORS is correctly restricted if needed (currently permissive for ease of use).
