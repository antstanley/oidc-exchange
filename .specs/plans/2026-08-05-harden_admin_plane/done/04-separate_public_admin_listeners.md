# 04 · Separate public and admin listeners

**Status:** Done (commit `32adeca2`, audited against this DoD)  
**Implements:** [source spec](../../../changes/merged/2026-08-05-harden_admin_plane.md) §"Implementation notes" step 4; [04-http-api](../../../service/specs/04-http-api.md) Routes/Service roles/Bootstrap targets; [06-configuration](../../../service/specs/06-configuration.md) target  
**Depends on:** 02 · exchange-only default and config validation  
**Produces:** Public and admin route sets bind separate sockets, share state/middleware correctly, and native/Lambda/FFI enforce the single-plane runtime rule.

**Pointers:** `crates/server/src/bootstrap.rs:246-363`; `crates/server/src/main.rs`; `crates/server/src/lambda.rs`; `crates/ffi`; `crates/server/src/routes/internal.rs`; `crates/core/src/config.rs`; server E2E/runtime tests.

## Work

- Split the current router assembly into public and admin builders sharing `AppState` and the documented middleware ordering; never merge `/internal/*` into the public router.
- Add validated admin listener host/port configuration, defaulting to loopback/admin port, and reject collisions with the public socket.
- Bind and join both native listeners under existing graceful shutdown semantics. Enforce the source-spec single-plane rule for Lambda and FFI and warn when native `role = all` is selected.
- Test every role/flag combination: `exchange`, `admin`, and `all`, with `internal_api.enabled` true/false, including health availability and public/admin route non-reachability on the wrong listener.

## Definition of done

- [x] No public router contains internal routes; `all` binds two distinct listeners, while `exchange` and `admin` bind only their plane. (`build_routers` composes `Routers { public, admin }`; `crates/server/tests/listeners.rs` proves `/internal/*` 404s on every public plane and `/token` 404s on every admin plane, including the collapsed single-plane runtime.)
- [x] Configuration rejects colliding host/port combinations and invalid listener setup before bind; defaults make the admin listener loopback-only. (`AppConfig::validate` → `listeners_collide`, `DEFAULT_INTERNAL_API_HOST:PORT = 127.0.0.1:8081`; an empty host fails closed via the bind-address assertion before any socket is opened.)
- [x] Native shutdown joins both listeners; Lambda/FFI reject or otherwise enforce the documented single-plane constraint without silently serving a merged surface. (`main.rs` binds both sockets before serving and joins them under one `ShutdownSignal` with the drain deadline; `Routers::single_plane` collapses role = "all" to the public plane with a startup warning naming the unmounted routes — same rule enforced in `lambda::run_lambda` callers and the FFI constructor.)
- [x] E2E tests prove `/token` is absent from the admin listener and `/internal/*` is absent from the public listener, with positive health checks for each bound role. (All six role × flag combinations are covered in `listeners.rs`, health asserted positive wherever a listener binds.)
- [x] Rust format/clippy/affected nextest tests pass; unrelated failures are recorded but not fixed. (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, full `cargo nextest run --workspace` green at filing time.)
- [x] Reviewable: deployment network policy can expose the admin plane without exposing exchange endpoints. (The planes share state/middleware, never route sets; the admin listener's route set is `/internal/*` + `/health` only.)
