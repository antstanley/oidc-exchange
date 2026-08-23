import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import YAML from "yaml";

export const ALLOWED_WRITE_PERMISSIONS = new Map([
  ["release.yml:build-docker", new Set(["packages"])],
  ["release.yml:docker-manifest", new Set(["packages"])],
  ["release.yml:publish-npm", new Set(["id-token"])],
  ["release.yml:publish-pypi", new Set(["id-token"])],
  ["release.yml:create-release", new Set(["contents"])],
]);

const ACTION_SHA = /^[^\s@]+@[0-9a-f]{40}$/;
const DYNAMIC_COMMANDS = [
  /\bnpx\b/,
  /\bpnpm\s+(?:dlx|add\s+-g)\b/,
  /\bnpm\s+install\s+-g\b/,
  /@latest\b/,
  /curl[^\n|]*\|\s*(?:ba)?sh\b/,
];

export function validateWorkflow(text, workflowName) {
  const workflow = YAML.parse(text);
  const errors = [];
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
      if (step.run.includes("pnpm exec") && !step.run.includes("--offline"))
        errors.push(`${key}: pnpm exec must be offline`);
    }
  }
  const publishNpm = workflow.jobs["publish-npm"];
  if (publishNpm) {
    const needs = Array.isArray(publishNpm.needs) ? publishNpm.needs : [publishNpm.needs];
    if (!needs.includes("validate-npm-package") || !needs.includes("build-nodejs"))
      errors.push(`${workflowName}: publish-npm bypasses validation or artifacts`);
  }
  return errors;
}

export function validateWorkflowFile(path, workflowName) {
  const text = readFileSync(path, "utf8");
  assert.ok(text.length > 0);
  assert.ok(workflowName.endsWith(".yml"));
  return validateWorkflow(text, workflowName);
}
