# Change: Add the atproto (Tier 3) identity provider

**Status:** Proposed · **Date:** 2026-06-24 · **Owner:** Ant Stanley · **Target:** crates/* (service)

Add a Tier 3, non-OIDC identity provider for atproto/Bluesky as a new `crates/providers/atproto`
module implementing `IdentityProvider`, selectable with `adapter = "atproto"`. This makes real
the provider that the docs, example configs, and the `IdentityProvider` doc comment already
name but that no code implements today.

---

## Motivation

atproto is referenced across the project (the `IdentityProvider` trait doc comment, several
example configs, the website docs) as a supported provider, but there is no implementation —
the registry only knows `oidc` and `apple`. That is a divergence the canonical spec currently
records as an Open question in [05-provider-system.md](../service/specs/05-provider-system.md).

atproto is not OIDC: authentication uses OAuth with PAR (pushed authorization requests), DPoP
proof-of-possession with server-nonce rotation, per-PDS authorization-server discovery, and DID
resolution to verify the subject. It therefore needs its own module rather than configuration of
the standard OIDC adapter, mirroring how Apple lives in `crates/providers`.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Replace the "Tier 3 — not implemented" note with the implemented atproto provider; remove the matching Open question |
| [`.specs/service/specs/00-overview.md`](../service/specs/00-overview.md) | Flip the scope-summary row for atproto from No to Yes; remove the atproto Open question |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Add `atproto` to the `[providers.<name>]` adapter list |

Adds a new crate `crates/providers/atproto` (no new canonical page; documented within
05-provider-system).

---

## Proposed changes

### `.specs/service/specs/05-provider-system.md` → Tiers, Tier 3 (Modify)

> **Tier 3 — non-OIDC (custom module).** `providers/atproto::AtprotoProvider` implements
> `IdentityProvider` for atproto/Bluesky:
>
> ```toml
> [providers.atproto]
> adapter = "atproto"
> client_id = "https://example.com/oauth/client-metadata.json"
> ```
>
> It does not use OIDC discovery. `exchange_code` performs a PAR-initiated OAuth exchange with a
> DPoP-bound token request (handling the authorization server's `DPoP-Nonce` challenge and
> retrying once with the supplied nonce). `validate_id_token` resolves the subject DID, verifies
> the identity against the resolving PDS's authorization server, and returns `IdentityClaims`
> with the DID as `subject`. The bound DPoP key is generated per provider instance.

### `.specs/service/specs/05-provider-system.md` → Provider registry (Modify)

> ```
> "oidc"    → OidcProvider::from_config
> "apple"   → AppleProvider::from_config
> "atproto" → AtprotoProvider::from_config
> other     → error (unknown adapter)
> ```

### `.specs/service/specs/00-overview.md` → Scope summary (Modify)

> | atproto / non-OIDC provider | Yes | `crates/providers/atproto` (Tier 3: PAR, DPoP, DID) |

---

## Type changes

No new domain entity. `IdentityClaims` already carries `subject` (the DID) and `raw_claims`;
atproto-specific fields land in `raw_claims`. No `canonical-types.schema.json` change.

---

## Implementation notes

1. Add `crates/providers/atproto.rs` (or a submodule) exposing `AtprotoProvider`, mirroring the
   shape of `crates/providers/src/apple.rs`; register `pub mod atproto;` in
   `crates/providers/src/lib.rs`.
2. Implement DPoP proof generation (ES256 key per instance), PAR request construction, the
   `DPoP-Nonce` retry loop, per-PDS authorization-server discovery, and DID resolution
   (`did:plc` and `did:web`).
3. Extend the registry match in the server bootstrap (`crates/server/src/bootstrap.rs`, provider
   construction) to map `"atproto"` to `AtprotoProvider::from_config`.
4. Add provider tests in `crates/providers/tests/` using `wiremock` for PAR/token/DID endpoints;
   no live network in CI.
5. Add a `[providers.atproto]` block to an example config and the provider docs.

References: atproto OAuth spec (PAR + DPoP), DID PLC and did:web resolution.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump those pages' `**Date:**`.
2. Remove the atproto Open questions from 00-overview and 05-provider-system.
3. No schema change to fold in.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`'s Change specs section.

---

## Assumptions and open questions

### Assumptions

- The existing `IdentityProvider` port is sufficient for atproto; no new port method is required.

### Decisions

- *Own module, not OIDC config.* **atproto ships as `crates/providers/atproto`.** Its protocol
  (PAR/DPoP/DID) is incompatible with the standard OIDC adapter, exactly as Apple warranted its
  own module.

### Open questions

- DPoP key lifecycle: per-instance ephemeral vs persisted across restarts — undecided. Persisted
  keys survive token refresh windows but add a storage concern.
- Whether refresh-token handling differs for atproto (DPoP-bound refresh) enough to affect the
  generic refresh flow is open.
