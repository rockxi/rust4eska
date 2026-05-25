# Work Plan: RBAC Integration (k3s practices)

## 1. Context & Goal
Replace the simple `Role::Admin` / `Role::Agent` prefix-based authentication with a structured Policy-based RBAC system inspired by k3s, but using simplified terminology. 

## 2. Key Decisions
- **Terminology**: `Policy`, `Binding`, `Subject`, `Permissions` (instead of K8s `ClusterRole`, `RoleBinding`).
- **Granularity**: High. Permissions define verbs (get, create, update, delete) and resources (nodes, manifests, vault, git_repos), plus optional `resource_names` (e.g., `["production/db-pass"]`).
- **Secret Bypass**: `X-R4A-Secret` is demoted to **Bootstrap Only** (node joins, emergency admin token generation). All standard API traffic must use JWT/Tokens.

## 3. Scope Boundaries & Guardrails
- **IN SCOPE**: Refactoring `r4a-core/src/models/mod.rs`, updating `RequireToken` / `RequireSecret` extractors in `r4a-server`, updating TUI and CLI to use the new structures.
- **OUT OF SCOPE**: Namespaces (all policies are cluster-wide), user groups (subjects map directly to usernames/node names).
- **GUARDRAILS**: 
  - Ensure database backwards compatibility or add a migration for the `tokens` table in Sled, so existing instances don't crash when deserializing old tokens.
  - TUI and CLI must be updated to securely store and use the token. Currently, they might rely heavily on `R4A_SECRET`. If `R4A_SECRET` is used, the client must auto-exchange it for an Admin Token on startup.

## 4. Tasks: Data Models & Storage
- [ ] In `crates/r4a-core/src/models/mod.rs`, define `Verb` enum (`Get`, `List`, `Create`, `Update`, `Delete`) and `Resource` enum (`Nodes`, `Manifests`, `Vault`, `GitRepos`, `Tokens`, `All`).
- [ ] Define `Policy` struct: `pub struct Policy { pub id: String, pub rules: Vec<Rule> }`.
- [ ] Define `Rule` struct: `pub struct Rule { pub verbs: Vec<Verb>, pub resources: Vec<Resource>, pub resource_names: Option<Vec<String>> }`.
- [ ] Define `Binding` struct: `pub struct Binding { pub id: String, pub subject: String, pub policy_id: String }`.
- [ ] Redefine `Token` struct. Remove old `Role` enum and `permissions` prefixes. A token should carry the `username` (Subject) and `id`.
- [ ] In `crates/r4a-store/src/db/sled_wrapper.rs`, add new trees (tables): `policies` and `bindings`.
- [ ] Write a startup migration function in `sled_wrapper.rs` (e.g., `migrate_v1_to_v2_rbac`) that iterates over the old `tokens` and `users` tables. For every `Role::Admin`, create an Admin `Policy` and a `Binding`. For every `Role::Agent`, create a specific `Policy` based on the old `permissions` array (convert `"secret/path*"` to `Resource::Vault`, `resource_names: ["secret/path"]`), save the `Binding`, and resave the `Token` in the new format.

## 5. Tasks: Axum Extractors & API
- [ ] Update `r4a-store` API (`src/lib.rs`) to include CRUD methods for `put_policy`, `get_policy`, `put_binding`, `get_binding`.
- [ ] In `binaries/r4a-server/src/main.rs`, update the `RequireToken` extractor. It should query the store for `Bindings` matching the token's `username`, then fetch the associated `Policies`, and provide an authorization method: `token.can(&store, Verb::Get, Resource::Vault, Some("production/db-pass"))`.
- [ ] Migrate `RequireSecret` on standard endpoints (`/api/nodes`, `/api/manifests`, `/api/vault`, `/api/git/repos`) to `RequireToken`.
- [ ] Ensure node join endpoints (`/api/join`, etc.) remain accessible via `RequireSecret` (bootstrap).
- [ ] Add `POST /api/tokens/exchange` endpoint which takes `X-R4A-Secret` via header and issues a full `Admin` Token for CLI/TUI initial bootstrap.

## 6. Tasks: CLI & TUI Updates
- [ ] In `crates/r4a-client/src/lib.rs`, add token auto-exchange logic. If instantiated with a `cluster_secret`, it should automatically call `/api/tokens/exchange` and inject the received `Bearer` token into subsequent requests. Store the token in memory to avoid repeated exchanges.
- [ ] Update `binaries/r4a-cli` and `binaries/r4a-tui` to work with the updated `r4a-client` flow. Ensure they don't break when connecting using only the secret.
- [ ] Update the Vault Grant Access logic (`a` key in TUI). Instead of creating a token with permissions `"secret/path*"`, it should call a new API endpoint (or reuse existing) to create a specific `Policy` and `Binding` for the agent allowing `Verb::Get` on `Resource::Vault` with `resource_names = ["path/to/secret"]`.

## Final Verification Wave
- [ ] Nodes can successfully join the cluster using the Bootstrap Secret.
- [ ] TUI can authenticate and list nodes/manifests/vault without using the Bootstrap Secret directly for those endpoints.
- [ ] An Agent cannot read a Vault secret it lacks a `Binding` for.
- [ ] A `Binding` granting access to specific `resource_names` in Vault successfully allows retrieval of *only* those secrets.
