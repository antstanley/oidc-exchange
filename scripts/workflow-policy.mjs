import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import YAML from "yaml";

const ROOT = resolve(import.meta.dirname, "..");
const PYPROJECT = resolve(ROOT, "bindings/python/pyproject.toml");
const UV_LOCK = resolve(ROOT, "bindings/python/uv.lock");

export const ALLOWED_WRITE_PERMISSIONS = new Map([
  ["release.yml:build-binaries", new Set(["id-token", "attestations"])],
  ["release.yml:build-docker", new Set(["packages", "id-token", "attestations"])],
  ["release.yml:docker-manifest", new Set(["packages", "id-token", "attestations"])],
  ["release.yml:publish-downloads", new Set(["id-token"])],
  ["release.yml:publish-latest", new Set(["id-token"])],
  ["release.yml:publish-npm", new Set(["id-token"])],
  ["release.yml:publish-pypi", new Set(["id-token"])],
  ["release.yml:create-release", new Set(["contents"])],
  // Docs publishing to oidc-exchange.iamstan.dev via blogwright: keyless AWS
  // deploys assume the GitHub OIDC role (id-token), and PR previews comment their
  // URL back on the pull request (pull-requests). No other write scope is granted.
  // Changesets "Version Packages" bot: pushes the version commit and opens its PR via
  // the GitHub API. Checkout still sets persist-credentials: false.
  ["version-packages.yml:version", new Set(["contents", "pull-requests"])],
  // Generates .changeset/auto-pr-<n>.md on same-repo PR branches through the contents API
  // (pull_request_target; the base branch script runs, PR code is never checked out).
  ["auto-changeset.yml:changeset", new Set(["contents"])],
  ["production.yml:deploy", new Set(["id-token"])],
  ["preview.yml:deploy", new Set(["id-token", "pull-requests"])],
  ["preview.yml:destroy", new Set(["id-token", "pull-requests"])],
]);

const ACTION_SHA = /^[^\s@]+@[0-9a-f]{40}$/;
const EXACT_STABLE_VERSION = /^\d+\.\d+\.\d+$/;
const MATURIN_ACTION = /^PyO3\/maturin-action@/;
const COSIGN_ACTION = /^sigstore\/cosign-installer@/;
const EXACT_COSIGN_RELEASE = /^v\d+\.\d+\.\d+$/;
const DYNAMIC_COMMANDS = [
  /\bnpx\b/,
  /\bpnpm\s+(?:dlx|add\s+-g)\b/,
  /\bnpm\s+install\s+-g\b/,
  /@latest\b/,
  /curl[^\n|]*\|\s*(?:ba)?sh\b/,
];

function auditedMaturinVersion() {
  const pyproject = readFileSync(PYPROJECT, "utf8");
  const buildSystem = /^requires = \["maturin==([^"\]]+)"\]$/m.exec(pyproject)?.[1];
  const buildGroup = /^\s*"maturin==([^"\]]+)",?$/m.exec(pyproject)?.[1];
  const lock = readFileSync(UV_LOCK, "utf8");
  const lockedBuildGroup = /^build = \[\{ name = "maturin", specifier = "==([^"]+)" \}\]$/m.exec(lock)?.[1];
  assert.ok(buildSystem, "pyproject build-system maturin pin is required");
  assert.ok(buildGroup, "pyproject build group maturin pin is required");
  assert.ok(lockedBuildGroup, "locked build group maturin pin is required");
  assert.equal(buildSystem, buildGroup, "pyproject maturin pins must match");
  assert.equal(buildSystem, lockedBuildGroup, "locked build group maturin pin must match");
  assert.match(buildSystem, EXACT_STABLE_VERSION, "maturin pin must be exact and stable");
  return buildSystem;
}

export function validateWorkflow(text, workflowName) {
  const workflow = YAML.parse(text);
  const errors = [];
  const maturinVersion = auditedMaturinVersion();
  assert.equal(typeof workflow.jobs, "object");
  assert.ok(Object.keys(workflow.jobs).length > 0);

  if (workflow.permissions !== undefined)
    errors.push(`${workflowName}: workflow permissions forbidden`);
  for (const [jobName, job] of Object.entries(workflow.jobs)) {
    const key = `${workflowName}:${jobName}`;
    const permissions = job.permissions;
    if (typeof permissions !== "object" || permissions === null) {
      errors.push(`${key}: explicit job permissions required`);
      continue;
    }
    for (const [scope, access] of Object.entries(permissions)) {
      if (access === "write" && !ALLOWED_WRITE_PERMISSIONS.get(key)?.has(scope)) {
        errors.push(`${key}: unauthorized ${scope}: write`);
      }
    }
    assert.ok(Array.isArray(job.steps));
    for (const step of job.steps) {
      if (step.uses && !ACTION_SHA.test(step.uses))
        errors.push(`${key}: action is not pinned: ${step.uses}`);
      if (MATURIN_ACTION.test(String(step.uses ?? ""))) {
        const version = step.with?.["maturin-version"];
        if (typeof version !== "string" || !EXACT_STABLE_VERSION.test(version))
          errors.push(`${key}: maturin-action requires literal exact stable maturin-version`);
        else if (version !== maturinVersion)
          errors.push(`${key}: maturin-action version ${version} does not match audited maturin ${maturinVersion}`);
      }
      if (COSIGN_ACTION.test(String(step.uses ?? ""))) {
        const release = step.with?.["cosign-release"];
        if (typeof release !== "string" || !EXACT_COSIGN_RELEASE.test(release))
          errors.push(`${key}: cosign-installer requires literal exact stable cosign-release`);
      }
      if (String(step.uses ?? "").startsWith("actions/checkout@")) {
        if (permissions.contents !== "read" && permissions.contents !== "write")
          errors.push(`${key}: checkout lacks contents scope`);
        if (step.with?.["persist-credentials"] !== false)
          errors.push(`${key}: checkout persists credentials`);
      }
      if (typeof step.run !== "string") continue;
      for (const pattern of DYNAMIC_COMMANDS)
        if (pattern.test(step.run)) errors.push(`${key}: dynamic command ${pattern}`);
      if (
        /\bpnpm(?:@[\w.-]+)?\s+install\b/.test(step.run) &&
        !step.run.includes("--frozen-lockfile")
      )
        errors.push(`${key}: non-frozen pnpm install`);
      // Node 25+ no longer bundles corepack; pnpm comes from a pinned
      // pnpm/action-setup step with a literal exact version.
      if (/\bcorepack\b/.test(step.run))
        errors.push(`${key}: corepack is not available on Node 26; use pnpm/action-setup`);
      // pnpm has no `--offline` flag for `exec`; the enforceable invariant is
      // that exec runs the exact pnpm provisioned by pnpm/action-setup (so the
      // tool comes from the frozen install's node_modules) and never a dynamic
      // runner — dlx/npx are rejected above as dynamic commands.
      if (/\bpnpm\b/.test(step.run)) {
        const setup = job.steps.find((s) => String(s.uses ?? "").startsWith("pnpm/action-setup@"));
        if (!setup || typeof setup.with?.version !== "string" || !EXACT_STABLE_VERSION.test(setup.with.version))
          errors.push(`${key}: pnpm requires a pnpm/action-setup step with a literal exact version`);
      }
      if (/\bpnpm(?:@[\w.-]+)?\s+exec\s+--offline\b/.test(step.run))
        errors.push(`${key}: pnpm 11 exec treats everything after exec as the command; --offline is not a flag there`);
      for (const command of step.run.matchAll(/\bcargo\s+install\s+cross\b[^\n]*/g)) {
        const version = /(?:^|\s)--version\s+(\S+)/.exec(command[0])?.[1];
        if (!version || !/^\d+\.\d+\.\d+$/.test(version))
          errors.push(`${key}: cargo install cross requires literal exact stable --version`);
      }
      // cargo-binstall must install literal exact stable crate versions from
      // crate-published (author-controlled) binaries only: no source-compile
      // fallback and no third-party QuickInstall mirror.
      const joined = step.run.replace(/\\\r?\n\s*/g, " ");
      const VALUE_FLAGS = new Set(["--strategies", "--disable-strategies", "--targets", "--pkg-url", "--bin-dir", "--pkg-fmt", "--version", "--root", "--manifest-path", "--index", "--git"]);
      for (const command of joined.matchAll(/\bcargo\s+binstall\b[^\n]*/g)) {
        const tokens = command[0].match(/'[^']*'|"[^"]*"|\S+/g).slice(2);
        const crates = [];
        const flags = new Map();
        for (let i = 0; i < tokens.length; i += 1) {
          if (tokens[i].startsWith("--")) {
            if (VALUE_FLAGS.has(tokens[i])) { flags.set(tokens[i], tokens[i + 1]); i += 1; } else flags.set(tokens[i], true);
          } else crates.push(tokens[i]);
        }
        if (crates.length === 0 || !crates.every((crate) => /^[\w-]+@\d+\.\d+\.\d+$/.test(crate)))
          errors.push(`${key}: cargo binstall requires literal exact stable crate@version`);
        if (flags.get("--strategies") !== "crate-meta-data" || flags.has("--disable-strategies"))
          errors.push(`${key}: cargo binstall must use --strategies crate-meta-data (no compile or QuickInstall fallback)`);
      }
    }
  }
  const publishNpm = workflow.jobs["publish-npm"];
  if (publishNpm) {
    const needs = Array.isArray(publishNpm.needs) ? publishNpm.needs : [publishNpm.needs];
    if (!needs.includes("validate-npm-package") || !needs.includes("build-nodejs"))
      errors.push(`${workflowName}: publish-npm bypasses validation or artifacts`);
  }

  if (workflowName === "release.yml") {
    const attestAction =
      "actions/attest-build-provenance@4d101475d8b20a2381f78447822ac1eab6504dd8";
    const binaryJob = workflow.jobs["build-binaries"];
    const dockerJob = workflow.jobs["build-docker"];
    const manifestJob = workflow.jobs["docker-manifest"];
    const releaseJob = workflow.jobs["create-release"];
    const dependencyPolicy = workflow.jobs["dependency-policy"];
    if (dependencyPolicy || workflow.jobs.validate) {
      if (!dependencyPolicy) errors.push(`${workflowName}: missing release dependency policy`);
      const validateNeeds = Array.isArray(workflow.jobs.validate?.needs)
        ? workflow.jobs.validate.needs
        : [workflow.jobs.validate?.needs];
      if (!validateNeeds.includes("dependency-policy"))
        errors.push(`${workflowName}: validate bypasses dependency policy`);
    }
    for (const [name, job] of [
      ["build-binaries", binaryJob],
      ["build-docker", dockerJob],
      ["docker-manifest", manifestJob],
    ]) {
      if (!job) {
        errors.push(`${workflowName}: missing ${name} attestation job`);
        continue;
      }
      if (job.permissions?.["id-token"] !== "write" || job.permissions?.attestations !== "write")
        errors.push(`${workflowName}:${name}: missing attestation permissions`);
      if (!job.steps.some((step) => step.uses === attestAction))
        errors.push(`${workflowName}:${name}: missing pinned provenance action`);
    }
    const binaryAttest = binaryJob?.steps.find((step) => step.uses === attestAction);
    if (!String(binaryAttest?.with?.["subject-checksums"] ?? "").includes("matrix.artifact"))
      errors.push(`${workflowName}: binary attestation does not consume produced checksum`);
    const dockerAttest = dockerJob?.steps.find((step) => step.uses === attestAction);
    if (dockerAttest?.with?.["subject-digest"] !== "${{ steps.build.outputs.digest }}")
      errors.push(`${workflowName}: platform image attestation uses wrong digest`);
    if (/:[^@]+$/.test(String(dockerAttest?.with?.["subject-name"] ?? "")))
      errors.push(`${workflowName}: mutable image tag attested`);
    const manifestAttest = manifestJob?.steps.find((step) => step.uses === attestAction);
    if (manifestAttest?.with?.["subject-digest"] !== "${{ steps.manifest.outputs.digest }}")
      errors.push(`${workflowName}: manifest attestation uses wrong digest`);
    const downloadsJob = workflow.jobs["publish-downloads"];
    if (downloadsJob) {
      const downloadsNeeds = Array.isArray(downloadsJob.needs) ? downloadsJob.needs : [downloadsJob.needs];
      if (!downloadsNeeds.includes("build-binaries") || !downloadsNeeds.includes("validate"))
        errors.push(`${workflowName}: publish-downloads must follow build-binaries and validate`);
      if (!downloadsJob.steps.some((step) => COSIGN_ACTION.test(String(step.uses ?? ""))))
        errors.push(`${workflowName}: publish-downloads must install a pinned cosign`);
      if (downloadsJob.environment !== "downloads")
        errors.push(`${workflowName}: publish-downloads must run in the downloads environment`);
      if (downloadsJob.permissions?.contents !== "read")
        errors.push(`${workflowName}: publish-downloads holds contents: read only`);
    }
    const latestJob = workflow.jobs["publish-latest"];
    if (latestJob) {
      const latestNeeds = Array.isArray(latestJob.needs) ? latestJob.needs : [latestJob.needs];
      if (!latestNeeds.includes("publish-downloads"))
        errors.push(`${workflowName}: publish-latest must follow publish-downloads`);
      if (latestJob.environment !== "downloads")
        errors.push(`${workflowName}: publish-latest must run in the downloads environment`);
      if (latestJob.permissions?.contents !== "read")
        errors.push(`${workflowName}: publish-latest holds contents: read only`);
    }
    const releaseNeeds = Array.isArray(releaseJob?.needs) ? releaseJob.needs : [releaseJob?.needs];
    if (!releaseNeeds.includes("build-binaries") || !releaseNeeds.includes("docker-manifest"))
      errors.push(`${workflowName}: release does not depend on attested artifacts`);
    if (releaseNeeds.includes("publish-downloads") || releaseNeeds.includes("publish-latest"))
      errors.push(`${workflowName}: create-release must not depend on the download host jobs`);
  }
  return errors;
}

export function validateWorkflowFile(path, workflowName) {
  const text = readFileSync(path, "utf8");
  assert.ok(text.length > 0);
  assert.ok(workflowName.endsWith(".yml"));
  return validateWorkflow(text, workflowName);
}
