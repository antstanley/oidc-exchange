# Done Certificate — Task 02: fail-safe `/revoke` error propagation

**Task:** [02-revoke_error_propagation.md](02-revoke_error_propagation.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> Verification protocol for Task 02. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** `/revoke` returns 503 when the session repository fails, while still returning
  200 for a revoked, invalid, or unknown token — a stolen-token client is never falsely told the
  session is dead.
- **P2 — Obligations.** Done iff O1…O6 all hold, in DoD order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not break the token-verification best-effort path (an `access_token`
  whose signature fails still returns 200) or the idempotent-delete semantics of the
  `SessionRepository` adapters.

## Obligations

- **O1 — 503 on store failure, 200 on success.**
  - *Claim:* `revoke_handler` returns 503 with the standard body when `revoke` yields
    `Err(StoreError)`, and 200 when it yields `Ok(())`.
  - *Evidence to collect:* read `crates/core/src/service/revoke.rs:20,28,34` — confirm each
    `let _ =` became `?`. Read `crates/server/src/routes/revoke.rs` — confirm the result is
    matched (`Ok` → 200, `Err` → 503 with `{"error","error_description"}`). Run the new
    `/revoke` E2E tests — expect the store-failure case PASS at 503 and the success case at 200.
  - *Checks:* resolve `revoke_all_user_sessions` / `revoke_session` in `revoke.rs` to the
    `session_repo` port methods (`crates/core/src/ports/repository.rs:27-28`), confirming the
    propagated `Err` is `Error::StoreError`, not a shadowed local.
  - *Status:* ☑ SATISFIED — Both `let _ =` store calls now propagate with `?`:
    `revoke.rs:53` (`self.session_repo.revoke_all_user_sessions(&user_id).await?`) and
    `revoke.rs:113` (`self.session_repo.revoke_session(&token_hash).await?`). The pre-change
    tree (`jj file show -r @-`) had exactly two `let _ =` sites (old :31 and :73); both are
    converted — the certificate's `:20,:28,:34` line refs are stale/approximate, not a defect.
    `crates/server/src/routes/revoke.rs:59-74` matches the result: `Ok(())` → `StatusCode::OK`,
    `Err(e)` → 503 (`StatusCode::SERVICE_UNAVAILABLE`) with a `RevokeErrorBody{error,
    error_description}`. Function resolution: `session_repo` is `Box<dyn SessionRepository>`
    (`service/mod.rs:24`); both calls resolve to the `SessionRepository` trait methods
    (`ports/repository.rs:27-28`) returning `crate::error::Result<()>`, so `?` yields
    `crate::error::Error` — the mock returns `Error::StoreError{detail}` (`test-utils/src/lib.rs`).
    No shadowing. E2E: `revoke_store_failure_returns_503_refresh_token` and
    `..._access_token` PASS at 503; `revoke_returns_200` PASS at 200.

- **O2 — Negative-space: token-verification failure still returns 200.**
  - *Claim:* an `access_token`-hint request with a malformed/unsigned token returns 200 and never
    propagates.
  - *Evidence to collect:* run the E2E case driving `/revoke` with a bad-signature token —
    expect 200. Trace `verify_and_extract_sub` returning `None` → the `Ok(())` arm, confirming no
    `?` fires on that path.
  - *Status:* ☑ SATISFIED — E2E `revoke_access_token_verification_failure_returns_200_no_propagation`
    drives `/revoke` with `token=not.a-valid.jwt&token_type_hint=access_token` while the store
    fail-mode is ON, and returns 200 (PASS). Trace: `verify_and_extract_sub` (`revoke.rs:130`)
    returns `None` for the malformed token, so the `if let Some(user_id)` block (`revoke.rs:44-64`)
    is skipped entirely — `revoke_all_user_sessions().await?` is never reached — and the arm falls
    through to `Ok(())` → 200. The store fail-mode being ON yet never observed proves no
    propagation on the verification-failure path.

- **O3 — The 503 path logs the detail; the client body leaks nothing.**
  - *Claim:* the `Err` arm emits `tracing::error!` with the error (captured under the request
    span from task 01) and the client body carries no infrastructure detail.
  - *Evidence to collect:* read the handler's `Err` arm; confirm the `tracing::error!` call and
    that the response body is a fixed generic message. Optionally capture the event with a tracing
    subscriber and confirm `request_id` is present.
  - *Status:* ☑ SATISFIED — Handler `Err` arm (`routes/revoke.rs:66-73`) emits
    `tracing::error!(error = %e, "revoke: session repository failed")` and returns a fixed
    generic body `{error:"server_error", error_description:"internal server error"}` — no
    infrastructure detail. E2E `revoke_store_failure_returns_503_refresh_token` asserts the
    body does NOT contain the store's internal `"mock session store failure"` string (no leak).
    The request span from task 01 exists (`server/src/middleware/request_id.rs` opens an
    `info_span!` carrying `request_id`), so the `error!` event is captured under it in
    production. NOTE: the optional subscriber-capture of `request_id` was not exercised here —
    `build_test_app` installs only `audit_context_layer`, not `request_id_layer` — but this is
    the certificate's explicitly optional evidence, and the span mechanism is task 01's
    (a dependency), so the mandatory content is met.

- **O4 — Two meaningful assertions each in `revoke` and `revoke_handler`.**
  - *Claim:* both functions carry two or more non-trivial assertions.
  - *Evidence to collect:* read both functions; count and confirm each assertion guards a real
    property (e.g. non-empty token, 64-char hash).
  - *Status:* ☑ SATISFIED — `revoke` (core, `revoke.rs`): `:33` `assert!(!request.token.is_empty())`
    (precondition) and `:49-52` `assert!(!user_id.is_empty())` (verified-sub postcondition); its
    helper `revoke_refresh_token` adds `:90-94` `assert_eq!(token_hash.len(), TOKEN_HASH_HEX_LEN)`
    and `:106-109` `assert!(!session.user_id.is_empty())` — four meaningful guards over the core
    revoke path (≥2). `revoke_handler` (server, `routes/revoke.rs`): `:43-46`
    `assert!(!form.token.is_empty(), ...)` (non-empty past the boundary check) and `:79-84`
    `assert!(status == OK || BAD_REQUEST || SERVICE_UNAVAILABLE, ...)` (response postcondition) —
    two guards. Each guards a real property (non-empty token/sub/user_id, 64-char hex hash,
    bounded response status), not a tautology. TOKEN_HASH_HEX_LEN is a named constant (=64).

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0 (clean);
    `cargo clippy --workspace --all-targets -- -D warnings` exit 0 (clean);
    `cargo nextest run --workspace` → 351 tests, 351 passed, 27 skipped (the single "leaky"
    mark is on the unrelated `oidc-exchange-adapters` HTTP-timeout timing test, not a failure).
    Named-constant limit honoured: `TOKEN_HASH_HEX_LEN = 64` replaces the magic number.

- **O6 — Reviewable: 503 on store failure, 200 on success, 200 on verification failure.**
  - *Claim:* a reviewer runs the `/revoke` E2E tests and sees the three outcomes.
  - *Evidence to collect:* run the `/revoke` E2E suite and read the three assertions (503, 200,
    200) passing.
  - *Status:* ☑ SATISFIED — Ran the `/revoke` E2E suite
    (`cargo nextest run -p oidc-exchange --test routes revoke`): 5 passed —
    `revoke_store_failure_returns_503_refresh_token` (503), `..._access_token` (503),
    `revoke_returns_200` (200 success), `revoke_access_token_verification_failure_returns_200_no_propagation`
    (200), and `revoke_empty_token_returns_400` (400). All three named outcomes (503 on store
    failure, 200 on success, 200 on verification failure) observed directly.

## Regression check

- `crates/server/src/routes/revoke.rs:revoke_handler` is the sole caller of
  `AppService::revoke`; trace it with a success token → expect 200 unchanged (the RFC 7009
  always-200-for-token-state contract preserved) : ☑ PRESERVED — `revoke_handler` is the sole
  caller of `AppService::revoke`; `revoke_returns_200` (a successful revoke) still returns 200.
  Only a genuine `Err(StoreError)` now maps to 503; every token-state outcome (revoked, invalid,
  unknown) still returns 200.
- Existing revoke tests in `crates/core` / `crates/server` that assumed a swallowed error still
  pass under the new propagation (updated where they asserted the old 200-on-failure) : ☑
  PRESERVED — no pre-existing test asserted 200-on-store-failure (that path was untested before);
  the pre-existing `revoke_returns_200` needed no change and passes, and the full workspace suite
  is 351/351. Nothing required updating.

## Residue

- The 503 uses `503 Service Unavailable` per RFC 7009 §2.2.1, not 500 — confirm the status
  constant is `SERVICE_UNAVAILABLE`. Outside the DoD but load-bearing for the Decision.
  CONFIRMED — `routes/revoke.rs:72` returns `StatusCode::SERVICE_UNAVAILABLE`; E2E asserts
  `response.status() == StatusCode::SERVICE_UNAVAILABLE`.
- Beyond-DoD addition (not a defect): the handler adds an `if form.token.is_empty()` boundary
  check returning `400 invalid_request` (`routes/revoke.rs:33-39`), so untrusted empty client
  input yields a clean 400 rather than tripping the core `assert!(!token.is_empty())` into a
  panic. Spec-compliant per RFC 7009 §2.1 (missing required `token` → `invalid_request`) and
  covered by `revoke_empty_token_returns_400`.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O6 all SATISFIED with evidence — both store calls propagate with `?`, the handler
maps `Err`→503 (SERVICE_UNAVAILABLE, generic no-leak body, logged via `tracing::error!`) and
`Ok`→200, the verification-failure path still returns 200 with no propagation, both functions
carry ≥2 meaningful assertions, and fmt/clippy(-D warnings)/nextest(351/351) are clean; the
sole `AppService::revoke` caller and existing tests are PRESERVED (no test asserted the old
200-on-failure). Only the certificate's stale `:20/:28/:34` line refs (two real sites, both
converted) and O3's explicitly-optional request_id-subscriber capture (not exercised; span is
task 01's) are notes, not gaps.
