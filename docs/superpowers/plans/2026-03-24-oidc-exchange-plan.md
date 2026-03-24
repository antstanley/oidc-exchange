# OIDC Exchange Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust OIDC token exchange service with hexagonal architecture, pluggable providers, and configurable audit/telemetry.

**Architecture:** Cargo workspace with 5 crates: `core` (domain + ports), `adapters` (DynamoDB, KMS, CloudTrail, OIDC, webhook), `providers` (Apple, atproto), `server` (axum + Lambda), `test-utils` (mocks). Trait objects for all ports, TOML config, single binary.

**Tech Stack:** Rust, axum, tokio, serde, thiserror, async-trait, jose (josekit), AWS SDK for Rust, tracing + tracing-opentelemetry, cargo-nextest, wiremock

**Spec:** `docs/superpowers/specs/2026-03-24-oidc-exchange-design.md`

**VCS:** jj (Jujutsu). Use `jj describe` for change descriptions, `jj new` to start the next change.

---

## Phase 1: Foundation

### Task 1: Workspace Scaffold

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/adapters/Cargo.toml`
- Create: `crates/adapters/src/lib.rs`
- Create: `crates/providers/Cargo.toml`
- Create: `crates/providers/src/lib.rs`
- Create: `crates/server/Cargo.toml`
- Create: `crates/server/src/main.rs`
- Create: `crates/test-utils/Cargo.toml`
- Create: `crates/test-utils/src/lib.rs`
- Create: `.config/nextest.toml`
- Create: `config/default.toml`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/core",
    "crates/adapters",
    "crates/providers",
    "crates/server",
    "crates/test-utils",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
# Core
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tokio = { version = "1", features = ["full"] }
ulid = "1"
base64 = "0.22"
sha2 = "0.10"
rand = "0.8"

# Internal crates
oidc-exchange-core = { path = "crates/core" }
oidc-exchange-adapters = { path = "crates/adapters" }
oidc-exchange-providers = { path = "crates/providers" }
oidc-exchange-test-utils = { path = "crates/test-utils" }
```

- [ ] **Step 2: Create crate Cargo.toml files and empty lib.rs/main.rs**

`crates/core/Cargo.toml`:
```toml
[package]
name = "oidc-exchange-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
async-trait.workspace = true
chrono.workspace = true
tracing.workspace = true
ulid.workspace = true
base64.workspace = true
sha2.workspace = true
rand.workspace = true

[dev-dependencies]
tokio.workspace = true
oidc-exchange-test-utils.workspace = true
```

`crates/adapters/Cargo.toml`:
```toml
[package]
name = "oidc-exchange-adapters"
version.workspace = true
edition.workspace = true

[dependencies]
oidc-exchange-core.workspace = true
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true
tracing.workspace = true
tokio.workspace = true
reqwest = { version = "0.12", features = ["json"] }

[dev-dependencies]
oidc-exchange-test-utils.workspace = true
wiremock = "0.6"
```

`crates/providers/Cargo.toml`:
```toml
[package]
name = "oidc-exchange-providers"
version.workspace = true
edition.workspace = true

[dependencies]
oidc-exchange-core.workspace = true
oidc-exchange-adapters.workspace = true
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true
tracing.workspace = true
tokio.workspace = true
reqwest = { version = "0.12", features = ["json"] }

[dev-dependencies]
oidc-exchange-test-utils.workspace = true
wiremock = "0.6"
```

`crates/server/Cargo.toml`:
```toml
[package]
name = "oidc-exchange"
version.workspace = true
edition.workspace = true

[dependencies]
oidc-exchange-core.workspace = true
oidc-exchange-adapters.workspace = true
oidc-exchange-providers.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "request-id"] }
toml = "0.8"
config = "0.14"

[dev-dependencies]
oidc-exchange-test-utils.workspace = true
```

`crates/test-utils/Cargo.toml`:
```toml
[package]
name = "oidc-exchange-test-utils"
version.workspace = true
edition.workspace = true

[dependencies]
oidc-exchange-core.workspace = true
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true
tokio.workspace = true
chrono.workspace = true
```

Each `lib.rs`: empty. `main.rs`:
```rust
fn main() {
    println!("oidc-exchange");
}
```

- [ ] **Step 3: Create nextest config and default TOML config**

`.config/nextest.toml`:
```toml
[profile.default]
retries = 0
slow-timeout = { period = "60s" }

[profile.ci]
retries = 2
fail-fast = true
```

`config/default.toml`:
```toml
[server]
host = "0.0.0.0"
port = 8080

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"

[audit]
adapter = "noop"
blocking_threshold = "warning"

[telemetry]
enabled = false
exporter = "none"
```

- [ ] **Step 4: Verify workspace compiles**

Run: `cargo build --workspace`
Expected: Compiles with no errors

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: scaffold cargo workspace with 5 crates"
jj new
```

---

### Task 2: Domain Types

**Files:**
- Create: `crates/core/src/domain/mod.rs`
- Create: `crates/core/src/domain/user.rs`
- Create: `crates/core/src/domain/session.rs`
- Create: `crates/core/src/domain/token.rs`
- Create: `crates/core/src/domain/audit.rs`
- Create: `crates/core/src/domain/provider.rs`
- Create: `crates/core/src/error.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write user domain types**

`crates/core/src/domain/user.rs` — `User`, `UserStatus`, `NewUser`, `UserPatch` as defined in spec section 4. Derive `Serialize`, `Deserialize`, `Debug`, `Clone`. Use `HashMap<String, serde_json::Value>` for metadata and claims.

- [ ] **Step 2: Write session domain type**

`crates/core/src/domain/session.rs` — `Session` struct as defined in spec section 4.

- [ ] **Step 3: Write token types**

`crates/core/src/domain/token.rs` — `TokenResponse` (with `skip_serializing_if` on refresh_token), `AccessTokenClaims` (with `#[serde(flatten)]` on custom), `ProviderTokens`, `IdentityClaims`.

- [ ] **Step 4: Write audit types**

`crates/core/src/domain/audit.rs` — `AuditEvent`, `AuditSeverity` (syslog levels 0-7), `AuditEventType` enum, `AuditOutcome`. Implement `Serialize` for `AuditOutcome` to produce `{"status": "success"}` / `{"status": "failure", "reason": "..."}` JSON shape per spec.

- [ ] **Step 5: Write provider config types**

`crates/core/src/domain/provider.rs` — `OidcProviderConfig` with optional discovery fields.

- [ ] **Step 6: Write error types**

`crates/core/src/error.rs` — `Error` enum with all variants from spec section 10. Derive `thiserror::Error`. Add `pub type Result<T> = std::result::Result<T, Error>;`.

- [ ] **Step 7: Write domain mod.rs and update lib.rs**

`crates/core/src/domain/mod.rs` — re-export all domain types.
`crates/core/src/lib.rs` — `pub mod domain; pub mod error;`

- [ ] **Step 8: Verify compiles**

Run: `cargo build -p oidc-exchange-core`
Expected: Compiles with no errors

- [ ] **Step 9: Commit**

```bash
jj describe -m "feat: add core domain types (user, session, token, audit, error)"
jj new
```

---

### Task 3: Port Traits

**Files:**
- Create: `crates/core/src/ports/mod.rs`
- Create: `crates/core/src/ports/repository.rs`
- Create: `crates/core/src/ports/key_manager.rs`
- Create: `crates/core/src/ports/audit.rs`
- Create: `crates/core/src/ports/identity_provider.rs`
- Create: `crates/core/src/ports/user_sync.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write all port traits**

Implement all five traits exactly as specified in spec section 3: `Repository`, `KeyManager`, `AuditLog`, `IdentityProvider`, `UserSync`. All use `#[async_trait]`, return `crate::error::Result<T>`.

Add a `Jwk` type alias or struct in `key_manager.rs` — use `serde_json::Value` for now (the actual JWK serialization is adapter-specific).

- [ ] **Step 2: Write ports/mod.rs and update lib.rs**

Re-export all traits. Update `lib.rs` to add `pub mod ports;`.

- [ ] **Step 3: Verify compiles**

Run: `cargo build -p oidc-exchange-core`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add port traits (Repository, KeyManager, AuditLog, IdentityProvider, UserSync)"
jj new
```

---

### Task 4: Configuration

**Files:**
- Create: `crates/core/src/config.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write config structs**

`crates/core/src/config.rs` — Define all config structs matching spec section 8 TOML structure. Use `serde::Deserialize`. Structs: `AppConfig`, `ServerConfig`, `RegistrationConfig`, `TokenConfig`, `AuditConfig`, `KeyManagerConfig`, `RepositoryConfig`, `UserSyncConfig`, `TelemetryConfig`, `InternalApiConfig`, `ProviderConfig` (generic per-provider config map). Add `Default` impls matching the defaults table in the spec.

- [ ] **Step 2: Write a test for config deserialization**

Test that the `config/default.toml` file deserializes into `AppConfig` correctly.

Run: `cargo nextest run -p oidc-exchange-core config`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat: add configuration structs with TOML deserialization"
jj new
```

---

## Phase 2: Test Infrastructure + Core Logic

### Task 5: Test Utilities Crate

**Files:**
- Modify: `crates/test-utils/src/lib.rs`

- [ ] **Step 1: Implement MockRepository**

In-memory `HashMap`-backed implementation of `Repository`. Use `Arc<Mutex<HashMap<...>>>` for interior mutability. Implement all methods: user CRUD stores/retrieves from a `users: HashMap<String, User>` and session ops from `sessions: HashMap<String, Session>`. The `get_user_by_external_id` method does a linear scan (fine for tests).

- [ ] **Step 2: Implement MockKeyManager**

Deterministic Ed25519 key pair — generate once from a fixed seed. Implement `sign()` (real Ed25519 signature using `ed25519-dalek` or equivalent), `public_jwk()` (return a fixed JWK JSON), `algorithm()` returns `"EdDSA"`, `key_id()` returns `"test-key-1"`.

Add `ed25519-dalek` to test-utils dependencies.

- [ ] **Step 3: Implement MockAuditLog**

Collects events into `Arc<Mutex<Vec<AuditEvent>>>`. Provide a `fn events(&self) -> Vec<AuditEvent>` accessor. Optionally configurable to return errors (for testing audit blocking logic).

- [ ] **Step 4: Implement MockUserSync**

Records calls into `Arc<Mutex<Vec<UserSyncCall>>>` where `UserSyncCall` is an enum of `Created(User)`, `Updated(User, Vec<String>)`, `Deleted(String)`. Provide accessor for assertions.

- [ ] **Step 5: Implement MockIdentityProvider**

Configurable: takes closures or preset responses for `exchange_code` and `validate_id_token`. Default: returns a valid `ProviderTokens` and `IdentityClaims` with configurable subject/email.

- [ ] **Step 6: Implement AppServiceBuilder**

Builder pattern that creates `AppService` with all mocks by default. Methods to override individual ports. Returns the `AppService` plus references to the mocks for assertion.

- [ ] **Step 7: Verify compiles**

Run: `cargo build -p oidc-exchange-test-utils`
Expected: Compiles with no errors

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat: add test-utils crate with mock implementations for all ports"
jj new
```

---

### Task 6: Core Service — Token Exchange

**Files:**
- Create: `crates/core/src/service/mod.rs`
- Create: `crates/core/src/service/exchange.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write AppService struct and constructor**

In `service/mod.rs`: define `AppService` holding all ports as `Box<dyn Trait>` plus `AppConfig`. Constructor takes all ports. Add `pub mod exchange;`.

- [ ] **Step 2: Write failing test for token exchange happy path**

In `service/exchange.rs` (inline test module): test that given valid provider response, exchange creates a user, stores a session, signs a JWT, and returns a `TokenResponse`.

Run: `cargo nextest run -p oidc-exchange-core exchange`
Expected: FAIL (method not implemented)

- [ ] **Step 3: Implement exchange flow**

Implement `AppService::exchange()` following spec section 5 flow steps 1-10. Use `ulid` for user ID generation with `usr_` prefix. Use `sha2` for refresh token hashing. Use `rand` for 256-bit refresh token generation. Use `base64` for encoding.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p oidc-exchange-core exchange`
Expected: PASS

- [ ] **Step 5: Write test for existing user exchange**

Test that a second exchange with the same external_id returns the existing user, does not create a new one.

- [ ] **Step 6: Write test for suspended user rejection**

Test that exchange rejects a suspended user with `Error::UserSuspended`.

- [ ] **Step 7: Run all exchange tests**

Run: `cargo nextest run -p oidc-exchange-core exchange`
Expected: All PASS

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat: implement token exchange flow with user lookup/creation"
jj new
```

---

### Task 7: Core Service — Registration Policy

**Files:**
- Modify: `crates/core/src/service/exchange.rs`

- [ ] **Step 1: Write failing test for domain allowlist rejection**

Test that exchange rejects a user whose email domain is not in the configured `domain_allowlist`.

Run: `cargo nextest run -p oidc-exchange-core registration`
Expected: FAIL

- [ ] **Step 2: Implement domain allowlist checking**

Add a `fn matches_domain_allowlist(email: &str, allowlist: &[String]) -> bool` that handles exact match and `*.` wildcard prefix matching. Integrate into the exchange flow at step 4 per spec.

- [ ] **Step 3: Write test for wildcard subdomain matching**

Test that `*.example.com` matches `user@sub.example.com` and `user@a.b.example.com` but not `user@example.com`.

- [ ] **Step 4: Write test for existing_users_only mode**

Test that with `mode = "existing_users_only"`, a new external_id is rejected with `AccessDenied`.

- [ ] **Step 5: Write test for existing user bypasses allowlist**

Test that an existing active user authenticates even when their domain is not in the allowlist.

- [ ] **Step 6: Write test for no-email rejection when allowlist configured**

Test that when `domain_allowlist` is configured and the provider returns no email, the request is rejected.

- [ ] **Step 7: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat: implement registration policy with domain allowlist"
jj new
```

---

### Task 8: Core Service — Token Refresh

**Files:**
- Create: `crates/core/src/service/refresh.rs`
- Modify: `crates/core/src/service/mod.rs`

- [ ] **Step 1: Write failing test for refresh happy path**

Test that given a valid refresh token hash in the repo, refresh returns a new access token without a new refresh token.

Run: `cargo nextest run -p oidc-exchange-core refresh`
Expected: FAIL

- [ ] **Step 2: Implement refresh flow**

Implement `AppService::refresh()` per spec section 5: hash token, look up session, verify not expired, look up user, verify active, sign new JWT.

- [ ] **Step 3: Write test for expired refresh token rejection**

Test that an expired session returns `Error::InvalidToken`.

- [ ] **Step 4: Write test for suspended user on refresh**

Test that refresh for a suspended user returns `Error::UserSuspended`.

- [ ] **Step 5: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat: implement token refresh flow"
jj new
```

---

### Task 9: Core Service — Revocation

**Files:**
- Create: `crates/core/src/service/revoke.rs`
- Modify: `crates/core/src/service/mod.rs`

- [ ] **Step 1: Write failing test for refresh token revocation**

Test that revoking a refresh token removes the session.

Run: `cargo nextest run -p oidc-exchange-core revoke`
Expected: FAIL

- [ ] **Step 2: Implement revocation flow**

Implement `AppService::revoke()` per spec section 5. Handle `token_type_hint` to distinguish refresh vs access token. For access tokens, decode JWT and revoke all user sessions.

- [ ] **Step 3: Write test for access token revocation (all sessions)**

Test that revoking an access token revokes all sessions for that user.

- [ ] **Step 4: Write test for unknown token (no error per RFC 7009)**

Test that revoking an unknown token returns Ok (not an error).

- [ ] **Step 5: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat: implement token revocation flow"
jj new
```

---

### Task 10: Core Service — Audit Blocking Logic

**Files:**
- Modify: `crates/core/src/service/mod.rs`

- [ ] **Step 1: Write failing test for non-blocking audit failure**

Configure MockAuditLog to return errors. Set blocking threshold to `Warning`. Emit an `Info` event. Test that `emit_audit` returns Ok and the event is not lost (check stderr capture or fallback behavior).

Run: `cargo nextest run -p oidc-exchange-core audit_blocking`
Expected: FAIL

- [ ] **Step 2: Implement emit_audit method**

Add `async fn emit_audit(&self, event: AuditEvent) -> Result<()>` to `AppService` per spec section 5. On failure: serialize to stdout/stderr as fallback, then check severity against threshold.

- [ ] **Step 3: Write test for blocking audit failure**

Set blocking threshold to `Warning`. Emit a `Warning` event with a failing audit provider. Test that `emit_audit` returns Err.

- [ ] **Step 4: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: implement audit blocking logic with stdout/stderr fallback"
jj new
```

---

### Task 11: Core Service — Custom Claims Resolution

**Files:**
- Create: `crates/core/src/service/claims.rs`
- Modify: `crates/core/src/service/mod.rs`

- [ ] **Step 1: Write failing test for static claim**

Test that `org = "example"` produces `{"org": "example"}` in the custom claims.

Run: `cargo nextest run -p oidc-exchange-core claims`
Expected: FAIL

- [ ] **Step 2: Implement claims resolver**

Parse `{{ user.field | default: 'value' }}` templates. Support: static strings, `user.*` field references (including `user.metadata.*` and `user.claims.*`), `default` filter. Ignore reserved claims (`sub`, `iss`, `aud`, `iat`, `exp`). Merge config claims first, then per-user claims on top.

- [ ] **Step 3: Write test for field reference with default**

Test `{{ user.metadata.role | default: 'user' }}` resolves to `'user'` when metadata has no `role` key, and to the actual value when present.

- [ ] **Step 4: Write test for per-user claims override**

Test that per-user `claims` override config template claims with the same key.

- [ ] **Step 5: Write test for reserved claim rejection**

Test that `sub = "override"` in config or user claims is silently ignored.

- [ ] **Step 6: Write test for missing field without default omits claim**

Test that `{{ user.metadata.missing }}` (no default) results in the claim being absent.

- [ ] **Step 7: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat: implement custom claims resolution with template syntax"
jj new
```

---

### Task 12: Core Service — User Admin

**Files:**
- Create: `crates/core/src/service/user_admin.rs`
- Modify: `crates/core/src/service/mod.rs`

- [ ] **Step 1: Write failing test for create user via admin**

Test `AppService::admin_create_user()` creates a user and triggers user_sync notification.

Run: `cargo nextest run -p oidc-exchange-core user_admin`
Expected: FAIL

- [ ] **Step 2: Implement user admin service methods**

`admin_create_user`, `admin_get_user`, `admin_update_user`, `admin_delete_user`, `admin_get_claims`, `admin_set_claims`, `admin_merge_claims`, `admin_clear_claims`. These are thin wrappers around the Repository port with audit events and user sync notifications.

- [ ] **Step 3: Write test for update user with claims**

Test that updating a user's claims triggers `user_sync.notify_user_updated` with correct changed_fields.

- [ ] **Step 4: Write test for merge claims**

Test that `admin_merge_claims` merges into existing claims without removing unmentioned keys.

- [ ] **Step 5: Write test for delete user revokes all sessions**

Test that deleting a user sets status to `Deleted` and calls `revoke_all_user_sessions`.

- [ ] **Step 6: Run all tests**

Run: `cargo nextest run -p oidc-exchange-core`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
jj describe -m "feat: implement user admin service with claims management"
jj new
```

---

## Phase 3: Adapters

### Task 13: Local Key Manager

**Files:**
- Create: `crates/adapters/src/local_keys/mod.rs`
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Write failing test for sign + verify round trip**

Test that signing a payload and verifying with the public key succeeds. Use an Ed25519 PEM key file generated in the test setup.

Run: `cargo nextest run -p oidc-exchange-adapters local_keys`
Expected: FAIL

- [ ] **Step 2: Implement LocalKeyManager**

Read PEM private key from file path, import using `ed25519-dalek` or `ring`. Implement `KeyManager` trait: `sign()` produces Ed25519 signature, `public_jwk()` returns the public key as JWK JSON, `algorithm()` returns configured value, `key_id()` returns configured value.

Add appropriate crypto dependencies to `crates/adapters/Cargo.toml`.

- [ ] **Step 3: Run test to verify pass**

Run: `cargo nextest run -p oidc-exchange-adapters local_keys`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: implement local key manager adapter (Ed25519)"
jj new
```

---

### Task 14: No-op Adapters

**Files:**
- Create: `crates/adapters/src/noop/mod.rs`
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Implement NoopAuditLog**

Implements `AuditLog` trait. `emit()` always returns `Ok(())`. Used as default when no audit adapter is configured.

- [ ] **Step 2: Implement NoopUserSync**

Implements `UserSync` trait. All methods return `Ok(())`. Used when `user_sync.enabled = false`.

- [ ] **Step 3: Verify compiles**

Run: `cargo build -p oidc-exchange-adapters`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add noop adapters for audit and user sync"
jj new
```

---

### Task 15: Shared OIDC Utilities

**Files:**
- Create: `crates/adapters/src/shared/mod.rs`
- Create: `crates/adapters/src/shared/jwks.rs`
- Create: `crates/adapters/src/shared/discovery.rs`
- Create: `crates/adapters/src/shared/token_endpoint.rs`
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Write failing test for OIDC discovery parsing**

Use `wiremock` to serve a mock `.well-known/openid-configuration` JSON. Test that `discover()` returns parsed endpoints.

Run: `cargo nextest run -p oidc-exchange-adapters discovery`
Expected: FAIL

- [ ] **Step 2: Implement discovery module**

`discover(issuer_url)` fetches `{issuer}/.well-known/openid-configuration`, parses JSON, returns struct with `token_endpoint`, `jwks_uri`, `revocation_endpoint`.

- [ ] **Step 3: Write failing test for JWKS cache**

Use `wiremock` to serve a JWKS endpoint. Test that `JwksCache` fetches keys, caches them, and uses cache on second call.

- [ ] **Step 4: Implement JwksCache**

Fetches JWKS from URL, caches in memory with TTL (default 1 hour). `get_keys()` returns cached if fresh, fetches if stale. Thread-safe with `Arc<RwLock<...>>`.

- [ ] **Step 5: Implement token_endpoint exchange helper**

`exchange(endpoint, params) -> ProviderTokens` — sends form-encoded POST, parses JSON response with `id_token`, `refresh_token`, `access_token`.

- [ ] **Step 6: Run all tests**

Run: `cargo nextest run -p oidc-exchange-adapters`
Expected: All PASS

- [ ] **Step 7: Commit**

```bash
jj describe -m "feat: add shared OIDC utilities (discovery, JWKS cache, token endpoint)"
jj new
```

---

### Task 16: Standard OIDC Provider Adapter

**Files:**
- Create: `crates/adapters/src/oidc/mod.rs`
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Write failing test for OIDC code exchange**

Use `wiremock` for discovery, JWKS, and token endpoints. Test that `OidcProvider::exchange_code()` calls the token endpoint and returns `ProviderTokens`.

Run: `cargo nextest run -p oidc-exchange-adapters oidc_provider`
Expected: FAIL

- [ ] **Step 2: Implement OidcProvider**

Implements `IdentityProvider`. Constructor takes `OidcProviderConfig`, runs discovery at init (or lazily), sets up `JwksCache`. `exchange_code()` calls token endpoint via shared utility. `validate_id_token()` verifies JWT signature against cached JWKS, checks `iss`, `aud`, `exp`. `revoke_token()` calls revocation endpoint if configured.

Use `josekit` crate for JWT verification. Add to adapters dependencies.

- [ ] **Step 3: Write test for ID token validation with JWKS**

Generate a test JWT signed with a known key. Serve the public key via wiremock JWKS endpoint. Test that `validate_id_token()` returns correct `IdentityClaims`.

- [ ] **Step 4: Write test for expired token rejection**

Create an expired JWT. Test that validation returns `Error::InvalidGrant`.

- [ ] **Step 5: Run all tests**

Run: `cargo nextest run -p oidc-exchange-adapters`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat: implement standard OIDC provider adapter with JWT validation"
jj new
```

---

### Task 17: Apple Provider

**Files:**
- Create: `crates/providers/src/apple.rs`
- Modify: `crates/providers/src/lib.rs`

- [ ] **Step 1: Write failing test for Apple client JWT generation**

Test that `AppleProvider` generates a valid ES256-signed client JWT with correct claims (`sub`, `iss`, `aud`, `kid`).

Run: `cargo nextest run -p oidc-exchange-providers apple`
Expected: FAIL

- [ ] **Step 2: Implement AppleProvider**

Implements `IdentityProvider`. Generates ES256 client JWT for each token endpoint call. Reuses shared OIDC utilities for JWKS/discovery. `from_config()` loads private key from file path.

- [ ] **Step 3: Write test for full Apple exchange flow with wiremock**

Mock Apple's token and JWKS endpoints. Test the full `exchange_code` + `validate_id_token` flow.

- [ ] **Step 4: Run all tests**

Run: `cargo nextest run -p oidc-exchange-providers`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: implement Apple Sign-In provider with ES256 client JWT"
jj new
```

---

### Task 18: DynamoDB Repository Adapter

**Files:**
- Create: `crates/adapters/src/dynamo/mod.rs`
- Create: `crates/adapters/src/dynamo/schema.rs`
- Modify: `crates/adapters/Cargo.toml` (add AWS SDK deps)
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Add AWS SDK dependencies**

Add `aws-sdk-dynamodb`, `aws-config` to adapters Cargo.toml.

- [ ] **Step 2: Implement DynamoDB attribute mapping**

In `schema.rs`: functions to convert `User` → DynamoDB item (with pk/sk/GSI1pk/GSI1sk per table-design.json) and back. Same for `Session`. Handle the key prefixes: `USER#`, `PROFILE`, `EXT#provider#external_id`, `SESSION#`.

- [ ] **Step 3: Implement DynamoRepository**

Implements `Repository` trait. Constructor takes table name and `aws_sdk_dynamodb::Client`. Implement all methods using the DynamoDB SDK:
- `get_user_by_id`: GetItem with `pk=USER#{id}, sk=PROFILE`
- `get_user_by_external_id`: Query GSI1 with `GSI1pk=EXT#{provider}#{external_id}`
- `create_user`: PutItem
- `update_user`: UpdateItem
- `delete_user`: UpdateItem (set status to Deleted)
- `store_refresh_token`: PutItem with TTL
- `get_session_by_refresh_token`: GetItem with `pk=SESSION#{hash}, sk=SESSION`
- `revoke_session`: DeleteItem
- `revoke_all_user_sessions`: Query GSI1 + BatchWriteItem

- [ ] **Step 4: Write integration test (requires DynamoDB Local)**

Mark with `#[ignore]` for CI without Docker. Test full CRUD cycle: create user, get by external_id, store session, get session, revoke session.

Run: `cargo nextest run -p oidc-exchange-adapters dynamo -- --ignored` (with DynamoDB Local running)
Expected: PASS

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: implement DynamoDB repository adapter with single-table design"
jj new
```

---

### Task 19: CloudTrail Audit Adapter

**Files:**
- Create: `crates/adapters/src/cloudtrail/mod.rs`
- Modify: `crates/adapters/Cargo.toml` (add cloudtrail-data SDK)
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Add AWS CloudTrail Data SDK dependency**

Add `aws-sdk-cloudtraildata` to adapters Cargo.toml.

- [ ] **Step 2: Implement CloudTrailAuditLog**

Implements `AuditLog`. `emit()` converts `AuditEvent` to CloudTrail Lake `PutAuditEvents` API format, calls the API. Constructor takes channel ARN and SDK client.

- [ ] **Step 3: Write unit test for event format conversion**

Test that `AuditEvent` is correctly serialized into the CloudTrail `AuditEvent` format (with `id`, `eventData` JSON string, `eventDataChecksum`).

Run: `cargo nextest run -p oidc-exchange-adapters cloudtrail`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: implement CloudTrail Lake audit adapter"
jj new
```

---

### Task 20: Webhook User Sync Adapter

**Files:**
- Create: `crates/adapters/src/webhook/mod.rs`
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Write failing test for webhook delivery with HMAC**

Use `wiremock` to receive the webhook POST. Verify correct payload format, HMAC-SHA256 signature in `X-Signature-256` header.

Run: `cargo nextest run -p oidc-exchange-adapters webhook`
Expected: FAIL

- [ ] **Step 2: Implement WebhookUserSync**

Implements `UserSync`. Constructor takes URL, secret, timeout, retries. Each notification: serialize payload JSON, compute HMAC-SHA256 of body, send POST with signature header. Retry with exponential backoff on 5xx/timeout.

Add `hmac` and `hex` crates to adapters dependencies.

- [ ] **Step 3: Write test for retry on 5xx**

Wiremock returns 500 twice, then 200. Verify the adapter retries and eventually succeeds.

- [ ] **Step 4: Run all tests**

Run: `cargo nextest run -p oidc-exchange-adapters`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: implement webhook user sync adapter with HMAC signing and retry"
jj new
```

---

### Task 21: KMS Key Manager Adapter

**Files:**
- Create: `crates/adapters/src/kms/mod.rs`
- Modify: `crates/adapters/Cargo.toml` (add KMS SDK)
- Modify: `crates/adapters/src/lib.rs`

- [ ] **Step 1: Add AWS KMS SDK dependency**

Add `aws-sdk-kms` to adapters Cargo.toml.

- [ ] **Step 2: Implement KmsKeyManager**

Implements `KeyManager`. `sign()` calls KMS `Sign` API. `public_jwk()` calls KMS `GetPublicKey` and converts to JWK format. Constructor takes key ID, algorithm, kid.

- [ ] **Step 3: Write integration test (requires LocalStack, #[ignore])**

Test sign + public key retrieval round trip against LocalStack KMS.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: implement AWS KMS key manager adapter"
jj new
```

---

## Phase 4: HTTP + Server

### Task 22: Axum Public Routes

**Files:**
- Create: `crates/server/src/routes/mod.rs`
- Create: `crates/server/src/routes/token.rs`
- Create: `crates/server/src/routes/revoke.rs`
- Create: `crates/server/src/routes/keys.rs`
- Create: `crates/server/src/routes/well_known.rs`
- Create: `crates/server/src/routes/health.rs`
- Create: `crates/server/src/state.rs`
- Modify: `crates/server/src/main.rs`

- [ ] **Step 1: Define AppState**

`crates/server/src/state.rs` — `AppState` with `Arc<AppService>` and `Arc<AppConfig>`.

- [ ] **Step 2: Write failing test for POST /token (code exchange)**

Use `tower::ServiceExt` to test the axum router directly. Send form-encoded `grant_type=authorization_code&code=test&provider=mock&redirect_uri=http://localhost`. Assert 200 with JSON containing `access_token`.

Run: `cargo nextest run -p oidc-exchange token_route`
Expected: FAIL

- [ ] **Step 3: Implement POST /token handler**

Parse form-encoded body. Dispatch to `AppService::exchange()` or `AppService::refresh()` based on `grant_type`. Map domain errors to OAuth 2.0 JSON error responses per spec section 10.

- [ ] **Step 4: Implement POST /revoke handler**

Parse form-encoded body. Call `AppService::revoke()`. Always return 200.

- [ ] **Step 5: Implement GET /keys handler**

Call `keys.public_jwk()` via AppState. Return `{"keys": [jwk]}`.

- [ ] **Step 6: Implement GET /.well-known/openid-configuration**

Return static JSON built from `AppConfig.server.issuer` with algorithm from key manager.

- [ ] **Step 7: Implement GET /health**

Return 200 with `{"status": "ok"}`.

- [ ] **Step 8: Build router and wire routes**

In `routes/mod.rs`: `pub fn public_routes() -> Router<AppState>` combining all public routes.

- [ ] **Step 9: Write test for error response format**

Send invalid grant_type. Assert 400 with `{"error": "invalid_request", "error_description": "..."}`.

- [ ] **Step 10: Run all tests**

Run: `cargo nextest run -p oidc-exchange`
Expected: All PASS

- [ ] **Step 11: Commit**

```bash
jj describe -m "feat: implement axum public routes (token, revoke, keys, well-known, health)"
jj new
```

---

### Task 23: Axum Internal Routes

**Files:**
- Create: `crates/server/src/routes/internal.rs`
- Create: `crates/server/src/middleware/mod.rs`
- Create: `crates/server/src/middleware/internal_auth.rs`

- [ ] **Step 1: Implement internal auth middleware**

Extract `Authorization: Bearer <secret>` header. Constant-time compare against configured secret. Return 401 if mismatch.

- [ ] **Step 2: Write failing test for internal auth rejection**

Send a request to `/internal/users` without auth header. Assert 401.

Run: `cargo nextest run -p oidc-exchange internal_auth`
Expected: FAIL

- [ ] **Step 3: Implement internal user CRUD routes**

`POST /internal/users` — create user
`GET /internal/users/{id}` — get user
`PATCH /internal/users/{id}` — update user (accepts `UserPatch` JSON)
`DELETE /internal/users/{id}` — soft-delete user

- [ ] **Step 4: Implement internal claims routes**

`GET /internal/users/{id}/claims` — get claims
`PUT /internal/users/{id}/claims` — replace claims
`PATCH /internal/users/{id}/claims` — merge claims
`DELETE /internal/users/{id}/claims` — clear claims

- [ ] **Step 5: Write test for claims PATCH merge**

Create a user with claims `{"a": 1}`. PATCH with `{"b": 2}`. Assert GET returns `{"a": 1, "b": 2}`.

- [ ] **Step 6: Build internal router with auth middleware**

In `routes/internal.rs`: `pub fn internal_routes() -> Router<AppState>` with auth middleware layer.

- [ ] **Step 7: Run all tests**

Run: `cargo nextest run -p oidc-exchange`
Expected: All PASS

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat: implement internal admin API routes with shared secret auth"
jj new
```

---

### Task 24: Middleware (Request ID, Audit Context, Error Handling)

**Files:**
- Create: `crates/server/src/middleware/request_id.rs`
- Create: `crates/server/src/middleware/audit_context.rs`
- Create: `crates/server/src/middleware/error_handler.rs`
- Modify: `crates/server/src/middleware/mod.rs`

- [ ] **Step 1: Implement request ID middleware**

Generate UUID if `X-Request-Id` header absent. Attach to tracing span. Set response header.

- [ ] **Step 2: Implement audit context extractor**

Extract `X-Forwarded-For` (IP), `User-Agent`, `X-Device-Id` from headers. Store in request extensions for use by service layer.

- [ ] **Step 3: Implement error handler layer**

Catch panics. Convert unhandled errors to structured JSON `{"error": "server_error", "error_description": "..."}`.

- [ ] **Step 4: Write test for request ID propagation**

Send request without `X-Request-Id`. Assert response has the header set.

- [ ] **Step 5: Run all tests**

Run: `cargo nextest run -p oidc-exchange`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat: add middleware for request ID, audit context, and error handling"
jj new
```

---

### Task 25: Telemetry Setup

**Files:**
- Create: `crates/server/src/telemetry.rs`
- Modify: `crates/server/Cargo.toml` (add OTEL deps)

- [ ] **Step 1: Add OTEL dependencies**

Add `tracing-opentelemetry`, `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`, `opentelemetry-aws` to server Cargo.toml.

- [ ] **Step 2: Implement telemetry init function**

`pub fn init_telemetry(config: &TelemetryConfig) -> Result<()>` — sets up `tracing_subscriber` with optional OTEL layer based on config. Exporters: `none` (tracing-subscriber only), `stdout` (OTEL stdout exporter), `otlp` (OTLP gRPC/HTTP exporter), `xray` (AWS X-Ray via `opentelemetry-aws` with X-Ray ID generator and propagator — Lambda-native). Falls back to `tracing_subscriber::fmt` when OTEL is disabled.

- [ ] **Step 3: Write test that init doesn't panic with "none" exporter**

Run: `cargo nextest run -p oidc-exchange telemetry`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add telemetry setup with pluggable OTEL exporters"
jj new
```

---

### Task 26: Server Bootstrap (main.rs)

**Files:**
- Modify: `crates/server/src/main.rs`

- [ ] **Step 1: Implement config loading**

Load TOML config following spec section 8 loading order: `config/default.toml`, `config/{OIDC_EXCHANGE_ENV}.toml`, env var overrides, `${VAR}` placeholder resolution.

- [ ] **Step 2: Implement adapter wiring**

`build_adapters(config) -> (Box<dyn Repository>, Box<dyn KeyManager>, Box<dyn AuditLog>, Box<dyn UserSync>)` — match on config adapter names, construct appropriate adapter.

- [ ] **Step 3: Implement provider registry**

`build_providers(config) -> HashMap<String, Box<dyn IdentityProvider>>` — iterate config providers, construct OidcProvider/AppleProvider based on adapter field.

- [ ] **Step 4: Implement runtime detection**

Check `AWS_LAMBDA_RUNTIME_API` env var. If set, wrap router with `lambda_http`. Otherwise, bind to `server.host:server.port` with hyper.

Add `lambda_http` and `lambda_runtime` to server Cargo.toml and workspace root `[workspace.dependencies]` (behind no feature flag — both paths compiled in).

- [ ] **Step 5: Wire everything together**

Init telemetry → load config → build adapters → build providers → construct AppService → build Router → run.

- [ ] **Step 6: Verify builds and runs locally**

Run: `cargo build -p oidc-exchange && ./target/debug/oidc-exchange`
Expected: Starts listening (or fails gracefully if no providers configured)

- [ ] **Step 7: Commit**

```bash
jj describe -m "feat: implement server bootstrap with config loading, adapter wiring, and Lambda/server detection"
jj new
```

---

## Phase 5: Schemas + Polish

### Task 27: JSON Schema Files

**Files:**
- Create: `schemas/datamodel.schema.json`
- Create: `schemas/dynamodb/table-design.json`

- [ ] **Step 1: Write datamodel.schema.json**

Copy the JSON schema from spec section 11 (User, Session, AuditEvent definitions).

- [ ] **Step 2: Write table-design.json**

Copy the DynamoDB table design from spec section 11 (table, GSIs, access patterns, item schemas).

- [ ] **Step 3: Commit**

```bash
jj describe -m "docs: add JSON schema for data model and DynamoDB table design"
jj new
```

---

### Task 28: End-to-End Integration Test

**Files:**
- Create: `crates/server/tests/e2e.rs`

- [ ] **Step 1: Write E2E test with all mocks**

Build the full axum router with MockRepository, MockKeyManager, MockAuditLog, MockUserSync, and a MockIdentityProvider (using wiremock for OIDC endpoints). Test the full flow: POST /token (exchange) → get access token → POST /token (refresh) → get new access token → POST /revoke → verify session gone.

- [ ] **Step 2: Write E2E test for internal API**

Create user via POST /internal/users, set claims via PUT /internal/users/{id}/claims, exchange token, verify custom claims appear in the JWT.

- [ ] **Step 3: Write E2E test for registration policy**

Configure `existing_users_only` mode. Attempt exchange without pre-existing user. Assert 403. Create user via internal API. Attempt exchange again. Assert success.

- [ ] **Step 4: Run all tests**

Run: `cargo nextest run --workspace`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
jj describe -m "test: add end-to-end integration tests for full auth flows"
jj new
```

---

## Task Dependency Summary

```
Task 1 (scaffold) → Task 2 (domain) → Task 3 (ports) → Task 4 (config)
                                                              ↓
Task 5 (test-utils) ← depends on Task 3
    ↓
Tasks 6-12 (core service logic) — sequential, each builds on prior
    ↓
Tasks 13-21 (adapters) — can be parallelized after Task 5
    ↓
Tasks 22-26 (HTTP + server) — depend on core + at least local_keys + noop adapters
    ↓
Tasks 27-28 (schemas + E2E) — final
```

**Parallelizable groups after Task 5:**
- Tasks 13, 14, 15 (local keys, noop, shared OIDC) can run in parallel
- Tasks 16, 17, 18, 19, 20, 21 (provider + adapter implementations) can run in parallel
- Tasks 22, 23, 24, 25 (HTTP routes + middleware + telemetry) are mostly independent of each other

**Note on atproto provider:** The atproto provider (spec section 7, Tier 3) is intentionally deferred from this plan due to its complexity (DPoP, PAR, DID resolution). It should be a separate implementation plan after the core system is working end-to-end. The provider registry supports adding it later with zero changes to existing code.
