# Task 03 — Clamp `[user_sync.webhook].retries` at config load

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-retries_config_clamp-certificate.md](03-retries_config_clamp-certificate.md)

**Implements:** [`06-configuration.md` §`[user_sync]`](../../../service/specs/06-configuration.md) — "`retries` is clamped at config validation (maximum 10)"
**Depends on:** —
**Produces:** a configured `[user_sync.webhook].retries` greater than a named maximum of 10 is clamped to 10 at config load time, with a warning logged; a valid value passes through unchanged
**Pointers:** `crates/core/src/config.rs:179-196` (`WebhookConfig` — define the named maximum and the clamp here); `crates/server/src/bootstrap.rs:436` (`let retries = wh_cfg.retries.unwrap_or(2)` — the effective-retries call site that must apply the clamp); `crates/server/src/bootstrap.rs:26-47` (`load_config`)

## Steps

- [ ] Add a named constant for the maximum in `crates/core/src/config.rs` (e.g. `MAX_WEBHOOK_RETRIES: u32 = 10`).
- [ ] Add a method on `WebhookConfig` in `config.rs` that returns the effective retries clamped to `MAX_WEBHOOK_RETRIES`, logging a `tracing::warn!` when the configured value exceeds the maximum (naming the configured and clamped values).
- [ ] Apply the clamp at load time so the value reaching `WebhookUserSync::new` is already bounded — call the new method at `crates/server/src/bootstrap.rs:436` in place of `wh_cfg.retries.unwrap_or(2)` (or validate/clamp in `load_config`), keeping the default of 2 when unset.
- [ ] Add a unit test in `config.rs` asserting `retries = 20` clamps to 10, that an in-range value (e.g. 5) and the unset default (2) pass through unchanged.

## Definition of done

- [ ] A `WebhookConfig` with `retries = 20` yields an effective `retries` of 10 at load time; an in-range value and the unset default are unchanged.
- [ ] The maximum is a named constant (not a literal), and reducing the value logs a warning naming the configured and clamped values (negative-space test covers the out-of-range input).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits, ≥2 assertions per touched function — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the config clamp test and confirms `retries = 20` is reduced to 10 with a warning, so the value handed to the webhook adapter can never exceed the maximum.
