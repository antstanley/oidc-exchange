# Task 04: Document the shared-resolve contract and embedding break

**Plan:** [plan.md](../plan.md)
**Implements:** [source spec](../../../changes/2026-08-05-resolve_config_placeholders_all_channels.md) → Affected spec pages / Proposed changes / Merge plan / Implementation note 9
**Depends on:** 02, 03
**Produces:** canonical service/FFI documentation matching implementation, release notes for Node/Lambda/PyPI embedding users, and change-spec/README merge housekeeping.
**Pointers:** `.specs/service/specs/06-configuration.md`; `.specs/service/specs/04-http-api.md`; `.specs/bindings/specs/01-ffi-core.md`; `.specs/README.md`; release-note/changelog locations in `bindings/nodejs`, `bindings/lambda`, and `bindings/python` (identify existing project convention before editing).

## Steps

- [ ] Apply the source spec's precise 06-configuration changes: source layering vs one shared resolve; validation framing; Placeholder resolution table; Configuration entry points; and config-check preflight contract. Preserve untouched field/default sections.
- [ ] Apply the 04-http-api Bootstrap changes and closing FFI paragraph, and the FFI-core Responsibilities and one-resolve Decision exactly against the code shipped in tasks 01–03.
- [ ] Locate existing release-note/changelog conventions for `@oidc-exchange/node`, `@oidc-exchange/lambda`, and PyPI; add a concise behaviour-change note to each: an unresolved/empty/malformed placeholder now fails binding construction instead of becoming literal configuration text.
- [ ] Follow the source spec merge plan only after all implementation/tests are accepted: set the change spec status/merged date, move it to `changes/merged/`, and update README change-spec indexing. Do not create a canonical-type schema update.
- [ ] Update the Plans table in `.specs/README.md` to list this plan and keep its status synchronized with the kanban state.
- [ ] Verify every Markdown relative link, source/back-reference, table target, task dependency, and status after moving the source spec; ensure no done certificate is introduced.

## Definition of done

- [ ] 06-configuration, 04-http-api, and FFI-core state the one-resolve/differing-sources invariant, total fail-closed placeholder contract, env overrides, redaction, all entry points, and config-check behaviour exactly as implemented.
- [ ] Release notes cover Node, Lambda, and PyPI embedders; they explain the construction-time compatibility impact without exposing a real secret or inventing new API behaviour.
- [ ] Change spec merge housekeeping and README indexes are internally consistent; no schema file changes because no TOML-visible shape changed.
- [ ] All Markdown links resolve; tasks 01–04 remain DAG-valid with lower-number dependencies; all task checkboxes and plan status accurately reflect actual work state.
- [ ] Done certificates remain intentionally absent: no `done/` directory and no certificate file is created.
