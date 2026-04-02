import { describe, it, beforeAll, afterAll, expect } from "vitest";
import { execSync } from "node:child_process";
import { unlinkSync, existsSync } from "node:fs";

const TEST_KEY_PATH = "/tmp/oidc-test-nodejs-key.pem";
const TEST_DB_PATH = "/tmp/oidc-test-nodejs.db";

const TEST_CONFIG = `
[server]
issuer = "https://auth.test.com"
role = "exchange"

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

beforeAll(async () => {
  execSync(`openssl genpkey -algorithm Ed25519 -out ${TEST_KEY_PATH}`);
  const binding = await import("../index.js");
  OidcExchange = binding.OidcExchange;
});

afterAll(() => {
  for (const p of [TEST_KEY_PATH, TEST_DB_PATH]) {
    if (existsSync(p)) {
      unlinkSync(p);
    }
  }
});

describe("OidcExchange", () => {
  it("should create an instance from config string", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    expect(oidc).toBeTruthy();
    oidc.shutdown();
  });

  it("should reject missing config", () => {
    expect(() => new OidcExchange({})).toThrow();
  });

  it("should handle GET /health", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: "GET",
      path: "/health",
      headers: [],
    });
    expect(response.status).toBe(200);
    oidc.shutdown();
  });

  it("should handle GET /keys and return JWKS", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: "GET",
      path: "/keys",
      headers: [],
    });
    expect(response.status).toBe(200);
    const body = JSON.parse(Buffer.from(response.body).toString("utf-8"));
    expect(Array.isArray(body.keys)).toBe(true);
    oidc.shutdown();
  });

  it("should handle GET /.well-known/openid-configuration", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: "GET",
      path: "/.well-known/openid-configuration",
      headers: [],
    });
    expect(response.status).toBe(200);
    const body = JSON.parse(Buffer.from(response.body).toString("utf-8"));
    expect(body.issuer).toBe("https://auth.test.com");
    oidc.shutdown();
  });

  it("should return 404 for unknown routes", () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: "GET",
      path: "/nonexistent",
      headers: [],
    });
    expect(response.status).toBe(404);
    oidc.shutdown();
  });
});
