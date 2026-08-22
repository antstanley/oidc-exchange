# Service Flows

**Status:** Implemented · **Date:** 2026-08-15 · **Owner:** Ant Stanley · **Scope:** crates/core/src/service

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
5. **Mint refresh token** — 32 random bytes, base64url-no-pad for the opaque token; SHA-256
   hex of the bytes is the stored hash.
6. **Store session** — `expires_at = now + refresh_token_ttl`; `store_refresh_token`.
7. **Sign access token** — `build_access_token(user)` (below).
8. **Respond** — `TokenResponse { access_token, refresh_token: Some(opaque), token_type:
   "Bearer", expires_in }`.

## Nonce issuance (`service/assertion.rs::mint_nonce`)

`POST /nonce`, served only when `grants.id_token = true`.

1. 32 random bytes, base64url-no-pad, is the returned nonce; its SHA-256 hex is the key.
2. `put_single_use("nonce:<hash>", now + grants.nonce_ttl)`. A `false` return is a 256-bit
   collision and is surfaced as `StoreError` rather than retried.
3. Respond `{ nonce, expires_in }`.

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
