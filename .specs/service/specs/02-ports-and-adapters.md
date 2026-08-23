# Ports and Adapters

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** crates/core/src/ports, crates/adapters

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

`create_user` fails with `Error::Conflict` when a live user with the same
`(provider, external_id)` already exists — every adapter maps its native uniqueness
violation (SQL unique index, DynamoDB transaction cancellation) to this variant so callers
can distinguish "already registered" from an infrastructure failure. `update_user` applies
a patch atomically with respect to concurrent updates using the user's integer `version`:
the write is conditioned on the version that was read and increments it, so two racing
patches serialize and neither silently overwrites the other's fields. `delete_user` frees
the `(provider, external_id)` key: after a delete, `get_user_by_external_id` returns
nothing for that identity and `create_user` succeeds as a new user.

### SessionRepository (`ports/repository.rs`)

```rust
async fn store_refresh_token(&self, session: &Session) -> Result<()>;
async fn get_session_by_refresh_token(&self, token_hash: &Secret<String>) -> Result<Option<Session>>;
async fn revoke_session(&self, token_hash: &Secret<String>) -> Result<()>;
async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
async fn count_active_sessions(&self) -> Result<u64>;
async fn cleanup_expired_sessions(&self) -> Result<u64>;  // returns rows deleted
```

The token-hash parameters are `&Secret<String>` rather than `&str`, so an adapter that leaves
them out of its span `skip(...)` fails to compile instead of publishing the session lookup
key as a span field. An adapter reaches the raw digest through `expose()` at the point it
builds a store key.

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
| SessionRepository | Valkey/Redis | `adapters/valkey` | `fred`; `{prefix}session:{hash}`, `{prefix}user_sessions:{user_id}` set (TTL bumped via `EXPIRE … GT`), `{prefix}active_sessions` counter; atomic pipelined writes; cleanup prunes index sets and reconciles the counter |
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

Reused by the OIDC and Apple providers. All outbound HTTP goes through a single shared
`reqwest::Client` per process with a 5s connect timeout, a 10s total request timeout
(compile-time constants, not configuration), and redirects disabled; a hung or slow provider
fails the request rather than stalling `/token`. On the token-endpoint and revocation paths a
non-2xx response is an error whose body reaches nothing but `upstream::error_detail`; success
payloads are parsed only from 2xx bodies.

- `jwks::JwksCache` — fetches and caches a remote JWKS behind a read/write lock with a TTL
  (default 1h); `with_ttl` overrides. A non-2xx JWKS response is a `ProviderError` and is
  never cached. When a token's `kid` is not in the cached set, the cache refetches once
  (rate-limited by a 30s minimum refresh interval) before the provider rejects the token, so
  upstream key rotation takes effect immediately instead of at TTL expiry.
- `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration` into
  `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }` and errors if
  the document's `issuer` does not equal the configured issuer (RFC 8414 §3.3).
- `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` —
  the standard form-encoded `grant_type=authorization_code` POST. Both outcomes read the body
  through `http::read_bounded`; a non-2xx response becomes a `ProviderError` detail via
  `upstream::error_detail`, and an over-ceiling 2xx payload fails closed rather than parsing
  truncated JSON. A 2xx response without an `id_token` is an error, not an empty string.
- `http::read_bounded(provider, response)` — reads a response body to at most
  `MAX_UPSTREAM_BODY_BYTES` (64 KiB) and returns it as `Secret<String>`, so an upstream cannot
  choose how many bytes the service retains and the body cannot be formatted by accident.
  Reaching the ceiling truncates (with a structured, body-free warning naming the provider);
  non-UTF-8 bytes convert lossily; stream failures yield a `ProviderError` carrying only
  transport text.
- `upstream::error_detail(status, body)` — the only way to build a `ProviderError` detail
  from upstream bytes. It prefers the structured RFC 6749 error object (`error`, optionally
  `error_description`); failing that it returns the status, the body's byte length, and a
  bounded excerpt (256 characters) produced by a redacting function that percent-decodes first
  and then masks the values of `token`, `refresh_token`, `client_secret`, `code`, and any bare
  compact JWS. Every field it emits passes the same redact-and-clamp pipeline. It consumes
  the `Secret<String>` and returns a plain `String`, so it is the single audited point at
  which upstream text becomes loggable.

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
