# Done Certificate — Task 02: document GIL release in python spec

**Task:** [02-document_gil_release_in_python_spec.md](02-document_gil_release_in_python_spec.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location or a diff) — not by assertion.

## Premises

- **P1 — Goal.** The canonical `03-python.md` Implementation bullet describes the GIL release
  delivered by task 01, so the page and the code agree.
- **P2 — Obligations.** The task is done iff O1…O4 all hold. One Oi per definition-of-done item,
  in DoD order; O4 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the rest of `03-python.md` — the API block, the Decisions
  block ("Async wraps sync via executor"), and the page's internal links — which stay consistent
  with the revised Implementation bullet.

## Obligations

- **O1 — Implementation bullet states the `py.allow_threads` release and re-acquisition.**
  - *Claim:* the "Rust (`src/lib.rs`)" bullet in `03-python.md` §Implementation says
    `handle_request_sync` releases the GIL (`py.allow_threads`) around the blocking FFI
    `handle_request` call and re-acquires it to build the result dict, matching the change spec's
    Proposed-changes block.
  - *Evidence to collect:* read `.specs/bindings/specs/03-python.md` around lines 32-34; confirm
    the bullet's wording matches `.specs/changes/2026-07-01-release_gil_in_python_binding.md:41-46`
    (GIL release, re-acquire, other Python threads including an asyncio event loop keep running,
    `shutdown` no-op).
  - *Status:* ☐ unverified

- **O2 — Bullet matches shipped code and no section contradicts it; Date bumped.**
  - *Claim:* the bullet reflects the behaviour in `bindings/python/src/lib.rs` with no drift, no
    other section of the page contradicts it, and the `**Date:**` field is bumped.
  - *Evidence to collect:* diff `03-python.md`; cross-read the Implementation bullet against
    `bindings/python/src/lib.rs` (the `py.allow_threads` wrap from task 01) and confirm no claim
    the code does not deliver; scan the API and Decisions blocks for contradiction; confirm the
    `**Date:**` header value changed from `2026-06-30`.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done as it applies to a prose page.**
  - *Claim:* the change carries a why in its description and every internal link on the page
    still resolves; no code gates apply to a spec-only change.
  - *Evidence to collect:* resolve each markdown link in `03-python.md` (e.g. `01-ffi-core.md`,
    `05-distribution.md`) from the page's directory and confirm each target exists; confirm the
    edit's description states why the bullet changed (per
    `.specs/development-guidelines.md` §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: diff confirms the bullet matches the change spec and shipped lib.rs (Reviewable).**
  - *Claim:* a reviewer diffs `03-python.md` and confirms the Implementation bullet reads the
    `py.allow_threads` behaviour and matches both the change spec's Proposed-changes block and the
    shipped `lib.rs`.
  - *Evidence to collect:* view the `03-python.md` diff side by side with
    `.specs/changes/2026-07-01-release_gil_in_python_binding.md:41-46` and
    `bindings/python/src/lib.rs`; confirm the three agree on the GIL-release behaviour.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- The change spec's "Affected spec pages" table names `03-python.md` as the single target →
  after the edit, expect the page's other sections (API signatures, Decisions) still describe the
  same binding surface : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the change-spec lifecycle (Status→Merged, move to
`.specs/changes/merged/`, `.specs/README.md` update) is handled by the orchestrator, not this
task — do not treat its absence as a regression. This is a prose page; no build/test gate runs.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
