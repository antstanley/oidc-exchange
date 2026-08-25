# Task 08 — Synchronize canonical specifications

**Status:** Done · **Plan:** [plan.md](../plan.md) · **Certificate:** forbidden and intentionally omitted

**Implements:** [source spec §Merge plan steps 1–4](../../../changes/merged/2026-08-05-eliminate_secret_leakage_in_logs_and_spans.md#merge-plan); canonical targets [`01-domain-model.md`](../../../service/specs/01-domain-model.md), [`02-ports-and-adapters.md`](../../../service/specs/02-ports-and-adapters.md), [`04-http-api.md`](../../../service/specs/04-http-api.md), [`05-provider-system.md`](../../../service/specs/05-provider-system.md), [`06-configuration.md`](../../../service/specs/06-configuration.md), [`07-telemetry-and-audit.md`](../../../service/specs/07-telemetry-and-audit.md), and [`08-persistence.md`](../../../service/specs/08-persistence.md)
**Depends on:** 07
**Produces:** all affected canonical documentation precisely describes demonstrated behavior and records no schema change; the proposed source change remains for merge-process ownership.
**Pointers:** source spec proposed-change blocks and type-change rationale; `.specs/service/specs/`; `.specs/README.md` merge-owned change table.

## Steps

- [x] Apply each source-spec canonical block after implementation is demonstrably complete; bump each affected canonical page date in the same change.
- [x] Add `Telemetry hygiene` to 07 with type-enforced formatting, public-description separation, and explicit session-instrumentation rules.
- [x] Replace—not append beside—the superseded per-type redacting-`Debug` decision in 06; update `[user_sync]` and `[internal_api]` wording for `Secret<String>`.
- [x] Document `Session`/repository signatures, request-ID bounds, provider error flow, Apple assertion type, and session-adapter span rules exactly as shipped.
- [x] Confirm `canonical-types.schema.json` and `schemas/datamodel.schema.json` remain untouched because serde/wire/store shape is unchanged; validate all internal Markdown links.

## Task-specific definition of done

- [x] Exactly the seven source-listed canonical pages are updated and their dates bumped; no unrelated canonical page changes are introduced.
- [x] Canonical prose matches tests/code, including named bounds and the no-schema-change rationale.
- [x] The source change remains `Proposed`; moving/stamping it and updating `.specs/README.md` are explicitly left to the merge process.
- [x] No certificate file is created; review plus link validation is the completion evidence.

**Evidence:** commits `docs(specs)` (this synchronization; 01/02/04/05 blocks landed with the task-07 work they described, completed here) — branch diff vs base touches exactly the seven listed pages under `.specs/service/specs/`, each with **Date: 2026-08-23**. Every applied claim was checked against shipped code before writing: `Session.refresh_token_hash: Secret<String>` + hand-written `Debug` printing `"<redacted>"` (`domain/session.rs`); repository token-hash parameters `&Secret<String>` (`ports/repository.rs`); `MAX_REQUEST_ID_LEN = 128`, charset predicate, silent rejection (`middleware/request_id.rs`); every-arm `client_description()` mapping with production `assert_ne!`/debug `assert_eq!` guards and warn/error logging under the request span (`server/src/error.rs`); single shared client (5 s connect / 10 s total, redirects disabled), JWKS 1 h TTL + 30 s refetch interval, RFC 8414 §3.3 issuer check, one bounded read per token-endpoint outcome with over-ceiling success failing closed (`adapters/src/shared/*`); Apple `generate_client_secret -> Result<Secret<String>>` and revocation through `read_bounded` + `error_detail` (`providers/apple.rs`); `[user_sync].secret: Secret<String>`, `[internal_api].shared_secret: Option<Secret<String>>` validated non-empty when served and compared via `subtle` constant time, internal routes mounted only for `admin`/`all` with the flag true (admin flag-off serves only `/health`) (`core/src/config.rs`, `server/src/bootstrap.rs`, `middleware/internal_auth.rs`); identical instrumentation on all five session adapters — `skip(self, session), fields(user_id = %session.user_id)` write path, `skip(self, token_hash), fields(token_hash)` lookup/revoke. No schema change: `jj diff --from mzqqsyul -- schemas/ .specs/service/specs/canonical-types.schema.json` is empty (serde-transparent wrap). Source change still `Proposed`; `.specs/README.md` and `.specs/changes/` untouched. Relative-link validation over all seven pages: 0 broken.
