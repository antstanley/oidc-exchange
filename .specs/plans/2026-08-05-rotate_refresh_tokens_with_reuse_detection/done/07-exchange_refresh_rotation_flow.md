# Task 07 — Exchange and refresh rotation flow

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §Token refresh, exchange family issuance, audit behaviour, rotation-disabled compatibility, and core refresh tests.
**Depends on:** 01 · domain_config_port_contract; 03 · sql_session_adapters; 04 · lmdb_session_adapter; 05 · valkey_session_adapter; 06 · dynamodb_session_adapter
**Produces:** policy-owning core refresh flow that atomically rotates through the port, detects/revokes reuse, preserves absolute expiry, and returns replacement tokens.
**Pointers:** `crates/core/src/service/{exchange,refresh,mod}.rs`; `crates/core/tests/{exchange,refresh}.rs`.

## Steps

- [x] At exchange, mint `fam_` lowercase-ULID family identifiers and generation-0 sessions with fixed family creation/expiry metadata.
- [x] Rewrite refresh classification: unknown/expired/missing user/a suspended user retain specified audit/error behaviour; grace is evaluated once in core; normal and grace rotations mint a replacement and use CAS.
- [x] On reuse, revoke family before emitting `RefreshTokenReuse` at Warning with only `{ family_id, sessions_revoked }`; return the exact unknown-token reason to the presenter.
- [x] Preserve explicit disabled behaviour: no mint/retire/alarm; retired classifications are refused as unknown; response has no refresh token.
- [x] Add deterministic tests for normal replacement, old-token behavior, one grace re-rotation, later/outside-grace reuse, fixed expiry, concurrent loser behavior, disabled compatibility, audit detail secrecy/severity, and suspended-before-write ordering.

## Definition of done

- [x] A successful enabled refresh returns a new opaque token and no longer leaves the presented generation live.
- [x] Reuse removes only the affected family before blocking audit emission; audit detail contains no token digest.
- [x] Replacement inherits `created_at`, device data, provider, user, and absolute `expires_at`; generation advances exactly once per successful CAS.
- [x] Tests cover each `RefreshResolution` branch and positive/negative grace boundary with an injected/deterministic clock.
- [x] Done certificates remain intentionally absent.

## Completion notes

- `crates/core/src/service/refresh.rs` rewritten as a classification-driven flow. Policy lives entirely in core: `Unknown` refuses exactly as before (`ValidationFailed` at Debug, then the `"unknown refresh token"` reason); `Live` rotates through `rotate_refresh_token`; `Superseded` inside the configured grace window (parsed once per request via `rotation_grace_secs`, compared with a jitter-tolerant `within_grace`) rotates forward once from the current live generation; `Superseded` outside grace and `Retired` both take the reuse branch. Ordering matches the source spec — resolve → reuse → expiry → user status → mint → swap → sign → audit → respond — so expired generations, missing users, and suspended users are turned away before any write.
- Reuse (`revoke_family_for_reuse`) revokes **before** emitting so a blocking-audit failure cannot leave the family alive, emits `RefreshTokenReuse` at Warning with detail `{family_id, sessions_revoked}` only, and returns an `InvalidToken` whose reason string is the same named constant (`UNKNOWN_REFRESH_TOKEN_REASON`) the unknown branch uses — asserted at write time so indistinguishability cannot drift. A losing CAS refuses generically without revoking or alarming.
- `mint_replacement` inherits user/provider/device context/`created_at`/absolute `expires_at`, advances generation by one, sets `rotated_at`, and mints the family for pre-rotation legacy rows (empty-string sentinel) on first redemption — no retirement record is written for that transition, matching the uniform adapter semantics from tasks 03–06.
- Rotation-disabled mode (`refresh_without_rotation`) keeps legacy reusable-token behaviour: nothing minted or retired, response carries no refresh token, leftover `Superseded`/`Retired` classifications refuse silently as unknown.
- Deterministic grace-boundary coverage uses a new test-only `MockRepository::backdate_retirement` hook instead of sleeps; the concurrent-loser branch uses a test-local `LosingCasRepo` wrapper whose CAS always loses, asserting byte-identical store state.
- Server e2e full-flow updated to revoke the *current* generation (rotation retires the presented one), preserving the test's intent.
- Gates at commit: nextest 451 passed / 50 skipped (+12 vs the 439 baseline); `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
