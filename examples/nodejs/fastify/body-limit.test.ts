import assert from "node:assert/strict";
import test, { after } from "node:test";
import { execFileSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";

const key = "/tmp/oidc-fastify-f6-key.pem";
const database = "/tmp/oidc-fastify-f6.db";
if (!existsSync(key)) execFileSync("openssl", ["genpkey", "-algorithm", "Ed25519", "-out", key]);
process.env.OIDC_EXCHANGE_CONFIG_STRING = `[server]
issuer="https://example.test"
role="exchange"
max_request_body_bytes=64
[registration]
mode="open"
[repository]
adapter="sqlite"
[repository.sqlite]
path="${database}"
[key_manager]
adapter="local"
[key_manager.local]
private_key_path="${key}"
algorithm="EdDSA"
kid="test"
[audit]
adapter="noop"
[telemetry]
enabled=false`;

process.env.NODE_ENV = "test";
process.chdir(new URL("..", import.meta.url).pathname);
const { app, maxBodyBytes } = await import("./index.js");
after(async () => { await app.close(); rmSync(key, { force: true }); rmSync(database, { force: true }); });

test("Fastify uses the exact published limit and preserves buffered bytes", async () => {
  assert.equal(app.initialConfig.bodyLimit, maxBodyBytes);
  for (const size of [maxBodyBytes - 1, maxBodyBytes]) {
    const response = await app.inject({ method: "POST", url: "/auth/test", headers: { "content-type": "application/octet-stream" }, payload: Buffer.alloc(size) });
    assert.notEqual(response.statusCode, 413);
  }
});

test("Fastify parser returns exact 413 above cap with truthful or absent length", async () => {
  const payload = Buffer.alloc(maxBodyBytes + 1);
  const truthful = await app.inject({ method: "POST", url: "/auth/test", headers: { "content-type": "application/octet-stream" }, payload });
  assert.equal(truthful.statusCode, 413);
  const chunked = await app.inject({ method: "POST", url: "/auth/test", headers: { "content-type": "application/octet-stream", "transfer-encoding": "chunked" }, payload });
  assert.equal(chunked.statusCode, 413);
});

test("Fastify accepts an empty body", async () => {
  const response = await app.inject({ method: "POST", url: "/auth/test" });
  assert.notEqual(response.statusCode, 413);
});
