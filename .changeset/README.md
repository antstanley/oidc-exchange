# Changesets

This directory holds [changesets](https://github.com/changesets/changesets). Every
user-facing change ships with one:

```sh
pnpm changeset
```

Pick the impact (patch / minor / major) and write a one-line summary. That summary
becomes the changelog entry.

## One version across a polyglot repo

The published npm packages (`@oidc-exchange/node`, `@oidc-exchange/lambda`, and the
four platform packages) are a **`fixed` group** in `config.json`, so they always
share a version. That single version is also the canonical Rust workspace version.

Because changesets only understands npm packages, select **`@oidc-exchange/node`**
in a changeset for any release-worthy change (Rust, Python, or Node). On
`pnpm changeset:version`, `scripts/post-changeset-version.mjs` projects the resolved
version onto `Cargo.toml`, `bindings/python/pyproject.toml`, the lockfiles, and
`apps/website/src/versions.json`. `Cargo.toml` stays the source of truth for the
number; the release workflow's parity gate is the backstop.

See CONTRIBUTING.md → Releasing for the full flow.
