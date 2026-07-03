# Done Certificate — Task 02: document GIL release in python spec

**Task:** [02-document_gil_release_in_python_spec.md](02-document_gil_release_in_python_spec.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

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
    the bullet's wording matches `.specs/changes/merged/2026-07-01-release_gil_in_python_binding.md:41-46`
    (GIL release, re-acquire, other Python threads including an asyncio event loop keep running,
    `shutdown` no-op).
  - *Status:* ☑ SATISFIED — `03-python.md:32-37` now reads word-for-word the change spec's
    Proposed-changes block (`.specs/changes/merged/2026-07-01-release_gil_in_python_binding.md:39-46`):
    extracts method/path/headers/body from the `PyDict`, "releases the GIL (`py.allow_threads`)
    around the blocking FFI `handle_request` call, and re-acquires it to build the result dict",
    other Python threads including an asyncio event loop keep running, `shutdown` is a no-op.

- **O2 — Bullet matches shipped code and no section contradicts it; Date bumped.**
  - *Claim:* the bullet reflects the behaviour in `bindings/python/src/lib.rs` with no drift, no
    other section of the page contradicts it, and the `**Date:**` field is bumped.
  - *Evidence to collect:* diff `03-python.md`; cross-read the Implementation bullet against
    `bindings/python/src/lib.rs` (the `py.allow_threads` wrap from task 01) and confirm no claim
    the code does not deliver; scan the API and Decisions blocks for contradiction; confirm the
    `**Date:**` header value changed from `2026-06-30`.
  - *Status:* ☑ SATISFIED — `bindings/python/src/lib.rs:96-98` wraps the FFI call in
    `py.allow_threads(|| self.inner.handle_request(&method, &path, headers, body))`, and the
    result dict is built only after (`PyDict::new_bound(py)` at `lib.rs:101`, under the re-held
    GIL) — exactly what the bullet claims; the surrounding comment (`lib.rs:91-95`) states the
    asyncio-event-loop rationale the bullet echoes. `jj diff` touches only `03-python.md`; the
    Decisions block ("Async wraps sync via executor") and the API block are unchanged and
    consistent with the new bullet (the executor-offloaded sync call is the one that now drops
    the GIL). `**Date:**` bumped `2026-06-30` → `2026-07-02` (line 3).

- **O3 — Meets the repo definition of done as it applies to a prose page.**
  - *Claim:* the change carries a why in its description and every internal link on the page
    still resolves; no code gates apply to a spec-only change.
  - *Evidence to collect:* resolve each markdown link in `03-python.md` (e.g. `01-ffi-core.md`,
    `05-distribution.md`) from the page's directory and confirm each target exists; confirm the
    edit's description states why the bullet changed (per
    `.specs/development-guidelines.md` §Definition of done).
  - *Status:* ☑ SATISFIED — both internal links resolve from `.specs/bindings/specs/`:
    `01-ffi-core.md` and `05-distribution.md` exist in that directory (no other relative links on
    the page). The diff touches only `03-python.md` (no `.rs`/`.ts`/`.py` files), so the code
    fmt/lint/test gates in development-guidelines §Definition of done have no surface. The *why*
    is stated in the change spec's Motivation
    (`.specs/changes/merged/2026-07-01-release_gil_in_python_binding.md` §Motivation) and the task's
    Produces line; the jj commit description is authored by the orchestrator at commit time,
    following the per-task pattern of the preceding commits (e.g. "gil 01/02").

- **O4 — Reviewable: diff confirms the bullet matches the change spec and shipped lib.rs (Reviewable).**
  - *Claim:* a reviewer diffs `03-python.md` and confirms the Implementation bullet reads the
    `py.allow_threads` behaviour and matches both the change spec's Proposed-changes block and the
    shipped `lib.rs`.
  - *Evidence to collect:* view the `03-python.md` diff side by side with
    `.specs/changes/merged/2026-07-01-release_gil_in_python_binding.md:41-46` and
    `bindings/python/src/lib.rs`; confirm the three agree on the GIL-release behaviour.
  - *Status:* ☑ SATISFIED — exercised: viewed the `jj diff` of `03-python.md` beside the change
    spec's Proposed-changes block (lines 39-46) and `bindings/python/src/lib.rs:86-101`. The
    three agree — the diff's new bullet is the Proposed-changes wording verbatim, and every
    behavioural claim in it (allow_threads wrap, re-acquire before building the result dict,
    event loop keeps running, no-op `shutdown`) is present in the shipped `lib.rs`.

## Regression check

For each module the task touched, the validator traces one downstream caller:

- The change spec's "Affected spec pages" table names `03-python.md` as the single target →
  after the edit, expect the page's other sections (API signatures, Decisions) still describe the
  same binding surface : ☑ PRESERVED — `jj diff --stat` shows `03-python.md` is the only file
  changed (6 insertions, 3 deletions, all inside the Implementation "Rust" bullet plus the Date
  field); the API block, Decisions block ("Async wraps sync via executor"), Distribution, and
  Tests sections are untouched and still describe the same binding surface.

## Residue

Notes for the validator: the change-spec lifecycle (Status→Merged, move to
`.specs/changes/merged/`, `.specs/README.md` update) is handled by the orchestrator, not this
task — do not treat its absence as a regression. This is a prose page; no build/test gate runs.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with direct evidence — the Implementation bullet now
carries the change spec's Proposed-changes wording verbatim, matches the shipped
`py.allow_threads` behaviour in `bindings/python/src/lib.rs`, the Date is bumped, both internal
links resolve, and the diff touches nothing else on the page — so the regression check is
PRESERVED and the rubric yields DONE.
