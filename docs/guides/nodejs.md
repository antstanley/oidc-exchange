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

```typescript
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import express from "express";
import { OidcExchange } from "@oidc-exchange/node";

const __dirname = dirname(fileURLToPath(import.meta.url));
const oidc = new OidcExchange({ config: resolve(__dirname, "..", "config.toml") });
const app = express();

app.all("/auth/*", (req, res) => {
  const chunks: Buffer[] = [];
  req.on("data", (chunk: Buffer) => chunks.push(chunk));
  req.on("end", () => {
    const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined;
    const headers = [];
    const raw = req.rawHeaders;
    for (let i = 0; i < raw.length; i += 2) {
      headers.push({ name: raw[i], value: raw[i + 1] });
    }
    const oidcPath = req.originalUrl.replace(/^\/auth/, "") || "/";
    const response = oidc.handleRequest({ method: req.method, path: oidcPath, headers, body });
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
import path from "node:path";
import { Hono } from "hono";
import { serve } from "@hono/node-server";
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: "./config.toml" });
const app = new Hono();

app.all("/auth/*", async (c) => {
  const req = c.req.raw;
  const url = new URL(req.url);
  const oidcPath = url.pathname.replace(/^\/auth/, "") || "/";

  const headers: { name: string; value: string }[] = [];
  req.headers.forEach((value, name) => {
    headers.push({ name, value });
  });

  const body = req.body ? Buffer.from(await req.arrayBuffer()) : undefined;

  const response = oidc.handleRequest({ method: req.method, path: oidcPath, headers, body });

  const responseHeaders = new Headers();
  for (const { name, value } of response.headers) {
    responseHeaders.append(name, value);
  }

  return new Response(response.body, { status: response.status, headers: responseHeaders });
});

serve({ fetch: app.fetch, port: 3000 });
```

### Fastify

```typescript
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import Fastify from "fastify";
import { OidcExchange } from "@oidc-exchange/node";

const __dirname = dirname(fileURLToPath(import.meta.url));
const oidc = new OidcExchange({ config: resolve(__dirname, "..", "config.toml") });
const app = Fastify();

app.addContentTypeParser("*", { parseAs: "buffer" }, (_req, body, done) => {
  done(null, body);
});

app.all("/auth/*", async (request, reply) => {
  const oidcPath = request.url.replace(/^\/auth/, "") || "/";
  const headers = [];
  for (const [name, value] of Object.entries(request.headers)) {
    if (Array.isArray(value)) {
      for (const v of value) headers.push({ name, value: v });
    } else if (value !== undefined) {
      headers.push({ name, value });
    }
  }

  const body = request.body instanceof Buffer && request.body.length > 0
    ? request.body : undefined;

  const response = oidc.handleRequest({ method: request.method, path: oidcPath, headers, body });
  for (const { name, value } of response.headers) reply.header(name, value);
  reply.status(response.status).send(response.body);
});

app.listen({ port: 3000 });
```

### Next.js (App Router)

```typescript
// app/auth/[...path]/route.ts
import path from "node:path";
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: path.resolve(process.cwd(), "..", "config.toml") });

async function handler(request: Request) {
  const url = new URL(request.url);
  const oidcPath = url.pathname.replace(/^\/auth/, "") || "/";

  const headers: { name: string; value: string }[] = [];
  request.headers.forEach((value, name) => { headers.push({ name, value }); });

  const body = request.body ? Buffer.from(await request.arrayBuffer()) : undefined;

  const response = oidc.handleRequest({ method: request.method, path: oidcPath, headers, body });

  const responseHeaders = new Headers();
  for (const { name, value } of response.headers) responseHeaders.append(name, value);

  return new Response(response.body, { status: response.status, headers: responseHeaders });
}

export const GET = handler;
export const POST = handler;
```

### SvelteKit

```typescript
// src/hooks.server.ts
import path from "node:path";
import type { Handle } from "@sveltejs/kit";
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: path.resolve(process.cwd(), "..", "config.toml") });

export const handle: Handle = async ({ event, resolve }) => {
  if (!event.url.pathname.startsWith("/auth/")) return resolve(event);

  const request = event.request;
  const oidcPath = event.url.pathname.replace(/^\/auth/, "") || "/";

  const headers: { name: string; value: string }[] = [];
  request.headers.forEach((value, name) => { headers.push({ name, value }); });

  const body = request.body ? Buffer.from(await request.arrayBuffer()) : undefined;

  const response = oidc.handleRequest({ method: request.method, path: oidcPath, headers, body });

  const responseHeaders = new Headers();
  for (const { name, value } of response.headers) responseHeaders.append(name, value);

  return new Response(response.body, { status: response.status, headers: responseHeaders });
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
