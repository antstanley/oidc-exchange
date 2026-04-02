---
title: Node.js
description: Use oidc-exchange as an embedded OIDC provider in Node.js applications.
---

## Installation

```bash
pnpm add @oidc-exchange/node
```

Requires **Node.js 22+**. Prebuilt native binaries are included for Linux (x64, ARM64), macOS (ARM64), and Windows (x64).

For AWS Lambda deployments, install the Lambda adapter instead:

```bash
pnpm add @oidc-exchange/lambda
```

## Basic Usage

```typescript
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: "./config.toml" });

const response = oidc.handleRequest({
  method: "GET",
  path: "/health",
  headers: [],
});

console.log(response.status); // 200
```

The `handleRequest` method takes `{ method, path, headers, body? }` and returns `{ status, headers, body }`. Headers are arrays of `{ name, value }` objects.

## Framework Integration

### Express

```javascript
const express = require("express");
const { OidcExchange } = require("@oidc-exchange/node");

const app = express();
const oidc = new OidcExchange({ config: "./config.toml" });

app.all("/auth/*", (req, res) => {
  const path = req.originalUrl.replace(/^\/auth/, "") || "/";
  const headers = [];
  const raw = req.rawHeaders;
  for (let i = 0; i < raw.length; i += 2) {
    headers.push({ name: raw[i], value: raw[i + 1] });
  }

  const chunks = [];
  req.on("data", (chunk) => chunks.push(chunk));
  req.on("end", () => {
    const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined;
    const response = oidc.handleRequest({ method: req.method, path, headers, body });
    for (const { name, value } of response.headers) {
      res.setHeader(name, value);
    }
    res.status(response.status).end(response.body);
  });
});

app.listen(3000);
```

### Hono

```typescript
import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { OidcExchange } from "@oidc-exchange/node";

const app = new Hono();
const oidc = new OidcExchange({ config: "./config.toml" });

app.all("/auth/*", async (c) => {
  const path = c.req.path.replace("/auth", "") || "/";
  const headers = [...c.req.raw.headers.entries()].map(([name, value]) => ({ name, value }));
  const body = Buffer.from(await c.req.arrayBuffer());

  const response = oidc.handleRequest({ method: c.req.method, path, headers, body });

  return new Response(response.body, {
    status: response.status,
    headers: Object.fromEntries(response.headers.map((h) => [h.name, h.value])),
  });
});

serve({ fetch: app.fetch, port: 3000 });
```

### Fastify

```javascript
const Fastify = require("fastify");
const { OidcExchange } = require("@oidc-exchange/node");

const fastify = Fastify();
const oidc = new OidcExchange({ config: "./config.toml" });

fastify.all("/auth/*", async (request, reply) => {
  const path = request.url.replace(/^\/auth/, "") || "/";
  const headers = Object.entries(request.headers).map(([name, value]) => ({
    name,
    value: String(value),
  }));

  const response = oidc.handleRequest({ method: request.method, path, headers });
  reply.status(response.status);
  for (const { name, value } of response.headers) {
    reply.header(name, value);
  }
  reply.send(response.body);
});

fastify.listen({ port: 3000 });
```

### Next.js (App Router)

```typescript
// app/auth/[...path]/route.ts
import { OidcExchange } from "@oidc-exchange/node";
import { NextRequest, NextResponse } from "next/server";

const oidc = new OidcExchange({ config: "./config.toml" });

async function handler(req: NextRequest) {
  const url = new URL(req.url);
  const path = url.pathname.replace("/auth", "") || "/";
  const headers = [...req.headers.entries()].map(([name, value]) => ({ name, value }));
  const body = req.body ? Buffer.from(await req.arrayBuffer()) : undefined;

  const response = oidc.handleRequest({ method: req.method, path, headers, body });

  return new NextResponse(response.body, {
    status: response.status,
    headers: Object.fromEntries(response.headers.map((h) => [h.name, h.value])),
  });
}

export { handler as GET, handler as POST };
```

### SvelteKit

```typescript
// src/hooks.server.ts
import { OidcExchange } from "@oidc-exchange/node";
import type { Handle } from "@sveltejs/kit";

const oidc = new OidcExchange({ config: "./config.toml" });

export const handle: Handle = async ({ event, resolve }) => {
  if (event.url.pathname.startsWith("/auth")) {
    const path = event.url.pathname.replace("/auth", "") || "/";
    const headers = [...event.request.headers.entries()].map(([name, value]) => ({ name, value }));
    const body = event.request.body ? Buffer.from(await event.request.arrayBuffer()) : undefined;

    const response = oidc.handleRequest({ method: event.request.method, path, headers, body });

    return new Response(response.body, {
      status: response.status,
      headers: Object.fromEntries(response.headers.map((h) => [h.name, h.value])),
    });
  }
  return resolve(event);
};
```

### AWS Lambda

Use the `@oidc-exchange/lambda` package for serverless deployments. It automatically detects the event source — API Gateway v1 (REST API), API Gateway v2 (HTTP API), Lambda Function URL, or ALB.

```typescript
import { createHandler } from "@oidc-exchange/lambda";

export const handler = createHandler({
  config: "./config.toml",
  basePath: "/auth",
});
```

That's it. The handler translates Lambda events into HTTP requests, routes them through oidc-exchange, and returns the appropriate Lambda response format.

Works with SAM, CDK, Serverless Framework, Terraform, or any other deployment tool.

## Configuration

Pass config as a file path or inline TOML string:

```typescript
// File path
const oidc = new OidcExchange({ config: "./config.toml" });

// Inline TOML
const oidc = new OidcExchange({
  configString: `
[server]
issuer = "https://auth.example.com"
role = "exchange"
[repository]
adapter = "sqlite"
[repository.sqlite]
path = ":memory:"
  `,
});
```

See the [Configuration guide](/guides/configuration) for all available options.
