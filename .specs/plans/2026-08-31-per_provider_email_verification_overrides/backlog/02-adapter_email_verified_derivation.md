# Task 02 — Adapter derivation with explicit-claim precedence

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-adapter_email_verified_derivation-certificate.md](02-adapter_email_verified_derivation-certificate.md)

**Implements:** change spec [§The delta → Derivation in the adapter and §Tests (adapter bullet)](../../../changes/2026-08-31-per_provider_email_verification_overrides.md); [05-provider-system.md](../../../service/specs/05-provider-system.md) §OidcProvider behaviour — the `validate_id_token` derivation the new §Email-verification overrides section will describe (prose republished by task 04)
**Depends on:** 01 (build — the enum and config field)
**Produces:** `validate_id_token` derives `email_verified` per the provider's configured mode under the two-step precedence rule; a ten-case wiremock matrix pins every mode, the both-directions passthrough, and the coercion negative space; `Standard` is byte-identical to 0.4.0.
**Pointers:** `crates/adapters/src/oidc/mod.rs:34-42` (`OidcProvider` struct — add the mode field), `:114-123` (`from_config` — copy the mode from config), `:187-201` (`validate_id_token` — replace the single `coerce_bool` at `:190`), `:338-356` (`make_config` test helper); `crates/adapters/src/shared/claims.rs:14` (`coerce_bool`, reused as-is); `crates/providers/src/apple.rs` (untouched)

## Steps

- [ ] Store the mode: add `email_verification: EmailVerification` to `OidcProvider` and copy it from the config in `from_config`.
- [ ] Replace `email_verified: coerce_bool(&claims["email_verified"])` with the precedence rule, as a small named helper carrying at least two assertions (e.g. that an explicit `Some` passes through untouched, and that `Standard` never fabricates a value): step 1 — `coerce_bool(claims["email_verified"])`; an explicit `Some(true)`/`Some(false)` always passes through. Step 2, only when step 1 yields `None` — `Standard` → `None`; `Claim(name)` → `coerce_bool(claims[name])`; `TrustEmail` → `Some(true)` iff `claims["email"]` is a non-empty string, else `None`.
- [ ] Extend `make_config` (or add a mode-aware variant of it) so wiremock tests construct a provider per mode without duplicating the fixture.
- [ ] Add the wiremock cases beside the existing oidc tests — under `Claim("xms_edov")`: JSON `true` → `Some(true)`; string `"true"` → `Some(true)`; claim absent → `None`; `xms_edov: 42` (a JSON number, neither boolean nor `"true"`/`"false"`) → `None` through `coerce_bool`; explicit `email_verified: false` beside `xms_edov: true` → `Some(false)`. Under `TrustEmail`: email present → `Some(true)`; email absent → `None`; empty-string email → `None`; explicit `email_verified: false` → `Some(false)`. Under `Standard`: absent stays `None` (a named pin of today's behaviour).
- [ ] Leave `crates/providers/src/apple.rs` untouched — Apple always emits `email_verified` and reads its own config.

## Definition of done

- [ ] The precedence rule holds in both directions: the `email_verified: false` beside `xms_edov: true` case yields `Some(false)` (overrides fill absence, never overturn), and an explicit `true` is never re-derived.
- [ ] All ten wiremock cases pass — positive and negative space per mode, including the coercion negative-space case (`xms_edov: 42`, a JSON number that is neither boolean nor `"true"`/`"false"`, yields `email_verified = None` through `coerce_bool`).
- [ ] `Standard` is behaviourally identical to 0.4.0: the named pin passes and no pre-existing adapter test changes its expectation; `crates/providers/src/apple.rs` has no diff.
- [ ] Meets the repo definition of done (`cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace`; touched functions carry two meaningful assertions — see plan.md baseline).
- [ ] Reviewable: a reviewer runs the oidc adapter wiremock suite and reads the false-beside-`xms_edov` case as the proof that an explicit provider signal can never be overturned by configuration.
