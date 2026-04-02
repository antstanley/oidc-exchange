# OIDC-Exchange Fastify Example

Demonstrates embedding oidc-exchange as a library inside a Fastify application.

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

3. Start the server:

   ```bash
   npm start
   ```

The OIDC-Exchange endpoints are available under `/auth/*` (e.g. `http://localhost:8080/auth/.well-known/openid-configuration`).
