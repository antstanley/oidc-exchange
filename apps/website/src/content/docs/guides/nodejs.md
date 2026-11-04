---
title: Node.js
description: Embed oidc-exchange in Node.js applications.
---

## Installation

```bash
pnpm add @oidc-exchange/node
```

Use **Node.js 22 or later**. Prebuilt native binaries are included for Linux (x64, ARM64), macOS (ARM64), and Windows (x64).

For AWS Lambda deployments, install the Lambda adapter instead:

```bash
pnpm add @oidc-exchange/lambda
```

## Basic usage

```typescript
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ config: "./config.toml" });

const response = await oidc.handleRequest({
  method: "GET",
  rawPath: Buffer.from("/health"),
  query: undefined,
  headers: [],
  pathIsRaw: true,
});

console.log(response.status); // 200
```

`handleRequest` returns a Promise and takes the wire request fields `{ method, rawPath, query?, headers, body?, pathIsRaw }`. Keep the still-percent-encoded path separate from the query; the binding owns decoding and configured base-path stripping. Headers are ordered arrays of `{ name, value }` objects.

### Migrating from 0.2

In 0.3, replace `path` with the raw, percent-encoded path bytes in `rawPath`, pass the still-encoded query bytes separately without `?`, set `pathIsRaw` to describe the source, and `await handleRequest`. Do not decode or strip `server.base_path` in application code. Callers that cannot migrate to async immediately can temporarily use deprecated `handleRequestSync` with the same new request shape; it blocks the calling thread and will be removed after one major release cycle.

## Framework integration

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
  req.on("end", async () => {
    const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined;
    const headers = [];
    const raw = req.rawHeaders;
    for (let i = 0; i < raw.length; i += 2) {
      headers.push({ name: raw[i], value: raw[i + 1] });
    }
    const queryIndex = req.originalUrl.indexOf("?");
    const rawPath = req.originalUrl.slice(0, queryIndex < 0 ? undefined : queryIndex);
    const query = queryIndex < 0 ? undefined : req.originalUrl.slice(queryIndex + 1);
    const response = await oidc.handleRequest({
      method: req.method, rawPath: Buffer.from(rawPath),
      query: query === undefined ? undefined : Buffer.from(query),
      headers, body, pathIsRaw: true,
    });
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
  const targetStart = req.url.indexOf("/", req.url.indexOf("://") + 3);
  const rawTarget = targetStart < 0 ? "/" : req.url.slice(targetStart);
  const queryIndex = rawTarget.indexOf("?");
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex);
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1);

  const headers: { name: string; value: string }[] = [];
  req.headers.forEach((value, name) => {
    headers.push({ name, value });
  });

  const body = req.body ? Buffer.from(await req.arrayBuffer()) : undefined;

  const response = await oidc.handleRequest({ method: req.method, rawPath: Buffer.from(rawPath), query: query === undefined ? undefined : Buffer.from(query), headers, body, pathIsRaw: true });

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
  const rawTarget = request.raw.url ?? request.url;
  const queryIndex = rawTarget.indexOf("?");
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex);
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1);
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

  const response = await oidc.handleRequest({ method: request.method, rawPath: Buffer.from(rawPath), query: query === undefined ? undefined : Buffer.from(query), headers, body, pathIsRaw: true });
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
  const targetStart = request.url.indexOf("/", request.url.indexOf("://") + 3);
  const rawTarget = targetStart < 0 ? "/" : request.url.slice(targetStart);
  const queryIndex = rawTarget.indexOf("?");
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex);
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1);

  const headers: { name: string; value: string }[] = [];
  request.headers.forEach((value, name) => { headers.push({ name, value }); });

  const body = request.body ? Buffer.from(await request.arrayBuffer()) : undefined;

  const response = await oidc.handleRequest({ method: request.method, rawPath: Buffer.from(rawPath), query: query === undefined ? undefined : Buffer.from(query), headers, body, pathIsRaw: true });

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
  const targetStart = request.url.indexOf("/", request.url.indexOf("://") + 3);
  const rawTarget = targetStart < 0 ? "/" : request.url.slice(targetStart);
  const queryIndex = rawTarget.indexOf("?");
  const rawPath = rawTarget.slice(0, queryIndex < 0 ? undefined : queryIndex);
  const query = queryIndex < 0 ? undefined : rawTarget.slice(queryIndex + 1);

  const headers: { name: string; value: string }[] = [];
  request.headers.forEach((value, name) => { headers.push({ name, value }); });

  const body = request.body ? Buffer.from(await request.arrayBuffer()) : undefined;

  const response = await oidc.handleRequest({ method: request.method, rawPath: Buffer.from(rawPath), query: query === undefined ? undefined : Buffer.from(query), headers, body, pathIsRaw: true });

  const responseHeaders = new Headers();
  for (const { name, value } of response.headers) responseHeaders.append(name, value);

  return new Response(response.body, { status: response.status, headers: responseHeaders });
};
```

### AWS Lambda

Use `@oidc-exchange/lambda` for AWS Lambda deployments. It detects events from API Gateway v1 (REST API), API Gateway v2 (HTTP API), Lambda Function URLs, and Application Load Balancers (ALB).

```typescript
import { createHandler } from "@oidc-exchange/lambda";

export const handler = createHandler({
  config: "./config.toml",
  basePath: "/auth",
});
```

The handler converts Lambda events to HTTP requests and routes them through oidc-exchange. It converts each response to the format expected by Lambda.

You can deploy with AWS Serverless Application Model (SAM), AWS Cloud Development Kit (CDK), Serverless Framework, or Terraform.

#### Deployment modes: managed runtime and container image

`@oidc-exchange/lambda` supports two deployment formats:

- **Managed runtime:** Deploy a zip bundle that includes the native addon from `@oidc-exchange/node`. The `nodejs22.x` runtime uses Amazon Linux 2023 with GNU C Library (glibc) 2.34.
- **Container image:** Package the function with the AWS Lambda Runtime Interface Client (RIC). Use an image such as `public.ecr.aws/lambda/nodejs:22` to reproduce the runtime locally or choose a specific base image.

The published `@oidc-exchange/linux-{x64,arm64}-gnu` addons require glibc 2.17 or later. Amazon Linux 2023 meets this requirement. The operating system provides glibc, so changing the Node.js version does not change the available glibc version.

Use a glibc base image such as `debian`, `ubuntu`, `public.ecr.aws/lambda/nodejs:*`, or `gcr.io/distroless/nodejs*`. Alpine uses a different C library, musl, and requires a `-musl` addon build. That build is not currently published.

For local testing, use an AWS base image that matches your production runtime.

## Configuration

Pass configuration as a file path or a string in TOML format:

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
