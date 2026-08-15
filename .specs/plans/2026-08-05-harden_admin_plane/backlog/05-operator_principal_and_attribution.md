# 05 · Operator principal and attribution

**Status:** Blocked — waits for task 03 and its sibling prerequisite  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 5 and Type changes; [01-domain-model](../../../service/specs/01-domain-model.md), [03-service-flows](../../../service/specs/03-service-flows.md), [04-http-api](../../../service/specs/04-http-api.md), [07-telemetry-and-audit](../../../service/specs/07-telemetry-and-audit.md), and [canonical types](../../../service/specs/canonical-types.schema.json) targets  
**Depends on:** 03 · admin authentication throttle and audit; 04 · separate public and admin listeners  
**Produces:** `OperatorPrincipal` is authenticated by configured mechanisms, attached to internal requests, and recorded on successful admin mutations.

**Pointers:** `crates/core/src/domain`; `crates/core/src/domain/audit.rs`; `crates/core/src/service/user_admin.rs`; `crates/server/src/middleware/internal_auth.rs`; `crates/server/src/bootstrap.rs:254`; `crates/core/src/config.rs`; server/core audit tests.

## Work

- Define `OperatorAuthMechanism`/`OperatorPrincipal` and schema/prose updates; model shared-secret success as `{ id: "unattributed", mechanism: "shared_secret" }`, never as omitted identity.
- Introduce server-layer `OperatorAuthenticator` implementations for shared secret, operator token, and proxy-asserted mTLS subject, trying configured methods in order and inserting the principal request extension.
- Validate mechanism configuration, token key-manager availability in `role = admin`, mTLS header use only on the admin listener, required token issuer/audience/claim, and redaction of credential-bearing configuration.
- Thread principal through internal handlers and `user_admin` audit emission so successful mutations carry it; preserve exchange-plane null attribution.

## Definition of done

- [ ] Shared-secret, valid operator-token, and valid mTLS paths each produce the correct mechanism/id; malformed, expired, wrong issuer/audience/claim, missing subject, and disabled mechanisms are rejected.
- [ ] A shared-secret success records `unattributed`; every successful internal mutation records its principal, and exchange-plane events retain null operator attribution.
- [ ] Operator-token configuration cannot start with the admin-role noop key manager; mTLS headers are not trusted on the public listener.
- [ ] Domain schema/prose and public Rust documentation update with the new types; credentials remain redacted from debug/log/event output.
- [ ] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed.
- [ ] Reviewable: an audit reader can distinguish attributed actions from the explicit shared-secret compatibility path.
