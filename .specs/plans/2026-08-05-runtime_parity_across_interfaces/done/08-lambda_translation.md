# Task 08 — Lambda translation

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [04-lambda.md §Responsibilities](../../../bindings/specs/04-lambda.md), [04-lambda.md §Event adapters](../../../bindings/specs/04-lambda.md), [04-lambda.md §Decisions](../../../bindings/specs/04-lambda.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6–7
**Depends on:** 05
**Produces:** API Gateway v1/v2 and ALB event translation that supplies the Node wire API with rawest available fields and never strips, decodes, or re-splices a path
**Pointers:** `bindings/lambda/src/adapters.ts:23-149`, `bindings/lambda/src/index.ts:58-105`, `bindings/lambda/src/types.ts:1-20`, `bindings/lambda/__tests__/`

## Steps

- [x] Redefine adapter output around Node `HttpRequest` raw path, separate query, ordered headers, bounded body, and `pathIsRaw`.
- [x] Remove all three local base-path strips and forward `basePath` to the FFI/Node instance at construction.
- [x] Translate API Gateway v1/v2 and ALB multi-value headers/query fields once, preserving available order and marking pre-decoded sources as non-raw.
- [x] Bound base64/UTF-8 decoding against `limits().maxBodyBytes` before handing data to the binding.
- [x] Add synthetic-event tests for prefix siblings, encoded delimiters, duplicated headers, raw-path hints, malformed/oversized bodies, and adapter-to-corpus records.

## Definition of done

- [x] No Lambda adapter contains prefix stripping, URI reconstruction, or local header deduplication.
- [x] `/authorize` with configured `/auth` remains a clean non-mangled routing result across all three event shapes.
- [x] Base64 and text bodies above the published cap receive 413 without a larger decoded allocation.
- [x] Every event source truthfully supplies its raw-path hint and qualified corpus expectation.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run synthetic v1/v2/ALB events through `createHandler` and compare their corpus records.

## Evidence

- Exact `pnpm@11.9.0 --ignore-workspace` gates: lint 0 warnings/errors; TypeScript typecheck clean; Vitest 11/11 passed.
- API Gateway v2 forwards `rawPath` and `rawQueryString` with `pathIsRaw=true`; v1 and ALB path fields are marked non-raw. Multi-value header/query iteration preserves the source array order and never merges single and multi forms.
- Body preflight checks UTF-8 byte length and base64 encoded-size bounds before decoding, then verifies decoded length; `createHandler` returns 413 for `BodyTooLargeError`.
- `node conformance/report.mjs`: 12 fixtures/6 shapes; final known differences native 0, FFI 4, Node 4, Lambda 4, ASGI 6, WSGI 7. Reporting remains intentionally non-gating until task 10.
- Final Rust gates: fmt clean; workspace clippy clean; nextest 405 passed, 0 failed, 27 skipped.
- Qualification: `basePath` remains an accepted deployment option but translation never strips or decodes it; service configuration/shared middleware owns base-path routing. AWS v1/ALB provide decoded path fields and cannot claim raw fidelity.

## PR #27 F7 follow-up evidence

Production `translateResponse` coverage proves API Gateway v2 emits ordered repeated Set-Cookie values via `cookies` and joins duplicate ordinary headers in order; API Gateway v1 and ALB preserve ordered repeats in `multiValueHeaders`, retain single-value `headers`, status, and base64 body. `createHandler` invokes this translator directly.
