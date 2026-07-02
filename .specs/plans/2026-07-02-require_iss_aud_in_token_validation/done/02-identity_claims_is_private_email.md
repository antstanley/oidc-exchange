# Task 02 — surface is_private_email on IdentityClaims

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-identity_claims_is_private_email-certificate.md](02-identity_claims_is_private_email-certificate.md)

**Implements:** [01-domain-model.md](../../../service/specs/01-domain-model.md) §"Token types" (`IdentityClaims` field list gains `is_private_email`) and [05-provider-system.md](../../../service/specs/05-provider-system.md) §Decisions *Surface `is_private_email`*; folds the `IdentityClaims` fragment into [canonical-types.schema.json](../../../service/specs/canonical-types.schema.json).
**Depends on:** —
**Produces:** `IdentityClaims` carries a new `is_private_email: Option<bool>` field; the canonical schema and the 01-domain-model prose describe it; every constructor of `IdentityClaims` across the workspace is updated (no backwards-compat shim), so the workspace compiles.
**Pointers:** struct at `crates/core/src/domain/token.rs:74-81`; constructors to update — `crates/adapters/src/oidc/mod.rs:157`, `crates/providers/src/apple.rs:280` (set `None` here; task 04 populates it), `crates/test-utils/src/lib.rs:414` and `:451`, `crates/core/tests/exchange.rs` (six sites: `:264, :308, :333, :358, :388, :501`); port unchanged at `crates/core/src/ports/identity_provider.rs:12`.

## Steps

- [x] Add `pub is_private_email: Option<bool>,` to the `IdentityClaims` struct in `crates/core/src/domain/token.rs`.
- [x] Update every `IdentityClaims { … }` constructor to include the field: `None` in `oidc/mod.rs`, `apple.rs`, `test-utils`, and the `core/tests/exchange.rs` sites (task 04 later populates the Apple one from coercion).
- [x] Fold the `is_private_email` property (`["boolean","null"]`, with the description from the change spec) into `$defs/IdentityClaims` in `crates/adapters/../.specs/service/specs/canonical-types.schema.json` (repo path `.specs/service/specs/canonical-types.schema.json`).
- [x] Update the `IdentityClaims` bullet under §"Token types" in `.specs/service/specs/01-domain-model.md` to list `is_private_email` (Apple private-relay flag; `None` for other providers), and bump that page's `**Date:**`.

## Definition of done

- [x] `IdentityClaims` has `is_private_email: Option<bool>` and the whole workspace compiles with every constructor supplying it (no `..Default` shim; every caller changed).
- [x] `canonical-types.schema.json` `$defs/IdentityClaims` and the 01-domain-model prose both describe `is_private_email`, updated together with the type change.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer builds the workspace (`cargo nextest run --workspace`) with no constructor left un-updated and confirms the schema and 01-domain-model prose name `is_private_email`.
