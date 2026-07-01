# Done Certificate — Task 06: Internal API gating

**Task:** [06-internal_api_gating.md](06-internal_api_gating.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — Flag on mounts internal behind Bearer; empty secret rejected at startup.**
  - *Claim:* with `internal_api.enabled = true` and role `admin`/`all`, `/internal/*` mounts behind the constant-time Bearer check; a missing/empty secret is rejected at startup (Task 02), never at request time.
  - *Evidence to collect:* run the flag-on test — `/internal/*` reachable with the correct Bearer token, 401 without. Confirm via Task 02's `validate()` that a served empty secret aborts startup (cross-reference the served-secret obligation).
  - *Checks:* resolve the auth layer to `internal_auth_layer` (`crates/server/src/middleware/internal_auth.rs`) and confirm `Some("")` is no longer accepted as a configured secret.
  - *Status:* ☐ unverified

- **O3 — Negative-space tests and named bounds.**
  - *Claim:* tests cover the flag-off (no internal routes) and empty-secret paths; any new bound is a named constant.
  - *Evidence to collect:* enumerate the router/middleware tests; confirm one asserts no `/internal/*` when `enabled = false` and one asserts the empty-secret rejection. Grep for any introduced numeric bound as a named `const`.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: admin+off serves only `/health`; admin+on reaches `/internal/*` with token.**
  - *Claim:* a reviewer builds a router for `role = "admin"` with `enabled = false` and sees `/internal/*` → 404 while `/health` responds; then with `enabled = true` and a secret sees `/internal/*` reachable with the correct Bearer token and 401 without.
  - *Evidence to collect:* run the router integration tests exercising both configurations and observe the 404/200 and 200/401 outcomes.
  - *Status:* ☐ unverified

## Regression check

- `build_router` for `role = "exchange"`/`all` still mounts the public routes and `/health` (`crates/server/src/bootstrap.rs:119-124`) → expect unchanged public routing : ☐ (PRESERVED / REGRESSION)
- `internal_auth_layer` with a valid non-empty secret and correct token still authorizes → expect unchanged accept path : ☐ (PRESERVED / REGRESSION)

## Residue

- None noted at authoring. The `validate()` guarantee of a non-empty served secret is Task 02's obligation; this task relies on it and hardens the middleware as defence in depth.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
