# Task 01 — npm package manifest

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-npm_package_manifest-certificate.md](01-npm_package_manifest-certificate.md)

**Implements:** [.specs/changes/merged/2026-06-29-add_npm_trusted_publishing.md](../../../changes/merged/2026-06-29-add_npm_trusted_publishing.md) §Implementation notes 1 & 2 (root `optionalDependencies` + `publishConfig`; `.npmrc` `ignore-scripts`); satisfies the [.specs/bindings/specs/02-nodejs.md](../../../bindings/specs/02-nodejs.md) §Distribution `optionalDependencies` fact recorded in task 03.
**Depends on:** —
**Produces:** `bindings/nodejs/package.json` declares `@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}` as `optionalDependencies` pinned to the workspace version and adds `"publishConfig": { "provenance": true, "access": "public" }`, keeping `files` as `index.js` + `index.d.ts`; `bindings/nodejs/.npmrc` sets `ignore-scripts=true`; the four `npm/<triple>/package.json` versions stay equal to the root version (parity preserved).
**Pointers:** `bindings/nodejs/package.json:11` (`files`), `:34` (`napi` block — the four targets that name the platform packages); `bindings/nodejs/npm/*/package.json` (existing `version: 0.1.0`, `os`/`cpu`/`main`); `bindings/nodejs/index.js:9` (`PLATFORM_MAP` — the exact `optionalDependencies` names must match these keys); new file `bindings/nodejs/.npmrc`.

## Steps

- [ ] Add an `optionalDependencies` block to `bindings/nodejs/package.json` naming all four platform packages (`@oidc-exchange/linux-x64-gnu`, `@oidc-exchange/linux-arm64-gnu`, `@oidc-exchange/win32-x64-msvc`, `@oidc-exchange/darwin-arm64`), each pinned to the exact root `version` (workspace version), so the values match the `PLATFORM_MAP` targets in `index.js` and the `npm/<triple>` package names.
- [ ] Add `"publishConfig": { "provenance": true, "access": "public" }` to the root `package.json`; leave `files` as `["index.js", "index.d.ts"]` (the curated wrappers are the only shipped module).
- [ ] Add `bindings/nodejs/.npmrc` containing `ignore-scripts=true` so lifecycle scripts never run on install during publish.
- [ ] Confirm the four `npm/<triple>/package.json` `version` fields equal the root `version`; bump any that drift so the release `validate` job's version-parity invariant holds.
- [ ] Run `pnpm -C bindings/nodejs install` to refresh `pnpm-lock.yaml` for the new `optionalDependencies` (the lockfile of record), and `npm pack --dry-run` (or `pnpm pack`) to confirm the tarball still contains only `index.js` + `index.d.ts`.

## Definition of done

- [ ] `bindings/nodejs/package.json` lists the four platform packages under `optionalDependencies` at the workspace version and carries `publishConfig.provenance = true` / `publishConfig.access = "public"`; `bindings/nodejs/.npmrc` contains `ignore-scripts=true` (verify by reading the files and parsing the JSON).
- [ ] `optionalDependencies` keys are exactly the four `@oidc-exchange/<triple>` names in `index.js`'s `PLATFORM_MAP` and the `npm/<triple>` package names — no typo, no missing target, no extra (negative space: a name that does not match a real platform package or loader key is a defect).
- [ ] `npm pack --dry-run` (or `pnpm pack`) of the root shows the tarball payload is still exactly `index.js` + `index.d.ts` (the `optionalDependencies`/`publishConfig` additions do not pull build/native files into the package).
- [ ] Version parity holds: each `npm/<triple>/package.json` `version` equals `bindings/nodejs/package.json` `version`, so the release `validate` job is not regressed.
- [ ] Meets the repo definition of done for the languages touched: `pnpm -C bindings/nodejs format:check` and `pnpm -C bindings/nodejs lint` pass; no `.ts` source changed so `pnpm typecheck`/`pnpm test` are unaffected (run them if a fast check is available and report the result).
- [ ] Reviewable: a reviewer reads the updated `package.json` + new `.npmrc`, runs `npm pack --dry-run` and (optionally) `npx publint`, and confirms the four `optionalDependencies` match the loader/platform-package names, provenance is configured, and the packed payload is unchanged.

## Open questions

- Whether to also raise `engines.node` (currently `>= 22`) in lockstep with the publish runner's Node `24.8.0` is deferred to task 02; this task leaves `engines` untouched because the change spec sets the Node floor on the *publish job*, not the published package.
