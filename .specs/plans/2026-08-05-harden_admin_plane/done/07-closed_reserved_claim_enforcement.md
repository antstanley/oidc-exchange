# 07 · Closed reserved-claim enforcement

**Status:** Done on branch; external merge-order gate noted  
**Gate note:** implemented here ahead of sibling `2026-08-05-validate_revoke_token_claims` (commit `ovztwrxo`), which must still merge first — or its `sid`/`nbf` reservations be folded without overwrite — when this branch's spec changes are applied to the canonical prose. Every DoD item below is implemented and verified on this branch; only that merge-order step sits outside it.  
**Implements:** [source spec](../../../changes/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 7; [01-domain-model](../../../service/specs/01-domain-model.md) User target; [03-service-flows](../../../service/specs/03-service-flows.md) Custom claims target  
**Depends on:** — (external gate: `2026-08-05-validate_revoke_token_claims` must merge first, or its `sid` and `nbf` additions must be folded without overwrite)  
**Produces:** The closed 24-name protocol set is rejected before persistence, configuration acceptance, template resolution, and token flattening.

**Pointers:** `crates/core/src/service/claims.rs:8,116-119`; `crates/core/src/service/user_admin.rs`; `crates/core/src/config.rs`; `crates/core/tests/claims.rs`; `crates/core/tests/user_admin.rs`; revoke sibling change spec.

## Work

- Define the source-spec closed set as a single canonical constant/set and explicitly retain sibling-owned `sid` and `nbf`; do not reduce it to the current five names.
- Reject reserved keys from `admin_set_claims`, `admin_merge_claims`, `token.custom_claims`, and `{{ user.claims.KEY }}` resolution before data can be persisted or re-exported.
- Preserve accepted custom claims and existing non-reserved template behaviour; return the stable invalid-request domain error at write/config boundaries.
- Add table-driven tests naming every one of the 24 reserved names, including `sid`, plus paired allowed-name and already-persisted defensive-filter cases.

## Definition of done

- [x] The test table enumerates all 24 exact reserved names and fails if any is accepted at any required boundary.
- [x] Set/merge/config/template rejection occurs before persistence or signed-token output; normal custom claims still persist, resolve, and emit.
- [x] The implementation preserves the revoke sibling’s `sid`/`nbf` protections and does not duplicate its revoke validation work.
- [x] Canonical domain/service prose and schema are updated when the merged spec workflow applies the target changes.
- [x] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed.
- [x] Reviewable: no admin-controlled claim can collide with a protocol-owned field through either write or template paths.
