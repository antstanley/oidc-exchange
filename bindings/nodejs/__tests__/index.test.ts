import { execSync } from "node:child_process";
import { unlinkSync, existsSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const TEST_KEY_PATH = "/tmp/oidc-test-nodejs-key.pem";
const TEST_DB_PATH = "/tmp/oidc-test-nodejs.db";
const TEST_CONFIG = `
[server]
issuer = "https://auth.test.com"
role = "exchange"
max_request_body_bytes = 32
[registration]
mode = "open"
[repository]
adapter = "sqlite"
[repository.sqlite]
path = "${TEST_DB_PATH}"
[key_manager]
adapter = "local"
[key_manager.local]
private_key_path = "${TEST_KEY_PATH}"
algorithm = "EdDSA"
kid = "test-key-1"
[audit]
adapter = "noop"
[telemetry]
enabled = false
`;

let OidcExchange: any;

const request = (rawPath: string, extra: Record<string, unknown> = {}) => ({
  method: "GET",
  rawPath: Buffer.from(rawPath),
  headers: [],
  pathIsRaw: true,
  ...extra,
});

beforeAll(async () => {
  execSync(`openssl genpkey -algorithm Ed25519 -out ${TEST_KEY_PATH}`);
  ({ OidcExchange } = await import("../index.js"));
});

afterAll(() => {
  for (const path of [TEST_KEY_PATH, TEST_DB_PATH]) {
    if (existsSync(path)) unlinkSync(path);
  }
});

describe("OidcExchange", () => {
  it("constructs and publishes the configured body limit", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    expect(oidc.limits()).toEqual({ maxBodyBytes: 32 });
    oidc.shutdown();
  });

  it("rejects missing config", () => {
    expect(() => new OidcExchange({})).toThrow();
  });

  it("returns a Promise and resolves health", async () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const pending = oidc.handleRequest(request("/health"));
    expect(pending).toBeInstanceOf(Promise);
    expect((await pending).status).toBe(200);
  });

  it("keeps raw path and query separate", async () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = await oidc.handleRequest(
      request("/health", { query: Buffer.from("first=1?second=2") }),
    );
    expect(response.status).toBe(200);
  });

  it("returns HTTP responses for malformed wire requests", async () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    expect((await oidc.handleRequest(request("/health", { method: "NOT A METHOD" }))).status).toBe(
      400,
    );
    expect((await oidc.handleRequest(request("//health"))).status).toBe(400);
    expect(
      (await oidc.handleRequest(request("/token", { method: "POST", body: Buffer.alloc(33) })))
        .status,
    ).toBe(413);
  });

  it("accepts ordered duplicate headers", async () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = await oidc.handleRequest(
      request("/health", {
        headers: [
          { name: "x-forwarded-for", value: "192.0.2.1" },
          { name: "x-forwarded-for", value: "198.51.100.2" },
        ],
      }),
    );
    expect(response.status).toBe(200);
  });

  it("applies basePath before routing and respects segment boundaries", async () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG, basePath: "/auth" });
    expect((await oidc.handleRequest(request("/auth/health"))).status).toBe(200);
    expect((await oidc.handleRequest(request("/authorize/health"))).status).toBe(404);
  });

  it.each(["", "/", "auth", "/auth/"])("rejects invalid basePath %j", (basePath) => {
    expect(() => new OidcExchange({ configString: TEST_CONFIG, basePath })).toThrow(
      /basePath must be an absolute, non-root path without a trailing slash/,
    );
  });

  it("preserves configured defaults and isolates handler overrides", async () => {
    const configured = TEST_CONFIG.replace("[server]", '[server]\nbase_path = "/configured"');
    const defaultInstance = new OidcExchange({ configString: configured });
    const overridden = new OidcExchange({ configString: configured, basePath: "/override" });
    const freshDefault = new OidcExchange({ configString: configured });
    expect((await defaultInstance.handleRequest(request("/configured/health"))).status).toBe(200);
    expect((await overridden.handleRequest(request("/override/health"))).status).toBe(200);
    expect((await freshDefault.handleRequest(request("/configured/health"))).status).toBe(200);
    expect((await overridden.handleRequest(request("/configured/health"))).status).toBe(404);
  });

  it("retains deprecated synchronous compatibility", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    expect(oidc.handleRequestSync(request("/health")).status).toBe(200);
    expect(oidc.handleRequestSync(request("/nonexistent")).status).toBe(404);
  });
});
