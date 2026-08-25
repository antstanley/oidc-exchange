# Lambda Binding (`@oidc-exchange/lambda`)

**Status:** Implemented · **Date:** 2026-08-23 · **Owner:** Ant Stanley · **Scope:** bindings/lambda

A pure-TypeScript adapter that turns AWS Lambda HTTP events into calls on the
[Node.js binding](02-nodejs.md). It contains no Rust; it depends on `@oidc-exchange/node`.

## Responsibilities

- Detect the incoming event shape (API Gateway REST v1, HTTP API v2 / Function URL, or ALB).
- Normalise it to the Node binding's `HttpRequest`, call `handleRequest`, and shape the result
  back into the matching Lambda response type.

## API (`src/index.ts`, `src/types.ts`)

```typescript
interface LambdaHandlerOptions extends OidcExchangeOptions { basePath?: string }  // default ""

function createHandler(
  options: LambdaHandlerOptions
): (event: LambdaEvent, context: Context) => Promise<LambdaResult>;

// LambdaEvent  = APIGatewayProxyEvent | APIGatewayProxyEventV2 | ALBEvent
// LambdaResult = APIGatewayProxyResult | APIGatewayProxyResultV2 | ALBResult
```

`createHandler` constructs one `OidcExchange` from the options and returns a handler. Per
invocation it normalises the event, calls `handleRequest`, converts response headers to a plain
object, base64-encodes the body, and returns the shape appropriate to the detected event type.

## Event adapters (`src/adapters.ts`)

- **Detection** — `isApiGatewayV1` (`httpMethod` + `resource`, no `version`), `isApiGatewayV2`
  (`version === "2.0"`), `isAlbEvent` (`requestContext.elb`).
- **Normalisation** — `fromApiGatewayV1`, `fromApiGatewayV2`, `fromAlbEvent` each strip
  `basePath`, append the query string (`queryStringParameters` / `rawQueryString`), flatten
  single- and multi-value headers, and base64-decode the body when `isBase64Encoded`.
- **Helpers** — `flattenHeaders(single, multi)` merges header maps; `decodeBody(body,
  isBase64Encoded)` decodes base64 or UTF-8.

## Distribution

Built with `tsc` to `dist/` (JS + `.d.ts`). Depends on `@oidc-exchange/node`;
`@types/aws-lambda` is a dev dependency. A pnpm workspace member, not a Cargo member.

## Tests

`__tests__/adapters.test.ts` (vitest): event detection for all three shapes and normalisation
behaviour (basePath stripping, query strings, header flattening, base64 bodies).

## Assumptions and open questions

### Assumptions

- The deployment mounts the function under a stage/base path that matches `basePath` so the
  service sees unprefixed routes.

### Decisions

- *TypeScript over the Node binding.* **The Lambda adapter reuses `@oidc-exchange/node` rather
  than the FFI directly.** Event shaping is pure data transformation best done in TS, and it
  inherits the native binary distribution for free.
- *Three event shapes, one handler.* **A single `createHandler` covers REST, HTTP API/Function
  URL, and ALB.** Detection at runtime avoids a per-trigger entry point.

### Open questions

- (None at this stage.)


## Runtime parity update

- Detect the incoming event shape (API Gateway REST v1, HTTP API v2 / Function URL, or ALB).
- Translate it to the Node binding's `HttpRequest` — event field to request field, and
  nothing more. The adapter performs no path stripping, no query re-encoding, and no header
  deduplication; those belong to the normaliser, which already implements them once and
  correctly.
- **Detection** — `isApiGatewayV1` (`httpMethod` + `resource`, no `version`), `isApiGatewayV2`
  (`version === "2.0"`), `isAlbEvent` (`requestContext.elb`).
- **Translation** — `fromApiGatewayV1`, `fromApiGatewayV2`, `fromAlbEvent` each read the
  rawest path the event carries (`rawPath` for v2, `path` for v1 and ALB, with
  `pathIsRaw: false` for the two sources that pre-decode), pass the query string through
  unmodified (`rawQueryString` for v2; `multiValueQueryStringParameters` re-encoded once for
  v1 and ALB), emit headers as ordered pairs preserving multi-value entries, and base64-decode
  the body when `isBase64Encoded`.
- **Base path** — `createHandler`'s `basePath` option is forwarded to the FFI instance and
  applied by `crates/server`'s segment-aware strip. The adapter no longer contains a strip of
  its own, so `/authorize` under `basePath: "/auth"` routes exactly as it does on the
  standalone server: a clean `404`, not a mangled `orize` or a `502` from an uncaught request
  build error.
- **Helpers** — `decodeBody(body, isBase64Encoded)` decodes base64 or UTF-8 and refuses a
  body above `limits().maxBodyBytes` with a `413` before it is handed across the boundary.
- *No control logic in the adapter.* **Prefix stripping, path decoding, and header handling
  live in Rust; the adapter only maps event fields.** Three hand-maintained copies of one
  control drifted from the Rust original that they were copied from; the way to keep them in
  step is for them not to exist.
