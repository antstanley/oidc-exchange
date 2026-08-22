# 01 · Typed exchange credential

**Plan:** [plan.md](../plan.md) · **Source:** [.specs/changes/2026-08-05-bind_grant_type_at_token_endpoint.md](../../../changes/2026-08-05-bind_grant_type_at_token_endpoint.md)

**Implements:** source-spec implementation notes 1–2 and 6; the structural portion of [03-service-flows.md](../../../service/specs/03-service-flows.md) and [01-domain-model.md](../../../service/specs/01-domain-model.md).

**Depends on:** —

**Produces:** a non-default `ExchangeRequest` with exactly one typed `ExchangeCredential`, so `AppService::exchange` chooses code exchange vs direct ID-token validation by the credential variant rather than optional-field presence.

**Pointers:** `crates/core/src/service/exchange.rs:12-27,64-94`; all `ExchangeRequest` constructors in `crates/core/tests/{exchange,refresh,revoke,user_admin}.rs`; server construction in `crates/server/src/routes/token.rs`.

## Steps

- [ ] In `crates/core/src/service/exchange.rs`, define public `ExchangeCredential` variants `AuthorizationCode { code: String, redirect_uri: String }` and `IdTokenAssertion { id_token: String }`; replace the optional credential fields in `ExchangeRequest` with `credential: ExchangeCredential` and remove `#[derive(Default)]`.
- [ ] Refactor `AppService::exchange` to resolve the provider once and then exhaustively `match request.credential`: the code variant calls `exchange_code(&code, &redirect_uri)` then validates its returned ID token; the assertion variant calls `validate_id_token(&id_token)` directly. Remove unreachable core-level missing-field / “either code or id_token” errors.
- [ ] Preserve the existing request context (`ip_address`, `user_agent`, `device_id`) explicitly on each `ExchangeRequest`; do not make credentials defaultable. If reducing test-literal churn, introduce a separate defaultable context value only if it does not permit credential omission.
- [ ] Migrate every repository-owned constructor across `crates/core/tests/exchange.rs`, `refresh.rs`, `revoke.rs`, and `user_admin.rs`, plus the server route after task 02's contract lands. Use the authorization-code variant for existing code-flow tests and add/retain an ID-token variant test that proves direct assertion still succeeds.
- [ ] Add meaningful precondition/invariant assertions in touched core functions consistent with the guidelines without testing untrusted HTTP input in core; the route remains the validation boundary.
- [ ] Run targeted core tests covering both variants, existing-user/suspended/registration/audit behavior, and helpers that indirectly construct exchange requests.

## Definition of done

- [ ] `ExchangeRequest` cannot be default-constructed and cannot express code plus ID token or a missing credential.
- [ ] `AppService::exchange` has no branch based on `Option` field presence; it is exhaustive over `ExchangeCredential`.
- [ ] Authorization-code exchange still sends both required values to the provider; direct ID-token assertion still validates the supplied assertion directly.
- [ ] Every existing core test constructor compiles with a coherent credential variant; no stale `code`, `redirect_uri`, or `id_token` request fields remain.
- [ ] Negative space is structural: no service-level call site can construct a request mixing the two exchange credentials.
- [ ] No certificate file is created; the user explicitly prohibited done certificates.
