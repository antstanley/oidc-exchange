import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { listUsers, listUsersPage } from "./api";
import { env } from "../../tests/env-stub";

/**
 * The generated client fails closed when no operator credential is
 * configured; the last-preference shared secret keeps these tests on the
 * plain-fetch path (the selection matrix lives in credentials.test.ts).
 */
const TEST_SHARED_SECRET = "vitest-only-shared-secret";

const DEFAULT_BASE = "http://localhost:8081";

interface RecordedRequest {
  url: string;
}

function okResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

/** Stub global fetch to replay `responses` in order and record every URL. */
function recordFetch(responses: Array<Response>): Array<RecordedRequest> {
  const calls: Array<RecordedRequest> = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      calls.push({ url: String(input) });
      return responses.shift() ?? okResponse({ users: [], next_cursor: null });
    }),
  );
  return calls;
}

beforeEach(() => {
  env.INTERNAL_API_SECRET = TEST_SHARED_SECRET;
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete env.INTERNAL_API_SECRET;
});

describe("listUsersPage speaks the published cursor contract", () => {
  it("sends cursor and limit as query parameters and never an offset", async () => {
    const calls = recordFetch([okResponse({ users: [], next_cursor: null })]);

    await listUsersPage({ cursor: "cur-1", limit: 25 });

    expect(calls[0].url.startsWith(`${DEFAULT_BASE}/internal/users?`)).toBe(true);
    const query = new URL(calls[0].url).searchParams;
    expect(query.get("cursor")).toBe("cur-1");
    expect(query.get("limit")).toBe("25");
    expect([...query.keys()]).toEqual(["cursor", "limit"]);
  });

  it("requests the bare path when the first page carries no parameters", async () => {
    const calls = recordFetch([okResponse({ users: [], next_cursor: null })]);

    await listUsersPage();

    expect(calls[0].url).toBe(`${DEFAULT_BASE}/internal/users`);
  });
});

describe("listUsers completes the listing through next_cursor", () => {
  it("follows cursors across pages and aggregates rows until next_cursor is null", async () => {
    const calls = recordFetch([
      okResponse({ users: [{ id: "usr_1" }], next_cursor: "cur-2" }),
      okResponse({ users: [{ id: "usr_2" }, { id: "usr_3" }], next_cursor: null }),
    ]);

    const page = await listUsers({ limit: 25 });

    expect(page.next_cursor).toBeNull();
    expect(page.users.map((user) => user.id)).toEqual(["usr_1", "usr_2", "usr_3"]);
    expect(calls.map((call) => call.url)).toEqual([
      `${DEFAULT_BASE}/internal/users?limit=25`,
      `${DEFAULT_BASE}/internal/users?cursor=cur-2&limit=25`,
    ]);
  });

  it("a short page with a non-null cursor does NOT end the traversal", async () => {
    const calls = recordFetch([
      // One row against a limit of 50 — short, yet more pages remain.
      okResponse({ users: [{ id: "usr_1" }], next_cursor: "more" }),
      okResponse({ users: [], next_cursor: null }),
    ]);

    const page = await listUsers({ limit: 50 });

    expect(page.next_cursor).toBeNull();
    expect(page.users.map((user) => user.id)).toEqual(["usr_1"]);
    expect(calls).toHaveLength(2);
    expect(new URL(calls[1].url).searchParams.get("cursor")).toBe("more");
  });
});
