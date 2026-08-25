# Task 08 — Reference baseline document

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Proposed changes — Reference deployments](../../../changes/2026-08-05-baseline_reference_deployments.md#proposed-changes), [change spec §Implementation notes — B baseline document](../../../changes/2026-08-05-baseline_reference_deployments.md#implementation-notes)
**Depends on:** —
**Produces:** a short, versioned operator-facing baseline document with B1–B7 requirements that each trace to a source finding or threat-model invariant.
**Pointers:** `docs/security/reference-baseline.md` (new); `.specs/changes/2026-08-05-baseline_reference_deployments.md:101-186`; `docs/deployment/`; `examples/`

## Steps

- [ ] Create `docs/security/reference-baseline.md` with an explicit revision identifier and one concise normative entry for each B1–B7 property.
- [ ] Link each baseline property to its change-spec finding/invariant and state how a template cites its conformance revision.
- [ ] State the boundary between deployable templates and framework-integration samples, including B7’s applicability and the unresolved extra framework-hardening question.
- [ ] Ensure every prospective policy rule has a matching normative baseline entry and no document language implies the gate owns sibling changes.

## Definition of done

- [ ] The document contains B1–B7, a revision, traceability, and no unscoped scanner-only policy.
- [ ] An operator can determine required transport, secret, privilege, immutability, config, generated-state, and relying-party behavior from the document.
- [ ] Links to the change spec and referenced operator documentation resolve.
- [ ] The framework-sample boundary and sibling-owned work are explicit rather than silently absorbed.
- [ ] Meets the repo definition of done (documentation link/format checks applicable to the repository — see plan.md baseline).
- [ ] Reviewable: compare each B1–B7 line to the change spec and identify its eventual policy/test enforcement point.
