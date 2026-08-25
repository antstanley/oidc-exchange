import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { exportJWK, generateKeyPair, SignJWT, UnsecuredJWT } from "jose";

const ISSUER = "https://issuer.example";
const AUDIENCE = "admin-ui";
const DISCOVERY_URL = `${ISSUER}/.well-known/openid-configuration`;
const JWKS_URL = `${ISSUER}/keys`;
const MAX_RESPONSE_BYTES = 1_048_576;
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
let responseFactory: ((value: unknown, name: "discovery" | "jwks") => Response) | undefined;

function jsonResponse(value: unknown, status = 200, url = ""): Response {
  const response = new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
  if (url) Object.defineProperty(response, "url", { value: url });
  return response;
}

function streamResponse(
  chunks: Uint8Array[],
  options: { contentType?: string; contentLength?: string; stall?: boolean; url?: string } = {},
): { response: Response; cancelled: ReturnType<typeof vi.fn> } {
  const cancelled = vi.fn();
  let index = 0;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk);
      index = chunks.length;
      if (!options.stall) controller.close();
    },
    pull(controller) {
      if (index < chunks.length) controller.enqueue(chunks[index++]);
    },
    cancel: cancelled,
  });
  const headers = new Headers();
  if (options.contentType !== undefined) headers.set("content-type", options.contentType);
  if (options.contentLength !== undefined) headers.set("content-length", options.contentLength);
  const response = new Response(body, { headers });
  if (options.url) Object.defineProperty(response, "url", { value: options.url });
  return { response, cancelled };
}

async function signToken(
  claims: Record<string, unknown> = {},
  options: {
    key?: CryptoKey;
    kid?: string;
    algorithm?: string;
    typ?: string;
    omit?: string[];
  } = {},
): Promise<string> {
  const omit = new Set(options.omit ?? []);
  const payload: Record<string, unknown> = { role: "admin" };
  let signer = new SignJWT(payload).setProtectedHeader({
    alg: options.algorithm ?? "RS256",
    kid: options.kid ?? "primary",
    typ: options.typ ?? "JWT",
  });
  if (!omit.has("iss")) signer = signer.setIssuer((claims.iss as string) ?? ISSUER);
  if (!omit.has("aud")) signer = signer.setAudience((claims.aud as string | string[]) ?? AUDIENCE);
  if (!omit.has("sub")) signer = signer.setSubject((claims.sub as string) ?? "operator-1");
  if (!omit.has("iat")) signer = signer.setIssuedAt((claims.iat as number) ?? now);
  if (!omit.has("exp")) signer = signer.setExpirationTime((claims.exp as number) ?? now + 300);
  if (typeof claims.nbf === "number") signer = signer.setNotBefore(claims.nbf);
  for (const [name, value] of Object.entries(claims)) {
    if (!["iss", "aud", "sub", "iat", "exp", "nbf"].includes(name)) payload[name] = value;
  }
  return signer.sign(options.key ?? privateKey);
}

beforeAll(async () => {
  const primary = await generateKeyPair("RS256", { modulusLength: 2048 });
  const alternate = await generateKeyPair("RS256", { modulusLength: 2048 });
  privateKey = primary.privateKey;
  alternateKey = alternate.privateKey;
  publicJwk = { ...(await exportJWK(primary.publicKey)), kid: "primary", alg: "RS256", use: "sig" };
  alternateJwk = {
    ...(await exportJWK(alternate.publicKey)),
    kid: "alternate",
    alg: "RS256",
    use: "sig",
  };
});

beforeEach(async () => {
  vi.useRealTimers();
  now = Math.floor(Date.now() / 1000);
  discoveryFetches = 0;
  jwksFetches = 0;
  fetchFailure = undefined;
  responseFactory = undefined;
  discovery = {
    issuer: ISSUER,
    jwks_uri: JWKS_URL,
    id_token_signing_alg_values_supported: ["RS256"],
  };
  jwks = { keys: [publicJwk] };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string | URL | Request) => {
      const url = String(input);
      if (url === DISCOVERY_URL) {
        discoveryFetches += 1;
        if (fetchFailure === "discovery") return jsonResponse({}, 503, url);
        return responseFactory?.(discovery, "discovery") ?? jsonResponse(discovery, 200, url);
      }
      if (url === JWKS_URL) {
        jwksFetches += 1;
        if (fetchFailure === "jwks") return jsonResponse({}, 503, url);
        return responseFactory?.(jwks, "jwks") ?? jsonResponse(jwks, 200, url);
      }
      throw new Error("Unexpected request");
    }),
  );
  const { resetAuthCacheForTesting } = await import("./auth");
  resetAuthCacheForTesting();
});

async function rejects(token = signToken()): Promise<void> {
  const { verifyAccessToken } = await import("./auth");
  await expect(verifyAccessToken(await token)).rejects.toBeInstanceOf(Error);
}

describe("verifyAccessToken cryptographic policy", () => {
  it("accepts a signed, bound admin token and caches discovery and JWKS", async () => {
    const { hasAdminClaim, verifyAccessToken } = await import("./auth");
    const token = await signToken();
    const [first, second] = await Promise.all([verifyAccessToken(token), verifyAccessToken(token)]);
    expect(first.sub).toBe("operator-1");
    expect(second.aud).toBe(AUDIENCE);
    expect(hasAdminClaim(first)).toBe(true);
    expect([discoveryFetches, jwksFetches]).toEqual([1, 1]);
  });

  it("rejects none, symmetric, unsupported future, duplicate, and mixed algorithms", async () => {
    for (const algorithms of [
      ["none"],
      ["HS256"],
      ["ML-DSA"],
      ["RS256", "RS256"],
      ["RS256", "ML-DSA"],
    ]) {
      discovery.id_token_signing_alg_values_supported = algorithms;
      await rejects();
      const { resetAuthCacheForTesting } = await import("./auth");
      resetAuthCacheForTesting();
    }
  });

  it("rejects a token algorithm outside discovery without refetching", async () => {
    const ec = await generateKeyPair("ES256");
    const token = await new SignJWT({ role: "admin" })
      .setProtectedHeader({ alg: "ES256", kid: "ec", typ: "JWT" })
      .setIssuer(ISSUER)
      .setAudience(AUDIENCE)
      .setSubject("operator-1")
      .setIssuedAt(now)
      .setExpirationTime(now + 60)
      .sign(ec.privateKey);
    await rejects(Promise.resolve(token));
    expect([discoveryFetches, jwksFetches]).toEqual([1, 1]);
  });
});

describe("strict JWKS validation", () => {
  it.each([
    ["duplicate kid", () => ({ keys: [publicJwk, { ...alternateJwk, kid: "primary" }] })],
    ["bad use", () => ({ keys: [{ ...publicJwk, use: "enc" }] })],
    ["bad key_ops", () => ({ keys: [{ ...publicJwk, key_ops: ["sign"] }] })],
    ["unknown key_ops", () => ({ keys: [{ ...publicJwk, key_ops: ["verify", "frobnicate"] }] })],
    ["missing alg", () => ({ keys: [{ ...publicJwk, alg: undefined }] })],
    ["mismatched alg", () => ({ keys: [{ ...publicJwk, alg: "PS256" }] })],
    ["wrong type", () => ({ keys: [{ ...publicJwk, kty: "EC" }] })],
    ["weak RSA", () => ({ keys: [{ ...publicJwk, n: "_".repeat(171) }] })],
    ["oversized kid", () => ({ keys: [{ ...publicJwk, kid: "k".repeat(129) }] })],
    ["oversized material", () => ({ keys: [{ ...publicJwk, n: "A".repeat(1025) }] })],
  ])("rejects %s", async (_name, makeJwks) => {
    jwks = makeJwks();
    await rejects();
  });

  it("accepts verification-only key operations", async () => {
    jwks = { keys: [{ ...publicJwk, key_ops: ["verify"] }] };
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken(await signToken())).resolves.toMatchObject({
      sub: "operator-1",
    });
  });

  it("rejects wrong ES curve and EdDSA type/curve", async () => {
    discovery.id_token_signing_alg_values_supported = ["ES256"];
    jwks = {
      keys: [
        {
          kty: "EC",
          crv: "P-384",
          x: "A".repeat(43),
          y: "A".repeat(43),
          kid: "ec",
          alg: "ES256",
          use: "sig",
        },
      ],
    };
    await rejects();
    const { resetAuthCacheForTesting } = await import("./auth");
    resetAuthCacheForTesting();
    discovery.id_token_signing_alg_values_supported = ["EdDSA"];
    jwks = {
      keys: [
        { kty: "OKP", crv: "X25519", x: "A".repeat(43), kid: "okp", alg: "EdDSA", use: "sig" },
      ],
    };
    await rejects();
  });
});

describe("bounded streamed JSON fetch", () => {
  it("accepts absent length, chunked JSON, +json, and exact cap", async () => {
    responseFactory = (value, name) => {
      const bytes = new TextEncoder().encode(JSON.stringify(value));
      const padded = name === "discovery" ? bytes : new Uint8Array(MAX_RESPONSE_BYTES).fill(0x20);
      if (name === "jwks") padded.set(bytes);
      return streamResponse([padded.subarray(0, 13), padded.subarray(13)], {
        contentType: "application/oidc+json",
        url: name === "discovery" ? DISCOVERY_URL : JWKS_URL,
      }).response;
    };
    const { verifyAccessToken } = await import("./auth");
    await expect(verifyAccessToken(await signToken())).resolves.toBeTruthy();
  });

  it("rejects streamed overflow despite deceptive small length and cancels reader", async () => {
    let cancelled: ReturnType<typeof vi.fn> | undefined;
    responseFactory = (value, name) => {
      if (name === "discovery") return jsonResponse(value, 200, DISCOVERY_URL);
      const made = streamResponse([new Uint8Array(MAX_RESPONSE_BYTES + 1)], {
        contentType: "application/json",
        url: JWKS_URL,
      });
      cancelled = made.cancelled;
      return made.response;
    };
    await rejects();
    expect(cancelled).toBeDefined();
  });

  it.each(["-1", "1.5", "9007199254740992", String(MAX_RESPONSE_BYTES + 1)])(
    "rejects invalid content length %s",
    async (length) => {
      responseFactory = (value, name) =>
        streamResponse([new TextEncoder().encode(JSON.stringify(value))], {
          contentType: "application/json",
          contentLength: length,
          url: name === "discovery" ? DISCOVERY_URL : JWKS_URL,
        }).response;
      await rejects();
    },
  );

  it.each([undefined, "text/plain"])("rejects content type %s", async (contentType) => {
    responseFactory = (value, name) =>
      streamResponse([new TextEncoder().encode(JSON.stringify(value))], {
        contentType,
        url: name === "discovery" ? DISCOVERY_URL : JWKS_URL,
      }).response;
    await rejects();
  });

  it("rejects cross-origin final URL", async () => {
    responseFactory = (value) => jsonResponse(value, 200, "https://foreign.example/redirect");
    await rejects();
  });

  it("aborts a stalled body on timeout and cancels the reader", async () => {
    vi.useFakeTimers();
    let cancelled: ReturnType<typeof vi.fn> | undefined;
    responseFactory = (_value, name) => {
      const made = streamResponse([], {
        contentType: "application/json",
        stall: true,
        url: name === "discovery" ? DISCOVERY_URL : JWKS_URL,
      });
      cancelled = made.cancelled;
      return made.response;
    };
    const verification = rejects();
    await vi.waitFor(() => expect(cancelled).toBeDefined());
    await vi.advanceTimersByTimeAsync(5_001);
    await verification;
    expect(cancelled).toHaveBeenCalledOnce();
  });
});

describe("bounded rotation refresh", () => {
  it.each([
    ["wrong issuer", () => signToken({ iss: "https://foreign.example" })],
    ["wrong audience", () => signToken({ aud: "other" })],
    ["expired", () => signToken({ exp: now - 60 })],
    ["nbf", () => signToken({ nbf: now + 60 })],
    ["future iat", () => signToken({ iat: now + 60 })],
    ["wrong type", () => signToken({}, { typ: "ID" })],
    ["malformed", async () => "not-a-jwt"],
  ])("does not refetch for %s", async (_name, makeToken) => {
    await rejects(makeToken());
    expect([discoveryFetches, jwksFetches]).toEqual(
      _name === "malformed" || _name === "wrong type" ? [0, 0] : [1, 1],
    );
  });

  it("coalesces exactly one refresh for concurrent unknown kids", async () => {
    const { verifyAccessToken } = await import("./auth");
    await verifyAccessToken(await signToken());
    jwks = { keys: [alternateJwk] };
    const token = await signToken({}, { key: alternateKey, kid: "alternate" });
    await Promise.allSettled([verifyAccessToken(token), verifyAccessToken(token)]).then(
      (results) => {
        expect(results.every((result) => result.status === "fulfilled")).toBe(true);
      },
    );
    expect([discoveryFetches, jwksFetches]).toEqual([2, 2]);
  });

  it("bounds random-kid refresh amplification and retries after cooldown", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(now * 1000));
    const { verifyAccessToken } = await import("./auth");
    await verifyAccessToken(await signToken());
    await rejects(signToken({}, { kid: "random-1" }));
    await rejects(signToken({}, { kid: "random-2" }));
    expect([discoveryFetches, jwksFetches]).toEqual([2, 2]);
    await vi.advanceTimersByTimeAsync(30_001);
    await rejects(signToken({}, { kid: "random-2" }));
    expect([discoveryFetches, jwksFetches]).toEqual([3, 3]);
  });

  it("accepts same-kid key rotation once cooldown permits", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(now * 1000));
    const { verifyAccessToken } = await import("./auth");
    await verifyAccessToken(await signToken());
    jwks = { keys: [{ ...alternateJwk, kid: "primary" }] };
    await expect(
      verifyAccessToken(await signToken({}, { key: alternateKey })),
    ).resolves.toBeTruthy();
    expect([discoveryFetches, jwksFetches]).toEqual([2, 2]);
  });
});

describe("token age and claim bounds", () => {
  it.each([
    ["oldest accepted", { iat: () => now - 3_599, exp: () => now + 1 }, true],
    ["one over age", { iat: () => now - 3_631, exp: () => now - 31 }, false],
    ["max lifetime", { iat: () => now, exp: () => now + 3_600 }, true],
    ["one over lifetime", { iat: () => now, exp: () => now + 3_601 }, false],
    ["future skew boundary", { iat: () => now + 30, exp: () => now + 60 }, true],
    ["future skew one over", { iat: () => now + 31, exp: () => now + 60 }, false],
    ["exp equals iat", { iat: () => now, exp: () => now }, false],
    ["exp before iat", { iat: () => now, exp: () => now - 1 }, false],
  ])("enforces %s", async (_name, times, accepted) => {
    const token = signToken({ iat: times.iat(), exp: times.exp() });
    const { verifyAccessToken } = await import("./auth");
    if (accepted) {
      await expect(verifyAccessToken(await token)).resolves.toBeTruthy();
    } else await expect(verifyAccessToken(await token)).rejects.toBeInstanceOf(Error);
  });

  it.each([
    ["long token", async () => "a".repeat(16_385)],
    ["long kid", () => signToken({}, { kid: "k".repeat(129) })],
    ["long sub", () => signToken({ sub: "s".repeat(513) })],
    ["long issuer", () => signToken({ iss: "i".repeat(2049) })],
    ["long audience", () => signToken({ aud: "a".repeat(513) })],
    [
      "too many audiences",
      () => signToken({ aud: Array.from({ length: 9 }, (_, index) => `a${index}`) }),
    ],
  ])("rejects %s", async (_name, makeToken) => {
    await rejects(makeToken());
  });

  it("rejects malformed, unsecured, and missing required claims", async () => {
    await rejects(Promise.resolve("not-a-jwt"));
    await rejects(Promise.resolve(new UnsecuredJWT({ role: "admin" }).encode()));
    for (const claim of ["exp", "iss", "aud", "sub", "iat"])
      await rejects(signToken({}, { omit: [claim] }));
  });
});

describe("hasAdminClaim", () => {
  it.each([undefined, ["admin"], 1, { value: "admin" }])(
    "rejects non-string value %j",
    async (role) => {
      const { hasAdminClaim } = await import("./auth");
      expect(hasAdminClaim({ role } as never)).toBe(false);
    },
  );
});
