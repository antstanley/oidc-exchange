# Change: Complete config loading (overlay, env overrides, placeholders) and startup validation

**Status:** Proposed · **Date:** 2026-07-01 · **Owner:** Ant Stanley · **Target:** crates/server + crates/core

Implement the full four-step config loading order that
[06-configuration.md](../service/specs/06-configuration.md) already documents — deep-merge
overlay, `OIDC_EXCHANGE__{section}__{key}` env overrides, fail-closed `${VAR}` placeholder
resolution — and validate the loaded config at startup (role, TTLs, registration allowlist,
internal-API secret) instead of failing silently or per-request.

---

## Motivation

`bootstrap::load_config` implements roughly half of spec 06. Of the documented loading order,
only step 1 works as specified: the `OIDC_EXCHANGE_ENV` TOML _replaces_ `config/default.toml`
instead of overlaying it, `OIDC_EXCHANGE__{section}__{key}` env overrides do not exist, and no
code anywhere in the repo resolves `${VAR}` placeholders. The last gap is a security bug: spec
06 (and the READMEs, docs, and shipped example configs) tell operators to write secrets as
`shared_secret = "${INTERNAL_API_SECRET}"` — the code accepts the literal string
`${INTERNAL_API_SECRET}` as the admin Bearer credential for every `/internal/*` route. A
deployment that follows the documentation fails open with a publicly guessable secret.

Startup validation has the same shape of problem — misconfiguration is absorbed instead of
rejected: `server.role` is an unvalidated free string, so a typo like `"exchang"` builds an
empty router (not even `/health`); an empty `shared_secret = ""` counts as configured and
`internal_api.enabled` is never read; token TTLs are parsed per-request by a function that can
panic on a multi-byte final character; and the registration allowlist matcher accepts `"*"`
(matches every domain) and `"*example.com"` (matches `evilexample.com`). One change fixes all
of these under a single theme: config is merged, resolved, and validated once, at startup, and
bad config fails closed.

---

## Affected spec pages

| Canonical page                                                                     | Nature of change                                                                                                                                                |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Loading order already reads correctly for the end state (spec is ahead of the code). Add fail-closed placeholder wording and a new "Validation at load" section |
| [`.specs/service/specs/04-http-api.md`](../service/specs/04-http-api.md)           | Bootstrap step 2 already reads correctly. Modify the internal-auth paragraph (`enabled` gate, non-empty secret) and note validation in the bootstrap sequence   |

---

## Proposed changes

### `.specs/service/specs/06-configuration.md` → Loading order (Modify)

> 3. `OIDC_EXCHANGE__{section}__{key}` environment overrides apply on top of the merged TOML
>    and reach every config path, including map-valued sections —
>    `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` sets `providers.google.client_id`. A double
>    underscore separates path segments and each segment is lowercased; a single underscore
>    stays inside its segment (`…__MY_IDP__…` targets `providers.my_idp`), so keys whose names
>    themselves contain `__` cannot be addressed from the environment.
> 4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment
>    (used for secrets and per-deployment values). A placeholder that names an unset variable is
>    a startup error — a secret never silently degrades to its literal placeholder text. `$${`
>    escapes to a literal `${` and is never resolved.

### `.specs/service/specs/06-configuration.md` → Validation at load (Add)

> After merging and placeholder resolution, `load_config` validates the result and refuses to
> start on failure (`ConfigError`):
>
> - `server.role` must be one of `all` | `exchange` | `admin`.
> - `token.access_token_ttl` and `refresh_token_ttl` must parse as `<integer><s|m|h|d>` without
>   overflow; the parsed values are reused at request time, which therefore cannot fail.
> - Each `registration.domain_allowlist` entry must be an exact domain (`example.com`) or a
>   `*.`-prefixed wildcard (`*.example.com`). Bare `*` and dotless prefixes (`*example.com`)
>   are rejected.
> - When the internal API will be served (`role` is `admin` or `all` and
>   `internal_api.enabled = true`), `internal_api.shared_secret` must be present and non-empty.
>
> The same validation runs for config supplied as a string through the FFI bindings.

### `.specs/service/specs/06-configuration.md` → Sections → `[internal_api]` (Modify)

> `enabled` (false — internal routes are not mounted unless true, regardless of `server.role`;
> a `role = "admin"` instance with the flag off serves only `/health`), `auth_method`
> (`shared_secret`), `shared_secret` (redacted in `Debug`; must be non-empty when the internal
> API is served).

### `.specs/service/specs/04-http-api.md` → Middleware stack (Modify)

> Internal routes mount only when `internal_api.enabled = true` and the role is `admin` or
> `all`; with the flag false no internal routes are mounted regardless of role, so an
> `admin`-role instance serves only `/health`. When mounted, they additionally pass through
> **internal auth** (`middleware/internal_auth.rs`):
> `Authorization: Bearer <secret>` compared to `internal_api.shared_secret` in constant time
> (`subtle`); missing/wrong → `401`. A missing or empty secret is rejected at startup, never
> discovered at request time.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

> 2. `bootstrap::load_config` — load `config/default.toml`, overlay
>    `config/{OIDC_EXCHANGE_ENV}.toml` if set, apply `OIDC_EXCHANGE__{section}__{key}` env
>    overrides, resolve `${VAR}` placeholders, then validate (role, TTLs, allowlist, internal
>    API secret) ([06-configuration.md](06-configuration.md)).

---

## Type changes

None. No config fields are added or removed; `server.role` stays a string in TOML (validated,
optionally an enum in code).

---

## Implementation notes

1. Rewrite `load_config` (`crates/server/src/bootstrap.rs:26-47`): parse both TOMLs as
   `toml::Value` and deep-merge the env file over the default (tables merge recursively,
   scalars/arrays replace), then apply env overrides. The `config` crate is already a declared
   but unused dependency (`crates/server/Cargo.toml:20`) — its layered builder plus an
   `Environment` source (prefix `OIDC_EXCHANGE`, separator `__`) covers steps 1–3, including
   map-valued sections: the `__` separator yields nested keys, so
   `OIDC_EXCHANGE__PROVIDERS__GOOGLE__CLIENT_ID` lands on `providers.google.client_id`.
   Segments are lowercased; single underscores are preserved inside a segment, so ordinary
   provider names (`my_idp`) are addressable.
2. Placeholder resolution: post-merge walk over all string values replacing `${VAR}` from the
   environment; any unset variable → `ConfigError` (fail closed). `$${` is the escape for a
   literal `${`: it is never treated as a placeholder opener and is rewritten to `${` after
   resolution. No resolution code exists anywhere today (repo-wide grep).
3. Add `AppConfig::validate()` in `crates/core/src/config.rs`, called from `load_config` and
   from `parse_config` (`crates/server/src/bootstrap.rs:50-53`) so the FFI path
   (`crates/ffi/src/lib.rs:52`) validates identically.
4. Role: validate `server.role` (`crates/core/src/config.rs:29`) against `all|exchange|admin`;
   today a typo reaches `build_router` (`crates/server/src/bootstrap.rs:110-132`) and builds an
   empty router.
5. TTLs: `parse_duration_secs` (`crates/core/src/service/mod.rs:168-190`) — `split_at` on
   `s.len() - 1` panics when the last char is multi-byte (`:176`), and the multiplications are
   unchecked (`:183-185`). Fix both, and call it from `validate()` so the per-request call
   sites can rely on pre-validated values.
6. Allowlist: the matcher (`crates/core/src/service/exchange.rs:33-38`) turns `"*"` into an
   empty suffix that matches every domain, and `"*example.com"` into a dotless suffix that
   matches `evilexample.com`. Reject both shapes in `validate()`; only exact and `*.domain`
   entries pass.
7. Internal API: `internal_auth_layer` (`crates/server/src/middleware/internal_auth.rs:19-28`)
   treats `Some("")` as configured, and `internal_api.enabled` is read nowhere. Gate the
   `internal_routes` mount (`crates/server/src/bootstrap.rs:119-132`) on `enabled` and require
   a non-empty secret in `validate()`. With `enabled = false` nothing internal mounts
   regardless of role — a `role = "admin"` instance builds a router containing only `/health`
   (not a startup error).

---

## Merge plan

1. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- Config paths (`config/*.toml`) remain relative to the process working directory; deployments
  already arrange this.
- Existing example configs contain only well-formed allowlist entries and TTLs, so the new
  validation breaks no shipped example.

### Decisions

- _Fail closed on unresolved placeholders._ **An unset `${VAR}` aborts startup.** The literal
  placeholder must never become a live credential.
- _Validate once, at load._ **Request paths consume pre-validated config.** No per-request
  parsing that can panic or silently mis-match.
- _Disabled internal API mounts nothing._ **`internal_api.enabled = false` mounts no internal
  routes regardless of role, so a `role = "admin"` instance serves only `/health` — not a
  startup error.** A deliberate off switch is valid config, and `/health` keeps the instance
  observable.
- _Env overrides reach every path._ **`OIDC_EXCHANGE__…` overrides address map-valued sections
  too, splitting on `__` with segments lowercased.** Single underscores stay inside a segment,
  so ordinary provider names work; keys whose names contain `__` are simply unaddressable from
  the environment — a documented limitation beats a fragile matching rule.
- _Escapable placeholders._ **`$${` is the escape for a literal `${`; only unescaped `${VAR}`
  is resolved.** Fail-closed resolution needs a way to express literal placeholder text.

### Open questions

- (None at this stage.)
