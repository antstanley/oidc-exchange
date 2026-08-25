# Task 01 — Index 2026-08-05 change specs

**Plan:** [plan.md](../plan.md) · **Certificate:** Not authored by request.

**Implements:** [PR #28 index-only intent](../plan.md#source-and-definition-of-done-baseline): index the fourteen 2026-08-05 change-spec proposals in `.specs/README.md` `## Change specs`.
**Depends on:** —
**Produces:** the canonical index lists each intended 2026-08-05 change-spec proposal once, with a relative link that resolves from `.specs/README.md` when the source spec set is present.
**Pointers:** `.specs/README.md:28-50`; branch diff `trunk()..spec/index-2026-08-05-change-specs`.

## Steps

- [x] Add the fourteen `changes/2026-08-05-*.md` rows from the branch diff to the Proposed portion of `.specs/README.md` `## Change specs`, after the existing 2026-06-24 proposed rows and before merged entries. (Rows sit at `.specs/README.md:37-50`, between the two 2026-06-24 proposed rows and the first merged entry.)
- [x] Preserve the exact row labels, status `Proposed`, target summaries, and `changes/...` relative-link form from the branch diff. (Every row keeps the `| [changes/<name>.md](changes/<name>.md) | Proposed | … |` form; verified row-for-row against `jj diff --from ba28c264 --to 7d8fefb0`.)
- [x] Do not add, edit, or implement any change spec, source code, configuration, workflow, or admin JWT behavior. (Branch diff touches only `.specs/README.md`, plan.md, and this task file; `.specs/changes/` still holds only the pre-existing 2026-06-24 specs.)
- [x] Inspect the rendered Markdown/table structure and validate each planned target path against the intended source spec set, including intentional unresolved links and the absence of certificates. (Rows are well-formed three-column Markdown; none of the fourteen targets exists in `.specs/changes/` in this checkout as planned; no certificate artifacts exist in the plan package.)
- [x] Confirm the plan DAG matches the single-task review scope and remains index-only. (plan.md task graph and dependency table both list exactly one node — `01 · index change specs` — with no dependencies.)

## Definition of done

- [x] `.specs/README.md` contains exactly the fourteen 2026-08-05 entries in the branch diff, each once and each marked `Proposed`. (Fourteen distinct `changes/2026-08-05-*.md` paths, fourteen rows, all `Proposed`; the fifteenth branch-diff README line is this plan's own row in the plans index.)
- [x] Each new `changes/2026-08-05-*.md` relative link has valid Markdown syntax and targets the expected source path when the intended spec set is present; unresolved links are documented as expected in this checkout. (Link text equals its relative target path resolving from `.specs/`; unresolved targets are declared intentional in plan.md assumptions.)
- [x] The diff is confined to the index entry update; it contains no admin JWT implementation work or other change-spec/source changes. (`jj diff --from ba28c264 --to 7d8fefb0 --stat`: three docs files only, +98/-0.)
- [x] Meets the documentation-only portion of the repo definition of done: repository hygiene is preserved and changed Markdown links are checked (see plan.md baseline). (Docs-only change; workspace clean; `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo nextest run --workspace` all green.)
- [x] Reviewable: compare `.specs/README.md` with the branch diff and follow the fourteen indexed links in the intended spec set. (Row-for-row comparison against the branch diff performed at filing; independent review of the branch returned CORRECT with no substantive findings.)
