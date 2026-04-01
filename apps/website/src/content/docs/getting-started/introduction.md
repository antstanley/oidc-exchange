---
title: Introduction
description: What oidc-exchange does and how it works.
---

oidc-exchange is a Rust service that validates ID tokens from third-party OIDC providers (Google, Apple, and others) and exchanges them for self-issued access and refresh tokens.

Your client application handles the OAuth flow with the identity provider and sends the resulting authorization code or ID token to oidc-exchange. The service validates the token, creates or looks up the user, and returns a short-lived JWT access token and a long-lived refresh token.

## Token exchange flow

![Token exchange flow diagram](/oidc-exchange.png)

The diagram above shows the three participants in the exchange: your **User Application**, the **Identity Provider** (Google, Apple, etc.), and **oidc-exchange**.

1. **Initiate sign-in** — the user taps "Sign In" and your app redirects to the identity provider's `/authorize` endpoint.
2. **Authenticate with the provider** — the provider presents its login form. The user enters their credentials and submits them back to the provider via `POST /authorize`.
3. **Redirect with ID token** — after successful authentication the provider issues a 302 redirect back to your app's callback URL, including the ID token.
4. **Exchange the token** — your app sends a `POST /token` request to oidc-exchange with the ID token. The service then:
   - Validates the ID token (signature, issuer, audience, expiry)
   - Applies registration policy checks (domain allowlist, open/existing-users mode)
   - Looks up or creates the user
   - Generates a refresh token (256-bit random, stored hashed)
   - Signs a short-lived JWT access token
5. **Receive credentials** — oidc-exchange responds with `{ access_token, refresh_token, token_type, expires_in }` and your app signs the user in.

## Features

- **Token Exchange** — accepts authorization codes from OIDC providers, validates ID tokens, issues short-lived JWTs (default 15min) and long-lived refresh tokens (default 30 days)
- **Pluggable Providers** — standard OIDC (Google, config-only), OIDC-with-quirks (Apple, ES256 client JWT), and non-OIDC (atproto, planned)
- **Hexagonal Architecture** — all infrastructure behind trait interfaces: database, key management, audit, user sync
- **Registration Policy** — open or existing-users-only mode with optional email domain/subdomain allowlists
- **Per-User Claims** — configurable custom JWT claims from TOML templates and per-user overrides via internal API
- **Audit Trail** — syslog severity levels, configurable blocking threshold, CloudTrail Lake or SQS integration
- **OpenTelemetry** — pluggable exporters (OTLP, X-Ray, stdout) via the `tracing` ecosystem
- **Dual Runtime** — same binary runs as an axum server or AWS Lambda function
- **Internal Admin API** — user CRUD and claims management with shared-secret authentication

## Next steps

- [Quick Start](/getting-started/quick-start/) — build and run in 5 minutes
- [Why oidc-exchange?](/getting-started/why-oidc-exchange/) — comparison with alternatives
- [Deployment guides](/deployment/overview/) — choose your infrastructure
