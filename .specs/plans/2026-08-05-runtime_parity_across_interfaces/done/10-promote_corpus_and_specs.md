# Task 10 — Promote corpus and specs

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Conformance corpus](../../../bindings/specs/01-ffi-core.md), [05-distribution.md §Version parity](../../../bindings/specs/05-distribution.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [03-python.md §API](../../../bindings/specs/03-python.md), [04-lambda.md §Event adapters](../../../bindings/specs/04-lambda.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [canonical-types.schema.json §HttpRequest and NormalisationLimits](../../../canonical-types.schema.json), source spec §Migration and §Merge plan
**Depends on:** 04, 07, 08, 09
**Produces:** a passing required conformance gate, updated canonical API/config/schema documentation, and a coordinated breaking-release migration record
**Pointers:** `.github/workflows/ci.yml:1-120`, `Cargo.toml`, `bindings/nodejs/package.json`, `bindings/python/pyproject.toml`, `.specs/canonical-types.schema.json`, `.specs/bindings/specs/`, `.specs/service/specs/`

## Steps

- [x] Convert the reporting conformance job into a required merge gate after all supported runners agree or carry declared qualifications.
- [x] Apply the source change blocks to each affected canonical bindings/service page and fold the new wire request and limits definitions into the canonical schema.
- [x] Bump the three version-parity manifests together from the current 0.2 line to 0.3.0 and add release notes for Node, Python, and Lambda migration.
- [x] Record the deprecation window and explicitly defer entry-point deletion until the following major cycle.
- [x] Update the change-spec merge metadata/index only when implementation is accepted for merge; do not merge or move the source spec in this planning PR.

## Definition of done

- [x] CI treats the conformance job as required and it verifies every supported runner/fixture pairing with declared qualifications only where host fidelity is unavailable.
- [x] Canonical prose, schema, and version-parity manifests describe the same wire API, body limit, panic-stack, and async migration.
- [x] Release notes state the Node Promise migration, Python ordered-header/path migration, and Lambda behaviour change.
- [x] The later removal of deprecated methods is tracked as intentionally deferred, not silently omitted.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: inspect a green required conformance run and the coordinated schema/spec/version/release-note diff.

## Audit / evidence

- Required CI: `.github/workflows/ci.yml` has `Conformance` without `continue-on-error`; it pins pnpm 11.9.0, builds the napi and PyO3 artifacts plus Lambda TypeScript, and runs `node conformance/report.mjs`.
- Executable parity: 72/72 production runner/fixture pairings executed; native 12/0 qualifications, FFI 12/0, Node 12/0, Lambda 12/8, ASGI 12/6, WSGI 12/6; zero unqualified method/path/query/header/status mismatches.
- Canonical targets: all eight affected prose pages dated 2026-08-23 carry the source change blocks; `canonical-types.schema.json` replaces `HttpRequest` and adds `NormalisationLimits`. Prose consistently records the 2 MiB host-prebuffer cap, segment-aware base path, three-level panic containment, and async migration.
- Version parity: workspace Cargo, Node package, and Python project manifests are all `0.3.0`; Cargo.lock was naturally regenerated.
- Migration: `RELEASE_NOTES.md` records Node Promise/wire-shape migration, Python ordered-header/raw-path migration, Lambda base-path behaviour, and intentionally deferred removal after the following major cycle.
- Source lifecycle: `.specs/changes/2026-08-05-runtime_parity_across_interfaces.md` remains Proposed and unmoved; `.specs/README.md` therefore remains correctly indexed as proposed.
- Zero done certificates: no certificate was introduced; task 10 itself is the reviewable completion record.
- F3 review evidence confirms the documented three-level panic stack at its shared FFI boundary: injected router-future and response-body polling panics cannot unwind through async `handle` or the deprecated synchronous trampoline, and map to the generic safe 500 without panic, token, subject, or invalid request-ID reflection.
