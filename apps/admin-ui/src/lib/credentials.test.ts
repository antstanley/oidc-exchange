import { afterEach, describe, expect, it } from "vitest";

import { InternalApiConfigurationError, resolveOperatorCredential } from "./api";
import { env } from "../../tests/env-stub";

/** Every credential-bearing variable the generated client consults. */
const CREDENTIAL_KEYS = [
  "INTERNAL_API_TOKEN",
  "INTERNAL_API_CLIENT_CERT",
  "INTERNAL_API_CLIENT_KEY",
  "INTERNAL_API_SECRET",
] as const;

function clearCredentials(): void {
  for (const key of CREDENTIAL_KEYS) {
    delete env[key];
  }
}

afterEach(clearCredentials);

describe("server-side credential selection follows the contract's preference order", () => {
  it("prefers the operator token over every other mechanism", () => {
    env.INTERNAL_API_TOKEN = "token-value";
    env.INTERNAL_API_CLIENT_CERT = "cert-value";
    env.INTERNAL_API_CLIENT_KEY = "key-value";
    env.INTERNAL_API_SECRET = "secret-value";

    const credential = resolveOperatorCredential();

    expect(credential).toEqual({
      kind: "operator_token",
      authorization: "Bearer token-value",
    });
  });

  it("falls back to a complete client-certificate pair when no token is configured", () => {
    env.INTERNAL_API_CLIENT_CERT = "cert-value";
    env.INTERNAL_API_CLIENT_KEY = "key-value";
    env.INTERNAL_API_SECRET = "secret-value";

    expect(resolveOperatorCredential()).toEqual({
      kind: "client_certificate",
      certificate: "cert-value",
      privateKey: "key-value",
    });
  });

  it("uses the compatibility shared secret last", () => {
    env.INTERNAL_API_SECRET = "secret-value";

    expect(resolveOperatorCredential()).toEqual({
      kind: "shared_secret",
      authorization: "Bearer secret-value",
    });
  });
});

describe("credential misconfiguration fails closed", () => {
  it("rejects a half-configured certificate pair instead of falling through", () => {
    env.INTERNAL_API_CLIENT_CERT = "cert-value";
    // The matching key is deliberately absent.

    expect(() => resolveOperatorCredential()).toThrow(
      /INTERNAL_API_CLIENT_CERT and INTERNAL_API_CLIENT_KEY must be configured together/,
    );
  });

  it("a half-configured pair is an error even when a weaker credential exists", () => {
    env.INTERNAL_API_CLIENT_CERT = "cert-value";
    env.INTERNAL_API_SECRET = "secret-value";

    // A broken deployment must not silently downgrade to the secret.
    expect(() => resolveOperatorCredential()).toThrow(InternalApiConfigurationError);
  });

  it("refuses to produce any credential when nothing is configured", () => {
    expect(() => resolveOperatorCredential()).toThrow(/no operator credential configured/);
  });
});
