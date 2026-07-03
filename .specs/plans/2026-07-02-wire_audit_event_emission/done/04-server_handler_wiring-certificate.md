# Done Certificate — Task 04: server handler wiring

**Task:** [04-server_handler_wiring.md](04-server_handler_wiring.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 04. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The `/token` and `/revoke` handlers consume `Extension<AuditContext>` and thread
  its fields into the core request structs, so a real request populates the stored session.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing grant-type routing in `token_handler`
  (`crates/server/src/routes/token.rs:27-54`) or the RFC 7009 always-200 contract of
  `revoke_handler` (`crates/server/src/routes/revoke.rs:20-28`).

## Obligations

- **O1 — Handlers consume `AuditContext` and thread it in.**
  - *Claim:* `token_handler` and `revoke_handler` take `Extension<AuditContext>` and populate the
    core requests' `ip_address`/`user_agent`/`device_id` from it.
  - *Evidence to collect:* read `token.rs:23-55` and `revoke.rs:16-29`; confirm the extractor is
    present and each request struct is built with the context fields.
  - *Checks:* resolve `AuditContext` to `crates/server/src/middleware/audit_context.rs`; confirm the
    layer is installed at `crates/server/src/bootstrap.rs:135` so the extension is present at
    request time (extractor would 500 otherwise).
  - *Status:* ☑ SATISFIED — `token_handler` takes `Extension(audit_ctx): Extension<AuditContext>`
    (`token.rs:26`) and builds `ExchangeRequest` (`token.rs:41-43`) and `RefreshRequest`
    (`token.rs:56-58`) with cloned `ip_address`/`user_agent`/`device_id`; `revoke_handler` takes the
    extractor (`revoke.rs:19`) and builds `RevokeRequest` with the three fields (`revoke.rs:27-29`).
    `AuditContext` in both files resolves via `use crate::middleware::audit_context::AuditContext`
    to the struct at `crates/server/src/middleware/audit_context.rs:10` — no shadowing. The layer is
    installed in `build_router` at `crates/server/src/bootstrap.rs:318` (the authored pointer `:135`
    has drifted; same `.layer(axum::middleware::from_fn(audit_context_layer))` call), so the
    extension is present at request time; the test routers in `tests/routes.rs` and `tests/e2e.rs`
    also add the layer so handlers do not 500.

- **O2 — Headers reach the stored session.**
  - *Claim:* a request with the audit headers stores a session with those exact values; without
    them, `None` for each.
  - *Evidence to collect:* run the new end-to-end handler test — expect PASS asserting the stored
    session's ip/ua/device equal the `X-Forwarded-For`/`User-Agent`/`X-Device-Id` values.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange` (the server crate's package
    name; `oidc-exchange-server` does not exist):
    `routes::token_exchange_with_audit_headers_stores_session_context` PASS — asserts the single
    stored session has `ip_address == Some("203.0.113.7")`, `user_agent ==
    Some("audit-test-client/1.0")`, `device_id == Some("device-42")` matching the sent headers;
    the no-headers half is proven by the O3 test (all three `None`).

- **O3 — Negative-space test: no headers → `None`.**
  - *Claim:* a `/token` request with no audit headers stores a session with `None` ip/ua/device.
  - *Evidence to collect:* run the no-headers handler test — expect PASS with all three session
    fields `None` (not empty strings).
  - *Status:* ☑ SATISFIED — `routes::token_exchange_without_audit_headers_stores_none_session_context`
    PASS: asserts `session.ip_address == None`, `session.user_agent == None`,
    `session.device_id == None` (exact `None` equality, so empty strings would fail).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named.
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --check` exit 0; `cargo clippy --workspace -- -D warnings`
    clean; `cargo nextest run --workspace` → 316 passed, 0 failed, 27 skipped. No new numeric
    limits were introduced by this diff (header values are threaded verbatim), so the
    named-constant rule is trivially met.

- **O5 — Reviewable: header-to-session test passes.**
  - *Claim:* a `POST /token` with the three headers stores a session carrying those values.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-server` — expect the
    header-to-session test PASS; inspect the stored session fields in the assertion.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange` (the server crate's actual
    package name): 56/56 tests pass, including
    `token_exchange_with_audit_headers_stores_session_context`. Inspected the assertion
    (`crates/server/tests/routes.rs:296-301`): the test pulls the stored session from the shared
    `MockRepository` handle returned by `build_test_app()` and asserts the three fields equal the
    exact header values sent.

## Regression check

- The grant-type match in `token_handler` (authorization_code / id_token / refresh_token) → trace
  each branch still builds its request and returns the same response after adding the extractor →
  expect unchanged routing : ☑ PRESERVED — the match at `token.rs:29-64` is structurally unchanged
  (authorization_code|id_token → exchange, refresh_token → refresh, `_` →
  `ApiError::UnsupportedGrantType`); only the three context fields replaced `..Default::default()`.
  Pre-existing tests `token_exchange_returns_200_with_access_token`,
  `token_invalid_grant_type_returns_400`, `token_missing_code_returns_400`, and the e2e suite all
  PASS.
- `revoke_handler` still returns `StatusCode::OK` regardless of outcome → expect the 200 contract
  preserved : ☑ PRESERVED — `revoke.rs:22-33` still discards the service result (`let _ =`) and
  unconditionally returns `StatusCode::OK`; `revoke_returns_200` PASS.

## Residue

- Emission of audit events from these flows is Tasks 05–07; Task 04 only threads the context. Not
  an obligation here.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (extractors and field threading read at
`token.rs:26,41-43,56-58` and `revoke.rs:19,27-29`; both new header-to-session tests PASS; fmt,
clippy `-D warnings`, and the 316-test workspace suite all clean), and both named regression
surfaces are PRESERVED — grant-type routing and the RFC 7009 always-200 contract are unchanged.
Notes: the certificate's package name `oidc-exchange-server` does not exist — the server crate is
`oidc-exchange` — and the bootstrap layer pointer has drifted from `:135` to `:318`; both are
protocol-text staleness, not implementation defects.
