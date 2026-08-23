import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { exportJWK, generateKeyPair, SignJWT, UnsecuredJWT } from "jose";

const ISSUER = "https://issuer.example";
const AUDIENCE = "admin-ui";
const DISCOVERY_URL = `${ISSUER}/.well-known/openid-configuration`;
const JWKS_URL = `${ISSUER}/keys`;
const env = {
  OIDC_EXCHANGE_URL: ISSUER,
  OIDC_EXCHANGE_ISSUER: ISSUER,
  ADMIN_UI_AUDIENCE: AUDIENCE,
  REQUIRED_CLAIM: "role",
  REQUIRED_VALUE: "admin",
};

vi.mock("$env/dynamic/private", () => ({ env }));

let privateKey: CryptoKey;
let alternateKey: CryptoKey;
let publicJwk: Record<string, unknown>;
let alternateJwk: Record<string, unknown>;
let now: number;
let discoveryFetches: number;
let jwksFetches: number;
let discovery: Record<string, unknown>;
let jwks: Record<string, unknown>;
let fetchFailure: "discovery" | "jwks" | undefined;

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function signToken(
  claims: Record<string, unknown> = {},
  options: { key?: CryptoKey; kid?: string; algorithm?: string; typ?: string; omit?: string[] } = {},
): Promise<string> {
  const omit = new Set(options.omit ?? []);
  const payload: Record<string, unknown> = { role: "admin" };
  let signer = new SignJWT(payload).setProtectedHeader({
    alg: options.algorithm ?? "RS256",
    kid: options.kid ?? "primary",
    typ: options.typ ?? "JWT",
  });
  if (!omit.has("iss")) signer = signer.setIssuer(typeof claims.iss === "string" ? claims.iss : ISSUER);
  if (!omit.has("aud")) signer = signer.setAudience(typeof claims.aud === "string" ? claims.aud : AUDIENCE);
  if (!omit.has("sub")) signer = signer.setSubject(typeof claims.sub === "string" ? claims.sub : "operator-1");
  if (!omit.has("iat")) signer = signer.setIssuedAt(typeof claims.iat === "number" ? claims.iat : now);
  if (!omit.has("exp")) signer = signer.setExpirationTime(typeof claims.exp === "number" ? claims.exp : now + 300);
  if (typeof claims.nbf === "number") signer = signer.setNotBefore(claims.nbf);
  for (const [name, value] of Object.entries(claims)) {
    if (!["iss", "aud", "sub", "iat", "exp", "nbf"].includes(name)) payload[name] = value;
  }
  return signer.sign(options.key ?? privateKey);
}

beforeAll(async () => {
  const primary = await generateKeyPair("RS256");
  const alternate = await generateKeyPair("RS256");
  privateKey = primary.privateKey;
  alternateKey = alternate.privateKey;
  publicJwk = { ...(await exportJWK(primary.publicKey)), kid: "primary", alg: "RS256", use: "sig" };
  alternateJwk = { ...(await exportJWK(alternate.publicKey)), kid: "alternate", alg: "RS256", use: "sig" };
});

beforeEach(async () => {
  now = Math.floor(Date.now() / 1000);
  discoveryFetches = 0;
  jwksFetches = 0;
  fetchFailure = undefined;
  discovery = {
    issuer: ISSUER,
    jwks_uri: JWKS_URL,
    id_token_signing_alg_values_supported: ["RS256"],
  };
  jwks = { keys: [publicJwk] };
  vi.stubGlobal("fetch", vi.fn(async (input: string | URL | Request) => {
    const url = String(input);
    if (url === DISCOVERY_URL) {
      discoveryFetches += 1;
      return fetchFailure === "discovery" ? jsonResponse({}, 503) : jsonResponse(discovery);
    }
    if (url === JWKS_URL) {
      jwksFetches += 1;
      return fetchFailure === "jwks" ? jsonResponse({}, 503) : jsonResponse(jwks);
    }
    throw new Error("Unexpected request");
  }));
  const { resetAuthCacheForTesting } = await import("./auth");
  resetAuthCacheForTesting();
});

describe("verifyAccessToken", () => {
  it("accepts a signed, bound admin token and caches discovery and JWKS", async () => {
    const { hasAdminClaim, verifyAccessToken } = await import("./auth");
    const token = await signToken();
    const [first, second] = await Promise.all([verifyAccessToken(token), verifyAccessToken(token)]);
    expect(first.sub).toBe("operator-1");
    expect(second.aud).toBe(AUDIENCE);
    expect(hasAdminClaim(first)).toBe(true);
    expect(discoveryFetches).toBe(1);
    expect(jwksFetches).toBe(1);
  });

  it.each([
    ["bad signature", () => signToken({}, { key: alternateKey })],
    ["unknown kid", () => signToken({}, { kid: "missing" })],
    ["wrong issuer", () => signToken({ iss: "https://foreign.example" })],
    ["wrong audience", () => signToken({ aud: "other" })],
    ["expired", () => signToken({ exp: now - 1 })],
    ["not yet valid", () => signToken({ nbf: now + 60 })],
    ["future issued-at", () => signToken({ iat: now + 60 })],
    ["wrong type", () => signToken({}, { typ: "ID" })],
  ])("rejects %s", async (_name, makeToken) => {
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken(await makeToken())).rejects.toBeInstanceOf(Error);
  });

  it.each(["exp", "iss", "aud", "sub", "iat"])("requires %s", async (claim) => {
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken(await signToken({}, { omit: [claim] }))).rejects.toBeInstanceOf(Error);
  });

  it("rejects malformed and unsecured tokens", async () => {
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken("not-a-jwt")).rejects.toBeInstanceOf(Error);
    const unsecured = new UnsecuredJWT({ role: "admin", iss: ISSUER, aud: AUDIENCE, sub: "operator-1", exp: now + 60, iat: now }).encode();
    await expect(verifyAccessToken(unsecured)).rejects.toBeInstanceOf(Error);
  });

  it("rejects algorithms not advertised by discovery", async () => {
    const ec = await generateKeyPair("ES256");
    const { verifyAccessToken } = await import("./auth");
    const token = await new SignJWT({ role: "admin" })
      .setProtectedHeader({ alg: "ES256", kid: "ec", typ: "JWT" })
      .setIssuer(ISSUER).setAudience(AUDIENCE).setSubject("operator-1").setIssuedAt(now).setExpirationTime(now + 60)
      .sign(ec.privateKey);
    await expect(verifyAccessToken(token)).rejects.toBeInstanceOf(Error);
  });

  it("refreshes once for key rotation and then accepts the new key", async () => {
    const { verifyAccessToken } = await import("./auth");
    await verifyAccessToken(await signToken());
    jwks = { keys: [alternateJwk] };
    const rotated = await signToken({}, { key: alternateKey, kid: "alternate" });
    await expect(verifyAccessToken(rotated)).resolves.toMatchObject({ sub: "operator-1" });
    expect(discoveryFetches).toBe(2);
    expect(jwksFetches).toBe(2);
  });

  it.each(["discovery", "jwks"] as const)("fails closed when %s fetch fails and does not cache failure", async (target) => {
    const { verifyAccessToken } = await import("./auth");
    fetchFailure = target;
    const token = await signToken();
    await expect(verifyAccessToken(token)).rejects.toBeInstanceOf(Error);
    fetchFailure = undefined;
    await expect(verifyAccessToken(token)).resolves.toMatchObject({ sub: "operator-1" });
    expect(discoveryFetches).toBeGreaterThanOrEqual(1);
    expect(jwksFetches).toBeGreaterThanOrEqual(target === "jwks" ? 2 : 1);
  });

  it.each([
    ["issuer mismatch", () => { discovery.issuer = "https://foreign.example"; }],
    ["cross-origin JWKS", () => { discovery.jwks_uri = "https://keys.example/jwks"; }],
    ["insecure JWKS", () => { discovery.jwks_uri = "http://issuer.example/keys"; }],
    ["empty algorithms", () => { discovery.id_token_signing_alg_values_supported = []; }],
    ["none algorithm", () => { discovery.id_token_signing_alg_values_supported = ["none"]; }],
    ["too many algorithms", () => { discovery.id_token_signing_alg_values_supported = Array.from({ length: 17 }, (_, index) => `A${index}`); }],
    ["empty JWKS", () => { jwks = { keys: [] }; }],
    ["too many keys", () => { jwks = { keys: Array.from({ length: 33 }, (_, index) => ({ ...publicJwk, kid: `key-${index}` })) }; }],
  ])("rejects bounded discovery/JWKS case: %s", async (_name, mutate) => {
    mutate();
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken(await signToken())).rejects.toBeInstanceOf(Error);
  });
});

describe("hasAdminClaim", () => {
  it.each([undefined, ["admin"], 1, { value: "admin" }])("rejects non-string value %j", async (role) => {
    const { hasAdminClaim } = await import("./auth");
    expect(hasAdminClaim({ role } as never)).toBe(false);
  });
});
