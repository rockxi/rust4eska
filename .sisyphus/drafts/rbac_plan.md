# Draft: Access Control System (RBAC) for rust4eska (r4a)

## 1. Goal
Implement a decision-complete work plan for a structured access control system inspired by k3s/Kubernetes practices, replacing the current flat token-based permission system.

## 2. Current State Analysis
- **Auth Method**: Token-based (Bearer) and Cluster Secret (`X-R4A-Secret`).
- **Authorization**: Flat `permissions: Vec<String>` inside `Token`.
- **RBAC Logic**: Hardcoded checks in Axum extractors and Vault handlers.
- **Storage**: Sled trees (`tokens`, `vault`, `peers`).

## 3. Proposed Architecture (k3s-inspired)
### 3.1. Models
- **Resource**: Enum/String (e.g., `Node`, `Secret`, `Manifest`, `Token`, `User`).
- **Verb**: Enum (`Get`, `List`, `Create`, `Update`, `Delete`, `Proxy`).
- **PolicyRule**: `Resources`, `Verbs`, `ResourceNames` (optional).
- **Role / ClusterRole**: A collection of `PolicyRule`s.
- **RoleBinding / ClusterRoleBinding**: Assigns a `Role` to a `Subject` (User, ServiceAccount).
- **Subject**: `User` or `ServiceAccount`.

### 3.2. Components
- **Auth Middleware**: Enhanced Axum middleware that validates tokens and resolves the Subject.
- **Authorizer**: A component that checks if a `Subject` has permission for a specific `(Resource, Verb, Name)`.
- **Management API**: Endpoints to manage Roles and Bindings.

## 4. Implementation Steps (Tentative)
1. Define new models in `r4a-core`.
2. Implement `Store` methods for Roles/Bindings.
3. Create the `Authorizer` logic.
4. Refactor `r4a-server` extractors to use the new `Authorizer`.
5. Update `r4a-tui` to support the new RBAC structure.

## 5. Open Questions
- Should we support Namespaces?
- Should we keep `X-R4A-Secret` as a "super-admin" shortcut or phase it out?
- How to handle migration of existing tokens?
