# Ports and Adapters

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/core/src/ports, crates/adapters

> **Read first:** [.specs/architecture-principles.md](../../architecture-principles.md) for
> the inward-dependency rule and why ports are `Box<dyn Trait>`.

The core declares seven port traits in `crates/core/src/ports/`. Adapters in
`crates/adapters/`, `crates/providers/`, and — for the in-process rate limiter —
`crates/server/` implement them. Every method returns the core's `Result<T>`; adapters
convert native errors into the domain [`Error`](04-http-api.md) at the boundary.

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
async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution>;
async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool>;
async fn revoke_session(&self, token_hash: &str) -> Result<()>;
async fn revoke_family(&self, family_id: &str) -> Result<u64>;
async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
async fn count_active_sessions(&self) -> Result<u64>;
async fn cleanup_expired_sessions(&self) -> Result<u64>;  // returns rows deleted
async fn put_single_use(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool>;
async fn take_single_use(&self, key: &str) -> Result<bool>;
```

```rust
pub enum RefreshResolution {
    /// The hash is the family's live generation.
    Live(Session),
    /// The hash is retired and the successor it names is still the family's
    /// live generation. `live` is that successor.
    Superseded { live: Session, retired_at: DateTime<Utc> },
    /// The hash is retired and its successor is no longer live.
    Retired { family_id: String, user_id: String, retired_at: DateTime<Utc> },
    /// No live generation and no retained retirement record matches.
    Unknown,
}
```

The port classifies; it does not decide policy. `Superseded` is a storage fact — the
successor pointer still names the live generation — and the grace window that turns it into
either a rotation or a reuse alarm is evaluated once in the core against
`token.refresh_rotation_grace`, not five times across the adapters.

Five obligations attach to the session port. They are contract, not description: an adapter
either meets them or it does not ship.

| | Obligation |
|---|---|
| **SR1** | **Consistency.** `resolve_refresh_token` is strongly consistent with the most recent write. Its negative and retired answers *are* security outcomes — an eventually consistent read turns a revoked token into a live one and a reuse alarm into a silent rejection. |
| **SR2** | **Atomicity.** `rotate_refresh_token` applies its three effects — delete the live session, write the retirement record, install the replacement — as one atomic unit conditioned on `live_hash` still being live, or applies none of them. A partial application either strands the old generation as still-valid or locks the holder out of a session they legitimately hold. |
| **SR3** | **Single live generation.** At most one generation of a family is live at any instant, under concurrent redemption. Two callers redeeming the same hash produce exactly one `true` return. |
| **SR4** | **Retirement durability.** By the time a rotation is observable, the retirement record it wrote is readable. A rotation whose replacement is visible before its retirement record leaves a window in which reuse reads as `Unknown`. |
| **SR5** | **Revocation completeness.** `revoke_family` removes the family's live generation and every retained retirement record, and returns the count removed, or it errors. `revoke_all_user_sessions` gives the same removal guarantee across all of a user's families (its `Result<()>` signature is unchanged). Neither reports success for work it did not do. |

`store_refresh_token` writes the generation-0 row of a new family. `get_session_by_refresh_token`
remains for `/revoke`, which needs only liveness.

User and session storage are separate traits so a deployment can back sessions with a fast
embedded or in-memory store while keeping users in a durable SQL/DynamoDB table. A single
adapter (DynamoDB, Postgres, SQLite) may implement both.

## Session-store conformance suite

`crates/test-utils/src/session_contract.rs` exports the obligations above as generic
assertions over any `impl SessionRepository`. Every session adapter — DynamoDB, Postgres,
SQLite, LMDB, Valkey — and `MockRepository` invoke the same suite from their own test
module, so the guarantee is a property the project asserts rather than one it assumes. The
suite covers, at minimum:

- a redemption returns a new generation and the presented one no longer resolves as `Live`;
- two concurrent `rotate_refresh_token` calls against the same `live_hash` produce exactly
  one `true` (SR3);
- a failed compare-and-swap leaves the store byte-identical (SR2);
- a retirement record is readable the instant its rotation is (SR4);
- a generation retired more than one rotation ago resolves as `Retired`, not `Unknown`;
- `revoke_family` removes the live generation and every retirement record, and its count
  matches (SR5);
- `resolve_refresh_token` immediately after `revoke_session` returns `Unknown` (SR1);
- the replacement's `expires_at` equals the retired generation's.

Adapters needing a live backend keep their existing `#[ignore]` gating and environment-variable
URLs; the suite runs against them in the integration job, and against SQLite, LMDB and
`MockRepository` on every build.

The single-use pair backs nonces and assertion-replay markers (see
[01-domain-model.md](01-domain-model.md) → SingleUseRecord). `put_single_use` is an atomic
insert-if-absent returning `true` when *this* call wrote the record and `false` when a live
record already held the key; `take_single_use` is an atomic remove-and-report returning
`true` when a live record was found and is now gone. **Both treat a record whose
`expires_at` has passed as absent**, so correctness never depends on the reaper having run:
an expired nonce cannot be taken, and an expired marker's key is reusable.
`cleanup_expired_sessions` also reclaims expired single-use records where the store has no
native expiry, and its return count covers sessions and single-use records. Nonces and
markers are short-lived and high-churn, exactly like sessions, so they live wherever
sessions live — the `[session_repository]` store when one is configured, otherwise the
`[repository]` store — with no new configuration surface. `key` is always a namespaced
digest (`"nonce:<sha256hex>"` or `"assertion:<provider>:[d:]<sha256hex>"`); storage never
holds raw nonce or raw assertion material.

### KeyManager (`ports/key_manager.rs`)

```rust
async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>>;
async fn verify(&self, payload: &[u8], signature: &[u8]) -> Result<bool>;
async fn public_jwk(&self) -> Result<serde_json::Value>;
fn algorithm(&self) -> &str;     // "EdDSA", "ES256", …
fn key_id(&self) -> &str;        // JWT kid
```

`verify` exists so `AppService::validate_access_token` can authenticate a service-minted
access token before any of its claims is read. The signature check is the first step of that
validation, not the whole of it: origin is established here, and validity — type, issuer,
audience and window — by the claim checks that follow
([03-service-flows.md](03-service-flows.md)).

`algorithm()` returns the algorithm **derived from the key material the adapter loaded**, not
the operator's configured string. The local adapter parses an Ed25519 PKCS#8 PEM and reports
`EdDSA`; the KMS adapter reports the algorithm its configured JWS name maps to, checked against
the SPKI it fetches for the JWK. Config load compares the declared `key_manager.*.algorithm`
against this value and fails when they disagree, so the `alg` in every issued JWT header, the
JWK at `GET /keys`, and `id_token_signing_alg_values_supported` in the discovery document all
describe the key that actually signs.

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
fn client_id(&self) -> &str;
```

### RateLimiter (`ports/rate_limit.rs`)

```rust
async fn check_and_consume(&self, key: &RateLimitKey) -> Result<RateLimitDecision>;

enum RateLimitKey {
    ClientAddr(IpAddr),
    ClientAddrFailure(IpAddr),
    Subject { provider: Option<String>, subject_hash: String },
    Provider(String),
}

enum RateLimitDecision {
    Allow,
    Deny { retry_after_secs: u64 },
}
```

One call consumes one unit against one key and reports whether the caller may proceed.
`Subject.subject_hash` is a SHA-256 hex digest of the provider subject, so limiter state never
holds a raw provider subject. A limiter error is logged and callers proceed; the in-process
limiter is a backstop, not a global control.

`client_id` reports the audience the provider pins, so the core's `azp` check does not
have to reach into `[providers.<name>]` config. `validate_id_token`'s signature is
unchanged — the binding controls read the claims it already returns.

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
| UserRepository + SessionRepository | DynamoDB | `adapters/dynamo` | single-table, GSI1; sessions, retirement items, single-use records, and a transactionally maintained per-user roster; see [08-persistence.md](08-persistence.md) |
| UserRepository + SessionRepository | Postgres | `adapters/postgres` | `users` + `sessions` + `retired_refresh_tokens` + `single_use` tables, JSONB columns, `sqlx` |
| UserRepository + SessionRepository | SQLite | `adapters/sqlite` | JSON-as-TEXT, WAL mode, `sqlx`; same four tables |
| SessionRepository | LMDB | `adapters/lmdb` | embedded; `heed`; five DBs — `sessions`, `user_sessions`, `retired_tokens`, `family_index`, `single_use`; batched cleanup |
| SessionRepository | Valkey/Redis | `adapters/valkey` | `fred`; `{prefix}session:{hash}`, `{prefix}retired:{hash}`, `{prefix}family:{family_id}` set, `{prefix}user_sessions:{user_id}` set (TTL bumped via `EXPIRE … GT`), `{prefix}active_sessions` counter, `{prefix}single_use:{digest}` (`SET NX EX` claim / `GETDEL` burn); Lua rotation and pipelined writes; cleanup prunes index sets and reconciles the counter |
| KeyManager | AWS KMS | `adapters/kms` | RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on `OnceCell`; `Sign`/`GetPublicKey` |
| KeyManager | Local Ed25519 | `adapters/local_keys` | EdDSA only; PKCS#8 PEM from file or bytes |
| KeyManager | Noop | `adapters/noop` | every op errors; used in admin-only role |
| RateLimiter | In-process | `server/middleware/throttle` | fixed window per key, bounded map with expiry eviction; per-process, not global |
| RateLimiter | Noop | `adapters/noop` | always `Allow`; selected when `rate_limit.enabled = false` |
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
  `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }`. A non-success
  HTTP status is rejected before the body is read (`ProviderError` naming the issuer and status),
  matching `JwksCache`'s handling of the same failure; the parsed `issuer` must then equal the
  configured issuer per RFC 8414 §3.3.
- `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` —
  the standard form-encoded `grant_type=authorization_code` POST.

## Mock adapters (`crates/test-utils`)

`MockRepository` (in-memory `HashMap`, implements both repository traits, runs the
session-store conformance suite), `MockKeyManager`
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
