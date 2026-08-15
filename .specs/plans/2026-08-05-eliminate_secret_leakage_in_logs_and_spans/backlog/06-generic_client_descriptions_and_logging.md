# Task 06 — Separate public descriptions from internal diagnostics

**Status:** Backlog · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §04-http-api — Error mapping](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#specsservicespecs04-http-apimd--error-mapping-modify); [§Implementation notes step 5](../../../changes/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#implementation-notes); [`04-http-api.md` Error mapping](../../../service/specs/04-http-api.md)
**Depends on:** 03
**Produces:** `Error::client_description()` supplies stable static public text for every domain variant; server mapping logs full internal `Display` for all mapped classes, under the request span, without publishing diagnostics.
**Pointers:** `crates/core/src/error.rs`; `crates/server/src/error.rs`; `crates/server/tests/routes.rs:148`; existing `invalid_grant_emits_no_server_error_detail_log` test.

## Steps

- [ ] Add exhaustive `Error::client_description(&self) -> &'static str` mapping with a small fixed description set that embeds no caller input, library text, key state, or cache internals.
- [ ] Refactor `map_domain_error_inner` so every domain-error arm emits `client_description()` while retaining the specified status/error code and the static `UnsupportedGrantType` description.
- [ ] Move internal diagnostic logging out of the current `server_error`-only branch: log 5xx at `error!`, 4xx at `warn!`, within the existing request span.
- [ ] Add/debug-assert the mapping invariant that returned text equals `client_description()` for every arm; remove assumptions that public text differs only for server errors.
- [ ] Update leaking expectations and add tests proving unknown `kid` is not echoed and bad signature/expired/wrong-audience grants are indistinguishable in response body while their internal details remain logged with request ID.

## Task-specific definition of done

- [ ] Every mapped domain error returns only a static client description and no internal `reason`/`detail` crosses `/token`.
- [ ] All mapped 4xx and 5xx errors produce the correctly leveled operator event under the request span.
- [ ] Existing tests that codified leaked `code already used`/`contains("code")` behavior are replaced with generic-description expectations.
- [ ] No certificate file is created; test output is the completion evidence.
