# Lambda Binding (`@oidc-exchange/lambda`)

**Status:** Implemented · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Scope:** bindings/lambda

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
