# 01 · Configuration and FFI canonical blocks

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Certificate:** intentionally omitted

**Implements:** [2026-07-01-complete_config_loading.md](../../../changes/merged/2026-07-01-complete_config_loading.md) — all `Proposed changes` blocks targeting [01-ffi-core.md](../../../bindings/specs/01-ffi-core.md), [04-http-api.md](../../../service/specs/04-http-api.md), and [06-configuration.md](../../../service/specs/06-configuration.md).
**Depends on:** —
**Produces:** canonical documentation for config load/validation parity, internal-API mounting and auth semantics, and FFI construction-time validation.

## Steps

- [x] Verify `01-ffi-core.md` Responsibilities states that `new` and `from_file` use the server load-time validation and reject invalid config as `FfiError` at construction.
- [x] Verify `06-configuration.md` contains the expanded loading-order semantics, `Validation at load`, and the `[internal_api]` enabled/non-empty-secret semantics.
- [x] Verify `04-http-api.md` contains internal-route gating in the Internal section, roles table, middleware/auth text, and bootstrap config-validation step.
- [x] Verify only the three owned pages are touched for this task and their metadata dates are `2026-08-05`.

## Definition of done

- [x] All eight complete-config source blocks (loading order, validation-at-load, internal API, internal routes, roles, internal auth, bootstrap, FFI responsibilities) are represented with equivalent semantics.
- [x] Negative-space documentation is preserved: unset placeholders fail startup; invalid role/TTLs/allowlist fail validation; no internal routes mount when disabled; a missing/empty served-internal secret fails startup.
- [x] Every local and source link resolves, including the FFI link to `06-configuration.md` and the plan/source links above.
- [x] No code, schema, change-spec, README-index, certificate, or unrelated canonical-page changes are introduced.

## Execution evidence

- The canonical prose was already present in the PR's initial documentation diff; no additional canonical-page edit was required.
- An independent combined review found the source-block semantics and negative cases complete. Local Markdown targets and the bounded PR scope were checked during integration.
