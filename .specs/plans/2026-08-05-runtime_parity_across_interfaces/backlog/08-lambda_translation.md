# Task 08 — Lambda translation

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [04-lambda.md §Responsibilities](../../../bindings/specs/04-lambda.md), [04-lambda.md §Event adapters](../../../bindings/specs/04-lambda.md), [04-lambda.md §Decisions](../../../bindings/specs/04-lambda.md), [00-overview.md §Shape](../../../bindings/specs/00-overview.md), source spec §Implementation notes 6–7
**Depends on:** 05
**Produces:** API Gateway v1/v2 and ALB event translation that supplies the Node wire API with rawest available fields and never strips, decodes, or re-splices a path
**Pointers:** `bindings/lambda/src/adapters.ts:23-149`, `bindings/lambda/src/index.ts:58-105`, `bindings/lambda/src/types.ts:1-20`, `bindings/lambda/__tests__/`

## Steps

- [ ] Redefine adapter output around Node `HttpRequest` raw path, separate query, ordered headers, bounded body, and `pathIsRaw`.
- [ ] Remove all three local base-path strips and forward `basePath` to the FFI/Node instance at construction.
- [ ] Translate API Gateway v1/v2 and ALB multi-value headers/query fields once, preserving available order and marking pre-decoded sources as non-raw.
- [ ] Bound base64/UTF-8 decoding against `limits().maxBodyBytes` before handing data to the binding.
- [ ] Add synthetic-event tests for prefix siblings, encoded delimiters, duplicated headers, raw-path hints, malformed/oversized bodies, and adapter-to-corpus records.

## Definition of done

- [ ] No Lambda adapter contains prefix stripping, URI reconstruction, or local header deduplication.
- [ ] `/authorize` with configured `/auth` remains a clean non-mangled routing result across all three event shapes.
- [ ] Base64 and text bodies above the published cap receive 413 without a larger decoded allocation.
- [ ] Every event source truthfully supplies its raw-path hint and qualified corpus expectation.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run synthetic v1/v2/ALB events through `createHandler` and compare their corpus records.
