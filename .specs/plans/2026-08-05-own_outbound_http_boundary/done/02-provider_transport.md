# Task 02 — Add bounded provider transport

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md)

**Implements:** source `Shared OIDC utilities`; implementation notes 3–4; success-body ceiling tests.

**Scope:** Add `shared::transport::ProviderTransport` and its `UpstreamBody` abstraction around the process-wide provider client. Migrate discovery, JWKS, token exchange, OIDC revocation, and Apple revocation—five call sites—so no adapter directly issues provider HTTP. Integrate the externally confirmed bounded-read and safe error-detail helpers without reimplementing sibling-owned behavior.

## Steps

- [x] Export the transport from `crates/adapters/src/shared/mod.rs`; use only the shared provider client with its existing 5s connect, 10s total, and no-redirect policy.
- [x] Implement typed `get_json` and form-post operations that inspect status before reading a body; success parsing consumes the bounded body, while non-success uses the confirmed error-detail path.
- [x] Migrate `shared/discovery.rs`, `shared/jwks.rs`, `shared/token_endpoint.rs`, `oidc/mod.rs` revocation, and `providers/src/apple.rs` revocation. Remove direct provider `reqwest` issuance from those call sites.
- [x] Route discovery and JWKS success parsing through the shared ceiling; retain token endpoint's single already-bounded body read rather than reading twice.
- [x] Add wiremock tests for over-limit bodies with both honest `Content-Length` and chunked transfer, distinctive cap errors, and no cache population after JWKS failure.

## Definition of done

- [x] Repository scan finds no direct provider `reqwest` request outside `ProviderTransport`; webhook keeps its independently owned client.
- [x] Both discovery and JWKS reject oversized success responses before JSON materialization; status is evaluated before any body read.
- [x] A non-2xx provider response uses bounded/safe detail handling and remains an error.
- [x] All five call sites and their existing success/error tests compile and pass; no done certificate is produced.

## Notes (completion record)

**Shape.** `crates/adapters/src/shared/transport.rs`: stateless
`ProviderTransport` with `get_json(provider, url)` and
`post_form(provider, url, form)` returning `Result<UpstreamBody>`. `collect()`
is the one place the ordering exists: status inspected first, then the body read
once through the vendored `read_bounded_bytes` ceiling. `UpstreamBody` carries
the pre-read status plus the bounded bytes and exposes `status()`, `is_success()`,
`parsed::<T>(provider)` (non-success → error via the safe detail path),
`error_into(provider)`, and `bytes()` (borrow of the single read — the token
endpoint branches on status and parses from the same buffer without a second
read). Its manual `Debug` redacts body bytes so response content can never reach
a panic or log line.

**Distinctive cap error.** Over-limit reads fail inside the transport as
`ProviderError { detail: "response from <endpoint> exceeded the 65536-byte
upstream limit" }` — naming both endpoint and limit, kept textually distinct
from parse failures. Honest oversized `Content-Length` is refused before any
byte is buffered; chunked bodies hit the mid-stream running-total bound.

**Migrations.** Discovery now checks status via the transport (it previously had
no status check at all — the transport became the sole caller, which is when the
source spec transfers that duty); JWKS `fetch_keys`, token exchange, and both
revocation sites route through the transport. Both revocation sites previously
embedded raw failure bodies in error details; they now get the sanitized
detail. The shared client was downgraded to `pub(crate)` so the compiler, not a
repository scan, enforces "nothing else issues provider requests" — webhook's
own operator-timeout client is untouched by design.

**Tests added.** Transport: success parse; status-before-body (500 + non-JSON
body reported by status, body never echoed); OAuth tokens surfaced on 400;
over-limit honest Content-Length naming limit + endpoint and staying distinct
from parse errors; over-limit chunked via raw TCP hitting the same cap;
form-encoded POST single-read proof; unreachable provider → ProviderError.
Discovery: over-limit document rejected pre-parse; 404 handled with safe
detail. JWKS: honest-CL cap error + cache unpopulated; chunked cap + cache
unpopulated; sub-ceiling fetch still works. Token endpoint: non-2xx never
echoes non-protocol bodies; over-limit success body rejected. All five call
sites' existing success/error suites pass unchanged.

**Deviation recorded.** The transport takes validated URL strings rather than
the vendored `HttpsUrl` in this wave: every existing wiremock test serves plain
`http://127.0.0.1` origins, and enforcing https at the transport today would
break them all while duplicating the fail-closed sibling's scheme check ahead
of task 03's config wiring. `HttpsUrl` is contract-established with tests in
task 01; adopting it at config/discovery boundaries belongs to task 03 (next
wave), after which the transport internals can switch to consuming it.

No done certificate produced.
