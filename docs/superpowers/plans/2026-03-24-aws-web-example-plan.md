# AWS Web Example Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a reference example deploying oidc-exchange on AWS with a SvelteKit demo app and Google Sign-In.

**Architecture:** CDK stack (TypeScript) provisions DynamoDB, KMS, CloudTrail Lake, API Gateway, and two Lambdas (auth + demo app). SvelteKit demo app uses Lambda Web Adapter with the node adapter. Google Identity Services SDK handles client-side auth, tokens exchanged server-side via `/auth/token`.

**Tech Stack:** AWS CDK (TypeScript), SvelteKit (node adapter), Lambda Web Adapter, DynamoDB, KMS, CloudTrail Lake, Google Identity Services SDK

**Spec:** `docs/superpowers/specs/2026-03-24-aws-web-example-design.md`

**VCS:** jj (Jujutsu). Use `jj describe` for change descriptions, `jj new` to start the next change.

---

## Task 1: Workspace Scaffold

**Files:**
- Create: `examples/aws-web/package.json`
- Create: `examples/aws-web/infra/package.json`
- Create: `examples/aws-web/infra/tsconfig.json`
- Create: `examples/aws-web/infra/cdk.json`
- Create: `examples/aws-web/demo-app/package.json`
- Create: `examples/aws-web/demo-app/svelte.config.js`
- Create: `examples/aws-web/demo-app/vite.config.ts`
- Create: `examples/aws-web/demo-app/bundle.sh`
- Create: `examples/aws-web/demo-app/tsconfig.json`
- Create: `examples/aws-web/config/oidc-exchange.toml`
- Create: `examples/aws-web/bootstrap/.gitkeep`

- [ ] **Step 1: Create workspace root package.json**

```json
{
  "name": "oidc-exchange-aws-web-example",
  "private": true,
  "workspaces": ["infra", "demo-app"]
}
```

- [ ] **Step 2: Create infra package.json**

```json
{
  "name": "infra",
  "private": true,
  "scripts": {
    "build": "tsc",
    "cdk": "cdk"
  },
  "dependencies": {
    "aws-cdk-lib": "^2.180.0",
    "constructs": "^10.4.0"
  },
  "devDependencies": {
    "aws-cdk": "^2.180.0",
    "typescript": "^5.7.0"
  }
}
```

- [ ] **Step 3: Create infra tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "commonjs",
    "lib": ["ES2022"],
    "outDir": "dist",
    "rootDir": ".",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "declaration": true
  },
  "include": ["bin/**/*", "lib/**/*"]
}
```

- [ ] **Step 4: Create infra cdk.json**

```json
{
  "app": "npx ts-node bin/app.ts",
  "context": {
    "googleClientId": "YOUR_GOOGLE_CLIENT_ID",
    "googleClientSecret": "YOUR_GOOGLE_CLIENT_SECRET"
  }
}
```

- [ ] **Step 5: Create demo-app package.json**

```json
{
  "name": "demo-app",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite dev",
    "build": "vite build",
    "bundle": "bash bundle.sh",
    "preview": "vite preview"
  },
  "dependencies": {
    "@sveltejs/adapter-node": "^5.0.0",
    "@sveltejs/kit": "^2.15.0",
    "svelte": "^5.0.0"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^4.0.0",
    "typescript": "^5.7.0",
    "vite": "^6.0.0"
  }
}
```

- [ ] **Step 6: Create demo-app svelte.config.js**

```js
import adapter from '@sveltejs/adapter-node';
import { vitePreprocess } from '@sveltejs/kit/vite';

/** @type {import('@sveltejs/kit').Config} */
export default {
  kit: {
    adapter: adapter({ out: 'dist/svelteKit' })
  },
  preprocess: [vitePreprocess()]
};
```

- [ ] **Step 7: Create demo-app vite.config.ts**

```ts
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()]
});
```

- [ ] **Step 8: Create demo-app bundle.sh**

```bash
#!/bin/bash
set -euo pipefail

rm -rf dist
mkdir -p dist/svelteKit
npx vite build

cat > dist/svelteKit/run.sh << 'SCRIPT'
#!/bin/bash
exec node index.js
SCRIPT
chmod +x dist/svelteKit/run.sh

echo '{"type":"module"}' > dist/svelteKit/package.json
```

- [ ] **Step 9: Create demo-app tsconfig.json**

```json
{
  "extends": "./.svelte-kit/tsconfig.json",
  "compilerOptions": {
    "allowJs": true,
    "checkJs": true,
    "esModuleInterop": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "skipLibCheck": true,
    "sourceMap": true,
    "strict": true,
    "moduleResolution": "bundler"
  }
}
```

- [ ] **Step 10: Create config/oidc-exchange.toml**

```toml
[server]
issuer = "${ISSUER_URL}"

[registration]
mode = "open"

[token]
access_token_ttl = "15m"
refresh_token_ttl = "30d"
audience = "${AUDIENCE_URL}"

[key_manager]
adapter = "kms"

[key_manager.kms]
key_id = "${KMS_KEY_ID}"
algorithm = "ECDSA_SHA_256"
kid = "example-key-1"

[repository]
adapter = "dynamodb"

[repository.dynamodb]
table_name = "${TABLE_NAME}"

[audit]
adapter = "cloudtrail"
blocking_threshold = "warning"

[audit.cloudtrail]
channel_arn = "${CLOUDTRAIL_CHANNEL_ARN}"

[telemetry]
enabled = true
exporter = "xray"
service_name = "oidc-exchange-example"

[providers.google]
adapter = "oidc"
issuer = "https://accounts.google.com"
client_id = "${GOOGLE_CLIENT_ID}"
client_secret = "${GOOGLE_CLIENT_SECRET}"
scopes = ["openid", "email", "profile"]
```

- [ ] **Step 11: Create bootstrap/.gitkeep**

Empty file. This directory is where the pre-built Rust binary goes.

- [ ] **Step 12: Commit**

```bash
jj describe -m "feat: scaffold aws-web example with npm workspaces"
jj new
```

---

## Task 2: CDK Stack — Infrastructure

**Files:**
- Create: `examples/aws-web/infra/bin/app.ts`
- Create: `examples/aws-web/infra/lib/stack.ts`

- [ ] **Step 1: Create CDK app entry point**

`examples/aws-web/infra/bin/app.ts`:

```ts
#!/usr/bin/env node
import 'source-map-support/register';
import * as cdk from 'aws-cdk-lib';
import { OidcExchangeExampleStack } from '../lib/stack';

const app = new cdk.App();

const googleClientId = app.node.tryGetContext('googleClientId') || 'YOUR_GOOGLE_CLIENT_ID';
const googleClientSecret = app.node.tryGetContext('googleClientSecret') || 'YOUR_GOOGLE_CLIENT_SECRET';

new OidcExchangeExampleStack(app, 'OidcExchangeExample', {
  googleClientId,
  googleClientSecret,
});
```

- [ ] **Step 2: Create CDK stack**

`examples/aws-web/infra/lib/stack.ts` — Single stack with all resources:

**DynamoDB table:**
- Table name: `oidc-exchange-example`
- pk (S) partition key, sk (S) sort key
- GSI1 with GSI1pk (S) / GSI1sk (S), projection ALL
- Billing: PAY_PER_REQUEST
- TTL on `ttl` attribute
- RemovalPolicy.DESTROY (example stack, easy cleanup)

**KMS key:**
- KeySpec: ECC_NIST_P256
- KeyUsage: SIGN_VERIFY
- Alias: `alias/oidc-exchange-example`
- RemovalPolicy.DESTROY

**CloudTrail Lake:**
- `CfnEventDataStore` with `advancedEventSelectors` for custom events
- `CfnChannel` linked to the event data store
- Note: CloudTrail Lake resources use L1 constructs (Cfn) since no L2 constructs exist yet

**Auth Lambda:**
- Runtime: `PROVIDED_AL2023`
- Handler: `bootstrap`
- Code: `lambda.Code.fromAsset(path.join(__dirname, '../../bootstrap'))` — must include both the `bootstrap` binary and `oidc-exchange.toml` config
- Actually: bundle `bootstrap/` and `config/oidc-exchange.toml` together. Use a `lambda.Code.fromAsset` with a path that contains both, or use `BundlingOptions`. Simplest: copy the config into the bootstrap dir before deploy, or use environment variables for all config (since the TOML uses `${VAR}` placeholders).
- Environment variables: `TABLE_NAME`, `KMS_KEY_ID`, `CLOUDTRAIL_CHANNEL_ARN`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `ISSUER_URL`, `AUDIENCE_URL`
- Memory: 256MB, timeout: 29s

**Demo App Lambda:**
- Runtime: `NODEJS_24_X`
- Handler: `run.sh`
- Code: `lambda.Code.fromAsset(path.join(__dirname, '../../demo-app/dist/svelteKit'))`
- Lambda Web Adapter layer: `lambda.LayerVersion.fromLayerVersionArn(this, 'WebAdapter', \`arn:aws:lambda:${Stack.of(this).region}:753240598075:layer:LambdaAdapterLayerX86:24\`)`
- Environment: `AWS_LAMBDA_EXEC_WRAPPER=/opt/bootstrap`, `PORT=8080`, `ORIGIN`, `AUTH_ENDPOINT`, `PUBLIC_GOOGLE_CLIENT_ID`
- Memory: 256MB, timeout: 29s

**API Gateway (HTTP API):**
- `HttpApi` with auto-deploy
- `ANY /auth/{proxy+}` → Auth Lambda (HttpLambdaIntegration)
- `GET /` → Demo App Lambda
- `ANY /{proxy+}` → Demo App Lambda
- Set `ORIGIN` and `AUTH_ENDPOINT` on Demo App Lambda using the API URL after creation

**IAM:**
- Grant table read/write + GSI query to Auth Lambda
- Grant KMS sign/getPublicKey to Auth Lambda
- Grant CloudTrail data put to Auth Lambda

**Stack outputs:**
- API Gateway URL
- Table name
- KMS key ARN

- [ ] **Step 3: Install CDK dependencies**

Run: `cd examples/aws-web/infra && npm install`

- [ ] **Step 4: Verify CDK synth**

Run: `cd examples/aws-web/infra && npx cdk synth`
Expected: CloudFormation template output (may have warnings about missing assets, that's OK)

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat: add CDK stack with DynamoDB, KMS, CloudTrail, API Gateway, and Lambda"
jj new
```

---

## Task 3: SvelteKit — App Shell and Layout

**Files:**
- Create: `examples/aws-web/demo-app/src/app.html`
- Create: `examples/aws-web/demo-app/src/app.d.ts`
- Create: `examples/aws-web/demo-app/src/routes/+layout.svelte`

- [ ] **Step 1: Create app.html with Google Identity Services script**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <script src="https://accounts.google.com/gsi/client" async defer></script>
    <title>OIDC Exchange Demo</title>
    %sveltekit.head%
  </head>
  <body>
    <div id="app">%sveltekit.body%</div>
  </body>
</html>
```

- [ ] **Step 2: Create app.d.ts**

Type declarations for the Google Identity Services global and SvelteKit env:

```ts
/// <reference types="@sveltejs/kit" />

declare namespace google.accounts.id {
  function initialize(config: {
    client_id: string;
    callback: (response: { credential: string }) => void;
  }): void;
  function renderButton(
    element: HTMLElement,
    config: { theme?: string; size?: string; width?: number }
  ): void;
}
```

- [ ] **Step 3: Create layout**

`+layout.svelte` — minimal shell with basic styling:

```svelte
<script>
  let { children } = $props();
</script>

<main style="max-width: 600px; margin: 2rem auto; font-family: system-ui, sans-serif;">
  {@render children()}
</main>
```

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add SvelteKit app shell with Google Identity Services"
jj new
```

---

## Task 4: SvelteKit — Login Page

**Files:**
- Create: `examples/aws-web/demo-app/src/routes/+page.svelte`
- Create: `examples/aws-web/demo-app/src/routes/+page.server.ts`
- Create: `examples/aws-web/demo-app/src/lib/auth.ts`

- [ ] **Step 1: Create auth helper**

`src/lib/auth.ts`:

```ts
export async function exchangeToken(credential: string): Promise<Response> {
  return fetch('/api/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ credential })
  });
}
```

- [ ] **Step 2: Create login page server load**

`src/routes/+page.server.ts` — passes Google Client ID to the page:

```ts
import { env } from '$env/dynamic/public';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async () => {
  return {
    googleClientId: env.PUBLIC_GOOGLE_CLIENT_ID || 'YOUR_GOOGLE_CLIENT_ID'
  };
};
```

- [ ] **Step 3: Create login page**

`src/routes/+page.svelte`:

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import { exchangeToken } from '$lib/auth';

  let { data } = $props();
  let buttonContainer: HTMLElement;

  $effect(() => {
    if (typeof google !== 'undefined' && buttonContainer) {
      google.accounts.id.initialize({
        client_id: data.googleClientId,
        callback: handleCredentialResponse
      });
      google.accounts.id.renderButton(buttonContainer, {
        theme: 'outline',
        size: 'large',
        width: 300
      });
    }
  });

  async function handleCredentialResponse(response: { credential: string }) {
    const result = await exchangeToken(response.credential);
    if (result.ok) {
      goto('/authenticated');
    } else {
      goto('/denied');
    }
  }
</script>

<h1>OIDC Exchange Demo</h1>
<p>Sign in to test the OIDC token exchange service.</p>

<div bind:this={buttonContainer}></div>

<noscript>JavaScript is required for Google Sign-In.</noscript>
```

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add login page with Google Sign-In button"
jj new
```

---

## Task 5: SvelteKit — API Routes (Login + Logout)

**Files:**
- Create: `examples/aws-web/demo-app/src/routes/api/login/+server.ts`
- Create: `examples/aws-web/demo-app/src/routes/api/logout/+server.ts`

- [ ] **Step 1: Create login API route**

`src/routes/api/login/+server.ts`:

```ts
import type { RequestHandler } from './$types';

const AUTH_ENDPOINT = process.env.AUTH_ENDPOINT || 'http://localhost:8080/auth';

export const POST: RequestHandler = async ({ request, cookies }) => {
  const { credential } = await request.json();

  if (!credential) {
    return new Response(JSON.stringify({ error: 'missing credential' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' }
    });
  }

  // Exchange the Google ID token for our tokens
  const tokenResponse = await fetch(`${AUTH_ENDPOINT}/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'id_token',
      id_token: credential,
      provider: 'google'
    })
  });

  if (!tokenResponse.ok) {
    const error = await tokenResponse.text();
    return new Response(error, {
      status: tokenResponse.status >= 500 ? 500 : 401,
      headers: { 'Content-Type': 'application/json' }
    });
  }

  const tokens = await tokenResponse.json();

  // Set tokens as httpOnly cookies
  cookies.set('access_token', tokens.access_token, {
    path: '/',
    httpOnly: true,
    secure: true,
    sameSite: 'strict',
    maxAge: tokens.expires_in
  });

  if (tokens.refresh_token) {
    cookies.set('refresh_token', tokens.refresh_token, {
      path: '/',
      httpOnly: true,
      secure: true,
      sameSite: 'strict',
      maxAge: 60 * 60 * 24 * 30 // 30 days
    });
  }

  return new Response(JSON.stringify({ ok: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
  });
};
```

- [ ] **Step 2: Create logout API route**

`src/routes/api/logout/+server.ts`:

```ts
import type { RequestHandler } from './$types';

export const POST: RequestHandler = async ({ cookies }) => {
  cookies.delete('access_token', { path: '/' });
  cookies.delete('refresh_token', { path: '/' });

  return new Response(JSON.stringify({ ok: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
  });
};
```

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat: add login and logout API routes with cookie management"
jj new
```

---

## Task 6: SvelteKit — Authenticated and Denied Pages

**Files:**
- Create: `examples/aws-web/demo-app/src/routes/authenticated/+page.server.ts`
- Create: `examples/aws-web/demo-app/src/routes/authenticated/+page.svelte`
- Create: `examples/aws-web/demo-app/src/routes/denied/+page.svelte`

- [ ] **Step 1: Create authenticated page server load**

`src/routes/authenticated/+page.server.ts`:

```ts
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ cookies }) => {
  const accessToken = cookies.get('access_token');

  if (!accessToken) {
    throw redirect(302, '/');
  }

  // Decode JWT payload (no signature verification — demo only)
  try {
    const parts = accessToken.split('.');
    if (parts.length !== 3) throw new Error('Invalid JWT');

    const payload = JSON.parse(
      Buffer.from(parts[1], 'base64url').toString('utf-8')
    );

    return {
      user: {
        sub: payload.sub,
        email: payload.email,
        exp: payload.exp,
        iat: payload.iat,
        iss: payload.iss,
        claims: Object.fromEntries(
          Object.entries(payload).filter(
            ([k]) => !['sub', 'iss', 'aud', 'iat', 'exp'].includes(k)
          )
        )
      }
    };
  } catch {
    throw redirect(302, '/');
  }
};
```

- [ ] **Step 2: Create authenticated page**

`src/routes/authenticated/+page.svelte`:

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';

  let { data } = $props();

  async function logout() {
    await fetch('/api/logout', { method: 'POST' });
    goto('/');
  }
</script>

<h1>Authenticated</h1>

<p>You are signed in.</p>

<dl>
  <dt>Subject</dt>
  <dd><code>{data.user.sub}</code></dd>

  {#if data.user.email}
    <dt>Email</dt>
    <dd>{data.user.email}</dd>
  {/if}

  <dt>Issued At</dt>
  <dd>{new Date(data.user.iat * 1000).toLocaleString()}</dd>

  <dt>Expires</dt>
  <dd>{new Date(data.user.exp * 1000).toLocaleString()}</dd>

  {#if Object.keys(data.user.claims).length > 0}
    <dt>Custom Claims</dt>
    <dd><pre>{JSON.stringify(data.user.claims, null, 2)}</pre></dd>
  {/if}
</dl>

<button onclick={logout}>Sign Out</button>
```

- [ ] **Step 3: Create denied page**

`src/routes/denied/+page.svelte`:

```svelte
<h1>Authorization Denied</h1>

<p>Your sign-in attempt was not successful.</p>

<a href="/">Try again</a>
```

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat: add authenticated and denied pages"
jj new
```

---

## Task 7: README

**Files:**
- Create: `examples/aws-web/README.md`

- [ ] **Step 1: Write README**

Cover:
1. **What this is** — reference example deploying oidc-exchange on AWS
2. **Architecture diagram** (text-based)
3. **Prerequisites** — AWS CLI configured, CDK CLI (`npm install -g aws-cdk`), Node.js 24+, Rust + cargo-lambda (for building the binary), Google Cloud project with OAuth client ID
4. **Google Auth setup** — step by step: create project, enable APIs, create OAuth client, note client ID and secret
5. **Build the auth service** — `cargo lambda build --release --output-format zip` → extract `bootstrap` to `examples/aws-web/bootstrap/`
6. **Build the demo app** — `cd examples/aws-web/demo-app && npm install && bash bundle.sh`
7. **Deploy** — `cd examples/aws-web/infra && npm install && npx cdk deploy -c googleClientId=xxx -c googleClientSecret=xxx`
8. **Test** — visit the API Gateway URL output from the deploy
9. **Cleanup** — `npx cdk destroy`

- [ ] **Step 2: Commit**

```bash
jj describe -m "docs: add aws-web example README"
jj new
```

---

## Task Dependency Summary

```
Task 1 (scaffold) → Task 2 (CDK stack)
Task 1 (scaffold) → Task 3 (app shell) → Task 4 (login) → Task 5 (API routes) → Task 6 (auth + denied pages)
Task 6 → Task 7 (README)
```

Tasks 2 and 3-6 can run in parallel after Task 1, but sequencing is simpler.
