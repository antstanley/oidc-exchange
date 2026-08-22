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
| SessionRepository | Valkey/Redis | `adapters/valkey` | `fred`; `{prefix}session:{hash}`, `{prefix}user_sessions:{user_id}` set (TTL bumped via `EXPIRE … GT`), `{prefix}active_sessions` counter; atomic pipelined writes; cleanup prunes index sets and reconciles the counter |
| KeyManager | AWS KMS | `adapters/kms` | RS/PS/ES 256/384/512; ECDSA DER→raw JWS conversion on sign; local verify against the cached public key; JWK cached on `OnceCell`; `Sign`/`GetPublicKey` |
| KeyManager | Local Ed25519 | `adapters/local_keys` | EdDSA only; PKCS#8 PEM from file or bytes |
| KeyManager | Noop | `adapters/noop` | every op errors; used in admin-only role |
| AuditLog | Stdout/stderr | `adapters/stdout_audit` | JSON lines; `Auto` routes error+ to stderr, else stdout |
| AuditLog | AWS SQS | `adapters/sqs_audit` | JSON message + `severity` attribute; FIFO auto-detected by `.fifo` suffix |
| AuditLog | Noop | `adapters/noop` | always `Ok(())` |
| IdentityProvider | Standard OIDC | `adapters/oidc` | Tier 1; discovery + JWKS cache |
| IdentityProvider | Apple | `providers/apple` | Tier 2; ES256 client JWT |
| UserSync | Webhook | `adapters/webhook` | HMAC-SHA256 over timestamp + delivery id + body; one signature per delivery, reused across the retry burst |
| UserSync | Noop | `adapters/noop` | always `Ok(())` |

The KMS adapter's JWKs are strict RFC 7517/7518: RSA `n`/`e` are Base64urlUInt with no
leading zero octets (`e = 65537` encodes as `AQAB`), and EC keys cover P-256, P-384, and
P-521, so every algorithm the adapter signs with has a published JWK at `/keys`.

The previous CloudTrail audit adapter has been removed; structured audit now flows through
the stdout/SQS/noop adapters (`crates/adapters/src/stdout_audit`, `sqs_audit`, `noop`).

## Webhook adapter contract

`POST` `application/json`, body `{ "event": "user.created"|"user.updated"|"user.deleted",
"timestamp": <RFC3339>, "data": <User> }`. Three headers authenticate and identify the
delivery: `X-Webhook-Timestamp` carrying the RFC3339 instant the delivery was minted,
`X-Webhook-Delivery-Id` carrying a ULID unique per delivery, and `X-Signature-256` carrying
`sha256=` followed by the hex HMAC-SHA256, under the configured secret, of the canonical
string `<timestamp> "." <delivery-id> "." <raw body>`. The separators make the input
unambiguous, and the algorithm prefix makes a future algorithm change expressible. The
signature and the delivery id are minted **once**, outside the retry loop, so every attempt
in a retry burst carries the same id and the same signature. The in-body `timestamp`
remains, so a receiver written against the previous contract keeps parsing.

A conforming receiver **must** reject a delivery whose `X-Webhook-Timestamp` is outside a
±5-minute tolerance of its own clock; **must** deduplicate on `X-Webhook-Delivery-Id`,
retaining seen ids for at least the tolerance window; and **must** treat a repeated delivery
id as a retry of one delivery rather than as an anomaly. It should verify the signature
before parsing the body, which makes the header timestamp an authenticated value. Any 2xx is
success; 5xx or timeout retries up to `retries` with exponential backoff; 4xx is not
retried. The client follows no redirects: re-signing semantics across hosts are undefined,
and forwarding a signed body to an unconfigured host is a credential-adjacent leak.

## Shared OIDC utilities (`adapters/shared`)

Reused by the OIDC and Apple providers. Every outbound request to a provider endpoint is
issued by `transport::ProviderTransport` and by nothing else — no adapter calls `reqwest`
against a provider directly. The transport issues the request through the single shared
process-wide `reqwest::Client` (5s connect timeout, 10s total request timeout, redirects
disabled), reads the response status **before** any body is read, and reads the body
through a ceiling that fails at the limit rather than after it. It returns an
`UpstreamBody`, which exposes `parsed::<T>()` for a known success shape and hands a
non-success response to `upstream::error_detail`. A body that exceeds the ceiling is a
distinct `ProviderError` naming the limit and the endpoint, so it is alertable as a
provider fault rather than indistinguishable from a parse failure.

- `transport::ProviderTransport` — `get_json::<T>(url)` and `post_form::<T>(url, params)`.
  The four provider fetch shapes — discovery, JWKS, token endpoint, revocation — are its
  only callers, across five call sites (`discovery::discover`, `JwksCache::fetch_keys`,
  `token_endpoint::exchange_code`, and each provider's `revoke_token`).
- `keys::VerificationKeySet` — the only way a JWK becomes something a signature is verified
  with. Built from a fetched JWKS and an admitted-algorithm set; held behind an `Arc` and
  handed out by cheap clone, never deep-cloned per request. Its constructor is where key
  eligibility lives: an entry is dropped when `use` is present and is not `"sig"`, when
  `key_ops` is present and does not contain `"verify"`, when its declared `alg` is outside
  the caller's admitted set, or when its `alg` is inconsistent with its `kty`/`crv`. Lookup
  by `kid` returns a `VerificationKey` carrying its algorithm as data, so no caller
  re-derives one. When entries share a `kid`, exactly one may be eligible: the eligible
  entry is returned regardless of array order, and several *eligible* entries under one
  `kid` are an ambiguity error for the whole set rather than a silent pick.
- `jwks::JwksCache` — fetches and caches a remote JWKS as an `Arc<VerificationKeySet>`
  behind a TTL (default 1h); `with_ttl` overrides; the constructor takes the provider's
  admitted-algorithm set. A non-2xx, oversized, or malformed JWKS response is a
  `ProviderError` and is never cached. No lock that protects the cached value is held
  across the fetch: refill elects one fetcher through a single-flight permit, releases the
  cache guard, fetches, and re-acquires the guard only to store. Callers that arrive during
  an in-flight refill are served the stale-but-parsable set if one exists and otherwise
  await the permit; a `kid` absent from a stale set still falls through to the rate-limited
  forced refetch, so staleness fails closed. `refresh()` records its rate-limit timestamp,
  releases that guard, and only then fetches. When a token's `kid` is not in the cached
  set, the cache refetches once (rate-limited by a 30s minimum refresh interval) before the
  provider rejects the token.
- `discovery::discover(issuer, permitted)` — fetches and parses
  `.well-known/openid-configuration` into
  `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }`. The body
  is read through the transport's ceiling, so an oversized document is rejected before it
  is materialised. Each endpoint the document supplies must have an origin in the
  provider's pinned endpoint-origin set; a document naming an origin outside it is
  rejected — once enforcement is enabled — with a `ProviderError` naming the endpoint, the
  origin, and the permitted set.
- `origins` — the pinned endpoint-origin set and its vocabulary:
  `EndpointOrigins` (built once from the issuer's origin, the origins of explicitly
  configured endpoint overrides, and every declared `endpoint_origins` entry; capped at
  `MAX_ENDPOINT_ORIGINS = 16`), `parse_https_origin` (strict `https` bare-origin parse for
  operator-declared entries), `origin_of` (lenient normalized-origin extraction for the
  issuer, configured overrides, and observed endpoints), `check_pinned_origin` (the
  warn-or-reject decision), and `OriginCheckMode::{Warn, Enforce}` behind the shipped
  `ENDPOINT_ORIGIN_CHECK_MODE = Warn` release constant. The service ships in warning mode
  for one release: an undeclared origin produces a structured warning naming the endpoint,
  its observed origin, and the permitted set, and the deployment is served unchanged.
  Flipping the constant to `Enforce` is a separate release-owner decision made after that
  warning window — it is not part of this change.
- `token_endpoint::exchange_code(endpoint, client_id, client_secret, code, redirect_uri)` —
  the standard form-encoded `grant_type=authorization_code` POST.
- `http::read_bounded_bytes(response)` — the accumulator both body readers share: it reads
  at most `MAX_UPSTREAM_BODY_BYTES` (64 KiB) and fails at the ceiling rather than after it,
  so the ceiling is one constant applied to success and failure bodies alike. `HttpsUrl`,
  `read_bounded_bytes`, and `upstream::error_detail` are vendored prerequisites from
  sibling changes (`fail_closed_across_config_and_adapters`,
  `eliminate_secret_leakage_in_logs_and_spans`); the owning PRs reconcile ownership.

### Cache lock discipline

The cache's correctness rules, in one place:

- **Single-flight election.** Expired-TTL callers race for a one-permit
  `tokio::sync::Semaphore` (`MAX_CONCURRENT_REFILLS = 1`); `try_acquire` before `acquire`
  keeps the cold-cache case correct, so the first arrival is elected without yielding.
- **No guard across the network await.** The elected caller re-checks freshness under the
  write guard, releases it, fetches with no data-lock guard alive across the network
  await, then re-acquires the guard only to store. The committed `clippy.toml`
  (`await-holding-invalid-types` for tokio's guard types) makes a regression a compile-time
  failure at the binding site.
- **Stale-serving rule.** A caller that loses the election is served the
  stale-but-parsable set when one exists and returns immediately: an expired entry is
  stale, not untrusted, and a `kid` absent from it still fails closed through the
  rate-limited forced refetch. Only a cold cache queues on the permit.
- **Store before the permit is released.** The winner stores the fresh entry before
  letting the permit drop, so everyone queued behind a successful wave finds the entry and
  spends no request of their own.
- **Forced refresh is time-serialized outside the permit.** `refresh()` writes its
  `MIN_REFRESH_INTERVAL` timestamp before the fetch, releases that guard before the await,
  and racing callers are declined by the timestamp alone — the TTL-refill and kid-miss
  triggers stay independently bounded.

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
- *One transport for provider traffic, one client for webhook delivery.* **Every provider
  request goes through `ProviderTransport`; webhook delivery keeps its own
  `reqwest::Client`.** The provider client's timeouts are compile-time constants because a
  provider is infrastructure this service does not control; the webhook client's timeout is
  operator-configured because the receiver is the operator's own. Two clients, two owners —
  but the same rule about who may issue a request applies to both, and neither is called
  from an adapter directly.
- *Key eligibility is a constructor, not a call-site check.* **`VerificationKeySet` is the
  only way a JWK becomes a verification key, and its constructor applies the RFC 7517
  §4.2–4.3 purpose filter.** Two independent `find_jwk` implementations both selected on
  `kid` alone and had already drifted on algorithm handling; fixing both copies leaves the
  copies. Concentrating the filter makes it worth testing exhaustively, which is not true of
  two copies — at the cost of it becoming a single trusted component, which the
  cross-provider corpus exists to hold.
- *A signed delivery is bound to one occasion.* **The webhook HMAC covers the timestamp and
  the delivery id as well as the body.** Origin authenticity ("this came from the holder of
  the secret") was already established; this adds "and this is the first time you have been
  asked to act on it". The sender emits byte-identical POSTs on retry, so without a
  delivery id a receiver cannot distinguish a retry from an injected replay — the one gap
  no receiver-side diligence closes.

### Open questions

- Audit currently has stdout and SQS network adapters; whether a generic HTTP/webhook audit
  sink (distinct from `UserSync`) is wanted is open.
