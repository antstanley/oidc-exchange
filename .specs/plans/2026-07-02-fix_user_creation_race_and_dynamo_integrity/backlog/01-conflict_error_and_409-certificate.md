# Done Certificate — Task 01: Conflict error variant and 409 mapping

**Task:** [01-conflict_error_and_409.md](01-conflict_error_and_409.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

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
  - *Status:* ☐ unverified

- **O2 — The `map_domain_error` match is exhaustive over the enum.**
  - *Claim:* the match has an arm per `Error` variant including `Conflict`, with no `_ =>` wildcard.
  - *Evidence to collect:* read `map_domain_error` in `crates/server/src/error.rs`; confirm no wildcard arm and every variant of `crates/core/src/error.rs::Error` is named. Run `cargo clippy --workspace -- -D warnings` — expect clean (a non-exhaustive match would fail to compile).
  - *Status:* ☐ unverified

- **O3 — Negative-space: an out-of-enum error code is rejected by the schema.**
  - *Claim:* the `OAuthErrorEnvelope` `error` enum stays closed at eight members.
  - *Evidence to collect:* validate `{"error":"teapot"}` against `$defs.OAuthErrorEnvelope` — expect INVALID; confirm the enum lists exactly the eight members with `conflict` between `unsupported_grant_type` and `server_error`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, and format are clean for the touched Rust crates.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — expect all clean. Confirm the `02-ports-and-adapters.md` §UserRepository and `04-http-api.md` prose edits accompany the code change (domain-type/contract change updates prose together).
  - *Status:* ☐ unverified

- **O5 — Reviewable: constructing and rendering a `Conflict` yields 409 `conflict`.**
  - *Claim:* a reviewer renders `ApiError::Domain(Error::Conflict { detail: "…" })` and observes `409` with `error == "conflict"`.
  - *Evidence to collect:* run the server unit test (or an ad-hoc `cargo test` case) that builds the error and inspects `into_response()` — expect status `409`, JSON `error` field `"conflict"`, `error_description` echoing the detail.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/error.rs::map_domain_error` is called by `ApiError::into_response`; trace an existing variant (e.g. `Error::InvalidGrant`) through the modified match → expect still `(400, "invalid_grant", reason)` : ☐ (PRESERVED / REGRESSION)
- A caller validating an existing envelope body (`{"error":"invalid_grant"}`) against the extended enum → expect still valid : ☐ (PRESERVED / REGRESSION)

## Residue

- The `02-ports-and-adapters.md` contract paragraph asserts version-atomicity and delete-frees-id, which are implemented by tasks 08 and 09; a validator should not treat those behaviours as obligations of Task 01 (only the contract prose and the `Conflict` mapping are in scope here).

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
