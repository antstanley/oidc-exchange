# Task 09 — Examples release migration

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [02-nodejs.md §API](../../../bindings/specs/02-nodejs.md), [05-distribution.md §Version parity](../../../bindings/specs/05-distribution.md), source spec §Migration
**Depends on:** 06
**Produces:** five documented Node integrations that await the async API and provide raw path/query request fields in the released shape
**Pointers:** `examples/nodejs/express/index.ts:19-30`, `examples/nodejs/fastify/index.ts:21-48`, `examples/nodejs/hono/index.ts:18-40`, `examples/nodejs/nextjs/app/auth/[...path]/route.ts:12-36`, `examples/nodejs/sveltekit/src/hooks.server.ts:17-41`

## Steps

- [ ] Update Express, Fastify, Hono, Next.js, and SvelteKit examples to `await handleRequest`.
- [ ] Split each framework request target into raw path and separate query without decoding or manual base-path logic.
- [ ] Preserve each example's ordered-header/body translation and response-header handling under the new async response.
- [ ] Update example tests or typechecks to exercise the documented async path.
- [ ] Add the Node release migration guidance needed by callers moving from `path` to `rawPath` plus `query`.

## Definition of done

- [ ] All five Node examples compile/typecheck and use the Promise-returning binding method.
- [ ] Each example sends raw path and query separately and does not reintroduce request normalisation in application code.
- [ ] Documentation tells consumers to await the call or temporarily choose the deprecated synchronous method.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run each example's relevant typecheck/test and inspect its awaited request forwarding path.
