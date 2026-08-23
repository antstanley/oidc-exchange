#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const OUTPUT = resolve(process.env.ADVISORY_OUTPUT_DIR ?? resolve(ROOT, ".advisory-results"));
const PNPM_VERSION = "11.9.0";
const PIP_AUDIT_VERSION = "2.9.0";
const CARGO_DENY_VERSION = "0.19.0";
const TODAY = process.env.ADVISORY_POLICY_DATE ?? new Date().toISOString().slice(0, 10);
mkdirSync(OUTPUT, { recursive: true });

function run(command, args, cwd = ROOT) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "", error: result.error };
}

function requireVersion(command, args, expected) {
  const result = run(command, args);
  if (result.error || result.status !== 0 || !`${result.stdout}${result.stderr}`.includes(expected))
    throw new Error(`${command} ${expected} is not provisioned exactly`);
}

function writeReport(ecosystem, findings) {
  const report = { schema_version: 1, ecosystem, complete: true, tool_status: "ok", findings };
  const path = resolve(OUTPUT, `${ecosystem}.json`);
  writeFileSync(path, `${JSON.stringify(report, null, 2)}\n`);
  return path;
}

function packageVersion(span) {
  const firstLine = span.split("\n")[0];
  const match = /^(\S+) (\S+) /.exec(firstLine);
  if (!match) throw new Error(`cargo scanner package span malformed: ${span}`);
  return { package: match[1], version: match[2] };
}

function cargoFindings() {
  requireVersion("cargo-deny", ["--version"], CARGO_DENY_VERSION);
  const result = run("cargo", ["deny", "--manifest-path", resolve(ROOT, "Cargo.toml"), "--format", "json", "check", "advisories", "bans", "sources"]);
  if (result.error || (result.status !== 0 && result.status !== 1)) throw new Error("cargo-deny tool failure");
  const findings = [];
  let summary = false;
  for (const line of result.stderr.split("\n").filter(Boolean)) {
    let entry;
    try { entry = JSON.parse(line); } catch { throw new Error("cargo-deny emitted malformed JSON"); }
    const fields = entry.fields ?? {};
    if (entry.type === "summary" || entry.advisories || fields.advisories) summary = true;
    if (!["vulnerability", "unmaintained", "yanked", "advisory-not-detected"].includes(fields.code)) continue;
    const advisory = fields.advisory?.id ?? (fields.code === "yanked" ? "RUSTSEC-0000-0000" : null);
    const label = fields.labels?.[0]?.span;
    if (!advisory || !label) throw new Error("cargo-deny finding is incomplete");
    const identity = packageVersion(label);
    findings.push({ kind: fields.code, advisory, ...identity, severity: fields.code === "vulnerability" ? "high" : "unknown", direct: false });
  }
  if (findings.length === 0 && result.status !== 0) throw new Error("cargo-deny output omitted findings and summary");
  return findings;
}

function pnpmFindings() {
  requireVersion("corepack", [`pnpm@${PNPM_VERSION}`, "--version"], PNPM_VERSION);
  const result = run("corepack", [`pnpm@${PNPM_VERSION}`, "audit", "--recursive", "--json", "--prod", "--audit-level", "high"]);
  if (result.error || ![0, 1].includes(result.status)) throw new Error("pnpm audit tool or registry failure");
  let report;
  try { report = JSON.parse(result.stdout); } catch { throw new Error("pnpm audit emitted malformed JSON"); }
  if (!report || typeof report !== "object" || !report.metadata || !report.advisories)
    throw new Error("pnpm audit output is incomplete");
  return Object.values(report.advisories).flatMap((advisory) => {
    const id = advisory.github_advisory_id ?? advisory.cves?.[0];
    if (!id || !Array.isArray(advisory.findings)) throw new Error("pnpm advisory is incomplete");
    return advisory.findings.map((finding) => ({
      kind: "vulnerability", advisory: id, package: advisory.module_name,
      version: finding.version, severity: advisory.severity, direct: Boolean(finding.paths?.some((path) => !path.includes(">"))),
    }));
  });
}

function pythonFindings() {
  requireVersion("pip-audit", ["--version"], PIP_AUDIT_VERSION);
  const result = run("uv", ["export", "--frozen", "--no-dev", "--no-emit-project", "--format", "requirements-txt", "--output-file", resolve(OUTPUT, "python-requirements.txt")], resolve(ROOT, "bindings/python"));
  if (result.error || result.status !== 0) throw new Error("uv frozen export failed");
  const audit = run("pip-audit", ["--requirement", resolve(OUTPUT, "python-requirements.txt"), "--no-deps", "--disable-pip", "--format", "json", "--progress-spinner", "off"]);
  if (audit.error || ![0, 1].includes(audit.status)) throw new Error("pip-audit tool or vulnerability DB failure");
  let report;
  try { report = JSON.parse(audit.stdout); } catch { throw new Error("pip-audit emitted malformed JSON"); }
  if (!report || !Array.isArray(report.dependencies)) throw new Error("pip-audit output is incomplete");
  return report.dependencies.flatMap((dependency) => dependency.vulns.map((vulnerability) => ({
    kind: "vulnerability", advisory: vulnerability.id, package: dependency.name,
    version: dependency.version, severity: "high", direct: false,
  })));
}

try {
  const selected = process.argv.slice(2);
  const ecosystems = selected.length === 0 ? ["cargo", "pnpm", "python"] : selected;
  for (const ecosystem of ecosystems) {
    const findings = ecosystem === "cargo" ? cargoFindings() : ecosystem === "pnpm" ? pnpmFindings() : ecosystem === "python" ? pythonFindings() : (() => { throw new Error(`unknown ecosystem ${ecosystem}`); })();
    const path = writeReport(ecosystem, findings);
    const evaluation = run(process.execPath, [resolve(ROOT, "scripts/advisory-policy.mjs"), "evaluate", resolve(ROOT, "config/advisory-policy.json"), ecosystem, path, TODAY]);
    process.stdout.write(evaluation.stdout);
    process.stderr.write(evaluation.stderr);
    if (evaluation.status !== 0) process.exitCode = evaluation.status;
  }
} catch (error) {
  console.error(`advisory scan failed closed: ${error.message}`);
  process.exitCode = 2;
}
