# Done Certificate — Task 01: not_found error variant

**Task:** [01-not_found_error_variant.md](01-not_found_error_variant.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The task produces a `NotFound` domain error that `map_domain_error` renders as HTTP 404 with error code `not_found`.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing mapping of any other `Error` variant in `map_domain_error` (`crates/server/src/error.rs:51-108`) — the 5xx catch-all must still cover `StoreError`/`KeyError`/`AuditError`/`SyncError`/`ConfigError`.

## Obligations

- **O1 — `map_domain_error(&Error::NotFound { detail })` returns `(404, "not_found", <detail>)`.**
  - *Claim:* the new match arm maps `Error::NotFound` to `StatusCode::NOT_FOUND`, code `not_found`, description equal to `detail`.
  - *Evidence to collect:* read the new arm in `crates/server/src/error.rs`; run the server-crate unit test that asserts `map_domain_error(&Error::NotFound { detail: "x".into() })` yields `(StatusCode::NOT_FOUND, "not_found", "x")` — expect PASS.
  - *Checks:* resolve `Error::NotFound` to the variant added in `crates/core/src/error.rs` (imported `oidc_exchange_core::error::Error`), not a server-local type.
  - *Status:* ☑ SATISFIED — arm at `crates/server/src/error.rs:91-95` returns `(StatusCode::NOT_FOUND, "not_found".to_string(), detail.clone())`; test `error::tests::not_found_error_renders_404_with_not_found_code` PASSED (asserts status 404, `error == "not_found"`, `error_description == "user abc-123 not found"` via `ApiError::Domain(...).into_response()`, which calls `map_domain_error`). Resolution check: `Error` in `crates/server/src/error.rs` resolves through `use oidc_exchange_core::error::Error;` (line 6) to the variant added at `crates/core/src/error.rs:30-31`; no server-local shadow (`ApiError` is a distinct wrapper type).

- **O2 — The `match` stays exhaustive with no wildcard swallowing `NotFound` into the 5xx arm.**
  - *Claim:* `NotFound` has its own arm; the 5xx group arm does not include it, and no `_ =>` wildcard exists.
  - *Evidence to collect:* read the full `match err` in `crates/server/src/error.rs`; confirm `NotFound` is an explicit arm and the `StoreError | KeyError | AuditError | SyncError | ConfigError` arm is unchanged; confirm compilation succeeds without a wildcard (a non-exhaustive match would fail to compile). Add or run a negative-space test asserting `NotFound` does not render 500.
  - *Status:* ☑ SATISFIED — read full `match err` at `crates/server/src/error.rs:52-115`: `Error::NotFound` has its own explicit arm (lines 91-95); the 5xx group arm (lines 106-114) is exactly `StoreError | KeyError | AuditError | SyncError | ConfigError`, unchanged by the diff; no `_ =>` wildcard anywhere in the match, and the workspace compiles clean (clippy `-D warnings` passed), proving exhaustiveness. Negative-space assertion `assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR)` in the new test PASSED.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, any new bound named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` (the commands in `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 285 tests run, 285 passed, 27 skipped, 0 failed. The diff introduces no new numeric bounds, so the named-constant rule is trivially met.

- **O4 — Reviewable: `Error::NotFound` produces a 404 `not_found` envelope, not the generic 500.**
  - *Claim:* a reviewer reading the arm and running the mapping unit test sees a 404 `not_found` result.
  - *Evidence to collect:* run the mapping unit test from O1 and read the arm; confirm the tuple is `(404, "not_found", detail)` and not the `server_error` / 500 branch.
  - *Status:* ☑ SATISFIED — exercised as named: read the arm (`crates/server/src/error.rs:91-95` — tuple is `(StatusCode::NOT_FOUND, "not_found", detail.clone())`, not the `server_error` branch) and ran `cargo nextest run -E 'test(not_found_error_renders_404_with_not_found_code)'` → PASS (1/1), observing the 404 `not_found` envelope. Also confirmed `crates/ffi/src/lib.rs` has no domain-error mapping table to update (it wraps errors in its own `FfiError` string type and proxies HTTP responses).

## Regression check

- `ApiError::into_response` (`crates/server/src/error.rs:29-49`) calls `map_domain_error` for every domain error → after adding the arm, trace one existing caller path (e.g. `Error::InvalidRequest`) and confirm it still maps to `(400, "invalid_request", reason)` : ☑ PRESERVED — the `Error::InvalidRequest` arm (`crates/server/src/error.rs:63-67`) is untouched by the diff and still returns `(StatusCode::BAD_REQUEST, "invalid_request", reason.clone())`; the pre-existing `conflict_error_renders_409_with_conflict_code` test exercising `ApiError::into_response → map_domain_error` still PASSES, as do all 285 workspace tests.

## Residue

- The adapters' `StoreError { "user not found" }` branches stay as an unreachable backstop; not an obligation of this task.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with direct evidence (arm read at crates/server/src/error.rs:91-95, targeted unit test PASS, fmt/clippy/nextest all clean at 285/285), and the InvalidRequest regression trace is PRESERVED with no wildcard introduced into the match.
