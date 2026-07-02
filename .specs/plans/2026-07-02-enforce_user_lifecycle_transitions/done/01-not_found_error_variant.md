# Task 01 — not_found error variant

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-not_found_error_variant-certificate.md](01-not_found_error_variant-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) §Error mapping (`NotFound` → 404 `not_found`; preamble widened beyond RFC 6749 §5.2); [01-domain-model.md](../../../service/specs/01-domain-model.md) (the new `Error::NotFound` error-enum variant)
**Depends on:** —
**Produces:** a `NotFound` domain error that `map_domain_error` renders as HTTP 404 with error code `not_found`
**Pointers:** `crates/core/src/error.rs:4-50` (variant list), `crates/server/src/error.rs:51-108` (`map_domain_error`); backstop context `crates/adapters/src/{dynamo,postgres,sqlite}/mod.rs` `update_user`

## Steps

- [x] Add `Error::NotFound { detail: String }` to the enum in `crates/core/src/error.rs` with a `#[error("not found: {detail}")]` message, placed with the 4xx auth-flow variants.
- [x] Add a match arm in `map_domain_error` (`crates/server/src/error.rs`) mapping `Error::NotFound { detail }` to `(StatusCode::NOT_FOUND, "not_found", detail.clone())`; keep the 5xx catch-all arm exhaustive over the remaining variants.
- [x] Confirm no FFI error table needs updating (record that `crates/ffi/src/lib.rs` proxies HTTP responses and has no domain-error mapping).

## Definition of done

- [x] `map_domain_error(&Error::NotFound { detail })` returns `(404, "not_found", <detail>)`, asserted by a server-crate unit test.
- [x] The `match` in `map_domain_error` stays exhaustive with no wildcard swallowing `NotFound` into the 5xx arm (negative-space: `NotFound` must not render as 500).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: a reviewer reads the new arm and runs the mapping unit test, seeing `Error::NotFound` produce a 404 `not_found` envelope rather than the generic 500 `server_error`.
