# Task 03 — Registration policy re-evaluation

**Plan:** [plan.md](../plan.md)  
**Status:** Done  
**Implements:** [source spec](../../../changes/merged/2026-08-05-fail_closed_across_config_and_adapters.md) → Service flows Exchange step 3, Decisions, Implementation note 1, and Compatibility; [service flows canonical page](../../../service/specs/03-service-flows.md) → Token exchange and Decisions  
**Depends on:** 01  
**Produces:** exhaustive typed `RegistrationMode` handling and a single verified-email/domain-allowlist predicate used for both found and not-found users, with correctly attributed denial auditing.  
**Pointers:** `crates/core/src/service/exchange.rs`; `crates/core/src/config.rs`; `crates/core/tests/exchange.rs`; `crates/core/src/domain/audit.rs`.

## Steps

- [x] Replace equality comparison against `"existing_users_only"` with an exhaustive match on
  the resolved `RegistrationMode`; there must be no unrecognised-string fallback.
- [x] Extract one predicate/operation that receives current ID-token claims, allowlist, optional
  existing user id, and request context. It requires `email_verified == Some(true)` for every JIT
  create and, when an allowlist exists, requires a matching current email domain.
- [x] Call that operation on the found-user arm after active-status validation and on the
  not-found arm before creation. A found-user denial must emit `RegistrationDenied` with that
  user id; a not-found denial has no subject.
- [x] Keep mode as an admission gate only: it controls only the no-user creation path. Do not
  add provisioning provenance, re-evaluate mode for existing users, or change refresh behavior.
- [x] Preserve conflict re-lookup and suspended-user behavior; after a race re-lookup, apply the
  same found-user policy necessary to avoid a bypass.
- [x] Add focused exchange tests for: open creation with verified email and no allowlist;
  unverified/missing email denial on creation; matching/nonmatching allowlist; existing active
  user denied after allowlist tightening with `RegistrationDenied` naming their id; existing user
  accepted when claims match; `ExistingUsersOnly` denies only creation; and race/suspension
  regressions.

## Definition of done

- [x] Registration mode is a resolved closed type and all service matches are exhaustive.
- [x] The verified-email rule is not conditional on enabling an allowlist for JIT creation.
- [x] A currently verified email outside a tightened allowlist denies both new and found users;
  found-user audit data includes the user id.
- [x] Existing-user mode and refresh semantics remain as stated in the source spec, with no
  out-of-scope schema migration or refresh-policy change.
- [x] Positive and negative exchange tests plus relevant core formatting/lint checks are reported.

## Execution evidence — 2026-08-16

- Completed in PR25; implementation and focused verification are covered by the final workspace suite: `cargo nextest run --workspace --no-fail-fast` — **389 passed, 27 skipped**.

## Sibling boundaries

- Provenance required to make registration mode retroactive and any refresh-side reevaluation are
  intentionally excluded by the source spec; record them as future work rather than absorbing
  them here.
