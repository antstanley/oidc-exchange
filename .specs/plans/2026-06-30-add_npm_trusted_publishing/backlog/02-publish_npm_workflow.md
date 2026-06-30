# Task 02 — publish-npm workflow

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-publish_npm_workflow-certificate.md](02-publish_npm_workflow-certificate.md)

**Implements:** [.specs/changes/2026-06-29-add_npm_trusted_publishing.md](../../../changes/2026-06-29-add_npm_trusted_publishing.md) §Implementation notes 3 (replace `publish-nodejs` with `publish-npm`); realises the [.specs/bindings/specs/05-distribution.md](../../../bindings/specs/05-distribution.md) §Release pipeline `build-nodejs` → `publish-npm` description recorded in task 03.
**Depends on:** 01
**Produces:** `.github/workflows/release.yml` replaces the `publish-nodejs` job with `publish-npm`: `needs: [build-nodejs]`, `permissions: { id-token: write, contents: read }`, `environment: publish`, Node `>= 24.8.0` via `actions/setup-node` with `registry-url`; it installs `@napi-rs/cli`, downloads the `nodejs-*` artifacts, runs `napi artifacts` to populate each `npm/<triple>` package, validates the root with `publint` and `@arethetypeswrong/cli --pack`, and publishes the four platform packages then the root with `npm publish --provenance --access public --ignore-scripts` under OIDC trusted publishing (no `NPM_TOKEN`); every `uses:` is SHA-pinned; the `@oidc-exchange/lambda` publish is preserved on the same trusted-publishing path; `create-release`'s `needs` is updated from `publish-nodejs` to `publish-npm`.
**Pointers:** `.github/workflows/release.yml:167` (`build-nodejs` matrix — keep, it feeds the artifacts), `:205` (`publish-nodejs` — replace), `:224`-`:236` (the root + `@oidc-exchange/lambda` publish steps to carry over), `:293`-`:300` (`create-release` `needs` list); `bindings/nodejs/npm/` (target of `napi artifacts`); `bindings/nodejs/package.json:34` (`napi.binaryName`/`targets` that drive `napi artifacts`).

## Steps

- [ ] Keep the `build-nodejs` matrix job as the artifact producer; confirm each matrix leg uploads its `oidc-exchange.<triple>.node` under a `nodejs-<triple>` artifact name the publish job can download and `napi artifacts` can place.
- [ ] Replace `publish-nodejs` with a `publish-npm` job declaring `needs: [build-nodejs]`, `permissions: { id-token: write, contents: read }`, and `environment: publish`; pin `runs-on: ubuntu-latest`.
- [ ] Use `actions/setup-node` with `node-version: '24.8.0'` (or newer) and `registry-url: https://registry.npmjs.org`; install the napi CLI (`npm install -g @napi-rs/cli` or `pnpm dlx`).
- [ ] Download the `nodejs-*` artifacts into `bindings/nodejs`, run `napi artifacts` to copy each built `.node` into its `npm/<triple>/` package, and assert all four platform packages now carry their `.node` (fail the job if any is missing — an empty platform package must not publish).
- [ ] Validate the root package with `npx publint` and `npx @arethetypeswrong/cli --pack` before any publish; a validation failure aborts the job.
- [ ] Publish each `npm/<triple>` platform package, then the root `@oidc-exchange/node`, with `npm publish --provenance --access public --ignore-scripts` and no `NODE_AUTH_TOKEN`/`NPM_TOKEN` (authentication is OIDC trusted publishing); preserve the `@oidc-exchange/lambda` build-and-publish on the same `--provenance` trusted-publishing path so `NPM_TOKEN` is no longer referenced anywhere in the file.
- [ ] Pin every `uses:` in the new/edited jobs to a full-length commit SHA (carry the existing pinned SHAs forward); update `create-release`'s `needs` from `publish-nodejs` to `publish-npm`.

## Definition of done

- [ ] `publish-npm` exists with `needs: [build-nodejs]`, `permissions: { id-token: write, contents: read }`, and `environment: publish`; it runs `napi artifacts`, then `publint` + `@arethetypeswrong/cli`, then publishes the four platform packages and the root with `npm publish --provenance --access public` (verify by reading the job).
- [ ] No `NPM_TOKEN` / `NODE_AUTH_TOKEN` reference remains anywhere in `release.yml` for any npm publish (the root, the platform packages, and `@oidc-exchange/lambda` all authenticate via OIDC trusted publishing) — negative space: grep the file for `NPM_TOKEN` returns nothing.
- [ ] The `@oidc-exchange/lambda` publish is preserved (still built with `tsc` and published) and `create-release`'s `needs` references `publish-npm`, not the removed `publish-nodejs`, so the release DAG has no dangling dependency.
- [ ] Every `uses:` in the touched jobs is pinned to a full-length commit SHA (no floating `@v*`/branch ref); the platform-package publish step fails closed if a `.node` is missing rather than publishing an empty package.
- [ ] Meets the repo definition of done for what the task touches: the workflow is valid YAML and passes `actionlint` if available (report its absence otherwise); the change is CI/packaging only, so no Rust/TS/Python test suite is affected.
- [ ] Reviewable: a reviewer reads the `publish-npm` job and confirms the build/publish separation (only `publish-npm` holds `id-token: write`), the `publish` Environment gate, the `napi artifacts` + `publint`/`attw` validation, the `--provenance` publishes of all five npm packages, the absence of `NPM_TOKEN`, and the SHA pins — and confirms `create-release` still resolves its `needs`.

## Open questions

- Whether the lambda publish becomes its own `publish-lambda` job or a final step inside `publish-npm` is left to the implementer; either is acceptable provided lambda still publishes, authenticates via trusted publishing, and `NPM_TOKEN` is fully removed.
- Whether to add a `zizmor`/`actionlint` workflow-lint step is optional polish, not required by the DoD.
