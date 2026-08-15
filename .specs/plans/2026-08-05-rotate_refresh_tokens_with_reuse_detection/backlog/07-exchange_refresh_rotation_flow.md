# Task 07 — Exchange and refresh rotation flow

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Token refresh, exchange family issuance, audit behaviour, rotation-disabled compatibility, and core refresh tests.
**Depends on:** 01 · domain_config_port_contract; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter
**Produces:** policy-owning core refresh flow that atomically rotates through the port, detects/revokes reuse, preserves absolute expiry, and returns replacement tokens.
**Pointers:** `crates/core/src/service/{exchange,refresh,mod}.rs`; `crates/core/tests/{exchange,refresh}.rs`.

## Steps

- [ ] At exchange, mint `fam_` lowercase-ULID family identifiers and generation-0 sessions with fixed family creation/expiry metadata.
- [ ] Rewrite refresh classification: unknown/expired/missing user/a suspended user retain specified audit/error behaviour; grace is evaluated once in core; normal and grace rotations mint a replacement and use CAS.
- [ ] On reuse, revoke family before emitting `RefreshTokenReuse` at Warning with only `{ family_id, sessions_revoked }`; return the exact unknown-token reason to the presenter.
- [ ] Preserve explicit disabled behaviour: no mint/retire/alarm; retired classifications are refused as unknown; response has no refresh token.
- [ ] Add deterministic tests for normal replacement, old-token behavior, one grace re-rotation, later/outside-grace reuse, fixed expiry, concurrent loser behavior, disabled compatibility, audit detail secrecy/severity, and suspended-before-write ordering.

## Definition of done

- [ ] A successful enabled refresh returns a new opaque token and no longer leaves the presented generation live.
- [ ] Reuse removes only the affected family before blocking audit emission; audit detail contains no token digest.
- [ ] Replacement inherits `created_at`, device data, provider, user, and absolute `expires_at`; generation advances exactly once per successful CAS.
- [ ] Tests cover each `RefreshResolution` branch and positive/negative grace boundary with an injected/deterministic clock.
- [ ] Done certificates remain intentionally absent.
