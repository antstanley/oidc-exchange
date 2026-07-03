# Done Certificate — Task 02: base_path strip layer

**Task:** [02-base_path_strip_layer.md](02-base_path_strip_layer.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
  - *Status:* ☑ SATISFIED — `strip_base_path` (`crates/server/src/middleware/base_path.rs:81`)
    early-returns the request unmodified when the prefix is `None`/empty (line 90-92) and rewrites
    the URI path only on a boundary match (lines 94-145). The wrapper `with_base_path_strip`
    (line 34) builds an outer routeless `Router` whose `fallback_service` strips the prefix
    *before* dispatching into the inner router (`BasePathStripService::call`, line 65-72), so the
    rewrite precedes axum's routing decision — proven by the `oneshot` unit test
    `with_base_path_strip_changes_the_routing_decision_not_just_the_handler_view` (PASS) and the
    E2E test `prefixed_request_routes_to_health_when_base_path_configured` → `/prod/health` → 200
    (PASS). With `base_path = None`, `unprefixed_request_routes_to_health_when_base_path_unset`
    → `/health` → 200 (PASS) and no rewrite occurs.

- **O2 — Negative space: no double-strip, no mismatched-prefix false route.**
  - *Claim:* with `base_path = Some("/prod")`, a request lacking the prefix (`GET /health`) is not
    double-stripped, and a path like `/production...` is not treated as prefixed.
  - *Evidence to collect:* run the negative-space E2E assertions — a no-prefix request resolves per
    the boundary rule (not a false 200 that implies a rewrite), and a `/production`-style path is
    left unmodified. Trace the strip function on input `/production/x` with prefix `/prod` and
    confirm it does not strip (segment-boundary check).
  - *Status:* ☑ SATISFIED — `strip_prefix_at_segment_boundary` (line 157) only returns `Some`
    when the byte after the shared prefix is `/` or end-of-string. Traced on `("/production/x",
    "/prod")`: `str::strip_prefix` yields `"uction/x"`, which is non-empty and does not start
    with `/`, so the helper returns `None` and `strip_base_path` returns the request untouched.
    E2E: `request_without_prefix_is_not_double_stripped` → `/health` (base_path `/prod`) → 200
    (unmangled, resolves as the real unprefixed route, not a substring-chopped garbage path);
    `mismatched_sibling_prefix_is_not_falsely_routed` → `/production/health` → 404 (PASS). No
    double-strip, no false route.

- **O3 — Single shared path, defensively asserted.**
  - *Claim:* the layer is applied once on the shared `build_router` path (no Lambda-only fork) and
    the layer function carries ≥2 meaningful assertions.
  - *Evidence to collect:* read `bootstrap.rs:109-138` and confirm the layer is added inside
    `build_router`, not duplicated in a mode-specific branch; count ≥2 assertions in the strip
    function (e.g. prefix-boundary precondition, post-rewrite path invariant).
  - *Status:* ☑ SATISFIED — `with_base_path_strip` is called exactly once, at
    `crates/server/src/bootstrap.rs:362`, inside `build_router` (the single shared router
    builder), unconditionally (no `None`-only shortcut, no Lambda-only branch). Both runtimes go
    through `build_router`: the hyper entry at `crates/server/src/main.rs:27`, and the Lambda
    runtime (task 01) shares the same builder. The strip core `strip_base_path` carries two
    post-rewrite invariant assertions — `assert!(!new_uri.path().is_empty(), …)` (line 128) and
    `assert_ne!(new_uri.path(), path, …)` (line 138) — plus two `unwrap_or_else` panic-guards on
    URI reconstruction (lines 114, 120). ≥2 meaningful assertions confirmed.

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests pass, format and clippy clean, any bound is a named constant.
  - *Evidence to collect:* run the repo's Rust gates from
    [.specs/development-guidelines.md](../../../development-guidelines.md) §Definition of done —
    `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run --workspace` — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo nextest run --workspace`: 382 passed, 0 failed (27 skipped).
    `cargo clippy --workspace -- -D warnings`: clean (finished, no warnings). `cargo fmt --all
    --check`: clean (exit 0, no diff). No new numeric bound was introduced by this task (the
    strip is a pure segment-boundary rewrite), so the named-constant rule is not triggered.

- **O5 — Reviewable: base-path routing exercised end to end (Reviewable).**
  - *Claim:* a reviewer runs the base-path E2E test and sees `/prod/health` → 200 with
    `base_path = "/prod"` and the no-prefix mismatch case not falsely routed.
  - *Evidence to collect:* run the server-crate E2E base-path test and observe the `/prod/health`
    success case and the mismatched/no-prefix negative case reported as passed.
  - *Status:* ☑ SATISFIED — ran the server-crate E2E suite `crates/server/tests/base_path.rs`
    over the real `build_router` output via `oneshot`. Observed: `/prod/health` → 200 and
    `/prod/keys` → 200 with `base_path = "/prod"` (`prefixed_request_routes_to_health_when_
    base_path_configured` PASS); the no-prefix `/health` → 200 unmangled
    (`request_without_prefix_is_not_double_stripped` PASS); the mismatched sibling
    `/production/health` → 404 (`mismatched_sibling_prefix_is_not_falsely_routed` PASS); the bare
    prefix `/prod` → 404 (`bare_prefix_request_strips_to_root` PASS). Mismatch/no-prefix cases are
    not falsely routed.

## Regression check

- The existing public-route E2E test `crates/server/tests/routes.rs` builds the router with a
  default `AppConfig` (`base_path = None`) and drives `oneshot`: expect its route assertions
  (e.g. `GET /health`, `POST /token`) still hold with the strip layer present but inactive :
  ☑ PRESERVED — the full `crates/server/tests/routes.rs` suite passes unchanged (`routes
  health_returns_200`, `keys_returns_jwks`, `well_known_returns_discovery_doc`, `token_exchange_
  returns_200_with_access_token`, the `revoke_*` set, etc. all PASS). With the default
  `base_path = None` the outer wrapper runs but `strip_base_path` is a pure pass-through, so the
  existing middleware stack (request-id, audit-context, catch-panic, timeout) and role-based
  route merging are unaffected.

## Residue

Notes for the validator: prefix normalization (leading/trailing slash handling) is the layer's own
concern; confirm the boundary rule is implemented as whole-segment matching rather than raw
`strip_prefix` on the string, which would mis-handle `/prod` vs `/production`. Not a separate
obligation but the key correctness risk.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence — a single shared `with_base_path_strip` wrapper in
`build_router` (bootstrap.rs:362) rewrites the URI path at a whole-segment boundary before axum
routes, proven by 8 unit tests and 5 E2E tests (`/prod/health` → 200, `/production/health` → 404,
`/prod` → 404, `/health` → 200 unmangled); gates clean (382 tests, clippy, fmt); the existing
`routes.rs` E2E suite is PRESERVED.
