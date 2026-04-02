# OIDC-Exchange SvelteKit Example

Demonstrates embedding oidc-exchange as a library inside a SvelteKit application using a server hook to intercept `/auth/*` requests.

## Setup

1. Generate a signing key:

   ```bash
   mkdir -p keys
   openssl genpkey -algorithm ed25519 -out keys/signing.pem
   ```

2. Install dependencies:

   ```bash
   npm install
   ```

3. Start the development server:

   ```bash
   npm run dev
   ```

The OIDC-Exchange endpoints are available under `/auth/*` (e.g. `http://localhost:5173/auth/.well-known/openid-configuration`).
