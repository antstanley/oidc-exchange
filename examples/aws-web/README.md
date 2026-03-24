# AWS Web Example

A reference example that deploys **oidc-exchange** on AWS behind an HTTP API Gateway, with a SvelteKit demo app that uses Google Sign-In to demonstrate the full OIDC token exchange flow.

## Architecture

```
Browser
  |
  v
API Gateway (HTTP API)
  |
  +-- /auth/{proxy+}  -->  Auth Lambda (oidc-exchange, Rust)
  |                            |
  |                            +-- DynamoDB (sessions, clients, tokens)
  |                            +-- KMS (ECDSA P-256 signing key)
  |                            +-- CloudTrail Lake (audit events)
  |
  +-- /{proxy+}       -->  Demo App Lambda (SvelteKit, Node.js)
  +-- $default        -->  Demo App Lambda
```

## Prerequisites

- **AWS CLI** configured with credentials (`aws configure`)
- **AWS CDK CLI** (`npm install -g aws-cdk`)
- **Node.js 24+**
- **Rust** toolchain with `cargo-lambda` (`cargo install cargo-lambda`)
- **Google Cloud project** with OAuth 2.0 credentials (see below)

## Google OAuth Setup

1. Go to the [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project (or select an existing one)
3. Navigate to **APIs & Services > Credentials**
4. Click **Create Credentials > OAuth 2.0 Client ID**
5. Select **Web application** as the application type
6. Under **Authorized JavaScript origins**, add the API Gateway URL (available after first deploy)
7. Note the **Client ID** and **Client Secret**

For the first deploy you can use placeholder values and update the origins after you have the API Gateway URL.

## Build the Auth Service

From the repository root, compile the oidc-exchange binary for Lambda:

```bash
cargo lambda build --release
```

Copy the bootstrap binary into the example directory:

```bash
cp target/lambda/oidc-exchange/bootstrap examples/aws-web/bootstrap/
```

## Build the Demo App

```bash
cd examples/aws-web/demo-app
npm install
bash bundle.sh
```

This produces a `dist/svelteKit/` directory containing the server-side rendered SvelteKit app ready for Lambda deployment.

## Deploy

```bash
cd examples/aws-web/infra
npm install
npx cdk deploy \
  -c googleClientId=YOUR_GOOGLE_CLIENT_ID \
  -c googleClientSecret=YOUR_GOOGLE_CLIENT_SECRET
```

After deployment, CDK outputs the API Gateway URL. Update your Google OAuth 2.0 client's authorized JavaScript origins to include this URL.

## Test

1. Open the **ApiUrl** from the CDK stack outputs in your browser
2. Click the **Sign in with Google** button
3. Complete the Google authentication flow
4. On success, you are redirected to the `/authenticated` page showing your token claims
5. Click **Sign Out** to clear session cookies

## Cleanup

```bash
cd examples/aws-web/infra
npx cdk destroy
```

This removes all AWS resources created by the stack (DynamoDB table, KMS key, Lambdas, API Gateway, CloudTrail Lake).

## Local Development

To run the example locally without deploying to AWS:

1. Start oidc-exchange locally (see the main project README for configuration)
2. Run the SvelteKit dev server:

```bash
cd examples/aws-web/demo-app
npm install
AUTH_ENDPOINT=http://localhost:8080/auth npm run dev
```

3. Set `PUBLIC_GOOGLE_CLIENT_ID` in your environment or in a `.env` file
4. Open `http://localhost:5173` in your browser

Note: For local development, update your Google OAuth client's authorized JavaScript origins to include `http://localhost:5173`.
