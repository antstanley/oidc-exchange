# Task 08 — Run full validation and release-readiness review

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** [01](01-prerequisite_contracts_and_guard_lint.md), [02](02-provider_transport.md), [03](03-endpoint_origin_pinning.md), [04](04-verification_key_set.md), [05](05-jwks_cache_single_flight.md), [06](06-webhook_delivery_binding.md), [07](07-compatibility_docs_and_spec_integration.md)

**Implements:** repository definition of done and source compatibility/merge gates.

**Scope:** Execute Rust quality gates and review the complete boundary invariants, source coverage, documentation, configuration, and release decisions. This task reports evidence in review output/task status and intentionally creates no done certificate.

## Steps

- [x] Run `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace`; resolve failures without bypasses.
- [x] Run focused adapter/provider wiremock tests for body limits, status ordering, origin pinning, C12 corpus, cache concurrency, and webhook signing/retry identity.
- [x] Audit all outbound provider HTTP call sites and JWK conversion paths by repository search; verify webhook remains the only separate outbound owner.
- [x] Recheck source coverage table, task links, DAG ordering, canonical/schema refs, TOML/JSON examples, and status placement in the kanban index.
- [x] Confirm warning-to-enforcement rollout and webhook receiver migration/release notes have an owner and a deployment window; block release if either is unresolved.

## Definition of done

- [x] All required Rust gates are clean with actual command output recorded in review context.
- [x] Every source test requirement has a passing positive and negative test or an explicitly approved, tracked exception.
- [x] The task graph remains acyclic and all required task links resolve.
- [x] No done certificate is produced.

## Notes (validation evidence)

This file is evidence, not a certificate. Commands run at the repo root of this workspace;
all output lines quoted verbatim from the runs below.

### Rust quality gates

- `cargo fmt --all --check` → exit 0, no output ("FMT CLEAN").
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0; zero warning/error
  diagnostics emitted (run twice: once cold over the finished tree, once warm). The
  committed `clippy.toml` (`await-holding-invalid-types` with the three tokio guard types)
  stays active — no `#[allow]`, no suppression anywhere in the diff.
- `cargo nextest run --workspace --no-fail-fast` →
  **`Summary [ 14.322s] 464 tests run: 464 passed, 27 skipped`** — identical counts to the
  inherited pre-07/08 state (tasks 07/08 changed docs/specs only, no production code), and
  the 27 skips are the pre-existing external-credential/ignored suites, unchanged.

### Focused adapter/provider wiremock suites

`cargo nextest run -p oidc-exchange-adapters -p oidc-exchange-providers --no-fail-fast` →
**`Summary [ 13.802s] 210 tests run: 210 passed, 27 skipped`**. Named coverage per source
test requirement:

- *Byte ceiling / body limits*: `shared::transport::tests::oversized_success_body_is_a_distinct_cap_error_naming_limit_and_endpoint`,
  `oversized_chunked_success_body_hits_the_same_cap_error`,
  `shared::http::tests::bounded_read_rejects_oversized_body_with_honest_content_length_without_reading_it`,
  `bounded_read_rejects_oversized_chunked_body_midstream`,
  `shared::discovery::tests::discover_rejects_oversized_success_document_before_parsing`,
  `token_endpoint::exchange_code_rejects_oversized_success_body`; negative-space
  sub-ceiling fetches still succeed (`first_call_fetches_and_builds_a_key_set`).
- *Status ordering*: `transport::tests::get_json_parses_a_success_shape`,
  `non_success_response_surfaces_oauth_error_tokens`,
  `discovery_rejects_non_success_status_with_safe_detail`,
  `jwks::non_2xx_response_is_error_and_leaves_cache_unpopulated`,
  `exchange_code_surfaces_oauth_error_on_non_2xx`,
  `exchange_code_non_2xx_never_echoes_a_non_protocol_body`.
- *Origin pinning*: unit — `parse_https_origin_accepts_bare_https_origins_and_normalizes_them`,
  `parse_https_origin_rejects_paths_queries_fragments_and_other_schemes`,
  `warn_mode_accepts_an_undeclared_cross_origin_endpoint_without_error`,
  `enforce_mode_rejects_an_undeclared_endpoint_naming_endpoint_origin_and_set`,
  `declared_members_pass_under_enforcement_too`, set-membership/cap cases; integration —
  `discovery_serves_warning_mode_for_an_undeclared_cross_origin_endpoint`,
  `discover_accepts_a_declared_cross_origin_endpoint`,
  `oidc::google_multi_origin_discovery_shape_passes_when_all_origins_are_declared`
  (Google's real two-origin shape), `configured_cross_origin_jwks_admits_discovered_endpoints_on_that_origin`,
  `invalid_endpoint_origins_entries_are_rejected_at_the_adapter_boundary`.
- *C12 corpus* (`providers::cross_provider_corpus`): `both_validators_agree_on_selection_for_every_corpus_case`
  (twelve source-listed fixtures, equal dispositions on both paths — 0 disagreements vs the
  committed pre-consolidation baseline's 6), plus `rsa_sig_key_verifies_on_the_oidc_path`,
  `rsa_sig_key_verifies_on_the_apple_path`, `ec_sig_key_verifies_on_the_apple_path`
  (the `use: "sig"` non-regression cases).
- *Cache concurrency*: `expired_ttl_racers_collapse_into_exactly_one_fetch_with_stale_serving`,
  `cold_cache_racers_collapse_into_exactly_one_fetch`,
  `failed_refill_serves_stale_to_waiters_and_costs_one_fetch`,
  `failed_refill_on_a_cold_cache_serializes_one_attempt_per_waiter`,
  `concurrent_unknown_kid_lookups_force_exactly_one_refetch`,
  `kid_matching_only_an_ineligible_entry_is_a_miss_that_forces_one_refetch`,
  `forced_refetch_within_interval_makes_no_second_request`,
  `failing_upstream_still_rate_limits_forced_refetch`; benchmark pinned by
  `large_key_set_cache_hits_are_sub_millisecond` (0.8–0.9 µs/hit mean on a 96-key set,
  per task 05's record).
- *Webhook signing/retry identity*: `webhook::tests::retry_burst_reuses_the_same_id_timestamp_and_signature`
  (byte-identical bodies + one id/timestamp/signature across attempts),
  `independent_deliveries_carry_distinct_delivery_ids`,
  `body_only_signature_from_the_old_scheme_never_validates`,
  `mutating_any_signed_field_invalidates_the_signature`,
  `test_successful_delivery_with_correct_hmac`, `test_retry_on_5xx`, `test_4xx_no_retry`,
  no-redirect and capped-backoff coverage retained.

### Repository-search audits

- **Outbound provider HTTP**: `.send()` over production code exists in exactly two places —
  `shared/transport.rs:68` (`get_json`) and `shared/transport.rs:88` (`post_form`), both
  inside `ProviderTransport`. Its five call sites:
  `shared/discovery.rs:45`, `shared/jwks.rs:296`, `shared/token_endpoint.rs:36`,
  `oidc/mod.rs:213`, `providers/apple.rs:382`. The shared client is `pub(crate)`
  (`shared/http.rs:29`), so the compiler rather than convention enforces sole ownership.
  Remaining `.send()` hits are AWS SDK clients outside this boundary (dynamo ×35, kms ×2,
  sqs_audit ×1) or test modules.
- **Webhook remains the only separate outbound owner**: the only other production
  `reqwest::Client` construction is `adapters/webhook/mod.rs:44` (operator-configured
  timeout), matching the two-clients-two-owners decision; it deliberately does not use
  `ProviderTransport`.
- **JWK conversion paths**: `DecodingKey::from_jwk` appears in production only at
  `shared/keys.rs:210` (inside the key-set constructor path); the one other hit
  (`providers/apple.rs:532`) is inside `#[cfg(test)] mod tests` verifying Apple's own
  client-secret JWT. No production code re-derives verification algorithms outside the key
  set (`Validation::new(key.algorithm())` from resolved keys in both providers).
- **Vendored prerequisite markers intact**: `VENDORED PREREQUISITE` present at
  `shared/http.rs:45` (HttpsUrl), `shared/http.rs:141` (`read_bounded_bytes` /
  `MAX_UPSTREAM_BODY_BYTES = 65536`), `shared/upstream.rs:3` (`error_detail`).

### Coverage, links, DAG, schema refs, examples, kanban placement

- **Source coverage table**: every row maps to completed tasks (01–08, 04a folded into 04);
  nothing uncovered, nothing absorbed from siblings (vendored pieces marked).
- **Task links**: scripted check over the plan directory + touched canonical pages found
  every relative markdown link resolving — after this file moved to `done/`, including all
  seven sibling-task links in this file's header and plan.md's nine index rows. Known
  non-resolving references are confined to the *source change spec's own body* and are
  pre-existing/external: three sibling change specs absent from this unstacked checkout by
  design (their prerequisites are vendored here) and the sealed `.security/…` scan bundle,
  which lives outside the repo. None of these were introduced or repairable by tasks
  07/08; recording replaces fabricating.
- **DAG ordering**: dependency column and Mermaid graph agree; every edge runs lower→higher
  (01→02, 01→03, 02→03, 01→04, 02→05, 04→05, 03/05/06→07, all→08); acyclic; all links in
  both resolve.
- **Canonical/schema refs**: programmatic validation of
  `.specs/service/specs/canonical-types.schema.json` — parses as JSON; all `$ref`s resolve
  (`#/$defs/*` locally; `../../canonical-types.schema.json#/$defs/Timestamp|Ulid|Id|
  NonEmptyString` against the existing repo-wide defs); `endpoint_origins` pattern
  `^https://[^/?#]+$` exercised positive (bare https origins incl. explicit port) and
  negative (http scheme, path/query/fragment, bare host).
- **TOML/JSON examples**: every fenced `toml` block in the ten doc pages touched by task 07
  parses (16/17; the single failure is the pre-existing `[providers.<name>]` angle-bracket
  template block, untouched by this plan); all seven shipped `examples/*/config/*.toml`
  parse.
- **Kanban placement**: `backlog/` empty; `done/` holds 01–08 with completion records;
  `in-progress/` empty at close; plan.md index statuses read Done with `done/` links;
  no done certificate exists anywhere in the plan.

### Release-decision confirmation (owners and windows)

- **Warning-to-enforcement rollout**: owner — the release owner, named as such in code
  (`ENDPOINT_ORIGIN_CHECK_MODE` doc comment), canonical pages, the embedding-surface
  release note, and the source spec's implementation-status annotation. Window — after one
  release of warning telemetry, as its own reviewed commit. This release ships `Warn`, so
  nothing about the flip blocks this release: the decision is deliberately future work with
  an assigned owner and an observable condition, not an unresolved question.
- **Webhook receiver migration**: release note + migration instructions shipped in
  `docs/architecture/adapters.md` (before/after signature formats, the four receiver MUSTs,
  worked Node.js verifier). Owner — the deploying operator; window — the receiver deploy
  must accompany this upgrade, stated in the note, with the blast radius bounded
  (`user_sync.enabled` defaults to `false`; no shipped config enables it; pre-1.0).

Both confirmations hold → **no release blocker raised.**

### Open questions resolved for this change (per plan)

- Stale-serving during refill: implemented **yes** (stale-but-parsable served; missing kids
  fail closed through the rate-limited forced refetch) — tested above.
- JWKS byte ceiling: **shared** with the upstream ceiling at `MAX_UPSTREAM_BODY_BYTES =
  64 KiB`; no separate knob shipped. The distinctive cap error is the alerting hook; if a
  maintainer later wants a knob it is a follow-up change, not silent drift.
- Cache-hit performance: measured **0.8–0.9 µs/hit** (96-key set, 2 000 hits, debug build)
  against the sub-millisecond target — recorded in task 05 and asserted by
  `large_key_set_cache_hits_are_sub_millisecond`.

No done certificate produced.
