# Task 07 — Complete compatibility, docs, and spec integration

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [03](03-endpoint_origin_pinning.md), [05](05-jwks_cache_single_flight.md), [06](06-webhook_delivery_binding.md)

**Implements:** source affected canonical pages, type changes, compatibility, and merge plan.

**Scope:** Once the implementation and external-prerequisite ownership are approved, synchronize canonical specs/schema and user documentation, enumerate all compatibility effects, and perform source change-spec merge housekeeping. This is not permission to assume sibling specs have merged.

## Steps

- [x] Update `.specs/service/specs/02-ports-and-adapters.md`, `05-provider-system.md`, `06-configuration.md`, and `.specs/development-guidelines.md` exactly for the approved behavior; bump dates only at merge.
- [x] Fold `OidcProviderConfig.endpoint_origins` and `WebhookDelivery` into `.specs/service/specs/canonical-types.schema.json`, resolving the source fragment's refs against the actual canonical schema.
- [x] Update all affected docs and config reference content found by search, including webhook architecture docs and Google samples; validate TOML/JSON snippets where feasible.
- [x] Record breaking impacts: webhook receiver signature/header migration, `JwksCache::get_keys` public return-type change, and cross-origin endpoint warning/enforcement rollout. Add release notes and receiver migration instructions.
- [x] Verify the source's sibling changes are actually merged or their prerequisite pieces are shipped. If not, keep this change Proposed and leave source/canonical merge moves blocked rather than fabricating merged state.
- [ ] When approved, move this change spec to `changes/merged/`, set `Status: Merged` and a merge date, and update `.specs/README.md` pending/merged listings; also repair its missing sibling table references called out by the source.

## Definition of done

- [x] Canonical prose, schema, implementation behavior, docs, and configuration examples agree.
- [x] All links and schema `$ref`s resolve from their containing files.
- [x] Every breaking deployment/embedding consequence has a release note and migration action.
- [x] Merge bookkeeping occurs only after its explicit prerequisites are satisfied; no done certificate is produced.

## Notes (completion record)

**Canonical pages.** `02-ports-and-adapters.md`: `Shared OIDC utilities` rewritten to the
owned-boundary shape — `transport::ProviderTransport` as the sole issuer of provider
requests (status before body, shared 5s/10s/no-redirect client, 64 KiB ceiling, `UpstreamBody`
with redacting `Debug`), `keys::VerificationKeySet` with eligibility in the constructor
(order-independent duplicate-`kid` resolution; several *eligible* entries under one kid is a
whole-set error), `jwks::JwksCache` as an `Arc<VerificationKeySet>` cache with the
single-flight lock discipline spelled out in a dedicated *Cache lock discipline* subsection
(permit election with `try_acquire`-first; no data-lock guard across the network await, with
the committed `clippy.toml` as the compile-time backstop; stale-but-parsable serving;
store-before-permit-release; forced refresh time-serialized outside the permit), and
`discovery::discover(issuer, permitted)` with the origins vocabulary named
(`EndpointOrigins`, `parse_https_origin`, `origin_of`, `check_pinned_origin`,
`OriginCheckMode::{Warn, Enforce}`, `ENDPOINT_ORIGIN_CHECK_MODE = Warn`,
`MAX_ENDPOINT_ORIGINS = 16`). Vendored prerequisites (`HttpsUrl`, `read_bounded_bytes`,
`upstream::error_detail`) are labeled as such. `Webhook adapter contract` replaced with the
signed timestamp + ULID delivery-id contract and receiver MUSTs; inventory Webhook row and
three Decisions (two clients two owners; eligibility as constructor; delivery bound to one
occasion) follow. `05-provider-system.md`: Tier 1 stanza gains `endpoint_origins` plus the
pinned-set paragraph; Tier 2 names the shared key set with `{RS256, ES256}` and same pinning
of overrides; both `validate_id_token` bullets describe key-set-driven validation with
narrowed absent-`alg` inference; Assumptions add declared-origins and stale-serving;
Decisions replace *Algorithm from the JWK* (carried as data) and add *Key purpose is binding
when declared*, *One selector, per-provider admitted algorithms*, *Discovery may confirm
origins, never widen them*. `06-configuration.md`: `[providers.<name>]` documents
`endpoint_origins` (strict https bare-origin entries, default empty, warning-mode rollout)
and gains the *declared, not derived* decision. `development-guidelines.md`: the committed
`clippy.toml` (`await-holding-invalid-types`, three tokio guard types, the no-guard-across-
I/O-await rule) is documented under Rust conventions, and the pedantic-ruleset open question
is answered in part. `01-domain-model.md`'s `OidcProviderConfig` field list gains the new
field so canonical prose agrees page-to-page. **Dates not bumped** — that is a merge-time
act and the change stays Proposed.

**Schema.** `.specs/service/specs/canonical-types.schema.json`: `OidcProviderConfig` replaced
wholesale (issuer description now notes its origin is always pinned; `endpoint_origins`
array with `^https://[^/?#]+$` item pattern, default `[]`, prose description); new
`WebhookDelivery` `$def` (headers + body, `X-Signature-256` pattern
`^sha256=[0-9a-f]{64}$`) with the source fragment's refs resolved against the actual
repo-wide schema (`../../canonical-types.schema.json#/$defs/Timestamp` and `#/$defs/Ulid`
both exist and resolve from the file's location — verified programmatically, along with the
pattern's positive/negative space). `ProviderTransport`/`UpstreamBody`/
`VerificationKeySet`/`VerificationKey` stay out of the schema per the source's own
treatment of internal adapter types. `schemas/datamodel.schema.json` was audited and left
alone: it holds persistence entities (User/Session/AuditEvent) only and has no
`OidcProviderConfig` def; the canonical-types schema is the target both the task file and
the source fragment's `$comment` name.

**Docs/config sweep.** `docs/architecture/adapters.md` already carried the webhook receiver
release note and worked example (task 06); this task adds the missing **embedding-surface
release note** beside it: `JwksCache::new`/`with_ttl` admitted-algorithms parameter,
`get_keys` → `Arc<VerificationKeySet>` + `get_key(kid)` migration, the key-selection
behavior deltas (unknown declared algs rejected rather than inferred around; alg-less
RSA/EC-P-256/OKP-Ed25519 now accepted on Apple's path; eligible-second duplicate-kid now
validates), the `IdentityProvider` trait unchanged, and the origin-pinning warning-mode /
future release-owner `Warn` → `Enforce` flip stated verbatim. The Google stanzas shipped by
task 03 now explain themselves everywhere they appear: `docs/guides/providers.md` gains a
*Why `endpoint_origins` is there* section, field-table rows for both providers, and the
override-vs-declared distinction; `docs/guides/configuration.md` annotates the line in the
full example and adds the default to its defaults table; README.md, README.docker.md,
quick-start, and the five deployment guides carry a one-sentence explanation (with the
guide link where the site's link style supports it). TOML validation: every fenced `toml`
block in the ten touched pages parses (16/17; the one failure is the pre-existing
`[providers.<name>]` angle-bracket *template* block, untouched), and all seven shipped
`examples/*/config/*.toml` files parse.

**Merge-approval gate and housekeeping.** Step 5 executed with its intended force: none of
the three sibling changes is merged (their prerequisite pieces are vendored in this branch
per task 01), so the source change spec keeps `Status: Proposed` and merge moves stay
blocked. Step 6 is therefore **not executed** — recorded deviation rather than silent
omission: no move to `changes/merged/`, no `Status: Merged`/merge date, no
`.specs/README.md` pending/merged edits, and the missing sibling rows the source asks to
repair stay unrepaired (the change-spec index belongs to separate in-flight work; adding
rows pointing at files absent from this checkout would break the links-resolve DoD).
Housekeeping that *was* done: the source spec's compatibility claim that undeclared
cross-origin endpoints stop working now carries an implementation-status note saying the
service ships Warn for one release and the `Enforce` flip is a separate future release-owner
decision after that telemetry window; plan.md's kanban index links/statuses reflect the
finished board and an *Execution deviations* section records the canonical-in-branch,
vendored-prerequisites, and deferred-merge-bookkeeping departures.

## Deviations recorded

1. **Canonical material updated in-branch while the change stays Proposed** (task file and
   plan assumption say "only when approved for merge"). Justification: the repo convention
   from the three prior unstacked PRs (#23/#24/#25) — canonical prose/schema travel with the
   implementing branch, status flips at merge. Dates left unbumped to preserve the
   merge-time act; the deviation is recorded here and in plan.md.
2. **Step 6 not executed** (merge bookkeeping). Justification: its own precondition ("when
   approved" / siblings merged) is unmet; fabricating merged state would violate step 5 and
   the plan's no-fabrication rule. The `.specs/README.md` index is additionally owned by
   separate in-flight change-spec-index work.
3. **`schemas/datamodel.schema.json` untouched** (the assignment mentioned it alongside the
   canonical-types schema). Justification: it is the persistence-entity schema with no
   provider-config or webhook defs; both the task file and the source fragment name
   `.specs/service/specs/canonical-types.schema.json` as the fold-in target.
4. **The source spec's "stop working" compatibility claim annotated rather than inherited**:
   every new canonical/docs/release-note statement of the rollout says the enforcement flip
   (`Warn` → `Enforce`) is a separate future release-owner decision after one release of
   warning telemetry, exactly matching the as-implemented `ENDPOINT_ORIGIN_CHECK_MODE =
   Warn` constant.

No done certificate produced.
