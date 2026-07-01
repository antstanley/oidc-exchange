# Done Certificate — Task 03: log the `server_error` internal detail

**Task:** [03-server_error_detail_logging.md](03-server_error_detail_logging.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> Verification protocol for Task 03. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** 500/502/504 (`server_error`) responses log their internal source error via
  `tracing::error!` under the request span, while the client still receives only the generic
  body.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not change the client-facing status/body for any arm, and must not
  log for the client-fault (4xx) arms.

## Obligations

- **O1 — Each `server_error` arm emits a `tracing::error!` with the internal detail.**
  - *Claim:* `ProviderError` (502), `ProviderTimeout` (504), and the
    `StoreError|KeyError|AuditError|SyncError|ConfigError` group (500) each log the source error.
  - *Evidence to collect:* read `crates/server/src/error.rs:88-106`; confirm a `tracing::error!`
    carrying the error precedes the generic-body return for those arms. Run the error-mapping
    test asserting a captured `error`-level event for a `ProviderError`/`StoreError` — expect PASS.
  - *Checks:* confirm the log emits inside the request span (task 01) — the `error!` fires within
    request handling, not at router-build time — so the captured event carries `request_id`.
  - *Status:* ☐ unverified

- **O2 — Negative-space: a client-fault error logs no `server_error` detail; 5xx bodies stay generic.**
  - *Claim:* `InvalidGrant` (and the other 4xx arms) produce no error-level detail log; the 5xx
    client bodies contain no infrastructure detail.
  - *Evidence to collect:* run the test asserting an `InvalidGrant` produces no captured
    `server_error` log; read the 5xx arms and confirm the returned `error_description` is a fixed
    generic string.
  - *Status:* ☐ unverified

- **O3 — The touched function carries at least two meaningful assertions.**
  - *Claim:* two or more non-trivial assertions on the touched path.
  - *Evidence to collect:* read the touched function; confirm the assertions guard real
    properties (e.g. mapped status is 5xx before logging; `error_code == "server_error"`).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: 5xx logs the detail, 4xx does not, client body stays generic.**
  - *Claim:* a reviewer runs the error-mapping tests and sees a captured `error` log with the
    internal detail for a 5xx, none for a 4xx, and a generic client body.
  - *Evidence to collect:* run the error-mapping test module and inspect the captured events and
    the response body for the 5xx vs 4xx cases.
  - *Status:* ☐ unverified

## Regression check

- `ApiError::into_response` (`crates/server/src/error.rs:29`) is the sole caller of
  `map_domain_error`; trace it for each existing arm and confirm the status/body are unchanged
  for the 4xx arms and the 5xx arms (only the new log is added) : ☐ (PRESERVED / REGRESSION)

## Residue

- If task 02's `/revoke` 503 goes through a bespoke path rather than `map_domain_error`, this
  task's log covers the general error mapper only; note any 5xx path that bypasses it — outside
  this DoD.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
