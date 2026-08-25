# Task 03 — Prebuffer body bounds

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), [03-python.md §Implementation](../../../bindings/specs/03-python.md), source spec §Implementation notes 3
**Depends on:** 01
**Produces:** one configured request-body ceiling enforced by the router and Python WSGI/ASGI before either host buffers beyond it
**Pointers:** `crates/core/src/config.rs:149-160`, `crates/server/src/bootstrap.rs:352-362`, `bindings/python/python/oidc_exchange/_asgi.py:23-60`, `bindings/python/python/oidc_exchange/_wsgi.py:20-73`

## Steps

- [x] Add `server.max_request_body_bytes` with the source-specified default and propagate it to server bootstrap as `DefaultBodyLimit`.
- [x] Make ASGI accumulate into a capped `bytearray`, return 413 at the cap boundary, and avoid retaining excess chunks.
- [x] Parse WSGI `CONTENT_LENGTH` defensively, reject non-numeric values with 400, reject over-cap values with 413, and read `wsgi.input` in bounded chunks.
- [x] Add server, ASGI, and WSGI below/at/above-bound tests, including `sys.maxsize` and non-numeric length cases.

## Definition of done

- [x] The configured named body limit is the single source passed to the router and published for later hosts.
- [x] ASGI and WSGI reject one byte over the limit before unbounded allocation; native router rejects the same over-limit body with 413.
- [x] Invalid and oversized WSGI content lengths return 400/413 without `ValueError` or `OverflowError` escaping.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run boundary tests proving all three paths accept at/below and reject above the configured cap.

## Audit outcome

**Verdict:** Complete. Added the named 2 MiB config default and propagated configured values into `DefaultBodyLimit`; ASGI now uses capped `bytearray` accumulation and returns 413 before retaining an excess chunk; WSGI validates lengths, rejects malformed/huge values, and reads in named bounded chunks. Boundary tests cover below/at/above, non-numeric, and `sys.maxsize`.

**Evidence (2026-08-23):** focused ASGI/WSGI tests 12 passed / 0 failed; ruff format/check passed; pyright 0 errors / 0 warnings; `cargo fmt --all --check` passed; `cargo clippy --workspace -- -D warnings` passed; `cargo nextest run --workspace --no-fail-fast` passed 399 / failed 0 / skipped 27.
