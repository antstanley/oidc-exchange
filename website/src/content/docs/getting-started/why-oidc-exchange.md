---
title: "Why oidc-exchange?"
description: "How oidc-exchange compares to Auth0, Keycloak, and rolling your own."
---

If your application needs to authenticate users via Google, Apple, or other OIDC providers, you typically have three choices: a hosted auth service (Auth0, Cognito, Firebase Auth), a full-blown self-hosted OIDC server (Keycloak, Dex, Ory Hydra), or rolling your own token validation. Each comes with trade-offs that oidc-exchange is designed to avoid.

## Compared to hosted auth services (Auth0, Cognito, Firebase Auth)

Hosted services are convenient but introduce external dependencies that affect cost, latency, and control:

- **No per-MAU pricing** --- oidc-exchange runs on your own infrastructure. You pay for compute and storage, not per authenticated user.
- **No vendor lock-in** --- your user data stays in your database, your tokens are signed with your keys, and your configuration is a TOML file in your repo.
- **No opaque behavior** --- every decision (registration policy, claims mapping, token lifetime) is explicit in configuration. There are no hidden rules or console toggles to discover in production.
- **Lower latency** --- token exchange happens in-process or within your VPC. There is no round-trip to a third-party service on every authentication.

## Compared to full OIDC servers (Keycloak, Dex, Ory Hydra)

Full OIDC servers are designed to be the identity provider --- they manage user credentials, host login pages, and implement the full OAuth 2.0 authorization server spec. If you are delegating authentication to external providers and just need to issue your own tokens, they are dramatically over-scoped:

- **No login UI to maintain** --- oidc-exchange does not host login pages or manage passwords. Your client handles the provider's OAuth flow and sends the resulting code or ID token. The service validates and exchanges.
- **No session management** --- there are no server-side sessions, cookies, or consent screens. You get a JWT and a refresh token.
- **Single-purpose** --- the entire codebase does one thing: validate upstream identity, issue downstream tokens. This makes it auditable, testable, and operationally simple.
- **Minutes to deploy, not days** --- a single binary, a TOML config, and a DynamoDB table. No database migrations, no admin consoles, no clustering configuration.

## Compared to rolling your own

Writing token validation and JWT issuance from scratch is straightforward until it isn't:

- **Provider quirks handled** --- Apple requires generating a per-request ES256 client JWT instead of using a static client secret. Standard OIDC libraries don't account for this. oidc-exchange does.
- **Security defaults** --- refresh tokens are stored hashed (SHA-256), access tokens are short-lived, registration policy enforcement and domain allowlists are built in.
- **Audit trail included** --- every token exchange, revocation, and user event can be logged to CloudTrail Lake with syslog severity levels. Adding this after the fact is painful.
- **Hexagonal architecture** --- swapping DynamoDB for Postgres or KMS for Vault means implementing a trait, not rewriting the service.

## When to use something else

oidc-exchange is not a general-purpose authorization server. Choose a different tool if you need:

- **Password-based authentication** --- oidc-exchange delegates authentication entirely to upstream providers.
- **OAuth 2.0 authorization server** --- if you need to issue tokens to third-party clients with scopes and consent, use a full OIDC server.
- **Multi-tenant SaaS auth** --- if you need organization management, RBAC, or SCIM provisioning, a hosted service like Auth0 or WorkOS is better suited.
- **Federation between internal services** --- if you need service-to-service authentication (mTLS, SPIFFE), oidc-exchange is the wrong layer.
