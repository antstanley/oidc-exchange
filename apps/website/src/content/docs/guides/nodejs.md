---
title: Node.js
description: Use oidc-exchange as an embedded OIDC provider in Node.js applications.
---

## Installation

```bash
npm install @oidc-exchange/node
```

Requires **Node.js 22+**.

## Basic Usage

```javascript
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({
  configPath: "./config/default.toml",
});

const response = await oidc.handleRequest(request);
```

The `handleRequest` method accepts a standard `Request` object and returns a `Response`. This makes it compatible with any framework that supports the Web Fetch API request/response model.

## Framework Integration

### Express

```javascript
import express from "express";
import { OidcExchange } from "@oidc-exchange/node";

const app = express();
const oidc = new OidcExchange({ configPath: "./config/default.toml" });

app.all("/oidc/*", async (req, res) => {
  const request = new Request(`http://localhost${req.originalUrl}`, {
    method: req.method,
    headers: req.headers,
    body: ["GET", "HEAD"].includes(req.method) ? undefined : req,
  });

  const response = await oidc.handleRequest(request);

  res.status(response.status);
  response.headers.forEach((value, key) => res.setHeader(key, value));
  const body = await response.text();
  res.send(body);
});

app.listen(3000);
```

### Hono

```javascript
import { Hono } from "hono";
import { OidcExchange } from "@oidc-exchange/node";

const app = new Hono();
const oidc = new OidcExchange({ configPath: "./config/default.toml" });

app.all("/oidc/*", async (c) => {
  return oidc.handleRequest(c.req.raw);
});

export default app;
```

### Fastify

```javascript
import Fastify from "fastify";
import { OidcExchange } from "@oidc-exchange/node";

const fastify = Fastify();
const oidc = new OidcExchange({ configPath: "./config/default.toml" });

fastify.all("/oidc/*", async (request, reply) => {
  const req = new Request(`http://localhost${request.url}`, {
    method: request.method,
    headers: request.headers,
    body: ["GET", "HEAD"].includes(request.method)
      ? undefined
      : JSON.stringify(request.body),
  });

  const response = await oidc.handleRequest(req);

  reply.status(response.status);
  response.headers.forEach((value, key) => reply.header(key, value));
  return reply.send(await response.text());
});

fastify.listen({ port: 3000 });
```

### Next.js (App Router)

```javascript
// app/oidc/[...path]/route.js
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ configPath: "./config/default.toml" });

export async function GET(request) {
  return oidc.handleRequest(request);
}

export async function POST(request) {
  return oidc.handleRequest(request);
}
```

### SvelteKit

```javascript
// src/hooks.server.js
import { OidcExchange } from "@oidc-exchange/node";

const oidc = new OidcExchange({ configPath: "./config/default.toml" });

export async function handle({ event, resolve }) {
  if (event.url.pathname.startsWith("/oidc")) {
    return oidc.handleRequest(event.request);
  }
  return resolve(event);
}
```

## Configuration

### File path

Point to a TOML config file on disk:

```javascript
const oidc = new OidcExchange({
  configPath: "./config/default.toml",
});
```

### Inline TOML

Pass configuration as an inline TOML string:

```javascript
const oidc = new OidcExchange({
  configToml: `
[server]
host = "0.0.0.0"
port = 8080
issuer = "https://auth.example.com"

[registration]
mode = "open"
`,
});
```

See the [Configuration guide](/guides/configuration) for all available options.
