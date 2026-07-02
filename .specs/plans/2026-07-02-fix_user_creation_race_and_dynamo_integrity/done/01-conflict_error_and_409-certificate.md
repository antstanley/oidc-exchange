# Done Certificate — Task 01: Conflict error variant and 409 mapping

**Task:** [01-conflict_error_and_409.md](01-conflict_error_and_409.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 01. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive
> the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence;
> do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** A first-class `Error::Conflict` variant maps to `409 {"error":"conflict"}` and joins the closed `OAuthErrorEnvelope` enum, so adapters and the exchange flow can distinguish "already registered" from an infrastructure failure.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the HTTP status or wire code of any existing `Error` variant mapped in `crates/server/src/error.rs:51-108`, nor break existing `OAuthErrorEnvelope` validation of the seven prior codes.

## Obligations

- **O1 — A `Conflict` renders 409 with a `conflict` body validating against the schema.**
  - *Claim:* `map_domain_error(&Error::Conflict { detail })` returns `(StatusCode::CONFLICT, "conflict", detail)` and the rendered body validates against the updated `OAuthErrorEnvelope`.
  - *Evidence to collect:* read the new arm in `crates/server/src/error.rs`; run the server unit test that renders `ApiError::Domain(Error::Conflict { .. })` and asserts `409` + `error == "conflict"` — expect PASS. Validate a sample `{"error":"conflict","error_description":"x"}` against `.specs/canonical-types.schema.json` `$defs.OAuthErrorEnvelope` — expect valid.
  - *Checks:* resolve `Error::Conflict` at the arm to the variant in `crates/core/src/error.rs`, not a server-local type.
  - *Status:* ☑ SATISFIED — arm at `crates/server/src/error.rs:88-90` returns `(StatusCode::CONFLICT, "conflict", detail.clone())`. Test `error::tests::conflict_error_renders_409_with_conflict_code` (renders `ApiError::Domain(Error::Conflict {..})`, asserts 409 + `error == "conflict"` + echoed detail) → PASS. Sample `{"error":"conflict","error_description":"x"}` validated against `$defs.OAuthErrorEnvelope` with python-jsonschema → VALID. Resolution: `Error` at the arm is `use oidc_exchange_core::error::Error` (server/error.rs:6) → the `Conflict { detail }` variant at `crates/core/src/error.rs:27-28`; the only server-local types are `ApiError`/`ErrorResponse` — no shadowing.

- **O2 — The `map_domain_error` match is exhaustive over the enum.**
  - *Claim:* the match has an arm per `Error` variant including `Conflict`, with no `_ =>` wildcard.
  - *Evidence to collect:* read `map_domain_error` in `crates/server/src/error.rs`; confirm no wildcard arm and every variant of `crates/core/src/error.rs::Error` is named. Run `cargo clippy --workspace -- -D warnings` — expect clean (a non-exhaustive match would fail to compile).
  - *Status:* ☑ SATISFIED — `map_domain_error` (`crates/server/src/error.rs:51-111`) names all 15 `Error` variants (InvalidGrant, InvalidToken, InvalidRequest, UnknownProvider, AccessDenied, UserSuspended, Unauthorized, Conflict, ProviderError, ProviderTimeout, and the 5-way `StoreError | KeyError | AuditError | SyncError | ConfigError` arm); `grep '_ =>'` finds no wildcard. `cargo clippy --workspace -- -D warnings` → clean (Finished, no warnings).

- **O3 — Negative-space: an out-of-enum error code is rejected by the schema.**
  - *Claim:* the `OAuthErrorEnvelope` `error` enum stays closed at eight members.
  - *Evidence to collect:* validate `{"error":"teapot"}` against `$defs.OAuthErrorEnvelope` — expect INVALID; confirm the enum lists exactly the eight members with `conflict` between `unsupported_grant_type` and `server_error`.
  - *Status:* ☑ SATISFIED — python-jsonschema: `{"error":"teapot"}` → INVALID ("'teapot' is not one of [...]"). Enum is exactly `[invalid_grant, invalid_token, invalid_request, access_denied, unauthorized, unsupported_grant_type, conflict, server_error]` (8 members, `conflict` in the stated position). Backed by test `error::tests::oauth_error_envelope_enum_includes_conflict_and_stays_closed` (asserts presence, closedness, len == 8) → PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean for the touched Rust crates.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all clean. Confirm the `02-ports-and-adapters.md` §UserRepository and `04-http-api.md` prose edits accompany the code change (domain-type/contract change updates prose together).
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` → exit 0; `cargo clippy --workspace -- -D warnings` → clean; `cargo nextest run --workspace` → 252 passed, 0 failed, 10 skipped. Diff includes the `02-ports-and-adapters.md` §UserRepository create-conflict / version-atomicity / delete-frees-id paragraph and the `04-http-api.md` `| Conflict | 409 | conflict |` table row. Verified `crates/ffi/src/lib.rs` routes through the axum `router.oneshot(request)` path (lib.rs:112) with no domain-error code table — no FFI change required, as the task recorded.

- **O5 — Reviewable: constructing and rendering a `Conflict` yields 409 `conflict`.**
  - *Claim:* a reviewer renders `ApiError::Domain(Error::Conflict { detail: "…" })` and observes `409` with `error == "conflict"`.
  - *Evidence to collect:* run the server unit test (or an ad-hoc `cargo test` case) that builds the error and inspects `into_response()` — expect status `409`, JSON `error` field `"conflict"`, `error_description` echoing the detail.
  - *Status:* ☑ SATISFIED — exercised via `cargo nextest run -p oidc-exchange -E 'test(conflict_error_renders_409_with_conflict_code)'` → PASS. The test (`crates/server/src/error.rs:126-140`) constructs `ApiError::Domain(Error::Conflict { detail: "user already registered for (google, sub-123)" })`, calls `into_response()`, and observes status 409, `error == "conflict"`, `error_description` echoing the detail.

## Regression check

- `crates/server/src/error.rs::map_domain_error` is called by `ApiError::into_response`; trace an existing variant (e.g. `Error::InvalidGrant`) through the modified match → expect still `(400, "invalid_grant", reason)` : ☑ PRESERVED — trace: `into_response` (error.rs:39-46) → `map_domain_error` → first arm (error.rs:53-57) still returns `(StatusCode::BAD_REQUEST, "invalid_grant", reason)`; the new `Conflict` arm is inserted between `Unauthorized` and `ProviderError` without touching any existing arm; full workspace suite (252 tests) passes.
- A caller validating an existing envelope body (`{"error":"invalid_grant"}`) against the extended enum → expect still valid : ☑ PRESERVED — python-jsonschema: `{"error":"invalid_grant","error_description":"y"}` → VALID; all seven prior enum members are unchanged and in their original order.

## Residue

- The `02-ports-and-adapters.md` contract paragraph asserts version-atomicity and delete-frees-id, which are implemented by tasks 08 and 09; a validator should not treat those behaviours as obligations of Task 01 (only the contract prose and the `Conflict` mapping are in scope here).

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (arm read + targeted tests PASS, no wildcard match, clippy/fmt/nextest clean, schema validated positively and negatively with jsonschema) and both named regression traces are PRESERVED.
