# Task 10 — Promote corpus and specs

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [01-ffi-core.md §Conformance corpus](../../../bindings/specs/01-ffi-core.md), [05-distribution.md §Version parity](../../../bindings/specs/05-distribution.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [03-python.md §API](../../../bindings/specs/03-python.md), [04-lambda.md §Event adapters](../../../bindings/specs/04-lambda.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [canonical-types.schema.json §HttpRequest and NormalisationLimits](../../../canonical-types.schema.json), source spec §Migration and §Merge plan
**Depends on:** 04, 07, 08, 09
**Produces:** a passing required conformance gate, updated canonical API/config/schema documentation, and a coordinated breaking-release migration record
**Pointers:** `.github/workflows/ci.yml:1-120`, `Cargo.toml`, `bindings/nodejs/package.json`, `bindings/python/pyproject.toml`, `.specs/canonical-types.schema.json`, `.specs/bindings/specs/`, `.specs/service/specs/`

## Steps

- [ ] Convert the reporting conformance job into a required merge gate after all supported runners agree or carry declared qualifications.
- [ ] Apply the source change blocks to each affected canonical bindings/service page and fold the new wire request and limits definitions into the canonical schema.
- [ ] Bump the three version-parity manifests together from the current 0.2 line to 0.3.0 and add release notes for Node, Python, and Lambda migration.
- [ ] Record the deprecation window and explicitly defer entry-point deletion until the following major cycle.
- [ ] Update the change-spec merge metadata/index only when implementation is accepted for merge; do not merge or move the source spec in this planning PR.

## Definition of done

- [ ] CI treats the conformance job as required and it verifies every supported runner/fixture pairing with declared qualifications only where host fidelity is unavailable.
- [ ] Canonical prose, schema, and version-parity manifests describe the same wire API, body limit, panic-stack, and async migration.
- [ ] Release notes state the Node Promise migration, Python ordered-header/path migration, and Lambda behaviour change.
- [ ] The later removal of deprecated methods is tracked as intentionally deferred, not silently omitted.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: inspect a green required conformance run and the coordinated schema/spec/version/release-note diff.
