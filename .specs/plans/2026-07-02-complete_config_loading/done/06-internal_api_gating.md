# Task 06 — Internal API gating

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-internal_api_gating-certificate.md](06-internal_api_gating-certificate.md)

**Implements:** [04-http-api.md](../../../service/specs/04-http-api.md) → Routes → Internal (mounted only when `internal_api.enabled = true`), Service roles (the `admin`/`all` `enabled` condition), Middleware stack (internal-auth paragraph); [06-configuration.md](../../../service/specs/06-configuration.md) → Sections → `[internal_api]` (`enabled` gate semantics)
**Depends on:** 02
**Produces:** a router that mounts the internal routes only when `internal_api.enabled = true` and the role is `admin` or `all`; with the flag false a `role = "admin"` instance builds a router containing only `/health` (not a startup error), and `internal_auth_layer` no longer treats an empty secret as configured.
**Pointers:** `crates/server/src/bootstrap.rs:119-132` (`build_router` — currently mounts `internal_routes` on `role` alone, ignoring `internal_api.enabled`); `crates/server/src/middleware/internal_auth.rs:19-28` (treats `Some("")` as a configured secret)

## Steps

- [x] Gate the `internal_routes` merge in `build_router` on `internal_api.enabled == true` in addition to the role check; when the flag is false, mount no internal routes regardless of role.
- [x] Ensure a `role = "admin"` (or `all`) instance with `enabled == false` still builds a router serving `/health`, so the instance stays observable and startup does not error.
- [x] Harden `internal_auth_layer` so an empty secret (`Some("")`) is not accepted as configured (defence in depth — task 02's `validate()` already rejects a served empty secret at startup).
- [x] Add tests: `enabled = true` + `admin`/`all` mounts `/internal/*` behind Bearer auth; `enabled = false` + `admin` mounts only `/health`; `enabled = false` + `all` mounts the public routes and `/health` but no `/internal/*`.

## Definition of done

- [x] With `internal_api.enabled = false`, no `/internal/*` route is mounted for any role; a `role = "admin"` instance serves only `/health`.
- [x] With `internal_api.enabled = true` and role `admin`/`all`, `/internal/*` is mounted and sits behind the constant-time Bearer check; a missing/empty secret is rejected at startup (task 02), never discovered at request time.
- [x] Negative-space tests cover the flag-off (no internal routes) and empty-secret paths; any new bound is a named constant.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [x] Reviewable: build a router for `role = "admin"` with `enabled = false` and assert a request to an `/internal/*` path returns 404 while `/health` responds; then with `enabled = true` and a secret, assert `/internal/*` is reachable with the correct Bearer token and 401 without.
