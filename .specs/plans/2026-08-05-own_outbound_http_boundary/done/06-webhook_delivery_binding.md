# Task 06 — Bind webhook deliveries and document receivers

**Plan:** [plan.md](../plan.md) · **Status:** Done · **Depends on:** —

**Implements:** source webhook contract and implementation note 9; webhook test list; compatibility section.

**Scope:** Bind the webhook HMAC to an RFC3339 timestamp and ULID delivery ID plus raw body, mint all delivery identity material once outside retries, and publish the required receiver verification/deduplication protocol. Webhook owns its own configured-timeout `reqwest::Client`; it does not use `ProviderTransport`.

## Steps

- [x] Change `compute_hmac_hex` to MAC `timestamp`, `.`, `delivery_id`, `.`, raw body; prefix emitted value with `sha256=`.
- [x] Mint `sent_at` and `ulid::Ulid` once per logical delivery outside the retry loop; send `X-Webhook-Timestamp`, `X-Webhook-Delivery-Id`, and `X-Signature-256` on every attempt.
- [x] Preserve JSON body timestamp for existing parsers while requiring receiver verification before parse, ±5-minute timestamp acceptance, and deduplication by ID retained for at least that tolerance.
- [x] Update wiremock tests to recompute the new MAC, prove the body-only MAC differs, prove timestamp mutation invalidates the signature, prove independent deliveries use distinct IDs, and prove retries reuse all signed delivery values.
- [x] Update `docs/architecture/adapters.md` with the new headers, canonical signing input, receiver algorithm, retry semantics, and a worked receiver example; add release-note content for the intentional breaking receiver contract.

## Definition of done

- [x] A captured old `(body, signature)` pair cannot validate as a new delivery under the documented receiver algorithm.
- [x] Retry attempts for one delivery are byte-identical in body and delivery authentication headers; separate deliveries have distinct IDs.
- [x] 2xx-only, no-redirect, timeout/5xx retry, and 4xx non-retry behavior remains covered.
- [x] The receiver migration requirement is explicit in docs/release material; no done certificate is produced.

## Notes (completion record)

**Sender changes** (`crates/adapters/src/webhook/mod.rs`):
`compute_hmac_hex` is now `compute_delivery_signature(secret, timestamp,
delivery_id, body)`, MACing `timestamp "." delivery-id "." body` and returning
`sha256=<hex>`. `sent_at` (RFC3339) and the ULID delivery id are minted once in
`send_webhook` before the retry loop, with a minting assertion; all three
headers travel on every attempt. The in-body `timestamp` is set to the same
single minted instant, so old parsers keep working and the body agrees with the
authenticated header. Retry/backoff/status semantics (2xx success, 5xx +
timeout retry with capped exponential backoff, 4xx no-retry, redirects never
followed) are unchanged, and the webhook keeps its own operator-timeout client
— it deliberately does not use `ProviderTransport` (two clients, two owners).

**Receiver protocol published** (`docs/architecture/adapters.md`): header table,
canonical signing input, the four receiver MUSTs (verify before parse, ±5-minute
freshness, dedup on id retained ≥ tolerance, 2xx-only), retry semantics, a worked
Node.js receiver example, and an explicit breaking-change release note stating
that every existing receiver rejects every delivery until updated, with no
compatibility mode (the source spec's deliberate hard-break decision).

**Tests** (10 webhook tests total, all passing): new-MAC recompute against the
captured headers (incl. `sha256=` prefix + 64-hex length shape); old body-only
pair fails new verification; timestamp/id/body mutation each invalidates;
separator positions matter (no concatenation ambiguity); independent deliveries
carry distinct 26-char ULIDs; a 3-attempt burst reuses one id, one timestamp,
one signature and byte-identical bodies, with the shared signature verifying;
plus the pre-existing 2xx/5xx-retry/4xx-no-retry/no-redirect coverage retained.

No done certificate produced.
