# Task 06 — Bind webhook deliveries and document receivers

**Plan:** [plan.md](../plan.md) · **Status:** Backlog · **Depends on:** —

**Implements:** source webhook contract and implementation note 9; webhook test list; compatibility section.

**Scope:** Bind the webhook HMAC to an RFC3339 timestamp and ULID delivery ID plus raw body, mint all delivery identity material once outside retries, and publish the required receiver verification/deduplication protocol. Webhook owns its own configured-timeout `reqwest::Client`; it does not use `ProviderTransport`.

## Steps

- [ ] Change `compute_hmac_hex` to MAC `timestamp`, `.`, `delivery_id`, `.`, raw body; prefix emitted value with `sha256=`.
- [ ] Mint `sent_at` and `ulid::Ulid` once per logical delivery outside the retry loop; send `X-Webhook-Timestamp`, `X-Webhook-Delivery-Id`, and `X-Signature-256` on every attempt.
- [ ] Preserve JSON body timestamp for existing parsers while requiring receiver verification before parse, ±5-minute timestamp acceptance, and deduplication by ID retained for at least that tolerance.
- [ ] Update wiremock tests to recompute the new MAC, prove the body-only MAC differs, prove timestamp mutation invalidates the signature, prove independent deliveries use distinct IDs, and prove retries reuse all signed delivery values.
- [ ] Update `docs/architecture/adapters.md` with the new headers, canonical signing input, receiver algorithm, retry semantics, and a worked receiver example; add release-note content for the intentional breaking receiver contract.

## Definition of done

- [ ] A captured old `(body, signature)` pair cannot validate as a new delivery under the documented receiver algorithm.
- [ ] Retry attempts for one delivery are byte-identical in body and delivery authentication headers; separate deliveries have distinct IDs.
- [ ] 2xx-only, no-redirect, timeout/5xx retry, and 4xx non-retry behavior remains covered.
- [ ] The receiver migration requirement is explicit in docs/release material; no done certificate is produced.
