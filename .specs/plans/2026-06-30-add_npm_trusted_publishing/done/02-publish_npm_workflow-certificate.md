# Done Certificate — Task 02: publish-npm workflow

**Task:** [02-publish_npm_workflow.md](02-publish_npm_workflow.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-30

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O6 below holds, each backed by the evidence the obligation
names (a YAML location, a grep result, or a parse) — not by assertion.

## Premises

- **P1 — Goal.** The task replaces the `publish-nodejs` job in `.github/workflows/release.yml`
  with a `publish-npm` job that builds nothing (it `needs: [build-nodejs]`), holds
  `id-token: write`, runs in the `publish` Environment, places each `.node` with `napi artifacts`,
  validates with `publint` + `@arethetypeswrong/cli`, and publishes the four platform packages and
  the root with `npm publish --provenance` under OIDC trusted publishing — no `NPM_TOKEN` — while
  preserving the `@oidc-exchange/lambda` publish and fixing `create-release`'s `needs`.
- **P2 — Obligations.** Done iff O1…O6 all hold. One Oi per definition-of-done item, in DoD
  order; O6 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break the rest of the release DAG: `build-nodejs`, `validate`,
  `build-binaries`, `build-docker`, `build-python`/`publish-python`, and `create-release` must
  still resolve their `needs`, and `@oidc-exchange/lambda` must still publish.

## Obligations

- **O1 — `publish-npm` exists with the correct gating, validation, and publish steps.**
  - *Claim:* `publish-npm` declares `needs: [build-nodejs]`, `permissions: { id-token: write,
    contents: read }`, and `environment: publish`; it runs `napi artifacts`, then `publint` +
    `@arethetypeswrong/cli`, then publishes the four platform packages and the root with
    `npm publish --provenance --access public`.
  - *Evidence to collect:* read the `publish-npm` job in `.github/workflows/release.yml`; confirm
    the `needs`, `permissions` (both `id-token: write` and `contents: read`), and `environment:
    publish` keys; confirm step order — `napi artifacts` → `npx publint` + `npx
    @arethetypeswrong/cli --pack` → five `npm publish --provenance --access public` invocations
    (four `npm/<triple>` packages then the root). Validate the file parses (e.g. `actionlint`, or a
    YAML load) — expect no parse error.
  - *Checks:* confirm `id-token: write` lives **only** on `publish-npm` (and not on `build-nodejs`),
    so build code never holds the publish credential.
  - *Status:* ☑ SATISFIED — `publish-npm` (release.yml:205) declares `needs: build-nodejs` (:207),
    `permissions: { id-token: write (:211), contents: read (:212) }`, `environment: publish` (:213);
    step order is `napi artifacts` (:235) → fail-closed binary guard (:237) → `npx --yes publint`
    (:258) → `npx --yes @arethetypeswrong/cli --pack` (:262) → four `npm publish --provenance
    --access public` platform publishes (loop :271) → root publish (:276). `id-token: write` occurs
    once in the file (only on `publish-npm`); `build-nodejs` has no `permissions` block. YAML parses.

- **O2 — No `NPM_TOKEN` / `NODE_AUTH_TOKEN` remains anywhere in the workflow.**
  - *Claim:* no npm publish (root, platform packages, or `@oidc-exchange/lambda`) references
    `NPM_TOKEN` or `NODE_AUTH_TOKEN`; all authenticate via OIDC trusted publishing.
  - *Evidence to collect:* `grep -nE 'NPM_TOKEN|NODE_AUTH_TOKEN' .github/workflows/release.yml` —
    expect no matches (negative space). Confirm no `npm publish`/`pnpm publish` step in the file
    sets an auth-token env.
  - *Status:* ☑ SATISFIED — `grep -nE 'NPM_TOKEN|NODE_AUTH_TOKEN' release.yml` returns no matches.
    No `npm publish`/`pnpm publish` step sets an auth-token env; only the unrelated `publish-python`
    job still uses `PYPI_TOKEN` (:340), which is out of scope and correctly left intact.

- **O3 — The lambda publish is preserved and `create-release` needs `publish-npm`.**
  - *Claim:* `@oidc-exchange/lambda` is still built (`tsc`) and published, and `create-release`'s
    `needs` references `publish-npm`, not the removed `publish-nodejs`.
  - *Evidence to collect:* find the `@oidc-exchange/lambda` build+publish (in `publish-npm` or a
    sibling `publish-lambda` job) and confirm it still runs `pnpm build` + a publish; read
    `create-release.needs` and confirm `publish-npm` appears and `publish-nodejs` does not; `grep -n
    'publish-nodejs' .github/workflows/release.yml` — expect no matches (the job is fully renamed).
  - *Checks:* trace every `needs:` list in the file — confirm none references a job name that no
    longer exists (no dangling dependency after the rename).
  - *Status:* ☑ SATISFIED — `@oidc-exchange/lambda` step (:279) still runs `pnpm build` then a
    publish (:282-285). `create-release.needs` lists `publish-npm` (:348) and `publish-python`
    (:349); `grep -n 'publish-nodejs'` returns no matches (fully renamed). All `needs:` targets
    (validate, build-binaries, build-docker, build-nodejs, build-python, publish-npm, publish-python)
    resolve to jobs that exist — no dangling dependency.

- **O4 — Every `uses:` is SHA-pinned and the platform publish fails closed on a missing `.node`.**
  - *Claim:* every `uses:` in the touched jobs is pinned to a full-length commit SHA (no floating
    `@v*`/branch ref); the platform-package publish fails if a `.node` is missing rather than
    publishing an empty package.
  - *Evidence to collect:* `grep -nE 'uses:' .github/workflows/release.yml` for the `publish-npm`
    (and any `publish-lambda`) job; confirm each ref is a 40-hex-char SHA with a `# vX` comment.
    Read the `napi artifacts` / publish steps and confirm a guard asserts each of the four
    `npm/<triple>/*.node` exists before publishing (e.g. a `test -f` loop or `napi artifacts`
    failure surfacing a non-zero exit).
  - *Status:* ☑ SATISFIED — every `uses:` in the file is pinned to a 40-hex SHA with a `# vX`
    comment (audit: no `uses:` ref fails the 40-hex pattern). The "Verify every platform binary is
    present" step (:237-254) runs `set -euo pipefail`, loops the four triples checking
    `npm/<triple>/oidc-exchange.<triple>.node` (which matches each platform package's `main`), sets
    `missing=1` and `exit 1` if any is absent — the publish fails closed rather than shipping an
    empty package.

- **O5 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the workflow is valid YAML and passes `actionlint` if available; no language test suite
    is affected (CI/packaging only).
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done (CI is the
    enforcement gate), run `actionlint .github/workflows/release.yml` if present — expect clean (or
    record its absence and fall back to a YAML parse); confirm via `jj diff --name-only` that only
    `.github/workflows/release.yml` changed, so no Rust/TS/Python source is touched.
  - *Status:* ☑ SATISFIED (via the obligation's own fallback) — `actionlint` is not installed in
    this environment (UNVERIFIED-environment, not a defect); per the obligation, fell back to a YAML
    parse (`yaml.safe_load`) which loaded cleanly with jobs
    [validate, build-binaries, build-docker, build-nodejs, publish-npm, build-python,
    publish-python, create-release]. `jj diff --name-only` shows only
    `.github/workflows/release.yml` changed — no language source touched.

- **O6 — Reviewable: a reviewer confirms the trusted-publishing wiring end to end (Reviewable).**
  - *Claim:* a reviewer reads `publish-npm` and confirms the build/publish separation (only
    `publish-npm` holds `id-token: write`), the `publish` Environment gate, the `napi artifacts` +
    `publint`/`attw` validation, the `--provenance` publishes of all five npm packages, the absence
    of `NPM_TOKEN`, and the SHA pins — and confirms `create-release` still resolves its `needs`.
  - *Evidence to collect:* walk the `publish-npm` job top to bottom in `release.yml`; cross-check
    against O1–O4 evidence; confirm the five `--provenance` publishes, no token, SHA pins, and a
    resolvable `create-release.needs`.
  - *Status:* ☑ SATISFIED — walked `publish-npm` top to bottom and cross-checked O1-O4:
    build/publish separation (sole `id-token: write`), `publish` Environment gate, `napi artifacts`
    + `publint`/`attw` validation, six `--provenance` publishes (four platform + root + lambda),
    zero auth tokens, all-SHA pins, and a `create-release.needs` that resolves to existing jobs.
    Residue surfaced to the reviewer (not obligations): the four `npm/<triple>` package.json files
    lack a `repository` field (root and lambda have one), so their live `--provenance` publish will
    fail until that is fixed in the manifest/scaffolding (outside this task's release.yml scope);
    and whether `pnpm publish` performs npm's OIDC token exchange for the lambda is unverifiable
    headlessly.

## Regression check

- `create-release` (`.github/workflows/release.yml`) `needs` the npm publish job. Trace: after the
  rename, `create-release.needs` lists `publish-npm` and every named job exists : ☑ PRESERVED —
  `create-release.needs` = [validate, build-binaries, build-docker, publish-npm, publish-python];
  no entry references the removed `publish-nodejs`, and all five jobs exist.
- The `build-nodejs` matrix feeds `publish-npm` via the `nodejs-<triple>` artifacts. Trace:
  `build-nodejs` still uploads `oidc-exchange.<triple>.node` under the artifact name
  `publish-npm`'s download step consumes : ☑ PRESERVED — `build-nodejs` is unchanged (still uploads
  `nodejs-${{ matrix.target }}` carrying `bindings/nodejs/*.node`); `publish-npm` downloads pattern
  `nodejs-*` with `merge-multiple: true`, which `napi artifacts` then distributes.
- `@oidc-exchange/lambda` consumers depend on continued publication. Trace: the lambda
  build+publish still runs in the file : ☑ PRESERVED — the `@oidc-exchange/lambda` step (:279) still
  runs `pnpm install` → `pnpm build` → `pnpm publish` (now token-free, on the provenance path).

## Residue

- Whether lambda publishes from its own `publish-lambda` job or a final step inside `publish-npm`
  is the implementer's choice and not an obligation, provided O2/O3 hold. A `zizmor`/`actionlint`
  lint step is optional polish outside the DoD.
- Live behaviour — that trusted publishing actually authenticates against npmjs — requires the
  out-of-repo trusted-publisher registration and a real tag push; a headless validator cannot drive
  it. Verify the *workflow wiring* statically; surface the live publish for manual confirmation
  rather than failing the task on it.
- **Resolved during build (orchestrator note, 2026-06-30):** the flagged `repository`-field gap on
  the four `bindings/nodejs/npm/<triple>/package.json` files was fixed within this task's workspace —
  each now carries `repository.url = https://github.com/antstanley/oidc-exchange.git` with its own
  `directory`, matching the root manifest, so the platform-package `--provenance` publishes are no
  longer blocked. This closed a task 01 DoD gap ("provenance-ready" did not enumerate the
  platform-package metadata) discovered here; recorded in the build summary. The `pnpm publish` OIDC
  question for lambda remains a verify-on-first-publish item.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All six obligations are SATISFIED and all three regression traces PRESERVED — `release.yml`
replaces `publish-nodejs` with a `publish-npm` job that needs `build-nodejs`, holds the sole
`id-token: write` under the `publish` Environment, distributes binaries with `napi artifacts`, fails
closed on a missing `.node`, validates with `publint` + `@arethetypeswrong/cli`, and publishes the
four platform packages, the root, and the preserved lambda via `npm/pnpm publish --provenance
--access public` with no `NPM_TOKEN`/`NODE_AUTH_TOKEN`; every `uses:` is SHA-pinned, `create-release`
needs `publish-npm`, and `publish-python`/`PYPI_TOKEN` are untouched. Residue (per the certificate,
not obligations, surfaced for manual action): the four `bindings/nodejs/npm/<triple>/package.json`
files lack a `repository` field, so their live `--provenance` publish will fail until that is added in
the manifest/scaffolding (outside this task's release.yml-only scope); whether `pnpm publish`
performs npm's OIDC token exchange for the lambda, and that trusted publishing authenticates against
npmjs, require the out-of-repo trusted-publisher registration and a real tag push and cannot be
verified headlessly.
