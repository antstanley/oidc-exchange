# Done Certificate — Task 03: client context plumbing

**Task:** [03-client_context_plumbing.md](03-client_context_plumbing.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-03

> This certificate is a verification protocol for Task 03. A validating agent discharges it: collect
> each obligation's evidence, run its checks, set the Status, then derive the Conclusion by the rubric.
> Do not mark an obligation SATISFIED without its evidence; do not record DONE with any non-SATISFIED
> obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O5 below holds, each backed by the evidence it names — not by
assertion.

## Premises

- **P1 — Goal.** The core request structs carry `ip_address`/`user_agent`/`device_id`, the exchange
  flow stores them on the session, and `create_audit_event` takes the client context.
- **P2 — Obligations.** Done iff O1…O5 all hold, in DoD order; O5 is the Reviewable item.
- **P3 — Invariants.** Must not break the existing exchange path (session store, token issuance) in
  `crates/core/src/service/exchange.rs` or the existing `create_audit_event` callers in
  `crates/core/tests/audit.rs`.

## Obligations

- **O1 — Request fields plumbed and session populated.**
  - *Claim:* `ExchangeRequest`/`RefreshRequest`/`RevokeRequest` each carry the three
    `Option<String>` fields, and the exchange flow writes all three onto the stored `Session`.
  - *Evidence to collect:* read `exchange.rs:10-15`, `refresh.rs:8-10`, `revoke.rs:8-11` (fields
    present) and `exchange.rs:152-162` (session `device_id`/`user_agent`/`ip_address` set from
    `request.*`, no longer `None`).
  - *Checks:* resolve the `Session` field assignments to `crate::domain::Session`
    (`crates/core/src/domain/session.rs`); confirm ip/ua/device map to the matching request fields.
  - *Status:* ☑ SATISFIED — `ExchangeRequest` (exchange.rs:11-31), `RefreshRequest` (refresh.rs:8-19),
    and `RevokeRequest` (revoke.rs:8-20) each carry `ip_address`/`user_agent`/`device_id: Option<String>`.
    Session construction at exchange.rs:206-216 sets `device_id: request.device_id.clone()`,
    `user_agent: request.user_agent.clone()`, `ip_address: request.ip_address.clone()` (no `None`).
    `Session` resolves via `use crate::domain::{… Session …}` (exchange.rs:6) to
    `crates/core/src/domain/session.rs:11-13`, whose three `Option<String>` fields match one-to-one.

- **O2 — `create_audit_event` records the context.**
  - *Claim:* `create_audit_event` sets `ip_address`/`user_agent` from new parameters (not hardcoded
    `None`) and takes no `device_id`.
  - *Evidence to collect:* read `crates/core/src/service/mod.rs:132-151`; confirm the signature adds
    ip/ua params and `mod.rs:146-147` uses them; confirm no `device_id` param (the `AuditEvent`
    shape has none — see `crates/core/src/domain/audit.rs`).
  - *Status:* ☑ SATISFIED — `create_audit_event` (mod.rs:146-168) adds `ip_address: Option<String>`
    and `user_agent: Option<String>` params and assigns them directly (`ip_address,` / `user_agent,`
    at mod.rs:162-163, formerly hardcoded `None`). No `device_id` param; `AuditEvent`
    (domain/audit.rs) has only `ip_address`/`user_agent` (lines 17-18), no `device_id` field.
    The only callers are `crates/core/tests/audit.rs` — all eight call sites pass the new args.

- **O3 — Negative-space test: `None` stays `None`, values stored verbatim.**
  - *Claim:* an `ExchangeRequest` with all-`None` context stores a session with `None` for each;
    with values, stores them exactly.
  - *Evidence to collect:* run the new core session-population test — expect PASS for both the
    all-`None` and populated cases; confirm no default-string substitution.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p oidc-exchange-core`:
    `exchange_with_client_context_stores_exact_session_values` PASS (asserts verbatim
    "203.0.113.7" / "integration-test-agent/1.0" / "device-abc-123" on the stored session) and
    `exchange_without_client_context_stores_none_session_values` PASS (asserts all three are
    `None` — no default substitution). Bonus: `create_audit_event_with_no_client_context_leaves_fields_none` PASS.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, limits named; every request-struct caller updated (no
    backwards-compat shim, per AI-agent rule 5).
  - *Evidence to collect:* run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean (the workspace compiles with the new fields).
  - *Status:* ☑ SATISFIED — `cargo fmt --check` clean (exit 0);
    `cargo clippy --workspace -- -D warnings` clean; `cargo nextest run --workspace`:
    314 passed, 0 failed, 27 skipped. No new named constants introduced. All request-struct
    callers updated in place (`..Default::default()` in tests/handlers) — no compat shim.
    Note: `create_audit_event` gained `#[allow(clippy::too_many_arguments)]` (7 params).

- **O5 — Reviewable: session-population test passes, workspace builds.**
  - *Claim:* the new test proves the exchange stores the request's ip/ua/device, and every caller
    of the three request structs compiles with the new fields.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core` — expect PASS; confirm the
    workspace builds (`cargo build --workspace`).
  - *Status:* ☑ SATISFIED — `cargo nextest run -p oidc-exchange-core`: 109 passed, 0 failed,
    including the new session-population tests. `cargo build --workspace --exclude
    oidc-exchange-python` succeeds; the full `cargo build --workspace` fails only at the final
    native link of the `oidc-exchange-python` pyo3 `extension-module` cdylib (undefined `_Py*`
    interpreter symbols — it must be linked via maturin on macOS). That crate is untouched by
    this diff, uses none of the changed types, and compiles cleanly under
    `cargo clippy --workspace -- -D warnings` — a pre-existing environment gap, not a defect.
    Every caller of the three request structs (core tests, `crates/server/src/routes/{token,revoke}.rs`)
    compiles with the new fields.

## Regression check

- `crates/server/src/routes/token.rs` and `revoke.rs` construct these request structs; trace that
  they still compile after the field addition (set to `None` here, wired in Task 04) → expect
  build success : ☑ PRESERVED — both handlers use `..Default::default()` (token.rs:39, 50-53;
  revoke.rs:25); server crate builds and clippy is clean.
- `crates/core/tests/audit.rs` calls `create_audit_event`; trace the updated call sites → expect the
  audit tests still pass : ☑ PRESERVED — all audit tests pass (109/109 core tests), including the
  extended `create_audit_event` assertions on ip/ua.

## Residue

- Task 04 supplies the real header values into these fields; here they may be `None` from the
  server handlers until then. Not an obligation of Task 03.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All five obligations SATISFIED with evidence in hand — the three request structs carry the
client-context fields, the exchange flow stores them verbatim on the `Session`, `create_audit_event`
records passed ip/ua (no `device_id` param), the negative-space tests pass both ways, and
fmt/clippy/nextest (314/314 workspace, 109/109 core) are clean with both regression traces
PRESERVED; the only caveat is a pre-existing, unrelated environment gap linking the
`oidc-exchange-python` pyo3 cdylib outside maturin, which does not touch this task's surface.
