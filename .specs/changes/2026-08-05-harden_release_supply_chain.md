# Change: Harden the release and dependency supply chain

**Status:** Proposed · **Date:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** .github/workflows, install.sh, Cargo.lock, bindings/* (distribution)

Close the supply-chain gaps around the tag-triggered release: pin every dependency resolved
inside a job that holds publishing rights, scope `GITHUB_TOKEN` per job instead of once at
workflow level, build releases against the committed lockfiles, attach a Sigstore build
attestation to the two channels that lack one (GitHub Release binaries and container images)
and have `install.sh` verify it, validate the installer's `--version` operand before it
reaches a URL, add a dependency-advisory gate to CI with a written policy, and add the
`.gitignore` rules that stop a generated signing key being committed.

---

## Motivation

The release pipeline is careful in the places its author looked at and unguarded in the
places nobody has looked at yet. Every `uses:` in `.github/workflows/release.yml` is pinned
to a full commit SHA; npm and PyPI both publish over OIDC trusted publishing with no
long-lived token; npm publishes are staged behind an out-of-band approval. Yet inside
`publish-npm` — the one job that can publish `@oidc-exchange/*` — the workflow performs four
unpinned package fetches before anything is staged: `npm install -g npm@latest`
(`release.yml:351`), `pnpm add -g @napi-rs/cli` (`:354`), `npx --yes publint` (`:396`), and
`npx --yes @arethetypeswrong/cli` (`:402`) — then, before staging the Lambda package, an
unfrozen `pnpm install --ignore-scripts --no-frozen-lockfile` (`:449`) against a package
with no committed lockfile at all. Publishing authority is therefore
delegated to whoever controls those packages on the day a tag is pushed, and every tagged
release re-delegates it. That is the highest-severity item in this change. Around it sit a
workflow-level `permissions: contents: write, packages: write` (`:8-10`) that seven of the
nine jobs inherit and only three need, a `pnpm install --no-frozen-lockfile` in the addon
build (`:318`, mirrored at `nodejs-addon-glibc.yml:52`) that lets the shipped native module
be built from dependency versions nobody reviewed, and release binaries published with only
a `.sha256` sidecar generated on the same runner from the same job (`:120-129`, `:578-616`)
— an integrity check against corruption, not an authenticity check against substitution.

The dependency side has the same shape: a control that does not exist, and a consequence
that is already realised. No workflow runs `cargo audit`, `cargo deny`, `pnpm audit`,
`osv-scanner` or anything equivalent; there is no `deny.toml`, no `audit.toml`, no
`.github/dependabot.yml` and no Renovate config anywhere in the tree. `cargo deny check
advisories` against the *committed* `Cargo.lock` fails today with six advisories across
seven crate instances, plus an unmaintained crate and a yanked version, and nothing in the
repository would ever have surfaced them. Two of those six are the `pyo3 0.22.6` advisories
that reach the published PyPI wheel (`Cargo.lock:3312-3315`, held on the 0.22 line by
`bindings/python/Cargo.toml:12`); five directly-declared cryptographic crates on the signing
and verification path are release candidates that have already floated several pre-releases
past their manifest specifiers (`crates/adapters/Cargo.toml:14,19-22` versus
`Cargo.lock:1289`, `:2935`, `:2960`, `:2974`, `:3716`; `curve25519-dalek 5.0.0-rc.1` at
`:1088` rides transitively beneath `ed25519-dalek`). And RUSTSEC-2023-0071 (Marvin) fires
against both `rsa 0.9.10` and `rsa 0.10.0-rc.18` — the latter a shipped edge, declared under
`[dependencies]` at `crates/adapters/Cargo.toml:19` and again under `[dev-dependencies]` at
`:40` — with **no current cryptographic exposure**, because every
`RsaPrivateKey` construction in the workspace is inside `#[cfg(test)]`
(`crates/adapters/src/kms/mod.rs:460` guards `:640`, `:837`, `:890`;
`crates/adapters/src/oidc/mod.rs:268` guards `:286`) and shipped code performs only
public-key verification (`crates/adapters/src/kms/mod.rs:269-284`). That property is held by
convention alone. The gap being closed is detection: the advisory becomes live the day
someone adds a private-key operation to shipped code, and today nothing would say so.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Artifacts | Add a provenance column; every channel carries an attestation |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Install script (`install.sh`) | `--version` pattern validation; attestation verification with a loud fallback |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Release pipeline | Per-job `permissions:`; pinned publish tooling; frozen-lockfile builds; build-provenance attestation; the CI advisory gate |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → *new section* Supply-chain gates | Adds a section stating the pinning, lockfile, permission and advisory invariants and the advisory policy |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) → Assumptions / Decisions | Seven new Decisions; one new Assumption; one new Open question |
| [`.specs/development-guidelines.md`](../development-guidelines.md) → Repository hygiene | Advisory gate as a standing CI gate; pattern-based ignore rules for generated key material and local state |

No new canonical page. The `Supply-chain gates` section is added to the existing
`05-distribution.md`.

**Out of scope.** The installer's checksum verification *failing open* when neither
`sha256sum` nor `shasum` is present (`install.sh:82-91`) belongs to the fail-open axis and is
specified in [`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md);
that delta is not restated here. This change owns the `--version` traversal and the
signing/attestation work. The two are complementary and neither substitutes for the other:
fail-closed removes the case where no check runs at all, attestation replaces a check the
artifact's own publisher can forge.

---

## Proposed changes

### `.specs/bindings/specs/05-distribution.md` → Artifacts (Modify)

> | Artifact | Target | Channel | Provenance |
> |---|---|---|---|
> | Binary `oidc-exchange` | linux x64/arm64, windows x64, macOS arm64 | GitHub Releases (+ checksums) | Sigstore build attestation |
> | Docker image | `linux/amd64`, `linux/arm64` | ghcr.io and Docker Hub | Sigstore build attestation |
> | `@oidc-exchange/node` + 4 platform packages (`@oidc-exchange/{linux-x64-gnu,linux-arm64-gnu,win32-x64-msvc,darwin-arm64}`) | 4 napi targets | npm (OIDC trusted publishing) | npm `--provenance` |
> | `oidc-exchange` wheels | 4 abi3 platforms | PyPI (OIDC trusted publishing) | PyPI attestations |
> | `@oidc-exchange/lambda` | TypeScript | npm (OIDC trusted publishing) | npm `--provenance` |
>
> Every channel carries a provenance claim binding the artifact digest to this repository,
> this workflow, and the tagged source revision. The checksum sidecar is retained alongside
> the binaries as a corruption check and a fallback, not as the authenticity claim.

### `.specs/bindings/specs/05-distribution.md` → Install script (`install.sh`) (Modify)

Replace the paragraph with:

> A bash installer for `antstanley/oidc-exchange`. It detects OS (`uname -s`) and arch
> (`uname -m`) and maps them to a release asset name. It accepts a `--version` pin and
> defaults to the latest release; a supplied pin is validated against
> `^v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$` before it is used, and a value that does not
> match is rejected with a non-zero exit before any request is made. The download URL is
> therefore determined by the installer, never by path structure in an operand. It then
> downloads the binary and its SHA-256 checksum from GitHub Releases and establishes
> provenance: when `gh` is present it runs `gh attestation verify <binary> --repo
> antstanley/oidc-exchange` and refuses to install if verification fails; when `gh` is absent
> it falls back to the checksum and states explicitly that the artifact was checked for
> corruption only and could not be authenticated. It installs to `/usr/local/bin` (root) or
> `~/.local/bin` (non-root); it requires `curl` and `sha256sum`/`shasum`. The
> documented install instructions lead with the verifying path.

### `.specs/bindings/specs/05-distribution.md` → Release pipeline, CI paragraph (Modify)

> **CI (`ci.yml`)** runs on push/PR: `lint` (`cargo fmt --check`, `cargo clippy -- -D
> warnings`), `test` (`cargo nextest run --workspace`), `nodejs-test` (build the napi module,
> vitest, lint/fmt), `python-test` (maturin build, pytest via uv), `web-apps` (lint, format,
> typecheck), and `advisories` (`cargo deny check advisories bans sources` against
> `deny.toml`). Every job declares its own `permissions:`; none needs a write scope.

### `.specs/bindings/specs/05-distribution.md` → Release pipeline, Release paragraph (Modify)

> **Release (`release.yml`)** triggers on a `v*.*.*` tag. It declares no workflow-level
> `permissions:`; each job declares only the scopes its own outputs require, so no job that
> compiles or executes third-party code can rewrite the repository or hold a publishing
> identity for a channel it does not itself produce. `validate` (extract
> version from the tag, preflight which registries already have it) and the build jobs run
> with `contents: read` and `persist-credentials: false`; `build-docker` and
> `docker-manifest` add `packages: write`; `create-release` holds `contents: write`;
> `publish-npm` and `publish-pypi` hold `id-token: write` and nothing else beyond
> `contents: read`. `build-binaries` (matrix per target, checksums) and `build-docker`
> (native runner per arch, push by digest) each add `attestations: write` and
> `id-token: write` for their own attestation step and call
> `actions/attest-build-provenance` on the artifact digest they produced. `docker-manifest`
> assembles the multi-arch manifest; `create-release` publishes the GitHub Release with the
> binaries, their checksums and a generated changelog.

### `.specs/bindings/specs/05-distribution.md` → Release pipeline, Node.js paragraph (Modify)

Amend the `build-nodejs` / `publish-npm` paragraph so it reads:

> `build-nodejs` (matrix per napi target) installs with `pnpm install --frozen-lockfile` and
> builds the native module with `napi build --release --target <triple>`, uploading the
> resulting `oidc-exchange.<triple>.node` as an artifact. `validate-npm-package` then runs
> `publint` and `@arethetypeswrong/cli` from the workspace `devDependencies` under
> `permissions: { contents: read }` — the job runs `actions/checkout`, so the read scope
> cannot be zeroed — and package validation never runs beside a publishing credential.
> `publish-npm` needs both, declares `permissions: { id-token: write, contents: read }`, and
> runs in the `publish` GitHub Environment. Every tool it runs is version-pinned: the napi
> CLI is installed at an exact version (or resolved from the workspace lockfile), the npm
> upgrade step names an exact npm version or is removed once the pinned Node release bundles
> a sufficient npm, and no step uses `@latest`, `npx --yes`, or a lockfile-bypassing install.
> It downloads the `.node` artifacts, runs `napi artifacts`, verifies every platform binary
> is present, and stages the four platform packages and the root `@oidc-exchange/node` with
> `npm stage publish --provenance --access public --ignore-scripts`. Authentication is GitHub
> OIDC trusted publishing — no `NPM_TOKEN`. Staged packages go live only on out-of-band
> approval. The same job builds and stages `@oidc-exchange/lambda` from its own committed
> lockfile with `pnpm install --frozen-lockfile`.

### `.specs/bindings/specs/05-distribution.md` → Supply-chain gates (Add)

Add as a new section after `Release pipeline`:

> ## Supply-chain gates
>
> Four invariants hold across the pipeline, each with a mechanical check.
>
> **Pinning.** Every `uses:` is a full-length commit SHA. Every tool a job executes is either
> committed in a lockfile or named at an exact version. No job that holds a write scope or a
> publishing identity contains an `@latest` specifier, an `npx --yes` invocation, an
> unversioned `pnpm add`, or a lockfile-bypassing install. `pnpm-workspace.yaml` sets
> `minimumReleaseAge` explicitly rather than relying on pnpm's default, which covers only
> pnpm-mediated fetches and not `npm`/`npx` ones; `minimumReleaseAgeExclude` continues to
> exempt this repository's own first-party packages.
>
> **Lockfiles.** Every install in CI and in the release pipeline is frozen.
> `bindings/lambda` carries its own committed lockfile.
>
> **Least privilege.** Permissions are declared per job, never at workflow level. A job-level
> block zeroes every scope it does not name, so jobs that check out the repository restate
> `contents: read`. Jobs that do not push set `persist-credentials: false`, since
> `actions/checkout` otherwise persists the token into `.git/config` in the workspace.
>
> **Advisories.** The `advisories` CI job runs `cargo deny check advisories bans sources`
> against `deny.toml` on every push and pull request, and the release pipeline runs the same
> check before any publish job. The policy: an advisory with no `deny.toml` entry **fails**
> the build; an advisory with an entry carrying a reachability rationale and an expiry date
> **warns** until that date and fails after it; an `unmaintained` or `yanked` finding warns.
> Each ignore entry records why the advisory is not reachable in this codebase, so the
> justification is written down where it can be falsified rather than held in someone's head.
> The entries carried today are RUSTSEC-2025-0020 and RUSTSEC-2026-0177 (`pyo3 0.22.6`, until
> the binding moves off the 0.22 line), RUSTSEC-2023-0071 twice (`rsa 0.9.10` and
> `rsa 0.10.0-rc.18`; no fixed release exists, and the rationale is that every
> `RsaPrivateKey` construction in the workspace is `#[cfg(test)]` — shipped code performs
> only public-key verification), and RUSTSEC-2026-0098/-0099/-0104 (`rustls-webpki 0.101.7`,
> reachable only through the AWS SDK stack; the outbound provider TLS path resolves through
> `rustls-webpki 0.103.13`, which is fixed for all three).
>
> **All three dependency graphs, not one.** `cargo deny` sees only the Cargo graph. Two
> published channels resolve dependencies it never inspects: the JavaScript graph
> (`apps/admin-ui`, `apps/website`, `bindings/lambda`, and the Node addon's own runtime
> dependencies) and the Python build graph (`bindings/python`). The `advisories` job
> therefore runs three checks under one policy — `cargo deny check advisories bans sources`
> for Cargo, `pnpm audit --audit-level high --recursive` for the pnpm workspace, and
> `pip-audit` over the Python build environment — each with the same fail/warn rule and each
> recording its ignores with a rationale and an expiry. Gating one graph and publishing four
> artifacts is the shape of the original defect, not a partial fix for it.
>
> The frozen-lockfile work above is a prerequisite for the JavaScript half: `bindings/lambda`
> ships no lockfile today, and auditing an unlocked graph reports on whatever resolved at
> that instant rather than on what was published.

### `.specs/bindings/specs/05-distribution.md` → Assumptions / Decisions (Modify)

Add one Assumption:

> - Release jobs run on GitHub-hosted runners with a workload identity, so
>   `actions/attest-build-provenance` can obtain an OIDC token and Sigstore is reachable at
>   build time.

Add the Decisions:

> - *Per-job token scopes.* **`release.yml` declares no workflow-level `permissions:`; each
>   job declares its own.** A workflow-level grant reaches every job that does not override
>   it, so the compile jobs — which build third-party `build.rs` scripts and run third-party
>   lifecycle code — held a token that could rewrite the repository and publish packages.
> - *Nothing unpinned inside a publishing job.* **A job holding `id-token: write` executes
>   only code whose exact version is recorded in the repository, and package validation moved
>   to its own job holding only `contents: read`.** Run-time resolution inside the publish
>   job delegates
>   publishing authority to whoever controls those packages at release time.
> - *Frozen-lockfile release builds.* **Every install in CI and in the release pipeline uses
>   `--frozen-lockfile`.** The shipped artifact is built from the reviewed dependency set;
>   the `--no-frozen-lockfile` workaround for the self-referential platform packages is
>   handled by `minimumReleaseAgeExclude` and a regenerated lockfile instead.
> - *Uniform build attestation.* **Binaries and container images carry a Sigstore build
>   attestation via `actions/attest-build-provenance`, matching what npm and PyPI already
>   provide.** A checksum generated by the same job on the same runner and published to the
>   same release proves integrity in transit and nothing about origin; an attestation binds
>   the digest to this repository, this workflow and this commit, so substitution after
>   publication is detectable without trusting the release host.
> - *Verify before install, fall back loudly.* **`install.sh` verifies the attestation with
>   `gh` when available and otherwise falls back to the checksum with an explicit statement
>   that the artifact was not authenticated.** Hard-requiring `gh` would be stronger and would
>   be widely worked around; a loud fallback plus verifying-first documentation is the design
>   people actually follow.
> - *Version pin validated before use.* **`install.sh` matches `--version` against an explicit
>   release-tag pattern and exits non-zero on a mismatch.** The operand was interpolated into
>   the download base unchanged, and because the checksum URL derives from the same base, a
>   re-parented request was verified against the attacker's own sidecar.
> - *Advisory gate with a recorded ignore list.* **CI gates all three dependency graphs —
>   Cargo via `cargo deny` against a committed `deny.toml`, the pnpm workspace via
>   `pnpm audit`, and the Python build environment via `pip-audit` — under one policy: a new
>   advisory fails the build, a recorded one warns until its expiry.** The committed lockfile
>   already fails an advisory check with six advisories, so a fail-on-everything gate would
>   block every release on day one; recording each with a rationale and an expiry converts an
>   unbounded remediation task into a bounded process. Gating only Cargo while publishing an
>   npm package, a wheel, a binary and a container would leave two of the four channels
>   resolving unexamined dependencies — the same blind spot in a smaller shape.

Amend the Open questions to add:

> - The release-candidate cryptographic crates on the signing and verification path
>   (`ed25519-dalek 3.0.0-rc.1` with `curve25519-dalek 5.0.0-rc.1` beneath it,
>   `rsa 0.10.0-rc.18`, and `p256`/`p384`/`p521 0.14.0-rc.15`) carry no security-support
>   commitment, and a caret requirement naming a pre-release pins nothing — all five
>   directly-declared crates have already floated several pre-releases past what their
>   manifests name. A one-time version bump regresses; a dependency-policy check over the
>   *resolved* graph is what would hold it. That check is not yet written.

### `.specs/development-guidelines.md` → Repository hygiene (Modify)

Amend the CI bullet and add two:

> - **CI is the enforcement gate** (`.github/workflows/ci.yml`): format-check, clippy,
>   nextest, the napi build + vitest, the maturin build + pytest, the web-app hygiene checks,
>   and the dependency-advisory gate (`cargo deny check advisories bans sources`) run on
>   every push and PR. See
>   [bindings/specs/05-distribution → Supply-chain gates](bindings/specs/05-distribution.md).
> - **Generated key material and local state are never committed.** `.gitignore` carries
>   pattern rules — `*.pem`, `*.p8`, `*.key`, `keys/`, `data/`, `lmdb/`, `*.db`,
>   `*.sqlite`, `*.sqlite3` — rather than path rules, because the documented setup flows
>   (`examples/linux-sqlite/setup.sh`, `CONTRIBUTING.md`, and the deployment guides) write
>   key material relative to whatever directory they are run from. The rules matter more here
>   than in a git-only project: `jj`'s `snapshot.auto-track = "all()"` default records new
>   files at the next snapshot with no deliberate add step. A CI secret-scanning job is the
>   complementary control.
> - **Dependency advisories are triaged, not carried silently.** An advisory that fires on a
>   committed lockfile either gets fixed or gets a `deny.toml` entry with a reachability
>   rationale and an expiry date. "Not reachable today" is a claim that must be written down
>   and dated, because nothing else enforces it.

---

## Type changes

None. This change touches workflows, the installer, ignore rules, and dependency policy
files; no domain entity, config field, or API shape changes. No
`canonical-types.schema.json` fragment.

---

## Implementation notes

Four work packages. **A** and **C** are independent of each other and can ship in either
order; **B** depends on A's permissions work; **D** is independent of all three. This
follows Option 2 of
[`hardening/proposals/release-provenance.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/release-provenance.md)
— uniform attestation with a verifying installer, subsuming Option 1 — with Option 3's
dependency-policy gate taken separately as **D**.

**A — Close the named gaps** (`.github/workflows/release.yml`, `install.sh`)

1. `install.sh:9-21`: validate `VERSION` against
   `^v?[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$` immediately after parsing, before
   `DOWNLOAD_BASE` is built at `:67`. Reject a value containing `/`, `..`, a scheme, or a
   leading `/`. Guard the `VERSION="$2"` assignment against a missing operand — under
   `set -u` a bare `--version` currently aborts with an unbound-variable error rather than a
   usage message.
2. `release.yml`: delete the workflow-level `permissions:` block at `:8-10` and add a
   per-job block to each of the nine jobs. Restate `contents: read` on every job that runs
   `actions/checkout` — a job-level block zeroes unlisted scopes, so omitting it breaks
   checkout, and that failure appears only at release time. Add
   `persist-credentials: false` to the checkout in jobs that do not push. Give `ci.yml`'s
   jobs per-job `permissions:` blocks likewise (none needs more than `contents: read`), and
   move `nodejs-addon-glibc.yml`'s workflow-level `contents: read` (`:21-22`) down to its
   single job, so the per-job rule holds across every workflow.
3. Pin the four unpinned tool fetches — `:351` (`npm@latest`), `:354`
   (`pnpm add -g @napi-rs/cli`), `:396` (`npx --yes publint`), `:402`
   (`npx --yes @arethetypeswrong/cli`) — plus the same napi-CLI resolution at `:308`,
   `ci.yml:46`, and `nodejs-addon-glibc.yml:47`; the unfrozen Lambda install at `:449` is
   closed by step 5's committed lockfile and frozen install. Move `publint` and
   `@arethetypeswrong/cli` into `bindings/nodejs` `devDependencies` and into a new
   `validate-npm-package` job holding only `contents: read`; `publish-npm` then
   `needs: [build-nodejs, validate-npm-package]`. Re-check whether the npm upgrade step at
   `:348-351` is needed at all — its comment cites Node 24.8.0, but `actions/setup-node`
   pins `24.18.0`.
4. Add `minimumReleaseAge` to `pnpm-workspace.yaml` explicitly; the file currently sets only
   `minimumReleaseAgeExclude`, so the policy it configures exemptions from is in force by
   default only, and the default does not cover `npm`/`npx` fetches.
5. Regenerate `bindings/nodejs/pnpm-lock.yaml` **first** — `package.json:31` declares
   `"@napi-rs/cli": "^3.7.2"` while the lockfile records `@napi-rs/cli@2.18.4` against a
   `^2` specifier, so `--frozen-lockfile` fails today. Add a committed lockfile for
   `bindings/lambda`, which has none. Then switch `release.yml:318`,
   `nodejs-addon-glibc.yml:52` and `release.yml:449` to `--frozen-lockfile`. In that order;
   the reverse fails.
6. Add the ignore rules to `.gitignore` — its only secret-adjacent rules today are the
   dotenv block at `:105-108`, and it has no rule matching `*.pem`, `keys/`, `data/`,
   `lmdb/`, or `*.db`. `examples/linux-sqlite/setup.sh:9-16` writes `keys/signing-key.pem`
   and `data/` relative to the caller's working directory, so the rules must be
   pattern-based, not path-based. Nothing is currently committed.

**B — Uniform attestation** (`release.yml`, `install.sh`)

7. Add `actions/attest-build-provenance` (SHA-pinned) to `build-binaries` after the checksum
   step at `:120-129`, attesting the built binary's digest, with `id-token: write` and
   `attestations: write` on that job only. Then the same for `build-docker` on the pushed
   image digest at `:183-191`.
8. Add the verification path to `install.sh` between the download at `:76-80` and the
   install at `:101-103`: `gh attestation verify` when `gh` is on `PATH`, otherwise the
   checksum with an explicit "corruption check only, not authenticated" message. Update
   `README.md`, `docs/getting-started/quick-start.md`, and the release-body install snippet
   at `release.yml:590-594` to lead with the verifying path, and document the matching image
   check — `gh attestation verify oci://ghcr.io/antstanley/oidc-exchange:<tag> --repo
   antstanley/oidc-exchange` — beside the `docker pull` instructions, so the container
   channel's attestation also has a stated consumer-side verification.

**C — Advisory gate** (independently shippable)

9. Create `deny.toml` — it does not exist. Add an `advisories` job to `ci.yml` running
   `cargo deny check advisories bans sources`, and the same check as a gate before the
   publish jobs in `release.yml`. Start in reporting mode; it fails against the committed
   lockfile today.
10. Write the six ignore entries with rationales and expiries (the set is listed in the
    `Supply-chain gates` block above; RUSTSEC-2023-0071 is a single entry covering both
    resolved `rsa` versions). Then make the gate blocking for advisories with no entry.
10b. Extend the same job to the other two graphs: `pnpm audit --audit-level high --recursive`
    over the pnpm workspace, and `pip-audit` over the `bindings/python` build environment.
    Both land in reporting mode first, for the same reason as the Cargo check — establish the
    current finding set before making it blocking. The pnpm half depends on step 5's lockfile
    work, since `bindings/lambda` has no lockfile to audit against. Record each graph's
    ignores in its own conventional location (`pnpm audit --ignore-registry-errors` config /
    `pip-audit` ignore list) rather than overloading `deny.toml`, which is Cargo-only.
11. Upgrade `pyo3` past the advisory line in `bindings/python/Cargo.toml:12` — the `"0.22"`
    requirement means no `cargo update` can ever reach a fix (RUSTSEC-2025-0020 is fixed in
    0.24.1, RUSTSEC-2026-0177 in 0.29.0). Confirm the `abi3-py310` feature and the maturin
    build still hold across the major bump before removing the ignore entries.

**D — Signing-path dependency policy** (independently shippable)

12. Add a policy check that the crates on the resolved signing and verification path carry a
    security-support commitment — i.e. are not pre-release. It must reason about the resolved
    graph, not the manifests: `Cargo.lock` carries two coexisting major lines of most of the
    crypto crates (`ed25519-dalek` 2.2.0 and 3.0.0-rc.1, `rsa` 0.9.10 and 0.10.0-rc.18,
    `p256` and `p384` likewise; `p521` resolves only to its release candidate), and the
    stable ones are reached only through `jsonwebtoken` while the workspace's own crates
    link the pre-release ones.

**Regression tests.** A container with neither `sha256sum` nor `shasum` (the sibling change
spec's case); `--version` values containing `../`, a URL, and a leading `/` rejected before
any fetch; a scratch-tag attestation round trip for the binary and the container image
asserting success, then failure after a one-byte edit, then failure against the wrong
repository; `pnpm install --frozen-lockfile`
succeeding in `bindings/nodejs` after regeneration; `cargo deny` failing without `deny.toml`
and passing with the documented ignores; a grep-equivalent assertion that no job holding a
write scope or a publishing identity contains `@latest`, `npx --yes`, or an unfrozen install
— derived from the `permissions:` blocks rather than from hardcoded job names, so it still
holds when a future job is granted `id-token: write`.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`
   to the merge date.
2. Insert the `Supply-chain gates` section into
   [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) after
   `Release pipeline`.
3. No schema change to fold in.
4. If the work packages ship separately, merge this spec only when all four have landed;
   until then leave it `Accepted` and record which packages have shipped in its header.
5. Flip `**Status:**` to `Merged`, stamp `**Merged:** YYYY-MM-DD`, move to
   `.specs/changes/merged/`.
6. Update [`.specs/README.md`](../README.md)'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- The release runs on GitHub-hosted runners, so `actions/attest-build-provenance` can mint
  an OIDC token and reach Sigstore. A self-hosted runner without a workload identity would
  make work package B unavailable and push this change back to Option 1, in which case the
  checksums should at least be published to a different origin than the binaries.
- `bindings/nodejs`'s lockfile can be regenerated against `@napi-rs/cli@^3.7.2` without a
  build break; the napi 2 → 3 gap is a lockfile staleness, not a deliberate pin.
- Nothing in the object graph currently contains key material — the ignore rules are a
  guardrail against a future commit, not a response to a disclosure.

### Decisions

- *Split from the fail-open change.* **The installer's `--version` validation and the
  attestation work live here; its fail-open checksum handling lives in
  [`2026-08-05-fail_closed_across_config_and_adapters.md`](2026-08-05-fail_closed_across_config_and_adapters.md).**
  Both touch `install.sh`, but they are different defects on different axes, and both files
  edit the same function — sequence them, do not merge them.
- *Marvin is specified as a detection gap, not an exposure.* **The `deny.toml` entry for
  RUSTSEC-2023-0071 records that every `RsaPrivateKey` construction is `#[cfg(test)]` and
  gives the claim an expiry.** There is no fixed `rsa` release to move to, so the only
  available control is making the justification falsifiable by CI. Overstating this as a live
  cryptographic exposure would be wrong; leaving it unrecorded would let it silently become
  one.
- *Reporting mode before blocking.* **The advisory gate lands non-blocking with the six
  entries written, then flips to blocking for advisories with no entry.** Turning it on
  blocking first would block every release until six unrelated triages complete.
- *Ignore rules are patterns, not paths.* **`*.pem`, `keys/`, `data/`, `*.db` rather than
  `examples/linux-sqlite/keys/signing-key.pem`.** Five documented flows write key material
  relative to the working directory, so a path rule covers one of them.
- *Attestation, not a signed release manifest.* **Extend the mechanism the pipeline already
  runs on npm and PyPI to the remaining two channels.** A per-tag signed manifest binding
  every artifact across every channel is a refinement of that guarantee and needs a bespoke
  format, a verification tool, and documentation to maintain; the marginal security does not
  justify it at this size.

### Open questions

- Should `install.sh` *require* verification rather than falling back? Requiring it is the
  better security answer and produces support load on hosts without `gh`. The fallback is
  proposed; maintainers who know the consumer base can overrule it.
- Container images: build attestation (one mechanism across all channels) or cosign
  registry signing (more idiomatic for registries)? Both work; this spec proposes attestation
  for uniformity.
- Which of the release-candidate crypto crates are actually on the signing path for each
  deployment mode? `Cargo.lock` carries two major lines of most of them, and the answer
  determines both how urgent work package D is and how its policy check must be written.
- Does `apps/admin-ui` ever become a published artifact? It is absent from `release.yml`
  today. If it becomes one, it needs to be in scope for attestation from the start.
