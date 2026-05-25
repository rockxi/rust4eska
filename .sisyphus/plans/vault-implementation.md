# Vault Implementation Plan: rust4eska

## 1. Goal
Implement a secure, encrypted Vault for the `rust4eska` project to store secrets, manage strict RBAC (Agent Tokens), and inject secrets into workloads, complete with Server API and TUI management screens.

## 2. Scope & Boundaries
- **IN**: `r4a-crypto` crate implementation (AES-GCM, Argon2). `Token`, `User`, `Role` models. `r4a-store` vault tree integration. `r4a-server` vault API endpoints. `r4a-agent` secret resolution in reconciler. `r4a-tui` screens for Vault and RBAC. Integration testing via `compose.yaml`.
- **OUT**: Secret Files (tmpfs) support in Agent (we will use environment variables for MVP). External KMS integration.

## 3. Architecture & Technical Approach

### 3.1 Key Management (KEK Pattern)
- The `cluster_secret` (via Argon2) derives the **Master Key**. (Run Argon2 ONCE at startup).
- The Master Key encrypts the **Data Encryption Key (DEK)** which is stored in the DB.
- The DEK encrypts all actual secret values. This allows rotating the `cluster_secret` without re-encrypting all Vault data.
- Use `zeroize` crate to clear decrypted secrets from memory immediately after use.

### 3.2 RBAC & Token Model (Flat RBAC)
- **Roles**: `Admin` (Full access, used by TUI/CLI), `Agent` (Read-only access).
- **Access Control**: Agents can only read secrets prefixed with their node name (e.g., `/vault/nodes/{node_name}/*`) or global secrets (e.g., `/vault/global/*`).
- **Bootstrapping**: When an agent calls `/api/join`, the master provisions a specific `AgentToken` and returns it. The agent uses this token for Vault API calls instead of the root `cluster_secret`.

### 3.3 Acceptance Criteria
1. Secrets are encrypted at rest in `sled` using AES-GCM.
2. Agents successfully parse `vault://` URIs in manifests and fetch values using their specific `AgentToken`.
3. Updating a secret in the Vault triggers a reconciliation loop on affected Agents, restarting the container with the new value.
4. TUI displays secrets masked (`****`) by default with a "Reveal" action.
5. All access (Success/Denied) is logged to `r4a-telemetry` with the Token ID.
6. Docker compose integration tests pass.

## 4. Execution Plan (Tasks)

### Phase 1: Core Models & Crypto primitives
1. **r4a-core**: Add Models for `User`, `Token`, `Role` (Admin/Agent) and `VaultSecret`. Add KEK/DEK structures.
2. **r4a-crypto**: Create new crate (or module in core) wrapping AES-GCM and Argon2. Expose `encrypt()`, `decrypt()`, `derive_master_key()`. Ensure `zeroize` is heavily used for intermediate decryption strings.
3. **r4a-store**: Add a new `vault` tree. Add `tokens` tree. Add logic to derive Master Key from `cluster_secret` ONCE on init. Save encrypted DEK in Sled.

### Phase 2: Server API & Bootstrapping
4. **r4a-server (Bootstrapping)**: Update the `/api/join` endpoint to securely generate, store, and return a unique `AgentToken` bound to the requesting Node ID.
5. **r4a-server (Vault API)**: Create CRUD endpoints for Vault (`/api/vault/...`) and Tokens (`/api/tokens/...`). Protect with `RequireSecret` (Admin) and `RequireToken` (Agent).
6. **r4a-server (Telemetry)**: Add access logging to `r4a-telemetry` for every Vault read/write indicating Token ID and timestamp.

### Phase 3: Agent Integration
7. **r4a-agent (Reconciler)**: Update the `Reconciler` to parse `vault://global/foo` or `vault://nodes/node-name/bar` syntaxes in `manifest.env`.
8. **r4a-agent (Fetcher)**: Implement an API client method using the provisioned `AgentToken` to fetch decrypted values directly before container startup.
9. **r4a-agent (Rotation)**: Implement trigger logic (polling or server push) so when a Vault secret changes, the agent re-evaluates manifests and restarts affected containers within 30 seconds.

### Phase 4: TUI & Management
10. **r4a-tui (Admin)**: Create the `RBAC` screen to manage Users and Tokens.
11. **r4a-tui (Vault)**: Create the `Secrets` screen to manage the Vault KV store. Ensure secrets are masked (`****`) by default, requiring a "Reveal" hotkey.

## Final Verification Wave
- Execute: `cargo test --workspace`
- Execute: `cargo clippy --workspace -- -D warnings`
- Verify Docker integration: `docker compose up -d` and test agent secret injection.
- Review output manually to ensure secrets are not logged in plaintext.
