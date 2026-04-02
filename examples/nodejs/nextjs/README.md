# OIDC-Exchange Next.js Example

Demonstrates embedding oidc-exchange as a library inside a Next.js App Router application using a catch-all route handler.

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

The OIDC-Exchange endpoints are available under `/auth/*` (e.g. `http://localhost:3000/auth/.well-known/openid-configuration`).
