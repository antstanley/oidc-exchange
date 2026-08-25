# Task 05 — HTTP grant surface

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `00-overview.md` and `04-http-api.md`](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 7–8](../../../changes/merged/2026-08-05-bind_id_token_grant_replay_protection.md); [00-overview.md §Scope summary and §Decisions](../../../service/specs/00-overview.md), [04-http-api.md §Routes, §POST /token request, and §GET /.well-known/openid-configuration](../../../service/specs/04-http-api.md)
**Depends on:** 01 (contract), 04 (review)
**Produces:** A deployment with `grants.id_token = true` can mint a nonce, submit the direct grant with an optional provider access token, and discover the grant; the default-disabled deployment has no nonce route and rejects any `id_token` field with `unsupported_grant_type`.
**Pointers:** `crates/server/src/routes/mod.rs:2-23`, new `crates/server/src/routes/nonce.rs`, `crates/server/src/routes/token.rs:14-65`, `crates/server/src/routes/well_known.rs:8-21`, `crates/server/src/bootstrap.rs:326-381`, `crates/server/tests/routes.rs:26-55`, `crates/server/tests/e2e.rs`, `crates/ffi/src/lib.rs:69`

## Steps

- [x] Add an unauthenticated `POST /nonce` handler that delegates minting to core, returns a base64url nonce and `expires_in`, and is mounted only in exchange-serving roles when `grants.id_token` is enabled.
  - `routes::nonce` is boundary-only (no body, delegate to `AppService::mint_nonce`, serialise `{nonce, expires_in}`); `routes::nonce_routes()` mounts from `build_router` only when the role serves exchanges **and** the switch is on — one mounting point shared by hyper, Lambda, and FFI.
- [x] Extend form parsing and `ExchangeRequest` construction for optional `provider_access_token`; before grant dispatch, reject any request carrying `id_token` when the feature switch is disabled, regardless of declared `grant_type`.
  - `TokenForm.provider_access_token` passes through on both exchange grant types (`provider_access_token: form.provider_access_token`). The gate sits at the top of `token_handler`: field presence + switch-off → `ApiError::UnsupportedGrantType` before any branch selection — handler-level because that error class is server-layer, handler-shared because every runtime routes through it.
- [x] Make discovery add `id_token` to `grant_types_supported` only when enabled, while preserving authorization-code and refresh grant advertisement in either state.
  - `well_known::openid_config_handler` builds the list via `grant_types_supported(switch)`; assertions pin both directions (always-served grants never drop; `id_token` mirrors the switch exactly).
- [x] Confirm server, Lambda, and FFI continue to use the shared router path and that disabled routing does not create a nonce endpoint in any exchange-capable runtime.
  - No interface-specific routing was added or needed: `lambda.rs::run_lambda` and `ffi::OidcExchange` both consume `bootstrap::build_router`'s output unchanged, so the mount condition and the token gate are structurally impossible to bypass.
- [x] Add route/E2E tests for enabled nonce issuance and one-time direct exchange, disabled nonce 404, conditional discovery metadata, enabled and disabled form shapes, and both off-switch bypass attempts named by the source spec.
  - New `crates/server/tests/grants.rs` drives `build_router` directly (8 tests): enabled discovery advertisement, nonce shape/independence, full mint→exchange-once→duplicate-fails flow where a deliberately wrong `at_hash` proves `provider_access_token` reaches core binding before the correct-hash pass, disabled `/nonce` 404, both bypass attempts (`grant_type=id_token`; `id_token` smuggled under `authorization_code`), disabled discovery, and admin-role non-mounting even when enabled. Handler-level gate tests appended to `routes.rs` (2); existing suites prove code/refresh flows unchanged.
- [x] Fold only this change's canonical API/overview/prose/schema deltas and README change-status updates when the implementation is complete; leave sibling proposed specs external and do not commit or push as part of planning.
  - Canonical folds committed separately (`docs(specs)`): `03-service-flows.md` (binding step, nonce issuance, decisions), `04-http-api.md` (route, request shape, discovery), `00-overview.md` (scope row, two-grants decision). No README change-status edits were made — `.specs/README.md` stays untouched per instruction, and the source change spec was neither moved nor status-flipped.

## Definition of done

- [x] Enabled `/nonce` returns exactly the specified response shape and creates a nonce usable once by the core binding flow; disabled `/nonce` is not mounted.
  - Shape asserted verbatim (43-char base64url value, `expires_in = 600` at defaults); usability proven end-to-end by the once-only direct exchange test.
- [x] With the switch disabled, an `id_token` field returns `unsupported_grant_type` for both `grant_type=id_token` and `grant_type=authorization_code`; authorization-code and refresh paths retain their existing behavior.
  - Both asserted at router level (`grants.rs`) and handler level (`routes.rs`); the pre-existing routes/e2e suites passing unmodified demonstrates retention.
- [x] Discovery accurately advertises `id_token` only in enabled deployments, and `provider_access_token` reaches the direct assertion binding path without appearing in logs or persisted state.
  - Conditional metadata asserted in both states; the wrong-at_hash refusal proves transport into the binding check. The token lives only in the parsed form and the transient binding context — no log statement, session field, or audit detail carries it.
- [x] Handler code stays boundary-only, validates form inputs before core calls, and relies on the shared router so Lambda/FFI cannot bypass the gate.
  - `nonce_handler` asserts its own mounting precondition and delegates; `token_handler` gates on validated form fields before dispatch; single `build_router` path confirmed for all runtimes.
- [x] Meets the repo definition of done (server route/E2E positive and negative tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
  - Baseline correction carried from tasks 01–04: the "three failing config tests" note is stale (fixed by merged PR #36). At this commit the workspace ran green: **456 passed / 44 skipped**, fmt + clippy (`--workspace --all-targets -D warnings`) clean. Baseline before this task: 445 passed / 44 skipped.
- [x] Reviewable: a reviewer can toggle `grants.id_token`, request discovery and `/nonce`, then prove one direct exchange succeeds and duplicate/disabled paths fail as specified.
  - `cargo nextest run -p oidc-exchange --test grants`.

## Notes

- The gate rejects on **field presence**, not `grant_type` spelling: a stray `id_token=` under `grant_type=authorization_code` is refused exactly like a declared direct grant, closing the field-presence branch-selection evasion the source spec names. When the sibling grant-type-parsing spec merges, this collapses naturally to the parse-level rejection.
- `nonce_routes()` is a separate `Router<AppState>` fragment merged by `build_router` rather than a member of `public_routes()`, keeping the static public surface honest for tests that compose it directly (existing route/E2E suites build without the nonce route unless they ask for it).
- No canonical-types.schema.json delta accompanies this task: the source spec's Type-changes block adds only `signing_alg` (folded by task 03) and `SingleUseRecord` (task 02); the nonce response shape is documented in `04-http-api.md` prose, not as a service entity.
