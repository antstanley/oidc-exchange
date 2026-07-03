# Task 04 — Placeholder resolution

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-placeholder_resolution-certificate.md](04-placeholder_resolution-certificate.md)

**Implements:** [06-configuration.md](../../../service/specs/06-configuration.md) → Loading order step 4 (`${VAR}` placeholders resolved from the environment, fail-closed on an unset variable; `$${` escapes to a literal `${`)
**Depends on:** 03
**Produces:** a post-merge pass over the assembled config that replaces every `${VAR}` in a string value with the environment variable's value, aborts with `ConfigError` when a named variable is unset, and rewrites `$${` to a literal `${` without ever treating it as a placeholder opener.
**Pointers:** `crates/server/src/bootstrap.rs` `load_config` (attach the pass after the merge/override step from task 03, before deserialization into `AppConfig` or over the string fields of the merged value); no resolution code exists anywhere today (repo-wide grep confirmed in the change spec)

## Steps

- [x] Add a resolver that walks every string value in the merged config (e.g. over the `toml::Value`/`config::Value` tree before/at deserialization) and rewrites placeholders.
- [x] Treat `$${` as an escape: it is never a placeholder opener and is rewritten to a literal `${` after resolution; only an unescaped `${NAME}` is resolved.
- [x] Resolve `${NAME}` from `std::env`; an unset variable is a `ConfigError` (fail closed) whose `detail` names the missing variable — never leave the literal placeholder in place.
- [x] Wire the pass into `load_config` after the task-03 merge/override step so a resolved secret reaches the deserialized `AppConfig`.
- [x] Add tests: a set `${VAR}` resolves; an unset `${VAR}` returns `Err`; `$${` yields a literal `${`; a value with no placeholder is unchanged; a placeholder inside a nested/section value resolves.

## Definition of done

- [x] A `shared_secret = "${INTERNAL_API_SECRET}"` with the env var set yields the secret's value in the loaded config; with the var **unset**, `load_config` returns `Err` and no config is produced (the literal `${INTERNAL_API_SECRET}` never survives).
- [x] `$${INTERNAL_API_SECRET}` resolves to the literal string `${INTERNAL_API_SECRET}` and is never looked up in the environment.
- [x] Negative-space tests cover the unset-variable fail-closed path and the escape path; any bound (e.g. a scan limit, if introduced) is a named constant.
- [x] Meets the repo definition of done (tests, `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo nextest run --workspace` — see plan.md baseline).
- [x] Reviewable: load a config containing `${SET_VAR}`, `${UNSET_VAR}`, and `$${LITERAL}`; confirm the first resolves, the second aborts with an error naming `UNSET_VAR`, and the third becomes `${LITERAL}`.
