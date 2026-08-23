import { readFile, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline";

const root = new URL("../", import.meta.url).pathname;
const corpus = JSON.parse(await readFile(new URL("./corpus/requests.json", import.meta.url), "utf8"));
const shapes = ["native", "ffi", "node", "lambda", "asgi", "wsgi"];
const temp = await mkdtemp(join(tmpdir(), "oidc-conformance-"));
const key = join(temp, "key.pem");
const database = join(temp, "db.sqlite");
spawnSync("openssl", ["genpkey", "-algorithm", "Ed25519", "-out", key], { stdio: "inherit" });
const config = join(temp, "config.toml");
await writeFile(config, `[server]\nissuer = "https://conformance.invalid"\nrole = "exchange"\nbase_path = "/auth"\nmax_request_body_bytes = 2097152\n[registration]\nmode = "open"\n[repository]\nadapter = "sqlite"\n[repository.sqlite]\npath = "${database}"\n[key_manager]\nadapter = "local"\n[key_manager.local]\nprivate_key_path = "${key}"\nalgorithm = "EdDSA"\nkid = "conformance"\n[audit]\nadapter = "noop"\n[telemetry]\nenabled = false\n`);

function input(fixture, shape, variant = "faithful") {
  return { id: fixture.id, method: fixture.request.method, rawPath: fixture.request.rawPath, query: fixture.request.query ?? null, headers: fixture.request.headers, bodyLength: fixture.request.body?.length ?? 0, pathIsRaw: variant === "faithful", orderedHeadersAvailable: variant === "faithful", lambdaEvent: variant === "faithful" ? "v2" : "v1" };
}

async function run(shape, variant = "faithful") {
  let command, args;
  if (shape === "ffi" || shape === "native") { command = join(root, "target/debug/oidc-exchange-conformance"); args = [shape]; }
  else if (shape === "node" || shape === "lambda") { command = process.execPath; args = [join(root, "conformance/node_runner.mjs"), shape, config]; }
  else { command = join(root, "bindings/python/.venv/bin/python"); args = [join(root, "conformance/python_runner.py"), shape, config]; }
  const child = spawn(command, args, { cwd: root, stdio: ["pipe", "pipe", "inherit"] });
  const outputs = [];
  createInterface({ input: child.stdout }).on("line", line => outputs.push(JSON.parse(line)));
  for (const fixture of corpus.fixtures) child.stdin.write(JSON.stringify(input(fixture, shape, variant)) + "\n");
  child.stdin.end();
  const exit = await new Promise(resolve => child.on("exit", resolve));
  if (exit !== 0) throw new Error(`${shape}/${variant} runner exited ${exit}`);
  return outputs;
}

try {
  const build = spawnSync("cargo", ["build", "-p", "oidc-exchange-conformance"], { cwd: root, stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
  let failures = 0, pairings = 0;
  console.log(`authentic conformance gate: ${corpus.fixtures.length} fixtures x ${shapes.length} shapes`);
  for (const shape of shapes) {
    const outputs = await run(shape);
    pairings += outputs.length;
    if (outputs.length !== corpus.fixtures.length || outputs.some(output => output.executed !== true)) throw new Error(`${shape}: runner did not execute all fixtures`);
    for (const [index, fixture] of corpus.fixtures.entries()) {
      const actual = outputs[index];
      for (const field of corpus.fieldsCompared) {
        const wanted = fixture.expected[field];
        if (JSON.stringify(actual[field]) !== JSON.stringify(wanted)) { failures++; console.error(`${shape}/${fixture.id}/${field}: expected=${JSON.stringify(wanted)} actual=${JSON.stringify(actual[field])}`); }
      }
    }
    console.log(`${shape}: ${outputs.length} faithful pairings executed`);
  }
  for (const shape of ["lambda", "asgi", "wsgi"]) {
    const key = `${shape}-fallback`, qualifications = corpus.qualifications[key] ?? {};
    const outputs = await run(shape, "fallback");
    for (const [index, fixture] of corpus.fixtures.entries()) for (const field of corpus.fieldsCompared) {
      const qualification = qualifications[fixture.id]?.[field];
      const wanted = qualification?.fallbackExpected ?? fixture.expected[field];
      if (JSON.stringify(outputs[index][field]) !== JSON.stringify(wanted) && !qualification) { failures++; console.error(`${key}/${fixture.id}/${field}: unqualified mismatch`); }
      if (qualification) console.log(`qualification ${key}/${fixture.id}/${field}: ${qualification.reason}`);
    }
  }
  console.log(`pairings: ${pairings} faithful executed; unqualified mismatches: ${failures}`);
  if (failures) process.exitCode = 1;
} finally { await rm(temp, { recursive: true, force: true }); }
