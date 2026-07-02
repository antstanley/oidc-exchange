# Ports and Adapters

**Status:** Implemented · **Date:** 2026-07-02 · **Owner:** Ant Stanley · **Scope:** crates/core/src/ports, crates/adapters

> **Read first:** [.specs/architecture-principles.md](../../architecture-principles.md) for
> the inward-dependency rule and why ports are `Box<dyn Trait>`.

The core declares six port traits in `crates/core/src/ports/`. Adapters in
`crates/adapters/` and `crates/providers/` implement them. Every method returns the core's
`Result<T>`; adapters convert native errors into the domain [`Error`](04-http-api.md) at the
boundary.

## Port traits

### UserRepository (`ports/repository.rs`)

```rust
async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>>;
async fn get_user_by_external_id(&self, external_id: &str, provider: &str) -> Result<Option<User>>;
async fn create_user(&self, user: &NewUser) -> Result<User>;
async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User>;
async fn delete_user(&self, user_id: &str) -> Result<()>;
async fn count_by_status(&self) -> Result<HashMap<String, u64>>;
async fn list_users(&self, offset: u64, limit: u64) -> Result<Vec<User>>;
```

### SessionRepository (`ports/repository.rs`)

```rust
async fn store_refresh_token(&self, session: &Session) -> Result<()>;
async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
async fn revoke_session(&self, token_hash: &str) -> Result<()>;
async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
async fn count_active_sessions(&self) -> Result<u64>;
async fn cleanup_expired_sessions(&self) -> Result<u64>;  // returns rows deleted
```

User and session storage are separate traits so a deployment can back sessions with a fast
embedded or in-memory store while keeping users in a durable SQL/DynamoDB table. A single
adapter (DynamoDB, Postgres, SQLite) may implement both.

### KeyManager (`ports/key_manager.rs`)

```rust
async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>>;
async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool>;
async fn public_jwk(&self) -> Result<serde_json::Value>;
fn algorithm(&self) -> &str;     // "EdDSA", "ES256", …
fn key_id(&self) -> &str;        // JWT kid
```

`verify` exists so the revoke flow can authenticate an access token JWT before revoking the
user's sessions.

`sign` returns signature bytes in the form the JWS serialization uses directly. For the ES\*
algorithms the KMS adapter converts the DER-encoded `Ecdsa-Sig-Value` returned by KMS Sign
into raw fixed-length `r || s` (64/96/132 bytes for ES256/384/512). RSA and PSS signatures
are already in JWS form and pass through unchanged. `verify` does not call KMS: it checks
the signature locally against the cached public key (the same SPKI fetched once for the
JWK), so revoking an access token costs no KMS round-trip. Local verification consumes the
raw `r || s` form directly, so no raw→DER conversion exists anywhere in the adapter.

### IdentityProvider (`ports/identity_provider.rs`)

```rust
async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens>;
async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims>;
async fn revoke_token(&self, token: &str) -> Result<()>;
fn provider_id(&self) -> &str;
```

### AuditLog (`ports/audit.rs`)

```rust
async fn emit(&self, event: &AuditEvent) -> Result<()>;
```

### UserSync (`ports/user_sync.rs`)

```rust
async fn notify_user_created(&self, user: &User) -> Result<()>;
async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()>;
async fn notify_user_deleted(&self, user_id: &str) -> Result<()>;
```

## Adapter inventory

| Port | Adapter | Module | Notes |
|---|---|---|---|
| UserRepository + SessionRepository | DynamoDB | `adapters/dynamo` | single-table, GSI1; see [08-persistence.md](08-persistence.md) |
| UserRepository + SessionRepository | Postgres | `adapters/postgres` | `users` + `sessions` tables, JSONB columns, `sqlx` |
| UserRepository + SessionRepository | SQLite | `adapters/sqlite` | JSON-as-TEXT, WAL mode, `sqlx` |
| SessionRepository | LMDB | `adapters/lmdb` | embedded; `heed`; `sessions` + `user_sessions` DBs |
| SessionRepository | Valkey/Redis | `adapters/valkey` | `fred`; `{prefix}session:{hash}`, `{prefix}user_sessions:{user_id}` set |
| KeyManager | AWS KMS | `adapters/kms` | RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on `OnceCell`; `Sign`/`GetPublicKey` |
| KeyManager | Local Ed25519 | `adapters/local_keys` | EdDSA only; PKCS#8 PEM from file or bytes |
| KeyManager | Noop | `adapters/noop` | every op errors; used in admin-only role |
| AuditLog | Stdout/stderr | `adapters/stdout_audit` | JSON lines; `Auto` routes error+ to stderr, else stdout |
| AuditLog | AWS SQS | `adapters/sqs_audit` | JSON message + `severity` attribute; FIFO auto-detected by `.fifo` suffix |
| AuditLog | Noop | `adapters/noop` | always `Ok(())` |
| IdentityProvider | Standard OIDC | `adapters/oidc` | Tier 1; discovery + JWKS cache |
| IdentityProvider | Apple | `providers/apple` | Tier 2; ES256 client JWT |
| UserSync | Webhook | `adapters/webhook` | HMAC-SHA256 signed POST, retry with backoff |
| UserSync | Noop | `adapters/noop` | always `Ok(())` |

The KMS adapter's JWKs are strict RFC 7517/7518: RSA `n`/`e` are Base64urlUInt with no
leading zero octets (`e = 65537` encodes as `AQAB`), and EC keys cover P-256, P-384, and
P-521, so every algorithm the adapter signs with has a published JWK at `/keys`.

The previous CloudTrail audit adapter has been removed; structured audit now flows through
the stdout/SQS/noop adapters (`crates/adapters/src/stdout_audit`, `sqs_audit`, `noop`).

## Webhook adapter contract

`POST` `application/json`, body `{ "event": "user.created"|"user.updated"|"user.deleted",
"timestamp": <RFC3339>, "data": <User> }`, authenticated by `X-Signature-256` carrying the
hex HMAC-SHA256 of the raw body under the configured secret. Any 2xx is success; 5xx or
timeout retries up to `retries` with exponential backoff; 4xx is not retried.

## Shared OIDC utilities (`adapters/shared`)

Reused by the OIDC and Apple providers:

- `jwks::JwksCache` — fetches and caches a remote JWKS behind a read/write lock with a TTL
  (default 1h); `with_ttl` overrides.
- `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration` into
  `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }`.
- `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` —
  the standard form-encoded `grant_type=authorization_code` POST.

## Mock adapters (`crates/test-utils`)

`MockRepository` (in-memory `HashMap`, implements both repository traits), `MockKeyManager`
(deterministic Ed25519 seed), `MockAuditLog` (collects events, `set_fail_mode` to inject
failures), `MockUserSync` (records calls), `MockIdentityProvider` (configurable exchange and
claims responses). These back the core unit tests and server E2E tests.

## Assumptions and open questions

### Assumptions

- Exactly one `KeyManager`, one `AuditLog`, and one `UserSync` are active per process; the
  user and session repositories may be the same adapter or two different ones.

### Decisions

- *Session-only stores.* **LMDB and Valkey implement `SessionRepository` only.** Sessions are
  high-churn and short-lived; pairing a fast session store with a durable user store is a
  supported topology (see the linux-sqlite and linux-postgres examples).
- *Noop adapters for role splitting.* **`NoopKeyManager`/`NoopUserSync` stand in for ports an
  admin- or exchange-only process doesn't need.** Lets the bootstrap skip building unused
  infrastructure without making the ports optional on `AppService`.
- *Errors mapped at the boundary.* **Each adapter converts its SDK/driver error into the
  domain `Error`.** Keeps cloud and SQL types out of `crates/core`.

### Open questions

- Audit currently has stdout and SQS network adapters; whether a generic HTTP/webhook audit
  sink (distinct from `UserSync`) is wanted is open.
