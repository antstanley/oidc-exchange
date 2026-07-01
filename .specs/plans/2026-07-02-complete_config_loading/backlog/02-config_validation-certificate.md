# Done Certificate — Task 02: Config validation

**Task:** [02-config_validation.md](02-config_validation.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

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
  - *Status:* ☐ unverified

- **O2 — No secret required when the internal API is not served.**
  - *Claim:* `validate()` returns `Ok(())` for a config whose role excludes the internal API, or whose `internal_api.enabled == false`, even with an empty/absent secret.
  - *Evidence to collect:* run the test covering `role = "exchange"` and the test covering `enabled = false` with no secret — expect `Ok`. Trace the served-condition in `validate()` and confirm it is `(role is admin|all) && enabled`.
  - *Status:* ☐ unverified

- **O3 — Negative-space tests, named constants, meaningful assertions.**
  - *Claim:* a negative test exists per rejected path; the allowed-role set (and any other new bound) is a named constant; the function/tests carry ≥2 meaningful assertions.
  - *Evidence to collect:* enumerate the `#[test]` cases in `config.rs`; confirm one per rejected shape. Grep for the role set as a named `const`/slice referenced by name rather than inline string literals at the check site.
  - *Status:* ☐ unverified

- **O4 — Meets the repo definition of done.**
  - *Claim:* tests, lint, format clean.
  - *Evidence to collect:* run `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` (per `.specs/development-guidelines.md` §Definition of done) — expect all clean.
  - *Status:* ☐ unverified

- **O5 — Reviewable: malformed cases `Err`, well-formed and not-served-secret cases `Ok`.**
  - *Claim:* a reviewer runs the validate tests and sees each malformed-config case fail closed and the valid/not-served cases pass.
  - *Evidence to collect:* run `cargo nextest run -p oidc-exchange-core validate` and observe the negative cases return `Err` and the positive/not-served cases return `Ok`.
  - *Status:* ☐ unverified

## Regression check

- `matches_domain_allowlist` (`crates/core/src/service/exchange.rs:23`) still matches a valid `*.example.com`/exact entry at request time (validate() only gates *acceptance* of malformed entries at startup) → expect unchanged matching : ☐ (PRESERVED / REGRESSION)

## Residue

- Whether to also harden `matches_domain_allowlist` against malformed shapes (belt-and-suspenders) is an Open question in the task; not an obligation here.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
