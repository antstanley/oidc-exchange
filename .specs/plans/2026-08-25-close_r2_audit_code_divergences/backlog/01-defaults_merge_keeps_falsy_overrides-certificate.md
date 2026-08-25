# Done Certificate — Task 01: Defaults merge keeps explicit falsy overrides

**Task:** [01-defaults_merge_keeps_falsy_overrides.md](01-defaults_merge_keeps_falsy_overrides.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 01. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Explicit `false`/`0`/`""` config overrides survive resolution instead of silently reverting to `config/default.toml`; an explicitly empty string fails loudly in its domain resolver.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break the placeholder-resolution and env-override channels that flow through `resolve_builder`/`resolve_config_toml`/`parse_config`, nor the inheritance of committed defaults for keys the operator never set.

## Obligations

- **O1 — Falsy overrides survive resolution.**
  - *Claim:* through `resolve_config_toml`, `[token] refresh_rotation = false` resolves `false`, `[rate_limit] per_subject = 0` resolves `0`, and `[rate_limit] enabled = false` resolves `false`.
  - *Evidence to collect:* run the new bootstrap config tests (beside the existing tests in `crates/server/src/bootstrap.rs`) for the three cases — expect each resolved `AppConfig` field to equal the explicit falsy value; confirm each test fails against the pre-fix `remove_empty_values` code (or that the removed function no longer exists).
  - *Checks:* confirm `remove_empty_values` is deleted (`grep` `crates/server/src/bootstrap.rs`) and that `merge_raw_defaults` now takes two `toml::Value` trees; resolve the merge call in `resolve_builder`/`resolve_config_toml` to the recursive table-merge, not the old `RawConfig` round-trip.
  - *Status:* ☐ unverified

- **O2 — Negative-space and preservation.**
  - *Claim:* an explicit `[token] access_token_ttl = ""` fails resolution with the duration parser's error; a config omitting the switches still inherits `config/default.toml` (`true`, `10`, `true`); `OIDC_EXCHANGE__TOKEN__REFRESH_ROTATION=false` through `parse_config` resolves `false`.
  - *Evidence to collect:* run the empty-string test — expect a `ConfigError` from the duration parser (not a silent revert to the default TTL); run the omit-keys test — expect the committed defaults; run the env-path test — expect `refresh_rotation == false`.
  - *Checks:* trace `access_token_ttl = ""` through the merge → deserialize → resolve path and confirm the empty string reaches the duration resolver rather than being stripped before it.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the behaviour is tested, touched functions carry meaningful assertions, and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: a falsy override reaches the resolved config unchanged (Reviewable).**
  - *Claim:* a reviewer runs the new bootstrap config tests and sees a `false`/`0`/`""` override reach the resolved `AppConfig` unchanged, and an empty duration rejected, where it previously reverted.
  - *Evidence to collect:* run the task's bootstrap config test module and read one resolved-value assertion for a falsy override plus the empty-duration rejection; confirm the outcomes match the change spec's S2 regression table.
  - *Status:* ☐ unverified

## Regression check

- `resolve_builder` (`bootstrap.rs:188`) callers on the server/lambda/config-check entry points: with a config that omits the falsy switches, expect the resolved `AppConfig` still inherits the committed defaults (unchanged behaviour) : ☐ (PRESERVED / REGRESSION)
- The placeholder-resolution path that produces `${VAR}`-derived values: an unset placeholder must still surface as its documented empty/malformed outcome rather than a new panic : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the change is behaviour-visible for deployments that relied on the bug (an explicit `""` that used to revert now fails loudly). This is intended per the change spec's Compatibility note; it is not a regression to flag.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
