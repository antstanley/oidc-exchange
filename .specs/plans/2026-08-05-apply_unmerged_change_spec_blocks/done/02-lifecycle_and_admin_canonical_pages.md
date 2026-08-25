# 02 · Lifecycle and admin canonical pages

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Certificate:** intentionally omitted

**Implements:** [2026-07-01-enforce_user_lifecycle_transitions.md](../../../changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md) — all `Proposed changes` blocks targeting [01-domain-model.md](../../../service/specs/01-domain-model.md), [03-service-flows.md](../../../service/specs/03-service-flows.md), and [04-http-api.md](../../../service/specs/04-http-api.md).
**Depends on:** —
**Produces:** canonical lifecycle transition/revocation rules and corresponding admin HTTP/not-found documentation.

## Steps

- [x] Verify `01-domain-model.md` has the `Suspended → Deleted` edge, suspend/delete session revocation, terminal Deleted behavior, and same-status no-op exception.
- [x] Verify `03-service-flows.md` documents validated admin transitions, session revocation on qualifying changes, and `NotFound` for unknown users/claims mutations.
- [x] Verify `04-http-api.md` annotates all affected internal user/claims routes with 404-if-absent behavior and maps `NotFound` to `404 not_found`.
- [x] Verify only the three owned pages are touched for this task and their metadata dates are `2026-08-05`.

## Definition of done

- [x] All lifecycle source blocks are represented with equivalent semantics, including the diagram, lifecycle/session/decision prose, admin operations, internal routes, and error mapping.
- [x] Negative-space documentation is explicit: off-diagram transitions and every patch on Deleted are rejected; Deleted-to-Deleted is not a no-op; unknown IDs receive `NotFound`/404.
- [x] Every local and source link resolves, including the service-flows lifecycle link and the plan/source links above.
- [x] No code, schema, change-spec, README-index, certificate, or unrelated canonical-page changes are introduced.

## Execution evidence

- The initial PR documentation diff already contains the lifecycle blocks. Integration rechecked the terminal `Deleted` behavior, the non-terminal same-status no-op rule, suspension/deletion session revocation, the affected 404 route annotations, and `NotFound` mapping.
- The canonical-page dates, local Markdown targets, and branch path scope were revalidated; no canonical remediation was required.
