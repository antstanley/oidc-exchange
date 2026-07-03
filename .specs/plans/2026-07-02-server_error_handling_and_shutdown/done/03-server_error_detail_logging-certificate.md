# Done Certificate — Task 03: log the `server_error` internal detail

**Task:** [03-server_error_detail_logging.md](03-server_error_detail_logging.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `crates/server/src/error.rs:51-78`: `map_domain_error` computes the
    tuple via `map_domain_error_inner` and, when `error_code == "server_error"`, emits
    `tracing::error!(error = %err, status = %status, "internal error mapped to server_error
    response")`. `map_domain_error_inner` (lines 125-143) sets `"server_error"` for exactly
    `ProviderError` (502), `ProviderTimeout` (504), and the `StoreError|KeyError|AuditError|
    SyncError|ConfigError` group (500), so all five arms hit the log. Tests
    `provider_error_logs_internal_detail_and_returns_generic_body`,
    `provider_timeout_logs_internal_detail_and_returns_generic_body`, and
    `store_error_logs_internal_detail_and_returns_generic_body` each PASS and assert exactly one
    ERROR event whose `error` field carries the internal detail (`connection reset by upstream`,
    `microsoft`, `database is locked`). Check: the `error!` fires from within
    `ApiError::into_response` (line 40) during request handling — the handler is wrapped by
    `request_id_layer`'s `info_span` (`crates/server/src/middleware/request_id.rs:42-56`), so the
    event inherits `request_id`, not at router-build time.

- **O2 — Negative-space: a client-fault error logs no `server_error` detail; 5xx bodies stay generic.**
  - *Claim:* `InvalidGrant` (and the other 4xx arms) produce no error-level detail log; the 5xx
    client bodies contain no infrastructure detail.
  - *Evidence to collect:* run the test asserting an `InvalidGrant` produces no captured
    `server_error` log; read the 5xx arms and confirm the returned `error_description` is a fixed
    generic string.
  - *Status:* ☑ SATISFIED — `invalid_grant_emits_no_server_error_detail_log` PASSes, asserting
    `error_events.is_empty()` for `InvalidGrant` (the branch is keyed on `error_code ==
    "server_error"`, which the 4xx arms never set). The 5xx arms (error.rs:125-143) return fixed
    generic strings — `"upstream provider error"`, `"upstream provider timeout"`, `"internal
    server error"` — carrying no infrastructure detail; the 5xx tests further assert
    `!description.contains("connection reset")` and `!description.contains("sqlite")`.

- **O3 — The touched function carries at least two meaningful assertions.**
  - *Claim:* two or more non-trivial assertions on the touched path.
  - *Evidence to collect:* read the touched function; confirm the assertions guard real
    properties (e.g. mapped status is 5xx before logging; `error_code == "server_error"`).
  - *Status:* ☑ SATISFIED — `map_domain_error` carries two non-trivial runtime assertions
    (error.rs:60-71): `assert!(status.is_server_error(), ...)` guards that every arm labelled
    `server_error` maps to a 5xx (catches a future arm that mislabels a client fault), and
    `assert_ne!(description, err.to_string(), ...)` guards that the client-facing description
    never repeats the internal `Display` detail (catches a detail leak into the response body).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D
    warnings`, `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0 (clean); `cargo clippy --workspace
    -- -D warnings` exit 0 (clean); `cargo nextest run --workspace` → 355 passed, 27 skipped, 0
    failed.

- **O5 — Reviewable: 5xx logs the detail, 4xx does not, client body stays generic.**
  - *Claim:* a reviewer runs the error-mapping tests and sees a captured `error` log with the
    internal detail for a 5xx, none for a 4xx, and a generic client body.
  - *Evidence to collect:* run the error-mapping test module and inspect the captured events and
    the response body for the 5xx vs 4xx cases.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange error::` ran the module: all 7
    tests PASS. The three 5xx tests capture one ERROR event carrying the internal detail and
    assert a generic body; `invalid_grant_emits_no_server_error_detail_log` captures zero ERROR
    events for the 4xx case; `conflict_error_renders_409...` and `not_found_error_renders_404...`
    confirm the 4xx/409/404 client bodies are unchanged.

## Regression check

- `ApiError::into_response` (`crates/server/src/error.rs:29`) is the sole caller of
  `map_domain_error`; trace it for each existing arm and confirm the status/body are unchanged
  for the 4xx arms and the 5xx arms (only the new log is added) : ☑ PRESERVED — `into_response`
  (error.rs:39-46) is the sole caller of `map_domain_error`, which now delegates the full
  `(status, error_code, description)` mapping to `map_domain_error_inner` (an unchanged move of
  every match arm) and only adds a side-effecting `tracing::error!` inside the
  `error_code == "server_error"` branch. The 4xx/409/404 arms never enter that branch, so their
  status/body are byte-for-byte unchanged; the 5xx arms return the same tuple as before plus the
  log. The pre-existing `conflict_error_renders_409...` and `not_found_error_renders_404...`
  response tests still PASS, confirming no client-facing regression.

## Residue

- If task 02's `/revoke` 503 goes through a bespoke path rather than `map_domain_error`, this
  task's log covers the general error mapper only; note any 5xx path that bypasses it — outside
  this DoD.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence (5 server_error arms log the detail via
`tracing::error!` inside the request span, 4xx arms log nothing, bodies stay generic, two
meaningful assertions, fmt/clippy clean, 355 tests pass) and the sole `into_response` caller of
`map_domain_error` is PRESERVED.
