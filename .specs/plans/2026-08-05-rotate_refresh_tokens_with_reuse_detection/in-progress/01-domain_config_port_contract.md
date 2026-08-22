# Task 01 — Domain, configuration, and port contract

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §§Type changes, SessionRepository, configuration, and implementation notes 1–3.
**Depends on:** —
**Produces:** typed session-family lifecycle model, refresh-resolution API/SR1–SR5 contract, config defaults/bounds, and `MockRepository` implementation surface.
**Pointers:** `crates/core/src/domain/{session,audit,token}.rs`; `crates/core/src/ports/repository.rs`; `crates/core/src/{config.rs,service/mod.rs}`; `config/default.toml`; `crates/test-utils/src/lib.rs`.

## Steps

- [ ] Add `Session.family_id`, `generation`, and `rotated_at`; add `RetiredRefreshToken`; add `RefreshTokenReuse`; make all construction/call sites explicit.
- [ ] Add `RefreshResolution`, `resolve_refresh_token`, atomic `rotate_refresh_token`, and count-returning `revoke_family` to the port with SR1–SR5 contract docs.
- [ ] Add rotation/default-duration fields and cleanup interval; validate durations at config load, reject zero/non-positive values, and cap grace at named `MAX_REFRESH_ROTATION_GRACE_SECS` (60 seconds).
- [ ] Implement the expanded port in `MockRepository` with a single mutex-protected state transition suitable for the shared harness.
- [ ] Add focused domain/config/mock tests for defaults, invalid/over-cap configuration, resolution shapes, and atomic false-return non-mutation.

## Definition of done

- [ ] The core compiles with no legacy `Session` literal or `SessionRepository` implementation missing a required family/port field or method.
- [ ] Config defaults are `true`, `10s`, `24h`, and `1h`; invalid, zero, and grace-over-cap values fail startup validation with field-specific errors.
- [ ] `MockRepository` proves a failed CAS makes no state mutation and supports deterministic resolution/revocation inspection.
- [ ] New validation paths have positive and negative tests; limits use named constants; no raw token/hash is introduced into audit detail.
- [ ] Done certificates remain intentionally absent.
