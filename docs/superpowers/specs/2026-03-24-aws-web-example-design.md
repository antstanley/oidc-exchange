# AWS Web Example — Design Specification

A reference example deploying the oidc-exchange service on AWS with a SvelteKit demo app, demonstrating Google Sign-In integration end-to-end.

## Overview

An `examples/aws-web/` directory containing:
- **CDK stack** (TypeScript) provisioning all AWS infrastructure
- **SvelteKit demo app** served via Lambda Web Adapter showing login, authenticated, and denied pages
- **oidc-exchange config** with Google placeholder credentials and AWS adapter settings
- **Pre-built binary slot** for the Rust oidc-exchange binary

### Auth Flow

1. User clicks "Sign in with Google" on the demo app
2. Google Identity Services SDK returns a credential (Google ID token)
3. Demo app's server-side `/api/login` endpoint POSTs `grant_type=id_token&id_token={credential}&provider=google` to `/auth/token`
4. oidc-exchange validates the Google ID token against Google's JWKS, creates/looks up the user, issues access + refresh tokens
5. Demo app stores tokens in httpOnly cookies, redirects to `/authenticated`
6. Authenticated page decodes JWT to display user info

---

## Project Structure

```
examples/
└── aws-web/
    ├── README.md
    ├── package.json              # npm workspaces root
    ├── infra/                    # CDK app
    │   ├── bin/app.ts
    │   ├── lib/stack.ts          # Single stack: all AWS resources
    │   ├── cdk.json
    │   ├── package.json
    │   └── tsconfig.json
    ├── demo-app/                 # SvelteKit app (node adapter)
    │   ├── package.json
    │   ├── svelte.config.js
    │   ├── vite.config.ts
    │   ├── bundle.sh
    │   ├── src/
    │   │   ├── app.html          # Google Identity Services script tag
    │   │   ├── app.d.ts
    │   │   ├── routes/
    │   │   │   ├── +layout.svelte
    │   │   │   ├── +page.svelte          # Login page (Google button)
    │   │   │   ├── api/
    │   │   │   │   ├── login/
    │   │   │   │   │   └── +server.ts    # Server-side token exchange proxy
    │   │   │   │   └── logout/
    │   │   │   │       └── +server.ts    # Clear auth cookies
    │   │   │   ├── authenticated/
    │   │   │   │   ├── +page.svelte      # Authenticated page
    │   │   │   │   └── +page.server.ts   # Server load: read token, decode JWT
    │   │   │   └── denied/
    │   │   │       └── +page.svelte      # Authorization denied page
    │   │   └── lib/
    │   │       └── auth.ts               # Auth helpers
    │   └── static/
    ├── config/
    │   └── oidc-exchange.toml
    └── bootstrap/
        └── .gitkeep
```

---

## CDK Stack

Single stack (`OidcExchangeExampleStack`) with all resources.

### Auth Lambda

- **Runtime:** `provided.al2023` (custom runtime for Rust binary)
- **Handler:** `bootstrap`
- **Code:** `AssetCode` from `bootstrap/` directory
- **Memory:** 256MB, timeout: 29s
- **Environment:** `TABLE_NAME`, `KMS_KEY_ID`, `CLOUDTRAIL_CHANNEL_ARN`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` (from CDK context or environment)
- **Config:** `oidc-exchange.toml` bundled into the Lambda code asset

### Demo App Lambda

- **Runtime:** `nodejs24.x`
- **Handler:** `run.sh` (shell script that runs `node index.js`)
- **Code:** `AssetCode` from `demo-app/dist/svelteKit/`
- **Lambda Web Adapter layer:** constructed dynamically using stack region: `` `arn:aws:lambda:${Stack.of(this).region}:753240598075:layer:LambdaAdapterLayerX86:24` ``
- **Memory:** 256MB, timeout: 29s
- **Environment:**
  - `AWS_LAMBDA_EXEC_WRAPPER=/opt/bootstrap`
  - `PORT=8080`
  - `ORIGIN=https://{api-gateway-url}`
  - `AUTH_ENDPOINT=https://{api-gateway-url}/auth`
  - `PUBLIC_GOOGLE_CLIENT_ID` (read by SvelteKit via `$env/dynamic/public` for the GSI button)

### API Gateway (HTTP API)

- `ANY /auth/{proxy+}` → Auth Lambda integration
- `GET /` → Demo App Lambda integration
- `ANY /{proxy+}` → Demo App Lambda integration
- Auto-deploy stage

### DynamoDB

- **Table name:** `oidc-exchange-example`
- **Partition key:** `pk` (S)
- **Sort key:** `sk` (S)
- **GSI1:** `GSI1pk` (S) / `GSI1sk` (S), projection ALL
- **Billing:** on-demand (PAY_PER_REQUEST)
- **TTL:** enabled on `ttl` attribute

### KMS

- **Key spec:** `ECC_NIST_P256` (for ES256 signing)
- **Key usage:** `SIGN_VERIFY`
- **Alias:** `alias/oidc-exchange-example`

### CloudTrail Lake

- **Event data store:** advanced events, custom integration
- **Channel:** linked to the event data store
- Channel ARN passed to Auth Lambda as environment variable

### IAM Permissions

**Auth Lambda role:**
- `dynamodb:GetItem`, `dynamodb:PutItem`, `dynamodb:UpdateItem`, `dynamodb:DeleteItem`, `dynamodb:Query`, `dynamodb:BatchWriteItem` on table + GSI1
- `kms:Sign`, `kms:GetPublicKey` on the KMS key
- `cloudtrail-data:PutAuditEvents` on the CloudTrail channel

**Demo App Lambda role:**
- Basic Lambda execution (logs only)

---

## SvelteKit Demo App

### `app.html`

Includes Google Identity Services script:
```html
<script src="https://accounts.google.com/gsi/client" async defer></script>
```

### `bundle.sh`

```bash
#!/bin/bash
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

### `svelte.config.js`

```js
import adapter from '@sveltejs/adapter-node';
import { vitePreprocess } from '@sveltejs/kit/vite';

export default {
  kit: { adapter: adapter({ out: 'dist/svelteKit' }) },
  preprocess: [vitePreprocess()]
};
```

### Routes

**`/` — Login Page (`+page.svelte`)**

- On mount, initialize Google Identity Services:
  ```js
  google.accounts.id.initialize({
    client_id: GOOGLE_CLIENT_ID,
    callback: handleCredentialResponse
  });
  google.accounts.id.renderButton(element, { theme: 'outline', size: 'large' });
  ```
- `GOOGLE_CLIENT_ID` read at runtime via `$env/dynamic/public` (`PUBLIC_GOOGLE_CLIENT_ID` env var on the Lambda)
- `handleCredentialResponse` sends the credential to `/api/login` via fetch POST
- On success (200): redirect to `/authenticated`
- On failure: redirect to `/denied`

**`/api/login/+server.ts` — Token Exchange Proxy**

- Receives `{ credential }` JSON body from the client
- Constructs full URL from `AUTH_ENDPOINT` env var: `${AUTH_ENDPOINT}/token`
- POSTs form-encoded: `grant_type=id_token&id_token={credential}&provider=google`
- On success: sets `access_token` and `refresh_token` as httpOnly secure SameSite=Strict cookies, returns 200
- On 4xx from auth service: returns 401 (client redirects to `/denied`)
- On 5xx from auth service: returns 500

**`/api/logout/+server.ts` — Logout**

- Clears `access_token` and `refresh_token` cookies (set maxAge=0)
- Returns 200

**`/authenticated/+page.server.ts` — Server Load**

- Reads `access_token` cookie
- If missing: redirect to `/`
- Decodes JWT payload (base64url decode the middle segment, parse JSON) — no signature verification needed since this is a demo app and the token was issued by the co-located auth service; a production app should verify against the `/auth/keys` JWKS endpoint
- Returns user info to the page: `sub`, `email`, custom claims, `exp`

**`/authenticated/+page.svelte` — Authenticated Page**

- Displays "You are authenticated"
- Shows user email, subject, token expiry, any custom claims
- Logout button: POST to `/api/logout`, on success redirect to `/`

**`/denied/+page.svelte` — Denied Page**

- "Authorization denied" message
- "Try again" link to `/`

### `lib/auth.ts`

```ts
export async function exchangeToken(credential: string): Promise<Response> {
  return fetch('/api/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ credential })
  });
}
```

---

## Configuration

### `config/oidc-exchange.toml`

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

All `${VAR}` placeholders are resolved from Lambda environment variables set by the CDK stack.

### Google Auth Setup (placeholder)

Users must:
1. Create a Google Cloud project
2. Enable the Google Identity Services API
3. Create an OAuth 2.0 client ID (Web application type)
4. Set authorized JavaScript origins to the API Gateway URL
5. Pass `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` as CDK context:
   ```bash
   cdk deploy -c googleClientId=xxx -c googleClientSecret=xxx
   ```

---

## README (`examples/aws-web/README.md`)

Documents:
1. Prerequisites (AWS CLI, CDK, Node.js 24, Google Cloud project)
2. Building the oidc-exchange binary (`cargo lambda build --release` → copy to `bootstrap/`)
3. Building the demo app (`cd demo-app && npm install && bash bundle.sh`)
4. Deploying (`cd infra && cdk deploy -c googleClientId=xxx -c googleClientSecret=xxx`)
5. Testing (visit the API Gateway URL)
6. Cleanup (`cdk destroy`)
