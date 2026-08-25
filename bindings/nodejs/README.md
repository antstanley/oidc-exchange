# @oidc-exchange/node

[![npm](https://img.shields.io/npm/v/@oidc-exchange/node)](https://www.npmjs.com/package/@oidc-exchange/node)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/antstanley/oidc-exchange/blob/main/LICENSE)

Native Node.js binding for [**oidc-exchange**](https://github.com/antstanley/oidc-exchange) — a Rust service that validates ID tokens from third-party OIDC providers (Google, Apple, …) and exchanges them for self-issued access and refresh tokens.

The whole service is embedded in-process as a native addon (built with [napi-rs](https://napi.rs)): you route HTTP requests through it and get HTTP responses back — no subprocess, no extra network hop.

## Install

```bash
npm install @oidc-exchange/node
```

The prebuilt native binary for your platform is installed automatically via `optionalDependencies`. Supported platforms:

| Platform            | Package                          |
| ------------------- | -------------------------------- |
| Linux x64 (glibc)   | `@oidc-exchange/linux-x64-gnu`   |
| Linux arm64 (glibc) | `@oidc-exchange/linux-arm64-gnu` |
| Windows x64         | `@oidc-exchange/win32-x64-msvc`  |
| macOS arm64         | `@oidc-exchange/darwin-arm64`    |

Requires Node.js ≥ 22.

## Usage

Construct the service with a TOML config (a file path or an inline string), then hand it HTTP requests:

```ts
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: "./config.toml" });
// or: new OidcExchange({ configString: "[server]\nissuer = \"https://auth.example.com\"\n…" })

const res = await oidc.handleRequest({
  method: "POST",
  rawPath: Buffer.from("/token"),
  query: undefined,
  headers: [{ name: "content-type", value: "application/x-www-form-urlencoded" }],
  body: Buffer.from(
    new URLSearchParams({
      grant_type: "authorization_code",
      code,
      redirect_uri: "https://app.example.com/callback",
      provider: "google",
    }).toString(),
  ),
  pathIsRaw: true,
});

console.log(res.status); // e.g. 200
console.log(res.body.toString("utf8")); // { "access_token": …, "refresh_token": … }
```

### API

```ts
class OidcExchange {
  constructor(options: { config?: string; configString?: string });
  handleRequest(request: HttpRequest): Promise<HttpResponse>;
  /** @deprecated Await handleRequest instead. */
  handleRequestSync(request: HttpRequest): HttpResponse;
  limits(): { maxBodyBytes: number };
  shutdown(): void; // graceful shutdown (currently a no-op)
}

interface HttpRequest {
  method: string;
  rawPath: Buffer;
  query?: Buffer;
  headers: { name: string; value: string }[];
  body?: Buffer;
  pathIsRaw: boolean;
}
interface HttpResponse {
  status: number;
  headers: { name: string; value: string }[];
  body: Buffer;
}
```

### Migrating from 0.2

`handleRequest` now returns a Promise, so await it. Replace the old combined `path` field with `rawPath` containing the still-percent-encoded path bytes and `query` containing the still-percent-encoded query bytes without a leading `?`; set `pathIsRaw` to `false` only when the host has already decoded the path. Do not strip the configured base path or decode request targets in application code. If an architecture must remain synchronous temporarily, use deprecated `handleRequestSync` with the same wire request shape; it blocks the calling thread and is scheduled for removal after one major release cycle.

`handleRequest` exposes the full service — `/token`, `/revoke`, `/keys`, `/.well-known/openid-configuration`, `/health`, and the internal admin API. See the [HTTP API reference](https://github.com/antstanley/oidc-exchange#api-endpoints) for request/response shapes.

## Framework adapters

Wiring for popular Node servers lives in the main repo's [examples](https://github.com/antstanley/oidc-exchange/tree/main/examples/nodejs) — Express, Fastify, Hono, Next.js, SvelteKit, serverless. For AWS Lambda, use [`@oidc-exchange/lambda`](https://www.npmjs.com/package/@oidc-exchange/lambda).

## Configuration

Configuration is TOML — providers, token TTLs, registration policy, key management, and storage. See the [configuration guide](https://github.com/antstanley/oidc-exchange#configuration).

### Behaviour change

Construction now fails when a `${VAR}` placeholder is unresolved, empty, or malformed instead of
using that placeholder as literal configuration text. Set every referenced environment variable
before constructing `OidcExchange`.

## Links

- [Repository & full docs](https://github.com/antstanley/oidc-exchange)
- [Why oidc-exchange?](https://github.com/antstanley/oidc-exchange#why-oidc-exchange)
- [Deployment guides](https://github.com/antstanley/oidc-exchange/tree/main/docs/integration)

Published from CI with npm provenance. MIT licensed.
