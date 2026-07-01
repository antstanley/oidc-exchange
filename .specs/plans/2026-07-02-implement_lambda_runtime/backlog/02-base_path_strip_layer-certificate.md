# Done Certificate — Task 02: base_path strip layer

**Task:** [02-base_path_strip_layer.md](02-base_path_strip_layer.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The task adds a single shared tower layer to `build_router` that strips a
  configured `server.base_path` prefix from request paths before routing, so a request to
  `/prod/health` routes to the health handler when `base_path = "/prod"`.
- **P2 — Obligations.** The task is done iff O1…O5 all hold. One Oi per definition-of-done item,
  in DoD order; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the existing middleware stack or routing in
  `build_router` (`crates/server/src/bootstrap.rs:109-138`) — request-id, audit-context, and
  catch-panic layers, role-based route merging, and unprefixed routing when `base_path` is `None`;
  the existing E2E tests in `crates/server/tests/routes.rs` must still pass.

## Obligations

- **O1 — Prefix stripped when set, paths unchanged when unset.**
  - *Claim:* with `base_path = Some("/prod")`, `GET /prod/health` routes to the health handler
    (200); with `base_path = None`, `GET /health` is unchanged (200) and no rewrite occurs.
  - *Evidence to collect:* read the strip layer in `crates/server/src/bootstrap.rs` and confirm it
    rewrites the request URI path only when `config.server.base_path` is `Some`; run the new
    server-crate E2E base-path test — expect the `/prod/health` → 200 and `base_path = None`
    `/health` → 200 cases PASS.
  - *Checks:* trace the layer over the router — confirm it runs before route matching (wraps the
    router), so the rewritten path is what axum matches against.
  - *Status:* ☐ unverified

- **O2 — Negative space: no double-strip, no mismatched-prefix false route.**
  - *Claim:* with `base_path = Some("/prod")`, a request lacking the prefix (`GET /health`) is not
    double-stripped, and a path like `/production...` is not treated as prefixed.
  - *Evidence to collect:* run the negative-space E2E assertions — a no-prefix request resolves per
    the boundary rule (not a false 200 that implies a rewrite), and a `/production`-style path is
    left unmodified. Trace the strip function on input `/production/x` with prefix `/prod` and
    confirm it does not strip (segment-boundary check).
  - *Status:* ☐ unverified

- **O3 — Single shared path, defensively asserted.**
  - *Claim:* the layer is applied once on the shared `build_router` path (no Lambda-only fork) and
    the layer function carries ≥2 meaningful assertions.
  - *Evidence to collect:* read `bootstrap.rs:109-138` and confirm the layer is added inside
    `build_router`, not duplicated in a mode-specific branch; count ≥2 assertions in the strip
    function (e.g. prefix-boundary precondition, post-rewrite path invariant).
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, any bound is a named constant.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: base-path routing exercised end to end (Reviewable).**
  - *Claim:* a reviewer runs the base-path E2E test and sees `/prod/health` → 200 with
    `base_path = "/prod"` and the no-prefix mismatch case not falsely routed.
  - *Evidence to collect:* run the server-crate E2E base-path test and observe the `/prod/health`
    success case and the mismatched/no-prefix negative case reported as passed.
  - *Status:* ☐ unverified

## Regression check

- The existing public-route E2E test `crates/server/tests/routes.rs` builds the router with a
  default `AppConfig` (`base_path = None`) and drives `oneshot`: expect its route assertions
  (e.g. `GET /health`, `POST /token`) still hold with the strip layer present but inactive : ☐
  (PRESERVED / REGRESSION)

## Residue

Notes for the validator: prefix normalization (leading/trailing slash handling) is the layer's own
concern; confirm the boundary rule is implemented as whole-segment matching rather than raw
`strip_prefix` on the string, which would mis-handle `/prod` vs `/production`. Not a separate
obligation but the key correctness risk.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
