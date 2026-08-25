import { env } from "$env/dynamic/private";
import {
  createLocalJWKSet,
  decodeProtectedHeader,
  errors,
  jwtVerify,
  type JSONWebKeySet,
  type JWTPayload,
} from "jose";

const DISCOVERY_PATH = "/.well-known/openid-configuration";
const CACHE_TTL_MS = 300_000;
const FETCH_TIMEOUT_MS = 5_000;
const MAX_RESPONSE_BYTES = 1_048_576;
const MAX_ALGORITHMS = 9;
const MAX_JWKS_KEYS = 32;
const MAX_TOKEN_BYTES = 16_384;
const MAX_KID_CHARS = 128;
const MAX_JOSE_NAME_CHARS = 16;
const MAX_CLAIM_NAME_CHARS = 128;
const MAX_CLAIM_VALUE_CHARS = 512;
const MAX_ISSUER_CHARS = 2_048;
const MAX_SUBJECT_CHARS = 512;
const MAX_AUDIENCE_CHARS = 512;
const MAX_AUDIENCES = 8;
const MAX_JWK_MATERIAL_CHARS = 1_024;
const MAX_TOKEN_AGE_SECONDS = 3_600;
const MAX_TOKEN_LIFETIME_SECONDS = 3_600;
const CLOCK_TOLERANCE_SECONDS = 30;
const REFRESH_COOLDOWN_MS = 30_000;
const MAX_NEGATIVE_KIDS = 128;
const KID_PATTERN = /^[A-Za-z0-9._~-]+$/;
const BASE64URL_PATTERN = /^[A-Za-z0-9_-]+$/;
const SUPPORTED_ALGORITHMS = Object.freeze([
  "EdDSA",
  "RS256",
  "RS384",
  "RS512",
  "PS256",
  "PS384",
  "PS512",
  "ES256",
  "ES384",
  "ES512",
] as const);
type SupportedAlgorithm = (typeof SUPPORTED_ALGORITHMS)[number];

const OIDC_EXCHANGE_URL = requireEnvironment("OIDC_EXCHANGE_URL", MAX_ISSUER_CHARS);
const OIDC_EXCHANGE_ISSUER = requireEnvironment("OIDC_EXCHANGE_ISSUER", MAX_ISSUER_CHARS);
const ADMIN_UI_AUDIENCE = requireEnvironment("ADMIN_UI_AUDIENCE", MAX_AUDIENCE_CHARS);
const REQUIRED_CLAIM = requireOptionalEnvironment("REQUIRED_CLAIM", "role", MAX_CLAIM_NAME_CHARS);
const REQUIRED_VALUE = requireOptionalEnvironment("REQUIRED_VALUE", "admin", MAX_CLAIM_VALUE_CHARS);

interface DiscoveryDocument {
  issuer: string;
  jwksUri: string;
  algorithms: SupportedAlgorithm[];
}

interface CachedVerificationMaterial {
  discovery: DiscoveryDocument;
  jwks: ReturnType<typeof createLocalJWKSet>;
  kids: ReadonlySet<string>;
  cachedAt: number;
}

export type VerifiedAccessTokenClaims = Readonly<JWTPayload & Record<string, unknown>>;

let cachedMaterial: CachedVerificationMaterial | undefined;
let materialRequest: Promise<CachedVerificationMaterial> | undefined;
let lastRefreshAt = Number.NEGATIVE_INFINITY;
const negativeKids = new Map<string, number>();

function requireEnvironment(name: string, maxChars: number): string {
  const value = env[name]?.trim();
  if (!value || value.length > maxChars) throw new Error(`${name} is invalid`);
  return value;
}

function requireOptionalEnvironment(name: string, fallback: string, maxChars: number): string {
  const raw = env[name];
  const value = raw === undefined || raw === "" ? fallback : raw;
  if (value.length > maxChars) throw new Error(`${name} is invalid`);
  return value;
}

function requireHttpsUrl(value: string, name: string): URL {
  if (value.length > MAX_ISSUER_CHARS) throw new Error(`${name} is too long`);
  const url = new URL(value);
  if (url.protocol !== "https:") throw new Error(`${name} must use HTTPS`);
  if (url.username || url.password || url.hash)
    throw new Error(`${name} contains forbidden URL components`);
  return url;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireBoundedString(value: unknown, name: string, maxChars: number): string {
  if (typeof value !== "string" || value.length === 0 || value.length > maxChars) {
    throw new Error(`${name} is invalid`);
  }
  return value;
}

function parseContentLength(value: string | null, name: string): number | undefined {
  if (value === null) return undefined;
  if (!/^(0|[1-9][0-9]*)$/.test(value)) throw new Error(`${name} content length is invalid`);
  const length = Number(value);
  if (!Number.isSafeInteger(length) || length > MAX_RESPONSE_BYTES) {
    throw new Error(`${name} response is too large`);
  }
  return length;
}

function requireJsonContentType(value: string | null, name: string): void {
  if (value === null) throw new Error(`${name} content type is invalid`);
  const mediaType = value.split(";", 1)[0]!.trim().toLowerCase();
  if (mediaType !== "application/json" && !mediaType.endsWith("+json")) {
    throw new Error(`${name} content type is invalid`);
  }
}

async function readBoundedBody(
  response: Response,
  name: string,
  signal: AbortSignal,
): Promise<Uint8Array> {
  if (!response.body) throw new Error(`${name} response body is missing`);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  const abort = () => {
    void reader.cancel().catch(() => undefined);
  };
  signal.addEventListener("abort", abort, { once: true });
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (signal.aborted) throw new Error(`${name} fetch timed out`);
      if (done) break;
      total += value.byteLength;
      if (total > MAX_RESPONSE_BYTES) {
        void reader.cancel().catch(() => undefined);
        throw new Error(`${name} response is too large`);
      }
      chunks.push(value);
    }
  } catch (error) {
    void reader.cancel().catch(() => undefined);
    throw error;
  } finally {
    signal.removeEventListener("abort", abort);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

async function fetchJson(url: URL, name: string): Promise<unknown> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  try {
    const response = await fetch(url, {
      headers: { accept: "application/json" },
      redirect: "error",
      signal: controller.signal,
    });
    const finalUrl = response.url || url.href;
    if (!response.ok || response.redirected || new URL(finalUrl).origin !== url.origin) {
      throw new Error(`${name} fetch failed`);
    }
    requireJsonContentType(response.headers.get("content-type"), name);
    parseContentLength(response.headers.get("content-length"), name);
    const bytes = await readBoundedBody(response, name, controller.signal);
    try {
      return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    } catch {
      throw new Error(`${name} response is not valid JSON`);
    }
  } finally {
    clearTimeout(timer);
  }
}

function isSupportedAlgorithm(value: string): value is SupportedAlgorithm {
  return (SUPPORTED_ALGORITHMS as readonly string[]).includes(value);
}

function parseDiscovery(value: unknown, exchangeUrl: URL): DiscoveryDocument {
  if (!isRecord(value)) throw new Error("Discovery document must be an object");
  const issuer = requireBoundedString(value.issuer, "Discovery issuer", MAX_ISSUER_CHARS);
  if (issuer !== OIDC_EXCHANGE_ISSUER) throw new Error("Discovery issuer mismatch");
  const jwksUri = requireBoundedString(value.jwks_uri, "Discovery jwks_uri", MAX_ISSUER_CHARS);
  const jwksUrl = requireHttpsUrl(jwksUri, "Discovery jwks_uri");
  if (jwksUrl.origin !== exchangeUrl.origin) throw new Error("Discovery jwks_uri origin mismatch");
  const rawAlgorithms = value.id_token_signing_alg_values_supported;
  if (
    !Array.isArray(rawAlgorithms) ||
    rawAlgorithms.length === 0 ||
    rawAlgorithms.length > MAX_ALGORITHMS
  ) {
    throw new Error("Discovery signing algorithms are invalid");
  }
  const algorithms = rawAlgorithms.map((algorithm) => {
    const name = requireBoundedString(
      algorithm,
      "Discovery signing algorithm",
      MAX_JOSE_NAME_CHARS,
    );
    if (!isSupportedAlgorithm(name)) throw new Error("Discovery signing algorithm is unsupported");
    return name;
  });
  if (new Set(algorithms).size !== algorithms.length) {
    throw new Error("Discovery signing algorithms are invalid");
  }
  return { issuer, jwksUri: jwksUrl.href, algorithms };
}

function decodeBase64url(value: unknown, name: string, expectedBytes?: number): Uint8Array {
  const encoded = requireBoundedString(value, name, MAX_JWK_MATERIAL_CHARS);
  if (!BASE64URL_PATTERN.test(encoded)) throw new Error(`${name} is invalid`);
  const bytes = Uint8Array.from(Buffer.from(encoded, "base64url"));
  if (expectedBytes !== undefined && bytes.byteLength !== expectedBytes) {
    throw new Error(`${name} is invalid`);
  }
  return bytes;
}

function validateJwkForAlgorithm(
  key: Record<string, unknown>,
  algorithm: SupportedAlgorithm,
): void {
  if (algorithm === "EdDSA") {
    if (key.kty !== "OKP" || key.crv !== "Ed25519") throw new Error("JWKS key type is invalid");
    decodeBase64url(key.x, "JWKS x", 32);
    return;
  }
  if (algorithm.startsWith("ES")) {
    const rules = {
      ES256: ["P-256", 32],
      ES384: ["P-384", 48],
      ES512: ["P-521", 66],
    } as const;
    const [curve, bytes] = rules[algorithm as keyof typeof rules];
    if (key.kty !== "EC" || key.crv !== curve) throw new Error("JWKS key type is invalid");
    decodeBase64url(key.x, "JWKS x", bytes);
    decodeBase64url(key.y, "JWKS y", bytes);
    return;
  }
  if (key.kty !== "RSA") throw new Error("JWKS key type is invalid");
  const modulus = decodeBase64url(key.n, "JWKS modulus");
  decodeBase64url(key.e, "JWKS exponent");
  if (modulus.byteLength < 256 || (modulus[0]! & 0x80) === 0) {
    throw new Error("JWKS RSA modulus is too weak");
  }
}

function validateKeyOperations(key: Record<string, unknown>): void {
  if (key.use !== undefined && key.use !== "sig") throw new Error("JWKS use is invalid");
  if (key.key_ops === undefined) return;
  if (!Array.isArray(key.key_ops) || key.key_ops.length !== 1 || key.key_ops[0] !== "verify") {
    throw new Error("JWKS key_ops is invalid");
  }
}

function parseJwks(value: unknown, algorithms: readonly SupportedAlgorithm[]): JSONWebKeySet {
  if (
    !isRecord(value) ||
    !Array.isArray(value.keys) ||
    value.keys.length === 0 ||
    value.keys.length > MAX_JWKS_KEYS
  ) {
    throw new Error("JWKS is invalid");
  }
  const kids = new Set<string>();
  const keys = value.keys.map((rawKey) => {
    if (!isRecord(rawKey)) throw new Error("JWKS key is invalid");
    const kid = requireBoundedString(rawKey.kid, "JWKS kid", MAX_KID_CHARS);
    if (!KID_PATTERN.test(kid) || kids.has(kid)) throw new Error("JWKS kid is invalid");
    kids.add(kid);
    const algorithm = requireBoundedString(rawKey.alg, "JWKS alg", MAX_JOSE_NAME_CHARS);
    if (!isSupportedAlgorithm(algorithm) || !algorithms.includes(algorithm)) {
      throw new Error("JWKS algorithm is invalid");
    }
    validateKeyOperations(rawKey);
    validateJwkForAlgorithm(rawKey, algorithm);
    return rawKey;
  });
  return { keys } as JSONWebKeySet;
}

async function loadVerificationMaterial(): Promise<CachedVerificationMaterial> {
  const exchangeUrl = requireHttpsUrl(OIDC_EXCHANGE_URL, "OIDC_EXCHANGE_URL");
  const discoveryUrl = new URL(DISCOVERY_PATH, `${exchangeUrl.origin}/`);
  const discovery = parseDiscovery(await fetchJson(discoveryUrl, "Discovery"), exchangeUrl);
  const parsedJwks = parseJwks(
    await fetchJson(new URL(discovery.jwksUri), "JWKS"),
    discovery.algorithms,
  );
  const kids = new Set(parsedJwks.keys.map((key) => key.kid!));
  return { discovery, jwks: createLocalJWKSet(parsedJwks), kids, cachedAt: Date.now() };
}

async function getVerificationMaterial(forceRefresh = false): Promise<CachedVerificationMaterial> {
  const now = Date.now();
  if (!forceRefresh && cachedMaterial && now - cachedMaterial.cachedAt < CACHE_TTL_MS)
    return cachedMaterial;
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

function parseBoundedHeader(token: string): { kid: string; alg: SupportedAlgorithm; typ?: string } {
  const headerSegment = token.split(".", 1)[0]!;
  if (headerSegment.length === 0 || headerSegment.length > 1_024)
    throw new Error("JWT header is invalid");
  const header = decodeProtectedHeader(token);
  const kid = requireBoundedString(header.kid, "JWT kid", MAX_KID_CHARS);
  if (!KID_PATTERN.test(kid)) throw new Error("JWT kid is invalid");
  const algorithm = requireBoundedString(header.alg, "JWT algorithm", MAX_JOSE_NAME_CHARS);
  if (!isSupportedAlgorithm(algorithm)) throw new Error("JWT algorithm is not allowed");
  if (header.typ !== undefined) {
    const typ = requireBoundedString(header.typ, "JWT type", MAX_JOSE_NAME_CHARS);
    if (typ !== "JWT" && typ !== "at+jwt") throw new Error("JWT type is not allowed");
    return { kid, alg: algorithm, typ };
  }
  return { kid, alg: algorithm };
}

function validatePayloadBounds(payload: JWTPayload): void {
  requireBoundedString(payload.iss, "JWT iss", MAX_ISSUER_CHARS);
  requireBoundedString(payload.sub, "JWT sub", MAX_SUBJECT_CHARS);
  const audiences = typeof payload.aud === "string" ? [payload.aud] : payload.aud;
  if (!Array.isArray(audiences) || audiences.length === 0 || audiences.length > MAX_AUDIENCES) {
    throw new Error("JWT aud is invalid");
  }
  for (const audience of audiences)
    requireBoundedString(audience, "JWT audience", MAX_AUDIENCE_CHARS);
  if (typeof payload.iat !== "number" || typeof payload.exp !== "number")
    throw new Error("JWT times are invalid");
  const now = Math.floor(Date.now() / 1000);
  if (payload.exp <= payload.iat || payload.iat > now + CLOCK_TOLERANCE_SECONDS)
    throw new Error("JWT times are invalid");
  if (now - payload.iat > MAX_TOKEN_AGE_SECONDS + CLOCK_TOLERANCE_SECONDS)
    throw new Error("JWT is too old");
  if (payload.exp - payload.iat > MAX_TOKEN_LIFETIME_SECONDS)
    throw new Error("JWT lifetime is too long");
}

async function verifyWithMaterial(
  token: string,
  material: CachedVerificationMaterial,
): Promise<VerifiedAccessTokenClaims> {
  const header = parseBoundedHeader(token);
  if (!material.discovery.algorithms.includes(header.alg))
    throw new Error("JWT algorithm is not allowed");
  const { payload } = await jwtVerify(token, material.jwks, {
    algorithms: material.discovery.algorithms,
    issuer: OIDC_EXCHANGE_ISSUER,
    audience: ADMIN_UI_AUDIENCE,
    requiredClaims: ["exp", "iss", "aud", "sub", "iat"],
    typ: header.typ,
    clockTolerance: CLOCK_TOLERANCE_SECONDS,
  });
  validatePayloadBounds(payload);
  return Object.freeze(payload) as VerifiedAccessTokenClaims;
}

function rememberNegativeKid(kid: string): void {
  negativeKids.delete(kid);
  negativeKids.set(kid, Date.now());
  while (negativeKids.size > MAX_NEGATIVE_KIDS)
    negativeKids.delete(negativeKids.keys().next().value!);
}

function canRefresh(kid: string): boolean {
  const now = Date.now();
  const seenAt = negativeKids.get(kid);
  return seenAt === undefined && now - lastRefreshAt >= REFRESH_COOLDOWN_MS;
}

function isSignatureFailure(error: unknown): boolean {
  return error instanceof errors.JWSSignatureVerificationFailed;
}

export async function verifyAccessToken(token: string): Promise<VerifiedAccessTokenClaims> {
  if (!token || token.length > MAX_TOKEN_BYTES) throw new Error("JWT is invalid");
  const header = parseBoundedHeader(token);
  const material = await getVerificationMaterial();
  if (!material.discovery.algorithms.includes(header.alg)) {
    throw new Error("JWT algorithm is not allowed");
  }
  const unknownKid = !material.kids.has(header.kid);
  try {
    return await verifyWithMaterial(token, material);
  } catch (error) {
    const rotationFailure = unknownKid || isSignatureFailure(error);
    if (!rotationFailure) throw error;
    if (!canRefresh(header.kid)) {
      if (materialRequest) return verifyWithMaterial(token, await materialRequest);
      throw error;
    }
    lastRefreshAt = Date.now();
    const refreshed = await getVerificationMaterial(true);
    try {
      return await verifyWithMaterial(token, refreshed);
    } catch (refreshError) {
      rememberNegativeKid(header.kid);
      throw refreshError;
    }
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
  lastRefreshAt = Number.NEGATIVE_INFINITY;
  negativeKids.clear();
}
