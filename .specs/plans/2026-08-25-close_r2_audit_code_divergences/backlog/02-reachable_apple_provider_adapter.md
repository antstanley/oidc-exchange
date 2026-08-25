# Task 02 — Reachable Apple provider adapter

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-reachable_apple_provider_adapter-certificate.md](02-reachable_apple_provider_adapter-certificate.md)

**Implements:** [05-provider-system.md](../../../service/specs/05-provider-system.md) §Tier 2 Apple / §Provider registry, [06-configuration.md](../../../service/specs/06-configuration.md) §Validation at load (`providers.<name>.adapter` domain), [02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §Adapter inventory; change spec §The delta → S1
**Depends on:** —
**Produces:** `[providers.x] adapter = "apple"` resolves and boots the shipped `AppleProvider`; a storage/key adapter value on a provider block is rejected at config load rather than at registry build.
**Pointers:** `crates/core/src/config.rs:1728` (`ProviderConfig.adapter`), `:1737-1775` (`ProviderConfig::resolve`, Oidc-only issuer at `:1756-1764`), `:1999-2041` (`ProviderAdapter`); `crates/server/src/bootstrap.rs:1596-1616` (`build_single_provider`, dead `"apple"` arm at `:1607`)

## Steps

- [ ] Add `enum IdentityProviderAdapter { Oidc, Apple }` beside `ProviderAdapter` in `config.rs`, with `as_str()` and a `parse_field` accepting exactly `"oidc"`/`"apple"` and rejecting everything else with the existing `providers.adapter: invalid provider adapter …` wording.
- [ ] Retype `ProviderConfig.adapter` to `IdentityProviderAdapter` and parse through it in `ProviderConfig::resolve`; keep the issuer requirement (`:1756-1764`) on `Oidc` only — `Apple` pins its issuer internally and reads from `extra`.
- [ ] Match the enum variants directly in `build_single_provider` (`bootstrap.rs:1596`): `Oidc` → `provider_config_to_oidc`, `Apple` → `AppleProvider::from_config(&config.extra)`; delete the string match and its `other` arm.
- [ ] Leave `ProviderAdapter` (all nine values) and its four other fields untouched; only `ProviderConfig.adapter` stops parsing through it.

## Definition of done

- [ ] Resolve-level test through `resolve_config_toml`: an `[providers.apple] adapter = "apple"` block with its `extra` settings resolves, the provider's adapter is `Apple`, and no `issuer` is required.
- [ ] Negative-space tests: `adapter = "atproto"` and `adapter = "postgres"` on a provider block each fail resolution with a `ConfigError` naming `providers.adapter` (pinning the failure point moved to config load); a new test asserts `[providers.x] adapter = "oidc"` without `issuer` fails resolution with the Oidc-only HTTPS-URL error.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; the new domain is a named enum — see plan.md baseline).
- [ ] Reviewable: a reviewer boots the Apple config fixture through `resolve_config_toml` and confirms it resolves to an `Apple` adapter, and that a `postgres` provider value is now a config-load error.
