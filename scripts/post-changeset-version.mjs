#!/usr/bin/env node
// scripts/post-changeset-version.mjs
//
// Runs immediately after `changeset version` (see the root `changeset:version`
// script). Changesets bumps only the npm packages (bindings/nodejs, bindings/lambda,
// the four platform packages) and writes their CHANGELOG.md files. This script
// projects that single resolved version onto the rest of the polyglot repo so a
// later `v<version>` tag passes the release workflow's parity gate
// (.github/workflows/release.yml `validate`: Cargo.toml == node == python == tag):
//
//   - Cargo.toml            [workspace.package] version   (+ Cargo.lock via cargo update)
//   - bindings/python/pyproject.toml  [project] version   (+ uv.lock via uv lock)
//   - pnpm-lock.yaml (refreshed) mirrored into the two binding copies
//   - apps/website/src/data/versions.json  (new { version, date } prepended)
//
// Cargo.toml stays the canonical source of the number. Idempotent: a no-op when the
// versions already agree (e.g. no pending changesets).

import { readFileSync, writeFileSync, existsSync, copyFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const R = (p) => join(repoRoot, p)

function die(msg) {
  console.error(`\x1b[31merror:\x1b[0m ${msg}`)
  process.exit(1)
}

// The npm packages are already bumped by `changeset version`; read the new number.
const newVersion = JSON.parse(readFileSync(R('bindings/nodejs/package.json'), 'utf8')).version
if (!newVersion) die('could not read the new version from bindings/nodejs/package.json')

// Current Rust workspace version (the number release.yml greps as `^version`).
const cargoText = readFileSync(R('Cargo.toml'), 'utf8')
const oldVersion = /^\s*version\s*=\s*"([^"]+)"/m.exec(cargoText)?.[1]
if (!oldVersion) die('could not read [workspace.package] version from Cargo.toml')

// Surgical string replacement with an exact occurrence-count assertion, so a drifted
// file fails loudly instead of silently skipping.
function sub(relPath, find, replace, count = 1) {
  const path = R(relPath)
  const text = readFileSync(path, 'utf8')
  const parts = text.split(find)
  if (parts.length - 1 !== count) {
    die(`${relPath}: expected ${count} occurrence(s) of ${JSON.stringify(find)}, found ${parts.length - 1}`)
  }
  writeFileSync(path, parts.join(replace))
  console.log(`  ✓ ${relPath}`)
}

function run(label, cmd, args, cwd) {
  try {
    execFileSync(cmd, args, { cwd, stdio: 'inherit' })
    console.log(`  ✓ ${label}`)
  } catch (err) {
    if (err.code === 'ENOENT') {
      console.warn(`  ! skipped ${label}: ${cmd} not found — regenerate it before tagging`)
      return
    }
    die(`${label} failed: ${err.message}`)
  }
}

if (oldVersion !== newVersion) {
  console.log(`Projecting ${oldVersion} -> ${newVersion} onto Rust + Python:\n`)
  sub('Cargo.toml', `\nversion = "${oldVersion}"\n`, `\nversion = "${newVersion}"\n`)
  sub('bindings/python/pyproject.toml', `\nversion = "${oldVersion}"\n`, `\nversion = "${newVersion}"\n`)
  run('Cargo.lock', 'cargo', ['update', '--workspace'], repoRoot)
  run('bindings/python/uv.lock', 'uv', ['lock'], R('bindings/python'))
} else {
  console.log(`Rust/Python already at ${newVersion}; skipping manifest projection.`)
}

// Refresh the root pnpm lockfile (npm package versions changed) and mirror it into
// the two binding copies the supply-chain invariant keeps byte-identical.
run('pnpm-lock.yaml', 'corepack', ['pnpm@11.9.0', 'install', '--no-frozen-lockfile'], repoRoot)
for (const copy of ['bindings/nodejs/pnpm-lock.yaml', 'bindings/lambda/pnpm-lock.yaml']) {
  copyFileSync(R('pnpm-lock.yaml'), R(copy))
  console.log(`  ✓ ${copy} (mirrored)`)
}

// Prepend the new version to the docs versions manifest (consumed by the Versions page).
const versionsPath = R('apps/website/src/versions.json')
const versions = existsSync(versionsPath) ? JSON.parse(readFileSync(versionsPath, 'utf8')) : []
if (!versions.some((v) => v.version === newVersion)) {
  const date = new Date().toISOString().slice(0, 10)
  versions.unshift({ version: newVersion, date })
  writeFileSync(versionsPath, `${JSON.stringify(versions, null, 2)}\n`)
  console.log(`  ✓ apps/website/src/data/versions.json (prepended ${newVersion})`)
}

console.log(`\nDone. Review the diff, then merge the Version Packages PR and tag it.`)
