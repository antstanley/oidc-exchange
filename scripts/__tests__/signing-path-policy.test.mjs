import assert from "node:assert/strict";
import test from "node:test";
import { evaluateSigningPaths } from "../signing-path-policy.mjs";

const DATE = "2026-08-23";
function pkg(name, version, id = `${name} ${version}`) { return { id, name, version }; }
function node(id, deps = []) { return { id, deps: deps.map((dep) => ({ name: dep.id.split(" ")[0], pkg: dep.id, dep_kinds: dep.dep_kinds ?? [{ kind: dep.kind ?? null, target: dep.target ?? null }] })), features: [] }; }
function metadata(packages, edges) { return { packages, resolve: { root: null, nodes: packages.map((item) => node(item.id, edges[item.id] ?? [])) } }; }
function policy(exceptions = [], roots = ["root@1.0.0"]) { return { version: 1, effective_date: DATE, modes: [{ name: "test", metadata_args: ["--locked"], roots }], protected_packages: ["crypto", "helper"], exceptions }; }
function exception(overrides = {}) { return { mode: "test", package: "crypto", version: "2.0.0-rc.1", path: ["root@1.0.0", "crypto@2.0.0-rc.1"], rationale: "Bounded existing path", owner: "Security owner", expires: "2026-09-01", review_date: "2026-08-28", ...overrides }; }
const root = pkg("root", "1.0.0");
const stable = pkg("crypto", "2.0.0");
const candidate = pkg("crypto", "2.0.0-rc.1");

test("stable signing path passes", () => assert.deepEqual(evaluateSigningPaths(policy(), metadata([root, stable], { [root.id]: [stable] }), "test", DATE), []));
test("direct prerelease fails with actionable root path", () => assert.deepEqual(evaluateSigningPaths(policy(), metadata([root, candidate], { [root.id]: [candidate] }), "test", DATE)[0].path, ["root@1.0.0", "crypto@2.0.0-rc.1"]));
test("transitive prerelease fails including build edge", () => {
  const helper = pkg("helper", "1.0.0");
  const result = evaluateSigningPaths(policy(), metadata([root, helper, candidate], { [root.id]: [{ ...helper, kind: "build" }], [helper.id]: [candidate] }), "test", DATE);
  assert.equal(result[0].status, "failure"); assert.equal(result[0].path.length, 3);
});
test("unrelated and dev-only prereleases pass", () => {
  const unrelated = pkg("bindings-tool", "9.0.0-rc.1");
  assert.deepEqual(evaluateSigningPaths(policy(), metadata([root, unrelated], {}), "test", DATE), []);
  assert.deepEqual(evaluateSigningPaths(policy(), metadata([root, candidate], { [root.id]: [{ ...candidate, kind: "dev" }] }), "test", DATE), []);
});
test("exact exception passes", () => assert.equal(evaluateSigningPaths(policy([exception()]), metadata([root, candidate], { [root.id]: [candidate] }), "test", DATE)[0].status, "allowed"));
test("expired, wrong-version, and wrong-path exceptions fail", () => {
  const graph = metadata([root, candidate], { [root.id]: [candidate] });
  for (const item of [exception({ expires: "2026-08-22" }), exception({ version: "2.0.0-rc.2", path: ["root@1.0.0", "crypto@2.0.0-rc.2"] }), exception({ path: ["root@1.0.0", "helper@1.0.0", "crypto@2.0.0-rc.1"] })]) assert.equal(evaluateSigningPaths(policy([item]), graph, "test", DATE)[0].status, "failure");
});
test("cycles terminate and multiple roots retain shortest evidence", () => {
  const other = pkg("other-root", "1.0.0");
  const result = evaluateSigningPaths(policy([], ["root@1.0.0", "other-root@1.0.0"]), metadata([root, other, candidate], { [root.id]: [candidate], [candidate.id]: [root], [other.id]: [candidate] }), "test", DATE);
  assert.equal(result.length, 1); assert.equal(result[0].path.length, 2);
});
test("target-qualified normal edge is evaluated", () => {
  const result = evaluateSigningPaths(policy(), metadata([root, candidate], { [root.id]: [{ ...candidate, target: "cfg(target_os = \"linux\")" }] }), "test", DATE);
  assert.equal(result[0].status, "failure");
});

test("mixed normal and dev dependency kinds preserve runtime prerelease", () => {
  const result = evaluateSigningPaths(policy(), metadata([root, candidate], { [root.id]: [{ ...candidate, dep_kinds: [{ kind: "dev", target: null }, { kind: null, target: null }] }] }), "test", DATE);
  assert.equal(result[0].status, "failure");
});
test("mixed target-qualified kinds preserve applicable normal edge", () => {
  const result = evaluateSigningPaths(policy(), metadata([root, candidate], { [root.id]: [{ ...candidate, dep_kinds: [{ kind: "dev", target: "cfg(windows)" }, { kind: "normal", target: "cfg(linux)" }] }] }), "test", DATE);
  assert.equal(result[0].status, "failure");
});
test("duplicate stable and prerelease lines only flag reachable identity", () => {
  const result = evaluateSigningPaths(policy(), metadata([root, stable, candidate], { [root.id]: [stable] }), "test", DATE);
  assert.deepEqual(result, []);
});
test("malformed and incomplete metadata fail closed", () => {
  assert.throws(() => evaluateSigningPaths(policy(), {}, "test", DATE), /malformed/);
  assert.throws(() => evaluateSigningPaths(policy(), { packages: [root], resolve: { nodes: [] } }, "test", DATE), /incomplete/);
});
