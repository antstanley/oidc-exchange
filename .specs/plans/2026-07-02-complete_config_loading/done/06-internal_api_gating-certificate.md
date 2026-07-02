# Done Certificate — Task 06: Internal API gating

**Task:** [06-internal_api_gating.md](06-internal_api_gating.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 06. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** The router mounts internal routes only when `internal_api.enabled = true` and the role is `admin`/`all`; with the flag false a `role = "admin"` instance builds a router containing only `/health` (not a startup error), and `internal_auth_layer` no longer treats an empty secret as configured.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Public routes and `/health` still mount per role; the constant-time Bearer comparison in `internal_auth_layer` is unchanged for a valid secret.

## Obligations

- **O1 — Flag off mounts no internal routes; admin serves only `/health`.**
  - *Claim:* with `internal_api.enabled = false`, no `/internal/*` route is mounted for any role, and a `role = "admin"` instance serves only `/health`.
  - *Evidence to collect:* read `build_router` (`crates/server/src/bootstrap.rs:119-132`); confirm the `internal_routes` merge is gated on `internal_api.enabled == true` in addition to the role check, and that the `admin` branch still adds `/health`. Run the flag-off tests — expect a 404 for an `/internal/*` path and a response from `/health`.
  - *Checks:* trace the `role = "admin"`, `enabled = false` path and confirm `build_router` returns a router (no error) with only `/health`.
  - *Status:* ☑ SATISFIED — `crates/server/src/bootstrap.rs:304` gates the `internal_routes` merge on `(role == "admin" || role == "all") && config.internal_api.enabled`; the `role == "admin"` branch (bootstrap.rs:307-315) adds `/health` unconditionally (admin never merges `public_routes`, the only other `/health` source), so `admin` + `enabled = false` yields a router with only `/health` and no error. Tests `build_router_tests::enabled_false_admin_serves_only_health_no_internal_routes` (`/internal/stats` → 404, `/health` → 200) and `enabled_false_all_serves_public_and_health_no_internal_routes` both PASS.

- **O2 — Flag on mounts internal behind Bearer; empty secret rejected at startup.**
  - *Claim:* with `internal_api.enabled = true` and role `admin`/`all`, `/internal/*` mounts behind the constant-time Bearer check; a missing/empty secret is rejected at startup (Task 02), never at request time.
  - *Evidence to collect:* run the flag-on test — `/internal/*` reachable with the correct Bearer token, 401 without. Confirm via Task 02's `validate()` that a served empty secret aborts startup (cross-reference the served-secret obligation).
  - *Checks:* resolve the auth layer to `internal_auth_layer` (`crates/server/src/middleware/internal_auth.rs`) and confirm `Some("")` is no longer accepted as a configured secret.
  - *Status:* ☑ SATISFIED — flag-on tests `build_router_tests::enabled_true_admin_mounts_internal_behind_bearer_auth` (correct Bearer → 200, missing → 401) and `enabled_true_all_mounts_public_and_internal` PASS. Resolution: `build_router` → `routes::internal_routes` (routes/mod.rs:25) → `internal::router` (routes/internal.rs:16), which layers `middleware::from_fn_with_state(state, internal_auth_layer)` (internal.rs:31) with `internal_auth_layer` imported from `crate::middleware::internal_auth` (internal.rs:11) — no shadowing. `internal_auth.rs:25-26` now matches `Some(s) if !s.is_empty()`, so `Some("")` falls to the "not configured" 401 arm. Task 02 cross-reference: `AppConfig::validate` (`crates/core/src/config.rs:69-85`) returns a `ConfigError` when the internal API is served with a missing/empty `shared_secret`, so an empty secret aborts startup.

- **O3 — Negative-space tests and named bounds.**
  - *Claim:* tests cover the flag-off (no internal routes) and empty-secret paths; any new bound is a named constant.
  - *Evidence to collect:* enumerate the router/middleware tests; confirm one asserts no `/internal/*` when `enabled = false` and one asserts the empty-secret rejection. Grep for any introduced numeric bound as a named `const`.
  - *Status:* ☑ SATISFIED — flag-off negative-space tests: `enabled_false_admin_serves_only_health_no_internal_routes` and `enabled_false_all_serves_public_and_health_no_internal_routes` (bootstrap.rs `build_router_tests`) assert `/internal/stats` → 404. Empty-secret tests: `build_router_tests::empty_shared_secret_is_never_accepted_as_configured` and `internal_auth_rejects_empty_configured_secret_even_with_empty_bearer_token` (`crates/server/tests/internal.rs`) both assert 401 with "internal API not configured" even against an empty `Bearer ` token. No new numeric bound was introduced; the only new constant is the named test secret `TEST_SECRET`.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all -- --check` clean; `cargo clippy --workspace -- -D warnings` finished with no warnings; `cargo nextest run --workspace` → 191 tests run: 191 passed, 2 skipped.

- **O5 — Reviewable: admin+off serves only `/health`; admin+on reaches `/internal/*` with token.**
  - *Claim:* a reviewer builds a router for `role = "admin"` with `enabled = false` and sees `/internal/*` → 404 while `/health` responds; then with `enabled = true` and a secret sees `/internal/*` reachable with the correct Bearer token and 401 without.
  - *Evidence to collect:* run the router integration tests exercising both configurations and observe the 404/200 and 200/401 outcomes.
  - *Status:* ☑ SATISFIED — exercised via `cargo nextest run -p oidc-exchange`: `enabled_false_admin_serves_only_health_no_internal_routes` builds `role = "admin"` + `enabled = false` and observes `/internal/stats` → 404 with `/health` → 200; `enabled_true_admin_mounts_internal_behind_bearer_auth` builds `role = "admin"` + `enabled = true` + secret and observes `/internal/stats` → 200 with the correct Bearer token and → 401 without. Both PASS.

## Regression check

- `build_router` for `role = "exchange"`/`all` still mounts the public routes and `/health` (`crates/server/src/bootstrap.rs:119-124`) → expect unchanged public routing : ☑ PRESERVED — the `exchange`/`all` public-routes branch (now bootstrap.rs:301-303) is untouched by the diff; `enabled_false_all_serves_public_and_health_no_internal_routes` asserts `/health` and `/keys` → 200, and the full-workspace e2e suite (incl. `e2e_internal_api_custom_claims`) passes.
- `internal_auth_layer` with a valid non-empty secret and correct token still authorizes → expect unchanged accept path : ☑ PRESERVED — the constant-time `ct_eq` accept arm (internal_auth.rs:42-45) is unchanged; only the "configured" guard tightened from `Some(s)` to `Some(s) if !s.is_empty()`. `internal_auth_passes_with_correct_secret` PASSES.

## Residue

- None noted at authoring. The `validate()` guarantee of a non-empty served secret is Task 02's obligation; this task relies on it and hardens the middleware as defence in depth.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with code evidence (bootstrap.rs:304-315, internal_auth.rs:25-26, core/config.rs:69-85) and passing targeted tests (10/10) plus a clean workspace suite (191 passed), fmt, and clippy; both regression traces PRESERVED.
