# Change: Fail closed across config, adapters, and the installer

**Status:** Merged · **Date:** 2026-08-05 · **Merged:** 2026-08-16 · **Owner:** Ant Stanley · **Target:** repo-wide (service, adapters, providers, install)

Establish one rule — *a security control that cannot be evaluated must deny, and a
configuration that cannot be validated must refuse to start* — and apply it to the seven
places where `oidc-exchange` currently degrades open instead: an unvalidated
`registration.mode`, empty `issuer`/`audience` defaults, unvalidated signing-algorithm
strings on both key-manager paths, an unchecked HTTP status on provider discovery, a
Postgres migration that degrades open on a probe that proves nothing, an installer that
treats verification-it-could-not-run as verification-that-passed, and URL fields that accept
`http://` where TLS is the whole point. Then remove the shape that produced them, by making
security-relevant configuration closed domain types that a single `resolve()` constructs, so
the next unvalidated field is a compile error rather than a silent downgrade.

---

## Motivation

These are seven independent defects with one signature. In each, a check that should have
been decisive is instead conditional on something that can be absent: a string that matches
no known literal, an HTTP response nobody looked at, a probe that asks a weaker question than
the invariant it stands in for, a hashing utility that is not installed. In every case the
absent thing resolves to *proceed*. The threat model states the rule these violate as **I20**
— "config that would produce an insecure runtime fails at load, not at request time and not
silently" — and, for individual sites, as **I5** (truthful outbound signing metadata), **I6**
(meaningful `iss`/`aud`), **I8** (registration policy is fail-closed), **I7** (one live user
per `(provider, external_id)`), and **I22** (the installer refuses to install what it could
not verify).

Two of them are worth naming individually because they are reachable without an adversary at
all. `registration.mode` is compared by equality against the single literal
`"existing_users_only"` (`crates/core/src/service/exchange.rs:184`), so the positive test is
for the *restrictive* value and every other string — `"existing_users"`,
`"exisiting_users_only"`, `"Open "` — means open registration. An operator typo therefore
re-opens just-in-time provisioning, and any anonymous client holding a genuine account at the
configured IdP gets itself a local user row, an access token, and a 30-day refresh session.
Open registration is also the shipped default, so there is no configuration mistake in this
area that fails toward strictness. And the two shipped AWS KMS reference deployments name
their algorithm in AWS `SigningAlgorithmSpec` vocabulary (`ECDSA_SHA_256`,
`ECDSA_SHA256`) that the adapter's `match` rejects; nothing reads the field at startup, so
those deployments boot, pass health checks, and fail every token issuance and every JWKS
request with a `500`.

The individual fixes are each small. The reason they belong in one change spec is that
fixing them one at a time is exactly what produced the current state: `AppConfig::validate`
is a careful function with a careful doc comment that enumerates five checks, and the fields
that decide security behaviour outnumber it three to one. An enumerating validator over a bag
of `String`s will always trail the fields it governs, so this change also carries the
structural preventive from
[`hardening/proposals/config-closed-domain.md`](../../.security/oidc-exchange/53cbdec9_20260804T102454Z/hardening/proposals/config-closed-domain.md)
Option 2.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/architecture-principles.md`](../architecture-principles.md) | Add a **Fail closed** principle section and its Decision |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Modify **Validation at load** (closed value domains, required fields, `resolve()`), Loading order, Committed default, `[server]`, `[registration]`, `[token]`, `[key_manager]`, `[user_sync]`, `[providers.<name>]`, and Defaults summary |
| [`.specs/service/specs/03-service-flows.md`](../service/specs/03-service-flows.md) | Modify Exchange step 3 (registration policy) to an exhaustive match on a typed mode and re-evaluate the allowlist on the Found arm; replace the *Domain allowlist demands a verified email* and *Existing users bypass policy* Decisions |
| [`.specs/service/specs/02-ports-and-adapters.md`](../service/specs/02-ports-and-adapters.md) | Modify KeyManager (algorithm derived from key material) and Shared OIDC utilities (`discover` checks status) |
| [`.specs/service/specs/05-provider-system.md`](../service/specs/05-provider-system.md) | Modify Tier 1 / Tier 2: provider endpoints are `https`-only; discovery rejects a non-success response |
| [`.specs/service/specs/08-persistence.md`](../service/specs/08-persistence.md) | Modify the PostgreSQL degrade-on-`42501` paragraph: the probe verifies the partial unique index and the `version` column |
| [`.specs/bindings/specs/05-distribution.md`](../bindings/specs/05-distribution.md) | Modify Install script and the *Checksum-verified install* Decision: a missing checksum utility aborts |
| [`.specs/service/specs/canonical-types.schema.json`](../service/specs/canonical-types.schema.json) | `AccessTokenClaims.iss` and `.aud` gain `minLength: 1` |

No new canonical page. Related but **out of scope** here, and cross-referenced rather than
restated: `parse_config`'s missing `${VAR}` resolution on the FFI channels
(`g2-parse-config-placeholder-gap`, the config placeholder-resolution change spec), the
colocation of `/internal/*` with the public listener under `role = "all"`
(`g2-role-all-admin-surface-colocation`, the admin-plane change spec), behavioural parity
across runtime shapes (**I19**, the runtime-parity change spec), and the installer's
`--version` operand traversal plus release signing/attestation
(`g4-installer-version-argument-url-traversal`, the release supply-chain change spec — which
in turn leaves the checksum fail-open to this change). Where this change unifies
`load_config` and `parse_config` onto one `resolve()`, that unification is the mechanism the
placeholder-resolution spec depends on; the two must land in that order. Merge coordination
runs the other way with
[2026-08-05-audit_and_throttle_authentication_failures.md](2026-08-05-audit_and_throttle_authentication_failures.md):
that spec merges after this one and supersedes the `[audit]` keys in the Committed-default
block below — `adapter` becomes `"stdout"`, and the block gains `audit.durability` and a
`[rate_limit]` section — so the `noop` default shown here is a faithful snapshot of the page
this spec merges onto, not a value that survives the pair.

---

## Proposed changes

### `.specs/architecture-principles.md` → Fail closed (Add)

A new section after *Why dynamic dispatch*:

> ## Fail closed
>
> A security control that cannot be evaluated denies. A configuration that cannot be
> validated refuses to start. Neither degrades to the permissive interpretation, and neither
> defers the decision to the first request that depends on it.
>
> Three rules follow, and every crate observes them:
>
> 1. **Closed value domains.** A configuration field that selects a security control is a
>    typed enum or newtype whose constructor is the only way to obtain a value. Comparing an
>    operator-supplied `String` by equality against one literal is the anti-pattern this
>    replaces: it makes the *unrecognised* case indistinguishable from the deliberate one,
>    and it always resolves to whichever branch the `==` did not select.
> 2. **Reject at startup, not at request time.** Wherever the input is configuration, the
>    rejection belongs in config load. A service that will never work correctly refuses to
>    boot rather than running in a weakened mode and reporting itself healthy. Request paths
>    consume already-narrowed types and have no fallback branch to take.
> 3. **A control that could not run did not pass.** An unread HTTP status, an absent hashing
>    utility, a probe that answers a weaker question than the invariant it stands in for — all
>    are failures, not silence. Where a degraded path is genuinely wanted (a DDL-denied
>    database role, an out-of-band migration), it is reached by explicit configuration and
>    still verifies the invariant it is skipping the enforcement of.
>
> The service loads configuration once, at startup; there is no reload path, so these
> guarantees are established exactly once per process.

And a new entry in the page's *Decisions* list:

> - *Fail closed.* **A security control that cannot be evaluated denies; a configuration that
>   cannot be validated refuses to start.** Closed value domains, rejection at startup, and
>   could-not-run-did-not-pass replace per-site permissive fallbacks — the three rules in the
>   *Fail closed* section above.

### `.specs/service/specs/06-configuration.md` → Loading order (Modify)

Step 4 already carries the fail-closed placeholder clause and is unchanged; a new step 5
names the resolve stage:

> 5. `Config::resolve` narrows the merged tree into the typed configuration the service runs
>    on, rejecting any value outside its domain (see *Validation at load*). Every entry point
>    that can construct a running service — the server binary, the Lambda handler, and the FFI
>    `new`/`from_file` constructors — produces its configuration through this one function.

### `.specs/service/specs/06-configuration.md` → Validation at load (Modify)

The existing section — added when
[2026-07-01-complete_config_loading.md](merged/2026-07-01-complete_config_loading.md) merged,
carrying the enumerating checks `AppConfig::validate` ships today — is replaced whole. Every
check it lists survives (role and the three duration fields in the table, allowlist entry
shape in `AsciiDomainPattern`, the internal-API secret as a cross-field check); what changes
is the mechanism, from an enumerating validator to construction of closed types:

> ## Validation at load
>
> Configuration is parsed in two stages. `RawConfig` mirrors the TOML exactly and is only ever
> an intermediate; `Config` is what the service holds, and its security-relevant fields are
> enums, newtypes, and constrained URLs rather than strings. `Config::resolve(raw, env)`
> performs placeholder substitution, applies environment overrides, and constructs the
> narrowed types, returning a `ConfigError` naming the offending field. A value that cannot be
> narrowed aborts startup; there is no permissive fallback and no per-request re-parse.
>
> The closed domains:
>
> | Field | Type | Domain |
> |---|---|---|
> | `server.role` | `ServerRole` | `all` \| `exchange` \| `admin` |
> | `server.issuer` | `HttpsUrl` | required, non-empty, absolute `https` URL |
> | `registration.mode` | `RegistrationMode` | `open` \| `existing_users_only` |
> | `registration.domain_allowlist[]` | `AsciiDomainPattern` | exact domain or `*.`-prefixed wildcard; non-ASCII rejected |
> | `token.audience` | `NonEmptyString` | required, non-empty |
> | `token.access_token_ttl`, `refresh_token_ttl`, `server.request_timeout` | `Duration` | `<integer><s\|m\|h\|d>`, no overflow |
> | `key_manager.local.algorithm` | `SigningAlgorithm` | `EdDSA` — the only algorithm the local adapter can produce |
> | `key_manager.kms.algorithm` | `SigningAlgorithm` | `RS256`\|`RS384`\|`RS512`\|`PS256`\|`PS384`\|`PS512`\|`ES256`\|`ES384`\|`ES512` (JWS names, RFC 7518 §3.1 — not AWS `SigningAlgorithmSpec` names) |
> | `audit.adapter` | `AuditAdapter` | `noop` \| `stdout` \| `stderr` \| `auto` \| `sqs` — the vocabulary the bootstrap accepts today ([02-ports-and-adapters.md](02-ports-and-adapters.md) → Adapter inventory) |
> | `audit.blocking_threshold`, `emit_threshold` | `AuditSeverity` | syslog severity name |
> | `telemetry.exporter` | `TelemetryExporter` | `none` \| `stdout` \| `otlp` \| `xray` |
> | `internal_api.auth_method` | `InternalAuthMethod` | `shared_secret` |
> | `user_sync.webhook.url` | `HttpsUrl` | `https` only |
> | `providers.<name>.{issuer,jwks_uri,token_endpoint,revocation_endpoint}` | `HttpsUrl` | `https` only |
> | `providers.<name>.adapter` | `ProviderAdapter` | `oidc` \| `apple` |
>
> Two cross-field checks run in the same pass:
>
> - **Internal-API secret.** When the internal API will be served (`server.role` is `admin` or
>   `all` and `internal_api.enabled = true`), `internal_api.shared_secret` must be present and
>   non-empty.
> - **Algorithm truthfulness.** The key manager reports the algorithm derived from the key
>   material it loaded; `resolve` compares the operator's declared `algorithm` against it and
>   fails when they disagree. The configured value is an assertion to verify, never metadata
>   to republish ([02-ports-and-adapters.md](02-ports-and-adapters.md) → KeyManager).
>
> `oidc-exchange config check <path>` runs the same `resolve` with no side effects and prints
> the configuration a deployment would get, so an operator can test a config file without
> starting the service.

### `.specs/service/specs/06-configuration.md` → Committed default (Modify)

The default block gains `issuer` and `audience`, and the paragraph below it changes:

> ```toml
> [server]
> host = "0.0.0.0"
> port = 8080
> issuer = "${OIDC_EXCHANGE_ISSUER}"
>
> [registration]
> mode = "open"
>
> [token]
> access_token_ttl = "15m"
> refresh_token_ttl = "30d"
> audience = "${OIDC_EXCHANGE_AUDIENCE}"
>
> [audit]
> adapter = "noop"
> blocking_threshold = "warning"
>
> [telemetry]
> enabled = false
> exporter = "none"
> ```
>
> The default is deliberately minimal — no key manager, no repository, no providers — but it is
> not startable on its own: `server.issuer` and `token.audience` have no defensible default, so
> the shipped file names them as placeholders and a deployment that does not supply them fails
> config load. A service that would sign tokens carrying `iss: ""` and `aud: ""` is not
> usefully runnable, and the empty values are not representable in `Config`.

### `.specs/service/specs/06-configuration.md` → Sections (Modify)

The affected section entries read:

> ### `[server]`
> `host` (`0.0.0.0`), `port` (`8080`), `issuer` (**required** — the `iss` claim and the
> discovery issuer; an absolute `https` URL, no default), `role` (`all` | `exchange` | `admin`,
> default `all`), `request_timeout` (humantime duration string like the token TTLs, default
> `"30s"`) — the per-request timeout the server's timeout layer enforces — and `base_path`
> (optional, default unset — a leading prefix such as `/prod` stripped from incoming request
> paths before routing; honored in both Lambda and server mode, though it exists chiefly for
> API Gateway stages and mount prefixes).
>
> ### `[registration]`
> `mode` (`open` | `existing_users_only`, default `open`) — a closed domain: an unrecognised
> value is a config-load error, never a silent selection of `open`. Optional
> `domain_allowlist` (exact or `*.domain` wildcard, ASCII only).
>
> ### `[token]`
> `access_token_ttl` (`"15m"`), `refresh_token_ttl` (`"30d"`), `audience` (**required**,
> non-empty — the `aud` claim of every issued access token), optional `custom_claims`
> (`HashMap<String,String>` of claim templates, see [03-service-flows.md](03-service-flows.md)).
>
> ### `[key_manager]`
> `adapter` (`local` | `kms`), with `[key_manager.local] { private_key_path, algorithm, kid }`
> or `[key_manager.kms] { key_id, algorithm, kid }`. `algorithm` is a JWS `alg` name (RFC 7518
> §3.1), validated at load against the algorithms the selected adapter can actually produce —
> `EdDSA` for `local`, and `RS`/`PS`/`ES` 256/384/512 for `kms`. AWS `SigningAlgorithmSpec`
> names (`ECDSA_SHA_256`) are not accepted. Skipped (noop) in the `admin` role.
>
> ### `[user_sync]`
> `enabled` (bool), `adapter` (`webhook`), `[user_sync.webhook] { url, secret, timeout?,
> retries? }`. `url` must be `https` — the payload carries the full user record. The `secret`
> is redacted in `Debug`.
>
> ### `[providers.<name>]`
> `adapter` (`oidc` | `apple`) plus adapter-specific fields captured via a flattened
> `extra: HashMap<String, toml::Value>`. Every endpoint field (`issuer`, `jwks_uri`,
> `token_endpoint`, `revocation_endpoint`) must be `https`. See
> [05-provider-system.md](05-provider-system.md).

### `.specs/service/specs/06-configuration.md` → Defaults summary (Modify)

> | Setting | Default |
> |---|---|
> | `server.host` / `port` / `role` / `request_timeout` | `0.0.0.0` / `8080` / `all` / `"30s"` |
> | `server.issuer`, `token.audience` | *(required — no default)* |
> | `registration.mode` | `open` |
> | `token.access_token_ttl` / `refresh_token_ttl` | `15m` / `30d` |
> | `audit.adapter` / `blocking_threshold` / `emit_threshold` | `noop` / `warning` / `info` |
> | `telemetry.enabled` / `exporter` / `sample_rate` | `false` / `none` / `1.0` |
> | `user_sync.enabled`, `internal_api.enabled` | `false`, `false` |

### `.specs/service/specs/03-service-flows.md` → Exchange, step 3 (Modify)

The **Not found** arm reads:

>    - **Not found** → apply policy. The policy value is a `RegistrationMode`, matched
>      exhaustively; there is no unrecognised case at this point because config load rejected
>      it.
>      - The ID token must carry a **verified** email (`email_verified == Some(true)`) — a
>        requirement of accepting the claim at all, not of the allowlist branch. A missing or
>        unverified email → `AccessDenied` (audited `RegistrationDenied`).
>      - If `registration.domain_allowlist` is set, the email's domain must match it — exact
>        (`example.com`) or wildcard (`*.example.com`, at least one subdomain, ASCII
>        case-insensitive). A non-matching domain → `AccessDenied` (audited
>        `RegistrationDenied`).
>      - `RegistrationMode::ExistingUsersOnly` → `AccessDenied` (`RegistrationDenied`).
>      - `RegistrationMode::Open` → `create_user(NewUser{…})` (audited `UserCreated`); if
>        creation returns `Conflict` (a concurrent first login won the race), re-run
>        `get_user_by_external_id` and continue with the existing user, re-applying the
>        suspended-status check. The losing racer emits no `UserCreated` event — the winning
>        create already audited it — and the flow otherwise proceeds as for a found user.

And the **Found** arm — which today checks `user.status` and nothing else, so a tightened
allowlist never affects the accounts it was tightened against
(`g1-registration-policy-never-reevaluated`) — gains the same predicate:

>    - **Found** → the suspended-status check as today; then, when
>      `registration.domain_allowlist` is set, re-apply it against the **assertion's current
>      claims** — `email_verified == Some(true)` and a matching domain, the identical checks
>      the Not-found arm applies, evaluated by the same predicate. A failure → `AccessDenied`
>      (audited `RegistrationDenied`, naming the user id — the Not-found arm has no subject
>      to name; this arm does, and an operator investigating which accounts a tightened
>      allowlist locked out needs it). The inputs are the live claims from the validated ID
>      token, not the stored `user.email`: the row's email is frozen at first login, and
>      enforcing yesterday's identity against today's policy is the shape of defect this arm
>      exists to close. `registration.mode` is **not** re-evaluated here — for an existing
>      user `existing_users_only` is trivially satisfied, and making the mode retroactive
>      needs a provisioning-provenance field on `User` that is a schema migration, not a
>      fail-closed fix (see Decisions).

### `.specs/service/specs/03-service-flows.md` → Decisions (Modify)

The *Domain allowlist demands a verified email* entry is replaced by:

> - *Registration demands a verified email.* **Every just-in-time user creation requires
>   `email_verified == true`, whether or not an allowlist is configured.** The requirement is a
>   property of accepting the email claim, not of the allowlist; nesting it inside an optional
>   feature's branch meant turning the allowlist off turned identity verification off with it.

And the *Existing users bypass policy* entry — "Registration policy applies only when no
user exists. Tightening the allowlist later does not lock out already-registered users." —
recorded the defect as a feature and is replaced by:

> - *The allowlist is an authorization predicate; the mode is an admission gate.* **The
>   domain allowlist is re-evaluated on every exchange, for existing users as well as new
>   ones; `registration.mode` applies only at creation.** An operator who tightens the
>   allowlist is trying to contain accounts that already exist, and a control consulted only
>   at admission silently does nothing for exactly those accounts — the containment action
>   an operator reaches for first must not be the one that does not work.
>   Re-evaluating the *mode* for an existing user is not coherent without recording how the
>   user was provisioned (`existing_users_only` is trivially satisfied by existing), and
>   adding that provenance is a `User` schema migration that belongs to its own change; the
>   refresh path likewise holds no fresh claims to evaluate the allowlist against. For both,
>   containment remains suspending or deleting the user — honoured immediately on every
>   path — and the refresh-side residual window is bounded by `token.refresh_token_ttl`.

### `.specs/service/specs/02-ports-and-adapters.md` → KeyManager (Modify)

The paragraph after the trait block gains:

> `algorithm()` returns the algorithm **derived from the key material the adapter loaded**, not
> the operator's configured string. The local adapter parses an Ed25519 PKCS#8 PEM and reports
> `EdDSA`; the KMS adapter reports the algorithm its configured JWS name maps to, checked
> against the SPKI it fetches for the JWK. Config load compares the declared
> `key_manager.*.algorithm` against this value and fails when they disagree, so the `alg` in
> every issued JWT header, the JWK at `GET /keys`, and
> `id_token_signing_alg_values_supported` in the discovery document all describe the key that
> actually signs.

### `.specs/service/specs/02-ports-and-adapters.md` → Shared OIDC utilities (Modify)

> - `discovery::discover(issuer)` — fetches and parses `.well-known/openid-configuration` into
>   `DiscoveryDocument { issuer, token_endpoint, jwks_uri, revocation_endpoint }`. A non-success
>   HTTP status is rejected before the body is read (`ProviderError` naming the issuer and the
>   status), matching `JwksCache`'s handling of the same failure; the parsed `issuer` must then
>   equal the configured issuer per RFC 8414 §3.3.

### `.specs/service/specs/05-provider-system.md` → Tiers (Modify)

The Tier 1 paragraph after the config block, and the Tier 2 block, gain the endpoint
constraint:

> `from_config` discovers the `token_endpoint`, `jwks_uri`, and `revocation_endpoint` from the
> issuer's `.well-known/openid-configuration` when they are not given. Every endpoint —
> configured or discovered — is an `https` URL; the config types make any other scheme
> unrepresentable, and discovery rejects a response whose HTTP status is not a success before
> it parses the body. Adding a Tier 1 provider is a new config block — no code.
>
> The same constraint applies to the Apple provider's optional `token_endpoint`, `jwks_uri`,
> and `revocation_endpoint` overrides, which take the identical `HttpsUrl` type rather than
> repeating the check.

### `.specs/service/specs/08-persistence.md` → PostgreSQL (Modify)

The degrade-on-`42501` sentences are replaced by:

> When the migration is instead denied by Postgres itself — the connected role lacks DDL rights
> and the DDL fails with SQLSTATE `42501` (`insufficient_privilege`) — `create_pool` degrades
> only after verifying the invariants the migration would have established. It logs a
> structured warning and probes for: the `users` and `sessions` tables; the
> `idx_users_external_id_provider` index, which must exist, be **unique**, and be **partial**
> (`indisunique` and a non-null `indpred` in `pg_index`); and the `users.version` column. The
> pool is returned only when every probe passes. If any is missing or the probe itself fails,
> `create_pool` returns the **original** migration error and startup fails — the denied DDL is
> why startup is failing, and an inconclusive probe must not mask it. Table presence alone is
> not sufficient: the partial unique index is the only enforcer of "one live user per
> `(provider, external_id)`", the registration path depends on the database raising `23505`,
> and a schema provisioned out of band without that index is otherwise indistinguishable from a
> fully migrated one. Every other migration failure still fails fast.

### `.specs/bindings/specs/05-distribution.md` → Install script (Modify)

> A bash installer for `antstanley/oidc-exchange`. It detects OS (`uname -s`) and arch
> (`uname -m`), maps to a release asset name, downloads the binary and its SHA-256 checksum from
> GitHub Releases, verifies the checksum, and installs to `/usr/local/bin` (root) or
> `~/.local/bin` (non-root). It accepts a `--version`/positional pin and defaults to the latest
> release; it requires `curl`/`wget` and `sha256sum`/`shasum`. Verification is mandatory: if
> neither checksum utility is present the installer prints the missing dependency and exits
> non-zero **before** the downloaded binary is made executable or moved onto `PATH`. There is no
> path through the script that installs an unverified binary.

### `.specs/bindings/specs/05-distribution.md` → Decisions (Modify)

> - *Checksum-verified install, fail closed.* **`install.sh` verifies SHA-256 before installing,
>   and aborts when it cannot verify.** Detects tampering or truncated downloads; a host without
>   `sha256sum` or `shasum` gets an error and no install, not a warning and an unchecked binary.
>   The checksum sidecar is fetched from the same release URL prefix as the binary, so it
>   establishes integrity — these are the bytes someone published — and not authenticity;
>   signing the checksum manifest in the release pipeline is the remaining gap.

---

## Type changes

No domain entity is added or removed. Two existing fields tighten, because the values they
now hold cannot be empty:

```json
{
  "$comment": "Fragment for 2026-08-05-fail_closed_across_config_and_adapters. Folds into .specs/service/specs/canonical-types.schema.json on merge.",
  "$defs": {
    "AccessTokenClaims": {
      "type": "object",
      "description": "JWT payload. custom claims are flattened alongside the registered claims.",
      "required": ["sub", "iss", "aud", "iat", "exp"],
      "properties": {
        "sub": { "$ref": "../../canonical-types.schema.json#/$defs/Id" },
        "iss": {
          "type": "string",
          "minLength": 1,
          "description": "server.issuer; required non-empty at config load, so an empty iss is unrepresentable."
        },
        "aud": {
          "type": "string",
          "minLength": 1,
          "description": "Single audience string (no array). token.audience; required non-empty at config load."
        },
        "iat": { "type": "integer" },
        "exp": { "type": "integer" }
      },
      "additionalProperties": true
    }
  }
}
```

The new Rust types are configuration types and do not belong in the canonical entity schema:
`RegistrationMode`, `ServerRole`, `SigningAlgorithm`, `AuditAdapter`, `TelemetryExporter`,
`InternalAuthMethod`, `ProviderAdapter` (enums), and `HttpsUrl`, `AsciiDomainPattern`,
`NonEmptyString` (newtypes). Their value domains are the table in *Validation at load* above,
which is the documentation surface for them.

---

## Implementation notes

Order matters: the closed-domain types land first so the per-site fixes can be expressed as
type changes rather than as new `if` statements.

```
 1. crates/core/src/config.rs — define the domain types and their constructors.
 2. crates/core/src/config.rs — split AppConfig into RawConfig + Config; write
    Config::resolve(raw, env), subsuming (not deleting) today's AppConfig::validate:45-93.
 3. crates/server/src/bootstrap.rs:90-128 — repoint load_config and parse_config at resolve().
 4. Update the use sites named below.
 5. Add `oidc-exchange config check <path>`; regenerate schemas/ and docs/.
```

Per site:

1. **`registration.mode`** — `crates/core/src/service/exchange.rs:184` compares
   `self.config.registration.mode == "existing_users_only"`. Replace with an exhaustive
   `match` on `RegistrationMode`. Lift the `email_verified` guard
   (`exchange.rs:128-145`) out of the `if let Some(ref allowlist)` branch that starts at
   `exchange.rs:124` so it runs for every JIT create; the allowlist branch keeps only the
   domain match. The `"open"` default stays — see Decisions. Lift the whole allowlist
   evaluation into one predicate (`enforce_domain_allowlist(email, email_verified, user_id,
   &request)` — the finding's `poc/fix.patch` is a compiling reference) called from **both**
   arms of the user lookup: the `Some(user)` arm immediately after its status check, passing
   `Some(&user.id)` so the denial event names its subject, and the `None` arm's inline block
   collapsing to the same call. `refresh`
   (`crates/core/src/service/refresh.rs`) is deliberately unchanged — see Decisions.
2. **`issuer` / `audience`** — `crates/core/src/config.rs:152-163` (`ServerConfig::default`
   sets `issuer: String::new()`) and `:190-199` (`TokenConfig::default` sets
   `audience: None`). Remove both defaults; make the fields required in `RawConfig`.
   Sinks that stop being able to emit empty values:
   `crates/core/src/service/mod.rs:66-100` (`build_access_token`) and
   `crates/server/src/routes/well_known.rs:8-20` (which string-concatenates the issuer into
   every advertised endpoint, today producing the bare relative strings `/keys`, `/token`,
   `/revoke`). Update `config/default.toml` per the Committed default block.
3. **Local-key algorithm** — `crates/adapters/src/local_keys/mod.rs:16-41` stores the operator
   string verbatim and always parses Ed25519; `:67-88` republishes it in the JWK and
   `:82-83` returns it from `algorithm()`. Have `from_pem`/`from_file` return the derived
   algorithm and reject a configured value that is not `EdDSA`. The half-followed recipe the
   scan calls out — an Ed25519 key labelled `ES256`, which two shipped docs pages present as
   valid — is the case the test must cover; a real P-256 key already fails closed at
   `from_pem`.
4. **KMS algorithm** — `crates/adapters/src/kms/mod.rs:41-57` (`signing_algorithm`) is a closed
   match over nine JWS names, but it is consulted only inside `sign` (`:389`), so a bad value
   surfaces at first signature. Validate the field at load, and fix the two shipped examples:
   `examples/aws-web/config/oidc-exchange.toml:17` (`ECDSA_SHA_256`) and
   `examples/ecs-fargate/config/fargate.toml:11` (`ECDSA_SHA256`) → `ES256`. Three docs pages
   carry the same AWS vocabulary and need the same edit.
5. **Discovery status** — `crates/adapters/src/shared/discovery.rs:18-47`: the request goes out
   at `:23-30` and the body is parsed at `:31-37` with nothing in between. Insert a
   `response.status().is_success()` guard mirroring `crates/adapters/src/shared/jwks.rs:179-185`.
   The RFC 8414 issuer-equality check at `:39-47` stays. The call site is
   `crates/adapters/src/oidc/mod.rs:74-96`, which runs once at startup and pins the result for
   the process lifetime.
6. **Postgres migrations** — `crates/adapters/src/postgres/mod.rs:157-195`. The probe at
   `:170-182` asks `to_regclass('users')` / `to_regclass('sessions')`. Extend it to join
   `pg_index`/`pg_class` and assert `indisunique` and `indpred IS NOT NULL` for
   `idx_users_external_id_provider` (the DDL it stands in for is at `:34-40`), plus the
   presence of `users.version`. Keep returning the original `err` on any failure (`:184-189`).
7. **Installer** — `install.sh:82-91`. The `else` arm prints a warning; `echo` returns zero, so
   `set -euo pipefail` has nothing to act on and control walks into `chmod +x` / `mv` at
   `:101-103`. Replace the warning with a message on stderr and `exit 1`.
8. **URL schemes** — `HttpsUrl` replaces the `String` in
   `crates/core/src/config.rs:322-328` (`WebhookConfig.url`, consumed at
   `crates/adapters/src/webhook/mod.rs:42-56` and posted to at `:133-155`) and in
   `crates/core/src/domain/provider.rs:6-20` (`OidcProviderConfig.{issuer, jwks_uri,
   token_endpoint, revocation_endpoint}`, consumed at `crates/adapters/src/oidc/mod.rs:74-96`).
   `crates/providers/src/apple.rs:133-151` reads its `token_endpoint`, `jwks_uri`, and
   `revocation_endpoint` overrides out of the flattened `extra` map and needs the same
   constructor rather than its own check — the scan's point is that the Apple path is a copy
   of the generic one and copies drift.

Tests: a table-driven fail-closed corpus asserting a `ConfigError` naming the field for each
offending value (`registration.mode = "existing_users"`, `key_manager.kms.algorithm =
"ECDSA_SHA_256"`, `jwks_uri = "http://…"`, `issuer = ""`, …); an `HttpsUrl` constructor test
rejecting `http`, `file`, and scheme-less inputs; an exchange for an *existing* user whose
verified email domain falls outside a newly-tightened allowlist, asserting `AccessDenied`
and a `RegistrationDenied` event carrying the user id; a discovery test asserting rejection of a
well-formed document served under `404` and `500`; a Postgres test that drops the partial index
and asserts a DDL-denied role fails startup; and an `install.sh` test with both checksum
utilities masked off `PATH`, asserting a non-zero exit and no file at the install path. Existing
`http://` test fixtures (wiremock servers) use a `#[cfg(test)]`-only `HttpsUrl` constructor.

References: `hardening/proposals/config-closed-domain.md` (Option 2 and its migration plan),
`hardening/proposals/provider-response-boundary.md`,
`findings/g1-registration-policy-never-reevaluated/` (the Found-arm re-evaluation and its
declined mode/refresh halves), `artifacts/01_context/threat_model.md`
invariants I5, I6, I7, I8, I20, I22 — all under
`.security/oidc-exchange/53cbdec9_20260804T102454Z/`.

---

## Compatibility

This change stops some existing deployments from booting. That is the intent: each one is a
deployment that is already not doing what its operator believes it is doing. The set is
knowable in advance:

| What breaks | Why | What the operator does |
|---|---|---|
| Any config without `server.issuer` or `token.audience`, including bare `config/default.toml` | Both become required | Set them; there is no defensible default |
| A mistyped `registration.mode` | Unrecognised values are rejected instead of meaning `open` | Fix the typo — and note the deployment has been open-registration until now |
| An existing user whose email domain is outside the configured `domain_allowlist` | The allowlist is re-evaluated on every exchange, not only at first login | Intended containment — the accounts a tightened allowlist excludes stop exchanging; their refresh sessions expire within `token.refresh_token_ttl`, and suspension remains the immediate lever |
| `examples/aws-web` and `examples/ecs-fargate` KMS configs | AWS `SigningAlgorithmSpec` names are not JWS `alg` names | Fixed in this change; a deployment copied from them needs `ES256` |
| `key_manager.local.algorithm` set to anything but `EdDSA` | The local adapter only signs Ed25519 | Set `EdDSA`, or move to the KMS adapter for `ES*`/`RS*` |
| Any `http://` `server.issuer`, webhook URL, or provider endpoint | `HttpsUrl` rejects it | Use the deployment's `https` URL; for a webhook, terminate TLS at the receiver or front it |
| A mistyped `audit.blocking_threshold` / `emit_threshold`, `telemetry.exporter`, or `internal_api.auth_method` | Values that today fall back silently (`Warning`/`Info`, stdout logs) are rejected | Fix the value; the permissive release's warnings name it |
| A `registration.domain_allowlist` entry containing non-ASCII | `AsciiDomainPattern` rejects non-ASCII, closing the Unicode case-fold collision | Use the ASCII (punycode) form of the domain |
| A Postgres deployment with a DDL-denied role and a schema missing the partial unique index | The probe now verifies the invariant, not just table presence | Apply the index out of band, or grant DDL for one startup |
| `curl \| sh` on a host without `sha256sum`/`shasum` | Verification is mandatory | Install `coreutils`, or download and verify manually |

Rollout follows the proposal's two-phase plan. `resolve()` ships first in **permissive mode**:
it constructs the narrowed types, and on any failure emits a structured `config_rejection`
warning naming the field and the value class, then falls back to today's interpretation. That
release's notes state plainly that the warnings become errors in the next minor version. The
flip to hard failure follows one minor version later, with the flag retained for one further
version as a documented, temporary escape hatch. `config check` ships in the permissive
release so operators can test the flip before it reaches them. The installer, the discovery
status check, and the tightened Postgres probe are not part of the permissive window — none of
them has a configuration surface, and none has a legitimate deployment that depends on the
current behaviour (a DDL-denied schema that boots today but fails the new probe is missing the
very uniqueness invariant it claims to enforce).

**The open-registration default does not flip in this change.** `registration.mode` stays
`open`: it is the product's advertised primary mode, and changing it would break every
correctly-configured existing deployment for a reason unrelated to the fail-open defect being
fixed. What changes is that `open` becomes reachable only by asking for it. Whether the default
should flip in a future major version is a product decision, recorded in Open questions.

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to
   the merge date.
2. Fold the `Type changes` `$defs` into `.specs/service/specs/canonical-types.schema.json`
   (`iss` and `aud` gain `minLength: 1` and descriptions).
3. Confirm the replaced *Validation at load* section on
   `.specs/service/specs/06-configuration.md` still carries every check the canonical page
   lists today (`server.role`, the three duration fields, allowlist entry shape, the
   internal-API secret), now expressed through the closed-domain table and the cross-field
   checks. `.specs/service/specs/04-http-api.md`'s bootstrap step 2 already says "then
   validate" and needs no edit here; the placeholder-resolution change spec owns rewording it
   for the shared resolve.
4. No new canonical page to create or index.
5. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
6. Update `.specs/README.md`'s Change specs table.

---

## Assumptions and open questions

### Assumptions

- Config is loaded once at startup and there is no reload path, so establishing these
  invariants in `resolve()` establishes them for the process lifetime. If hot reload is ever
  added, they must be re-established at reload time.
- The five checks `AppConfig::validate` performs today (`crates/core/src/config.rs:45-93`) are
  subsumed by `resolve()`, not replaced by it, and are not deleted until `resolve()` is
  authoritative.
- `.specs/service/specs/06-configuration.md` now carries the `Validation at load` section and
  the fail-closed placeholder wording in Loading order, and
  `.specs/service/specs/04-http-api.md`'s bootstrap step 2 carries the "then validate" clause —
  the previously-unapplied blocks from
  [2026-07-01-complete_config_loading.md](merged/2026-07-01-complete_config_loading.md) have
  been applied, so canonical and code agree on the shipped checks
  (`crates/core/src/config.rs:45-93`). This change therefore modifies that section in place,
  and its replacement must keep subsuming every shipped check.
- Every deployment reachable by the maintainers can be given one minor version of warnings
  before the flip, and release notes are an adequate channel for reaching the rest.

### Decisions

- *One change spec, seven sites.* **The fail-open sites ship together with the type change that
  prevents the eighth.** Fixing them individually is what produced the current state: an
  enumerating validator over a bag of `String`s covers a third of the fields that decide
  security behaviour and has no mechanism that forces it to keep up.
- *Startup over request time.* **Wherever the input is configuration, the rejection is at config
  load.** A service that will never work correctly should refuse to boot rather than pass its
  health checks and fail every sign-in — which is precisely what the shipped KMS examples do
  today.
- *Closed domain types, not more validator arms.* **Option 2 of `config-closed-domain.md`.**
  Under a validator, the check is a statement that runs before the value is used; under a type,
  the check is the only way the value can be constructed, and the compiler refuses to build a
  use site that has not decided what a new variant means.
- *String equality against one literal is the anti-pattern.* **`mode == "existing_users_only"`
  is replaced by an exhaustive match, not by a second comparison.** A positive test for the
  restrictive value makes every unrecognised string mean permissive.
- *Algorithm is derived, declared value is verified.* **`KeyManager::algorithm()` reports what
  the key material says; the configured string is cross-checked at load.** A type on the config
  field alone would still let an Ed25519 key labelled `ES256` boot and lie on three published
  surfaces.
- *`token.audience` is required rather than omitted when absent.* **A missing audience is a
  config error, not an absent claim.** I6 requires a token minted for one audience not to be
  replayable at another sharing the issuer; today the field serialises as `""`, which is worse
  than either a real value or a genuinely absent claim.
- *No loopback exemption for `HttpsUrl`.* **`http://127.0.0.1` is rejected like any other
  non-`https` URL; test fixtures use a `#[cfg(test)]`-only constructor.** A runtime exemption is
  a scheme check with a bypass, and the bypass is what ends up in production.
- *The open-registration default stays.* **`registration.mode = "open"` remains the default; the
  fix is that it can no longer be selected by accident.** Flipping it is a product decision about
  who the default operator is, and bundling it here would break correctly-configured deployments
  for an unrelated reason.
- *Permissive window for config, none for the installer, discovery, or the Postgres probe.*
  **The two-phase rollout covers `resolve()` only.** None of `install.sh`, `discover()`, or the
  migration probe has a configuration surface or a legitimate deployment depending on the
  current behaviour.

### Open questions

- Do the two shipped KMS reference deployments (`examples/aws-web`, `examples/ecs-fargate`) mean
  those templates have never been run end to end? Both name an algorithm the adapter rejects, so
  neither could ever have issued a token. A `config check` run over `examples/` answers this in
  minutes, and the answer changes how much trust every shipped template deserves — and whether
  template conformance belongs in CI (Option 3 of the proposal) or with the
  reference-deployment baseline work.
- Should the KMS adapter make a `kms:GetPublicKey` call at startup to establish algorithm
  truthfulness? It already fetches the SPKI lazily for the JWK, so a startup fetch would also
  surface a wrong `key_id` early — but it adds an AWS API call to boot, and a cold Lambda pays it
  per init. If it is not acceptable, the KMS half of the algorithm cross-check needs a
  documented exception rather than a silent gap.
- How long should the permissive window be? One minor version is the proposal's suggestion; it
  depends on the release cadence and on whether there are known deployments the maintainers can
  consult directly.
- Should the release pipeline sign the checksum manifest? Fixing the fail-open in `install.sh`
  makes verification mandatory but leaves it same-origin: the sidecar comes from the same URL
  prefix as the binary and carries no signature. That is an authenticity gap this change names
  but does not close — the release supply-chain change spec owns it (Sigstore build
  attestation, verified by the installer where `gh` is available, with this change's mandatory
  checksum as the floor beneath it).
- Should `RawConfig` gain `deny_unknown_fields`, so a mistyped *key* is rejected as loudly as a
  mistyped *value*? It is a genuine control — a renamed field silently ignores its
  `OIDC_EXCHANGE__…` override today — but it turns every stale key in every existing TOML into a
  startup failure and would need its own flag and a longer permissive window.
