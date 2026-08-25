# @oidc-exchange/lambda

[![npm](https://img.shields.io/npm/v/@oidc-exchange/lambda)](https://www.npmjs.com/package/@oidc-exchange/lambda)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/antstanley/oidc-exchange/blob/main/LICENSE)

AWS Lambda adapter for [**oidc-exchange**](https://github.com/antstanley/oidc-exchange). It wraps [`@oidc-exchange/node`](https://www.npmjs.com/package/@oidc-exchange/node) and translates Lambda HTTP events into requests for the embedded token-exchange service — automatically detecting **API Gateway v1**, **API Gateway v2 / Function URL**, and **ALB** event shapes.

## Install

```bash
npm install @oidc-exchange/lambda @oidc-exchange/node
```

`@oidc-exchange/node` is a peer dependency (it carries the native binary). Requires Node.js ≥ 22.

## Usage

```ts
import { createHandler } from "@oidc-exchange/lambda";

export const handler = createHandler({
  config: "./config.toml",
  basePath: "/auth", // optional: strip a stage/route prefix before routing
});
```

Inline configuration instead of a file:

```ts
export const handler = createHandler({
  configString: `
    [server]
    issuer = "https://auth.example.com"
    …
  `,
});
```

`createHandler(options)` returns an `async (event, context) => result` handler. `options` are the same as [`OidcExchange`](https://www.npmjs.com/package/@oidc-exchange/node) (`config` / `configString`) plus an optional `basePath`.

## Configuration behaviour change

Handler construction now fails when a `${VAR}` placeholder is unresolved, empty, or malformed
instead of using that placeholder as literal configuration text. Set every referenced environment
variable in the Lambda runtime before calling `createHandler`.

## Deploy

See the [AWS Lambda deployment guide](https://github.com/antstanley/oidc-exchange/blob/main/docs/integration/aws-lambda.md) and the [Lambda example](https://github.com/antstanley/oidc-exchange/tree/main/examples/nodejs/lambda). The same service also runs as a long-lived server — see the [main repository](https://github.com/antstanley/oidc-exchange).

## Links

- [Repository & full docs](https://github.com/antstanley/oidc-exchange)
- [Configuration](https://github.com/antstanley/oidc-exchange#configuration)

Published from CI with npm provenance. MIT licensed.
