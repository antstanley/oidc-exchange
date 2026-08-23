import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { evaluateFindings, parseScannerOutput, validatePolicy } from "../advisory-policy.mjs";

const ROOT = resolve(import.meta.dirname, "../..");
const POLICY = JSON.parse(readFileSync(resolve(ROOT, "config/advisory-policy.json"), "utf8"));
const DATE = "2026-08-23";

function policy(exception = null) {
  const copy = structuredClone(POLICY);
  copy.exceptions = exception ? [exception] : [];
  return copy;
}

function finding(overrides = {}) {
  return { kind: "vulnerability", advisory: "GHSA-aaaa-bbbb-cccc", package: "sample", version: "1.2.3", severity: "high", direct: false, ...overrides };
}

function exception(overrides = {}) {
  return { ecosystem: "cargo", advisory: "GHSA-aaaa-bbbb-cccc", package: "sample", version: "1.2.3", range: "=1.2.3", rationale: "Exact reviewed finding is temporarily accepted.", owner: "Security owner", expires: "2026-09-01", review_date: "2026-08-28", ...overrides };
}

for (const ecosystem of ["cargo", "pnpm", "python"]) {
  test(`${ecosystem}: clean graph passes`, () => {
    assert.deepEqual(evaluateFindings(policy(), ecosystem, [], DATE), { allowed: [], warnings: [], failures: [] });
  });
  test(`${ecosystem}: over-threshold direct and transitive findings fail`, () => {
    const result = evaluateFindings(policy(), ecosystem, [finding({ direct: true }), finding({ package: "transitive" })], DATE);
    assert.equal(result.failures.length, 2);
  });
  test(`${ecosystem}: exact allowed exception passes`, () => {
    const allowed = exception({ ecosystem });
    assert.equal(evaluateFindings(policy(allowed), ecosystem, [finding()], DATE).allowed.length, 1);
  });
  test(`${ecosystem}: expired exception fails`, () => {
    const expired = exception({ ecosystem, expires: "2026-08-22" });
    assert.match(evaluateFindings(policy(expired), ecosystem, [finding()], DATE).failures[0].reason, /expired/);
  });
  test(`${ecosystem}: wrong advisory and version do not match`, () => {
    const wrong = exception({ ecosystem, advisory: "GHSA-dddd-eeee-ffff", version: "1.2.4", range: "=1.2.4" });
    assert.equal(evaluateFindings(policy(wrong), ecosystem, [finding()], DATE).failures.length, 1);
  });
  test(`${ecosystem}: yanked and unmaintained findings warn`, () => {
    const findings = [finding({ kind: "yanked", severity: "unknown" }), finding({ kind: "unmaintained", advisory: "GHSA-dddd-eeee-ffff", severity: "unknown" })];
    assert.equal(evaluateFindings(policy(), ecosystem, findings, DATE).warnings.length, 2);
  });
  test(`${ecosystem}: malformed, missing, and failed scanner output fail closed`, () => {
    assert.throws(() => parseScannerOutput(ecosystem, "{"), /not valid JSON/);
    assert.throws(() => parseScannerOutput(ecosystem, JSON.stringify({ schema_version: 1, ecosystem, complete: true, tool_status: "ok" })), /findings/);
    assert.throws(() => parseScannerOutput(ecosystem, JSON.stringify({ schema_version: 1, ecosystem, complete: true, tool_status: "failed", findings: [] })), /tool failed/);
  });
}

test("policy requires exact non-wildcard exception versions and complete ownership", () => {
  assert.throws(() => validatePolicy(policy(exception({ version: "1.*", range: "=1.*" }))), /version must be exact/);
  assert.throws(() => validatePolicy(policy(exception({ owner: "" }))), /owner must be non-empty/);
});

test("committed policy has exact scanners and bounded current exceptions", () => {
  validatePolicy(POLICY);
  assert.deepEqual(Object.fromEntries(Object.entries(POLICY.ecosystems).map(([name, graph]) => [name, graph.scanner.version])), { cargo: "0.19.0", pnpm: "11.9.0", python: "2.9.0" });
  assert.equal(POLICY.exceptions.length, 18);
});
