# Done Certificate — Task 03: canonical edits + merge

**Task:** [03-canonical_edits_and_merge.md](03-canonical_edits_and_merge.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-30 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — 02-nodejs.md Distribution records the optionalDependencies + napi artifacts mechanism.**
  - *Claim:* `02-nodejs.md` §Distribution notes that `package.json` declares the four platform
    packages as `optionalDependencies` populated by `napi artifacts`, and that npm installs only the
    host-matching entry which the loader resolves (local fallback).
  - *Evidence to collect:* read `.specs/bindings/specs/02-nodejs.md` §Distribution; confirm the
    appended note; cross-check it against the `optionalDependencies` task 01 added to
    `bindings/nodejs/package.json`.
  - *Status:* ☐ unverified

- **O3 — Both pages bumped, no stale publish-nodejs / NPM_TOKEN claim survives.**
  - *Claim:* both edited pages carry a bumped `**Date:**`, and no `publish-nodejs` /
    "NPM_TOKEN as a secret" claim remains on either page.
  - *Evidence to collect:* `grep -nE 'publish-nodejs|NPM_TOKEN' .specs/bindings/specs/05-distribution.md
    .specs/bindings/specs/02-nodejs.md` — expect no matches; read each page header and confirm
    `**Date:**` is `2026-06-30` (or later than its prior value).
  - *Status:* ☐ unverified

- **O4 — The change spec is Merged-stamped and relocated; README updated.**
  - *Claim:* the change spec is `Merged`-stamped and lives at
    `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md`; `.specs/README.md` references
    it under `merged/` and lists this plan.
  - *Evidence to collect:* confirm `.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md`
    exists and `.specs/changes/2026-06-29-add_npm_trusted_publishing.md` no longer does
    (`jj diff --name-only` shows the rename/move); read its header for `**Status:** Merged` and a
    `**Merged:**` stamp; read `.specs/README.md` and confirm the Changes row points at `merged/`
    with Status `Merged` and the Plans table lists `2026-06-30-add_npm_trusted_publishing`.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* docs/spec only — no test suite runs; the edits are internally consistent and every
    cross-link resolves.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, confirm the
    change is docs-only via `jj diff --name-only` (only `.specs/**` Markdown changed); resolve each
    relative link the edits touch (the moved change spec's inbound links from `README.md` and the
    plan) and confirm none 404.
  - *Status:* ☐ unverified

- **O6 — Reviewable: a reviewer confirms the spec describes reality and the merge landed (Reviewable).**
  - *Claim:* a reviewer reads the two canonical pages against the shipped `package.json` (task 01)
    and `release.yml` (task 02) and confirms the spec now describes reality with no surviving
    secret/`publish-nodejs` reference, and that the change spec + README reflect the merge.
  - *Evidence to collect:* read `05-distribution.md` and `02-nodejs.md` side by side with
    `bindings/nodejs/package.json` and `release.yml`; confirm each spec claim has a matching shipped
    fact; confirm the change spec sits under `merged/` and `README.md` links resolve.
  - *Status:* ☐ unverified

## Regression check

- `.specs/README.md` and any spec page linking the change spec must not break when it moves to
  `merged/`. Trace: every reference to `2026-06-29-add_npm_trusted_publishing.md` resolves to the
  `merged/` path after the move : ☐ (PRESERVED / REGRESSION)
- The pypi change spec / other plans referencing 05-distribution.md must still resolve. Trace:
  links into `05-distribution.md` and `02-nodejs.md` are unaffected by the prose edits (no heading
  anchors renamed) : ☐ (PRESERVED / REGRESSION)

## Residue

- The external trusted-publisher registration and staged-publish approval are out-of-repo
  follow-ups recorded in `plan.md`'s Open questions; they are not obligations of this task and need
  not be done for the spec edits to be correct.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
