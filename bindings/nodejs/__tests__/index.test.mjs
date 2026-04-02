import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execSync } from 'node:child_process';
import { unlinkSync, existsSync } from 'node:fs';

const TEST_KEY_PATH = '/tmp/oidc-test-nodejs-key.pem';
const TEST_DB_PATH = '/tmp/oidc-test-nodejs.db';

const TEST_CONFIG = `
[session_store]
type = "sqlite"
path = "${TEST_DB_PATH}"

[key_manager]
type = "local"
key_path = "${TEST_KEY_PATH}"

[audit]
type = "noop"

[server]
issuer = "https://auth.test.com"
registration_mode = "open"
role = "exchange"

[telemetry]
enabled = false
`;

let OidcExchange;

before(async () => {
  execSync(`openssl genpkey -algorithm Ed25519 -out ${TEST_KEY_PATH}`);
  const binding = await import('../index.js');
  OidcExchange = binding.OidcExchange;
});

after(() => {
  for (const p of [TEST_KEY_PATH, TEST_DB_PATH]) {
    if (existsSync(p)) {
      unlinkSync(p);
    }
  }
});

describe('OidcExchange', () => {
  it('should create an instance from config string', () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    assert.ok(oidc);
    oidc.shutdown();
  });

  it('should reject missing config', () => {
    assert.throws(() => {
      new OidcExchange({});
    });
  });

  it('should handle GET /health', () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: 'GET',
      path: '/health',
      headers: [],
    });
    assert.equal(response.status, 200);
    oidc.shutdown();
  });

  it('should handle GET /keys and return JWKS', () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: 'GET',
      path: '/keys',
      headers: [],
    });
    assert.equal(response.status, 200);
    const body = JSON.parse(Buffer.from(response.body).toString('utf-8'));
    assert.ok(Array.isArray(body.keys));
    oidc.shutdown();
  });

  it('should handle GET /.well-known/openid-configuration', () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: 'GET',
      path: '/.well-known/openid-configuration',
      headers: [],
    });
    assert.equal(response.status, 200);
    const body = JSON.parse(Buffer.from(response.body).toString('utf-8'));
    assert.equal(body.issuer, 'https://auth.test.com');
    oidc.shutdown();
  });

  it('should return 404 for unknown routes', () => {
    const oidc = new OidcExchange({ configString: TEST_CONFIG });
    const response = oidc.handleRequest({
      method: 'GET',
      path: '/nonexistent',
      headers: [],
    });
    assert.equal(response.status, 404);
    oidc.shutdown();
  });
});
