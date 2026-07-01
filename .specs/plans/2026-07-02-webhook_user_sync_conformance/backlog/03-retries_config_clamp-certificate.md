# Done Certificate — Task 03: Clamp `[user_sync.webhook].retries` at config load

**Task:** [03-retries_config_clamp.md](03-retries_config_clamp.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-02 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** A configured `[user_sync.webhook].retries` above a named maximum of 10 is clamped to 10 at config load, logging a warning; a valid value passes through unchanged.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break `WebhookConfig` deserialization (`config.rs:179-185`), its redacted-`secret` `Debug` (`config.rs:187-196`), the default of 2 when `retries` is unset (`bootstrap.rs:436`), or the `build_user_sync` webhook wiring (`bootstrap.rs:436-445`).

## Obligations

- **O1 — `retries = 20` yields effective 10; in-range and default pass through.**
  - *Claim:* the effective retries is `min(configured, MAX_WEBHOOK_RETRIES)`; `retries = 20` → 10, `retries = 5` → 5, unset → 2.
  - *Evidence to collect:* read the new clamp method on `WebhookConfig` in `crates/core/src/config.rs`. Run the new `config.rs` unit test — expect PASS asserting `20 → 10`, `5 → 5`, and the unset default `→ 2`.
  - *Checks:* resolve the clamp call site at `crates/server/src/bootstrap.rs:436` — confirm the value handed to `WebhookUserSync::new` comes through the clamp method, not the raw `wh_cfg.retries.unwrap_or(2)`.
  - *Status:* ☐ unverified

- **O2 — The maximum is a named constant and reducing the value logs a warning.**
  - *Claim:* the maximum is a named constant (e.g. `MAX_WEBHOOK_RETRIES = 10`), not a literal, and clamping emits a `tracing::warn!` naming the configured and clamped values.
  - *Evidence to collect:* read `config.rs` — confirm the named constant and a `tracing::warn!` in the clamp path that includes the configured and clamped values. Confirm the negative-space test exercises the out-of-range (`20`) input.
  - *Checks:* confirm the warning fires only when the configured value exceeds the maximum, not for in-range values.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, lint/format clean, the bound is a named constant, touched functions keep ≥2 meaningful assertions.
  - *Evidence to collect:* run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run -p oidc-exchange-core -p oidc-exchange-server` — expect all clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: `retries = 20` is reduced to 10 with a warning at load.**
  - *Claim:* a reviewer can confirm a config with `retries = 20` hands the webhook adapter an effective 10 and logs a warning.
  - *Evidence to collect:* run the config clamp test and read the `bootstrap.rs:436` call site; observe the clamp is applied before `WebhookUserSync::new` and that a warning is logged for the out-of-range value.
  - *Status:* ☐ unverified

## Regression check

For each module the task touched, the validator traces one downstream caller:

- `build_user_sync` still constructs a `WebhookUserSync` with the configured URL/secret/timeout and now a clamped `retries` → expect the existing config deserialization tests (`config.rs` webhook tests, `bootstrap` wiring) still pass : ☐ (PRESERVED / REGRESSION)

## Residue

- The per-attempt adapter cap (task 02) is a separate, complementary bound; this task only clamps the configured count. Not an obligation here.

## Conclusion

VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
