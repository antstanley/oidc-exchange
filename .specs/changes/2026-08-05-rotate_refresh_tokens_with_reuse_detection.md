# Change: Rotate refresh tokens on redemption, with reuse detection

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/core, crates/adapters (service)

Make refresh-token redemption a state transition instead of a read. Each successful refresh
mints a replacement refresh token and retires the presented generation in one atomic store
operation, and returns the replacement to the client. Presenting an already-retired
generation is treated as evidence that two parties hold the same credential: the whole token
family is revoked and a `Warning`-severity audit event is emitted. Because a read-then-write
across the current `SessionRepository` is a race on every backend, the rotation and
reuse-detection behaviour is specified as an obligation on the port and proven by a shared
conformance suite that all five session adapters run.

---

## Motivation

`AppService::refresh` writes nothing. It hashes the presented token, looks the session up by
that hash, checks the session's expiry and the user's status, signs an access token and
returns (`crates/core/src/service/refresh.rs:22-128`). The presented token is not consumed,
no replacement is issued, `expires_at` is untouched, and no last-use metadata is recorded
anywhere. After ten redemptions the stored session row is byte-for-byte what it was before
the first. The capability is not missing from the port — the sibling `/revoke` path performs
the identical hash-and-lookup and then calls `revoke_session`
(`crates/core/src/service/revoke.rs:99-114`) — the refresh path simply calls neither
`store_refresh_token` nor `revoke_session`.

The consequence is that a captured refresh token mints access tokens for the remainder of its
30-day window, and the legitimate holder and the attacker are indistinguishable to the
service. That second half is the part worth more. Rotation's security value is not primarily
that it shortens a stolen token's life; it is that presenting a superseded token is an
unambiguous signal that the credential leaked. With no write on the redemption path there is
nothing in the store that could ever raise that signal, so a compromise is undetectable by
construction. `.specs/service/specs/03-service-flows.md` records non-rotation as a deliberate
choice justified on client-compatibility grounds; that argument is about rotation and does not
reach reuse detection, last-use metadata, or the shipped 30-day default. This change keeps the
compatibility escape hatch as an explicit configuration switch and makes the detectable
posture the default.

Evidence: `.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-refresh-token-never-rotated/`,
the related consistency defect in
`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g3-dynamo-session-read-eventual-consistency/`,
and the structural argument in
`.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/credential-lifecycle-contract.md`
(invariants CL3 and CL4; Option 2, "A Typed Lifecycle And A Store Contract").

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Scope-summary row for refresh; replace the reusable-token Decision; the external-scheduler Assumption becomes an owned reaping obligation |
| [`.specs/service/specs/01-domain-model.md`](../service/specs/01-domain-model.md) | `Session` gains family fields; add the `RetiredRefreshToken` entity; ID scheme, session lifecycle, query patterns, token types, `AuditEventType` variants; `AccessTokenClaims.sid` re-described against `family_id` |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | `SessionRepository` gains three methods and a stated contract; add the conformance suite |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Rewrite the token-refresh flow; replace the non-rotation Decision; re-point the access token's `sid` from `refresh_token_hash` to `family_id` (Build/Validate access token, Revocation) and replace the sibling's `sid` Decision |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md) | `/token` refresh response now carries a refresh token; Bootstrap spawns the session reaper; Internal routes gain `POST /internal/sessions/cleanup` |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Three new `[token]` keys and their defaults; `[session_repository]` gains `cleanup_interval` |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | Per-adapter retirement storage, atomic rotation, DynamoDB consistent read and an authoritative per-user session roster replacing GSI enumeration on the revocation paths; the LMDB reaper batches its deletes; the Valkey counter clamps instead of asserting |
| [`canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | Modify `Session` and `TokenResponse`; add `RetiredRefreshToken`; add `refresh_token_reuse` to `AuditEventType`; re-pattern `AccessTokenClaims.sid` to the `family_id` form |

No new canonical page. `schemas/datamodel.schema.json` (the adapter-agnostic logical model
that 08-persistence names as cross-adapter source of truth) changes alongside the canonical
schema.

---

## Proposed changes

### `.specs/service/specs/03-service-flows.md` → Token refresh (Modify)

> ## Token refresh (`refresh.rs`)
>
> `POST /token` with `grant_type=refresh_token`. Redemption is a state transition: the
> family's live generation is retired and a replacement is issued in one atomic store
> operation. A refresh token belongs to a **family** — every generation descended from one
> sign-in shares a `family_id`, and a family has exactly one live generation at any instant.
>
> 1. SHA-256 hex the presented token.
> 2. `resolve_refresh_token(hash)` classifies the hash against the family's live generation
>    and its retained retirement records:
>    - **`Unknown`** — no live generation and no retained record → `InvalidToken` (audited
>      `ValidationFailed`, `Debug`), as before.
>    - **`Live(session)`** — the hash is the live generation → rotate (step 4).
>    - **`Superseded { live, retired_at }`** — the hash is retired and its successor is still
>      the family's live generation. Inside `token.refresh_rotation_grace` of `retired_at` →
>      rotate from `live` (step 4). Outside it → reuse (step 3).
>    - **`Retired { family_id, user_id, .. }`** — the hash is retired and its successor is no
>      longer live → reuse (step 3).
> 3. **Reuse.** `revoke_family(family_id)`, then emit `RefreshTokenReuse` at
>    `AuditSeverity::Warning` with `detail { family_id, sessions_revoked }`, then return
>    `InvalidToken` carrying the same reason string as the unknown-token branch — the response
>    does not tell the presenter that an alarm fired. Revocation runs before the emission so a
>    blocking audit failure cannot leave the family alive.
> 4. `session.expires_at < now` → `InvalidToken`. The family's absolute expiry is fixed at
>    exchange; rotation never moves it.
> 5. `get_user_by_id(session.user_id)`; missing → `InvalidToken`; suspended → `UserSuspended`.
>    Both are decided before anything is written.
> 6. Mint the replacement — 32 random bytes, base64url-no-pad for the opaque token, SHA-256
>    hex of the bytes for the new hash. The replacement `Session` inherits `family_id`,
>    `user_id`, `provider`, `created_at`, `expires_at` and the device fields unchanged, sets
>    `generation = live.generation + 1` and `rotated_at = now`.
> 7. `rotate_refresh_token(live_hash, replacement)` — one atomic compare-and-swap conditioned
>    on `live_hash` still being the family's live generation. It deletes the live session,
>    writes a `RetiredRefreshToken` for `live_hash` naming the replacement as its successor,
>    and installs the replacement, or it writes nothing. A `false` return means a concurrent
>    redemption won the race; the caller returns `InvalidToken` without revoking the family,
>    and the loser's retry lands on the grace path.
> 8. `build_access_token(user, &family_id)` — the access token's `sid` claim carries the
>    family identifier, which rotation never moves (see the *Build access token* block below).
> 9. Audit `TokenRefresh` at `Info` with `detail { family_id, generation, grace }`. No token
>    hash appears in an audit event.
> 10. Respond `TokenResponse { access_token, refresh_token: Some(replacement), token_type:
>     "Bearer", expires_in }`.
>
> **Grace.** A client that loses the response to a rotation still holds the generation the
> server just retired. Returning the current token again is impossible here — the store keeps
> only digests, so the service cannot reproduce a plaintext it has already discarded — so the
> grace window instead lets that client rotate forward once more: presenting the
> immediately-preceding generation inside `token.refresh_rotation_grace` rotates from the
> current live generation and issues a fresh one. "Immediately preceding" and "once" are the
> same condition, and it needs no extra state: a retirement record grants grace only while the
> successor it names is still live, and a grace rotation retires that successor. Every later
> presentation of the same generation is reuse.
>
> **Rotation disabled.** With `token.refresh_rotation = false` the flow is steps 1–2, 4, 5,
> 8, 9 and a response with `refresh_token: None`. Nothing is minted and nothing is retired.
> Retirement records left over from a rotation-enabled period still resolve until they expire;
> while rotation is off, `Superseded` and `Retired` are treated as `Unknown` — refused as
> `InvalidToken`, no alarm, no family revocation — because the switch disables the response
> along with the rotation.
>
> `RefreshRequest` carries the same client context as exchange; audit events in the flow
> record its `ip_address` and `user_agent`. A suspended user audits `UserSuspended` (warning,
> failure); unknown and expired tokens audit `ValidationFailed` (debug, failure) — below the
> default `emit_threshold` of `info` — and `RefreshTokenReuse` (warning) is the signal that
> survives the defaults.

### `.specs/service/specs/03-service-flows.md` → Decisions (Modify)

> - *Refresh rotates.* **Each redemption issues a replacement refresh token and retires the
>   presented generation in one atomic store operation.** A long-lived credential that is
>   never consumed makes possession indistinguishable from entitlement for its whole TTL;
>   rotation bounds a stolen token to one use and, more importantly, makes a second holder
>   visible.
> - *Rotation does not slide the expiry.* **The replacement inherits the family's original
>   `expires_at` and `created_at`.** Recomputing the expiry on every rotation would convert a
>   bounded 30-day session into an unbounded one that never dies while it is used, removing
>   the only bound that currently ends a stolen token's life.
> - *Reuse revokes the family, not the user.* **A retired generation presented outside its
>   grace window revokes every generation of that one login chain.** The evidence is that one
>   credential chain leaked; logging the user out of every other device is disproportionate to
>   it.
> - *The reuse alarm is emitted at `Warning`.* **`RefreshTokenReuse` carries
>   `AuditSeverity::Warning`.** The shipped audit defaults are `emit_threshold = "info"` and
>   `blocking_threshold = "warning"`, so `Warning` is the least severe level that both
>   survives a default deployment and fails the request rather than being silently dropped
>   when the audit backend is down.
> - *`sid` is the session's family identifier.* **The access token carries `family_id` as its
>   `sid` claim, and `/revoke`'s access-token arm resolves it with `revoke_family`.** This
>   replaces the *`sid` is the session's refresh-token hash* Decision merged by
>   [2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md):
>   the hash was the right identifier while refresh never rotated, and rotation is exactly
>   the change that orphans it — after one refresh every outstanding access token would name
>   a retired hash and access-token revocation would silently become a no-op. That sibling
>   defined `sid` as *the current session identifier*; a rotation-independent identifier is
>   what keeps that definition resolvable for the token's full TTL.

### `.specs/service/specs/03-service-flows.md` → Build access token / Validate access token / Revocation (Modify)

[2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md)
merges before this spec and binds the access token's `sid` claim to the session's
`refresh_token_hash` — correct while refresh does not rotate, and orphaned by the first
rotation. That sibling defines `sid` as the current session identifier and names this spec
as the one that re-points it. Three of the sections it wrote change:

> In **Build access token**, step 2's `sid` clause becomes: `sid` is the `family_id` of the
> session this token is minted for — supplied by the caller, from the family `exchange` has
> just created or the one `refresh` has just rotated. `family_id` is stable across every
> rotation, so a `sid` minted at exchange names its session for the token's whole validity
> however often the refresh token rotates beneath it.
>
> In **Validate access token**, step 7 becomes: `sub` is non-empty and `sid` is a
> well-formed family identifier (`fam_` + lowercase ULID). A token whose `sid` is not —
> including one minted before this change, carrying a 64-hex `refresh_token_hash` — fails
> validation with a fixed reason and emits the same single `AuthenticationFailed` event as
> any other rejection. Failing closed here is deliberate: passing a hash-valued `sid`
> onward would "revoke" a family that does not exist, audit a `TokenRevocation` that
> removed nothing, and hide the miss. The exposure window is one `token.access_token_ttl`
> (default 15 minutes) after deploy — the same fail-closed cutover that sibling accepted
> for its own `typ`/`sid` transition.
>
> In **Revocation**, the access-token arm becomes: `validate_access_token(token)` returns
> claims whose `sid` names the token's family; `revoke_family(sid)` removes the family's
> live generation and its retirement records (audited `TokenRevocation`, with the removed
> count in `detail`). One family is one sign-in, so the authority is unchanged: the
> credential revokes exactly the session it was minted for, under whichever generation that
> session has rotated to. The subject's other families are untouched, and
> `revoke_all_user_sessions` remains unreachable from this endpoint.

`sid` stays in `RESERVED_CLAIMS` exactly as that sibling specified — the reservation
exists because the name carries revocation authority, not because of the shape of its
value — so the reserved-claim set needs no further delta.

### `.specs/service/specs/01-domain-model.md` → Session / Token types (Modify)

The two `sid` sentences the same sibling added follow the re-pointing:

> `refresh_token_hash` identifies one **generation** and remains the key every
> `SessionRepository` lookup and revocation takes; `family_id` identifies the session
> across rotations, and it is the value minted access tokens carry as their `sid` claim, so
> a presented access token names the session it belongs to independently of how often the
> refresh token has rotated.
>
> - **`AccessTokenClaims`** — JWT payload: `sub` (internal user id), `iss`, `aud`, `iat`,
>   `exp`, `sid` (the `family_id` of the session the token was minted for), plus a
>   flattened `custom: HashMap<String, Value>` of resolved claims. All six registered
>   fields are required on both serialization and deserialization.

### `.specs/service/specs/02-ports-and-adapters.md` → SessionRepository (Modify)

> ### SessionRepository (`ports/repository.rs`)
>
> ```rust
> async fn store_refresh_token(&self, session: &Session) -> Result<()>;
> async fn get_session_by_refresh_token(&self, token_hash: &str) -> Result<Option<Session>>;
> async fn resolve_refresh_token(&self, token_hash: &str) -> Result<RefreshResolution>;
> async fn rotate_refresh_token(&self, live_hash: &str, replacement: &Session) -> Result<bool>;
> async fn revoke_session(&self, token_hash: &str) -> Result<()>;
> async fn revoke_family(&self, family_id: &str) -> Result<u64>;
> async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<()>;
> async fn count_active_sessions(&self) -> Result<u64>;
> async fn cleanup_expired_sessions(&self) -> Result<u64>;  // returns rows deleted
> ```
>
> ```rust
> pub enum RefreshResolution {
>     /// The hash is the family's live generation.
>     Live(Session),
>     /// The hash is retired and the successor it names is still the family's
>     /// live generation. `live` is that successor.
>     Superseded { live: Session, retired_at: DateTime<Utc> },
>     /// The hash is retired and its successor is no longer live.
>     Retired { family_id: String, user_id: String, retired_at: DateTime<Utc> },
>     /// No live generation and no retained retirement record matches.
>     Unknown,
> }
> ```
>
> The port classifies; it does not decide policy. `Superseded` is a storage fact — the
> successor pointer still names the live generation — and the grace window that turns it into
> either a rotation or a reuse alarm is evaluated once in the core against
> `token.refresh_rotation_grace`, not five times across the adapters.
>
> Five obligations attach to the session port. They are contract, not description: an adapter
> either meets them or it does not ship.
>
> | | Obligation |
> |---|---|
> | **SR1** | **Consistency.** `resolve_refresh_token` is strongly consistent with the most recent write. Its negative and retired answers *are* security outcomes — an eventually consistent read turns a revoked token into a live one and a reuse alarm into a silent rejection. |
> | **SR2** | **Atomicity.** `rotate_refresh_token` applies its three effects — delete the live session, write the retirement record, install the replacement — as one atomic unit conditioned on `live_hash` still being live, or applies none of them. A partial application either strands the old generation as still-valid or locks the holder out of a session they legitimately hold. |
> | **SR3** | **Single live generation.** At most one generation of a family is live at any instant, under concurrent redemption. Two callers redeeming the same hash produce exactly one `true` return. |
> | **SR4** | **Retirement durability.** By the time a rotation is observable, the retirement record it wrote is readable. A rotation whose replacement is visible before its retirement record leaves a window in which reuse reads as `Unknown`. |
> | **SR5** | **Revocation completeness.** `revoke_family` removes the family's live generation and every retained retirement record, and returns the count removed, or it errors. `revoke_all_user_sessions` gives the same removal guarantee across all of a user's families (its `Result<()>` signature is unchanged). Neither reports success for work it did not do. |
>
> `store_refresh_token` writes the generation-0 row of a new family. `get_session_by_refresh_token`
> remains for `/revoke`, which needs only liveness.

### `.specs/service/specs/02-ports-and-adapters.md` → Session-store conformance suite (Add)

> ## Session-store conformance suite
>
> `crates/test-utils/src/session_contract.rs` exports the obligations above as generic
> assertions over any `impl SessionRepository`. Every session adapter — DynamoDB, Postgres,
> SQLite, LMDB, Valkey — and `MockRepository` invoke the same suite from their own test
> module, so the guarantee is a property the project asserts rather than one it assumes. The
> suite covers, at minimum:
>
> - a redemption returns a new generation and the presented one no longer resolves as `Live`;
> - two concurrent `rotate_refresh_token` calls against the same `live_hash` produce exactly
>   one `true` (SR3);
> - a failed compare-and-swap leaves the store byte-identical (SR2);
> - a retirement record is readable the instant its rotation is (SR4);
> - a generation retired more than one rotation ago resolves as `Retired`, not `Unknown`;
> - `revoke_family` removes the live generation and every retirement record, and its count
>   matches (SR5);
> - `resolve_refresh_token` immediately after `revoke_session` returns `Unknown` (SR1);
> - the replacement's `expires_at` equals the retired generation's.
>
> Adapters needing a live backend keep their existing `#[ignore]` gating and environment-variable
> URLs; the suite runs against them in the integration job, and against SQLite, LMDB and
> `MockRepository` on every build.

### `.specs/service/specs/01-domain-model.md` → Session (Modify)

> ### Session (`domain/session.rs`)
>
> ```rust
> struct Session {
>     user_id: String,
>     refresh_token_hash: String,       // SHA-256 hex; never the raw token
>     family_id: String,                // "fam_…"; stable across every rotation
>     generation: u32,                  // 0 at exchange, +1 per rotation
>     provider: String,
>     expires_at: DateTime<Utc>,        // absolute; set at exchange, never moved
>     rotated_at: Option<DateTime<Utc>>, // when this generation was issued; None at generation 0
>     device_id: Option<String>,
>     user_agent: Option<String>,
>     ip_address: Option<String>,
>     created_at: DateTime<Utc>,        // when the family was created
> }
> ```
>
> A `Session` is one **generation** of a token family. The raw refresh token exists only in
> memory during issuance and in the response to the client; only the hash is stored. `family_id`
> and `created_at` identify the sign-in and survive rotation; `expires_at` is the family's
> absolute deadline and is copied unchanged into every replacement.

### `.specs/service/specs/01-domain-model.md` → RetiredRefreshToken (Add)

> ### RetiredRefreshToken (`domain/session.rs`)
>
> ```rust
> struct RetiredRefreshToken {
>     refresh_token_hash: String,       // SHA-256 hex of the retired generation
>     family_id: String,
>     user_id: String,
>     successor_hash: String,           // the generation that replaced it
>     retired_at: DateTime<Utc>,
>     expires_at: DateTime<Utc>,        // min(retired_at + reuse retention, family expires_at)
> }
> ```
>
> The record that makes reuse detectable. It is written by the same atomic operation that
> retires the generation it names, and reaped by the same expiry machinery as sessions —
> DynamoDB and Valkey natively, SQL and LMDB through `cleanup_expired_sessions`. A generation
> presented after its record has expired resolves as `Unknown`: it is refused, but it raises
> no alarm.

### `.specs/service/specs/01-domain-model.md` → Token types (Modify)

> - **`TokenResponse`** — the `/token` body: `access_token`, optional `refresh_token`
>   (present on exchange and on refresh; absent on refresh only when
>   `token.refresh_rotation = false`), `token_type` (always `"Bearer"`), `expires_in` seconds.

### `.specs/service/specs/01-domain-model.md` → AuditEvent (Modify)

> `AuditEventType` variants: `TokenExchange`, `TokenRefresh`, `TokenRevocation`,
> `SessionRevoked`, `AllSessionsRevoked`, `UserCreated`, `UserUpdated`, `UserSuspended`,
> `UserDeleted`, `ValidationFailed`, `RegistrationDenied`, `ProviderError`, `Unauthorized`,
> `RefreshTokenReuse`.

### `.specs/service/specs/01-domain-model.md` → ID scheme (Modify)

> | Entity | Identifier | Form | Generated where |
> |---|---|---|---|
> | User | `User.id` | `usr_` + lowercase ULID | the repository adapter on `create_user` |
> | Session family | `Session.family_id` | `fam_` + lowercase ULID | core (`exchange`) |
> | Session generation | `Session.refresh_token_hash` | SHA-256 hex of the opaque refresh token | core (`exchange`, `refresh`) |
> | AuditEvent | `AuditEvent.id` | bare ULID | core (`create_audit_event`) |

### `.specs/service/specs/01-domain-model.md` → Session lifecycle (Modify)

> ### Session
>
> ```
>   exchange                refresh                    refresh
>      │                       │                          │
>      ▼                       ▼                          ▼
>  ┌────────┐  rotate     ┌────────┐  rotate         ┌────────┐
>  │ gen 0  │ ──────────► │ gen 1  │ ──────────────► │ gen 2  │  ◄── live
>  └───┬────┘             └───┬────┘                 └────────┘
>      │ retired              │ retired
>      ▼                      ▼
>  ┌──────────────────────────────────┐
>  │ RetiredRefreshToken records      │  presenting one of these:
>  │ (kept for the reuse-retention    │   · successor still live, inside grace → rotate
>  │  window, capped at family expiry)│   · otherwise → reuse: revoke the whole family
>  └──────────────────────────────────┘
> ```
>
> A family is created on token exchange with `expires_at = now + refresh_token_ttl` and
> generation 0. Each refresh advances the live generation and retires its predecessor; the
> absolute `expires_at` never moves, so the family dies at the deadline set at sign-in however
> often it rotates. A family ends at that deadline, on explicit revocation (`/revoke`,
> delete-user, revoke-all-user-sessions), or on reuse detection.

### `.specs/service/specs/01-domain-model.md` → Required query patterns (Modify)

> | Classify a presented refresh token | `SessionRepository::resolve_refresh_token(hash)` |
> | Rotate the live generation | `SessionRepository::rotate_refresh_token(live_hash, replacement)` |
> | Revoke one family | `SessionRepository::revoke_family(family_id)` |

### `.specs/service/specs/00-overview.md` → Scope summary (Modify)

> | Token refresh, revocation | Yes | rotating refresh tokens with reuse detection; rotation switchable |

### `.specs/service/specs/00-overview.md` → Decisions (Modify)

> - *Opaque, hashed, rotating refresh tokens.* **256-bit random, stored as a SHA-256 hash,
>   single-use, valid until the family's absolute expiry or revocation.** Revocable and
>   leak-resistant, and consuming the credential on use is what makes a second holder
>   observable; `token.refresh_rotation = false` restores reusable tokens for deployments with
>   clients that cannot discard a rotated token.

### `.specs/service/specs/00-overview.md` → Assumptions (Modify)

The external-scheduler Assumption — "A scheduler external to the service drives
`SessionRepository::cleanup_expired_sessions` where the store does not expire rows itself"
— assigned the retention boundary to a party no operator-facing document ever told about
it. It is replaced; the boundary becomes something the service owns:

> - Long-lived runtimes reap expired sessions and retirement records themselves — the
>   bootstrap-spawned session reaper ([04-http-api.md](specs/04-http-api.md)). Deployments
>   with no long-lived process (Lambda) drive `POST /internal/sessions/cleanup` from an
>   external scheduler such as EventBridge. DynamoDB TTL and Valkey key expiry reap
>   natively; the reaper is a backstop there.

### `.specs/service/specs/04-http-api.md` → POST /token request (Modify)

> The client names the provider (`provider=google`), not a raw issuer URL. Unknown
> `grant_type` → `unsupported_grant_type`. Response body is `TokenResponse`
> ([01-domain-model.md](01-domain-model.md)); it carries a `refresh_token` on every grant,
> including `refresh_token`, and a client must discard the token it presented once it holds
> the replacement (RFC 6749 §6). With `token.refresh_rotation = false` the refresh grant
> returns no `refresh_token` and the presented one stays valid.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

The retirement table raises the stakes on a sweep nothing currently schedules.
`cleanup_expired_sessions` has no production caller — only the trait, five adapter
implementations, and test call sites — so on the SQL and LMDB stores expired sessions and
the `ip_address`/`user_agent`/`device_id` they captured are retained indefinitely, and this
change would have retirement records join them
(`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-expired-sessions-never-cleaned/`).
The canonical pages assume "a scheduler external to the service" that no operator-facing
document ever mentions. These blocks give the sweep an owner. A new step follows the
runtime-detect step:

> 7. Under a long-lived runtime — hyper, and a `crates/ffi` embedder whose host process
>    persists — spawn the **session reaper**: a periodic task that calls
>    `SessionRepository::cleanup_expired_sessions` every
>    `session_repository.cleanup_interval` (default `"1h"`), logs the deleted count on every
>    run — a silently dead reaper must be distinguishable from one with nothing to delete —
>    and is aborted with the graceful-shutdown drain. Under Lambda there is no long-lived
>    process to host the task; the reaper is not spawned, and the same control is reachable
>    as `POST /internal/sessions/cleanup` for an external scheduler (EventBridge) to drive
>    on the deployment's own cadence.

### `.specs/service/specs/04-http-api.md` → Routes, Internal (Modify)

One row joins the internal-routes table:

> | POST | `/internal/sessions/cleanup` | run `cleanup_expired_sessions` once; returns `{ "deleted": <count> }` |

It exists for the runtimes that cannot host the periodic task — Lambda above all — and
doubles as the operator's manual lever. It sits behind internal auth like every other
`/internal/*` route and mutates nothing but expired rows, so it is safe to invoke on any
schedule alongside a running reaper.

### `.specs/service/specs/06-configuration.md` → Validation at load (Modify)

> - `server.request_timeout`, `token.access_token_ttl`, `token.refresh_token_ttl`,
>   `token.refresh_rotation_grace`, `token.refresh_reuse_retention`, and
>   `session_repository.cleanup_interval` must parse as `<integer><s|m|h|d>` without
>   overflow; the parsed values are reused at request time, which therefore cannot fail.
>   `refresh_rotation_grace` must additionally be at most `60s`.

### `.specs/service/specs/06-configuration.md` → `[token]` (Modify)

> ### `[token]`
> `access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), optional `audience`, optional
> `custom_claims` (`HashMap<String,String>` of claim templates, see
> [03-service-flows.md](03-service-flows.md)), `refresh_rotation` (bool, default `true`),
> `refresh_rotation_grace` (duration string, default `"10s"`) and `refresh_reuse_retention`
> (duration string, default `"24h"`).
>
> `refresh_rotation_grace` is the window in which the immediately-preceding generation is
> still redeemable; config loading rejects a value above `60s`, because the window is a
> deliberate weakening and an unbounded one is indistinguishable from no rotation.
> `refresh_reuse_retention` is how long a retired generation is remembered so its
> re-presentation raises an alarm; it is capped per record at the family's own `expires_at`.

### `.specs/service/specs/06-configuration.md` → `[session_repository]` (Modify)

Appended to the section:

> `cleanup_interval` (duration string, default `"1h"`) — how often the long-lived runtimes
> run `cleanup_expired_sessions` ([04-http-api.md](04-http-api.md) → Bootstrap). The sweep
> covers `sessions` and `retired_refresh_tokens` alike; on the natively-expiring stores
> (DynamoDB TTL, Valkey key expiry) it is a cheap backstop for whatever native expiry has
> not yet reaped.

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | `token.refresh_rotation` / `refresh_rotation_grace` / `refresh_reuse_retention` | `true` / `"10s"` / `"24h"` |
> | `session_repository.cleanup_interval` | `"1h"` |

### `.specs/service/specs/08-persistence.md` → DynamoDB (Modify)

> | Item | pk | sk | GSI1pk | GSI1sk |
> |---|---|---|---|---|
> | Session | `SESSION#<refresh_token_hash>` | `SESSION` | `USER#<user_id>` | `FAM#<family_id>#SESSION#<created_at>` |
> | Retired refresh token | `RETIRED#<refresh_token_hash>` | `RETIRED` | `USER#<user_id>` | `FAM#<family_id>#RETIRED#<retired_at>` |
>
> `resolve_refresh_token` issues strongly consistent `GetItem`s — `consistent_read(true)` on
> both the `SESSION#` and the `RETIRED#` lookup, matching what `get_user_by_external_id`
> already does on the identity path. Both answers carry a security decision: an eventually
> consistent `SESSION#` read lets a revoked token mint an access token for the width of the
> replication window, and an eventually consistent `RETIRED#` read reports reuse as an unknown
> token, which is refused but raises no alarm.
>
> `rotate_refresh_token` is one `TransactWriteItems`: a `Delete` of the live session item
> conditioned on `attribute_exists(pk)`, a `Put` of the retirement item, and a `Put` of the
> replacement session conditioned on `attribute_not_exists(pk)`. A `TransactionCanceledException`
> whose reasons include `ConditionalCheckFailed` means the live generation moved and maps to a
> `false` return, not an error; any other transaction failure maps to `Error::StoreError`.
> Retirement items carry the same numeric `ttl` attribute as sessions, so DynamoDB reaps them
> natively. `revoke_family` queries GSI1 on `USER#<user_id>` with
> `begins_with(GSI1sk, "FAM#<family_id>")` and batch-deletes with the same
> `unprocessed_items` retry discipline as `revoke_all_user_sessions`.
>
> **The GSI is an index, not the roster.** GSI1 is eventually consistent, so a session written
> moments before a revocation can be absent from the index at query time and survive the sweep
> permanently — the token stays live with nothing left to find it (`g3-dynamo-revoke-all-gsi-incompleteness`).
> Enumeration therefore reads an authoritative list instead: the user item
> (`pk = USER#<user_id>`, `sk = USER`) carries a `sessions` attribute, a string set of the live
> `refresh_token_hash` values, and a `families` attribute mapping each `family_id` to its
> member hashes. Every write that creates or removes a session updates that list **inside the
> same `TransactWriteItems`** as the session item itself — creation on exchange, the
> delete-plus-put of a rotation, retirement, and revocation — so the list and the items cannot
> disagree, and a strongly consistent `GetItem` on the user item is a complete roster.
> `revoke_family` and `revoke_all_user_sessions` read that roster with `consistent_read(true)`,
> delete exactly the hashes it names, and return the count. GSI1 remains for the admin
> listing paths, where a stale read is a cosmetic defect rather than a surviving credential.
>
> The cost is stated plainly: every session write becomes a transaction touching the user item,
> so a single user's concurrent logins now contend on one item, and the item grows with the
> user's live session count. That is the price of a revocation that means what it says. The
> conformance suite's SR5 is what proves it: run against the GSI-only implementation it fails,
> which is the point — the suite was written to catch exactly this class of partial removal.

### `.specs/service/specs/08-persistence.md` → PostgreSQL / SQLite (Modify)

One block, two targets: the same text lands in both the `## PostgreSQL (adapters/postgres)`
and `## SQLite (adapters/sqlite)` sections. The DDL shows the Postgres types; SQLite stores
`retired_at` and `expires_at` as `TEXT`, matching its `sessions` table.

> `sessions` carries `family_id TEXT`, `generation INTEGER NOT NULL DEFAULT 0` and
> `rotated_at` alongside the existing columns, added by the same idempotent `ALTER TABLE` step
> that added `users.version` (`ADD COLUMN IF NOT EXISTS` on Postgres, a bare `ADD COLUMN` on
> SQLite). `family_id` is nullable: a session row written before rotation shipped needs no
> backfill, and its first redemption mints a family for the replacement and deletes the legacy
> row without writing a retirement record — there is no prior generation to detect reuse
> against. A second table holds the retirement records:
>
> ```sql
> CREATE TABLE IF NOT EXISTS retired_refresh_tokens (
>     refresh_token_hash  TEXT PRIMARY KEY,
>     family_id           TEXT NOT NULL,
>     user_id             TEXT NOT NULL,
>     successor_hash      TEXT NOT NULL,
>     retired_at          TIMESTAMPTZ NOT NULL,
>     expires_at          TIMESTAMPTZ NOT NULL
> );
> CREATE INDEX IF NOT EXISTS idx_retired_family ON retired_refresh_tokens (family_id);
> CREATE INDEX IF NOT EXISTS idx_retired_expires_at ON retired_refresh_tokens (expires_at);
> ```
>
> `rotate_refresh_token` runs its delete, retirement insert and replacement insert inside one
> `BEGIN … COMMIT`. The compare-and-swap condition is the delete's affected-row count: zero
> rows means the live generation moved, the transaction rolls back and the method returns
> `false`. `cleanup_expired_sessions` sweeps both tables and its count covers both.

### `.specs/service/specs/08-persistence.md` → Session-only stores (Modify)

> - **LMDB (`adapters/lmdb`)** — embedded `heed` store with four named databases: `sessions`
>   (hash → session), `user_sessions` (user → set of hashes for revoke-all), `retired_tokens`
>   (hash → retirement record) and `family_index` (`{family_id}\0{hash}` → kind, for
>   `revoke_family`). `rotate_refresh_token` performs all of its reads and writes inside a
>   single `heed` write transaction, which is where its compare-and-swap condition is
>   evaluated. Constructed with a path and a max map size in MB.
>   `cleanup_expired_sessions` commits its deletes in fixed-size batches (256 keys per
>   write transaction) rather than one all-or-nothing transaction. LMDB is copy-on-write,
>   so a delete must allocate dirty pages before it frees old ones: the shipped
>   single-transaction sweep itself fails `MDB_MAP_FULL` on a map filled past roughly 95%,
>   which means a reaper wired up after the store has wedged cannot rescue it — recovery
>   from a full map is raising `max_size_mb` and restarting, then reaping. Batching keeps
>   the reaper effective up to the boundary; the scheduled reaper is what keeps a healthy
>   deployment from ever reaching it.
> - **Valkey/Redis (`adapters/valkey`)** — adds `{prefix}retired:{hash}` hashes and a
>   `{prefix}family:{family_id}` set to the existing session keys, both TTL'd like the session
>   keys they accompany. `rotate_refresh_token` runs as one `EVAL`'d Lua script rather than a
>   pipeline: the swap is conditional on the live hash still existing, and a pipeline gives
>   batching without atomicity or a condition. The unconditional writes on the
>   `store_refresh_token` path keep their pipeline.
>   The `{prefix}active_sessions` counter is reconciled state, not an invariant the adapter
>   establishes: it is a permanent key summarising TTL'd ones, related to the session keys
>   only by convention, so natural expiry drives it above the live count and external
>   administration can drive it below. A decrement that returns a negative value therefore
>   **clamps the key to zero and emits one `tracing::warn!`** (`counter_clamped = true`,
>   with the observed value) rather than asserting; the same rule binds `revoke_family` and
>   the rotation script — no counter comparison may unwind. The shipped
>   `assert!(counter >= 0)` on both revoke paths panics on drift, reachable from
>   unauthenticated `POST /revoke`; the catch-panic layers
>   ([2026-08-05-runtime_parity_across_interfaces.md](2026-08-05-runtime_parity_across_interfaces.md))
>   only contain that panic as a `500` on an endpoint whose contract is a token-state
>   `200`, and because the deficit persists, every subsequent effective revocation repeats
>   it until an operator repairs the key.

### `.specs/service/specs/08-persistence.md` → Adapter inventory note (Add)

A new closing paragraph at the end of the `## Session-only stores` section — the page has no
adapter-inventory heading of its own; this note points at the inventory in
[02-ports-and-adapters.md](../service/specs/02-ports-and-adapters.md).

> Every session adapter stores retirement records alongside sessions and passes the
> session-store conformance suite ([02-ports-and-adapters.md](02-ports-and-adapters.md)),
> which is what makes rotation and reuse detection a property of the port rather than of
> whichever backend a deployment happens to configure.

---

## Type changes

```json
{
  "$comment": "Fragment for 2026-08-05-rotate_refresh_tokens_with_reuse_detection. Folds into .specs/service/specs/canonical-types.schema.json on merge; the same field additions apply to schemas/datamodel.schema.json in its untyped form. AuditEventType gains refresh_token_reuse. AccessTokenClaims shows only the property this change re-patterns: sid moves from the refresh_token_hash form merged by 2026-08-05-validate_revoke_token_claims to the family_id form.",
  "$defs": {
    "AccessTokenClaims": {
      "properties": {
        "sid": {
          "type": "string",
          "pattern": "^fam_[0-9a-z]{26}$",
          "description": "Session identifier: the family_id of the session this access token was minted for. Stable across refresh-token rotation; names the only session (token family) the token may revoke. Supersedes the ^[0-9a-f]{64}$ refresh_token_hash pattern."
        }
      }
    },
    "Session": {
      "type": "object",
      "description": "One generation of a refresh-token family. Added: family_id, generation, rotated_at.",
      "required": ["user_id", "refresh_token_hash", "family_id", "generation", "provider", "expires_at", "created_at"],
      "properties": {
        "user_id": { "$ref": "../../canonical-types.schema.json#/$defs/Id" },
        "refresh_token_hash": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString", "description": "SHA-256 hex of the opaque refresh token." },
        "family_id": { "$ref": "../../canonical-types.schema.json#/$defs/Id", "description": "fam_<ulid>; stable across every rotation of one sign-in." },
        "generation": { "type": "integer", "minimum": 0, "description": "0 at exchange, incremented once per rotation." },
        "provider": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
        "expires_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp", "description": "Absolute family deadline; set at exchange and never moved by rotation." },
        "rotated_at": { "type": ["string", "null"], "format": "date-time", "description": "When this generation was issued; null at generation 0." },
        "device_id": { "type": ["string", "null"] },
        "user_agent": { "type": ["string", "null"] },
        "ip_address": { "type": ["string", "null"] },
        "created_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp", "description": "When the family was created." }
      }
    },
    "RetiredRefreshToken": {
      "type": "object",
      "description": "A retired generation, retained so that its re-presentation is detectable as reuse.",
      "required": ["refresh_token_hash", "family_id", "user_id", "successor_hash", "retired_at", "expires_at"],
      "additionalProperties": false,
      "properties": {
        "refresh_token_hash": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString" },
        "family_id": { "$ref": "../../canonical-types.schema.json#/$defs/Id" },
        "user_id": { "$ref": "../../canonical-types.schema.json#/$defs/Id" },
        "successor_hash": { "$ref": "../../canonical-types.schema.json#/$defs/NonEmptyString", "description": "The generation that replaced this one. Grace applies only while this is still the family's live generation." },
        "retired_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp" },
        "expires_at": { "$ref": "../../canonical-types.schema.json#/$defs/Timestamp", "description": "min(retired_at + token.refresh_reuse_retention, family expires_at)." }
      }
    },
    "TokenResponse": {
      "type": "object",
      "required": ["access_token", "token_type", "expires_in"],
      "properties": {
        "access_token": { "type": "string" },
        "refresh_token": { "type": ["string", "null"], "description": "Present on exchange and on refresh; null on refresh only when token.refresh_rotation is false." },
        "token_type": { "type": "string", "const": "Bearer" },
        "expires_in": { "type": "integer", "minimum": 0, "description": "Seconds until the access token expires." }
      }
    },
    "AuditEventType": {
      "type": "string",
      "enum": [
        "token_exchange", "token_refresh", "token_revocation", "session_revoked",
        "all_sessions_revoked", "user_created", "user_updated", "user_suspended",
        "user_deleted", "validation_failed", "registration_denied", "provider_error",
        "unauthorized", "refresh_token_reuse"
      ]
    }
  }
}
```

`AuditEventType` gains a `RefreshTokenReuse` variant (`crates/core/src/domain/audit.rs:39-53`).
The canonical schema closes `AuditEventType` over an enum, so it gains `refresh_token_reuse`
(the `$def` above); `schemas/datamodel.schema.json` keeps `event_type` as an open string
(`{ "type": "string" }`) and needs no change for it.

---

## Implementation notes

Order matters: the port and its conformance suite land before the service change, so no
adapter can ship a non-atomic rotation.

1. **Domain** — `crates/core/src/domain/session.rs`: add `family_id`, `generation`,
   `rotated_at` to `Session`; add `RetiredRefreshToken`. Add `RefreshTokenReuse` to
   `AuditEventType` (`crates/core/src/domain/audit.rs:39-53`).
2. **Config** — `crates/core/src/config.rs:183-199`: add `refresh_rotation: bool` (default
   `true`), `refresh_rotation_grace: String` (`"10s"`), `refresh_reuse_retention: String`
   (`"24h"`) to `TokenConfig`; reject a grace above 60s in `bootstrap::load_config`. Add the
   three keys to `config/default.toml`.
3. **Port** — `crates/core/src/ports/repository.rs:24-34`: add `RefreshResolution`,
   `resolve_refresh_token`, `rotate_refresh_token`, `revoke_family`, and write the five SR
   obligations as the trait's doc comments, not as prose elsewhere.
4. **Conformance suite** — new `crates/test-utils/src/session_contract.rs` exporting generic
   assertions over `impl SessionRepository`. `oidc-exchange-test-utils` is already a
   dev-dependency of `crates/adapters` (`crates/adapters/Cargo.toml:35-41`), so each adapter's
   `#[cfg(test)]` module can call the suite with no new wiring. Wire `MockRepository`
   (`crates/test-utils/src/lib.rs:220`) through it too.
5. **Adapters**, in ascending order of difficulty:
   - `crates/adapters/src/sqlite/mod.rs:40-51` — DDL plus the `ALTER TABLE` upgrade step;
     rotation in a `sqlx` transaction.
   - `crates/adapters/src/postgres/mod.rs:42-53` — same, with `ADD COLUMN IF NOT EXISTS`.
   - `crates/adapters/src/lmdb/mod.rs:56-95` — two more `heed` databases; rotation inside the
     existing `write_txn` pattern in `spawn_blocking`.
   - `crates/adapters/src/valkey/mod.rs:53-140` — retired hashes and the family set; an
     `EVAL`'d Lua script for the swap. The "pipeline, not MULTI" decision recorded for
     `store_refresh_token` does not carry over: that path's writes are unconditional and this
     one's are not. While in the file, replace the two `assert!(counter >= 0)` on the revoke
     paths (`crates/adapters/src/valkey/mod.rs:245-248`, `:468-471` — the
     `g3-valkey-active-sessions-counter-assert` finding; the panic is reachable from
     unauthenticated `POST /revoke`) with a clamp: `SET` the counter to `0` and emit one
     `tracing::warn!` with `counter_clamped = true` and the observed value. `revoke_family`
     and the Lua script follow the same rule. Test: with the counter seeded to `0` and a
     live session revoked, the call returns `Ok`, the key reads `0`, and a warning was
     emitted — no panic.
   - `crates/adapters/src/dynamo/mod.rs:653-669` — add `consistent_read(true)` here (the
     `g3-dynamo-session-read-eventual-consistency` fix; the precedent is at
     `crates/adapters/src/dynamo/mod.rs:263-272`, and the same finding names the refresh
     path's status-gate read `get_user_by_id` at `crates/adapters/src/dynamo/mod.rs:230-246`,
     which takes the same one-line fix in the same commit); add the `sessions` string set and
     the `families` map to the user item and update them inside every session-writing
     `TransactWriteItems`, then re-point `revoke_family` and `revoke_all_user_sessions`
     (`crates/adapters/src/dynamo/mod.rs`) from the GSI1 query to a `consistent_read(true)`
     `GetItem` on the user item — this is the `g3-dynamo-revoke-all-gsi-incompleteness` fix,
     and SR5 in the conformance suite is what holds it in place; add the `RETIRED#` item, change the session
     item's `GSI1sk` to the `FAM#…` form, and implement rotation as `TransactWriteItems`
     mirroring the `create_user` transaction at the same module's user path.
6. **Service** — `crates/core/src/service/exchange.rs:291-313`: mint `family_id`, set
   `generation = 0`, `rotated_at = None`. `crates/core/src/service/refresh.rs:22-128`: replace
   the body with the flow above. Keep the ordering — resolve, reuse check, expiry, user status,
   mint, swap, sign, audit, respond — so a suspended user is turned away before any write.
7. **Tests** — `crates/core/tests/refresh.rs`: a redemption returns a new token and the old one
   stops working; a superseded token inside the grace window rotates and raises no alarm; the
   same token outside the window revokes the family and emits `RefreshTokenReuse` at `Warning`;
   the replacement's `expires_at` equals the original (assert this directly — it is the
   regression most likely to arrive with the fix); `refresh_rotation = false` reproduces
   today's behaviour exactly.
8. **`sid` re-pointing** — `crates/core/src/service/mod.rs` (`build_access_token`): the `sid`
   parameter now receives the `family_id`; call sites in `exchange.rs` pass the family just
   minted and in `refresh.rs` the family just rotated. `validate_access_token` step 7 gains
   the `fam_`-form shape check, rejecting hash-valued `sid`s from pre-rotation tokens with a
   fixed reason. `crates/core/src/service/revoke.rs`: the access-token arm calls
   `revoke_family(claims.sid)` in place of `revoke_session`. Tests: the `sid` assertions the
   revoke sibling added to `crates/core/tests/exchange.rs` and `refresh.rs` change from
   "equals the SHA-256 hex of the refresh token" to "equals the session's `family_id`, and is
   unchanged across a rotation"; a token carrying a 64-hex `sid` revokes nothing and emits
   one `ValidationFailed`; a valid token's revocation removes the live generation *and* the
   family's retirement records.
9. **Session reaper** — `crates/server/src/bootstrap.rs`: spawn the periodic task
   (`tokio::time::interval` with `MissedTickBehavior::Skip`) for the long-lived runtimes,
   aborted on shutdown alongside the existing drain in `crates/server/src/main.rs`; add
   `cleanup_interval` to `SessionRepositoryConfig` (`crates/core/src/config.rs:264-269`) and
   `config/default.toml`; add the `POST /internal/sessions/cleanup` handler to
   `crates/server/src/routes/internal.rs`. `crates/adapters/src/lmdb/mod.rs:241-304`: split
   the sweep's single write transaction into 256-key batches, committing each — the
   finding's headroom probe shows the one-transaction shape failing `MDB_MAP_FULL` at ≥99%
   full while batches keep freeing pages. Tests: an expired session (and an expired
   retirement record) is gone after one reaper tick and a live one survives; the internal
   endpoint requires internal auth and returns the count; LMDB reaps a map filled past 95%
   (adapt the finding's `poc/tests/zz_lmdb_probes.rs`).

Reference PoC: the finding ships a four-probe harness at
`.security/oidc-exchange/53cbdec9_20260804T102454Z/findings/g1-refresh-token-never-rotated/poc/`
whose probes 1 and 2 must fail against a fixed build; they are ready-made negative tests.

References: RFC 6749 §6 and §10.4, RFC 9700 (refresh tokens for public clients must be
sender-constrained or one-time use, and servers must detect and respond to replay), and
`hardening/proposals/credential-lifecycle-contract.md` Option 2. The session-reaper and
Valkey-clamp deltas implement `findings/g1-expired-sessions-never-cleaned/` and
`findings/g3-valkey-active-sessions-counter-assert/` in the same sealed bundle.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to
   the merge date.
2. Fold the `Type changes` `$defs` into `.specs/service/specs/canonical-types.schema.json` —
   including `refresh_token_reuse` on the existing `AuditEventType` enum and the re-patterned
   `AccessTokenClaims.sid` — and mirror the `Session` field additions and the new
   `RetiredRefreshToken` definition into `schemas/datamodel.schema.json` (its open-string
   `event_type` needs no change).
3. Remove the superseded Decisions — "*Refresh does not rotate*" in 03-service-flows.md,
   "*Opaque, hashed, reusable refresh tokens*" in 00-overview.md, and "*`sid` is the
   session's refresh-token hash*" in 03-service-flows.md (merged there by
   [2026-08-05-validate_revoke_token_claims.md](2026-08-05-validate_revoke_token_claims.md))
   — replacing each with the blocks above rather than leaving both. In the same pass update
   that sibling's other merged `sid` text per the *Build access token / Validate access
   token / Revocation* and *Session / Token types* blocks, and replace 00-overview.md's
   external-scheduler Assumption per its block.
4. No new canonical page.
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs section (this file is not yet listed there; add
   its row directly as `Merged`).

---

## Assumptions and open questions

### Assumptions

- Clients follow RFC 6749 §6 and discard the presented refresh token once a response carries a
  replacement. Clients that cannot are the reason `token.refresh_rotation` exists.
- `parse_duration_secs` (integer plus `s`/`m`/`h`/`d`) is the parser for the two new duration
  values, as it is for the existing TTLs.
- The reuse alarm reaches somewhere an operator watches. Under the shipped
  `audit.adapter = "noop"` it does not; the family is still revoked, but the operator learns
  about the compromise from a support ticket.
- `crates/test-utils` remains a dev-dependency of `crates/adapters`, so the conformance suite
  needs no new crate.

### Decisions

- *Revocation enumerates an authoritative roster, not an index.* **On DynamoDB, `revoke_family`
  and `revoke_all_user_sessions` read a `sessions` string set on the user item under
  `consistent_read(true)`, maintained transactionally with every session write; GSI1 is kept
  only for admin listing.** An eventually consistent index can omit a session written moments
  earlier, and a revocation that silently misses one leaves a live credential with nothing left
  to find it — the failure is permanent, not transient, which is what separates this from the
  other consistency defects in this change. The alternative considered was documenting a weaker
  guarantee for this backend, which was rejected: SR5 is a security obligation, and a port
  contract with a per-adapter exemption for the adapter most deployments use is not a contract.
  The cost is real and accepted — session writes contend on one item per user, and that item
  grows with live session count.

- *Compare-and-swap on the port, not a read-then-write in the service.* **`rotate_refresh_token`
  is a single conditional store operation.** `store_refresh_token` followed by `revoke_session`
  is two port calls with a window between them, and a failure in that window either strands the
  retired token as still-valid or destroys a session the holder legitimately owns — on every one
  of the five adapters, none of which can make the pair atomic from outside.
- *Rotation preserves the absolute expiry.* **The replacement inherits `expires_at` and
  `created_at`.** Recomputing the expiry per rotation converts a bounded 30-day session into one
  that never dies while it is used, removing the single bound that today limits a stolen token.
- *A grace window, bounded and on by default.* **`token.refresh_rotation_grace`, default `10s`,
  rejected above `60s`.** Rotation with no grace turns every lost response into a forced
  re-authentication and, with reuse detection armed, into a family revocation — which is a worse
  product and not a better security posture. Ten seconds covers a retried HTTP round trip and
  little else.
- *Grace re-rotates rather than replaying.* **The immediately-preceding generation, inside the
  window, rotates forward from the current live generation.** The idiomatic answer — return the
  same token again idempotently — is not available here: the store keeps only SHA-256 digests,
  so the service cannot reproduce a plaintext it has already handed out and discarded.
- *Grace is carried by the successor pointer, not a flag.* **A retirement record grants grace
  only while the successor it names is still live.** "Immediately preceding" and "at most once"
  become the same condition, which removes a mutable bit from a record that five adapters would
  otherwise each have to update atomically.
- *Reuse revokes the family.* **`revoke_family(family_id)`, not `revoke_all_user_sessions`.**
  The evidence is that one login chain leaked; ending the user's other sessions is not
  proportionate to it.
- *Retirement records are time-bounded.* **`token.refresh_reuse_retention`, default `24h`,
  capped per record at the family's `expires_at`.** Remembering every generation for the family's
  full 30 days would accumulate roughly 2 900 records per continuously-refreshing session (30d at
  the 15m access-token cadence); 24 hours holds about 96 and still covers the case that matters —
  an attacker racing the legitimate holder for the current credential. Beyond the window a stolen
  generation is refused as unknown rather than alarmed on, which is the cost.
- *Reuse is invisible to the presenter.* **The reuse branch returns the same `InvalidToken` and
  reason string as the unknown-token branch.** Telling a presenter that they tripped a detector
  tells an attacker exactly when to stop.
- *No token hash in audit detail.* **`TokenRefresh` carries `detail { family_id, generation,
  grace }`, `RefreshTokenReuse` carries `detail { family_id, sessions_revoked }`, and no token
  hash or digest ever appears in either.** Family and generation are enough to correlate
  redemptions of one credential chain, and the
  scan's `g3-lmdb-token-hash-span-exposure` and `g3-valkey-session-span-exposure` findings are
  about digests reaching telemetry — this change should not add a third.
- *Rotation is switchable, not mandatory.* **`token.refresh_rotation`, default `true`.** It
  preserves the compatibility the previous decision was protecting while making the detectable
  posture the default; setting it to `false` is an explicit operator choice that restores the
  behaviour this change exists to remove.

### Open questions

- Whether a reuse alarm should escalate beyond the family — suspending the user, or revoking
  every family they hold — is undecided. Family-scoped is the proportionate default; an operator
  who wants the stronger response has no switch for it here.
- Migration sequencing across a fleet is not specified. The hardening proposal recommends three
  phases (issue rotated tokens while still accepting the presented one, then reject, then arm
  family revocation) so the reuse rate can be observed before the response is turned on. That
  needs a config surface this change does not define, or a documented rolling procedure using
  `refresh_rotation` alone.
- `token.refresh_token_ttl` stays at `"30d"`. Rotation bounds a stolen generation to one use,
  which is the reason not to shorten it in the same change, but 30 days remains a long absolute
  window and is worth re-arguing separately.
- A session row written before rotation shipped carries no `family_id` or `generation`, yet the
  domain `Session` (and the schema fragment) require both. The SQL migration leaves the columns
  nullable and the flow deletes the legacy row on its first redemption, but how such a row is
  represented when `resolve_refresh_token` returns it — an optional family on the domain type,
  a sentinel, or adapter-side synthesis — is unspecified, and the DynamoDB, LMDB and Valkey
  adapters hold the same pre-rotation rows with no stated migration at all.
- With rotation switched off after a rotation-enabled period, leftover retirement records are
  refused as `Unknown` with no alarm (the flow's rotation-disabled paragraph). The evidence of
  two holders is no weaker for the switch being off, so whether such a hit should still emit
  `RefreshTokenReuse` — alarming without revoking — is undecided.
