# Task 03 — Thread real client provenance into the core flows

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-thread_client_provenance-certificate.md](03-thread_client_provenance-certificate.md)

**Implements:** [01-domain-model.md](../../../service/specs/01-domain-model.md) §Exchange request types / §ClientAddr, [03-service-flows.md](../../../service/specs/03-service-flows.md) §Token exchange (client context); change spec §The delta → S7
**Depends on:** —
**Produces:** Core-flow (`exchange`/`refresh`/`revoke`) audit events record the middleware's true `ip_address_source` (`peer`/`forwarded`/`unknown`) instead of the flattened `asserted`.
**Pointers:** `crates/core/src/service/exchange.rs:34-54` (`ExchangeRequest`), `:121-125` (asserted rebuild), `:491` (session address); `refresh.rs:41-51` (`RefreshRequest`); `revoke.rs:14-25` (`RevokeRequest`), `:44-48`; `crates/core/src/domain/audit.rs:95` (`ClientAddr`); `crates/server/src/middleware/audit_context.rs:62-79` (`resolve_client_addr`); `crates/server/src/routes/token.rs:245,263,275`, `revoke.rs:53`

## Steps

- [ ] Replace `ip_address: Option<String>` with `client_addr: ClientAddr` in `ExchangeRequest`, `RefreshRequest`, and `RevokeRequest`.
- [ ] Add `impl Default for ClientAddr` = `Unknown` (fail-closed) so `RefreshRequest`/`RevokeRequest` keep their `#[derive(Default)]`.
- [ ] Delete the `ClientAddr::asserted(request.ip_address)` rebuilds in `exchange.rs:121-125`, `refresh.rs` (the `ValidationFailed` refusal at `:161-174` swaps its `ClientAddr` argument; the reuse/suspension/success sites fold into task 04's emission calls), and `revoke.rs:44-48`; use `request.client_addr` directly.
- [ ] Populate `Session.ip_address` (still `Option<String>`) via `request.client_addr.audit_address()` (`exchange.rs:491`); route handlers pass `audit_ctx.client_addr.clone()` instead of `audit_ctx.ip_address()` (`token.rs:245,263,275`; `revoke.rs:53`).
- [ ] Update core-test request constructors across `crates/core/tests/{exchange,exchange_mandatory_outcomes,assertion,refresh,revoke,service_leak_corpus,user_admin}.rs`.

## Definition of done

- [ ] Server e2e: a `/token` terminal audit event records `ip_address_source == "peer"`, and `"forwarded"` behind a `server.trusted_proxies` proxy; a request with no server-established address records `"unknown"`.
- [ ] Negative-space / invariant: the stored `Session.ip_address` value is unchanged (`audit_address()`), and `ClientAddr::default()` is `Unknown`; the `Asserted` variant remains in the domain for embedder hints.
- [ ] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [ ] Reviewable: a reviewer drives `/token` through the production router and inspects the emitted audit event's `ip_address_source`, confirming it is the resolved `peer`/`forwarded` value rather than `asserted`.
