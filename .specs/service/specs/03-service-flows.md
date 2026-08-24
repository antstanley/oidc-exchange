# Service Flows

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/core/src/service

`AppService` orchestrates the ports. It holds `user_repo`, `session_repo`, `keys`, `audit`,
`user_sync`, the configured retained `rate_limiter`, a `providers` map, and `config`. The
retained limiter enforces provider and subject budgets across service-flow calls; the server
retains a configured public limiter in `AppState` for per-IP HTTP throttling. The flows below
live in `crates/core/src/service/{exchange,refresh,revoke,user_admin,claims,maintenance}.rs`
and the helpers in `service/mod.rs`.

## Token exchange (`exchange.rs`)

`POST /token` with `grant_type=authorization_code` or `grant_type=id_token`. The handler has
already parsed the form into a `TokenGrant`, so `AppService::exchange` receives an
`ExchangeRequest` whose `credential` names the grant that was declared
([04-http-api.md](04-http-api.md)). Emission is terminal and single: the flow maps its result
to exactly one `SecurityEvent`, with fixed classification strings rather than upstream error
text (an assertion-binding rejection's detail-enriched `ValidationFailed` record is that
terminal event, and infrastructure store failures are 5xx conditions, not recorded
authentication outcomes). Success is emitted after storing the session and signing the access
token; under `audit.durability = "enforce"`, a failed terminal emit revokes that just-stored
session before returning the error. Principal creation is a separate state-change event, so a
losing JIT-registration racer emits none.

1. **Resolve provider** — look up `request.provider` in the `providers` map; missing →
   `UnknownProvider`. Then consume one unit of the per-provider rate budget — a `Deny` is
   `TooManyRequests` before any outbound provider work.
2. **Obtain verified claims** — match on `request.credential`:
   - `ExchangeCredential::AuthorizationCode { code, redirect_uri }` → `provider.exchange_code`
     to get `ProviderTokens`, then `validate_id_token` on the returned `id_token`.
   - `ExchangeCredential::IdTokenAssertion { id_token }` → `provider.validate_id_token`.

   Both fields of the authorization-code variant are non-optional, so the `redirect_uri`
   binding is a property of the type rather than a runtime check: there is no field
   combination that reaches this step carrying a credential for one grant while executing
   another. After validation, one unit of the per-subject budget (keyed by provider plus a
   hash of the subject — never the raw subject) is consumed before any store side effect.
3. **Bind the assertion** — every accepted ID token, on both grant paths, passes
   `service::assertion::bind`, which runs in this order and rejects with `InvalidGrant` at
   the first failure (each rejection audited as `ValidationFailed`/`Warning` with a
   `detail.check` naming the failed control):
   - **Lifetime ceiling** — `exp - now` must not exceed `grants.max_assertion_lifetime`, so
     the single-use marker below always outlives the assertion it guards.
   - **`azp`** — when `aud` is an array of more than one value, `azp` is required; whenever
     `azp` is present it must equal `provider.client_id()`. A token minted for a sibling
     client of the same provider is rejected.
   - **`at_hash`** — when an access token accompanies the assertion (`provider_access_token`
     on the direct grant, `ProviderTokens.access_token` on the code path) and the assertion
     carries `at_hash`, the claim must equal the base64url of the left-most half of the
     digest of the access token's ASCII octets (OIDC Core §3.1.3.6). The digest follows
     `IdentityClaims.signing_alg`: SHA-256 for `*256`, SHA-384 for `*384`, SHA-512 for
     `*512`. An `at_hash` on an `EdDSA`-signed assertion is unverifiable and is rejected.
     An `at_hash` with no accompanying access token is not verifiable and is skipped.
   - **Nonce (direct grant only)** — the `nonce` claim must be present, and
     `take_single_use("nonce:<sha256hex>")` must report it present. That single atomic
     operation is both the nonce check and the nonce's own one-time-use guarantee: an
     absent, expired, or already-burned nonce is indistinguishable and all three reject.
     The code-exchange path requires no nonce — redeeming a single-use code at the
     provider supplies the binding.
   - **Single use** — `put_single_use(assertion_key, exp)` must report the key newly
     inserted; a key already present means the assertion has been spent and is rejected as
     a replay. `assertion_key` is `assertion:<provider>:<sha256hex(jti)>` when the token
     carries a `jti`, else `assertion:<provider>:d:<sha256hex(compact_jwt)>`; the `d:`
     discriminator keeps a literal `jti` from colliding with a digest. The record's
     `expires_at` is the assertion's own `exp`.

   Store failures during the two atomic operations propagate as typed infrastructure
   errors (`StoreError` → 5xx), never disguised as client-fault rejections; no rejection
   audit is emitted for them.
4. **User lookup / registration policy** — `get_user_by_external_id(subject, provider)`:
   - **Found, suspended** → `UserSuspended` (audited `Unauthorized`/`UserSuspended`).
   - **Found, active** → when `registration.domain_allowlist` is set, re-apply it against the
     assertion's current claims: `email_verified == Some(true)` and a matching domain, using
     the same predicate as the Not-found arm. A failure → `AccessDenied` (audited
     `RegistrationDenied`, naming the user id). The live ID-token claims are used rather than
     the stored `user.email`, which is frozen at first login. `registration.mode` is not
     re-evaluated here: it is an admission gate and is trivially satisfied by an existing user.
   - **Not found** → apply policy. The policy value is a `RegistrationMode`, matched
     exhaustively; there is no unrecognised case because config load rejected it.
     - The ID token must carry a **verified** email (`email_verified == Some(true)`) — a
       requirement of accepting the claim at all, not merely of the allowlist branch. A missing
       or unverified email → `AccessDenied` (audited `RegistrationDenied`).
     - If `registration.domain_allowlist` is set, the email's domain must match it — exact
       (`example.com`) or wildcard (`*.example.com`, at least one subdomain, ASCII
       case-insensitive). A non-matching domain → `AccessDenied` (audited
       `RegistrationDenied`).
     - `RegistrationMode::ExistingUsersOnly` → `AccessDenied` (`RegistrationDenied`).
     - `RegistrationMode::Open` → `create_user(NewUser{…})` (audited `UserCreated`); if
       creation returns `Conflict` (a concurrent first login won the race), re-run
       `get_user_by_external_id` and continue with the existing user, re-applying the
       suspended-status check. The losing racer emits no `UserCreated` event — the winning
       create already audited it — and the flow otherwise proceeds as for a found user.
5. **Mint refresh token** — 32 random bytes, base64url-no-pad for the opaque token; SHA-256
   hex of the bytes is the stored hash.
6. **Store session** — mint the family id (`fam_` + lowercase ULID), set `generation = 0`
   and `rotated_at = None`, `expires_at = now + refresh_token_ttl`; `store_refresh_token`.
7. **Sign access token** — `build_access_token(user, &family_id)` (below): the token is
   minted bound to the family stored in step 6.
8. **Respond** — `TokenResponse { access_token, refresh_token: Some(opaque), token_type:
   "Bearer", expires_in }`.

`ExchangeRequest` carries the client context (`ip_address`, `user_agent`, `device_id`)
extracted by the server's audit-context middleware; the stored session records all three,
and every audit event in the flow records the `ip_address` and `user_agent` (the
`AuditEvent` shape carries no `device_id`). A suspended user audits `UserSuspended` (warning,
failure); the registration-policy denials audit `RegistrationDenied` (warning, failure); a
created user audits `UserCreated` (notice, success); a successful exchange audits
`TokenExchange` (info, success) after the token response is assembled.

## Nonce issuance (`service/assertion.rs::mint_nonce`)

`POST /nonce`, served only when `grants.id_token = true`.

1. 32 random bytes, base64url-no-pad, is the returned nonce; its SHA-256 hex is the key.
2. `put_single_use("nonce:<hash>", now + grants.nonce_ttl)`. A `false` return is a 256-bit
   collision and is surfaced as `StoreError` rather than retried.
3. Respond `{ nonce, expires_in }`.

## Token refresh (`refresh.rs`)

`POST /token` with `grant_type=refresh_token`. Redemption is a state transition: the
family's live generation is retired and a replacement is issued in one atomic store
operation. A refresh token belongs to a **family** — every generation descended from one
sign-in shares a `family_id`, and a family has exactly one live generation at any instant.

1. SHA-256 hex the presented token.
2. `resolve_refresh_token(hash)` classifies the hash against the family's live generation
   and its retained retirement records:
   - **`Unknown`** — no live generation and no retained record → `InvalidToken` (audited
     `ValidationFailed`, `Debug`), as before.
   - **`Live(session)`** — the hash is the live generation → rotate (step 4).
   - **`Superseded { live, retired_at }`** — the hash is retired and its successor is still
     the family's live generation. Inside `token.refresh_rotation_grace` of `retired_at` →
     rotate from `live` (step 4). Outside it → reuse (step 3).
   - **`Retired { family_id, user_id, .. }`** — the hash is retired and its successor is no
     longer live → reuse (step 3).
3. **Reuse.** `revoke_family(family_id)`, then emit `RefreshTokenReuse` at
   `AuditSeverity::Warning` with `detail { family_id, sessions_revoked }`, then return
   `InvalidToken` carrying the same reason string as the unknown-token branch — the response
   does not tell the presenter that an alarm fired. Revocation runs before the emission so a
   blocking audit failure cannot leave the family alive.
4. `session.expires_at < now` → `InvalidToken`. The family's absolute expiry is fixed at
   exchange; rotation never moves it.
5. `get_user_by_id(session.user_id)`; missing → `InvalidToken`; suspended → `UserSuspended`.
   Both are decided before anything is written.
6. Mint the replacement — 32 random bytes, base64url-no-pad for the opaque token, SHA-256
   hex of the bytes for the new hash. The replacement `Session` inherits `family_id`,
   `user_id`, `provider`, `created_at`, `expires_at` and the device fields unchanged, sets
   `generation = live.generation + 1` and `rotated_at = now`.
7. `rotate_refresh_token(live_hash, replacement)` — one atomic compare-and-swap conditioned
   on `live_hash` still being the family's live generation. It deletes the live session,
   writes a `RetiredRefreshToken` for `live_hash` naming the replacement as its successor,
   and installs the replacement, or it writes nothing. A `false` return means a concurrent
   redemption won the race; the caller returns `InvalidToken` without revoking the family,
   and the loser's retry lands on the grace path.
8. `build_access_token(user, &family_id)` — the access token's `sid` claim carries the
   family identifier, which rotation never moves (see the *Build access token* section).
9. Audit `TokenRefresh` at `Info` with `detail { family_id, generation, grace }`. No token
   hash appears in an audit event.
10. Respond `TokenResponse { access_token, refresh_token: Some(replacement), token_type:
    "Bearer", expires_in }`.

**Grace.** A client that loses the response to a rotation still holds the generation the
server just retired. Returning the current token again is impossible here — the store keeps
only digests, so the service cannot reproduce a plaintext it has already discarded — so the
grace window instead lets that client rotate forward once more: presenting the
immediately-preceding generation inside `token.refresh_rotation_grace` rotates from the
current live generation and issues a fresh one. "Immediately preceding" and "once" are the
same condition, and it needs no extra state: a retirement record grants grace only while the
successor it names is still live, and a grace rotation retires that successor. Every later
presentation of the same generation is reuse.

**Rotation disabled.** With `token.refresh_rotation = false` the flow is steps 1–2, 4, 5,
8, 9 and a response with `refresh_token: None`. Nothing is minted and nothing is retired.
Retirement records left over from a rotation-enabled period still resolve until they expire;
while rotation is off, `Superseded` and `Retired` are treated as `Unknown` — refused as
`InvalidToken`, no alarm, no family revocation — because the switch disables the response
along with the rotation.

`RefreshRequest` carries the same client context as exchange; audit events in the flow
record its `ip_address` and `user_agent`. A suspended user audits `UserSuspended` (warning,
failure); unknown and expired tokens audit `ValidationFailed` (debug, failure) — below the
default `emit_threshold` of `info` — and `RefreshTokenReuse` (warning) is the signal that
survives the defaults.

`RefreshRequest` carries the same client context; audit events in the flow record its
`ip_address` and `user_agent`. A suspended user audits `UserSuspended`; a successful refresh
audits `TokenRefresh` (info, success). Unknown or expired tokens return `InvalidToken` and
audit `ValidationFailed` (debug, failure) — an abuse-detection signal that the default
`[audit] emit_threshold` of `info` suppresses; lowering the threshold to `debug` enables it.

## Revocation (`revoke.rs`)

`POST /revoke` (RFC 7009 — token-state failures still succeed toward the client; backend
failures propagate). Revocation authority comes from the credential the caller presents, and
reaches exactly the token family that credential names. `/revoke` never removes a session
the caller presented no credential for. Every resolved terminal path emits exactly one
mandatory, fixed-classification audit outcome, so even under `audit.durability = "enforce"`
an audit sink failure cannot make token existence observable.

- hint `refresh_token`, absent, or unknown → SHA-256 hex the token,
  `get_session_by_refresh_token(hash)`, and on a match `revoke_session(hash)` (audited
  `TokenRevocation`). A missing session is `Ok` toward the client (idempotent delete, 200)
  and records one fixed-classification `ValidationFailed` outcome; a store error propagates,
  and the server maps it to 503.
- hint `access_token` → `validate_access_token(token)` (below). The returned claims carry
  `sid`, the `family_id` of the session the token was minted for, which must additionally
  be a well-formed family identifier (`fam_` + lowercase ULID) — a validated token whose
  `sid` is not (including one minted before rotation shipped, carrying a 64-hex
  refresh-token hash) fails closed with the same fixed-reason rejection and revokes
  nothing; passing a hash-valued `sid` onward would "revoke" a family that does not exist
  and hide the miss. `revoke_family(claims.sid)` then removes the live generation and every
  retained retirement record of exactly that family (audited `TokenRevocation`, with
  `family_id` and the removed count in `detail`). One family is one sign-in, so the
  authority is unchanged: the credential revokes exactly the session it was minted for,
  under whichever generation that session has rotated to. The subject's other families are
  untouched, and `revoke_all_user_sessions` is not reachable from this endpoint.
- Any validation failure — malformed, wrong type, bad signature, expired, wrong issuer or
  audience, or an unusable `sid` — revokes nothing and emits one `ValidationFailed` event
  carrying a fixed reason string, then returns 200 like every other token-state outcome.
  The client cannot distinguish a rejected token from an accepted one (RFC 7009 §2.2); an
  operator can see the attempt. Both arms emit exactly one event at the same severity
  whenever they emit — success only when a family or session matched, rejection always —
  which is what keeps them indistinguishable under the current blocking-threshold
  durability model as well as in normal operation. A session-repo error from
  `revoke_family` propagates, and the server maps it to 503.

`RevokeRequest` carries the same client context; audit events in the flow record its
`ip_address` and `user_agent`.

## Build access token (`service/mod.rs::build_access_token`)

1. Parse `token.access_token_ttl` to seconds (`parse_duration_secs`).
2. Assemble `AccessTokenClaims { sub: user.id, iss: server.issuer, aud: token.audience,
   iat, exp, sid, custom }`, where `sid` is the `family_id` of the session this token is
   minted for — supplied by the caller, from the family `exchange` has just created or the
   one `refresh` has just rotated — and `custom` comes from `resolve_custom_claims`.
   `family_id` is stable across every rotation, so a `sid` minted at exchange names its
   session for the token's whole validity however often the refresh token rotates beneath
   it. `iss` and `aud` are required non-empty configuration values.
3. Header `{ alg: keys.algorithm(), typ: "at+jwt", kid: keys.key_id() }` — the RFC 9068 media
   type for a JWT access token, which `validate_access_token` requires.
4. base64url(header).base64url(payload), `keys.sign` the signing input, append
   base64url(signature). Return `(jwt, ttl_secs)`.

## Validate access token (`service/mod.rs::validate_access_token`)

The only path by which a claim of a service-minted JWT becomes readable. It returns
`AccessTokenClaims` or a fixed rejection reason — used solely as the audit `reason`; it never
reaches the client — and a caller cannot reach `sub` without having proved everything below.

1. Split on `.` — exactly three non-empty segments, each base64url-no-pad decodable.
2. Header: `alg == keys.algorithm()`, `kid == keys.key_id()`, `typ == "at+jwt"`. The header is
   covered by the signature but is not self-authenticating, so it is pinned to what this
   service mints rather than read for direction; the three members are required fields of the
   typed header struct, so a header missing any of them fails to parse.
3. `keys.verify(signing_input, signature)` over `header.payload` exactly as received. No
   claim is read before this step succeeds.
4. Deserialize the payload into `AccessTokenClaims`. `sub`, `iss`, `aud`, `iat`, `exp` and
   `sid` are required fields, so a missing claim is a parse failure rather than a check that
   can be omitted.
5. `iss == server.issuer`; `aud == token.audience` (the empty string when unset — the same
   value `build_access_token` stamps, so the two agree by construction).
6. Validity window against one captured `Utc::now()` with 60 seconds of clock skew
   (`CLOCK_SKEW_SECS`) on every comparison, saturating arithmetic throughout: expired when
   `now > exp + skew` — expiry exactly at the skew edge is still inside the window;
   future-dated when `iat > now + skew`; not-yet-valid when an optional `nbf > now + skew`.
   The service never mints `nbf` and it is deliberately not a field of `AccessTokenClaims`;
   it is parsed separately only after the typed required claims succeed, and a non-numeric
   `nbf` is rejected.
7. `sub` and `sid` are non-empty after trimming.

## Custom claims (`claims.rs`)

Two sources merge into `AccessTokenClaims.custom`:

1. **Config templates** — `token.custom_claims` (a `HashMap<String,String>`).
2. **Per-user claims** — `user.claims`, applied on top (per-user overrides config on key
   collision).

Reserved names `sub`, `iss`, `aud`, `iat`, `exp`, `nbf` and `sid` are silently dropped from
both sources. `sid` carries revocation authority and `nbf` bounds validity, so neither may be
set from a config template or a per-user claim.

Template language (config values only):

- Static string — `org = "example"` → used verbatim.
- Field reference — `"{{ user.email }}"` → dot-path lookup.
- Default filter — `"{{ user.metadata.role | default: 'user' }}"` → fallback when the field
  is missing/null. A reference with no value and no default is omitted from the token.

Resolvable paths: `user.id`, `user.email`, `user.display_name`, `user.provider`,
`user.external_id`, `user.metadata.<key>`, `user.claims.<key>`. No loops or conditionals.

## Audit emission (`service/mod.rs`)

Two channels have different guarantees:

```
emit_security_event(SecurityEvent)          — mandatory
   render to AuditEvent (severity derived from the variant)
   audit.emit(event)
      Err → tracing fallback with audit_fallback = true
            durability = "enforce" → fail the operation
            durability = "observe" → log audit_durability_degraded = true, continue
   No threshold is consulted.

emit_audit(AuditEvent)                      — best-effort
   severity less severe than emit_threshold → drop before dispatch
   audit.emit(event)
      Err → tracing fallback; blocking_threshold decides as before
```

Severity follows RFC 5424 (emergency 0 … debug 7); lower is more severe. Every shipped flow
uses the mandatory channel. The HTTP public per-IP throttle also emits `ThrottleExceeded`
through this same API before returning its terminal `429`; the middleware
logs an enforce-mode emission error but preserves the `429`, so audit-sink behavior cannot make
the denial unsafe. `emit_audit` remains available for operational events supplied by embedders,
and only that best-effort channel is governed by `emit_threshold` and `blocking_threshold`.

## Admin operations (`user_admin.rs`)

All under `/internal/*`. User-sync notifications are **best-effort** — failures are logged
via `tracing` and never fail the admin call. Admin mutations use the mandatory audit channel,
so an audit write failure follows `audit.durability`; their events have
`ip_address_source = "unknown"`.

| Method | Behaviour |
|---|---|
| `admin_create_user` | `create_user`, then `notify_user_created` |
| `admin_get_user` | `get_user_by_id` |
| `admin_update_user` | load the user (missing → `NotFound`), validate any status change against the [user lifecycle](01-domain-model.md) (`Deleted` is strictly terminal — any status patch on a deleted user, including `Deleted → Deleted`, is rejected; a patch to the current status is otherwise an accepted no-op; invalid transition → `InvalidRequest`), `update_user`, revoke all sessions when the patch changed the status to `Suspended` or `Deleted`, diff changed fields, `notify_user_updated` |
| `admin_delete_user` | patch `status=Deleted` via the same validated path (valid from `Active` or `Suspended`), `revoke_all_user_sessions`, `notify_user_deleted`; unknown id → `NotFound` |
| `admin_get_claims` | return `user.claims` (missing user → `NotFound`; `admin_set_claims` / `admin_merge_claims` / `admin_clear_claims` return the same on an unknown id) |
| `admin_set_claims` | replace the whole claims map |
| `admin_merge_claims` | merge new keys over existing (new wins) |
| `admin_clear_claims` | set claims to empty |
| `admin_stats` | `count_by_status` + `count_active_sessions` → `AdminStats` |
| `admin_list_users` | `list_users(offset, limit)` |

Admin mutations are audited: `admin_create_user` → `UserCreated`, `admin_update_user` →
`UserUpdated` (and `UserSuspended` when the patch sets `status = Suspended`),
`admin_delete_user` → `UserDeleted`, and the claims mutations → `UserUpdated` with the
operation in `detail`. Read-only operations (get, list, stats, get-claims) are not audited.
Audit failures follow `emit_audit`'s blocking rules, unlike best-effort user sync. Admin
operations carry no client `ip_address`/`user_agent` context.

## Assumptions and open questions

### Assumptions

- The provider has already verified the ID token's signature and issuer in
  `validate_id_token`; the service trusts the returned `IdentityClaims`.
- `parse_duration_secs` accepts an integer followed by `s`/`m`/`h`/`d`.
- Access-token revocation records rejections as fixed-reason `ValidationFailed` events on
  the current audit surface. Two sibling proposals are external dependencies that merge
  separately and are deliberately not absorbed here: the
  `2026-08-05-audit_and_throttle_authentication_failures` change (dedicated
  `AuthenticationFailed` security-event type, mandatory security-event channel, durability
  config, per-IP throttling) and the
  `2026-08-05-rotate_refresh_tokens_with_reuse_detection` change (supersedes the
  hash-valued `sid` with a rotation-independent `family_id`). This page keeps their
  ordering/supersession caveats with the decisions that depend on them.

### Decisions

- *The declared grant is the flow selector.* **`ExchangeRequest` carries an
  `ExchangeCredential` enum parsed at the HTTP boundary; the service matches on it and never
  inspects field presence.** An incoherent grant/field combination fails to parse at the edge
  instead of choosing a branch, so a later refactor cannot re-flatten the decision without
  deleting the type.
- *Binding lives in the core, not the providers.* **`nonce`, `azp`, `at_hash` and
  single-use are enforced once in `AppService::exchange`, reading `IdentityClaims.raw_claims`.**
  The same four controls were omitted twice in two independent validators; a control that
  every provider must inherit belongs above the provider boundary, not inside it. A Tier 3
  provider sharing no code with either OIDC validator is covered by construction.
- *Burn the nonce before claiming the assertion marker.* **The nonce is consumed first; the
  single-use marker is claimed second.** The reverse order lets an attacker holding a
  victim's assertion but no valid nonce pin the marker and deny the legitimate client its
  own first use. In this order a partial failure costs the honest client one `POST /nonce`
  round trip and never admits a replay.
- *A lifetime ceiling instead of a capped marker TTL.* **An assertion whose remaining
  lifetime exceeds `grants.max_assertion_lifetime` (default 1h) is refused.** Capping the
  marker's TTL instead would leave the assertion replayable after the cap. Real ID tokens
  live 5–60 minutes, so the ceiling rejects nothing legitimate and bounds the state a
  single assertion can pin.
- *Refresh rotates.* **Each redemption issues a replacement refresh token and retires the
  presented generation in one atomic store operation.** A long-lived credential that is
  never consumed makes possession indistinguishable from entitlement for its whole TTL;
  rotation bounds a stolen token to one use and, more importantly, makes a second holder
  visible. `token.refresh_rotation = false` restores reusable tokens.
- *Rotation does not slide the expiry.* **The replacement inherits the family's original
  `expires_at` and `created_at`.** Recomputing the expiry on every rotation would convert a
  bounded 30-day session into an unbounded one that never dies while it is used, removing
  the only bound that currently ends a stolen token's life.
- *Reuse revokes the family, not the user.* **A retired generation presented outside its
  grace window revokes every generation of that one login chain.** The evidence is that one
  credential chain leaked; logging the user out of every other device is disproportionate to
  it.
- *The reuse alarm is emitted at `Warning`.* **`RefreshTokenReuse` carries
  `AuditSeverity::Warning`.** The shipped audit defaults are `emit_threshold = "info"` and
  `blocking_threshold = "warning"`, so `Warning` is the least severe level that both
  survives a default deployment and fails the request rather than being silently dropped
  when the audit backend is down.
- *`sid` is the session's family identifier.* **The access token carries `family_id` as its
  `sid` claim, and `/revoke`'s access-token arm resolves it with `revoke_family`.** The hash
  was the right identifier while refresh never rotated, and rotation is exactly the change
  that orphans it — after one refresh every outstanding access token would name a retired
  hash and access-token revocation would silently become a no-op. A rotation-independent
  identifier keeps the `sid` resolvable for the token's full TTL.
- *Registration demands a verified email.* **Every just-in-time user creation requires
  `email_verified == true`, whether or not an allowlist is configured.** The requirement is a
  property of accepting the email claim, not of the allowlist; nesting it inside an optional
  feature's branch meant turning the allowlist off turned identity verification off with it.
- *The allowlist is an authorization predicate; the mode is an admission gate.* **The domain
  allowlist is re-evaluated on every exchange, for existing users as well as new ones;
  `registration.mode` applies only at creation.** An operator who tightens the allowlist is
  trying to contain accounts that already exist. Re-evaluating the mode for an existing user is
  not coherent without recording provisioning provenance; the refresh path likewise has no
  fresh claims to evaluate the allowlist against. For both, containment remains suspending or
  deleting the user — honored immediately on every path — and the refresh-side residual window
  is bounded by `token.refresh_token_ttl`.
- *Best-effort user sync.* **Sync notifications never fail an admin or exchange operation.**
  Sync is a downstream convenience, not a correctness dependency.
- *Audit durability is unconditional on the mandatory channel.* **A mandatory-channel write
  failure follows `audit.durability`, independent of severity.** A tracing fallback line is
  still written first, but it is not a substitute for the configured durable trail.
- *One validator for first-party tokens.* **Every read of a claim from a JWT this service
  minted goes through `AppService::validate_access_token`.** `exchange` delegates JWT
  validation to the provider adapters and `refresh` validates an opaque token against the
  session store, so neither validates a first-party JWT; revoke's former hand-rolled check
  was the only one in the workspace, and hand-rolling is what made stopping after the
  signature possible.
- *Required claims are parse-enforced.* **`sub`, `iss`, `aud`, `iat`, `exp` and `sid` are
  required fields of `AccessTokenClaims`, so presence is a deserialization outcome, not a
  check.** The same discipline `set_required_spec_claims` gives the provider paths, in a
  crate that carries no `jsonwebtoken` dependency.
- *A credential revokes only its own session.* **The access-token branch of `/revoke`
  revokes the single session named by `sid`.** A stateless access token is not a session
  credential; treating it as authority over every session of its subject gave any holder of
  any leaked token an account-wide logout. Account-wide revocation remains on the
  authenticated admin path — `apply_validated_patch` revokes every session when a status
  patch moves a user into `Suspended` or `Deleted`, on behalf of both `admin_update_user`
  and `admin_delete_user`.
- *Failed revocation is recorded, not silent.* **A rejected `/revoke` emits one
  `ValidationFailed` event and still returns 200.** RFC 7009 §2.2 constrains what the caller
  observes, not what the operator records — and an unauthenticated endpoint that answers 200
  regardless is precisely the one whose abuse is invisible without a record. The rejection
  event carries the same severity as the success-path `TokenRevocation` emission, so success
  and failure have identical durability semantics under the current blocking-threshold
  config: emitting only on success would answer 503 for a token that existed and 200 for one
  that did not whenever the sink is down — reintroducing, as degraded-mode behaviour, the
  existence oracle the silence was meant to prevent. Renaming the event to a dedicated
  `AuthenticationFailed` type on a mandatory security-event channel, with per-IP throttling,
  belongs to the external `2026-08-05-audit_and_throttle_authentication_failures` proposal
  and is not absorbed here.

### Open questions

- None.
