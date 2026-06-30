# Done Certificate — Task 01: npm package manifest

**Task:** [01-npm_package_manifest.md](01-npm_package_manifest.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-06-30 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a file location, a command result, or a parsed manifest) — not by assertion.

## Premises

- **P1 — Goal.** The task makes `bindings/nodejs/package.json` declare the four platform packages
  as `optionalDependencies` at the workspace version plus `publishConfig: { provenance: true,
  access: "public" }`, adds `bindings/nodejs/.npmrc` with `ignore-scripts=true`, and keeps the
  four `npm/<triple>` package versions parity-aligned with the root.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD
  order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not change the shipped payload (`files` stays `index.js` +
  `index.d.ts`), the `index.js` loader, or the version-parity invariant the release `validate`
  job enforces across `Cargo.toml` / `bindings/nodejs/package.json` / `bindings/python/pyproject.toml`.

## Obligations

- **O1 — Root manifest carries optionalDependencies, publishConfig, and `.npmrc` sets ignore-scripts.**
  - *Claim:* `bindings/nodejs/package.json` lists the four platform packages under
    `optionalDependencies` at the workspace version and carries `publishConfig.provenance = true`
    and `publishConfig.access = "public"`; `bindings/nodejs/.npmrc` contains `ignore-scripts=true`.
  - *Evidence to collect:* `jq '.optionalDependencies, .publishConfig' bindings/nodejs/package.json`
    — confirm all four `@oidc-exchange/<triple>` keys are present with the root `version` as value
    and the `publishConfig` object has `provenance: true` / `access: "public"`. Read
    `bindings/nodejs/.npmrc` and confirm the literal line `ignore-scripts=true`.
  - *Checks:* confirm `version` values equal `jq -r '.version' bindings/nodejs/package.json` exactly
    (no `^`/`~` range — the change spec pins to the workspace version).
  - *Status:* ☐ unverified

- **O2 — optionalDependencies keys exactly match the loader keys and platform-package names.**
  - *Claim:* the four `optionalDependencies` keys are exactly the `@oidc-exchange/<triple>` names
    in `index.js`'s `PLATFORM_MAP` and the `npm/<triple>` package names — no typo, missing, or extra.
  - *Evidence to collect:* read `bindings/nodejs/index.js:9` (`PLATFORM_MAP`) and list the four
    mapped package names; `jq -r '.name' bindings/nodejs/npm/*/package.json`; compare both sets to
    `jq -r '.optionalDependencies | keys[]' bindings/nodejs/package.json` — expect three identical
    sets of `{linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc, darwin-arm64}`.
  - *Checks:* negative space — a key not matching a real platform package or loader value, or a
    missing target, is a defect; confirm the intersection is exactly four with no remainder.
  - *Status:* ☐ unverified

- **O3 — Packed payload is still exactly the curated wrappers.**
  - *Claim:* `npm pack --dry-run` (or `pnpm pack`) of the root shows the tarball payload is exactly
    `index.js` + `index.d.ts`.
  - *Evidence to collect:* run `npm pack --dry-run` in `bindings/nodejs` (or `pnpm pack`) and read
    the file list — expect only `index.js`, `index.d.ts`, and `package.json`; confirm no `.node`,
    `src/`, or generated file is pulled in.
  - *Status:* ☐ unverified

- **O4 — Version parity holds across the platform packages.**
  - *Claim:* each `npm/<triple>/package.json` `version` equals the root `version`.
  - *Evidence to collect:* `jq -r '.version' bindings/nodejs/package.json` and
    `jq -r '.version' bindings/nodejs/npm/*/package.json` — expect every value identical; the
    release `validate` job (`.github/workflows/release.yml`) is therefore not regressed.
  - *Status:* ☐ unverified

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* format and lint pass for the touched Node package; no `.ts` source changed so type/test
    gates are unaffected.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `pnpm -C bindings/nodejs format:check` and `pnpm -C bindings/nodejs lint` — expect clean;
    confirm via `jj diff --name-only` that no `.ts`/`.rs` source changed (only `package.json`,
    `.npmrc`, and possibly `pnpm-lock.yaml` / platform `package.json`).
  - *Status:* ☐ unverified

- **O6 — Reviewable: a reviewer confirms the manifest and unchanged payload (Reviewable).**
  - *Claim:* a reviewer reads the updated `package.json` + new `.npmrc`, runs `npm pack --dry-run`
    and optionally `npx publint`, and confirms the four `optionalDependencies` match the
    loader/platform-package names, provenance is configured, and the packed payload is unchanged.
  - *Evidence to collect:* read the diff of `package.json` and the new `.npmrc`; run
    `npm pack --dry-run` (payload check) and `npx publint` in `bindings/nodejs` — expect publint to
    report no errors against the manifest.
  - *Status:* ☐ unverified

## Regression check

- The release `validate` job reads `bindings/nodejs/package.json` `.version`; adding
  `optionalDependencies`/`publishConfig` keys must not change `.version`. Trace: `validate` →
  `jq -r '.version' bindings/nodejs/package.json` still returns the parity version : ☐ (PRESERVED / REGRESSION)
- The `index.js` loader `require`s the `optionalDependencies` names at runtime; adding them as
  declared deps must not change the loader. Confirm `index.js` is byte-unchanged : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether to raise `engines.node` to match the publish runner's Node `24.8.0` is deferred to task
  02; not an obligation here. Refreshing `pnpm-lock.yaml` for the new `optionalDependencies` is a
  side effect of O1, acceptable as long as the lockfile stays the record.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
