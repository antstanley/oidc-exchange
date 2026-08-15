# 02 · Lifecycle and admin canonical pages

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Certificate:** intentionally omitted (planning backlog only)

**Implements:** [2026-07-01-enforce_user_lifecycle_transitions.md](../../../changes/merged/2026-07-01-enforce_user_lifecycle_transitions.md) — all `Proposed changes` blocks targeting [01-domain-model.md](../../../service/specs/01-domain-model.md), [03-service-flows.md](../../../service/specs/03-service-flows.md), and [04-http-api.md](../../../service/specs/04-http-api.md).
**Depends on:** —
**Produces:** canonical lifecycle transition/revocation rules and corresponding admin HTTP/not-found documentation.

## Steps

- [ ] Verify `01-domain-model.md` has the `Suspended → Deleted` edge, suspend/delete session revocation, terminal Deleted behavior, and same-status no-op exception.
- [ ] Verify `03-service-flows.md` documents validated admin transitions, session revocation on qualifying changes, and `NotFound` for unknown users/claims mutations.
- [ ] Verify `04-http-api.md` annotates all affected internal user/claims routes with 404-if-absent behavior and maps `NotFound` to `404 not_found`.
- [ ] Verify only the three owned pages are touched for this task and their metadata dates are `2026-08-05`.

## Definition of done

- [ ] All lifecycle source blocks are represented with equivalent semantics, including the diagram, lifecycle/session/decision prose, admin operations, internal routes, and error mapping.
- [ ] Negative-space documentation is explicit: off-diagram transitions and every patch on Deleted are rejected; Deleted-to-Deleted is not a no-op; unknown IDs receive `NotFound`/404.
- [ ] Every local and source link resolves, including the service-flows lifecycle link and the plan/source links above.
- [ ] No code, schema, change-spec, README-index, certificate, or unrelated canonical-page changes are introduced.
