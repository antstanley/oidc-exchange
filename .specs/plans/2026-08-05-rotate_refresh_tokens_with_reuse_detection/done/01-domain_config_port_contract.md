# Task 01 — Domain, configuration, and port contract

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted (done certificates are forbidden)

**Implements:** source spec §§Type changes, SessionRepository, configuration, and implementation notes 1–3.
**Depends on:** —
**Produces:** typed session-family lifecycle model, refresh-resolution API/SR1–SR5 contract, config defaults/bounds, and `MockRepository` implementation surface.
**Pointers:** `crates/core/src/domain/{session,audit,token}.rs`; `crates/core/src/ports/repository.rs`; `crates/core/src/{config.rs,service/mod.rs}`; `config/default.toml`; `crates/test-utils/src/lib.rs`.

## Steps

- [x] Add `Session.family_id`, `generation`, and `rotated_at`; add `RetiredRefreshToken`; add `RefreshTokenReuse`; make all construction/call sites explicit.
- [x] Add `RefreshResolution`, `resolve_refresh_token`, atomic `rotate_refresh_token`, and count-returning `revoke_family` to the port with SR1–SR5 contract docs.
- [x] Add rotation/default-duration fields and cleanup interval; validate durations at config load, reject zero/non-positive values, and cap grace at named `MAX_REFRESH_ROTATION_GRACE_SECS` (60 seconds).
- [x] Implement the expanded port in `MockRepository` with a single mutex-protected state transition suitable for the shared harness.
- [x] Add focused domain/config/mock tests for defaults, invalid/over-cap configuration, resolution shapes, and atomic false-return non-mutation.

## Definition of done

- [x] The core compiles with no legacy `Session` literal or `SessionRepository` implementation missing a required family/port field or method.
- [x] Config defaults are `true`, `10s`, `24h`, and `1h`; invalid, zero, and grace-over-cap values fail startup validation with field-specific errors.
- [x] `MockRepository` proves a failed CAS makes no state mutation and supports deterministic resolution/revocation inspection.
- [x] New validation paths have positive and negative tests; limits use named constants; no raw token/hash is introduced into audit detail.
- [x] Done certificates remain intentionally absent.

## Completion notes

- Domain (`crates/core/src/domain/session.rs`): `Session` carries `family_id`/`generation`/`rotated_at` with `#[serde(default)]` sentinels for pre-rotation rows (empty-string family, generation 0, `rotated_at: None`), plus a legacy-row round-trip test; `RetiredRefreshToken` with `retention_deadline` centralizing `min(retired_at + retention, family expires_at)` (zero retention rejected); `RefreshResolution::{Live,Superseded,Retired,Unknown}`; `is_valid_family_id`/`new_family_id` enforcing `fam_` + 26 lowercase Crockford characters with malformed-shape negative tests (wrong prefix/case/length, SHA-256-hex-valued sid, non-Crockford letters). `AuditEventType::RefreshTokenReuse` added.
- Port (`crates/core/src/ports/repository.rs`): the trait documents the family model and the SR1–SR5 obligation table; `resolve_refresh_token`, `rotate_refresh_token`, and `revoke_family` each carry their obligation references and no-logging-of-raw-tokens rule on the trait doc.
- Config (`crates/core/src/config.rs`): `refresh_rotation` (default `true`), `refresh_rotation_grace` (`"10s"`), `refresh_reuse_retention` (`"24h"`), `session_repository.cleanup_interval` (`"1h"`) — all backed by named constants (`MAX_REFRESH_ROTATION_GRACE_SECS = 60`, `DEFAULT_*`); `AppConfig::validate` rejects unparseable, zero, and over-cap values with field-specific `ConfigError`s; positive/negative/boundary tests include a `config/default.toml` parity check.
- `MockRepository` (`crates/test-utils/src/lib.rs`): one mutex-guarded transition for the whole CAS (condition check → retirement record → swap, or nothing), `is_valid_family_id` preconditions on store/rotate/revoke, deterministic sorted inspection (`get_all_sessions`, `get_all_retired_tokens`), and a configurable reuse-retention constructor.
- Interim family-aware implementations landed on all five backends in the same task scope (`crates/adapters/src/{sqlite,postgres,lmdb,valkey,dynamo}`), each documented as interim pending tasks 03–06.
- No raw token or hash appears in any audit detail; no done certificate exists.
