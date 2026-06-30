# Done Certificate — Task 01: pyproject abi3 manifest

**Task:** [01-pyproject_abi3_manifest.md](01-pyproject_abi3_manifest.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-06-30   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names (a parsed manifest, a built wheel filename, or a command result) — not by assertion.

## Premises

- **P1 — Goal.** The task makes `bindings/python/pyproject.toml` `[tool.maturin] features` declare
  `pyo3/abi3-py310` alongside `pyo3/extension-module`, confirms `Cargo.toml` already enables the
  same feature, and verifies a local `maturin build` emits a single `cp310-abi3` wheel — without
  changing `pyproject.toml` `version`.
- **P2 — Obligations.** Done iff O1…O5 all hold. One Oi per definition-of-done item, in DoD order;
  O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not change `pyproject.toml` `version`, the `requires-python`/classifier
  set, the Python or Rust source, or the version-parity invariant the release `validate` job
  enforces across `Cargo.toml` / `bindings/nodejs/package.json` / `bindings/python/pyproject.toml`.

## Obligations

- **O1 — maturin features declare abi3-py310 and agree with Cargo.toml.**
  - *Claim:* `bindings/python/pyproject.toml` `[tool.maturin] features` lists both
    `pyo3/extension-module` and `pyo3/abi3-py310`; `Cargo.toml`'s `pyo3` features still include
    `abi3-py310`, so the two agree.
  - *Evidence to collect:* read `bindings/python/pyproject.toml` `[tool.maturin]` block (around
    `:31`-`:34`) and confirm `features` contains both strings; read `bindings/python/Cargo.toml:12`
    and confirm `pyo3`'s `features` array still contains `"abi3-py310"`.
  - *Checks:* confirm no conflicting feature (e.g. a second pyo3 abi3-pyXYZ pinned to a different
    minor) is introduced.
  - *Collected:* `bindings/python/pyproject.toml:32` reads
    `features = ["pyo3/extension-module", "pyo3/abi3-py310"]` — both required strings present.
    `bindings/python/Cargo.toml:12` reads
    `pyo3 = { version = "0.22", features = ["extension-module", "abi3-py310"] }` — `abi3-py310`
    still enabled at the crate level. The two agree.
  - *Check result:* the `features` array contains exactly the two expected strings; no second
    `pyo3/abi3-pyXYZ` pinned to a different minor, no duplicate `[tool.maturin]` table. No conflict.
  - *Status:* ☑ SATISFIED

- **O2 — A local build emits a single cp310-abi3 wheel.**
  - *Claim:* `maturin build --release` in `bindings/python` emits exactly one wheel whose filename
    carries the `cp310-abi3` ABI tag.
  - *Evidence to collect:* run `uvx maturin build --release` (or `uv run maturin build --release`)
    in `bindings/python` and list `target/wheels/*.whl`; confirm the filename matches
    `oidc_exchange-*-cp310-abi3-*.whl` and that exactly one wheel was produced for the host.
  - *Checks:* negative space — a bare `cp310-cp310` (non-abi3) tag, or more than one wheel for the
    host interpreter, is a defect.
  - *Collected:* inspected the already-built artifact at
    `/Volumes/Delorean/code/oidc-exchange-pytask-01/target/wheels/` — `ls *.whl` returns exactly
    **one** file: `oidc_exchange-0.1.0-cp310-abi3-macosx_11_0_arm64.whl` (host = arm64 macOS). The
    abi3 tag was verified independently of the filename: `unzip -p … '*.dist-info/WHEEL'` reports
    `Tag: cp310-abi3-macosx_11_0_arm64` (Generator: maturin 1.14.1).
  - *Check result:* tag is `cp310-abi3`, **not** the bare `cp310-cp310` defect; exactly one wheel
    for the host. PASS.
  - *Status:* ☑ SATISFIED

- **O3 — Version parity holds; version unchanged.**
  - *Claim:* `pyproject.toml` `version` is unchanged and equals `Cargo.toml`
    `workspace.package.version` and `bindings/nodejs/package.json` `version`.
  - *Evidence to collect:* `grep '^version' bindings/python/pyproject.toml`,
    `grep -A2 'workspace.package' Cargo.toml` (or the workspace `version`), and
    `jq -r '.version' bindings/nodejs/package.json` — expect all three identical; confirm via
    `jj diff` that `pyproject.toml` `version` was not touched.
  - *Collected:* `bindings/python/pyproject.toml` `version = "0.1.0"`; workspace `Cargo.toml`
    `version = "0.1.0"`; `jq -r .version bindings/nodejs/package.json` → `0.1.0`. All three
    identical. `jj diff` shows the only change is the `[tool.maturin] features` line; `version`
    (line 7) is untouched, as are `requires-python` and the classifier set.
  - *Status:* ☑ SATISFIED

- **O4 — Meets the repo definition of done for what the task touches.**
  - *Claim:* the Python format/lint gate passes; no Python/Rust source changed so type/test gates
    are unaffected.
  - *Evidence to collect:* per `.specs/development-guidelines.md` §Definition of done, run
    `uv run ruff format --check .` and `uv run ruff check .` in `bindings/python` — expect clean;
    confirm via `jj diff --name-only` that only `bindings/python/pyproject.toml` (and possibly
    `uv.lock`) changed, with no `.py`/`.rs` source touched.
  - *Collected:* in `bindings/python`, `uv run ruff format --check .` → exit 0 (`7 files already
    formatted`); `uv run ruff check .` → exit 0 (`All checks passed!`). `jj diff --name-only`
    returns exactly one path: `bindings/python/pyproject.toml` — no `uv.lock`, no `.py`/`.rs`
    source touched, so pyright/pytest are unaffected.
  - *Status:* ☑ SATISFIED

- **O5 — Reviewable: a reviewer confirms the abi3 feature and the cp310-abi3 wheel (Reviewable).**
  - *Claim:* a reviewer reads the updated `[tool.maturin]` block, confirms it names
    `pyo3/abi3-py310` and agrees with `Cargo.toml`, and runs `maturin build` to confirm the emitted
    wheel is `cp310-abi3`.
  - *Evidence to collect:* read the diff of `pyproject.toml`; run `maturin build --release` and
    read the wheel filename — expect a single `cp310-abi3` wheel.
  - *Collected:* read `jj diff` — the single hunk sets
    `[tool.maturin] features = ["pyo3/extension-module", "pyo3/abi3-py310"]`, which names
    `pyo3/abi3-py310` and agrees with `Cargo.toml:12`. Exercised the wheel evidence per O2: a
    single wheel `oidc_exchange-0.1.0-cp310-abi3-macosx_11_0_arm64.whl` whose internal `WHEEL`
    metadata records `Tag: cp310-abi3-macosx_11_0_arm64`. Review action performed, not assumed.
  - *Status:* ☑ SATISFIED

## Regression check

- The release `validate` job greps `^version` in `pyproject.toml`; changing only `[tool.maturin]
  features` must not change `version`. Trace: `grep '^version' bindings/python/pyproject.toml`
  still returns `version = "0.1.0"` (the parity version) and `jj diff` shows `version` untouched :
  ☑ PRESERVED
- `Cargo.toml` already enabling `abi3-py310` means adding it to maturin features must not produce a
  feature conflict at build time. Trace: the `--release` build completed and emitted a valid wheel
  (`oidc_exchange-0.1.0-cp310-abi3-macosx_11_0_arm64.whl`) with no cargo feature error :
  ☑ PRESERVED

## Residue

- Whether the manylinux build (task 02) strictly needs the maturin-level feature is moot —
  `Cargo.toml` already enables abi3, so this task is belt-and-suspenders plus the abi3-tag
  verification. Refreshing `uv.lock` is an acceptable side effect as long as it stays the record.
  (Observed: no `uv.lock` change was needed — the diff is the single `features` line only.)

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with evidence (pyproject.toml:32 + Cargo.toml:12 agree on abi3-py310; a single inspected wheel carries internal Tag cp310-abi3-macosx_11_0_arm64; version parity 0.1.0 across all three manifests; ruff format/check clean; only pyproject.toml changed), and both regression traces are PRESERVED.
