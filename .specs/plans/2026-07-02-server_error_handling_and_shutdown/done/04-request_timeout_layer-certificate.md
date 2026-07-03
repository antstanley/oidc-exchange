# Done Certificate — Task 04: request-timeout middleware layer

**Task:** [04-request_timeout_layer.md](04-request_timeout_layer.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `bootstrap.rs:340-343` layers
    `TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, request_timeout_duration(config))`
    (import `use tower_http::timeout::TimeoutLayer;` at line 7). `with_status_code` pins the
    expiry response to `408` explicitly. `bootstrap::request_timeout_tests`
    `slow_handler_past_timeout_yields_408_with_request_id` (a handler sleeping 60 s under a 50 ms
    bound) returns `StatusCode::REQUEST_TIMEOUT` and `fast_handler_under_timeout_yields_200`
    returns `200`; both PASS.

- **O2 — `request_timeout` is a `[server]` key defaulting to `"30s"` via a named constant.**
  - *Claim:* `ServerConfig` has `request_timeout: String` defaulting to `"30s"` through a named
    constant; the config tests cover the default and an override.
  - *Evidence to collect:* read `crates/core/src/config.rs` — confirm the field, the named
    constant (e.g. `DEFAULT_REQUEST_TIMEOUT`), and the `Default` impl. Run the config tests
    (`deserialize_default_toml` / the override test) — expect PASS.
  - *Status:* ☑ SATISFIED — `config.rs:144` adds `pub request_timeout: String` to `ServerConfig`;
    `config.rs:130` defines `pub const DEFAULT_REQUEST_TIMEOUT: &str = "30s";` and the `Default`
    impl (`config.rs:154`) sets `request_timeout: DEFAULT_REQUEST_TIMEOUT.to_string()`.
    `deserialize_default_toml` asserts `config.server.request_timeout == DEFAULT_REQUEST_TIMEOUT`
    and `== "30s"`; `deserialize_full_config` asserts the `request_timeout = "45s"` override reads
    back and `parse_duration_secs` resolves it to `45`. Both PASS.

- **O3 — Negative-space: unparseable `request_timeout` fails fast; layer sits inside request-id.**
  - *Claim:* an unparseable duration returns a startup config error (no silent default), and a
    timed-out response still carries the request id.
  - *Evidence to collect:* trace the parse path — confirm an unparseable value yields an `Err`
    (config error) at startup, not a fallback. Confirm from the middleware ordering (and, given
    task 01, from the 408 response header) that the timeout layer is inside the request-id layer.
  - *Checks:* resolve the parser to the humantime-style `[token]`-TTL parser
    (`crates/core/src/service/mod.rs:168`) or an equivalent, not an ad-hoc `unwrap`.
  - *Status:* ☑ SATISFIED — fail-fast: `AppConfig::validate` (`config.rs:55-58`) parses
    `server.request_timeout` via `crate::service::parse_duration_secs` behind `prefix_config_error`,
    returning `Error::ConfigError { detail }` naming the field; `validate()` is called by both
    production entry points (`bootstrap.rs:115` `load_config`, `bootstrap.rs:125` `parse_config`)
    before any router is built. Test `validate_rejects_unparseable_request_timeout` confirms a
    `"not-a-duration"` value yields a `ConfigError` echoing the field and bad value — PASS. No
    silent fallback. Parser is the shared humantime `parse_duration_secs` (made `pub`), not an
    ad-hoc `unwrap`. Layer-inside-request-id: `bootstrap.rs:337-344` applies layers innermost-first
    — catch-panic, audit-context, timeout, request-id (last ⇒ outermost) — matching
    `04-http-api.md`'s "outermost first: Request ID, Audit context, Catch-panic" with timeout as
    entry 2. Test `slow_handler_past_timeout_yields_408_with_request_id` asserts the manufactured
    `408` still carries the `x-request-id` header, empirically confirming request-id wraps the
    timeout layer — PASS.

- **O4 — Two meaningful assertions where the duration is built; the default is a named constant.**
  - *Claim:* the duration-building code carries two or more non-trivial assertions and no magic
    literal for the default.
  - *Evidence to collect:* read the duration-building site; confirm the assertions (e.g. non-zero,
    within a sane upper bound) and the named constant.
  - *Status:* ☑ SATISFIED — `request_timeout_duration` (`bootstrap.rs:359-379`) carries two
    non-trivial assertions: `assert!(secs > 0, ...)` (non-zero) and
    `assert!(secs <= REQUEST_TIMEOUT_MAX_SECS, ...)` (sane upper bound). The bound is the named
    constant `REQUEST_TIMEOUT_MAX_SECS: u64 = 60 * 60` (`bootstrap.rs:48`) and the default is the
    named constant `DEFAULT_REQUEST_TIMEOUT` — no magic literal. Both assertions are exercised:
    `request_timeout_duration_panics_on_zero_seconds` (`"0s"`) and
    `request_timeout_duration_panics_on_unparseable_value` PASS.

- **O5 — Meets the repo definition of done.**
  - *Claim:* tests pass and lint/format are clean; the `timeout` feature is enabled on
    `tower-http`.
  - *Evidence to collect:* confirm `crates/server/Cargo.toml` lists the `timeout` feature. Run
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run
    --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `crates/server/Cargo.toml:18` lists
    `tower-http = { version = "0.6", features = ["trace", "request-id", "catch-panic", "timeout"] }`.
    `cargo fmt --all --check` clean (exit 0); `cargo clippy --workspace -- -D warnings` clean
    (Finished, no warnings); `cargo nextest run --workspace` → 362 passed, 0 failed (27 skipped).

- **O6 — Reviewable: slow request → 408, fast → 200, config default 30 s.**
  - *Claim:* a reviewer runs the timeout test and sees 408/200 and confirms `request_timeout`
    reads from config with a 30 s default.
  - *Evidence to collect:* run the timeout test and read the config default assertion.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange request_timeout_tests` → 6/6 PASS,
    including `slow_handler_past_timeout_yields_408_with_request_id` (408) and
    `fast_handler_under_timeout_yields_200` (200). `request_timeout_duration_resolves_documented_default`
    asserts `AppConfig::default()` resolves `request_timeout` to `Duration::from_secs(30)` and that
    the field equals `DEFAULT_REQUEST_TIMEOUT`; `deserialize_default_toml` independently asserts the
    `"30s"` default reads from config. A reviewer can reproduce all three.

## Regression check

- `crates/server/src/bootstrap.rs:build_router` assembles the stack; trace the existing E2E in
  `crates/server/tests/e2e.rs` and confirm normal (fast) requests still return their prior status
  with the new layer in place : ☑ PRESERVED — the full workspace suite (362 passed / 0 failed),
  including `crates/server/tests/{e2e,internal,routes}.rs`, is green with the timeout layer in the
  stack. Note: this task also flips the pre-existing middleware nesting — the old code had
  catch-panic outermost and request-id innermost (the reverse of `04-http-api.md`'s stated order);
  the new code makes request-id outermost, then timeout (entry 2), then audit-context, then
  catch-panic innermost. This reorder is required by the DoD (timeout can only sit *inside*
  request-id and *outside* catch-panic if request-id is outer and catch-panic is inner) and brings
  the code into line with the canonical spec. Panic handling still works because a handler panic
  originates inside catch-panic in both orderings; no e2e test depends on catch-panic wrapping the
  outer middleware, and all panic/request-id tests pass.
- `crates/core/src/config.rs` `deserialize_full_config` / `deserialize_default_toml` still pass
  with the added field : ☑ PRESERVED — both PASS; each was extended to assert the new
  `request_timeout` field (default `"30s"` / override `"45s"`).

## Residue

- If `parse_duration_secs` is made `pub` to reach the server, confirm no other crate relies on
  its `pub(crate)` visibility — outside this DoD but a compile-surface note.
  - Resolved: `parse_duration_secs` is now `pub` (`service/mod.rs:201`); widening visibility from
    `pub(crate)` to `pub` cannot break existing callers. The whole workspace compiles and clippy is
    clean, confirming no downstream breakage. The doc comment correctly reads "`pub` (not
    `pub(crate)`)".

## Conclusion

VERDICT: ☑ DONE
CONFIDENCE: ☑ high
SUMMARY: All six obligations SATISFIED with named evidence — `TimeoutLayer::with_status_code(408, …)`
inserted as middleware entry 2 (inside request-id, outside audit-context/catch-panic); slow→408
(carrying x-request-id) and fast→200 tests pass; `request_timeout` is a `[server]` key defaulting to
`"30s"` via the named constant `DEFAULT_REQUEST_TIMEOUT`, parsed by the shared humantime
`parse_duration_secs`; an unparseable value fails fast in `AppConfig::validate` (no silent fallback);
`request_timeout_duration` carries non-zero and `≤ REQUEST_TIMEOUT_MAX_SECS` assertions; the
`timeout` feature is enabled; fmt/clippy clean and 362 workspace tests pass. Both regression checks
PRESERVED (the middleware reorder is DoD-required and aligns the code with `04-http-api.md`).
