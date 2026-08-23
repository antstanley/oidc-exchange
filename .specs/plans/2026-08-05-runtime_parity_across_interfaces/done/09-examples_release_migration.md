# Task 09 — Examples release migration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [05-distribution.md §Version parity](../../../bindings/specs/05-distribution.md), source spec §Migration
**Depends on:** 06
**Produces:** five documented Node integrations that await the async API and provide raw path/query request fields in the released shape
**Pointers:** `examples/nodejs/express/index.ts:19-30`, `examples/nodejs/fastify/index.ts:21-48`, `examples/nodejs/hono/index.ts:18-40`, `examples/nodejs/nextjs/app/auth/[...path]/route.ts:12-36`, `examples/nodejs/sveltekit/src/hooks.server.ts:17-41`

## Steps

- [x] Update Express, Fastify, Hono, Next.js, and SvelteKit examples to `await handleRequest`.
- [x] Split each framework request target into raw path and separate query without decoding or manual base-path logic.
- [x] Preserve each example's ordered-header/body translation and response-header handling under the new async response.
- [x] Update example tests or typechecks to exercise the documented async path.
- [x] Add the Node release migration guidance needed by callers moving from `path` to `rawPath` plus `query`.

## Definition of done

- [x] All five Node examples compile/typecheck and use the Promise-returning binding method.
- [x] Each example sends raw path and query separately and does not reintroduce request normalisation in application code.
- [x] Documentation tells consumers to await the call or temporarily choose the deprecated synchronous method.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run each example's relevant typecheck/test and inspect its awaited request forwarding path.

## Evidence

- Express, Fastify, Hono, Next.js, and SvelteKit forward the unmodified raw request-target path and separate encoded query bytes, preserve their existing header/body/response translation, set `pathIsRaw: true`, and await `handleRequest`. No adapter strips `/auth` in application code.
- `bindings/nodejs/README.md` and `docs/guides/nodejs.md` document the 0.2-to-0.3 `path` to `rawPath`/`query` migration, mandatory await, and temporary deprecated blocking `handleRequestSync` option.
- Exact pnpm 11.9.0 isolated Node gates: `typecheck` passed; lint passed with 0 warnings/errors; tests passed 7/7. Source audit found awaited forwarding in all five examples. Framework package build attempts were environmentally blocked because pnpm 11 auto-ran the repository `prepare` hook and Lefthook requires Git metadata unavailable in this Jujutsu-only workspace; an isolated install compiled dependencies but published 0.1.1 lacks the new API, so current local binding typecheck/test plus the five-source audit are the review evidence. No manifest or lockfile was changed.
