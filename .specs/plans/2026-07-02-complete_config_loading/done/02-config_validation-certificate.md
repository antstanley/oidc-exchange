# Done Certificate — Task 02: Config validation

**Task:** [02-config_validation.md](02-config_validation.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-02

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O5 below holds, each backed by the evidence the obligation
names — not by assertion.

## Premises

- **P1 — Goal.** `AppConfig::validate()` returns `ConfigError` for a bad role, an unparseable/overflowing TTL, a malformed allowlist entry, or a served-but-empty internal secret, and `Ok(())` for well-formed config.
- **P2 — Obligations.** Done iff O1…O5 all hold; O5 is the `Reviewable:` item.
- **P3 — Invariants.** Must not break `parse_duration_secs` (Task 01), reused here, nor `matches_domain_allowlist` (`crates/core/src/service/exchange.rs:23`), whose valid-entry behaviour is unchanged.

## Obligations

- **O1 — Rejects bad role, bad TTL, malformed allowlist, and served-with-empty-secret.**
  - *Claim:* `validate()` returns `Err(ConfigError)` naming the offending field for each of: `server.role` outside `all|exchange|admin`; an unparseable TTL; a `*` allowlist entry; a `*example.com` allowlist entry; a served internal API (`admin`/`all` + `enabled`) with empty/missing `shared_secret`; and `Ok(())` for well-formed config.
  - *Evidence to collect:* read `AppConfig::validate()` in `crates/core/src/config.rs`; confirm one branch per rule. Run `cargo nextest run -p oidc-exchange-core validate` — expect the positive case `Ok` and each negative case `Err`.
  - *Checks:* resolve the TTL check to `parse_duration_secs` from `crates/core/src/service/mod.rs` (Task 01), not a re-implemented local parser; resolve the allowlist shape check so `*.example.com` is accepted while `*` and `*example.com` are rejected.
  - *Status:* ☑ SATISFIED — read `AppConfig::validate()` (`crates/core/src/config.rs`, added in this diff): one branch per rule (role vs `ALLOWED_SERVER_ROLES`, both TTLs via `prefix_config_error(crate::service::parse_duration_secs(...), "token.…_ttl")`, per-entry `validate_allowlist_entry`, served-internal-API secret check). Ran `cargo nextest run -p oidc-exchange-core validate` → 11/11 PASS: negatives (`validate_rejects_unknown_role`, `_unparseable_access_token_ttl`, `_overflowing_refresh_token_ttl`, `_bare_wildcard_allowlist_entry`, `_dotless_wildcard_allowlist_entry`, `_served_internal_api_with_missing_secret`, `_served_internal_api_with_empty_secret`) each assert `Err(ConfigError)` whose `detail` names the offending field; `validate_accepts_well_formed_default_config` asserts `Ok`. Check: TTL call resolves to `pub(crate) fn parse_duration_secs` at `crates/core/src/service/mod.rs:181` via the `crate::service::` path — the Task 01 function, not a local shadow. Allowlist shape: `validate_allowlist_entry` rejects `starts_with('*') && !starts_with("*.")`, so `*` and `*example.com` are rejected while `*.example.com` and `example.com` pass (`validate_accepts_exact_and_wildcard_allowlist_entries` → PASS).

- **O2 — No secret required when the internal API is not served.**
  - *Claim:* `validate()` returns `Ok(())` for a config whose role excludes the internal API, or whose `internal_api.enabled == false`, even with an empty/absent secret.
  - *Evidence to collect:* run the test covering `role = "exchange"` and the test covering `enabled = false` with no secret — expect `Ok`. Trace the served-condition in `validate()` and confirm it is `(role is admin|all) && enabled`.
  - *Status:* ☑ SATISFIED — `validate_does_not_require_secret_when_internal_api_not_served` → PASS; it covers both sub-cases in one test: `role = "exchange"`, `enabled = true`, `shared_secret = None` → `Ok`; and `role = "admin"`, `enabled = false`, `shared_secret = None` → `Ok`. Traced the served-condition in `validate()`: `matches!(self.server.role.as_str(), "admin" | "all") && self.internal_api.enabled` — exactly `(role is admin|all) && enabled`, and the secret check is inside `if internal_api_served`.

- **O3 — Negative-space tests, named constants, meaningful assertions.**
  - *Claim:* a negative test exists per rejected path; the allowed-role set (and any other new bound) is a named constant; the function/tests carry ≥2 meaningful assertions.
  - *Evidence to collect:* enumerate the `#[test]` cases in `config.rs`; confirm one per rejected shape. Grep for the role set as a named `const`/slice referenced by name rather than inline string literals at the check site.
  - *Status:* ☑ SATISFIED — 11 new `#[test]` cases enumerated; one negative per rejected path (bad role; unparseable TTL; overflowing TTL; bare `*`; `*example.com`; missing secret; empty secret) plus 4 positives (default config, exact+wildcard allowlist, non-served-no-secret, served-with-secret). Role set is the named `const ALLOWED_SERVER_ROLES: [&str; 3]` and the check site reads `ALLOWED_SERVER_ROLES.contains(&self.server.role.as_str())` — no inline literal list. (The served-condition `matches!(…, "admin" | "all")` uses literals, but that is the pre-existing role vocabulary, not a new bound; noted, not a defect.) Every test carries ≥2 meaningful assertions (Err/Ok outcome plus detail-content assertions).

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☑ SATISFIED — `cargo fmt --all -- --check` → clean; `cargo clippy --workspace -- -D warnings` → finished with no warnings; `cargo nextest run --workspace` → 166 tests run: 166 passed, 2 skipped.

- **O5 — Reviewable: malformed cases `Err`, well-formed and not-served-secret cases `Ok`.**
  - *Claim:* a reviewer runs the validate tests and sees each malformed-config case fail closed and the valid/not-served cases pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core validate` and observe the negative cases return `Err` and the positive/not-served cases return `Ok`.
  - *Status:* ☑ SATISFIED — exercised: `cargo nextest run -p oidc-exchange-core validate` → 11/11 PASS (72 skipped by filter). The 7 negative tests observe `Err` via `expect_err` (a returned `Ok` would fail them); the 4 positive/not-served tests assert `is_ok()`.

## Regression check

- `matches_domain_allowlist` (`crates/core/src/service/exchange.rs:23`) still matches a valid `*.example.com`/exact entry at request time (validate() only gates *acceptance* of malformed entries at startup) → expect unchanged matching : ☑ PRESERVED — `jj diff --stat` shows the change touches only `crates/core/src/config.rs`; `matches_domain_allowlist` and `parse_duration_secs` are unmodified, and their tests (`exchange::tests` exact/wildcard matching, `parse_duration_secs_tests`) pass in the workspace run.

## Residue

- Whether to also harden `matches_domain_allowlist` against malformed shapes (belt-and-suspenders) is an Open question in the task; not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O5 all SATISFIED with direct evidence (validate() branches read, TTL check resolved to the Task 01 `parse_duration_secs` at `service/mod.rs:181`, 11/11 targeted tests and the full workspace suite/fmt/clippy clean), and the `matches_domain_allowlist` regression surface is PRESERVED since only `config.rs` changed.
