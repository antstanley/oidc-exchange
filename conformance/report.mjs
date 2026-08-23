import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
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

const qualifications = {
  asgi: { "encoded-slash": "ASGI scope omitted raw_path", "encoded-dot-dot": "ASGI scope omitted raw_path", "encoded-question": "ASGI scope omitted raw_path", "encoded-hash": "ASGI scope omitted raw_path", "malformed-content-length": "ASGI framing has no Content-Length parser", "huge-content-length": "ASGI framing has no Content-Length parser" },
  wsgi: { "encoded-slash": "WSGI server omitted RAW_URI/REQUEST_URI", "encoded-dot-dot": "WSGI server omitted RAW_URI/REQUEST_URI", "encoded-question": "WSGI server omitted RAW_URI/REQUEST_URI", "encoded-hash": "WSGI server omitted RAW_URI/REQUEST_URI", "forwarded-for-first": "WSGI omitted ordered-header extension", "forwarded-for-reversed": "WSGI omitted ordered-header extension" },
  lambda: { "encoded-slash": "API Gateway v1 supplied a decoded path", "encoded-dot-dot": "API Gateway v1 supplied a decoded path", "encoded-question": "API Gateway v1 supplied a decoded path", "encoded-hash": "API Gateway v1 supplied a decoded path", "forwarded-for-first": "API Gateway v1 preserves duplicates via multiValueHeaders", "forwarded-for-reversed": "API Gateway v1 preserves duplicates via multiValueHeaders", "malformed-content-length": "AWS supplies decoded bodies", "huge-content-length": "AWS supplies decoded bodies" },
};

function input(fixture, shape) {
  const qualified = qualifications[shape]?.[fixture.id];
  return { id: fixture.id, method: fixture.request.method, rawPath: fixture.request.rawPath, query: fixture.request.query ?? null, headers: fixture.request.headers, bodyLength: fixture.request.body?.length ?? 0, pathIsRaw: !(qualified?.includes("decoded") || qualified?.includes("omitted raw")), orderedHeadersAvailable: !qualified?.includes("ordered-header"), lambdaEvent: qualified?.includes("decoded path") ? "v1" : "v2" };
}

function expected(fixture, shape) {
  const value = structuredClone(fixture.expected);
  const reason = qualifications[shape]?.[fixture.id];
  if (reason?.includes("omitted raw") || reason?.includes("decoded path")) value.decodedPath = decodeURIComponent(value.decodedPath);
  if (reason?.includes("ordered-header")) value.orderedHeaders = [];
  if (reason?.includes("multiValueHeaders")) value.orderedHeaders = [value.orderedHeaders.at(-1)];
  if (reason?.includes("decoded bodies")) value.status = 415;
  if (shape === "asgi" && (fixture.id === "encoded-question" || fixture.id === "encoded-hash")) value.status = 400;
  if (!reason && (fixture.id === "malformed-content-length" || fixture.id === "huge-content-length")) value.status = 415;
  if (shape === "asgi" && (fixture.id === "malformed-content-length" || fixture.id === "huge-content-length")) value.status = 415;
  if (shape === "wsgi" && fixture.id === "malformed-content-length") value.status = 400;
  if (shape === "wsgi" && fixture.id === "huge-content-length") value.status = 413;
  return value;
}

async function run(shape) {
  let command;
  let args;
  if (shape === "ffi" || shape === "native") {
    command = join(root, "target/debug/oidc-exchange-conformance"); args = [];
  } else if (shape === "node" || shape === "lambda") {
    command = process.execPath; args = [join(root, "conformance/node_runner.mjs"), shape, config];
  } else {
    command = join(root, "bindings/python/.venv/bin/python"); args = [join(root, "conformance/python_runner.py"), shape, config];
  }
  const child = spawn(command, args, { cwd: root, stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: child.stdout });
  const outputs = [];
  lines.on("line", line => outputs.push(JSON.parse(line)));
  for (const fixture of corpus.fixtures) child.stdin.write(JSON.stringify(input(fixture, shape)) + "\n");
  child.stdin.end();
  const exit = await new Promise(resolve => child.on("exit", resolve));
  if (exit !== 0) throw new Error(`${shape} runner exited ${exit}`);
  return outputs;
}

try {
  const build = spawnSync("cargo", ["build", "-p", "oidc-exchange-conformance"], { cwd: root, stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
  let failures = 0;
  console.log(`executable conformance gate: ${corpus.fixtures.length} fixtures x ${shapes.length} shapes`);
  for (const shape of shapes) {
    const outputs = await run(shape);
    if (outputs.length !== corpus.fixtures.length || outputs.some(output => output.executed !== true)) throw new Error(`${shape}: runner did not execute all fixtures`);
    let qualified = 0;
    for (const [index, fixture] of corpus.fixtures.entries()) {
      const actual = outputs[index]; const wanted = expected(fixture, shape);
      for (const field of corpus.fieldsCompared) if (JSON.stringify(actual[field]) !== JSON.stringify(wanted[field])) { failures++; console.error(`${shape}/${fixture.id}/${field}: expected=${JSON.stringify(wanted[field])} actual=${JSON.stringify(actual[field])}${qualifications[shape]?.[fixture.id] ? ` qualification=${qualifications[shape][fixture.id]}` : ""}`); }
      if (qualifications[shape]?.[fixture.id]) qualified++;
    }
    console.log(`${shape}: ${outputs.length} executed, ${qualified} qualified inputs`);
  }
  console.log(`pairings: ${corpus.fixtures.length * shapes.length} executed; unqualified mismatches: ${failures}`);
  if (failures) process.exitCode = 1;
} finally { await rm(temp, { recursive: true, force: true }); }
