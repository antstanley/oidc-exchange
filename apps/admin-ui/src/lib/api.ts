// GENERATED FILE — do not edit.
// Source: schemas/internal-api.schema.json (run `pnpm generate`).
//
// Server-side only: this module imports SvelteKit's `$env/dynamic/private`,
// so reaching it from browser code fails the build. Operator credentials
// live only in the server runtime and never reach the browser bundle.
import { env } from "$env/dynamic/private";

import { requestViaTls } from "./tls-transport";
import type { ClaimsMap, NewUser, Stats, User, UserPage, UserPatch } from "./types";

/** The documented default is the admin listener, not the public one. */
const INTERNAL_API_URL = env.INTERNAL_API_URL || "http://localhost:8081";

export class InternalApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, description: string | null) {
    super(description === null ? `${code} (${status})` : `${code} (${status}): ${description}`);
    this.name = "InternalApiError";
    this.status = status;
    this.code = code;
  }
}

export class InternalApiConfigurationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "InternalApiConfigurationError";
  }
}

interface AuthorizationCredential {
  kind: "operator_token" | "shared_secret";
  authorization: string;
}

interface ClientCertificateCredential {
  kind: "client_certificate";
  certificate: string;
  privateKey: string;
}

type OperatorCredential = AuthorizationCredential | ClientCertificateCredential;

export type OperatorCredentialKind = OperatorCredential["kind"];

function present(value: string | undefined): value is string {
  return value !== undefined && value !== "";
}

/**
 * Resolve the operator credential in the contract's documented preference
 * order. Values come from the server-side environment only and are never
 * logged; a half-configured client certificate is a configuration error,
 * never a silent downgrade to a weaker credential.
 */
export function resolveOperatorCredential(): OperatorCredential {
  const [certSet, keySet] = [present(env.INTERNAL_API_CLIENT_CERT), present(env.INTERNAL_API_CLIENT_KEY)];
  if (certSet !== keySet) {
    throw new InternalApiConfigurationError(
      "INTERNAL_API_CLIENT_CERT and INTERNAL_API_CLIENT_KEY must be configured together",
    );
  }
  if (present(env.INTERNAL_API_TOKEN)) {
    return { kind: "operator_token", authorization: `Bearer ${env.INTERNAL_API_TOKEN}` };
  }
  if (present(env.INTERNAL_API_CLIENT_CERT) && present(env.INTERNAL_API_CLIENT_KEY)) {
    return {
      kind: "client_certificate",
      certificate: env.INTERNAL_API_CLIENT_CERT,
      privateKey: env.INTERNAL_API_CLIENT_KEY,
    };
  }
  if (present(env.INTERNAL_API_SECRET)) {
    return { kind: "shared_secret", authorization: `Bearer ${env.INTERNAL_API_SECRET}` };
  }
  throw new InternalApiConfigurationError(
    "no operator credential configured: set one of INTERNAL_API_TOKEN, INTERNAL_API_CLIENT_CERT, INTERNAL_API_CLIENT_KEY, INTERNAL_API_SECRET",
  );
}

async function request(path: string, init: RequestInit = {}): Promise<unknown> {
  const url = `${INTERNAL_API_URL}${path}`;
  const credential = resolveOperatorCredential();
  let response: Response;
  if (credential.kind === "client_certificate") {
    // Mutual TLS is presented by the TLS layer itself, not a header.
    response = await requestViaTls(url, { ...init, credential });
  } else {
    const headers = new Headers(init.headers);
    headers.set("Authorization", credential.authorization);
    response = await fetch(url, { ...init, headers });
  }
  await assertOk(response);
  const text = await response.text();
  // Null-typed successes (the bare mutation verbs) ship an empty body.
  return text === "" ? null : JSON.parse(text);
}

async function assertOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }
  let code = "unknown_error";
  let description: string | null = null;
  try {
    const body = (await response.json()) as { error?: unknown; error_description?: unknown };
    if (typeof body.error === "string") code = body.error;
    if (typeof body.error_description === "string") description = body.error_description;
  } catch {
    // A non-JSON error body still surfaces its status via the envelope.
  }
  throw new InternalApiError(response.status, code, description);
}

export async function clearClaims(id: string): Promise<void> {
  const target = `/internal/users/${encodeURIComponent(id)}/claims`;
  const init: RequestInit = { method: "DELETE" };
  await request(target, init);
}

export async function createUser(body: NewUser): Promise<User> {
  const target = "/internal/users";
  const init: RequestInit = {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
  return (await request(target, init)) as User;
}

export async function deleteUser(id: string): Promise<void> {
  const target = `/internal/users/${encodeURIComponent(id)}`;
  const init: RequestInit = { method: "DELETE" };
  await request(target, init);
}

export async function getStats(): Promise<Stats> {
  const target = "/internal/stats";
  return (await request(target)) as Stats;
}

export async function getUser(id: string): Promise<User | null> {
  const target = `/internal/users/${encodeURIComponent(id)}`;
  try {
    return (await request(target)) as User;
  } catch (error) {
    if (error instanceof InternalApiError && error.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function getUserClaims(id: string): Promise<ClaimsMap> {
  const target = `/internal/users/${encodeURIComponent(id)}/claims`;
  return (await request(target)) as ClaimsMap;
}

export async function listUsersPage({
  cursor,
  limit,
}: {
  cursor?: string | null;
  limit?: number;
} = {}): Promise<UserPage> {
  const search = new URLSearchParams();
  if (cursor !== null && cursor !== undefined) search.set("cursor", cursor);
  if (limit !== undefined) search.set("limit", String(limit));
  const target = search.size === 0 ? "/internal/users" : "/internal/users?" + search.toString();
  return (await request(target)) as UserPage;
}

export async function mergeClaims(id: string, body: ClaimsMap): Promise<void> {
  const target = `/internal/users/${encodeURIComponent(id)}/claims`;
  const init: RequestInit = {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
  await request(target, init);
}

export async function setClaims(id: string, body: ClaimsMap): Promise<void> {
  const target = `/internal/users/${encodeURIComponent(id)}/claims`;
  const init: RequestInit = {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
  await request(target, init);
}

export async function updateUser(id: string, body: UserPatch): Promise<User> {
  const target = `/internal/users/${encodeURIComponent(id)}`;
  const init: RequestInit = {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  };
  return (await request(target, init)) as User;
}

/** Upper bound on pages one completing call follows. */
const PAGER_MAX_PAGES = 1000;

/**
 * Follow next_cursor until the listing is exhausted. A short page does NOT
 * end the traversal — only a null next_cursor does — because adapters may
 * legitimately return fewer rows than the limit with more pages remaining.
 */
export async function listUsers(options: { limit?: number } = {}): Promise<UserPage> {
  let cursor: string | null = null;
  const rows: Array<User> = [];
  for (let page = 0; page < PAGER_MAX_PAGES; page += 1) {
    const result = await listUsersPage({ cursor, limit: options.limit });
    rows.push(...result.users);
    cursor = result.next_cursor;
    if (cursor === null) {
      return { users: rows, next_cursor: null };
    }
  }
  throw new InternalApiError(
    500,
    "pager_exhausted",
    `listing did not terminate within ${PAGER_MAX_PAGES} pages`,
  );
}
