# Service Flows

**Status:** Implemented · **Date:** 2026-08-17 · **Owner:** Ant Stanley · **Scope:** crates/core/src/service

`AppService` orchestrates the ports. It holds `user_repo`, `session_repo`, `keys`, `audit`,
`user_sync`, the configured retained `rate_limiter`, a `providers` map, and `config`. The
retained limiter enforces provider and subject budgets across service-flow calls; the server
retains a configured public limiter in `AppState` for per-IP HTTP throttling. The flows below
live in `crates/core/src/service/{exchange,refresh,revoke,user_admin,claims}.rs` and the helpers
in `service/mod.rs`.

## Token exchange (`exchange.rs`)

`POST /token` with `grant_type=authorization_code` or `grant_type=id_token`. Emission is
terminal and single: the flow maps its result to exactly one `SecurityEvent`, with fixed
classification strings rather than upstream error text. Success is emitted after storing the
session and signing the access token; under `audit.durability = "enforce"`, a failed terminal
emit revokes that just-stored session before returning the error. Principal creation is a
separate state-change event, so a losing JIT-registration racer emits none.

The flow consumes a per-provider unit before outbound exchange or validation and a
per-subject unit after validation establishes the subject. Either denial emits
`ThrottleExceeded` and returns `TooManyRequests`; the public HTTP layer has already applied
the per-IP check.

## Token refresh (`refresh.rs`)

`POST /token` with `grant_type=refresh_token`. The flow hashes the presented token, resolves
the session and user, rejects missing/expired/suspended states, signs a new access token, and
returns no new refresh token. It uses the same single terminal emission: success emits
`AuthenticationSucceeded { kind: Refresh }`; missing/expired session or user emits
`AuthenticationFailed`; a suspended user emits `PrincipalSuspended`. A per-subject unit is
consumed after session lookup; pre-lookup guessing is bounded by the public per-IP budget.

## Revocation (`revoke.rs`)

`POST /revoke` follows RFC 7009: token-state failures remain client-visible `200`, while
backend failures propagate. The access-token path emits `SessionsRevoked` after successful
signature verification; the refresh-token path emits `SessionRevoked` when a session matched.
Rejected tokens emit `AuthenticationFailed` with a fixed classification. Every branch emits
exactly one event so, even under `durability = "enforce"`, audit sink failure cannot make token
existence observable. The later revoke-claim-validation change may narrow the access-token
revocation scope, but does not change this rejected-token symmetry.

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
- *Audit durability is unconditional.* **A mandatory-channel write failure follows
  `audit.durability`, independent of severity.** A tracing fallback line is still written
  first, but it is not a substitute for the configured durable trail.

### Open questions

- Suspended-user exchange is rejected, but whether an audit `Unauthorized` vs `UserSuspended`
  event type is emitted in every rejection branch is worth confirming against the handlers.
