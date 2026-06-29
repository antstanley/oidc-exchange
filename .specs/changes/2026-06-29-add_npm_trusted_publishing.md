# Change: Publish `@oidc-exchange/node` to npm via trusted publishing

**Status:** Proposed · **Date:** 2026-06-29 · **Owner:** Ant Stanley · **Target:** bindings/nodejs, .github/workflows

Add a dedicated, hardened npm publish job to `release.yml` that ships `@oidc-exchange/node`
together with its four platform packages, authenticated by GitHub OIDC **trusted publishing**
(no long-lived `NPM_TOKEN`) with provenance, following the
[e18e publishing guidance](https://e18e.dev/docs/publishing.html).

---

## Motivation

The canonical spec already describes a working napi-rs distribution that the pipeline does not
deliver. [`05-distribution.md`](../bindings/specs/05-distribution.md) lists the npm artifact as
"`@oidc-exchange/node` (+ platform packages)", and [`02-nodejs.md`](../bindings/specs/02-nodejs.md)
records "Optional-dependency native packages — per-platform `.node` binaries ship as
optionalDependencies". Neither is true on `main`: the `publish-nodejs` job publishes only the
root package, the four `npm/<triple>` platform packages are never published and never receive
their built `.node` file, and `bindings/nodejs/package.json` declares no `optionalDependencies`.
A consumer who runs `npm install @oidc-exchange/node` installs a package whose loader cannot
resolve a native binary on any platform.

Separately, `publish-nodejs` authenticates with a long-lived `NPM_TOKEN` secret and runs build
and publish in one job. The e18e guidance is to drop publish tokens in favour of OIDC trusted
publishing, keep the publish step isolated from build/runtime code, pin every action to a
full-length commit SHA, run lifecycle scripts off (`--ignore-scripts`), gate publishing behind a
restricted GitHub Environment, and validate the package with `publint` and
`@arethetypeswrong/cli` before it ships.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Release pipeline | Replace the single `publish-nodejs` step with a `build-nodejs` → `publish-npm` pair; describe trusted publishing |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Artifacts | Note provenance and the published platform-package set |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Assumptions / Decisions | Replace "npm credentials configured as secrets" with OIDC trusted publishing |
| [`.specs/bindings/specs/02-nodejs.md`](../bindings/specs/02-nodejs.md) → Distribution | Note `package.json` carries `optionalDependencies` for the platform packages, populated by `napi artifacts` |

---

## Proposed changes

### `.specs/bindings/specs/05-distribution.md` → Release pipeline (Modify)

Replace the `build-nodejs` + `publish-nodejs` description with:

> `build-nodejs` (matrix per napi target) builds the native module with
> `napi build --release --target <triple>` and uploads the resulting `oidc-exchange.<triple>.node`
> as an artifact. `publish-npm` then runs as a separate job — it needs `build-nodejs`, declares
> `permissions: { id-token: write, contents: read }`, and runs in the `publish` GitHub
> Environment so publish credentials are never exposed to build or runtime code. It downloads the
> `.node` artifacts, runs `napi artifacts` to place each binary into its `npm/<triple>` package,
> validates the root package with `publint` and `@arethetypeswrong/cli`, and publishes the four
> platform packages and the root `@oidc-exchange/node` with `npm publish --provenance
> --access public`. Authentication is GitHub OIDC trusted publishing — no `NPM_TOKEN`. Installs
> use `--ignore-scripts` and every action is pinned to a full-length commit SHA. The publish step
> runs on Node.js ≥ 24.8.0.

### `.specs/bindings/specs/05-distribution.md` → Artifacts (Modify)

> | `@oidc-exchange/node` + 4 platform packages (`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}`) | 4 napi targets | npm (OIDC trusted publishing, provenance) |

### `.specs/bindings/specs/05-distribution.md` → Assumptions / Decisions (Modify)

Drop npm from the "credentials configured as repository secrets" assumption and add a Decision:

> - *npm trusted publishing.* **`@oidc-exchange/node` and its platform packages publish via GitHub
>   OIDC, not a stored `NPM_TOKEN`.** Short-lived per-run credentials and npm provenance remove a
>   long-lived secret and attest the build to the source commit. The package is configured as a
>   trusted publisher for `antstanley/oidc-exchange` / `release.yml` on npmjs.

### `.specs/bindings/specs/02-nodejs.md` → Distribution (Modify)

Append:

> The root `package.json` declares the four platform packages as `optionalDependencies` pinned to
> the workspace version; `napi artifacts` copies each built `.node` into its `npm/<triple>` package
> at release time. npm installs only the entry matching the host `{os, cpu}`; the `index.js`
> loader resolves that package, falling back to a co-located `oidc-exchange.node`.

---

## Type changes

None. No domain entity or config field changes; this is packaging and CI only.

---

## Implementation notes

1. `bindings/nodejs/package.json`: add `optionalDependencies` for
   `@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}` at the workspace
   version, and `"publishConfig": { "provenance": true, "access": "public" }`. Keep `files` as
   `index.js` + `index.d.ts` (the curated, hand-maintained module).
2. Add `bindings/nodejs/.npmrc` with `ignore-scripts=true`.
3. `release.yml`: keep `build-nodejs` (matrix) building per-target `.node` artifacts. Replace
   `publish-nodejs` with `publish-npm`:
   - `needs: [build-nodejs]`, `permissions: { id-token: write, contents: read }`,
     `environment: publish`.
   - `actions/setup-node` with `node-version: '24.8.0'` (or newer) and `registry-url`.
   - `npm install -g @napi-rs/cli` (or `pnpm dlx`), download artifacts, run `napi artifacts` to
     populate `npm/<triple>/`.
   - Validate: `npx publint` and `npx @arethetypeswrong/cli --pack`.
   - Publish each `npm/<triple>` package, then the root, with `npm publish --provenance
     --access public --ignore-scripts`.
   - Pin every `uses:` to a commit SHA; optionally lint the workflow with `zizmor`.
4. One-time, outside the repo: register `@oidc-exchange/node` (and each platform package) as a
   trusted publisher on npmjs bound to `antstanley/oidc-exchange` and `release.yml`, and create a
   `publish` GitHub Environment restricted to the release tag/branch. Remove the `NPM_TOKEN` secret
   once trusted publishing is verified.

References: napi-rs CLI `artifacts`/`prepublish`; npm trusted publishing and `--provenance`;
e18e publishing guide; `publint`; `@arethetypeswrong/cli`.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. Update the `05-distribution.md` Artifacts table and Assumptions/Decisions per the blocks above.
3. No schema change.
4. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
5. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- The `@oidc-exchange` npm organisation exists and the four platform package names are available
  (or already owned) so they can be registered as trusted publishers.
- The native binary that `napi build --release --target <triple>` emits is named
  `oidc-exchange.<triple>.node`, matching the platform packages' `main`.

### Decisions

- *Separate build and publish jobs.* **Native builds run in `build-nodejs`; only `publish-npm`
  holds `id-token: write`.** Build/runtime code never sees publish credentials, per e18e.
- *Curated wrappers stay the source of truth.* **`index.js` and `index.d.ts` remain hand-maintained
  and shipped as-is.** The release build emits only `.node` binaries; it does not regenerate the
  wrappers (see the CI `--dts` redirect already in `ci.yml`).

### Open questions

- npm trusted publishing may stage the publish and require a one-time approval at
  `npmjs.com/.../staged-packages`; whether to keep staging on for every release or auto-approve in
  the workflow is deferred to the first live publish.
- Whether to adopt `@e18e/setup-publish` to scaffold the workflow, or hand-write it against the
  existing `release.yml`, is left to implementation.
