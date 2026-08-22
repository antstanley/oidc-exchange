# Service Flows

**Status:** Implemented · **Date:** 2026-08-22 · **Owner:** Ant Stanley · **Scope:** crates/core/src/service

`AppService` orchestrates the ports. It holds `user_repo`, `session_repo`, `keys`, `audit`,
`user_sync`, a `providers` map, and `config`. The flows below live in
`crates/core/src/service/{exchange,refresh,revoke,user_admin,claims,maintenance}.rs` and the
helpers in `service/mod.rs`.

## Token exchange (`exchange.rs`)

`POST /token` with `grant_type=authorization_code` or `grant_type=id_token`.

1. **Resolve provider** — look up `request.provider` in the `providers` map; missing →
   `UnknownProvider`.
2. **Obtain verified claims**
   - If an `id_token` was supplied directly → `provider.validate_id_token(id_token)`.
   - Otherwise require `code` and `redirect_uri` (else `InvalidRequest`),
     `provider.exchange_code` to get `ProviderTokens`, then `validate_id_token` on the
     returned `id_token`.
3. **User lookup / registration policy** — `get_user_by_external_id(subject, provider)`:
   - **Found, suspended** → `UserSuspended` (audited `Unauthorized`/`UserSuspended`).
   - **Found, active** → proceed (existing users bypass registration policy).
   - **Not found** → apply policy:
     - If `registration.domain_allowlist` is set: the ID token must carry a **verified**
       email (`email_verified == Some(true)`) whose domain matches the allowlist — exact
       (`example.com`) or wildcard (`*.example.com`, at least one subdomain, case-insensitive).
       A missing/unverified email or non-matching domain → `AccessDenied`
       (audited `RegistrationDenied`).
     - If `registration.mode == "existing_users_only"` → `AccessDenied` (`RegistrationDenied`).
     - Otherwise (`mode == "open"`) → `create_user(NewUser{…})` (audited `UserCreated`);
       if creation returns `Conflict` (a concurrent first login won the race), re-run
       `get_user_by_external_id` and continue with the existing user, re-applying the
       suspended-status check. The losing racer emits no `UserCreated` event — the winning
       create already audited it — and the flow otherwise proceeds as for a found user.
4. **Mint refresh token** — 32 random bytes, base64url-no-pad for the opaque token; SHA-256
   hex of the bytes is the stored hash.
5. **Store session** — mint the family id (`fam_` + lowercase ULID), set `generation = 0`
   and `rotated_at = None`, `expires_at = now + refresh_token_ttl`; `store_refresh_token`.
6. **Sign access token** — `build_access_token(user, &family_id)` (below).
7. **Respond** — `TokenResponse { access_token, refresh_token: Some(opaque), token_type:
   "Bearer", expires_in }`.

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

## Revocation (`revoke.rs`)

`POST /revoke` (RFC 7009 — token-state failures still succeed toward the client; backend
failures propagate).

- `token_type_hint == "access_token"` → verify the JWT (`keys.verify` over the base64url
  signing input), decode its payload into `AccessTokenClaims`, and require `sub` non-empty
  and `sid` to be a well-formed family identifier (`fam_` + lowercase ULID). A token that
  fails verification, or whose `sid` is not — including one minted before rotation shipped,
  carrying a 64-hex refresh-token hash — fails closed with a fixed reason and emits a single
  `ValidationFailed` event (debug): nothing is revoked, no family is invented. Passing a
  hash-valued `sid` onward would "revoke" a family that does not exist and hide the miss.
- A usable access token names its family: `revoke_family(claims.sid)` removes the live
  generation and every retained retirement record of exactly that family (audited
  `TokenRevocation`, with the removed count in `detail`). One family is one sign-in, so the
  authority is unchanged: the credential revokes exactly the session it was minted for,
  under whichever generation that session has rotated to. The subject's other families are
  untouched, and `revoke_all_user_sessions` remains unreachable from this endpoint. A
  token-verification failure is swallowed toward the client and still returns 200 — RFC 7009
  §2.2 forbids leaking whether a token existed — but a session-repo error from
  `revoke_family` propagates, and the server maps it to 503.
- hint `refresh_token`, absent, or unknown → SHA-256 hex the token and
  `revoke_session(hash)`. A missing session is `Ok` (idempotent delete, 200); a store
  error propagates, and the server maps it to 503.

## Build access token (`service/mod.rs::build_access_token`)

1. Parse `token.access_token_ttl` to seconds (`parse_duration_secs`).
2. Assemble `AccessTokenClaims { sub: user.id, iss: server.issuer, aud: token.audience or "",
   sid, iat, exp, custom }` where `custom` comes from `resolve_custom_claims`. `sid` is the
   `family_id` of the session this token is minted for — supplied by the caller, from the
   family `exchange` has just created or the one `refresh` has just rotated. `family_id` is
   stable across every rotation, so a `sid` minted at exchange names its session for the
   token's whole validity however often the refresh token rotates beneath it.
3. Header `{ alg: keys.algorithm(), typ: "JWT", kid: keys.key_id() }`.
4. base64url(header).base64url(payload), `keys.sign` the signing input, append
   base64url(signature). Return `(jwt, ttl_secs)`.

## Custom claims (`claims.rs`)

Two sources merge into `AccessTokenClaims.custom`:

1. **Config templates** — `token.custom_claims` (a `HashMap<String,String>`).
2. **Per-user claims** — `user.claims`, applied on top (per-user overrides config on key
   collision).

Reserved names `sub`, `iss`, `aud`, `iat`, `exp` are silently dropped from both sources.

Template language (config values only):

- Static string — `org = "example"` → used verbatim.
- Field reference — `"{{ user.email }}"` → dot-path lookup.
- Default filter — `"{{ user.metadata.role | default: 'user' }}"` → fallback when the field
  is missing/null. A reference with no value and no default is omitted from the token.

Resolvable paths: `user.id`, `user.email`, `user.display_name`, `user.provider`,
`user.external_id`, `user.metadata.<key>`, `user.claims.<key>`. No loops or conditionals.

## Audit emission and blocking (`service/mod.rs::emit_audit`)

```
audit.emit(event)
   Ok  → done
   Err → serialize event to a tracing log (fallback record)
         if severity ≤ audit.blocking_threshold → propagate Err (fail the operation)
         else → tracing::warn! "audit provider down", return Ok
```

Severity follows RFC 5424 (emergency 0 … debug 7); lower number = more severe.
`blocking_threshold` is a severity name parsed by `parse_severity`. So with the default
`warning` threshold, an audit-backend failure on a warning-or-more-severe event fails the
request, while a notice/info event is logged and the request continues.

## Admin operations (`user_admin.rs`)

All under `/internal/*`. User-sync notifications are **best-effort** — failures are logged
via `tracing` and never fail the admin call.

| Method | Behaviour |
|---|---|
| `admin_create_user` | `create_user`, then `notify_user_created` |
| `admin_get_user` | `get_user_by_id` |
| `admin_update_user` | `update_user`, diff changed fields, `notify_user_updated` |
| `admin_delete_user` | patch `status=Deleted`, `revoke_all_user_sessions`, `notify_user_deleted` |
| `admin_get_claims` | return `user.claims` (missing user → `InvalidRequest`) |
| `admin_set_claims` | replace the whole claims map |
| `admin_merge_claims` | merge new keys over existing (new wins) |
| `admin_clear_claims` | set claims to empty |
| `admin_stats` | `count_by_status` + `count_active_sessions` → `AdminStats` |
| `admin_list_users` | `list_users(offset, limit)` |

## Assumptions and open questions

### Assumptions

- The provider has already verified the ID token's signature and issuer in
  `validate_id_token`; the service trusts the returned `IdentityClaims`.
- `parse_duration_secs` accepts an integer followed by `s`/`m`/`h`/`d`.

### Decisions

- *Refresh rotates.* **Each redemption issues a replacement refresh token and retires the
  presented generation in one atomic store operation.** A long-lived credential that is
  never consumed makes possession indistinguishable from entitlement for its whole TTL;
  rotation bounds a stolen token to one use and, more importantly, makes a second holder
  visible.
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
- *Domain allowlist demands a verified email.* **New-user registration under an allowlist
  requires `email_verified == true`.** Prevents allowlist bypass via an unverified address.
- *Existing users bypass policy.* **Registration policy applies only when no user exists.**
  Tightening the allowlist later does not lock out already-registered users.
- *Best-effort user sync.* **Sync notifications never fail an admin or exchange operation.**
  Sync is a downstream convenience, not a correctness dependency.
- *Audit fallback always records.* **On backend failure the event is still written to a
  tracing log before the blocking decision.** No audited event is silently lost.

### Open questions

- Suspended-user exchange is rejected, but whether an audit `Unauthorized` vs `UserSuspended`
  event type is emitted in every rejection branch is worth confirming against the handlers.
