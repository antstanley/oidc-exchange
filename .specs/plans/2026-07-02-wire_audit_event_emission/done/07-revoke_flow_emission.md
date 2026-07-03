# Task 07 — revoke flow emission

**Plan:** [plan.md](../plan.md) · **Certificate:** [07-revoke_flow_emission-certificate.md](07-revoke_flow_emission-certificate.md)

**Implements:** [03-service-flows.md](../../../service/specs/03-service-flows.md) §Revocation (the revocation audit events and the RFC 7009 silence on failed verification)
**Depends on:** 01, 03, 04
**Produces:** the access-token path emits `AllSessionsRevoked` on successful signature verification; the refresh-token path emits `TokenRevocation` when a session was actually revoked; failed verification and unknown tokens emit nothing.
**Pointers:** `crates/core/src/service/revoke.rs:19-21` (access-token path, after `verify_and_extract_sub` succeeds and sessions are revoked) → `AllSessionsRevoked`; `revoke.rs:28`/`:34` (refresh-token path, when `revoke_session` reports a session removed) → `TokenRevocation`; `crates/core/src/service/mod.rs:102`/`:132`

## Steps

- [x] On the access-token path, emit `AllSessionsRevoked` only when `verify_and_extract_sub` returns a user id and the session revocation ran; emit nothing when verification fails (preserve the RFC 7009 silence and the always-`Ok(())` return).
- [x] On the refresh-token path (`:28` and the unknown-hint fallback `:34`), emit `TokenRevocation` only when the session store reports a session was actually revoked; emit nothing for an unknown token.
- [x] Build events via `create_audit_event` with `request.ip_address`/`request.user_agent`; keep the handler's RFC 7009 contract (return `Ok(())` regardless), applying `emit_audit`'s blocking semantics to the emission result.
- [x] Add tests: a valid access-token revoke emits `AllSessionsRevoked`; a valid refresh-token revoke emits `TokenRevocation`; a failed-verification access token and an unknown refresh token emit nothing.

## Definition of done

- [x] The access-token path emits `AllSessionsRevoked` on verified revocation; the refresh-token path emits `TokenRevocation` on an actual session removal; both record the request's ip/ua.
- [x] Failed signature verification and unknown tokens emit no audit event and still return success (RFC 7009 silence preserved).
- [x] Negative-space test: an access token with an invalid signature and a refresh token with no matching session each record nothing on `MockAuditLog`.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core revoke` and observe the emit-on-success and silence-on-failure behaviour via `MockAuditLog`.
