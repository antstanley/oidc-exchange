import { env } from '$env/dynamic/private';

const OIDC_EXCHANGE_URL = env.OIDC_EXCHANGE_URL || 'http://localhost:8080';
const REQUIRED_CLAIM = env.REQUIRED_CLAIM || 'role';
const REQUIRED_VALUE = env.REQUIRED_VALUE || 'admin';

interface JWK {
	kty: string;
	kid: string;
	[key: string]: unknown;
}

interface JWKS {
	keys: JWK[];
}

let cachedJwks: JWKS | null = null;
let jwksCachedAt = 0;
const JWKS_CACHE_TTL = 300_000; // 5 minutes

export async function getJwks(): Promise<JWKS> {
	const now = Date.now();
	if (cachedJwks && now - jwksCachedAt < JWKS_CACHE_TTL) {
		return cachedJwks;
	}
	const res = await fetch(`${OIDC_EXCHANGE_URL}/keys`);
	if (!res.ok) throw new Error(`JWKS fetch failed: ${res.status}`);
	cachedJwks = await res.json();
	jwksCachedAt = now;
	return cachedJwks!;
}

/** Decode a JWT without verification (for reading claims after JWKS verify). */
export function decodeJwtPayload(token: string): Record<string, unknown> {
	const parts = token.split('.');
	if (parts.length !== 3) throw new Error('Invalid JWT');
	const payload = Buffer.from(parts[1], 'base64url').toString('utf-8');
	return JSON.parse(payload);
}

/** Check if a decoded JWT payload has the required admin claim. */
export function hasAdminClaim(payload: Record<string, unknown>): boolean {
	return String(payload[REQUIRED_CLAIM]) === REQUIRED_VALUE;
}

/** Check if a JWT is expired. */
export function isExpired(payload: Record<string, unknown>): boolean {
	const exp = payload.exp as number;
	if (!exp) return true;
	return Date.now() / 1000 > exp;
}

export function getOidcExchangeUrl(): string {
	return OIDC_EXCHANGE_URL;
}
