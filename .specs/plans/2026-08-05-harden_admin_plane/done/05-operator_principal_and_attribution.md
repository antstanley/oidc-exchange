# 05 · Operator principal and attribution

**Status:** Done (commits `84189272`, `032e195a`, plus the server wiring; task 03's sibling primitives are vendored behind `VENDORED SEAM (task 03)` markers naming PR #24 for merge-time replacement)  
**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 5 and Type changes; [01-domain-model](../../../service/specs/01-domain-model.md), [03-service-flows](../../../service/specs/03-service-flows.md), [04-http-api](../../../service/specs/04-http-api.md), [07-telemetry-and-audit](../../../service/specs/07-telemetry-and-audit.md), and [canonical types](../../../service/specs/canonical-types.schema.json) targets  
**Depends on:** 03 · admin authentication throttle and audit; 04 · separate public and admin listeners  
**Produces:** `OperatorPrincipal` is authenticated by configured mechanisms, attached to internal requests, and recorded on successful admin mutations.

**Pointers:** `crates/core/src/domain`; `crates/core/src/domain/audit.rs`; `crates/core/src/service/user_admin.rs`; `crates/server/src/middleware/internal_auth.rs`; `crates/server/src/bootstrap.rs:254`; `crates/core/src/config.rs`; server/core audit tests.

## Work

- Define `OperatorAuthMechanism`/`OperatorPrincipal` and schema/prose updates; model shared-secret success as `{ id: "unattributed", mechanism: "shared_secret" }`, never as omitted identity.
- Introduce server-layer `OperatorAuthenticator` implementations for shared secret, operator token, and proxy-asserted mTLS subject, trying configured methods in order and inserting the principal request extension.
- Validate mechanism configuration, token key-manager availability in `role = admin`, mTLS header use only on the admin listener, required token issuer/audience/claim, and redaction of credential-bearing configuration.
- Thread principal through internal handlers and `user_admin` audit emission so successful mutations carry it; preserve exchange-plane null attribution.

## Definition of done

- [x] Shared-secret, valid operator-token, and valid mTLS paths each produce the correct mechanism/id; malformed, expired, wrong issuer/audience/claim, missing subject, and disabled mechanisms are rejected. (`crates/server/src/middleware/operator_auth.rs` unit tests cover the full per-mechanism matrix; `crates/server/tests/operator_auth.rs` drives the same through the production router, including expired/wrong-audience/wrong-issuer/missing-claim tokens verified against a real local key file.)
- [x] A shared-secret success records `unattributed`; every successful internal mutation records its principal, and exchange-plane events retain null operator attribution. (`attributed()` in `user_admin.rs` stamps the extension-extracted principal; core `user_admin` tests and the server E2E assert operator presence on mutations and `operator: None` on exchange-plane events.)
- [x] Operator-token configuration cannot start with the admin-role noop key manager; mTLS headers are not trusted on the public listener. (`validate_internal_api` refuses `adapter = "noop"` with `operator_token`, tested; the mtls mechanism is mounted only inside the admin router's auth gate — E2E proves a presented subject header changes nothing on the public plane.)
- [x] Domain schema/prose and public Rust documentation update with the new types; credentials remain redacted from debug/log/event output. (`OperatorPrincipal`/`OperatorAuthMechanism` carry full rustdoc incl. the reserved-id pairing rule; `InternalApiConfig`, `SharedSecretAuthenticator`, `OperatorTokenAuthenticator`, and `OperatorAuthGate` Debug impls redact secrets/values; rejection logs record fixed reasons only, never presented credentials. Canonical `.specs` schema/prose folding remains orchestrator-owned per the plan's recorded merge-plan decision.)
- [x] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed. (Full workspace gates green at filing time.)
- [x] Reviewable: an audit reader can distinguish attributed actions from the explicit shared-secret compatibility path. (Every mutation event carries `operator`; under the shared secret it is present and explicitly `{ id: "unattributed", mechanism: "shared_secret" }`.)
