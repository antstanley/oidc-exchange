# Task 01 — Typed email-verification form in core

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-email_verification_typed_form-certificate.md](01-email_verification_typed_form-certificate.md)

**Implements:** change spec [§The delta → Typed form](../../../changes/2026-08-31-per_provider_email_verification_overrides.md); [01-domain-model.md](../../../service/specs/01-domain-model.md) §OidcProviderConfig (the code side — the prose and sidecar are republished by task 04)
**Depends on:** —
**Produces:** `EmailVerification` (default `Standard`) and `OidcProviderConfig.email_verification` exist, re-exported, set in every struct-literal constructor and emitted by the hand-written Debug; the workspace is green and behaviour is byte-identical to before.
**Pointers:** `crates/core/src/domain/provider.rs:8-59` (struct and Debug), `:67-81` (`sample_config`); `crates/core/src/domain/mod.rs:18` (re-export); constructors to update: `crates/server/src/bootstrap.rs:1702` (`provider_config_to_oidc` — set `EmailVerification::default()` until task 03), `crates/adapters/src/oidc/mod.rs:338-356` (`make_config`), `crates/providers/tests/cross_provider_corpus.rs:193`, `crates/providers/tests/upstream_error_leak_corpus.rs:50` and `:172`, `crates/server/tests/request_leak_oracle.rs:599`

## Steps

- [ ] Add the `EmailVerification` enum to `crates/core/src/domain/provider.rs` exactly as the change spec's Typed form block: `#[derive(Debug, Clone, Default, PartialEq, Eq)]`, variants `#[default] Standard`, `TrustEmail`, `Claim(String)`, with the spec's doc comments carried over.
- [ ] Add `pub email_verification: EmailVerification` to `OidcProviderConfig` and emit it from the hand-written `Debug` impl — it is a configuration-grade fact and stays visible, like `endpoint_origins`; the secret redaction is unchanged.
- [ ] Re-export `EmailVerification` beside `OidcProviderConfig` in `crates/core/src/domain/mod.rs`.
- [ ] Set the field in every struct-literal constructor (all seven sites in Pointers); `provider_config_to_oidc` sets `EmailVerification::default()` for now — task 03 replaces that with the lifted variant.
- [ ] Add unit tests beside the existing Debug test: `EmailVerification::default()` is `Standard`; the Debug output names the mode; the secret-sentinel redaction assertion still holds.

## Definition of done

- [ ] The enum and field compile across the workspace; a unit test pins `EmailVerification::default() == EmailVerification::Standard`.
- [ ] The Debug test is extended: the rendered output carries `email_verification` and still never contains the secret sentinel (the existing negative-space assertion is preserved, not replaced).
- [ ] No behavioural change: nothing reads the field yet, and the full existing workspace suite passes unmodified apart from constructor-site additions.
- [ ] Meets the repo definition of done (`cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the `crates/core` domain provider tests, sees the default-mode and Debug assertions pass, and greps the workspace for `OidcProviderConfig {` to confirm every constructor sets the new field.
