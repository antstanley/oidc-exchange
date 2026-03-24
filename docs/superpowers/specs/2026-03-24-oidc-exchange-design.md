# OIDC Exchange Service — Design Specification

A Rust service that validates ID tokens from third-party OIDC providers and exchanges them for self-issued access and refresh tokens. Built with hexagonal architecture for pluggable infrastructure, configurable via TOML, and deployable as a Lambda function or long-lived server from a single binary.

## Table of Contents

1. [Overview](#1-overview)
2. [Crate Structure](#2-crate-structure)
3. [Port Traits](#3-port-traits)
4. [Domain Model](#4-domain-model)
5. [Service Orchestration](#5-service-orchestration)
6. [HTTP Layer](#6-http-layer)
7. [Provider System](#7-provider-system)
8. [Configuration](#8-configuration)
9. [Telemetry](#9-telemetry)
10. [Error Handling](#10-error-handling)
11. [Schemas](#11-schemas)
12. [Testing Strategy](#12-testing-strategy)

---

## 1. Overview

### Problem

Applications that authenticate users via external OIDC providers (Google, Apple, atproto/Bluesky) need to validate the provider's ID token, then issue their own short-lived access tokens for internal API authorization. The reference implementation (PetSpeak backend, TypeScript/Node.js) works but has gaps: 31-day JWT expiry, no audience validation on ID tokens, no self-issued refresh tokens, and tightly coupled infrastructure.

### Solution

A standalone Rust service that:

- Accepts authorization codes from configured OIDC providers
- Validates ID tokens with full claim verification (signature, `iss`, `aud`, `exp`)
- Issues short-lived access tokens (configurable, default 15 minutes) signed via pluggable key management
- Issues long-lived, reusable refresh tokens (configurable, default 30 days), stored hashed
- Provides pluggable infrastructure via hexagonal architecture (database, key management, audit, user sync)
- Supports three tiers of identity providers: standard OIDC (config-only), OIDC-with-quirks (Apple), and non-OIDC (atproto)
- Enforces registration policy: open or existing-users-only mode, with optional email domain/subdomain allowlist
- Provides OpenTelemetry-based observability via `tracing` with pluggable exporters (OTLP, X-Ray, stdout)
- Runs as a Lambda function or long-lived server from the same binary
- Ships as a single binary configurable via TOML

### Key Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Language | Rust | User requirement |
| HTTP framework | axum | Tower-based, Lambda-compatible, strong ecosystem |
| Architecture | Hexagonal, trait objects (dynamic dispatch) | All ports are IO-bound; runtime adapter selection from config outweighs nanosecond dispatch overhead |
| Config format | TOML | Rust-idiomatic, supports comments, good for structured config |
| Access token TTL | 15 minutes (configurable) | Short-lived JWTs are effectively irrevocable; short TTL limits blast radius |
| Refresh token format | Opaque (random), stored hashed | Revocable, no information leakage |
| Refresh token behavior | Long-lived, reusable until expiry or revocation | Most client libraries expect reusable refresh tokens |
| Test runner | cargo-nextest | Process isolation, parallel execution, structured output |

### Out of Scope (v1)

These are intentionally deferred. The audit event types and port traits are designed to accommodate them later without breaking changes:

- **Rate limiting** — expected to be handled externally (API Gateway, ALB, or reverse proxy). No rate limiting middleware in this service.
- **Key rotation** — KMS handles rotation transparently. For the local key manager, manual key rotation (replace file, restart) is sufficient for v1. Automated rotation with graceful rollover is a future enhancement.
- **Config hot-reload** — config is loaded at startup. Changes require a restart. Live config reload is a future enhancement.
- **Token introspection endpoint** (RFC 7662) — downstream services verify tokens using the JWKS endpoint. A dedicated `/introspect` endpoint is a future enhancement if opaque access tokens are needed.

---

## 2. Crate Structure

A Cargo workspace with the domain core isolated from adapters and the HTTP layer.

```
oidc-exchange/
├── Cargo.toml                    # workspace root
├── .config/
│   └── nextest.toml              # cargo-nextest configuration
├── config/
│   └── default.toml              # default configuration
├── schemas/
│   ├── datamodel.schema.json     # generic domain model (adapter-agnostic)
│   └── dynamodb/
│       └── table-design.json     # DynamoDB-specific table/GSI/access patterns
├── crates/
│   ├── core/                     # domain logic + port traits (zero AWS/HTTP deps)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── ports/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── repository.rs
│   │   │   │   ├── key_manager.rs
│   │   │   │   ├── audit.rs
│   │   │   │   ├── identity_provider.rs
│   │   │   │   └── user_sync.rs
│   │   │   ├── domain/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── token.rs
│   │   │   │   ├── user.rs
│   │   │   │   ├── session.rs
│   │   │   │   ├── audit.rs
│   │   │   │   ├── provider.rs
│   │   │   │   └── schema.rs     # types derived from/validated against JSON schema
│   │   │   ├── service/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── exchange.rs
│   │   │   │   ├── refresh.rs
│   │   │   │   ├── revoke.rs
│   │   │   │   └── user_admin.rs   # backing logic for /internal/* routes
│   │   │   ├── config.rs
│   │   │   └── error.rs
│   │   └── Cargo.toml            # minimal deps: serde, thiserror, async-trait
│   │
│   ├── adapters/                 # port implementations
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── dynamo/           # DynamoDB session store
│   │   │   │   ├── mod.rs
│   │   │   │   └── schema.rs    # attribute mappings per table-design.json
│   │   │   ├── kms/              # AWS KMS key manager
│   │   │   ├── local_keys/       # local key signing (in-memory keys)
│   │   │   ├── cloudtrail/       # CloudTrail Lake audit
│   │   │   ├── oidc/             # standard OIDC provider adapter
│   │   │   ├── webhook/          # HTTP webhook adapter for UserSync
│   │   │   └── shared/           # shared OIDC utilities (JWKS cache, discovery)
│   │   └── Cargo.toml            # AWS SDK, reqwest, jose, etc.
│   │
│   ├── providers/                # non-standard provider modules
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── apple.rs
│   │   │   └── atproto.rs
│   │   └── Cargo.toml
│   │
│   ├── server/                   # HTTP layer (axum routes, Lambda entrypoint)
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── routes/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── token.rs
│   │   │   │   ├── revoke.rs
│   │   │   │   ├── keys.rs
│   │   │   │   ├── well_known.rs
│   │   │   │   └── internal.rs
│   │   │   ├── middleware/
│   │   │   ├── telemetry.rs      # OTEL subscriber setup from config
│   │   │   └── state.rs
│   │   └── Cargo.toml            # axum, lambda_http, tower, tracing-opentelemetry
│   │
│   └── test-utils/               # dev-dependency: mock implementations
│       ├── src/
│       │   └── lib.rs
│       └── Cargo.toml
```

### Dependency Rules

- `core` has no knowledge of AWS, HTTP, or any adapter — only std + serde + async-trait + thiserror + tracing (for instrumentation, not OTEL-aware)
- `adapters` and `providers` depend on `core` (implement its traits)
- `server` depends on all three, wires everything together at startup
- `test-utils` depends on `core`, used as a dev-dependency by all other crates
- `providers` is separate from `adapters` because providers are user-facing integrations that may require complex, provider-specific logic (atproto with DPoP/PAR/DID), while adapters are infrastructure backends

### Distribution

The workspace compiles to a single binary from the `server` crate. All adapters and providers are compiled in. Configuration selects which ones are active at runtime. A user can `cargo install oidc-exchange`, drop a `config.toml`, and run.

---

## 3. Port Traits

All ports live in `crates/core/src/ports/`. They define contracts the core depends on — no adapter-specific types leak in. All return `Result<T>` using a domain-specific error type. Adapters map their internal errors into domain errors before returning.

### Repository

```rust
#[async_trait]
pub trait Repository: Send + Sync {
    // User operations
    async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>>;
    async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<User>>;
    async fn create_user(&self, user: &NewUser) -> Result<User>;
    async fn update_user(&self, user_id: &str, patch: &UserPatch) -> Result<User>;
    async fn delete_user(&self, user_id: &str) -> Result<()>;

    // Session/refresh token operations
    async fn store_refresh_token(&self, session: &Session) -> Result<()>;
    async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
    async fn revoke_session(&self, token_hash: &str) -> Result<()>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
}
// Note: User listing/search is deferred to a future enhancement. v1 supports lookup by ID or external_id only.
// User ID generation: `usr_` prefix + ULID, generated in the core service layer.
```

### KeyManager

```rust
#[async_trait]
pub trait KeyManager: Send + Sync {
    /// Sign a byte payload, return the signature
    async fn sign(&self, payload: &[u8]) -> Result<Vec<u8>>;

    /// Return the public key in JWK format for the JWKS endpoint
    async fn public_jwk(&self) -> Result<Jwk>;

    /// Key algorithm identifier (e.g., "EdDSA", "ES256")
    fn algorithm(&self) -> &str;

    /// Key ID for the JWT kid header
    fn key_id(&self) -> &str;
}
```

### AuditLog

```rust
#[async_trait]
pub trait AuditLog: Send + Sync {
    /// Emit an audit event.
    /// For blocking-configured severities, failure propagates and fails the operation.
    async fn emit(&self, event: &AuditEvent) -> Result<()>;
}
```

### IdentityProvider

```rust
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Exchange an authorization code for provider tokens
    async fn exchange_code(&self, code: &str, redirect_uri: &str) -> Result<ProviderTokens>;

    /// Validate an ID token and return verified claims
    async fn validate_id_token(&self, id_token: &str) -> Result<IdentityClaims>;

    /// Revoke a token at the provider (if supported)
    async fn revoke_token(&self, token: &str) -> Result<()>;

    /// Provider identifier (e.g., "google", "apple", "atproto")
    fn provider_id(&self) -> &str;
}
```

### UserSync

```rust
#[async_trait]
pub trait UserSync: Send + Sync {
    async fn notify_user_created(&self, user: &User) -> Result<()>;
    async fn notify_user_updated(&self, user: &User, changed_fields: &[&str]) -> Result<()>;
    async fn notify_user_deleted(&self, user_id: &str) -> Result<()>;
}
```

**Webhook adapter contract:**

The webhook adapter sends notifications as HTTP requests:

- **Method:** `POST`
- **Content-Type:** `application/json`
- **Authentication:** HMAC-SHA256 of the raw request body using the configured `secret`, sent in `X-Signature-256` header (hex-encoded)
- **Payload:**
  ```json
  {
    "event": "user.created",
    "timestamp": "2026-03-24T10:00:00Z",
    "data": { /* User object */ }
  }
  ```
  Event types: `user.created`, `user.updated`, `user.deleted`
- **Success:** Any 2xx response
- **Retry:** Up to `retries` attempts (configurable) with exponential backoff on 5xx or timeout

---

## 4. Domain Model

Core types that flow through the system. The JSON schema in `schemas/datamodel.schema.json` is derived from these.

### User

```rust
pub struct User {
    pub id: String,                        // internal ID, e.g., "usr_01ARZ3NDEK..."
    pub external_id: String,               // provider's sub claim / DID
    pub provider: String,                  // "google", "apple", "atproto"
    pub email: Option<String>,             // not all providers guarantee email
    pub display_name: Option<String>,
    pub metadata: HashMap<String, Value>,  // extensible fields from sync
    pub claims: HashMap<String, Value>,    // per-user private claims added to access token JWT
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum UserStatus {
    Active,
    Suspended,  // can't get new tokens, existing tokens still valid until expiry
    Deleted,    // soft delete, all sessions revoked
}

pub struct NewUser {
    pub external_id: String,
    pub provider: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

pub struct UserPatch {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub metadata: Option<HashMap<String, Value>>,
    pub claims: Option<HashMap<String, Value>>,  // replace entire claims map
    pub status: Option<UserStatus>,
}
```

### Session

```rust
pub struct Session {
    pub user_id: String,
    pub refresh_token_hash: String,   // SHA-256 hash of the opaque token
    pub provider: String,
    pub expires_at: DateTime<Utc>,
    pub device_id: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

The raw refresh token is only held in memory during issuance and returned to the client. Only the hash is stored.

### Token Types

```rust
/// Returned to the client from POST /token
pub struct TokenResponse {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,   // present on code exchange, absent on refresh
    pub token_type: &'static str,        // always "Bearer"
    pub expires_in: u64,                 // seconds
}

/// Claims embedded in the access token JWT
pub struct AccessTokenClaims {
    pub sub: String,                     // internal user ID
    pub iss: String,                     // this service's issuer URL
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
    #[serde(flatten)]
    pub custom: HashMap<String, Value>,  // merged: config template claims + user.claims
}
// Note: `aud` is a single string (v1 simplification). Multi-audience (array) is a future enhancement.

/// What we get back from a provider after code exchange
pub struct ProviderTokens {
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub access_token: Option<String>,
}

/// Verified claims extracted from a provider's ID token
pub struct IdentityClaims {
    pub subject: String,                 // provider's sub / DID
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub raw_claims: HashMap<String, Value>,
}
```

### Audit Events

```rust
pub struct AuditEvent {
    pub id: String,                      // ULID
    pub timestamp: DateTime<Utc>,
    pub severity: AuditSeverity,
    pub event_type: AuditEventType,
    pub actor: Option<String>,           // user ID if known
    pub provider: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub detail: HashMap<String, Value>,
    pub outcome: AuditOutcome,
}

/// Mapped to syslog severity levels (RFC 5424)
pub enum AuditSeverity {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

pub enum AuditEventType {
    TokenExchange,
    TokenRefresh,
    TokenRevocation,
    SessionRevoked,
    AllSessionsRevoked,
    UserCreated,
    UserUpdated,
    UserSuspended,
    UserDeleted,
    ValidationFailed,
    RegistrationDenied,  // domain not allowed or existing_users_only mode
    ProviderError,
    Unauthorized,
}

pub enum AuditOutcome {
    Success,
    Failure { reason: String },
}
```

### Provider Config

```rust
/// Loaded from TOML config — standard OIDC providers need only this
pub struct OidcProviderConfig {
    pub provider_id: String,
    pub issuer: String,                              // required — used for discovery
    pub client_id: String,
    pub client_secret: Option<String>,
    pub jwks_uri: Option<String>,                    // optional — discovered from issuer if absent
    pub token_endpoint: Option<String>,              // optional — discovered from issuer if absent
    pub revocation_endpoint: Option<String>,         // optional — discovered from issuer if absent
    pub scopes: Vec<String>,
    pub additional_params: HashMap<String, String>,
}
// For Tier 1 providers, only `issuer`, `client_id`, and `client_secret` are required.
// Endpoint fields are populated from the issuer's .well-known/openid-configuration at startup.
// If provided in config, they override the discovered values.
```

---

## 5. Service Orchestration

The `service/` module in core contains business logic that coordinates ports.

### AppService

```rust
pub struct AppService {
    repo: Box<dyn Repository>,
    keys: Box<dyn KeyManager>,
    audit: Box<dyn AuditLog>,
    user_sync: Box<dyn UserSync>,
    providers: HashMap<String, Box<dyn IdentityProvider>>,
    config: AppConfig,
}
```

### Token Exchange Flow (POST /token with grant_type=authorization_code)

1. Resolve provider from request (`provider` field maps to provider lookup)
2. `provider.exchange_code(code, redirect_uri)`
3. `provider.validate_id_token(id_token)` — verify signature via provider JWKS, validate `iss`, `aud`, `exp`
4. **User lookup and registration policy check:**
   - `repo.get_user_by_external_id(claims.subject)`:
     - If Some(user) and `user.status != Active` → reject, audit `Unauthorized`
     - If Some(user) and `user.status == Active` → proceed to step 5 (existing users bypass registration policy)
     - If None → apply registration policy:
       - If `domain_allowlist` is configured:
         - Extract domain from `claims.email`
         - If no email claim → reject with `access_denied`, audit `RegistrationDenied`
         - If domain not in allowlist (exact or wildcard match) → reject with `access_denied`, audit `RegistrationDenied`
       - If `mode == "open"` → `repo.create_user(new_user)`, audit `UserCreated` (Notice)
       - If `mode == "existing_users_only"` → reject with `access_denied`, audit `RegistrationDenied`
5. Generate refresh token (random 256-bit, base64url encoded)
6. `repo.store_refresh_token(Session { token_hash: sha256(token), ... })`
7. `keys.sign(access_token_claims)` to produce JWT
8. Audit: emit `TokenExchange` outcome (Notice, blocking per config)
9. `user_sync.notify_user_created(user)` if new user — **non-blocking**: sync failures are logged via `tracing::warn!` and do not fail the exchange
10. Return `TokenResponse { access_token, refresh_token, ... }`

### Token Refresh Flow (POST /token with grant_type=refresh_token)

1. Hash the presented refresh token
2. `repo.get_session_by_refresh_token(hash)` — if None or expired, reject, audit `Unauthorized`
3. `repo.get_user_by_id(session.user_id)` — if `status != Active`, reject, audit `Unauthorized`
4. `keys.sign(new_access_token_claims)` to produce JWT
5. Audit: emit `TokenRefresh` (Info, non-blocking)
6. Return `TokenResponse { access_token, token_type, expires_in }` (no new refresh token)

### Revocation Flow (POST /revoke)

1. Accept either `refresh_token` or `access_token` with `token_type_hint`
2. If refresh_token: hash it, `repo.revoke_session(hash)`
3. If access_token: decode JWT, extract `sub`, `repo.revoke_all_user_sessions(user_id)` (since individual JWTs can't be revoked)
4. Optionally revoke at upstream provider if supported
5. Audit: emit `TokenRevocation` (Notice, blocking per config)
6. Return `200 OK` (per RFC 7009, always 200 even if token unknown)

### Audit Blocking Logic

```rust
async fn emit_audit(&self, event: AuditEvent) -> Result<()> {
    match self.audit.emit(&event).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Always emit to stdout/stderr as fallback
            let serialized = serde_json::to_string(&event)?;
            if event.severity as u8 <= AuditSeverity::Error as u8 {
                eprintln!("{serialized}");
            } else {
                println!("{serialized}");
            }

            if event.severity as u8 <= self.config.audit.blocking_threshold as u8 {
                // Severity meets blocking threshold — fail the operation
                Err(e)
            } else {
                tracing::warn!(error = %e, "audit provider down, event emitted to std stream");
                Ok(())
            }
        }
    }
}
```

On audit provider failure, events are always written to stderr (Error and above) or stdout (Warning and below) as structured JSON. In Lambda this means CloudWatch Logs captures them. In containers, the log driver captures them.

### Custom Claims Resolution

```toml
[token.custom_claims]
org = "example"
role = "{{ user.metadata.role | default: 'user' }}"
tier = "{{ user.metadata.membership | default: 'free' }}"
```

Custom claims in the access token JWT are built by merging two sources, in order:

1. **Config template claims** (`[token.custom_claims]`) — resolved against the User model at token issuance time
2. **Per-user claims** (`user.claims`) — set via the internal API, merged on top of config claims

Per-user claims take precedence over config claims with the same key. This allows config to set org-wide defaults while the internal API overrides per user (e.g., a user with an elevated role).

Config template syntax supports only:

- Static strings: `org = "example"` — value used as-is
- Field references: `{{ user.email }}` — dot-notation access into User fields and metadata
- Default filter: `{{ user.metadata.role | default: 'user' }}` — fallback if field is null/missing

No loops, conditionals, or other template features. If a referenced field is missing and no default is provided, the claim is omitted from the token.

Reserved claim names (`sub`, `iss`, `aud`, `iat`, `exp`) cannot be overridden by either source — they are silently ignored if present.

---

## 6. HTTP Layer

### Routes

| Method | Path | Purpose |
|---|---|---|
| POST | `/token` | Token exchange and refresh |
| POST | `/revoke` | Token revocation |
| GET | `/keys` | JWKS endpoint |
| GET | `/.well-known/openid-configuration` | OIDC discovery document |
| GET | `/health` | Health check (200 if operational) |
| POST | `/internal/users` | Create/upsert user (trusted) |
| GET | `/internal/users/{id}` | Get user (trusted) |
| PATCH | `/internal/users/{id}` | Update user (trusted) |
| DELETE | `/internal/users/{id}` | Soft-delete user (trusted) |
| GET | `/internal/users/{id}/claims` | Get user's private claims (trusted) |
| PUT | `/internal/users/{id}/claims` | Replace user's private claims (trusted) |
| PATCH | `/internal/users/{id}/claims` | Merge into user's private claims (trusted) |
| DELETE | `/internal/users/{id}/claims` | Clear all user's private claims (trusted) |

### POST /token

Follows RFC 6749 conventions. `grant_type` determines the flow:

```
Content-Type: application/x-www-form-urlencoded

# Code exchange
grant_type=authorization_code
&code=AUTH_CODE
&redirect_uri=https://app.example.com/callback
&provider=google

# Refresh
grant_type=refresh_token
&refresh_token=dGhpcyBpcyBhIHJlZnJlc2g...
```

Response:
```json
{
  "access_token": "eyJ...",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "dGhpcyBpcyBh..."
}
```

The `provider` field replaces the `iss` field from the reference implementation. The client specifies which provider it authenticated with by name, not raw issuer URL.

### POST /revoke

Per RFC 7009:

```
Content-Type: application/x-www-form-urlencoded

token=dGhpcyBpcyBh...
&token_type_hint=refresh_token
```

Always returns `200 OK` with empty body.

### GET /keys

```json
{
  "keys": [
    {
      "kty": "OKP",
      "crv": "Ed25519",
      "alg": "EdDSA",
      "use": "sig",
      "kid": "key-2024-01",
      "x": "..."
    }
  ]
}
```

### GET /.well-known/openid-configuration

```json
{
  "issuer": "https://auth.example.com",
  "jwks_uri": "https://auth.example.com/keys",
  "token_endpoint": "https://auth.example.com/token",
  "revocation_endpoint": "https://auth.example.com/revoke",
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "response_types_supported": ["code"],
  "subject_types_supported": ["public"],
  "id_token_signing_alg_values_supported": ["EdDSA"]  // dynamically populated from key_manager.algorithm()
}
```

### Internal Routes Authentication

`/internal/*` routes use separate middleware. Configured via:

```toml
[internal_api]
auth_method = "shared_secret"
shared_secret = "${INTERNAL_API_SECRET}"
```

Callers pass `Authorization: Bearer <secret>`. Middleware compares using constant-time comparison.

### AppState

```rust
pub struct AppState {
    pub service: Arc<AppService>,
    pub config: Arc<AppConfig>,
}
```

Axum's state extraction passes `AppState` to handlers.

### Runtime Bootstrap

`main.rs`:

1. Load config (TOML file, env var overrides)
2. Initialize telemetry subscriber based on `[telemetry]` config (must be first — captures all subsequent spans)
3. Detect runtime mode: if `AWS_LAMBDA_RUNTIME_API` env var is set, Lambda mode; otherwise, server mode
4. Instantiate adapters based on config
5. Construct `AppService` with injected ports
6. Build axum `Router` with routes and middleware (including OTEL tower layer for HTTP spans)
7. Lambda: wrap router with `lambda_http`, run `lambda_runtime`; Server: bind to configured address, run with hyper

Same binary, same router, same code paths. Only the outermost transport differs.

### Middleware Stack

All routes:
- **Telemetry** — tower OTEL layer: auto-creates spans per request with method, path, status, latency; propagates trace context from incoming `traceparent` header
- **Request ID** — generate or extract `X-Request-Id`, attach to current tracing span
- **Audit context** — extract IP, user-agent, device-id from headers, attach to request extensions
- **Error handling** — catch panics and unhandled errors, return structured JSON errors

`/internal/*` only:
- **Internal auth** — shared secret or mTLS verification

---

## 7. Provider System

### Three Tiers

**Tier 1 — Standard OIDC (config-only)**

Providers like Google that follow the OIDC spec. A generic `OidcProvider` struct in the `adapters` crate handles these entirely from config:

```toml
[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

The `OidcProvider` adapter fetches `/.well-known/openid-configuration` from the issuer, caches JWKS with TTL-based refresh, and handles code exchange, ID token validation (signature, `iss`, `aud`, `exp`), and revocation. Adding a new standard OIDC provider is just a new config block.

**Tier 2 — OIDC-with-quirks (custom module)**

Providers like Apple that are mostly OIDC but have non-standard requirements:

```toml
[providers.apple]
adapter = "apple"
client_id = "com.example.app"
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"
```

The `AppleProvider` implements `IdentityProvider`, generates a signed ES256 client JWT for each token endpoint call, and reuses shared OIDC utilities (JWKS caching, discovery) from the adapters crate for the standard parts.

**Tier 3 — Non-OIDC (custom module)**

Providers like atproto that use a fundamentally different protocol:

```toml
[providers.atproto]
adapter = "atproto"
client_id = "https://example.com/oauth/client-metadata.json"
```

The `AtprotoProvider` implements `IdentityProvider` with entirely different internals: PAR, DPoP proof generation with nonce rotation, DID resolution and verification, and per-PDS authorization server discovery.

### Provider Registry

At startup, the server crate builds a `HashMap<String, Box<dyn IdentityProvider>>` from config:

```rust
fn build_providers(config: &AppConfig) -> Result<HashMap<String, Box<dyn IdentityProvider>>> {
    let mut providers = HashMap::new();
    for (name, provider_config) in &config.providers {
        let provider: Box<dyn IdentityProvider> = match provider_config.adapter.as_str() {
            "oidc" => Box::new(OidcProvider::from_config(provider_config)?),
            "apple" => Box::new(AppleProvider::from_config(provider_config)?),
            "atproto" => Box::new(AtprotoProvider::from_config(provider_config)?),
            other => return Err(Error::UnknownAdapter(other.to_string())),
        };
        providers.insert(name.clone(), provider);
    }
    Ok(providers)
}
```

When `POST /token` arrives with `provider=google`, the service looks up `"google"` in the map. Unknown provider returns `400 Bad Request`.

### Shared Utilities

Common OIDC operations in `adapters/src/shared/` for reuse by Tier 1 and Tier 2 providers:

- `jwks::JwksCache` — fetches and caches JWKS with TTL, automatic refresh
- `discovery::discover(issuer_url)` — fetches and parses `.well-known/openid-configuration`
- `token_endpoint::exchange(endpoint, params)` — standard form-encoded POST to token endpoint

Tier 3 providers use their own logic entirely.

---

## 8. Configuration

A single TOML file controls the entire service. Environment variables override via `${VAR_NAME}` syntax for secrets and `OIDC_EXCHANGE__{section}__{key}` for structural overrides.

### Full Config Structure

```toml
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
# "open" — any authenticated user gets a record created
# "existing_users_only" — user must already exist (created via /internal/users)
mode = "open"
# Optional — if set, only these email domains are allowed (applies in both modes)
# Exact: "example.com", Wildcard: "*.example.com" (matches any subdomain depth)
domain_allowlist = ["example.com", "*.acme.corp"]

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "https://api.example.com"

[token.custom_claims]
org = "example"
role = "{{ user.metadata.role | default: 'user' }}"

[audit]
adapter = "cloudtrail"
blocking_threshold = "warning"

[audit.cloudtrail]
channel_arn = "arn:aws:cloudtrail:us-east-1:123456789:channel/my-channel"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "arn:aws:kms:us-east-1:123456789:key/abcd-1234"
algorithm = "ECDSA_SHA_256"
kid = "key-2024-01"

[key_manager.local]
private_key_path = "/secrets/ed25519.pem"
algorithm = "EdDSA"
kid = "key-2024-01"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "oidc-exchange"
region = "us-east-1"

[user_sync]
enabled = false
adapter = "webhook"

[user_sync.webhook]
url = "https://internal-api.example.com/user-events"
secret = "${SYNC_WEBHOOK_SECRET}"
timeout = "5s"
retries = 2

[telemetry]
enabled = true
exporter = "otlp"               # "otlp", "stdout", "xray", "none"
endpoint = "http://localhost:4317"
service_name = "oidc-exchange"
sample_rate = 1.0               # 0.0 to 1.0
protocol = "grpc"               # "grpc" or "http"

[internal_api]
auth_method = "shared_secret"
shared_secret = "${INTERNAL_API_SECRET}"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]

[providers.apple]
adapter = "apple"
client_id = "com.example.app"
team_id = "${APPLE_TEAM_ID}"
key_id = "${APPLE_KEY_ID}"
private_key_path = "/secrets/apple.p8"

[providers.atproto]
adapter = "atproto"
client_id = "https://example.com/oauth/client-metadata.json"
```

### Config Loading Order

1. Load `config/default.toml` (compiled-in defaults)
2. Load `config/{ENVIRONMENT}.toml` if it exists (e.g., `config/production.toml`). The `OIDC_EXCHANGE_ENV` environment variable sets `ENVIRONMENT`; defaults to `default` if unset.
3. Override with environment variables: `OIDC_EXCHANGE__{section}__{key}`
4. Resolve `${VAR_NAME}` placeholders from environment

### Defaults

| Setting | Default |
|---|---|
| `server.host` | `0.0.0.0` |
| `server.port` | `8080` |
| `registration.mode` | `open` |
| `registration.domain_allowlist` | none (all domains allowed) |
| `token.access_token_ttl` | `15m` |
| `token.refresh_token_ttl` | `30d` |
| `telemetry.enabled` | `false` |
| `telemetry.exporter` | `none` |
| `telemetry.sample_rate` | `1.0` |
| `audit.adapter` | `noop` (stdout-only) |
| `audit.blocking_threshold` | `warning` |
| `user_sync.enabled` | `false` |
| `internal_api` | disabled |

---

## 9. Telemetry

Observability via OpenTelemetry, integrated through Rust's `tracing` ecosystem. This is distinct from audit logging — telemetry is for operational monitoring (latency, error rates, trace propagation), audit is for compliance/security event records.

### Approach

Core code and adapters use `tracing` for instrumentation (`#[tracing::instrument]`, `tracing::info!`, etc.) with no awareness of OTEL. The `server` crate wires up a `tracing-opentelemetry` subscriber layer at startup that bridges tracing spans to OTEL spans and exports them via the configured exporter.

### What Gets Instrumented

- **HTTP layer** — automatic span per request via tower OTEL layer: method, path, status code, latency, trace ID from `traceparent` header
- **Service methods** — `#[tracing::instrument]` on `exchange()`, `refresh()`, `revoke()`, `user_admin()` with relevant fields (provider, user_id)
- **Port calls** — `#[tracing::instrument]` on adapter methods: DynamoDB operations, KMS signing, provider HTTP calls, audit emit. Captures latency and errors per external call
- **Provider interactions** — spans around code exchange, ID token validation, JWKS fetch/cache

### Exporters

| Exporter | Use Case |
|---|---|
| `otlp` | Standard OTEL collector (gRPC or HTTP) — production default |
| `xray` | AWS X-Ray via `opentelemetry-aws` — Lambda-native, propagates X-Ray trace header |
| `stdout` | Prints spans to stdout as JSON — local development |
| `none` | Disabled, zero overhead — when telemetry is not needed |

### Key Dependencies

- `tracing` — instrumentation API (used in core, adapters, server)
- `tracing-subscriber` — subscriber setup with filtering
- `tracing-opentelemetry` — bridges tracing to OTEL
- `opentelemetry` + `opentelemetry-otlp` — OTEL SDK and OTLP exporter
- `opentelemetry-aws` — X-Ray ID generator and propagator (optional, for xray exporter)

### Relationship to Audit

Telemetry and audit are independent systems with different purposes:

| | Telemetry | Audit |
|---|---|---|
| **Purpose** | Operational monitoring | Compliance / security record |
| **Data** | Spans, metrics, traces | Structured events with severity |
| **Failure mode** | Best-effort, never blocks | Configurable blocking per severity |
| **Storage** | OTEL collector → Grafana/Datadog/X-Ray | CloudTrail Lake / pluggable |

A single request may produce both a telemetry trace (with spans for each step) and an audit event (recording the auth decision). They share the request ID / trace ID for correlation but are otherwise independent.

---

## 10. Error Handling

### Domain Error Type

```rust
pub enum Error {
    // Auth flow errors (4xx)
    InvalidGrant { reason: String },
    InvalidToken { reason: String },
    InvalidRequest { reason: String },
    UnknownProvider { provider: String },
    AccessDenied { reason: String },     // domain not allowed or existing_users_only
    UserSuspended { user_id: String },
    Unauthorized { reason: String },

    // Provider errors (upstream)
    ProviderError { provider: String, detail: String },
    ProviderTimeout { provider: String },

    // Infrastructure errors (5xx)
    StoreError { detail: String },
    KeyError { detail: String },
    AuditError { detail: String },
    SyncError { detail: String },

    // Internal
    ConfigError { detail: String },
}
```

### HTTP Error Mapping

Responses follow OAuth 2.0 error codes (RFC 6749 Section 5.2):

```json
{
  "error": "invalid_grant",
  "error_description": "Authorization code has expired"
}
```

| Domain Error | HTTP Status | `error` field |
|---|---|---|
| InvalidGrant | 400 | `invalid_grant` |
| InvalidToken | 401 | `invalid_token` |
| InvalidRequest | 400 | `invalid_request` |
| UnknownProvider | 400 | `invalid_request` |
| AccessDenied | 403 | `access_denied` |
| UserSuspended | 403 | `access_denied` |
| Unauthorized | 401 | `unauthorized` |
| ProviderError | 502 | `server_error` |
| ProviderTimeout | 504 | `server_error` |
| StoreError, KeyError, AuditError | 500 | `server_error` |
| ConfigError | 500 | `server_error` |

`SyncError` is not mapped to an HTTP response — user sync is non-blocking and never fails a request. Sync failures are logged via `tracing::warn!`. `ConfigError` is typically startup-fatal, but can occur at runtime during custom claims template resolution.

Internal details are never leaked to the client. `server_error` responses log the detail internally and return a generic message.

### Adapter Error Mapping

Each adapter converts its native errors at the boundary:

```rust
impl From<SdkError<GetItemError>> for Error {
    fn from(e: SdkError<GetItemError>) -> Self {
        Error::StoreError {
            detail: format!("DynamoDB GetItem failed: {e}"),
        }
    }
}
```

The core never sees AWS SDK types.

### Error + Audit Integration

Every error reaching the HTTP layer is emitted as an audit event before the response:

| Error | Audit Event | Severity |
|---|---|---|
| InvalidGrant, InvalidToken | `ValidationFailed` | Warning |
| AccessDenied (domain/mode) | `RegistrationDenied` | Notice |
| UserSuspended | `Unauthorized` | Warning |
| Unauthorized (internal API) | `Unauthorized` | Warning |
| ProviderError/Timeout | `ProviderError` | Error |
| StoreError, KeyError | not audited (infrastructure, use tracing) | — |
| AuditError | not audited (stderr only) | — |

Infrastructure errors go through `tracing` only — attempting to audit a database failure through a potentially-also-broken audit system is not useful.

---

## 11. Schemas

### Generic Data Model (schemas/datamodel.schema.json)

Defines the logical entities any adapter must support. Adapter-agnostic.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "OIDC Exchange Data Model",
  "definitions": {
    "User": {
      "type": "object",
      "required": ["id", "external_id", "provider", "status", "created_at", "updated_at"],
      "properties": {
        "id": { "type": "string", "description": "Internal user ID (e.g., usr_01ARZ3NDEK...)" },
        "external_id": { "type": "string", "description": "Provider subject claim or DID" },
        "provider": { "type": "string", "description": "Provider identifier (google, apple, atproto)" },
        "email": { "type": ["string", "null"] },
        "display_name": { "type": ["string", "null"] },
        "metadata": {
          "type": "object",
          "additionalProperties": true,
          "description": "Extensible key-value pairs for sync data"
        },
        "claims": {
          "type": "object",
          "additionalProperties": true,
          "description": "Per-user private claims added to access token JWT, managed via internal API"
        },
        "status": { "enum": ["active", "suspended", "deleted"] },
        "created_at": { "type": "string", "format": "date-time" },
        "updated_at": { "type": "string", "format": "date-time" }
      }
    },
    "Session": {
      "type": "object",
      "required": ["user_id", "refresh_token_hash", "provider", "expires_at", "created_at"],
      "properties": {
        "user_id": { "type": "string" },
        "refresh_token_hash": { "type": "string", "description": "SHA-256 hash of the opaque refresh token" },
        "provider": { "type": "string" },
        "expires_at": { "type": "string", "format": "date-time" },
        "device_id": { "type": ["string", "null"] },
        "user_agent": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "created_at": { "type": "string", "format": "date-time" }
      }
    },
    "AuditEvent": {
      "type": "object",
      "required": ["id", "timestamp", "severity", "event_type", "outcome"],
      "properties": {
        "id": { "type": "string", "description": "ULID" },
        "timestamp": { "type": "string", "format": "date-time" },
        "severity": { "enum": ["emergency", "alert", "critical", "error", "warning", "notice", "info", "debug"] },
        "event_type": { "type": "string" },
        "actor": { "type": ["string", "null"], "description": "User ID if known" },
        "provider": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "user_agent": { "type": ["string", "null"] },
        "detail": { "type": "object", "additionalProperties": true },
        "outcome": {
          "type": "object",
          "required": ["status"],
          "properties": {
            "status": { "enum": ["success", "failure"] },
            "reason": { "type": ["string", "null"] }
          }
        }
      }
    }
  }
}
```

### DynamoDB Table Design (schemas/dynamodb/table-design.json)

Single-table design. The DynamoDB adapter maps the logical model into this structure.

```json
{
  "table": {
    "table_name": "oidc-exchange",
    "key_schema": {
      "pk": { "type": "S", "description": "Partition key" },
      "sk": { "type": "S", "description": "Sort key" }
    },
    "global_secondary_indexes": [
      {
        "index_name": "GSI1",
        "key_schema": {
          "GSI1pk": { "type": "S" },
          "GSI1sk": { "type": "S" }
        },
        "projection": "ALL"
      }
    ]
  },
  "access_patterns": {
    "get_user_by_id": {
      "operation": "GetItem",
      "pk": "USER#<user_id>",
      "sk": "PROFILE"
    },
    "get_user_by_external_id": {
      "operation": "Query",
      "index": "GSI1",
      "GSI1pk": "EXT#<provider>#<external_id>",
      "GSI1sk": "USER"
    },
    "get_session_by_refresh_token": {
      "operation": "GetItem",
      "pk": "SESSION#<refresh_token_hash>",
      "sk": "SESSION"
    },
    "list_user_sessions": {
      "operation": "Query",
      "index": "GSI1",
      "GSI1pk": "USER#<user_id>",
      "GSI1sk": "begins_with SESSION#"
    },
    "revoke_all_user_sessions": {
      "operation": "Query + BatchWrite (delete)",
      "description": "Query all sessions for user via GSI1, then batch delete"
    }
  },
  "item_schemas": {
    "User": {
      "pk": "USER#<id>",
      "sk": "PROFILE",
      "GSI1pk": "EXT#<provider>#<external_id>",
      "GSI1sk": "USER",
      "attributes": {
        "id": "S",
        "external_id": "S",
        "provider": "S",
        "email": "S (optional)",
        "display_name": "S (optional)",
        "metadata": "M (map, optional)",
        "claims": "M (map, optional)",
        "status": "S",
        "created_at": "S (ISO 8601)",
        "updated_at": "S (ISO 8601)"
      }
    },
    "Session": {
      "pk": "SESSION#<refresh_token_hash>",
      "sk": "SESSION",
      "GSI1pk": "USER#<user_id>",
      "GSI1sk": "SESSION#<created_at>",
      "attributes": {
        "user_id": "S",
        "refresh_token_hash": "S",
        "provider": "S",
        "expires_at": "S (ISO 8601)",
        "device_id": "S (optional)",
        "user_agent": "S (optional)",
        "ip_address": "S (optional)",
        "created_at": "S (ISO 8601)",
        "ttl": "N (epoch seconds, DynamoDB TTL for automatic session cleanup)"
      }
    }
  }
}
```

Key design decisions:
- GSI1pk for users includes provider prefix (`EXT#google#12345`) so the same external ID from different providers doesn't collide
- Session pk is the token hash — direct GetItem on refresh, no query needed
- Session GSI1 groups sessions by user for the "revoke all" operation
- TTL attribute on sessions lets DynamoDB automatically clean up expired sessions

---

## 12. Testing Strategy

### Test Runner

`cargo-nextest` for all test execution. Config in `.config/nextest.toml` at workspace root. CI runs `cargo nextest run --workspace`. Tests use standard `#[tokio::test]` — nextest is the runner, not the assertion framework.

### Layer-by-Layer

**Core unit tests** — service orchestration with mock ports:

- Mock all ports using in-memory implementations (`HashMap`-backed `SessionStore`, deterministic `KeyManager`)
- Tests live alongside code in `crates/core/src/`
- Cover: token exchange happy path, refresh flow, revocation, user suspension, custom claims resolution, audit blocking logic, error mapping

**Adapter integration tests** — each adapter against real infrastructure:

- Live in `crates/adapters/tests/`
- DynamoDB: test against DynamoDB Local (Docker) — create table, CRUD users, CRUD sessions, TTL behavior
- KMS: test against LocalStack or skip in CI with `#[ignore]`— sign/verify round-trip
- CloudTrail: test against LocalStack or mock HTTP endpoint — verify event format matches PutAuditEvents API
- OIDC: test JWKS caching, discovery parsing against recorded HTTP responses (using `wiremock`)

**Provider tests** — provider-specific logic:

- Live in `crates/providers/tests/`
- Apple: client JWT generation (ES256 signing), token endpoint request format
- atproto: DPoP proof generation, PAR request construction, DID resolution
- Use `wiremock` to simulate provider endpoints — no real provider calls in CI

**HTTP integration tests** — full request/response cycle:

- Live in `crates/server/tests/`
- Spin up axum router with all-mock ports
- Test: HTTP status codes, OAuth error format compliance, content-type headers, JWKS response format, discovery document, internal API auth rejection
- Use `axum::test::TestClient` or `tower::ServiceExt` — no TCP binding needed

### Test Utilities Crate (crates/test-utils/)

Dev-dependency providing:

- `MockRepository` — in-memory `HashMap` implementation
- `MockKeyManager` — deterministic Ed25519 key pair, reproducible across test runs
- `MockAuditLog` — collects events into `Vec<AuditEvent>` for assertion
- `MockUserSync` — records calls for assertion
- `MockIdentityProvider` — configurable responses for code exchange and validation
- Builder pattern for constructing `AppService` with any combination of real and mock ports
