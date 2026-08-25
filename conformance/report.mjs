import { createInterface } from "node:readline";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import { join } from "node:path";
import { tmpdir } from "node:os";

const root = new URL("../", import.meta.url).pathname;
const corpus = JSON.parse(await readFile(new URL("./corpus/requests.json", import.meta.url), "utf8"));
const allShapes = ["native", "ffi", "node", "lambda", "asgi", "wsgi"];
const requestedShapes = process.env.CONFORMANCE_SHAPES?.split(",").map((shape) => shape.trim()).filter(Boolean);
const shapes = requestedShapes ?? allShapes;
const unknownShapes = shapes.filter((shape) => !allShapes.includes(shape));
if (unknownShapes.length || new Set(shapes).size !== shapes.length || shapes.length === 0) {
  throw new Error(`CONFORMANCE_SHAPES must be a non-empty, unique subset of ${allShapes.join(",")}`);
}
const canonical = corpus.fixtures.filter((fixture) => !fixture.probe);
const probes = corpus.fixtures.filter((fixture) => fixture.probe === "provenance");
const temp = await mkdtemp(join(tmpdir(), "oidc-conformance-"));
const config = join(temp, "config.toml");
const key = join(temp, "key.pem");
const database = join(temp, "db.sqlite");
const pyArtifact = join(temp, "_oidc_exchange.so");

function runSync(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed`);
}
function input(fixture, shape, variant) {
  return {
    id: fixture.id,
    method: fixture.request.method,
    rawPath: fixture.request.rawPath,
    query: fixture.request.query ?? null,
    headers: fixture.request.headers,
    bodyLength: fixture.request.body?.length ?? 0,
    pathIsRaw: variant === "faithful",
    lambdaEvent: shape === "lambda" ? ["v2", "v1", "alb"][canonical.indexOf(fixture) % 3] : undefined,
  };
}
async function execute(shape, fixtures, variant = "faithful", runnerOverride) {
  let command;
  let args;
  if (["native", "ffi"].includes(shape)) {
    command = join(root, "target/debug/oidc-exchange-conformance");
    args = [shape];
  } else if (["node", "lambda"].includes(shape)) {
    command = process.execPath;
    args = [runnerOverride ?? join(root, "conformance/node_runner.mjs"), shape, config, join(root, "bindings/nodejs/oidc-exchange.node"), variant];
  } else {
    command = join(root, "bindings/python/.venv/bin/python");
    args = [runnerOverride ?? join(root, "conformance/python_runner.py"), shape, config, pyArtifact, variant];
  }
  const child = spawn(command, args, { cwd: root, stdio: ["pipe", "pipe", "inherit"] });
  const output = [];
  createInterface({ input: child.stdout }).on("line", (line) => output.push(JSON.parse(line)));
  for (const fixture of fixtures) child.stdin.write(`${JSON.stringify(input(fixture, shape, variant))}\n`);
  child.stdin.end();
  const code = await new Promise((resolve) => child.on("exit", resolve));
  if (code !== 0) throw new Error(`${shape}/${variant} runner exited ${code}`);
  if (output.length !== fixtures.length || output.some((record) => record.executed !== true)) throw new Error(`${shape}/${variant}: missing executions`);
  return output;
}
function matchesHeaders(wanted, actual) {
  if (!Array.isArray(wanted) || !Array.isArray(actual)) return JSON.stringify(wanted) === JSON.stringify(actual);
  let cursor = 0;
  for (const header of actual) if (cursor < wanted.length && JSON.stringify(header) === JSON.stringify(wanted[cursor])) cursor += 1;
  return cursor === wanted.length;
}
function compare(key, fixtures, outputs) {
  let failures = 0;
  let qualified = 0;
  let statusOnly = 0;
  const qualifications = corpus.qualifications[key] ?? {};
  for (const [i, fixture] of fixtures.entries()) {
    const notApplicable = fixture.notApplicable ?? [];
    if (notApplicable.length) statusOnly += 1;
    for (const field of corpus.fieldsCompared) {
      if (notApplicable.includes(field)) continue;
      const qualification = qualifications[fixture.id]?.[field];
      const wanted = qualification && Object.hasOwn(qualification, "fallbackExpected") ? qualification.fallbackExpected : fixture.expected[field];
      const matched = field === "orderedHeaders" ? matchesHeaders(wanted, outputs[i][field]) : JSON.stringify(outputs[i][field]) === JSON.stringify(wanted);
      if (qualification?.kind === "host-loss") {
        qualified += 1;
        console.log(`qualification ${key}/${fixture.id}/${field} [host-loss]: ${qualification.reason}`);
      } else if (!matched) {
        failures += 1;
        console.error(`mismatch ${key}/${fixture.id}/${field}: expected=${JSON.stringify(wanted)} actual=${JSON.stringify(outputs[i][field])}`);
      }
    }
  }
  return { failures, qualified, statusOnly };
}

try {
  runSync("openssl", ["genpkey", "-algorithm", "Ed25519", "-out", key]);
  await writeFile(config, `[server]\nissuer="https://conformance.invalid"\nrole="exchange"\nbase_path="/auth"\nmax_request_body_bytes=2097152\n[registration]\nmode="open"\n[repository]\nadapter="sqlite"\n[repository.sqlite]\npath="${database}"\n[key_manager]\nadapter="local"\n[key_manager.local]\nprivate_key_path="${key}"\nalgorithm="EdDSA"\nkid="conformance"\n[audit]\nadapter="noop"\n[telemetry]\nenabled=false\n`);
  runSync("corepack", ["pnpm@11.9.0", "install", "--frozen-lockfile", "--ignore-scripts"]);
  if (shapes.some((shape) => ["native", "ffi"].includes(shape))) {
    runSync("cargo", ["build", "-p", "oidc-exchange-conformance"]);
  }
  if (shapes.some((shape) => ["node", "lambda"].includes(shape))) {
    const buildStarted = Date.now();
    runSync("corepack", ["pnpm@11.9.0", "--dir", "bindings/nodejs", "exec", "napi", "build", "--features", "conformance", "--dts", "index.generated.d.ts"]);
    runSync("corepack", ["pnpm@11.9.0", "--dir", "bindings/lambda", "build"]);
    const artifact = join(root, "bindings/nodejs/oidc-exchange.node");
    const artifactStat = await stat(artifact).catch(() => null);
    if (!artifactStat || artifactStat.mtimeMs + 1000 < buildStarted) throw new Error(`missing or stale artifact ${artifact}`);
  }
  if (shapes.some((shape) => ["asgi", "wsgi"].includes(shape))) {
    runSync("bindings/python/.venv/bin/maturin", ["build", "--manifest-path", "bindings/python/Cargo.toml", "--features", "conformance", "--interpreter", "bindings/python/.venv/bin/python", "--out", temp]);
    const wheel = (spawnSync("bash", ["-lc", `ls '${temp}'/*.whl`], { encoding: "utf8" }).stdout || "").trim();
    runSync("unzip", ["-q", wheel, "-d", join(temp, "wheel")]);
    const ext = (spawnSync("bash", ["-lc", `ls '${join(temp, "wheel/oidc_exchange")}'/_oidc_exchange*.so`], { encoding: "utf8" }).stdout || "").trim();
    runSync("cp", [ext, pyArtifact]);
    if (Date.now() - (await stat(pyArtifact)).mtimeMs > 900000) throw new Error(`stale artifact ${pyArtifact}`);
  }

  let failures = 0;
  let qualifications = 0;
  let canonicalExecutions = 0;
  let fallbackExecutions = 0;
  let provenanceExecutions = 0;
  let statusOnly = 0;
  console.log(`conformance gate: ${canonical.length} fixtures x ${shapes.length} canonical shapes`);
  for (const shape of shapes) {
    const output = await execute(shape, canonical);
    canonicalExecutions += output.length;
    const result = compare(shape, canonical, output);
    failures += result.failures;
    qualifications += result.qualified;
    statusOnly += result.statusOnly;
    console.log(`${shape}: ${output.length} canonical executions`);
  }
  for (const shape of ["lambda", "asgi", "wsgi"].filter((shape) => shapes.includes(shape))) {
    const fallbackFixtures = shape === "lambda"
      ? canonical.filter((fixture) => corpus.qualifications["lambda-fallback"]?.[fixture.id])
      : canonical;
    const output = await execute(shape, fallbackFixtures, "fallback");
    fallbackExecutions += output.length;
    const result = compare(`${shape}-fallback`, fallbackFixtures, output);
    failures += result.failures;
    qualifications += result.qualified;
    statusOnly += result.statusOnly;
    console.log(`${shape}-fallback: ${output.length} explicit probes`);
  }
  for (const shape of shapes) {
    const output = await execute(shape, probes);
    provenanceExecutions += output.length;
    const result = compare(shape, probes, output);
    failures += result.failures;
    qualifications += result.qualified;
    console.log(`${shape}: ${output.length} provenance probes`);
  }
  console.log(`execution counts: canonical=${canonicalExecutions}; fallback probes=${fallbackExecutions}; provenance probes=${provenanceExecutions}; status-only=${statusOnly}; qualifications=${qualifications}; unqualified mismatches=${failures}`);
  if (failures) process.exitCode = 1;
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  await rm(temp, { recursive: true, force: true });
}
