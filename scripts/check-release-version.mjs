import { readFileSync } from "node:fs";
const root = new URL("../", import.meta.url);
const read = p => readFileSync(new URL(p, root), "utf8");
const json = p => JSON.parse(read(p));
const version = "0.3.0";
const packages = [
  "bindings/nodejs/package.json", "bindings/lambda/package.json",
  "bindings/nodejs/npm/darwin-arm64/package.json", "bindings/nodejs/npm/linux-arm64-gnu/package.json",
  "bindings/nodejs/npm/linux-x64-gnu/package.json", "bindings/nodejs/npm/win32-x64-msvc/package.json",
];
for (const p of packages) if (json(p).version !== version) throw new Error(`${p} version mismatch`);
const node = json(packages[0]);
for (const [name, v] of Object.entries(node.optionalDependencies)) if (v !== version) throw new Error(`${name}=${v}`);
const lambda = json(packages[1]);
if (lambda.peerDependencies["@oidc-exchange/node"] !== "^0.3.0") throw new Error("Lambda peer range mismatch");
for (const [p, pattern] of [["Cargo.toml", /version = "0\.3\.0"/], ["Cargo.lock", /name = "oidc-exchange"\nversion = "0\.3\.0"/], ["bindings/python/pyproject.toml", /version = "0\.3\.0"/], ["bindings/python/uv.lock", /name = "oidc-exchange"\nversion = "0\.3\.0"/], ["pnpm-lock.yaml", /specifier: workspace:\^[\s\S]{0,80}version: link:\.\.\/nodejs/]]) if (!pattern.test(read(p))) throw new Error(`${p} derived version mismatch`);
for (const p of packages) if (/"(?:version|@oidc-exchange\/node)"\s*:\s*"(?:\^)?0\.2/.test(read(p))) throw new Error(`${p} has stale release range`);
console.log(`release parity OK: ${packages.length} npm manifests + Cargo, Python, pnpm, uv metadata at ${version}`);
