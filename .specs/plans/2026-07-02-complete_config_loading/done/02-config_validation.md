# Task 02 — Config validation

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-config_validation-certificate.md](02-config_validation-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) → Validation at load (role, TTLs, registration allowlist, served-but-empty internal secret); the non-empty-secret half of → Sections → `[internal_api]`
**Depends on:** 01
**Produces:** `AppConfig::validate()` — a fail-closed check that returns `ConfigError` for an out-of-range `server.role`, an unparseable/overflowing TTL, a malformed allowlist entry, or a served internal API with a missing/empty `shared_secret`; and returns `Ok(())` for well-formed config.
**Pointers:** `crates/core/src/config.rs` (add `impl AppConfig { pub fn validate(&self) -> Result<(), Error> }`); role field `crates/core/src/config.rs:29`; reuse `parse_duration_secs` from `crates/core/src/service/mod.rs` (task 01); allowlist shape mirrors `matches_domain_allowlist` `crates/core/src/service/exchange.rs:23-38`

## Steps

- [x] Add `AppConfig::validate(&self) -> Result<(), Error>` returning `Error::ConfigError` with a specific `detail` per failing rule.
- [x] Role: reject any `server.role` not in `all` | `exchange` | `admin` (define the allowed set as a named constant).
- [x] TTLs: call `parse_duration_secs` (task 01) on both `token.access_token_ttl` and `refresh_token_ttl`, propagating its `ConfigError`.
- [x] Allowlist: for each `registration.domain_allowlist` entry, accept only an exact domain (`example.com`) or a `*.`-prefixed wildcard (`*.example.com`); reject bare `*` and dotless prefixes (`*example.com`).
- [x] Internal secret: when the internal API will be served (`role` is `admin` or `all` **and** `internal_api.enabled == true`), require `internal_api.shared_secret` present and non-empty.
- [x] Ensure `parse_duration_secs` is reachable from `config.rs` (adjust its visibility from `pub(crate)` within the crate as needed) without widening it beyond the core crate.
- [x] Add unit tests: one positive (well-formed config passes) and one negative per rule (bad role, unparseable TTL, each rejected allowlist shape, served-with-empty-secret).

## Definition of done

- [x] `validate()` returns `Ok(())` for a well-formed config and a `ConfigError` naming the offending field for each of: bad role, bad TTL, `*` allowlist entry, `*example.com` allowlist entry, and served internal API with empty/missing secret.
- [x] `validate()` does **not** require a secret when the internal API is not served (role excludes it, or `enabled == false`) — covered by a test.
- [x] Negative-space tests exist for every rejected path; the allowed role set and any other new bound are named constants; the function carries at least two meaningful assertions (or its tests do).
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [x] Reviewable: run `cargo nextest run -p oidc-exchange-core validate` and confirm each malformed-config case returns `Err` and the well-formed and not-served-secret cases return `Ok`.

## Open questions

- Whether to also harden `matches_domain_allowlist` (`exchange.rs:33-38`) against the malformed shapes as belt-and-suspenders, or rely solely on `validate()` rejecting them at startup. The plan relies on `validate()`; the matcher already handles well-formed entries correctly.
