# Task 01 — Conflict error variant and 409 mapping

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-conflict_error_and_409-certificate.md](01-conflict_error_and_409-certificate.md)

**Implements:** [02-ports-and-adapters.md](../../../service/specs/02-ports-and-adapters.md) §UserRepository (the `create_user` conflict / `update_user` version-atomicity / delete-frees-id contract paragraph, stated contract-first here; the version and deletion behaviours are realized by tasks 08 and 09) · [04-http-api.md](../../../service/specs/04-http-api.md) §Error mapping (`Conflict` → 409 `conflict`) · repo-wide [canonical-types.schema.json](../../../canonical-types.schema.json) `$defs.OAuthErrorEnvelope`
**Depends on:** —
**Produces:** A first-class `Error::Conflict` variant that maps to `409 {"error":"conflict"}`, added to the closed `OAuthErrorEnvelope` error enum, so adapters (tasks 05, 06) and the exchange flow (task 03) can distinguish "already registered" from an infrastructure failure.
**Pointers:** `crates/core/src/error.rs` (add the variant) · `crates/server/src/error.rs:51-108` (`map_domain_error`, add the arm) · `.specs/canonical-types.schema.json` `$defs.OAuthErrorEnvelope.properties.error.enum` · `crates/ffi/src/lib.rs` (verify no domain-error table exists to extend)

## Steps

- [x] Add `Conflict { detail: String }` to the `Error` enum in `crates/core/src/error.rs` with a `thiserror` message (`"conflict: {detail}"`), placed with the 4xx auth-flow variants.
- [x] Add a `map_domain_error` arm in `crates/server/src/error.rs` mapping `Error::Conflict { detail }` to `(StatusCode::CONFLICT, "conflict", detail)`; keep the match exhaustive (no wildcard).
- [x] Add `"conflict"` to the `$defs.OAuthErrorEnvelope.properties.error.enum` in the repo-wide `.specs/canonical-types.schema.json`, in the change spec's stated position (after `unsupported_grant_type`, before `server_error`).
- [x] Update the `04-http-api.md` error-mapping table with a `Conflict | 409 | conflict` row.
- [x] Replace the `02-ports-and-adapters.md` §UserRepository contract paragraph with the change spec's create-conflict / version-atomicity / delete-frees-id wording.
- [x] Confirm `crates/ffi/src/lib.rs` maps errors through the axum `oneshot` HTTP path (no domain-error code table); record that no FFI change is required.

## Definition of done

- [x] A `Conflict` domain error renders `409` with body `{"error":"conflict","error_description":<detail>}` and validates against the updated `OAuthErrorEnvelope` schema.
- [x] The `map_domain_error` match is exhaustive over the enum including the new variant (no `_ =>` arm), so clippy's non-exhaustive check would catch a future addition.
- [x] Negative-space test: an error code other than the eight enum members is rejected by the `OAuthErrorEnvelope` schema (the enum stays closed).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer constructs `ApiError::Domain(Error::Conflict { detail: "…" })`, renders it in a server unit test, and observes `409` with `error == "conflict"`.
