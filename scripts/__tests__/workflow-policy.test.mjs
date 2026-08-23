import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import YAML from "yaml";
import { validateWorkflow, validateWorkflowFile } from "../workflow-policy.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const WORKFLOW_DIRECTORY = join(REPO_ROOT, ".github/workflows");
const FIXTURE_DIRECTORY = join(REPO_ROOT, "scripts/__tests__/fixtures/workflow-policy");

function fixture(name) {
  const text = readFileSync(join(FIXTURE_DIRECTORY, name), "utf8");
  assert.ok(text.length > 0);
  assert.ok(name.endsWith(".yml"));
  return text;
}

test("all workflows parse and satisfy structural release policy", () => {
  const workflowNames = readdirSync(WORKFLOW_DIRECTORY).filter((name) => name.endsWith(".yml"));
  assert.equal(workflowNames.length, 4);
  for (const workflowName of workflowNames) {
    assert.deepEqual(
      validateWorkflowFile(join(WORKFLOW_DIRECTORY, workflowName), workflowName),
      [],
    );
  }
});

test("positive fixture permits read-only locked validation before publishing", () => {
  assert.deepEqual(validateWorkflow(fixture("valid.yml"), "fixture.yml"), []);
});

test("negative fixtures reject each authority and tool-resolution violation", () => {
  const expected = new Map([
    ["workflow-write.yml", "workflow permissions forbidden"],
    ["unauthorized-write.yml", "unauthorized actions: write"],
    ["unpinned-action.yml", "action is not pinned"],
    ["persisted-checkout.yml", "checkout persists credentials"],
    ["dynamic-tool.yml", "dynamic command"],
    ["non-frozen-install.yml", "non-frozen pnpm install"],
    ["publish-bypass.yml", "publish-npm bypasses validation or artifacts"],
  ]);
  assert.equal(expected.size, 7);
  for (const [name, message] of expected) {
    const errors = validateWorkflow(fixture(name), "fixture.yml");
    assert.ok(errors.length > 0, `${name} must fail`);
    assert.ok(
      errors.some((error) => error.includes(message)),
      `${name}: ${errors.join(", ")}`,
    );
  }
});

test("workflow fixtures are valid YAML objects", () => {
  const names = readdirSync(FIXTURE_DIRECTORY).filter((name) => name.endsWith(".yml"));
  assert.equal(names.length, 8);
  for (const name of names) assert.equal(typeof YAML.parse(fixture(name)).jobs, "object");
});
