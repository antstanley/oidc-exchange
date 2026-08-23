# Distribution

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** install.sh, Dockerfile, .github/workflows

How every artifact is built and shipped: the binary install script, the Docker image, the
language packages, and the tag-triggered release pipeline.

## Artifacts

| Artifact | Target | Channel |
|---|---|---|
| Binary `oidc-exchange` | linux x64/arm64, windows x64, macOS arm64 | GitHub Releases (+ checksums) |
| Docker image | `linux/amd64`, `linux/arm64` | ghcr.io and Docker Hub |
| `@oidc-exchange/node` + 4 platform packages (`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}`) | 4 napi targets | npm (OIDC trusted publishing, provenance) |
| `oidc-exchange` wheels | 4 abi3 platforms | PyPI |
| `@oidc-exchange/lambda` | TypeScript | npm |

## Install script (`install.sh`)

A bash installer for `antstanley/oidc-exchange`. It detects OS (`uname -s`) and arch
(`uname -m`), maps to a release asset name, downloads the binary and its SHA-256 checksum from
GitHub Releases, verifies the checksum, and installs to `/usr/local/bin` (root) or
`~/.local/bin` (non-root). It accepts a `--version`/positional pin and defaults to the latest
release; it requires `curl`/`wget` and `sha256sum`/`shasum`.

## Docker (`Dockerfile`)

Multi-stage: `rust:1.85-slim` builder (`cargo build --release --bin oidc-exchange`, with
`pkg-config`/`libssl-dev`) → `debian:bookworm-slim` runtime with `ca-certificates`/`curl`,
exposing 8080 and running the binary. Example Dockerfiles layer config onto the published
image.

## Release pipeline (`.github/workflows`)

**CI (`ci.yml`)** runs on push/PR: `lint` (`cargo fmt --check`, `cargo clippy -- -D
warnings`), `test` (`cargo nextest run --workspace`), `nodejs-test` (build the napi module,
vitest, lint/fmt), `python-test` (maturin build, pytest via uv).

**Release (`release.yml`)** triggers on a `v*.*.*` tag and runs: `validate` (extract version
from the tag), `build-binaries` (matrix per target, checksums), `build-docker` (buildx
multi-arch, push to both registries with `latest`/`vX.Y.Z`/`vX.Y`/`vX` tags), and
`create-release` (GitHub Release with binaries and a generated changelog).

`build-nodejs` (matrix per napi target) builds the native module with `napi build --release
--target <triple>` and uploads the resulting `oidc-exchange.<triple>.node` as an artifact.
`publish-npm` then runs as a separate job — it needs `build-nodejs`, declares `permissions: {
id-token: write, contents: read }`, and runs in the `publish` GitHub Environment so publish
credentials are never exposed to build or runtime code. It downloads the `.node` artifacts, runs
`napi artifacts` to place each binary into its `npm/<triple>` package, validates the root package
with `publint` and `@arethetypeswrong/cli`, and publishes the four platform packages and the root
`@oidc-exchange/node` with `npm publish --provenance --access public`. Authentication is GitHub
OIDC trusted publishing — no `NPM_TOKEN`. Installs use `--ignore-scripts` and every action is
pinned to a full-length commit SHA. The publish step runs on Node.js >= 24.8.0. The same job also
builds and publishes `@oidc-exchange/lambda` on the same OIDC trusted-publishing path.

`build-python` (matrix per platform) builds an `abi3` wheel with maturin in a `manylinux_2_28`
container (`PyO3/maturin-action`, `manylinux: 2_28` for the Linux targets), and one job also
builds the sdist (`maturin sdist`); all wheels and the sdist upload as artifacts. Because the
extension is built against the Python 3.10 stable ABI, one wheel per platform covers 3.10–3.13.
`publish-pypi` then runs as a separate job — it needs `build-python`, declares `permissions: {
id-token: write }`, runs in the `pypi` GitHub Environment, downloads every wheel and the sdist,
and uploads them with `pypa/gh-action-pypi-publish`. Authentication is PyPI Trusted Publishing —
no `PYPI_TOKEN`. Every action is pinned to a full-length commit SHA.

## Version parity

One version string must match across `Cargo.toml` `workspace.package.version`,
`bindings/nodejs/package.json`, and `bindings/python/pyproject.toml`. The `validate` job checks
this before building. Bumps are manual: edit the three files, commit, tag, push. npm and PyPI
use the bare `X.Y.Z`; GitHub and Docker use the `v`-prefixed tag.

## Assumptions and open questions

### Assumptions

- ghcr.io and Docker Hub credentials are configured as repository secrets for the publish
  jobs.
- The git tag is the single source of truth for a release's version.

### Decisions

- *Tag-triggered unified release.* **A `v*.*.*` tag drives binaries, Docker, npm, and PyPI in
  one pipeline.** Atomic, consistent versioning across every artifact from one monorepo.
- *Checksum-verified install.* **`install.sh` verifies SHA-256 before installing.** Detects
  tampering or truncated downloads.
- *npm trusted publishing.* **`@oidc-exchange/node` and its platform packages publish via GitHub
  OIDC, not a stored `NPM_TOKEN`.** Short-lived per-run credentials and npm provenance remove a
  long-lived secret and attest the build to the source commit. The package is configured as a
  trusted publisher for `antstanley/oidc-exchange` / `release.yml` on npmjs.
- *PyPI trusted publishing.* **`oidc-exchange` wheels publish via GitHub OIDC, not a stored
  `PYPI_TOKEN`.** `pypa/gh-action-pypi-publish` exchanges the workflow's OIDC token for a
  short-lived upload credential; the project is registered as a trusted publisher for
  `antstanley/oidc-exchange` / `release.yml` on PyPI.

### Open questions

- Version bumps are manual across three manifests; a single-command bump (or a release-please
  style automation) is not yet in place.


## Runtime parity update

One version string must match across `Cargo.toml` `workspace.package.version`,
`bindings/nodejs/package.json`, and `bindings/python/pyproject.toml`. The `validate` job
checks this before building. Bumps are manual: edit the three files, commit, tag, push. npm
and PyPI use the bare `X.Y.Z`; GitHub and Docker use the `v`-prefixed tag. Because the three
artifacts share one version, a breaking change to the FFI surface bumps all of them together
— under `0.x` that is a minor bump (`0.2.x` → `0.3.0`), and the release notes name the two
packages whose API changed (`@oidc-exchange/node`, `oidc-exchange` on PyPI) and the migration
for each.
- The `conformance` CI job has Rust, Node, and Python toolchains available in one runner; it
  is a required check on the default branch.
