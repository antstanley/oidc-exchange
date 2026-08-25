#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const POLICY_VERSION = 1;
export const SEVERITY_ORDER = new Map([
  ["unknown", 0],
  ["low", 1],
  ["moderate", 2],
  ["high", 3],
  ["critical", 4],
]);
const FINDING_KINDS = new Set(["vulnerability", "unmaintained", "yanked"]);
const ECOSYSTEMS = new Set(["cargo", "pnpm", "python"]);
const EXACT_VERSION = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;
const ADVISORY_ID = /^(?:RUSTSEC-\d{4}-\d{4}|GHSA-[0-9a-z-]+|PYSEC-\d{4}-\d+)$/;

function requireString(value, field) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${field} must be non-empty`);
  return value;
}

function parseDate(value, field) {
  requireString(value, field);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || Number.isNaN(Date.parse(`${value}T00:00:00Z`)))
    throw new Error(`${field} must be an ISO date`);
  return value;
}

export function validatePolicy(policy) {
  if (policy === null || typeof policy !== "object" || Array.isArray(policy))
    throw new Error("policy must be an object");
  if (policy.version !== POLICY_VERSION) throw new Error(`policy.version must be ${POLICY_VERSION}`);
  parseDate(policy.effective_date, "effective_date");
  if (!SEVERITY_ORDER.has(policy.severity_threshold)) throw new Error("invalid severity_threshold");
  if (policy.direct_and_transitive !== "same-policy")
    throw new Error("direct_and_transitive must be same-policy");
  for (const ecosystem of ECOSYSTEMS) {
    const graph = policy.ecosystems?.[ecosystem];
    if (!graph || graph.lockfile.length === 0) throw new Error(`${ecosystem} policy is required`);
    if (!SEVERITY_ORDER.has(graph.severity_threshold))
      throw new Error(`${ecosystem}.severity_threshold is invalid`);
    if (!['warn', 'fail'].includes(graph.unmaintained) || !['warn', 'fail'].includes(graph.yanked))
      throw new Error(`${ecosystem} lifecycle behavior is invalid`);
    requireString(graph.scanner.name, `${ecosystem}.scanner.name`);
    if (!EXACT_VERSION.test(graph.scanner.version))
      throw new Error(`${ecosystem}.scanner.version must be exact`);
  }
  if (!Array.isArray(policy.exceptions)) throw new Error("exceptions must be an array");
  const keys = new Set();
  for (const [index, exception] of policy.exceptions.entries()) {
    const prefix = `exceptions[${index}]`;
    if (!ECOSYSTEMS.has(exception.ecosystem)) throw new Error(`${prefix}.ecosystem is invalid`);
    if (!ADVISORY_ID.test(requireString(exception.advisory, `${prefix}.advisory`)))
      throw new Error(`${prefix}.advisory is invalid`);
    requireString(exception.package, `${prefix}.package`);
    if (!EXACT_VERSION.test(requireString(exception.version, `${prefix}.version`)))
      throw new Error(`${prefix}.version must be exact`);
    if (exception.range !== `=${exception.version}`) throw new Error(`${prefix}.range must be exact`);
    requireString(exception.rationale, `${prefix}.rationale`);
    requireString(exception.owner, `${prefix}.owner`);
    parseDate(exception.expires, `${prefix}.expires`);
    parseDate(exception.review_date, `${prefix}.review_date`);
    const key = `${exception.ecosystem}\0${exception.advisory}\0${exception.package}\0${exception.version}`;
    if (keys.has(key)) throw new Error(`${prefix} duplicates an exception`);
    keys.add(key);
  }
  return policy;
}

export function parseScannerOutput(ecosystem, text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`${ecosystem} scanner output is not valid JSON: ${error.message}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed))
    throw new Error(`${ecosystem} scanner output must be an object`);
  if (parsed.schema_version !== 1 || parsed.ecosystem !== ecosystem || parsed.complete !== true)
    throw new Error(`${ecosystem} scanner output is incomplete or has the wrong schema`);
  if (parsed.tool_status !== "ok") throw new Error(`${ecosystem} scanner tool failed`);
  if (!Array.isArray(parsed.findings)) throw new Error(`${ecosystem} findings must be an array`);
  return parsed.findings.map((finding, index) => {
    if (!FINDING_KINDS.has(finding.kind)) throw new Error(`finding ${index} has invalid kind`);
    if (!ADVISORY_ID.test(requireString(finding.advisory, `finding ${index}.advisory`)))
      throw new Error(`finding ${index} has invalid advisory`);
    requireString(finding.package, `finding ${index}.package`);
    if (!EXACT_VERSION.test(requireString(finding.version, `finding ${index}.version`)))
      throw new Error(`finding ${index}.version must be exact`);
    if (!SEVERITY_ORDER.has(finding.severity)) throw new Error(`finding ${index} severity is invalid`);
    return finding;
  });
}

export function evaluateFindings(policyInput, ecosystem, findings, today) {
  const policy = validatePolicy(policyInput);
  if (!ECOSYSTEMS.has(ecosystem)) throw new Error(`unsupported ecosystem ${ecosystem}`);
  parseDate(today, "today");
  const graph = policy.ecosystems[ecosystem];
  const threshold = SEVERITY_ORDER.get(graph.severity_threshold);
  const result = { allowed: [], warnings: [], failures: [] };
  for (const finding of findings) {
    const exception = policy.exceptions.find(
      (candidate) =>
        candidate.ecosystem === ecosystem &&
        candidate.advisory === finding.advisory &&
        candidate.package === finding.package &&
        candidate.version === finding.version,
    );
    if (exception) {
      if (exception.expires < today) result.failures.push({ ...finding, reason: "exception expired" });
      else result.allowed.push({ ...finding, exception });
      continue;
    }
    if (finding.kind === "unmaintained" || finding.kind === "yanked") {
      const behavior = graph[finding.kind];
      result[behavior === "fail" ? "failures" : "warnings"].push({
        ...finding,
        reason: `${finding.kind} policy is ${behavior}`,
      });
    } else if (SEVERITY_ORDER.get(finding.severity) >= threshold) {
      result.failures.push({ ...finding, reason: `severity ${finding.severity} meets ${graph.severity_threshold}` });
    } else {
      result.warnings.push({ ...finding, reason: "below severity threshold" });
    }
  }
  return result;
}

function main(argv) {
  if (argv.length !== 7 || argv[2] !== "evaluate") {
    console.error("usage: advisory-policy.mjs evaluate <policy.json> <ecosystem> <scanner-output.json> <YYYY-MM-DD>");
    return 2;
  }
  try {
    const policy = JSON.parse(readFileSync(resolve(argv[3]), "utf8"));
    const findings = parseScannerOutput(argv[4], readFileSync(resolve(argv[5]), "utf8"));
    const result = evaluateFindings(policy, argv[4], findings, argv[6]);
    for (const finding of result.allowed)
      console.log(`ALLOW ${argv[4]} ${finding.advisory} ${finding.package}@${finding.version} until ${finding.exception.expires}`);
    for (const finding of result.warnings)
      console.warn(`WARN ${argv[4]} ${finding.advisory} ${finding.package}@${finding.version}: ${finding.reason}`);
    for (const finding of result.failures)
      console.error(`FAIL ${argv[4]} ${finding.advisory} ${finding.package}@${finding.version}: ${finding.reason}`);
    console.log(JSON.stringify({ ecosystem: argv[4], counts: Object.fromEntries(Object.entries(result).map(([key, value]) => [key, value.length])) }));
    return result.failures.length === 0 ? 0 : 1;
  } catch (error) {
    console.error(`advisory policy error: ${error.message}`);
    return 2;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv);
