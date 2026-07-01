# Done Certificate — Task 04: request-timeout middleware layer

**Task:** [04-request_timeout_layer.md](04-request_timeout_layer.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> Verification protocol for Task 04. A validating agent discharges it: for each obligation,
> collect the named evidence, run the named checks, set the Status, then derive the Conclusion
> by the rubric. Do not mark an obligation SATISFIED without its evidence; do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O6 below holds, each backed by the evidence it names — not
by assertion.

## Premises

- **P1 — Goal.** A request running longer than `server.request_timeout` (default 30 s) is
  aborted with 408, and the bound is a `[server]` config key, not a hard-coded constant.
- **P2 — Obligations.** Done iff O1…O6 all hold, in DoD order; O6 is the Reviewable item.
- **P3 — Invariants.** Must not reorder the existing middleware stack incorrectly — the timeout
  layer sits inside request-id and outside audit-context and catch-panic.

## Obligations

- **O1 — Over-bound request → 408, under-bound → 200.**
  - *Claim:* a handler sleeping past a short configured `request_timeout` returns 408; a fast one
    returns 200.
  - *Evidence to collect:* read `crates/server/src/bootstrap.rs` — confirm a
    `TimeoutLayer::new(request_timeout)` is layered. Run the timeout server test — expect the
    slow case 408 and the fast case 200.
  - *Checks:* confirm `TimeoutLayer` is `tower_http::timeout::TimeoutLayer` and that the pinned
    `tower-http` 0.6 responds `408 Request Timeout` on expiry (not 503/500).
  - *Status:* ☐ unverified

- **O2 — `request_timeout` is a `[server]` key defaulting to `"30s"` via a named constant.**
  - *Claim:* `ServerConfig` has `request_timeout: String` defaulting to `"30s"` through a named
    constant; the config tests cover the default and an override.
  - *Evidence to collect:* read `crates/core/src/config.rs` — confirm the field, the named
    constant (e.g. `DEFAULT_REQUEST_TIMEOUT`), and the `Default` impl. Run the config tests
    (`deserialize_default_toml` / the override test) — expect PASS.
  - *Status:* ☐ unverified

- **O3 — Negative-space: unparseable `request_timeout` fails fast; layer sits inside request-id.**
  - *Claim:* an unparseable duration returns a startup config error (no silent default), and a
    timed-out response still carries the request id.
  - *Evidence to collect:* trace the parse path — confirm an unparseable value yields an `Err`
    (config error) at startup, not a fallback. Confirm from the middleware ordering (and, given
    task 01, from the 408 response header) that the timeout layer is inside the request-id layer.
  - *Checks:* resolve the parser to the humantime-style `[token]`-TTL parser
    (`crates/core/src/service/mod.rs:168`) or an equivalent, not an ad-hoc `unwrap`.
  - *Status:* ☐ unverified

- **O4 — Two meaningful assertions where the duration is built; the default is a named constant.**
  - *Claim:* the duration-building code carries two or more non-trivial assertions and no magic
    literal for the default.
  - *Evidence to collect:* read the duration-building site; confirm the assertions (e.g. non-zero,
    within a sane upper bound) and the named constant.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean; the `timeout` feature is enabled on
    `tower-http`.
  - *Evidence to collect:* confirm `crates/server/Cargo.toml` lists the `timeout` feature. Run
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O6 — Reviewable: slow request → 408, fast → 200, config default 30 s.**
  - *Claim:* a reviewer runs the timeout test and sees 408/200 and confirms `request_timeout`
    reads from config with a 30 s default.
  - *Evidence to collect:* run the timeout test and read the config default assertion.
  - *Status:* ☐ unverified

## Regression check

- `crates/server/src/bootstrap.rs:build_router` assembles the stack; trace the existing E2E in
  `crates/server/tests/e2e.rs` and confirm normal (fast) requests still return their prior status
  with the new layer in place : ☐ (PRESERVED / REGRESSION)
- `crates/core/src/config.rs` `deserialize_full_config` / `deserialize_default_toml` still pass
  with the added field : ☐ (PRESERVED / REGRESSION)

## Residue

- If `parse_duration_secs` is made `pub` to reach the server, confirm no other crate relies on
  its `pub(crate)` visibility — outside this DoD but a compile-surface note.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
