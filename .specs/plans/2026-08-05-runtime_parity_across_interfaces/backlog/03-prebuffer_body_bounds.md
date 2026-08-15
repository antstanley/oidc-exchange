# Task 03 — Prebuffer body bounds

**Plan:** [plan.md](../plan.md) · **Certificate:** intentionally omitted

**Implements:** [06-configuration.md §[server]](../../../service/specs/06-configuration.md), [04-http-api.md §Middleware stack](../../../service/specs/04-http-api.md), [03-python.md §Implementation](../../../bindings/specs/03-python.md), source spec §Implementation notes 3
**Depends on:** 01
**Produces:** one configured request-body ceiling enforced by the router and Python WSGI/ASGI before either host buffers beyond it
**Pointers:** `crates/core/src/config.rs:149-160`, `crates/server/src/bootstrap.rs:352-362`, `bindings/python/python/oidc_exchange/_asgi.py:23-60`, `bindings/python/python/oidc_exchange/_wsgi.py:20-73`

## Steps

- [ ] Add `server.max_request_body_bytes` with the source-specified default and propagate it to server bootstrap as `DefaultBodyLimit`.
- [ ] Make ASGI accumulate into a capped `bytearray`, return 413 at the cap boundary, and avoid retaining excess chunks.
- [ ] Parse WSGI `CONTENT_LENGTH` defensively, reject non-numeric values with 400, reject over-cap values with 413, and read `wsgi.input` in bounded chunks.
- [ ] Add server, ASGI, and WSGI below/at/above-bound tests, including `sys.maxsize` and non-numeric length cases.

## Definition of done

- [ ] The configured named body limit is the single source passed to the router and published for later hosts.
- [ ] ASGI and WSGI reject one byte over the limit before unbounded allocation; native router rejects the same over-limit body with 413.
- [ ] Invalid and oversized WSGI content lengths return 400/413 without `ValueError` or `OverflowError` escaping.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run boundary tests proving all three paths accept at/below and reject above the configured cap.
