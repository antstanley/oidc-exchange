import { afterEach, describe, expect, it, vi } from "vitest";

import {
  clearClaims,
  deleteUser,
  getUser,
  getUserClaims,
  mergeClaims,
  setClaims,
  updateUser,
} from "./api";

/**
 * The exact hostile payloads from correction #41: each carries an encoded
 * separator so that, interpolated raw into the request path, it would
 * re-address credentialed traffic from `/internal/users/<id>` to
 * `/internal/stats` (or another route).
 */
const HOSTILE_IDS = ["x%2f..%2fstats", "%2e%2e%2fstats"] as const;

const DEFAULT_BASE = "http://localhost:8081";

interface RecordedRequest {
  url: string;
  init?: RequestInit;
}

function okResponse(body: unknown = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function recordFetch(): RecordedRequest[] {
  const calls: RecordedRequest[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), init });
      return okResponse();
    }),
  );
  return calls;
}

/** One entry per user-specific helper with the path shape it must produce. */
const USER_HELPERS = [
  {
    name: "getUser",
    expectedPath: (encoded: string) => `/internal/users/${encoded}`,
    invoke: (id: string) => getUser(id),
  },
  {
    name: "updateUser",
    method: "PATCH",
    expectedPath: (encoded: string) => `/internal/users/${encoded}`,
    invoke: (id: string) => updateUser(id, { email: null }),
  },
  {
    name: "deleteUser",
    method: "DELETE",
    expectedPath: (encoded: string) => `/internal/users/${encoded}`,
    invoke: (id: string) => deleteUser(id),
  },
  {
    name: "getUserClaims",
    expectedPath: (encoded: string) => `/internal/users/${encoded}/claims`,
    invoke: (id: string) => getUserClaims(id),
  },
  {
    name: "setClaims",
    method: "PUT",
    expectedPath: (encoded: string) => `/internal/users/${encoded}/claims`,
    invoke: (id: string) => setClaims(id, { tier: "gold" }),
  },
  {
    name: "mergeClaims",
    method: "PATCH",
    expectedPath: (encoded: string) => `/internal/users/${encoded}/claims`,
    invoke: (id: string) => mergeClaims(id, { tier: "gold" }),
  },
  {
    name: "clearClaims",
    method: "DELETE",
    expectedPath: (encoded: string) => `/internal/users/${encoded}/claims`,
    invoke: (id: string) => clearClaims(id),
  },
] as const;

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("user-id path encoding", () => {
  for (const hostileId of HOSTILE_IDS) {
    const encoded = encodeURIComponent(hostileId);

    it(`every user-specific helper addresses only /internal/users/<literal> for ${hostileId}`, async () => {
      for (const helper of USER_HELPERS) {
        const calls = recordFetch();

        await helper.invoke(hostileId);

        expect(calls).toHaveLength(1);
        expect(calls[0].url).toBe(`${DEFAULT_BASE}${helper.expectedPath(encoded)}`);
        expect(calls[0].url).not.toContain("/internal/stats");
      }
    });

    it(`the hostile id ${hostileId} never survives as a path separator`, async () => {
      for (const helper of USER_HELPERS) {
        const calls = recordFetch();

        await helper.invoke(hostileId);

        // The only slashes allowed are the ones the client itself composed:
        // none may come out of the id segment.
        const path = new URL(calls[0].url).pathname;
        expect(path.startsWith("/internal/users/")).toBe(true);
        expect(path.slice("/internal/users/".length).replace(/\/claims$/, "")).toBe(encoded);
      }
    });
  }

  it("a normal id still reaches its intended endpoint", async () => {
    const calls = recordFetch();

    await getUser("usr_123");
    await getUserClaims("usr_123");

    expect(calls.map((call) => call.url)).toEqual([
      `${DEFAULT_BASE}/internal/users/usr_123`,
      `${DEFAULT_BASE}/internal/users/usr_123/claims`,
    ]);
  });
});

describe("request composition is otherwise unchanged by encoding", () => {
  it("helpers keep their HTTP verbs and stay credentialed", async () => {
    for (const helper of USER_HELPERS) {
      const calls = recordFetch();

      await helper.invoke("usr_123");

      if ("method" in helper) {
        expect(calls[0].init?.method, helper.name).toBe(helper.method);
      }
      // The credential must still ride along on every encoded request; with no
      // secret configured the scheme alone is expected (Headers trims it).
      const authorization = new Headers(calls[0].init?.headers).get("Authorization");
      expect(authorization, helper.name).toMatch(/^Bearer/);
    }
  });

  it("mutation bodies still carry their JSON payload", async () => {
    const calls = recordFetch();

    await setClaims("usr_123", { tier: "gold" });

    expect(JSON.parse(String(calls[0].init?.body))).toEqual({ tier: "gold" });
  });

  it("getUser maps a 404 to null without throwing", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ error: "not_found" }), { status: 404 })),
    );

    const result = await getUser("usr_absent");

    expect(result).toBeNull();
  });
});
