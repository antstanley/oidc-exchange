# Change: Remove stale CloudTrail and atproto-as-shipped references

**Status:** Merged · **Date:** 2026-06-24 · **Merged:** 2026-06-29 · **Owner:** Ant Stanley · **Target:** docs/, examples/, config

Sweep the prose docs, example configs, and inline comments for references that no longer match
the code: the removed CloudTrail audit adapter (`adapter = "cloudtrail"`), audit adapters that
never existed (`file`, `webhook`), and atproto described as a shipped provider. Align them with
the implemented `noop`/`stdout`/`sqs` audit adapters and `oidc`/`apple` providers.

---

## Motivation

The CloudTrail audit adapter was replaced by the stdout/stderr and SQS adapters (commit
`407460c`), but `adapter = "cloudtrail"` and `[audit.cloudtrail]` blocks still appear in docs
and example configs, and `config/default.toml`'s comment lists `cloudtrail`/`file`/`webhook`
audit adapters that the code does not implement. atproto is similarly described as available in
the docs and the features list while no provider exists. These are documentation divergences the
canonical spec records as Open questions in
[06-configuration.md](../service/specs/06-configuration.md) and
[00-overview.md](../service/specs/00-overview.md).

Stale references mislead operators (a `cloudtrail` config silently fails to select a working
audit backend) and contradict the canonical spec. This change removes them.

---

## Affected spec pages

| Canonical page | Nature of change |
|---|---|
| [`.specs/service/specs/06-configuration.md`](../service/specs/06-configuration.md) | Remove the Open question about stale `cloudtrail` references once the sweep is done |

No canonical body change (the canonical spec already describes the correct adapters); this change
brings the non-spec docs/examples into line and clears the Open question.

---

## Proposed changes

This change edits documentation and configuration, not canonical spec bodies. The only canonical
edit is removing the resolved Open question:

### `.specs/service/specs/06-configuration.md` → Open questions (Remove)

> ~~The committed default lists only `noop`/`stdout`/`sqs` audit adapters; older docs mention a
> `cloudtrail`/`file`/`webhook` audit adapter that no longer exists. Doc and example configs
> should be swept for stale `adapter = "cloudtrail"` references.~~ (resolved by this change)

---

## Type changes

None.

---

## Implementation notes

1. Grep the repo for the stale tokens and fix each hit:
   - `rg -n 'cloudtrail' docs/ examples/ config/ apps/`
   - `rg -n 'adapter = "(file|webhook)"' -g '*.toml'` under `[audit]`
   - `rg -n 'atproto' docs/ apps/website` and reword any "supported/available" phrasing to
     "planned" (or remove) until the atproto change spec ships.
2. Replace example `[audit.cloudtrail]` blocks with a working `[audit]` choice
   (`adapter = "stdout"`, or `adapter = "sqs"` with `[audit.sqs]`).
3. Correct the `config/default.toml` audit-adapter comment to list only `noop`, `stdout`, `sqs`.
4. Update the README features list and `docs/` so atproto is described as planned, not shipped
   (until [2026-06-24-add_atproto_provider.md](2026-06-24-add_atproto_provider.md) lands).
5. Rebuild the website (`apps/website`) if the docs symlink content changed.

---

## Merge plan

1. Remove the resolved Open question from 06-configuration; bump its `**Date:**`.
2. No schema change.
3. Flip `**Status:**` to `Merged`, stamp `**Merged:**`, move to `.specs/changes/merged/`.
4. Update `.specs/README.md`.

---

## Assumptions and open questions

### Assumptions

- No deployment currently relies on `adapter = "cloudtrail"` resolving to a real backend (it does
  not — it would fail adapter construction).

### Decisions

- *Docs follow code.* **The sweep aligns docs/examples to the implemented adapters and
  providers.** The canonical spec is already correct; the non-spec material must not contradict
  it.

### Open questions

- Whether to keep a thin migration note for users who had `cloudtrail` configured (pointing them
  to `sqs` + a downstream CloudTrail ingestion) is undecided.
