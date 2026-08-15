# Task 05 — HTTP grant surface

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted at user request

**Implements:** [`.specs/changes/2026-08-05-bind_id_token_grant_replay_protection.md` §Proposed changes → `00-overview.md` and `04-http-api.md`](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md), [§Implementation notes 7–8](../../../changes/2026-08-05-bind_id_token_grant_replay_protection.md); [00-overview.md §Scope summary and §Decisions](../../../service/specs/00-overview.md), [04-http-api.md §Routes, §POST /token request, and §GET /.well-known/openid-configuration](../../../service/specs/04-http-api.md)
**Depends on:** 01 (contract), 04 (review)
**Produces:** A deployment with `grants.id_token = true` can mint a nonce, submit the direct grant with an optional provider access token, and discover the grant; the default-disabled deployment has no nonce route and rejects any `id_token` field with `unsupported_grant_type`.
**Pointers:** `crates/server/src/routes/mod.rs:2-23`, new `crates/server/src/routes/nonce.rs`, `crates/server/src/routes/token.rs:14-65`, `crates/server/src/routes/well_known.rs:8-21`, `crates/server/src/bootstrap.rs:326-381`, `crates/server/tests/routes.rs:26-55`, `crates/server/tests/e2e.rs`, `crates/ffi/src/lib.rs:69`

## Steps

- [ ] Add an unauthenticated `POST /nonce` handler that delegates minting to core, returns a base64url nonce and `expires_in`, and is mounted only in exchange-serving roles when `grants.id_token` is enabled.
- [ ] Extend form parsing and `ExchangeRequest` construction for optional `provider_access_token`; before grant dispatch, reject any request carrying `id_token` when the feature switch is disabled, regardless of declared `grant_type`.
- [ ] Make discovery add `id_token` to `grant_types_supported` only when enabled, while preserving authorization-code and refresh grant advertisement in either state.
- [ ] Confirm server, Lambda, and FFI continue to use the shared router path and that disabled routing does not create a nonce endpoint in any exchange-capable runtime.
- [ ] Add route/E2E tests for enabled nonce issuance and one-time direct exchange, disabled nonce 404, conditional discovery metadata, enabled and disabled form shapes, and both off-switch bypass attempts named by the source spec.
- [ ] Fold only this change's canonical API/overview/prose/schema deltas and README change-status updates when the implementation is complete; leave sibling proposed specs external and do not commit or push as part of planning.

## Definition of done

- [ ] Enabled `/nonce` returns exactly the specified response shape and creates a nonce usable once by the core binding flow; disabled `/nonce` is not mounted.
- [ ] With the switch disabled, an `id_token` field returns `unsupported_grant_type` for both `grant_type=id_token` and `grant_type=authorization_code`; authorization-code and refresh paths retain their existing behavior.
- [ ] Discovery accurately advertises `id_token` only in enabled deployments, and `provider_access_token` reaches the direct assertion binding path without appearing in logs or persisted state.
- [ ] Handler code stays boundary-only, validates form inputs before core calls, and relies on the shared router so Lambda/FFI cannot bypass the gate.
- [ ] Meets the repo definition of done (server route/E2E positive and negative tests, `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, and applicable `cargo nextest` tests; record but do not fix the known unrelated three config-test baseline failures).
- [ ] Reviewable: a reviewer can toggle `grants.id_token`, request discovery and `/nonce`, then prove one direct exchange succeeds and duplicate/disabled paths fail as specified.
