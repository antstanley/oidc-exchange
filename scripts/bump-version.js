#!/usr/bin/env node
// scripts/bump-version.js
//
// Bump every version-bearing manifest to a new release version and regenerate
// the Rust and Python lockfiles, so a subsequent `v<version>` tag push passes
// the release workflow's version-match gate (.github/workflows/release.yml, the
// `validate` job checks Cargo.toml, bindings/nodejs/package.json, and
// bindings/python/pyproject.toml against the tag).
//
// Usage:
//   pnpm bump-version <new-version>          # e.g. pnpm bump-version 0.1.2
//   node scripts/bump-version.js <version>
//
// What it changes:
//   - Cargo.toml                    [workspace.package] version   (+ Cargo.lock via `cargo update --workspace`)
//   - bindings/python/pyproject.toml  [project] version           (+ uv.lock via `uv lock`)
//   - bindings/nodejs/package.json    version + the four @oidc-exchange/* optionalDependencies
//   - bindings/nodejs/npm/<triple>/package.json   version         (x4 platform packages)
//   - bindings/lambda/package.json    version + the @oidc-exchange/node peerDependency
//
// What it deliberately does NOT touch:
//   - pnpm-lock.yaml — the self-published @oidc-exchange/* platform packages can
//     only enter the lockfile after they are published to npm. Refresh it in a
//     follow-up commit once the release is live (see CONTRIBUTING.md -> Releasing).
//   - git / jj state — it does not commit, tag, or push. Review the diff, then
//     follow the Releasing runbook in CONTRIBUTING.md.
//
// Edits are surgical string replacements (only the version substrings change),
// so file formatting is preserved and the oxfmt `format:check` gate stays green.

import { readFileSync, writeFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')

// Numeric-only major.minor.patch, with optional -prerelease and +build metadata.
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z-.]+)?(?:\+[0-9A-Za-z-.]+)?$/

// The four napi platform packages, kept in lockstep with @oidc-exchange/node.
const PLATFORMS = ['darwin-arm64', 'linux-arm64-gnu', 'linux-x64-gnu', 'win32-x64-msvc']

function die(msg) {
  console.error(`\x1b[31merror:\x1b[0m ${msg}`)
  process.exit(1)
}

const newVersion = process.argv[2]
if (!newVersion || newVersion === '-h' || newVersion === '--help') {
  console.log('usage: pnpm bump-version <new-version>   (e.g. pnpm bump-version 0.1.2)')
  process.exit(newVersion ? 0 : 1)
}
if (!SEMVER.test(newVersion)) {
  die(`"${newVersion}" is not a valid semantic version (expected e.g. 0.1.2)`)
}

// Current version — read from the Node package (source of truth for the JS side;
// the release gate requires every manifest to agree, so any of them would do).
const oldVersion = JSON.parse(readFileSync(join(repoRoot, 'bindings/nodejs/package.json'), 'utf8'))
  .version
if (!oldVersion) die('could not read the current version from bindings/nodejs/package.json')
if (oldVersion === newVersion) die(`version is already ${newVersion}; nothing to do`)

console.log(`Bumping ${oldVersion} -> ${newVersion}\n`)

const changed = []

// Replace literal `find` with `replace`, asserting an exact occurrence count so a
// drifted file (renamed key, extra pin) fails loudly instead of silently skipping.
function sub(relPath, replacements) {
  const path = join(repoRoot, relPath)
  let text = readFileSync(path, 'utf8')
  for (const { find, replace, count } of replacements) {
    const parts = text.split(find)
    const found = parts.length - 1
    if (found !== count) {
      die(`${relPath}: expected ${count} occurrence(s) of ${JSON.stringify(find)}, found ${found}`)
    }
    text = parts.join(replace)
  }
  writeFileSync(path, text)
  changed.push(relPath)
  console.log(`  ✓ ${relPath}`)
}

const v = () => ({ find: `"version": "${oldVersion}"`, replace: `"version": "${newVersion}"`, count: 1 })

// Rust workspace version (matched by release.yml's `grep '^version' | head -1`).
sub('Cargo.toml', [
  { find: `\nversion = "${oldVersion}"\n`, replace: `\nversion = "${newVersion}"\n`, count: 1 },
])

// Python project version.
sub('bindings/python/pyproject.toml', [
  { find: `\nversion = "${oldVersion}"\n`, replace: `\nversion = "${newVersion}"\n`, count: 1 },
])

// Node package: its own version plus the four exact-pinned platform optionalDependencies.
sub('bindings/nodejs/package.json', [
  v(),
  ...PLATFORMS.map((t) => ({
    find: `"@oidc-exchange/${t}": "${oldVersion}"`,
    replace: `"@oidc-exchange/${t}": "${newVersion}"`,
    count: 1,
  })),
])

// The four platform packages (version only).
for (const t of PLATFORMS) {
  sub(`bindings/nodejs/npm/${t}/package.json`, [v()])
}

// Lambda: its own version plus the @oidc-exchange/node peer floor. The `workspace:^`
// dependency entry is left untouched (it carries no version literal).
sub('bindings/lambda/package.json', [
  v(),
  {
    find: `"@oidc-exchange/node": "^${oldVersion}"`,
    replace: `"@oidc-exchange/node": "^${newVersion}"`,
    count: 1,
  },
])

// Regenerate lockfiles. Best-effort: a missing toolchain warns rather than aborts,
// so the manifest bumps still land and the user can regenerate by hand.
console.log('\nRegenerating lockfiles...')

function regen(label, cmd, args, cwd, hint) {
  try {
    execFileSync(cmd, args, { cwd, stdio: 'inherit' })
    changed.push(label)
    console.log(`  ✓ ${label}`)
  } catch (err) {
    const why = err.code === 'ENOENT' ? `${cmd} not found` : `${cmd} exited non-zero`
    console.warn(`  ! skipped ${label} (${why}) — run \`${hint}\` manually`)
  }
}

regen('Cargo.lock', 'cargo', ['update', '--workspace'], repoRoot, 'cargo update --workspace')
regen('bindings/python/uv.lock', 'uv', ['lock'], join(repoRoot, 'bindings/python'), 'cd bindings/python && uv lock')

// Summary + runbook.
console.log(`\nBumped ${changed.length} file(s) to ${newVersion}:`)
for (const f of changed) console.log(`  ${f}`)

console.log(`
Next steps (see CONTRIBUTING.md -> Releasing for the full runbook):
  1. Review the diff:            jj diff
  2. Commit on a bookmark:       jj describe -m "release: v${newVersion}" && jj bookmark set release/v${newVersion} && jj git push --bookmark release/v${newVersion}
  3. Open a PR and merge to main.
  4. Tag the merged commit:      git tag v${newVersion} <merge-commit> && git push origin v${newVersion}
     (pushing the tag triggers .github/workflows/release.yml)
  5. Approve the staged npm packages (npm publishing is staged).
  6. Once the packages are live, refresh pnpm-lock.yaml (pnpm install) and PR it to main.

pnpm-lock.yaml was left at ${oldVersion} on purpose — see the note in this script's header.`)
