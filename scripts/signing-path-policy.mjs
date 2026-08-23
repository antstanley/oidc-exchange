#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EXACT_IDENTITY = /^([^@\s]+)@(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$/;
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

function identity(name, version) { return `${name}@${version}`; }
function prerelease(version) { return version.split("+")[0].includes("-"); }
function date(value, field) {
  if (typeof value !== "string" || !ISO_DATE.test(value) || Number.isNaN(Date.parse(`${value}T00:00:00Z`))) throw new Error(`${field} must be an ISO date`);
}

export function validateSigningPolicy(policy) {
  if (!policy || policy.version !== 1) throw new Error("signing policy version must be 1");
  date(policy.effective_date, "effective_date");
  if (!Array.isArray(policy.modes) || policy.modes.length === 0) throw new Error("modes are required");
  if (!Array.isArray(policy.protected_packages) || policy.protected_packages.length === 0) throw new Error("protected_packages are required");
  const modes = new Set();
  for (const mode of policy.modes) {
    if (typeof mode.name !== "string" || modes.has(mode.name)) throw new Error("mode name must be unique");
    modes.add(mode.name);
    if (!Array.isArray(mode.roots) || mode.roots.length === 0 || mode.roots.some((root) => !EXACT_IDENTITY.test(root))) throw new Error(`${mode.name} roots must be exact identities`);
    if (!Array.isArray(mode.metadata_args) || !mode.metadata_args.includes("--locked")) throw new Error(`${mode.name} metadata must be locked`);
  }
  if (!Array.isArray(policy.exceptions)) throw new Error("exceptions are required");
  for (const [index, exception] of policy.exceptions.entries()) {
    if (!modes.has(exception.mode)) throw new Error(`exception ${index} mode is invalid`);
    if (typeof exception.package !== "string" || !EXACT_IDENTITY.test(identity(exception.package, exception.version))) throw new Error(`exception ${index} identity must be exact`);
    if (!Array.isArray(exception.path) || exception.path.length < 2 || exception.path.some((entry) => !EXACT_IDENTITY.test(entry))) throw new Error(`exception ${index} path must be exact`);
    if (exception.path.at(-1) !== identity(exception.package, exception.version)) throw new Error(`exception ${index} path endpoint mismatch`);
    for (const field of ["rationale", "owner"]) if (typeof exception[field] !== "string" || exception[field].length === 0) throw new Error(`exception ${index} ${field} is required`);
    date(exception.expires, `exception ${index} expires`);
    date(exception.review_date, `exception ${index} review_date`);
  }
  return policy;
}

export function parseMetadata(metadata) {
  if (!metadata || !Array.isArray(metadata.packages) || !metadata.resolve || !Array.isArray(metadata.resolve.nodes)) throw new Error("cargo metadata is malformed or incomplete");
  const packages = new Map();
  for (const pkg of metadata.packages) {
    if (typeof pkg.id !== "string" || typeof pkg.name !== "string" || typeof pkg.version !== "string" || packages.has(pkg.id)) throw new Error("cargo metadata package is malformed or duplicated");
    packages.set(pkg.id, pkg);
  }
  const nodes = new Map();
  for (const node of metadata.resolve.nodes) {
    if (!packages.has(node.id) || !Array.isArray(node.deps) || nodes.has(node.id)) throw new Error("cargo metadata resolve node is malformed");
    for (const dep of node.deps) {
      if (!packages.has(dep.pkg) || !Array.isArray(dep.dep_kinds)) throw new Error("cargo metadata dependency is incomplete");
      for (const kind of dep.dep_kinds) if (kind.kind !== null && kind.kind !== "normal" && kind.kind !== "build" && kind.kind !== "dev") throw new Error("cargo metadata dependency kind is invalid");
    }
    nodes.set(node.id, node);
  }
  if (nodes.size !== packages.size) throw new Error("cargo metadata resolve is incomplete");
  return { packages, nodes };
}

export function evaluateSigningPaths(policyInput, metadataInput, modeName, today) {
  const policy = validateSigningPolicy(policyInput);
  date(today, "today");
  const mode = policy.modes.find((candidate) => candidate.name === modeName);
  if (!mode) throw new Error(`unknown mode ${modeName}`);
  const { packages, nodes } = parseMetadata(metadataInput);
  const idsByIdentity = new Map([...packages].map(([id, pkg]) => [identity(pkg.name, pkg.version), id]));
  const roots = mode.roots.map((root) => {
    const id = idsByIdentity.get(root);
    if (!id) throw new Error(`protected root missing: ${root}`);
    return id;
  });
  const protectedNames = new Set(policy.protected_packages);
  const queue = roots.map((id) => ({ id, path: [id] }));
  const best = new Map();
  while (queue.length > 0) {
    const current = queue.shift();
    if (best.has(current.id) && best.get(current.id).length <= current.path.length) continue;
    best.set(current.id, current.path);
    for (const dep of nodes.get(current.id).deps) {
      const applicableKinds = dep.dep_kinds.filter((kind) => kind.target === null || kind.target === undefined || typeof kind.target === "string");
      if (applicableKinds.length > 0 && applicableKinds.every((kind) => kind.kind === "dev")) continue;
      if (current.path.includes(dep.pkg)) continue;
      queue.push({ id: dep.pkg, path: [...current.path, dep.pkg] });
    }
  }
  const findings = [];
  for (const [id, pathIds] of best) {
    const pkg = packages.get(id);
    if (!protectedNames.has(pkg.name) || !prerelease(pkg.version)) continue;
    const path = pathIds.map((pathId) => { const item = packages.get(pathId); return identity(item.name, item.version); });
    const exception = policy.exceptions.find((candidate) => candidate.mode === modeName && candidate.package === pkg.name && candidate.version === pkg.version && JSON.stringify(candidate.path) === JSON.stringify(path));
    findings.push({ package: pkg.name, version: pkg.version, path, status: exception && exception.expires >= today ? "allowed" : "failure", reason: exception ? "exception expired" : "no exact path exception", exception });
  }
  return findings.sort((a, b) => `${a.package}@${a.version}`.localeCompare(`${b.package}@${b.version}`));
}

function main(argv) {
  if (argv.length !== 7 || argv[2] !== "evaluate") {
    console.error("usage: signing-path-policy.mjs evaluate <policy.json> <metadata.json> <mode> <YYYY-MM-DD>");
    return 2;
  }
  try {
    const policy = JSON.parse(readFileSync(resolve(argv[3]), "utf8"));
    const metadata = JSON.parse(readFileSync(resolve(argv[4]), "utf8"));
    const findings = evaluateSigningPaths(policy, metadata, argv[5], argv[6]);
    for (const finding of findings) {
      const label = finding.status === "allowed" ? "ALLOW" : "FAIL";
      const message = `${label} ${argv[5]} ${finding.package}@${finding.version}: ${finding.path.join(" -> ")}`;
      (finding.status === "allowed" ? console.log : console.error)(message);
    }
    console.log(JSON.stringify({ mode: argv[5], prereleases: findings.length, failures: findings.filter((finding) => finding.status === "failure").length }));
    return findings.some((finding) => finding.status === "failure") ? 1 : 0;
  } catch (error) {
    console.error(`signing path policy failed closed: ${error.message}`);
    return 2;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv);
