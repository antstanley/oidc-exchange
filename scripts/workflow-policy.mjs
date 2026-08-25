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
  ["release.yml:publish-npm", new Set(["id-token"])],
  ["release.yml:publish-pypi", new Set(["id-token"])],
  ["release.yml:create-release", new Set(["contents"])],
]);

const ACTION_SHA = /^[^\s@]+@[0-9a-f]{40}$/;
const EXACT_STABLE_VERSION = /^\d+\.\d+\.\d+$/;
const MATURIN_ACTION = /^PyO3\/maturin-action@/;
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
      // pnpm 11 has no `--offline` flag for `exec`; the enforceable invariant
      // is that exec runs the pinned corepack pnpm (so the tool comes from the
      // frozen install's node_modules) and never a dynamic runner — dlx/npx
      // are rejected above as dynamic commands.
      if (step.run.includes("pnpm exec") && !/corepack pnpm@[\w.-]+ (?:--\S+ )*exec\b/.test(step.run))
        errors.push(`${key}: pnpm exec must use the pinned corepack invocation`);
      if (/\bpnpm(?:@[\w.-]+)?\s+exec\s+--offline\b/.test(step.run))
        errors.push(`${key}: pnpm 11 exec treats everything after exec as the command; --offline is not a flag there`);
      for (const command of step.run.matchAll(/\bcargo\s+install\s+cross\b[^\n]*/g)) {
        const version = /(?:^|\s)--version\s+(\S+)/.exec(command[0])?.[1];
        if (!version || !/^\d+\.\d+\.\d+$/.test(version))
          errors.push(`${key}: cargo install cross requires literal exact stable --version`);
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
      "actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a";
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
    const releaseNeeds = Array.isArray(releaseJob?.needs) ? releaseJob.needs : [releaseJob?.needs];
    if (!releaseNeeds.includes("build-binaries") || !releaseNeeds.includes("docker-manifest"))
      errors.push(`${workflowName}: release does not depend on attested artifacts`);
  }
  return errors;
}

export function validateWorkflowFile(path, workflowName) {
  const text = readFileSync(path, "utf8");
  assert.ok(text.length > 0);
  assert.ok(workflowName.endsWith(".yml"));
  return validateWorkflow(text, workflowName);
}
