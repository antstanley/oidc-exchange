# Release notes

## 0.3.0

- **Node:** `handleRequest` now returns a `Promise`; callers must `await` it. Requests use separate `rawPath` and `query` fields and ordered header entries. `handleRequestSync` remains deprecated for one major cycle.
- **Python:** direct callers now pass `raw_path` and `query` separately and use ordered `(name, value)` header pairs. ASGI/WSGI applications migrate automatically and enforce the published 2 MiB body cap before buffering.
- **Lambda:** base-path handling moved from event adapters into the Rust normaliser. API Gateway and ALB adapters preserve the rawest event representation available, and sibling paths such as `/authorize` are no longer stripped as `/auth` children.

Deprecated synchronous entry points are intentionally retained in 0.3 and scheduled for removal in the following major release cycle; removal is deferred from this change.
