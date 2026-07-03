# Service Flows

**Status:** Implemented · **Date:** 2026-07-02 · **Owner:** Ant Stanley · **Scope:** crates/core/src/service

`AppService` orchestrates the ports. It holds `user_repo`, `session_repo`, `keys`, `audit`,
`user_sync`, a `providers` map, and `config`. The flows below live in
`crates/core/src/service/{exchange,refresh,revoke,user_admin,claims}.rs` and the helpers in
`service/mod.rs`.

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
5. **Store session** — `expires_at = now + refresh_token_ttl`; `store_refresh_token`.
6. **Sign access token** — `build_access_token(user)` (below).
7. **Respond** — `TokenResponse { access_token, refresh_token: Some(opaque), token_type:
   "Bearer", expires_in }`.

## Token refresh (`refresh.rs`)

`POST /token` with `grant_type=refresh_token`.

1. SHA-256 hex the presented token.
2. `get_session_by_refresh_token(hash)`; missing → `InvalidToken`.
3. `session.expires_at < now` → `InvalidToken`.
4. `get_user_by_id(session.user_id)`; missing → `InvalidToken`; suspended → `UserSuspended`.
5. `build_access_token(user)`.
6. Respond with `refresh_token: None` — refresh tokens are reusable until expiry; refreshing
   does not rotate them.

## Revocation (`revoke.rs`)

`POST /revoke` (RFC 7009 — token-state failures still succeed toward the client; backend
failures propagate).

- `token_type_hint == "access_token"` → `verify_and_extract_sub(token)`: split the JWT,
  base64url the signature, `keys.verify(signing_input, signature)`, and on success decode
  the payload and read `sub`; then `revoke_all_user_sessions(sub)`. A token-verification
  failure (malformed, unsigned, expired, or unknown token) is swallowed and still returns
  200 — individual access JWTs cannot be revoked and RFC 7009 §2.2 forbids leaking whether
  a token existed — but a session-repo error from `revoke_all_user_sessions` propagates,
  and the server maps it to 503.
- hint `refresh_token`, absent, or unknown → SHA-256 hex the token and
  `revoke_session(hash)`. A missing session is `Ok` (idempotent delete, 200); a store
  error propagates, and the server maps it to 503.

## Build access token (`service/mod.rs::build_access_token`)

1. Parse `token.access_token_ttl` to seconds (`parse_duration_secs`).
2. Assemble `AccessTokenClaims { sub: user.id, iss: server.issuer, aud: token.audience or "",
   iat, exp, custom }` where `custom` comes from `resolve_custom_claims`.
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

- *Refresh does not rotate.* **A successful refresh returns no new refresh token.** Reusable
  refresh tokens match common client libraries; rotation is not implemented.
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
