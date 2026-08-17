# Change: Resolve `${VAR}` placeholders on every configuration entry point

**Status:** Merged · **Date:** 2026-08-05 · **Merged:** 2026-08-05 · **Owner:** Ant Stanley · **Target:** crates/server, bindings/* (service, bindings)

Route every configuration entry point through one shared resolve step — fail-closed `${VAR}`
placeholder resolution, `OIDC_EXCHANGE__{section}__{key}` overrides, then validation — so the
napi Node binding, the PyO3 Python binding and the `@oidc-exchange/lambda` handler stop loading
configuration in which a documented secret placeholder survives as literal text. Entry points
differ only in which *sources* they layer; nothing after the merge is duplicated. The change adds
an `oidc-exchange config check` subcommand so an operator can prove their environment satisfies a
config's placeholders before deploying it.

---

## Motivation

`crates/server/src/bootstrap.rs` has two config entry points with two pipelines.
`load_config_from_dir` (`:90-118`) layers the TOML sources, calls `resolve_placeholders`
(`:114`), deserializes, and validates. `parse_config` (`:124-128`) is two statements —
`toml::from_str` then `validate()` — and resolves nothing. `crates/ffi/src/lib.rs:52` calls
`parse_config`, and it is the sole config path for the napi addon
(`bindings/nodejs/src/lib.rs:46-63`), the PyO3 extension (`bindings/python/src/lib.rs:17-32`),
and the TypeScript Lambda handler that wraps the addon (`bindings/lambda/src/index.ts` →
`createHandler`). On those three published channels the placeholder the documentation tells
operators to use — *"Secrets (client secrets, API keys, KMS ARNs) should always use `${VAR_NAME}`
placeholders"*, `docs/guides/configuration.md:17` — is not a reference to a secret; it is the
value. `internal_api.shared_secret` becomes the string `${INTERNAL_API_SECRET}`, which this
repository prints verbatim in its own guides, and `internal_auth.rs` then compares a bearer token
against it in constant time — correctly, against the wrong string. `user_sync.webhook.secret`
becomes a published HMAC key. Anything that reads that config, or an error that echoes it, yields
a working credential. Evidence:
`g2-parse-config-placeholder-gap` (tracked in the security findings archive).

The fix is not three parallel patches. Adding a `resolve_placeholders` call to `parse_config`
restores parity between exactly today's two functions and leaves the next entry point — the
`config check` subcommand below, or any future embedding shape — equally free to become the
third divergent pipeline. The defect is that one documented contract has two implementations, and
the resolver's own doc comment (`:137-140`) asserted the invariant while nothing tested it across
both paths. What removes the class is a single resolve that owns everything after the source
merge, so an entry point's only remaining choice is which sources it layers, and there is no way
to obtain a config that did not pass through resolution. That is Option 2 of
the `config-closed-domain` hardening proposal (invariant CV2), and it is the half of that
proposal this change lands.

---

## Affected spec pages

| Canonical page | Nature of change |
| --- | --- |
| [`.specs/service/specs/06-configuration.md`](../../service/specs/06-configuration.md) | Modify Loading order into a source-layering list plus one shared resolve; reframe `Validation at load` from `load_config` onto the shared resolve; add `Placeholder resolution` (fail-closed table, residual guard, redaction rule), `Configuration entry points`, and `Pre-flight check (config check)` |
| [`.specs/service/specs/04-http-api.md`](../../service/specs/04-http-api.md) | Modify Bootstrap steps 1–2 for the `config check` subcommand and the shared resolve; modify the closing `crates/ffi` paragraph to cover configuration, not just routing |
| [`.specs/bindings/specs/01-ffi-core.md`](../../bindings/specs/01-ffi-core.md) | Modify Responsibilities: config through `new`/`from_file` passes the server's resolve; add a Decision recording the one-resolve/differing-sources rule |

No new canonical page. The `[providers.<name>]`, `[internal_api]` and Defaults-summary sections of
06-configuration are untouched — no TOML-visible field changes.

---

## Proposed changes

### `.specs/service/specs/06-configuration.md` → Loading order (Modify)

The section currently lists four numbered steps under the heading
`## Loading order (bootstrap::load_config)`. Replace the heading and the list with:

> ## Loading order
>
> Configuration reaches a running service through exactly one pipeline. An entry point chooses
> which *sources* it layers; everything after the merge is the shared resolve, which every entry
> point calls and none can bypass.
>
> Sources, lowest precedence first:
>
> 1. `config/default.toml` — compiled-in defaults (committed; see below).
> 2. `config/{OIDC_EXCHANGE_ENV}.toml` — deep-merged overlay when `OIDC_EXCHANGE_ENV` is set
>    (e.g. `production`, `sqlite-only`); tables merge recursively, scalars and arrays replace.
>    File-backed entry points only.
> 3. `OIDC_EXCHANGE__{section}__{key}` environment variables — structural overrides reaching
>    every config path, including map-valued sections. A double underscore separates path
>    segments and each segment is lowercased; a single underscore stays inside its segment
>    (`…__MY_IDP__…` targets `providers.my_idp`), so keys whose names themselves contain `__`
>    cannot be addressed from the environment.
>
> The shared resolve then runs over the merged tree, for every entry point:
>
> 4. `${VAR_NAME}` placeholders anywhere in the merged config are resolved from the environment
>    (see Placeholder resolution).
> 5. The resolved config is validated — `server.role`, the duration strings, allowlist entry
>    shape, and a non-empty `internal_api.shared_secret` when the internal API will be served
>    (see Validation at load) — and a failure aborts before any adapter or router is built.
>
> Steps 4 and 5 are one function. Deserializing the merged tree yields the raw config; only the
> resolve produces the `Config` the runtime consumes, so a code path that skipped resolution has
> nothing to hand `build_service`.

### `.specs/service/specs/06-configuration.md` → Validation at load (Modify)

The section's framing predates the shared resolve. It opens "After merging and placeholder
resolution, `load_config` validates the result and refuses to start on failure (`ConfigError`):"
and closes "The same validation runs for config supplied as a string through the FFI bindings
(`bootstrap::parse_config`)." Replace those two framing sentences — the list of checks between
them is unchanged — with:

> After merging and placeholder resolution, the shared resolve validates the result and refuses
> to produce a config on failure (`ConfigError`):

and

> Validation is a step of the shared resolve, so it runs identically on every entry point in
> Configuration entry points — including config supplied as a string through the FFI bindings
> (`bootstrap::parse_config`).

### `.specs/service/specs/06-configuration.md` → Placeholder resolution (Add)

> ## Placeholder resolution
>
> `${VAR_NAME}` in any string value, at any depth, is replaced with that environment variable's
> value. Resolution is total and fail-closed: every placeholder either resolves to a real value
> or aborts the load with a `ConfigError`. Literal placeholder text never reaches a running
> service.
>
> | Input | Outcome |
> | --- | --- |
> | `${NAME}`, `NAME` set to a non-empty value | replaced with the value |
> | `${NAME}`, `NAME` unset | `ConfigError` naming `NAME` and the config path; the load produces no config |
> | `${NAME}`, `NAME` set to the empty string | `ConfigError` naming `NAME`, worded to distinguish "set but empty" from "unset" |
> | `${` with no closing `}` within 256 bytes | `ConfigError` naming the config path and the unterminated opener |
> | `${}` — empty name | `ConfigError` naming the config path |
> | `$${` | the escape: rewritten to a literal `${`, never looked up in the environment |
>
> An empty variable is rejected rather than substituted because the fields this idiom exists for
> are the ones where an empty value means "no protection": an unpopulated secret-manager
> reference is a plumbing failure, not an operator's intent. A value that is genuinely meant to
> be empty is expressed by omitting the key (defaults apply) or by writing `""` in the TOML.
>
> After resolution, no config value may still contain an unescaped `${`. This holds as a
> post-condition on the resolved tree, so a value carrying placeholder text is a load failure
> whatever assembled it.
>
> Errors raised during resolution or validation name the environment variable and the config
> path, never the resolved value. `internal_api.shared_secret` and `user_sync.webhook.secret`
> stay redacted on every error and diagnostic path, exactly as they are in `Debug`.

### `.specs/service/specs/06-configuration.md` → Configuration entry points (Add)

> ## Configuration entry points
>
> | Entry point | Sources layered | Code |
> | --- | --- | --- |
> | Standalone server (hyper) | 1 + 2 + 3 | `crates/server/src/main.rs` → `bootstrap::load_config` |
> | Lambda runtime (same binary, `AWS_LAMBDA_RUNTIME_API` present) | 1 + 2 + 3 | `crates/server/src/main.rs` → `bootstrap::load_config` |
> | `config check` subcommand | 1 + 2 + 3, or a single named file | `crates/server/src/main.rs` |
> | FFI inline TOML (`OidcExchange::new`) | the supplied document + 3 | `crates/ffi/src/lib.rs` → `bootstrap::parse_config` |
> | FFI file (`OidcExchange::from_file`) | the named file + 3 | reads the file, then `new` |
> | Node binding (napi) | via the FFI entry points | `bindings/nodejs/src/lib.rs` |
> | Python binding (PyO3) | via the FFI entry points | `bindings/python/src/lib.rs` |
> | `@oidc-exchange/lambda` handler | via the Node binding | `bindings/lambda/src/index.ts` |
>
> Every row ends in the same resolve, so placeholder handling, override handling, and rejection
> behaviour are identical across channels. The `OIDC_EXCHANGE_ENV` overlay is the one legitimate
> difference: it applies only where the service selects its own files, and an FFI caller supplies
> the whole document, so there is nothing to overlay it onto.

### `.specs/service/specs/06-configuration.md` → Pre-flight check (Add)

> ## Pre-flight check (`oidc-exchange config check`)
>
> ```
> oidc-exchange config check [--dir <config-dir>] [--file <path>]
> ```
>
> `config check` layers the sources for the shape being checked — `--dir` (default `config/`) for
> the server layering, `--file` for the single-document layering the bindings use — runs the same
> resolve, and exits without constructing an adapter, binding a socket, or writing anything.
> Exit `0` prints a summary of the resolved configuration with every secret-bearing field
> rendered through its redacting `Debug`; any `ConfigError` exits non-zero with the message the
> server would have printed at startup. It is the supported way to prove that a deployment's
> environment satisfies its placeholders before the deployment happens.

### `.specs/service/specs/04-http-api.md` → Bootstrap (Modify)

Steps 1 and 2 currently read "Honour `--version` …" and "`bootstrap::load_config` — load
`config/default.toml`, overlay … resolve `${VAR}` placeholders". Replace both with:

> 1. Handle the CLI surface and exit: `--version` prints the crate version; `config check`
>    layers configuration sources and runs the same resolve as step 2, prints a redacted summary,
>    and exits non-zero on any `ConfigError` without building adapters or binding a socket
>    (see the canonical 06-configuration Bootstrap contract).
> 2. `bootstrap::load_config` — layer `config/default.toml`, the
>    `config/{OIDC_EXCHANGE_ENV}.toml` overlay if set, and `OIDC_EXCHANGE__{section}__{key}` env
>    overrides, then run the shared resolve: fail-closed `${VAR}` placeholder resolution followed
>    by validation (see the canonical 06-configuration Bootstrap contract).

### `.specs/service/specs/04-http-api.md` → Bootstrap, closing paragraph (Modify)

The section ends "`crates/ffi` calls the same `build_service` / `build_router` path, so in-process
bindings get identical routing and middleware." Replace with:

> `crates/ffi` layers its own sources into the same resolve and then calls the same
> `build_service` / `build_router` path, so in-process bindings get identical configuration
> semantics, routing, and middleware.

### `.specs/bindings/specs/01-ffi-core.md` → Responsibilities (Modify)

Replace the first bullet:

> - Build an `AppService` and axum `Router` from a TOML config (re-using `crates/server`'s
>   bootstrap), and own the tokio `Runtime` that drives them. Config supplied through `new`
>   (inline string) or `from_file` passes the server's shared resolve —
>   `OIDC_EXCHANGE__{section}__{key}` overrides, fail-closed `${VAR}` placeholder resolution,
>   then validation ([06-configuration.md](../../service/specs/06-configuration.md) →
>   Loading order). An unresolvable placeholder or an invalid value is an `FfiError` at
>   construction; a literal `${…}` never reaches a running router.

### `.specs/bindings/specs/01-ffi-core.md` → Decisions (Add)

> - *One resolve, differing sources.* **FFI config passes through the server's resolve; only the
>   source set differs — the supplied document plus `OIDC_EXCHANGE__…` overrides, with no
>   `OIDC_EXCHANGE_ENV` file overlay.** A second config pipeline is exactly how the published
>   Node, Python and Lambda packages came to load documented secret placeholders as literal text.

---

## Type changes

No `canonical-types.schema.json` change, and no TOML-visible field is added, removed, or
retyped. The two-stage split renames the deserialization target (the struct the merged tree
deserializes into) and introduces the resolved config type the runtime consumes; both mirror
today's `AppConfig` field-for-field. Narrowing the security-relevant fields to closed domain
types — `RegistrationMode`, `SigningAlgorithm`, `HttpsUrl`, `AsciiDomainPattern`, typed audit
severities — is the other half of the hardening proposal's Option 2 and is deliberately **not**
in this change; it hangs off the seam this change creates and is proposed separately in
the sibling `2026-08-05-fail_closed_across_config_and_adapters` change, which is outside this
workspace.

---

## Implementation notes

1. Factor the shared tail out of `load_config_from_dir` (`crates/server/src/bootstrap.rs:90-118`)
   into one function taking the assembled `config::ConfigBuilder`: `build()` →
   `resolve_placeholders(&mut merged.cache)` → deserialize → `validate()` → resolved config.
   This is the single resolve point; no other code path calls `try_deserialize` or `validate`
   directly. `load_config_from_dir` keeps only its source layering.
2. Rewrite `parse_config` (`:124-128`) to add a `config::File::from_str(toml_str,
   FileFormat::Toml)` source plus the same `Environment::with_prefix(ENV_OVERRIDE_PREFIX)
   .separator(ENV_OVERRIDE_SEPARATOR).try_parsing(true)` source used at `:107-111`, then call the
   shared tail. `File::from_str` is available in the `config` 0.15 release already depended on
   (`crates/server/Cargo.toml:20`). The standalone `toml::from_str` at `:125` goes away — it is
   the whole defect.
3. `OidcExchange::from_file` (`crates/ffi/src/lib.rs:75-81`) keeps reading the file and
   delegating to `new`, so there is no third path to keep in step.
4. Empty-variable rule: `std::env::var` returns `Ok("")` for a set-but-empty variable, so the
   lookup at `:186` currently substitutes an empty string silently. Add an explicit empty check
   with its own message so "unset" and "set but empty" are distinguishable in an operator's logs.
5. Malformed-placeholder rule: `scan_placeholder_name` (`:223-237`) returns `None` when no `}`
   appears within `PLACEHOLDER_NAME_LEN_MAX`, and the caller (`:184-197`) then falls through to
   copying `${` as ordinary text — the same fail-open shape as the finding. Make an unescaped
   `${` that fails to scan a `ConfigError`, and reject an empty name explicitly rather than
   letting `${}` reach `std::env::var("")`. The `ConfigError`s in the Placeholder-resolution
   table also name the config *path*; `resolve_placeholders` (`:141-160`) walks the tree without
   tracking keys today, so the walk gains a path argument threaded through the recursion.
6. Residual guard: after resolution, walk the tree once more and reject any value still holding
   an unescaped `${`. Redundant while resolution is total, and cheap insurance against a future
   source that bypasses it.
7. `config check`: extend the argument handling at `crates/server/src/main.rs:12-15` (the crate
   has no argument-parsing dependency today; adding one is a deliberate choice, not a
   requirement). It needs `load_config_from_dir` made `pub` (private at `:90`) plus a
   single-file variant. Print the summary through the redacting `Debug` impls on
   `InternalApiConfig::shared_secret` and `WebhookConfig::secret` (`crates/core/src/config.rs`),
   never the raw fields.
8. Tests, beside the existing placeholder tests (`crates/server/src/bootstrap.rs:1031-1200`):
   a **parity table** driven from one body over both entry points — set, unset, empty, escaped,
   unterminated, empty-name — asserting identical outcomes; a `parse_config` case resolving
   `shared_secret = "${INTERNAL_API_SECRET}"` with `assert_ne!` against the literal; a
   `parse_config` case applying `OIDC_EXCHANGE__REGISTRATION__MODE=existing_users_only`; and a
   `config check` case exiting non-zero on an unset variable while printing no secret value. The
   absence of a two-entry-point test is what let this through, so the parity table is the load-
   bearing one — a future third entry point has an obvious place to be added.
9. Release notes: this is a behaviour change for embedders. A binding host that constructs
   successfully today with an unset variable will fail at construction. It needs its own note in
   the `@oidc-exchange/node`, `@oidc-exchange/lambda` and PyPI changelogs, not only the server's.

---

## Merge plan

1. The earlier merge this step used to guard has completed: the `Proposed changes` blocks of
   the earlier complete-config-loading change are on
   the canonical pages — 06-configuration carries the `Validation at load` section and the
   fail-closed placeholder wording, and 04-http-api's Bootstrap step 2 and internal-route
   conditions are in place. The blocks above are written against that text as it now stands.
2. Apply each `Proposed changes` block to its canonical page; bump each page's `**Date:**` to the
   merge date.
3. No schema change to fold in.
4. Flip this file's `**Status:**` to `Merged`, add `**Merged:** YYYY-MM-DD`, and move it to
   `.specs/changes/merged/`.
5. Update `.specs/README.md`'s Change specs section — add this file when it is proposed, remove
   it from the pending list on merge.

---

## Assumptions and open questions

### Assumptions

- The Rust Lambda runtime is the same binary as the standalone server
  (`crates/server/src/main.rs:33` selects it after `load_config`), so it already resolves
  placeholders. The Lambda channel that does not is `@oidc-exchange/lambda`, which reaches
  configuration through the Node addon.
- No shipped example or test config depends on an unescaped `${` surviving as literal text; the
  `$${` escape covers any that later needs to.
- Config is read once at startup. If hot reload is ever added, these rules have to be
  re-established at reload time.

### Decisions

- *One resolve, not three patched call sites.* **Everything after the source merge lives in one
  function that every entry point calls.** Patching `parse_config` alone restores parity between
  two functions and leaves the next entry point free to diverge; making the source layering the
  only variable removes the class rather than the instance.
- *No permissive warning phase.* **Placeholder resolution fails closed from the first release
  that ships it, with no warn-and-continue window.** The hardening proposal's two-phase migration
  is right for narrowing field *types*, where a rejection is a new opinion about a working value.
  Here the permissive behaviour is the vulnerability: a warning that still loads
  `${INTERNAL_API_SECRET}` as a live credential is the bug with logging added.
- *Empty resolves are rejected.* **A placeholder naming a set-but-empty variable is a startup
  error, not an empty string.** The idiom exists for fields where empty means unprotected; the
  escape hatch is to omit the key or write `""` in the TOML.
- *Malformed placeholders are errors, not literals.* **An unterminated `${` or an empty `${}`
  aborts the load instead of being copied through as text.** Passing a malformed placeholder
  through verbatim is the same failure this change exists to remove, and `$${` is the documented
  way to write a literal.
- *Env overrides apply on the FFI path.* **`OIDC_EXCHANGE__…` overrides reach binding-supplied
  config too.** The documentation promises them unconditionally, and an operator who sets
  `OIDC_EXCHANGE__REGISTRATION__MODE=existing_users_only` on a binding runtime today gets no
  error and no effect.
- *`config check` ships with this change, not after it.* **The subcommand lands in the same
  change as the fail-closed rules.** Making a load fail where it used to succeed is only safe if
  an operator can find out before the deploy, and the subcommand is the cheapest piece of the
  proposal's Option 3.
- *Closed domain types are out of scope.* **This change builds the seam; it does not narrow the
  field types.** A change that both unifies the pipeline and retypes a dozen security-relevant
  fields is two changes, and only the first is a security fix.

### Open questions

- Should the FFI offer an opt-out from ambient environment overrides for embedders that want a
  hermetic config (the host already computed the values it wants)? Parity argues no; an embedder
  building config programmatically may reasonably not expect process env to override it.
- How should `config check --file` model the binding shape's environment? It can prove the
  placeholders resolve in the *checking* process's environment, which is not necessarily the
  Lambda or container environment the addon will run in.
- Does the empty-string rejection need a per-field opt-out for a value legitimately supplied as
  empty through the environment? No shipped config needs one today.
- Merge coordination: the sibling `2026-08-05-fail_closed_across_config_and_adapters` change
  also modifies 06-configuration's Loading order and rewrites `Validation at load`; whichever of
  the two merges second must refresh its Modify blocks against the merged page.
