import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";
import YAML from "yaml";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const PNPM_VERSION = "11.9.0";
const PACKAGE_MANAGER = `pnpm@${PNPM_VERSION}`;
const WORKFLOWS = ["ci.yml", "release.yml", "nodejs-addon-glibc.yml"];

function read(relativePath) {
  const path = join(REPO_ROOT, relativePath);
  const value = readFileSync(path, "utf8");
  assert.ok(value.length > 0, `${relativePath} must not be empty`);
  assert.ok(path.startsWith(REPO_ROOT), `${relativePath} must remain inside the repository`);
  return value;
}

function manifest(relativePath) {
  const parsed = JSON.parse(read(relativePath));
  assert.equal(parsed.packageManager, PACKAGE_MANAGER);
  assert.equal(typeof parsed.scripts, "object");
  return parsed;
}

function frozenInstall(packageDirectory) {
  const result = spawnSync(
    "corepack",
    [
      `pnpm@${PNPM_VERSION}`,
      "--filter",
      `./${packageDirectory}`,
      "install",
      "--frozen-lockfile",
      "--ignore-scripts",
    ],
    { cwd: REPO_ROOT, encoding: "utf8", env: { ...process.env, CI: "true" } },
  );
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.equal(result.signal, null);
}

function mkdtempSync(prefix) {
  const result = spawnSync(
    process.execPath,
    ["-e", `process.stdout.write(require('fs').mkdtempSync(${JSON.stringify(prefix)}))`],
    { encoding: "utf8" },
  );
  assert.equal(result.status, 0);
  assert.ok(result.stdout.startsWith(prefix));
  return result.stdout;
}

test("package managers, lockfiles, and release-age policy are coherent", () => {
  const root = manifest("package.json");
  const node = manifest("bindings/nodejs/package.json");
  const lambda = manifest("bindings/lambda/package.json");
  const workspace = YAML.parse(read("pnpm-workspace.yaml"));

  assert.equal(root.packageManager, node.packageManager);
  assert.equal(node.packageManager, lambda.packageManager);
  assert.equal(workspace.minimumReleaseAge, 1440);
  assert.ok(workspace.minimumReleaseAgeExclude.includes("@oidc-exchange/*"));
  assert.equal(read("pnpm-lock.yaml"), read("bindings/nodejs/pnpm-lock.yaml"));
  assert.equal(read("pnpm-lock.yaml"), read("bindings/lambda/pnpm-lock.yaml"));
});

test("owned workflow installs use the exact frozen package manager", () => {
  let installCount = 0;
  for (const workflowName of WORKFLOWS) {
    const workflow = YAML.parse(read(`.github/workflows/${workflowName}`));
    assert.equal(typeof workflow.jobs, "object");
    for (const job of Object.values(workflow.jobs)) {
      assert.ok(Array.isArray(job.steps));
      for (const step of job.steps) {
        if (
          typeof step.run !== "string" ||
          !step.run.includes("pnpm") ||
          !step.run.includes("install")
        )
          continue;
        installCount += 1;
        assert.match(step.run, new RegExp(`corepack pnpm@${PNPM_VERSION.replaceAll(".", "\\.")}`));
        assert.match(step.run, /--frozen-lockfile/);
        assert.doesNotMatch(step.run, /--no-frozen-lockfile/);
      }
    }
  }
  assert.equal(installCount, 6);
});

test("isolated Node and Lambda frozen installs accept reviewed graphs", () => {
  frozenInstall("bindings/nodejs");
  frozenInstall("bindings/lambda");
});

test("frozen install rejects manifest drift without rewriting the lock", () => {
  const temporaryDirectory = mkdtempSync(join(tmpdir(), "oidc-exchange-stale-lock-"));
  try {
    const changedManifest = JSON.parse(read("bindings/lambda/package.json"));
    changedManifest.devDependencies.typescript = "6.0.2";
    writeFileSync(
      join(temporaryDirectory, "package.json"),
      `${JSON.stringify(changedManifest, null, 2)}\n`,
    );
    writeFileSync(
      join(temporaryDirectory, "pnpm-lock.yaml"),
      read("bindings/lambda/pnpm-lock.yaml"),
    );
    const before = readFileSync(join(temporaryDirectory, "pnpm-lock.yaml"), "utf8");
    const result = spawnSync(
      "corepack",
      [`pnpm@${PNPM_VERSION}`, "install", "--frozen-lockfile", "--ignore-scripts"],
      {
        cwd: temporaryDirectory,
        encoding: "utf8",
        env: { ...process.env, CI: "true" },
      },
    );
    assert.notEqual(result.status, 0);
    assert.match(`${result.stdout}\n${result.stderr}`, /ERR_PNPM_OUTDATED_LOCKFILE/);
    assert.equal(readFileSync(join(temporaryDirectory, "pnpm-lock.yaml"), "utf8"), before);
  } finally {
    rm(temporaryDirectory, { recursive: true, force: true });
  }
});
