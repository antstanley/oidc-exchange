import { env } from "$env/dynamic/private";
import { createLocalJWKSet, decodeProtectedHeader, jwtVerify, type JWTPayload } from "jose";

const DISCOVERY_PATH = "/.well-known/openid-configuration";
const CACHE_TTL_MS = 300_000;
const FETCH_TIMEOUT_MS = 5_000;
const MAX_RESPONSE_BYTES = 1_048_576;
const MAX_ALGORITHMS = 16;
const MAX_JWKS_KEYS = 32;
const OIDC_EXCHANGE_URL = requireEnvironment("OIDC_EXCHANGE_URL");
const OIDC_EXCHANGE_ISSUER = requireEnvironment("OIDC_EXCHANGE_ISSUER");
const ADMIN_UI_AUDIENCE = requireEnvironment("ADMIN_UI_AUDIENCE");
const REQUIRED_CLAIM = env.REQUIRED_CLAIM || "role";
const REQUIRED_VALUE = env.REQUIRED_VALUE || "admin";

interface DiscoveryDocument {
  issuer: string;
  jwksUri: string;
  algorithms: string[];
}

interface CachedVerificationMaterial {
  discovery: DiscoveryDocument;
  jwks: ReturnType<typeof createLocalJWKSet>;
  cachedAt: number;
}

export type VerifiedAccessTokenClaims = Readonly<JWTPayload & Record<string, unknown>>;

let cachedMaterial: CachedVerificationMaterial | undefined;
let materialRequest: Promise<CachedVerificationMaterial> | undefined;

function requireEnvironment(name: string): string {
  const value = env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function requireHttpsUrl(value: string, name: string): URL {
  const url = new URL(value);
  if (url.protocol !== "https:") throw new Error(`${name} must use HTTPS`);
  if (url.username || url.password || url.hash) throw new Error(`${name} contains forbidden URL components`);
  return url;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function fetchJson(url: URL, name: string): Promise<unknown> {
  const response = await fetch(url, {
    headers: { accept: "application/json" },
    redirect: "error",
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (!response.ok) throw new Error(`${name} fetch failed`);
  const declaredLength = Number(response.headers.get("content-length"));
  if (Number.isFinite(declaredLength) && declaredLength > MAX_RESPONSE_BYTES) {
    throw new Error(`${name} response is too large`);
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > MAX_RESPONSE_BYTES) throw new Error(`${name} response is too large`);
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    throw new Error(`${name} response is not valid JSON`);
  }
}

function parseDiscovery(value: unknown, exchangeUrl: URL): DiscoveryDocument {
  if (!isRecord(value)) throw new Error("Discovery document must be an object");
  if (value.issuer !== OIDC_EXCHANGE_ISSUER) throw new Error("Discovery issuer mismatch");
  if (typeof value.jwks_uri !== "string") throw new Error("Discovery jwks_uri is required");
  const jwksUrl = requireHttpsUrl(value.jwks_uri, "Discovery jwks_uri");
  if (jwksUrl.origin !== exchangeUrl.origin) throw new Error("Discovery jwks_uri origin mismatch");
  const rawAlgorithms = value.id_token_signing_alg_values_supported;
  if (!Array.isArray(rawAlgorithms) || rawAlgorithms.length === 0 || rawAlgorithms.length > MAX_ALGORITHMS) {
    throw new Error("Discovery signing algorithms are invalid");
  }
  const algorithms = rawAlgorithms.filter(
    (algorithm): algorithm is string => typeof algorithm === "string" && algorithm.length > 0 && algorithm !== "none",
  );
  if (algorithms.length !== rawAlgorithms.length || new Set(algorithms).size !== algorithms.length) {
    throw new Error("Discovery signing algorithms are invalid");
  }
  return { issuer: value.issuer, jwksUri: jwksUrl.href, algorithms };
}

function parseJwks(value: unknown): { keys: Record<string, unknown>[] } {
  if (!isRecord(value) || !Array.isArray(value.keys) || value.keys.length === 0 || value.keys.length > MAX_JWKS_KEYS) {
    throw new Error("JWKS is invalid");
  }
  const keys = value.keys.map((key) => {
    if (!isRecord(key) || typeof key.kty !== "string" || typeof key.kid !== "string" || key.kid.length === 0) {
      throw new Error("JWKS key is invalid");
    }
    return key;
  });
  return { keys };
}

async function loadVerificationMaterial(): Promise<CachedVerificationMaterial> {
  const exchangeUrl = requireHttpsUrl(OIDC_EXCHANGE_URL, "OIDC_EXCHANGE_URL");
  const discoveryUrl = new URL(DISCOVERY_PATH, `${exchangeUrl.origin}/`);
  const discovery = parseDiscovery(await fetchJson(discoveryUrl, "Discovery"), exchangeUrl);
  const jwks = parseJwks(await fetchJson(new URL(discovery.jwksUri), "JWKS"));
  return { discovery, jwks: createLocalJWKSet(jwks), cachedAt: Date.now() };
}

async function getVerificationMaterial(forceRefresh = false): Promise<CachedVerificationMaterial> {
  const now = Date.now();
  if (!forceRefresh && cachedMaterial && now - cachedMaterial.cachedAt < CACHE_TTL_MS) return cachedMaterial;
  if (!materialRequest) {
    materialRequest = loadVerificationMaterial()
      .then((material) => {
        cachedMaterial = material;
        return material;
      })
      .finally(() => {
        materialRequest = undefined;
      });
  }
  return materialRequest;
}

async function verifyWithMaterial(token: string, material: CachedVerificationMaterial): Promise<VerifiedAccessTokenClaims> {
  const header = decodeProtectedHeader(token);
  if (typeof header.kid !== "string" || header.kid.length === 0) throw new Error("JWT kid is required");
  if (typeof header.alg !== "string" || !material.discovery.algorithms.includes(header.alg)) {
    throw new Error("JWT algorithm is not allowed");
  }
  if (header.typ !== undefined && header.typ !== "JWT" && header.typ !== "at+jwt") {
    throw new Error("JWT type is not allowed");
  }
  const { payload } = await jwtVerify(token, material.jwks, {
    algorithms: material.discovery.algorithms,
    issuer: OIDC_EXCHANGE_ISSUER,
    audience: ADMIN_UI_AUDIENCE,
    requiredClaims: ["exp", "iss", "aud", "sub", "iat"],
    typ: header.typ,
  });
  if (typeof payload.sub !== "string" || payload.sub.length === 0) throw new Error("JWT sub is invalid");
  if (typeof payload.iat !== "number" || payload.iat > Math.floor(Date.now() / 1000)) {
    throw new Error("JWT iat is invalid");
  }
  return Object.freeze(payload) as VerifiedAccessTokenClaims;
}

export async function verifyAccessToken(token: string): Promise<VerifiedAccessTokenClaims> {
  if (!token || token.length > MAX_RESPONSE_BYTES) throw new Error("JWT is invalid");
  const material = await getVerificationMaterial();
  try {
    return await verifyWithMaterial(token, material);
  } catch (error) {
    const refreshed = await getVerificationMaterial(true);
    if (refreshed === material) throw error;
    return verifyWithMaterial(token, refreshed);
  }
}

export function hasAdminClaim(payload: VerifiedAccessTokenClaims): boolean {
  const value = payload[REQUIRED_CLAIM];
  return typeof value === "string" && value === REQUIRED_VALUE;
}

export function getOidcExchangeUrl(): string {
  return OIDC_EXCHANGE_URL;
}

export function resetAuthCacheForTesting(): void {
  if (import.meta.env.MODE !== "test") throw new Error("Auth cache reset is test-only");
  cachedMaterial = undefined;
  materialRequest = undefined;
}
