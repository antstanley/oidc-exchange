# Task 09 — Canonical spec merge

**Plan:** [plan.md](../plan.md) · **Certificate:** omitted by requester

**Implements:** [change spec §Affected spec pages](../../../changes/2026-08-05-baseline_reference_deployments.md#affected-spec-pages), [change spec §Merge plan](../../../changes/2026-08-05-baseline_reference_deployments.md#merge-plan), [bindings distribution](../../../bindings/specs/05-distribution.md), [service persistence](../../../service/specs/08-persistence.md), [service configuration](../../../service/specs/06-configuration.md), [architecture principles](../../../architecture-principles.md)
**Depends on:** —
**Produces:** canonical pages that normatively describe the merged reference-deployment baseline, associated deployment behavior, and CI contract.
**Pointers:** `.specs/bindings/specs/05-distribution.md:26-37`; `.specs/service/specs/08-persistence.md:78-145`; `.specs/service/specs/06-configuration.md:77-80`; `.specs/architecture-principles.md:69-97`; `.specs/changes/2026-08-05-baseline_reference_deployments.md:97-346,620-640`

## Steps

- [ ] Apply the Reference deployments, Docker, CI, and closing-block changes to the distribution spec after the release-pipeline section.
- [ ] Apply the Postgres, SQLite, Valkey, and single-writer assumption changes to persistence; mark Valkey URLs secret-bearing in configuration and add the shipped-TOML CI assumption.
- [ ] Mark `examples/` as a gated product surface in architecture principles and ensure the new document is described as the operator-facing rendering.
- [ ] Update canonical page dates to the merge date and reconcile all references with the actual implemented task outcomes.
- [ ] Update the change header/index only when all required remediation and blocking gate work is merged; do not merge or rewrite sibling specs.

## Definition of done

- [ ] Every canonical target and affected heading listed in the change spec reflects the final implemented behavior, including assumptions/decisions/open questions.
- [ ] The specification differentiates normative baseline requirements from operator documentation and implementation mechanics.
- [ ] Cross-links resolve and no canonical page claims an unimplemented sibling feature as part of this change.
- [ ] Change-spec status/index housekeeping occurs only at the stated merge condition.
- [ ] Meets the repo definition of done (documentation link/format checks applicable to the repository — see plan.md baseline).
- [ ] Reviewable: compare the changed canonical headings with the source change-spec blocks and the landed implementation.
