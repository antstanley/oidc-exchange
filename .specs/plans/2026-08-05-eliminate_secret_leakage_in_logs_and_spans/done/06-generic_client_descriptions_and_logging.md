# Task 06 — Separate public descriptions from internal diagnostics

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §04-http-api — Error mapping](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs04-http-apimd--error-mapping-modify); [§Implementation notes step 5](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`04-http-api.md` Error mapping](../../../service/specs/04-http-api.md)
**Depends on:** 03
**Produces:** `Error::client_description()` supplies stable static public text for every domain variant; server mapping logs full internal `Display` for all mapped classes, under the request span, without publishing diagnostics.
**Pointers:** `crates/core/src/error.rs`; `crates/server/src/error.rs`; `crates/server/tests/routes.rs:148`; existing `invalid_grant_emits_no_server_error_detail_log` test.

## Steps

- [x] Add exhaustive `Error::client_description(&self) -> &'static str` mapping with a small fixed description set that embeds no caller input, library text, key state, or cache internals.
- [x] Refactor `map_domain_error_inner` so every domain-error arm emits `client_description()` while retaining the specified status/error code and the static `UnsupportedGrantType` description.
- [x] Move internal diagnostic logging out of the current `server_error`-only branch: log 5xx at `error!`, 4xx at `warn!`, within the existing request span.
- [x] Add/debug-assert the mapping invariant that returned text equals `client_description()` for every arm; remove assumptions that public text differs only for server errors.
- [x] Update leaking expectations and add tests proving unknown `kid` is not echoed and bad signature/expired/wrong-audience grants are indistinguishable in response body while their internal details remain logged with request ID.

## Task-specific definition of done

- [x] Every mapped domain error returns only a static client description and no internal `reason`/`detail` crosses `/token`.
- [x] All mapped 4xx and 5xx errors produce the correctly leveled operator event under the request span.
- [x] Existing tests that codified leaked `code already used`/`contains("code")` behavior are replaced with generic-description expectations.
- [x] No certificate file is created; test output is the completion evidence.

**Evidence:** commit `feat(server)` (task 06). Core: exhaustive `Error::client_description()` — thirteen variants over a fixed set (grant/token-validation failures share "the provided grant could not be validated" etc.); unit tests assert non-empty stable text for every variant, no sentinel/reason material in any description, and signature/expiry/audience indistinguishability at the type level. Server: `map_domain_error_inner` returns `client_description().to_string()` in every arm (`UnsupportedGrantType` keeps its own static description); `map_domain_error` now logs every class — 5xx via `tracing::error!`, 4xx via `tracing::warn!`, both carrying the full internal `Display` inside the active request span — and guards the split twice: production `assert_ne!` against the full `Display` plus a `debug_assert_eq!` that each arm's text equals `err.client_description()`. Tests: the leak-codifying `invalid_grant_emits_no_server_error_detail_log` replaced by `invalid_grant_returns_generic_body_and_logs_reason_at_warn` (generic body + zero error-level events + exactly one warn event carrying the reason); new `unknown_kid_is_not_echoed_but_is_logged` (kid sentinel absent from body, present in warn log); `grant_validation_failures_are_indistinguishable_in_responses` (bad signature/expired/wrong audience → identical status/code/description, each detail still logged); `warn_log_inherits_the_active_request_span` proves via a LookupSpan capture layer that the warn event carries the enclosing span's `request_id`; conflict/not-found body tests now expect the static descriptions with negative space ("sub-123"/"abc-123" absent); routes.rs missing-code test expects the generic invalid_request description and asserts it does not contain "code". Full workspace: 445 nextest tests passed / 28 skipped ([ignored] integration tier), `cargo fmt --all --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean.
