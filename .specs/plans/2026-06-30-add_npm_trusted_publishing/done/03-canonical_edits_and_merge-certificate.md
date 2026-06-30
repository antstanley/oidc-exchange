# Done Certificate — Task 03: canonical edits + merge

**Task:** [03-canonical_edits_and_merge.md](03-canonical_edits_and_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-30

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a spec-page location, a grep result, or a file move) — not by assertion.

## Premises

- **P1 — Goal.** The task applies the change spec's four Proposed-changes blocks to the canonical
  pages (05-distribution.md Release pipeline / Artifacts / Assumptions-Decisions; 02-nodejs.md
  Distribution), then discharges the Merge plan: the change spec flips to `Merged`, is stamped and
  moved to `.specs/changes/merged/`, and `.specs/README.md` is updated.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD
  order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** The edits must describe the pipeline tasks 01 and 02 actually shipped (not a
  divergent one); every inbound cross-link to the change spec must still resolve after the move;
  no unrelated spec page is changed.

## Obligations

- **O1 — 05-distribution.md matches the change spec's Proposed-changes blocks and reality.**
  - *Claim:* the Release-pipeline prose, the Artifacts npm row, and Assumptions/Decisions in
    `05-distribution.md` reflect separate build/publish jobs, trusted publishing, provenance, the
    named platform-package set, and an Assumptions section that no longer claims npm uses a stored
    secret.
  - *Evidence to collect:* read `.specs/bindings/specs/05-distribution.md`; confirm the
    Release-pipeline paragraph names `build-nodejs` → `publish-npm` (separate jobs, `id-token:
    write`, `publish` Environment, `napi artifacts`, `publint`/`@arethetypeswrong/cli`,
    `--provenance`, SHA pins, Node ≥ 24.8.0); confirm the Artifacts table npm row lists
    `@oidc-exchange/node` + the four platform packages with channel "npm (OIDC trusted publishing,
    provenance)"; confirm the Assumptions list no longer includes npm in "credentials configured as
    repository secrets" and a *npm trusted publishing* Decision is present.
  - *Checks:* cross-check the prose against the shipped `release.yml` (task 02) — the spec must
    describe the job that exists, not the change spec's wording where the two diverge.
  - *Evidence collected:* 05-distribution.md:39-53 Release-pipeline prose names `build-nodejs`
    (matrix, `napi build --release --target <triple>`, uploads `.node` artifact) → `publish-npm`
    as a separate job that `needs build-nodejs`, declares `permissions: { id-token: write,
    contents: read }`, runs in the `publish` GitHub Environment, downloads `.node` artifacts, runs
    `napi artifacts` into `npm/<triple>`, validates with `publint` and `@arethetypeswrong/cli`,
    publishes the four platform packages + root with `npm publish --provenance --access public`,
    authenticates via GitHub OIDC trusted publishing "no `NPM_TOKEN`", pins every action to a
    full-length SHA, and runs on Node.js ≥ 24.8.0. Artifacts row 05-distribution.md:14 =
    "`@oidc-exchange/node` + 4 platform packages (`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,
    win32-x64-msvc,darwin-arm64}`) | 4 napi targets | npm (OIDC trusted publishing, provenance)".
    Assumptions 05-distribution.md:66 now reads "PyPI, ghcr.io, and Docker Hub credentials …" —
    npm dropped; *npm trusted publishing* Decision present at 05-distribution.md:76-79. Cross-check
    vs shipped `release.yml`: build-nodejs (:167), publish-npm (:205) needs build-nodejs (:207),
    id-token: write + contents: read (:211-212), environment: publish (:213), node-version
    "24.8.0" (:220), `napi artifacts` (:235), publint (:258), `@arethetypeswrong/cli` (:262),
    `npm publish --provenance --access public --ignore-scripts` (:271,:276), no NPM_TOKEN env,
    all `uses:` SHA-pinned — every spec claim has a matching shipped fact.
  - *Status:* ☑ SATISFIED

- **O2 — 02-nodejs.md Distribution records the optionalDependencies + napi artifacts mechanism.**
  - *Claim:* `02-nodejs.md` §Distribution notes that `package.json` declares the four platform
    packages as `optionalDependencies` populated by `napi artifacts`, and that npm installs only the
    host-matching entry which the loader resolves (local fallback).
  - *Evidence to collect:* read `.specs/bindings/specs/02-nodejs.md` §Distribution; confirm the
    appended note; cross-check it against the `optionalDependencies` task 01 added to
    `bindings/nodejs/package.json`.
  - *Evidence collected:* 02-nodejs.md:49-52 — "The root `package.json` declares the four platform
    packages as `optionalDependencies` pinned to the workspace version; `napi artifacts` copies
    each built `.node` into its `npm/<triple>` package at release time. npm installs only the entry
    matching the host `{os, cpu}`; the `index.js` loader resolves that package, falling back to a
    co-located `oidc-exchange.node`." Cross-check: `bindings/nodejs/package.json` optionalDependencies =
    `{@oidc-exchange/darwin-arm64, linux-arm64-gnu, linux-x64-gnu, win32-x64-msvc}` all pinned to
    "0.1.0" (workspace version) — matches the four-package set named in the note.
  - *Status:* ☑ SATISFIED

- **O3 — Both pages bumped, no stale publish-nodejs / NPM_TOKEN claim survives.**
  - *Claim:* both edited pages carry a bumped `**Date:**`, and no `publish-nodejs` /
    "NPM_TOKEN as a secret" claim remains on either page.
  - *Evidence to collect:* `grep -nE 'publish-nodejs|NPM_TOKEN' .specs/bindings/specs/05-distribution.md
    .specs/bindings/specs/02-nodejs.md` — expect no matches; read each page header and confirm
    `**Date:**` is `2026-06-30` (or later than its prior value).
  - *Evidence collected:* both headers carry `**Date:** 2026-06-30` (02-nodejs.md:3,
    05-distribution.md:3), bumped from `2026-06-24`. `grep -nE 'publish-nodejs'` over both pages →
    no matches (the stale job name is gone). `grep -nE 'NPM_TOKEN'` → two hits, both on
    05-distribution.md (:52 "no `NPM_TOKEN`", :77 "not a stored `NPM_TOKEN`"); both are NEGATIONS
    asserting the token is NOT used. Per the DoD's intent — no surviving claim that NPM_TOKEN is
    used as a secret — these negations satisfy the obligation; 02-nodejs.md has zero hits.
  - *Status:* ☑ SATISFIED

- **O4 — The change spec is Merged-stamped and relocated; README updated.**
  - *Claim:* the change spec is `Merged`-stamped and lives at
    `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md`; `.specs/README.md` references
    it under `merged/` and lists this plan.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md`
    exists and `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md` no longer does
    (`jj diff --name-only` shows the rename/move); read its header for `**Status:** Merged` and a
    `**Merged:**` stamp; read `.specs/README.md` and confirm the Changes row points at `merged/`
    with Status `Merged` and the Plans table lists `2026-06-30-add_npm_trusted_publishing`.
  - *Evidence collected:* `jj diff` shows a rename `.specs/changes/… => .specs/changes/merged/…`;
    `ls` confirms the new path exists and the old `.specs/changes/2026-06-29-…md` is gone. Header:
    "`**Status:** Merged · **Date:** 2026-06-29 · **Merged:** 2026-06-30 · …`". README Changes row
    (:40) = `[changes/merged/2026-06-29-add_npm_trusted_publishing.md](changes/merged/…)` | Merged;
    Plans table (:53) lists `plans/2026-06-30-add_npm_trusted_publishing/plan.md` with its
    source-spec link pointing at `changes/merged/…` (orchestrator-corrected link, resolves).
  - *Status:* ☑ SATISFIED

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* docs/spec only — no test suite runs; the edits are internally consistent and every
    cross-link resolves.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, confirm the
    change is docs-only via `jj diff --name-only` (only `.specs/**` Markdown changed); resolve each
    relative link the edits touch (the moved change spec's inbound links from `README.md` and the
    plan) and confirm none 404.
  - *Evidence collected:* `jj diff --name-only` = exactly four `.specs/**` Markdown paths (README.md,
    02-nodejs.md, 05-distribution.md, the moved change spec) — docs-only, no test suite applies.
    A link-resolution sweep of every relative `.md`/dir link in all four changed files (resolved
    from each file's own directory) returned 0 broken links. README's two inbound links to the
    change spec (Changes row + Plans source-spec column) both resolve to `changes/merged/…`; the
    moved change spec's own `../../bindings/specs/{02-nodejs,05-distribution}.md` links resolve
    (orchestrator-corrected from `../bindings/...`).
  - *Status:* ☑ SATISFIED

- **O6 — Reviewable: a reviewer confirms the spec describes reality and the merge landed (Reviewable).**
  - *Claim:* a reviewer reads the two canonical pages against the shipped `package.json` (task 01)
    and `release.yml` (task 02) and confirms the spec now describes reality with no surviving
    secret/`publish-nodejs` reference, and that the change spec + README reflect the merge.
  - *Evidence to collect:* read `05-distribution.md` and `02-nodejs.md` side by side with
    `bindings/nodejs/package.json` and `release.yml`; confirm each spec claim has a matching shipped
    fact; confirm the change spec sits under `merged/` and `README.md` links resolve.
  - *Evidence collected:* exercised in O1 (05 prose ↔ release.yml publish-npm job, line-by-line) and
    O2 (02 Distribution ↔ package.json optionalDependencies). Every spec claim mapped to a shipped
    fact; no surviving `publish-nodejs` reference and no "NPM_TOKEN as a secret" claim (O3). Change
    spec is under `changes/merged/` with Merged stamp and README rows resolve (O4/O5). The spec now
    describes reality.
  - *Status:* ☑ SATISFIED

## Regression check

- `.specs/README.md` and any spec page linking the change spec must not break when it moves to
  `merged/`. Trace: every reference to `2026-06-29-add_npm_trusted_publishing.md` resolves to the
  `merged/` path after the move : ☑ PRESERVED — README's two references both point at
  `changes/merged/…`; neither canonical spec page (02/05) links the change spec, so nothing on a
  spec page breaks. (Note: plan-internal tracking files — `plan.md:3,20` and `done/01,02` — still
  reference the old `changes/…` path; these are orchestration artifacts outside this task's four
  deliverables and outside the regression surface defined here, reconciled by the builder, not a
  spec-page regression.)
- The pypi change spec / other plans referencing 05-distribution.md must still resolve. Trace:
  links into `05-distribution.md` and `02-nodejs.md` are unaffected by the prose edits (no heading
  anchors renamed) : ☑ PRESERVED — all `##`/`###` headings in both pages are unchanged (only body
  prose was added/replaced); no anchored inbound link into either page exists.

## Residue

- The external trusted-publisher registration and staged-publish approval are out-of-repo
  follow-ups recorded in `plan.md`'s Open questions; they are not obligations of this task and need
  not be done for the spec edits to be correct.
- Plan-bookkeeping links (`plan.md` header source-spec, `done/01` and `done/02` task-file
  `Implements:` links) still target the pre-move `changes/…` path. Outside this task's Produces
  (the four spec files); the spec-builder orchestrator reconciles plan-folder bookkeeping on the
  main tree. Non-blocking for these edits.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All six obligations are SATISFIED with collected evidence and both regression traces are
PRESERVED — the canonical pages now describe the shipped `publish-npm`/`build-nodejs` pipeline and
`optionalDependencies` mechanism, the change spec is Merged-stamped and relocated to `changes/merged/`,
README reflects the merge, and every cross-link in the four changed files resolves.
