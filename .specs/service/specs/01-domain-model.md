# Domain Model

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/core/src/domain

The entities that flow through the service, their identifiers, and their lifecycles. Types
live in `crates/core/src/domain/`; the JSON Schema in
[canonical-types.schema.json](canonical-types.schema.json) is the machine-readable mirror of
this page.

## ID scheme

| Entity | Identifier | Form | Generated where |
|---|---|---|---|
| User | `User.id` | `usr_` + lowercase ULID | the repository adapter on `create_user` |
| Session | `Session.refresh_token_hash` | SHA-256 hex of the opaque refresh token | core (`exchange`) |
| AuditEvent | `AuditEvent.id` | bare ULID | core (`create_audit_event`) |

User IDs are minted by each repository adapter (`dynamo`, `postgres`, `sqlite`), not by the
core service — every adapter independently formats `usr_{ulid}` in its `create_user`. The
external identifier (`User.external_id`) is the provider's `sub` claim (or DID), namespaced
by `provider` so the same subject from two providers cannot collide.

## Entities

### User (`domain/user.rs`)

```rust
struct User {
    id: String,                       // "usr_…"
    external_id: String,              // provider sub / DID
    provider: String,                 // "google", "apple"
    email: Option<String>,
    display_name: Option<String>,
    metadata: HashMap<String, Value>, // extensible sync data
    claims: HashMap<String, Value>,   // per-user private claims merged into the access JWT
    status: UserStatus,
    version: u64,                     // optimistic-concurrency counter; 1 on create
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

`metadata` is operator/sync data and is readable by claim templates; `claims` is the set of
private claims injected into the access token, managed through the internal API. `NewUser`
(creation input) carries `external_id`, `provider`, `email`, `display_name`. `UserPatch`
(update input) carries optional `email`, `display_name`, `metadata`, `claims`, `status`; a
`Some(claims)` patch **replaces** the entire claims map.

`version` is store-managed, never caller-supplied: `create_user` writes `1`, every
`update_user` increments it, and the write is conditioned on the version that was read
(see [08-persistence.md](08-persistence.md)). It appears in neither `NewUser` nor
`UserPatch`.

### Session (`domain/session.rs`)

```rust
struct Session {
    user_id: String,
    refresh_token_hash: String,       // SHA-256 hex; never the raw token
    provider: String,
    expires_at: DateTime<Utc>,
    device_id: Option<String>,
    user_agent: Option<String>,
    ip_address: Option<String>,
    created_at: DateTime<Utc>,
}
```

The raw refresh token exists only in memory during issuance and in the response to the
client. Only the hash is stored. `device_id`, `user_agent`, and `ip_address` are populated
`None` by the core at issuance today (the audit-context middleware captures them at the HTTP
edge but they are not threaded into the stored session).

### SingleUseRecord (`domain/single_use.rs`)

```rust
struct SingleUseRecord {
    key: String,                  // "nonce:<sha256hex>" | "assertion:<provider>:…"
    expires_at: DateTime<Utc>,
}
```

A presence-only record: the key is all the information there is. Nonce values and
assertions are stored only as SHA-256 hex digests, as refresh tokens are. Records are
removed by `take_single_use`, by store-native expiry, or by `cleanup_expired_sessions`.

### Token types (`domain/token.rs`)

- **`TokenResponse`** — the `/token` body: `access_token`, optional `refresh_token` (present
  on exchange, absent on refresh), `token_type` (always `"Bearer"`), `expires_in` seconds.
- **`AccessTokenClaims`** — JWT payload: `sub` (internal user id), `iss`, `aud`, `iat`,
  `exp`, plus a flattened `custom: HashMap<String, Value>` of resolved claims.
- **`ProviderTokens`** — what a provider returns from code exchange: `id_token`, optional
  `refresh_token`, optional `access_token`.
- **`IdentityClaims`** — verified claims from a provider ID token: `subject`, optional
  `email`, `email_verified`, `name`, `is_private_email` (Apple private-relay flag; `None`
  for other providers), and `raw_claims`.

### AuditEvent (`domain/audit.rs`)

```rust
struct AuditEvent {
    id: String,                       // ULID
    timestamp: DateTime<Utc>,
    severity: AuditSeverity,          // RFC 5424 syslog levels, emergency(0)..debug(7)
    event_type: AuditEventType,
    actor: Option<String>,            // user id if known
    provider: Option<String>,
    ip_address: Option<String>,
    user_agent: Option<String>,
    detail: HashMap<String, Value>,
    outcome: AuditOutcome,            // Success | Failure { reason }
}
```

`AuditEventType` variants: `TokenExchange`, `TokenRefresh`, `TokenRevocation`,
`SessionRevoked`, `AllSessionsRevoked`, `UserCreated`, `UserUpdated`, `UserSuspended`,
`UserDeleted`, `ValidationFailed`, `RegistrationDenied`, `ProviderError`, `Unauthorized`.
`AuditOutcome` serializes to `{ "status": "success" }` or `{ "status": "failure", "reason": … }`.

### OidcProviderConfig (`domain/provider.rs`)

The normalized config the standard OIDC adapter consumes: `provider_id`, `issuer`,
`client_id`, optional `client_secret`, optional `jwks_uri` / `token_endpoint` /
`revocation_endpoint` (discovered from the issuer if absent), `scopes`, and
`additional_params`. See [05-provider-system.md](05-provider-system.md).

### AdminStats (`service/user_admin.rs`)

Aggregate returned by `/internal/stats`: `users { total, active, suspended, deleted }` and
`sessions { active }`, computed from `UserRepository::count_by_status` and
`SessionRepository::count_active_sessions`.

## Relationships

```
User 1 ──────< Session        (Session.user_id → User.id; revoke-all walks them)
User 1 ──────< AuditEvent      (AuditEvent.actor → User.id, when known)
User.provider / external_id ── upstream provider subject
```

## Lifecycles

### User status

```
        create_user
            │
            ▼
        ┌────────┐  admin suspend   ┌───────────┐
        │ Active │ ───────────────► │ Suspended │
        └───┬────┘ ◄─────────────── └───────────┘
            │        admin reactivate
            │ admin delete (soft)
            ▼
        ┌─────────┐
        │ Deleted │  record retained, all sessions revoked
        └─────────┘
```

- **Active** — may obtain and refresh tokens.
- **Suspended** — exchange and refresh are rejected (`UserSuspended`); already-issued access
  JWTs remain valid until they expire (JWTs are not individually revocable).
- **Deleted** — soft delete via `UserPatch { status: Deleted }`; the service revokes all the
  user's sessions on delete. The row is kept, but the identity is freed:
  `get_user_by_external_id` no longer returns the deleted user, and a later first login
  for the same `(provider, external_id)` re-registers as a brand-new user with no claims
  or sessions carried over.

### Session

Created on token exchange with `expires_at = now + refresh_token_ttl`. Removed by explicit
revocation (`/revoke`, delete-user, or revoke-all-user-sessions) or by expiry — DynamoDB via
its TTL attribute, other stores via `cleanup_expired_sessions`.

## Required query patterns

| Pattern | Port method |
|---|---|
| Look up a user by internal id | `UserRepository::get_user_by_id` |
| Look up a user by provider subject | `UserRepository::get_user_by_external_id(external_id, provider)` |
| Count users by status | `UserRepository::count_by_status` |
| List users (paged) | `UserRepository::list_users(offset, limit)` |
| Resolve a refresh token | `SessionRepository::get_session_by_refresh_token(hash)` |
| Revoke one / all sessions | `SessionRepository::revoke_session` / `revoke_all_user_sessions` |
| Count active sessions | `SessionRepository::count_active_sessions` |
| Reap expired sessions | `SessionRepository::cleanup_expired_sessions` |
| Claim a single-use key | `SessionRepository::put_single_use(key, expires_at)` |
| Burn a single-use key | `SessionRepository::take_single_use(key)` |

## Assumptions and open questions

### Assumptions

- At most one **non-deleted** user exists per `(provider, external_id)`; provider
  namespacing prevents cross-provider subject collisions. Deleting a user frees the
  identity for re-registration — the deleted record is retained but no longer occupies
  the key.

### Decisions

- *User IDs minted in the adapter.* **Each repository adapter generates `usr_{ulid}` in
  `create_user`.** Keeps ID creation next to the write that persists it; the trade-off is the
  `usr_` + lowercase-ULID convention is duplicated across three adapters rather than owned by
  the core.
- *Suspended keeps live tokens valid.* **Suspension blocks new/refreshed tokens but not
  outstanding access JWTs.** Access JWTs are stateless and short-lived; revoking them would
  require introspection, which is out of scope.
- *Claims vs metadata split.* **`claims` feeds the JWT, `metadata` feeds templates/sync.**
  Separating the injected claim set from general profile data keeps token contents explicit.

### Open questions

- `Session.device_id` / `user_agent` / `ip_address` exist on the entity and in the store
  schemas but are written as `None` at issuance; wiring the audit-context values into the
  stored session is unresolved.
