# Done Certificate — Task 02: Reachable Apple provider adapter

**Task:** [02-reachable_apple_provider_adapter.md](02-reachable_apple_provider_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-08-25 — unverified   <!-- validator sets: Validated YYYY-MM-DD -->

> This certificate is a verification protocol for Task 02. A validating agent discharges it: for
> each obligation, collect the named evidence, run the named checks, set the Status, then derive the
> Conclusion by the rubric below. Do not mark an obligation SATISFIED without its evidence; do not
> record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation names
(a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `[providers.x] adapter = "apple"` resolves and boots the shipped `AppleProvider`; a storage/key adapter value on a provider block is rejected at config load rather than at registry build.
- **P2 — Obligations.** Done iff O1…O4 all hold. One Oi per definition-of-done item, in DoD order; O4 is the Reviewable item.
- **P3 — Invariants.** Must not break `ProviderAdapter`'s four other fields (`key_manager`/`repository`/`session_repository`/`user_sync`), which keep all nine values, nor the generic `Oidc` provider construction path.

## Obligations

- **O1 — Apple block resolves to an `Apple` adapter.**
  - *Claim:* through `resolve_config_toml`, an `[providers.apple] adapter = "apple"` block with its `extra` settings resolves, the provider's adapter is `Apple`, and no `issuer` is required.
  - *Evidence to collect:* run the new resolve-level Apple boot test — expect success and `adapter == IdentityProviderAdapter::Apple`; read `ProviderConfig::resolve` (`config.rs:1737`) and confirm the issuer requirement (`:1756-1764`) is gated on `Oidc` only.
  - *Checks:* resolve the constructor dispatched in `build_single_provider` (`bootstrap.rs:1596`) — confirm `Apple` → `AppleProvider::from_config(&config.extra)` (the previously dead `:1607` arm), reached via the enum variant, not the deleted string match.
  - *Status:* ☐ unverified

- **O2 — Negative-space rejections at config load.**
  - *Claim:* `adapter = "atproto"` and `adapter = "postgres"` on a provider block each fail resolution with a `ConfigError` naming `providers.adapter`; `[providers.x] adapter = "oidc"` without `issuer` fails resolution with the Oidc-only HTTPS-URL error.
  - *Evidence to collect:* run the three negative tests — expect a `ConfigError` at resolution (not at registry build) for `atproto` and `postgres`, each message naming `providers.adapter`; expect the `providers.<id>.issuer: missing required HTTPS URL` error for the issuer-less oidc block.
  - *Checks:* resolve `IdentityProviderAdapter::parse_field` — confirm it accepts exactly `"oidc"`/`"apple"` and that `ProviderConfig.adapter` no longer parses through the shared `ProviderAdapter` (which would have admitted `postgres`).
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the new enum is a named domain, the paths are tested, and format/lint/test gates pass.
  - *Evidence to collect:* run `cargo fmt` (check), `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace` — expect all clean (per [development-guidelines.md](../../../development-guidelines.md) §Definition of done).
  - *Status:* ☐ unverified

- **O4 — Reviewable: Apple fixture boots, postgres provider value errors (Reviewable).**
  - *Claim:* a reviewer boots the Apple config fixture through `resolve_config_toml` and confirms it resolves to an `Apple` adapter, and that a `postgres` provider value is now a config-load error.
  - *Evidence to collect:* run the Apple boot test and the `postgres`-provider negative test together; read the resolved adapter value and the error message and confirm they match the change spec's S1 description.
  - *Status:* ☐ unverified

## Regression check

- The four shared-enum fields (`key_manager.adapter` etc.) still parse every storage/key value: trace `ProviderAdapter::parse_field("repository.adapter", …)` with `"postgres"` → expect `Ok(Postgres)` unchanged : ☐ (PRESERVED / REGRESSION)
- An existing working `[providers.google] adapter = "oidc"` fixture (`bootstrap.rs` tests around `:2338`): expect it still resolves to `Oidc` and builds via `provider_config_to_oidc` : ☐ (PRESERVED / REGRESSION)

## Residue

Notes for the validator: the `role = "admin"` residual break (a provider block with a storage value that boots today only because admin builds no registry, now rejected at config load) is an accepted, documented compatibility change per the change spec — not a regression to flag.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
