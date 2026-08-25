import { beforeEach, describe, expect, it, vi } from "vitest";

const verifyAccessToken = vi.fn();
const hasAdminClaim = vi.fn((claims: Record<string, unknown>) => claims.role === "admin");
vi.mock("$lib/auth", () => ({ verifyAccessToken, hasAdminClaim }));

interface CookieCall {
  name: string;
  value?: string;
  options: Record<string, unknown>;
}

function cookies(initial?: string) {
  const setCalls: CookieCall[] = [];
  const deleteCalls: CookieCall[] = [];
  return {
    get: vi.fn(() => initial),
    set: vi.fn((name: string, value: string, options: Record<string, unknown>) =>
      setCalls.push({ name, value, options }),
    ),
    delete: vi.fn((name: string, options: Record<string, unknown>) =>
      deleteCalls.push({ name, options }),
    ),
    setCalls,
    deleteCalls,
  };
}

async function captureRedirect(
  call: () => PromiseLike<unknown> | unknown,
): Promise<{ status: number; location: string }> {
  try {
    await call();
  } catch (error) {
    return error as { status: number; location: string };
  }
  throw new Error("Expected redirect");
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("session hook", () => {
  it("allows a protected request only with verified admin claims in locals", async () => {
    const claims = { sub: "operator-1", role: "admin", exp: 2_000_000_000 };
    verifyAccessToken.mockResolvedValue(claims);
    const { handle } = await import("./hooks.server");
    const jar = cookies("signed-token");
    const event = { url: new URL("https://console.example/"), cookies: jar, locals: {} } as never;
    const resolve = vi.fn(async () => new Response("ok"));
    const response = await handle({ event, resolve } as never);
    expect(await response.text()).toBe("ok");
    expect(verifyAccessToken).toHaveBeenCalledWith("signed-token");
    expect((event as { locals: Record<string, unknown> }).locals).toEqual({
      userId: "operator-1",
      claims,
    });
    expect(jar.deleteCalls).toHaveLength(0);
  });

  it.each([undefined, "invalid-token"])(
    "redirects invalid session %j and clears a present cookie",
    async (token) => {
      if (token) verifyAccessToken.mockRejectedValue(new Error("invalid"));
      const { handle } = await import("./hooks.server");
      const jar = cookies(token);
      const event = {
        url: new URL("https://console.example/users"),
        cookies: jar,
        locals: {},
      } as never;
      const result = await captureRedirect(() => handle({ event, resolve: vi.fn() } as never));
      expect(result).toMatchObject({ status: 303, location: "/login" });
      expect(jar.deleteCalls).toHaveLength(token ? 1 : 0);
      if (token)
        expect(jar.deleteCalls[0]).toMatchObject({
          name: "__Host-admin_session",
          options: { path: "/", secure: true, httpOnly: true, sameSite: "strict" },
        });
    },
  );

  it("redirects verified non-admin sessions to denied without clearing", async () => {
    verifyAccessToken.mockResolvedValue({ sub: "user-1", role: "user", exp: 2_000_000_000 });
    const { handle } = await import("./hooks.server");
    const jar = cookies("signed-user-token");
    const event = { url: new URL("https://console.example/"), cookies: jar, locals: {} } as never;
    expect(await captureRedirect(() => handle({ event, resolve: vi.fn() } as never))).toMatchObject(
      { location: "/denied" },
    );
    expect(jar.deleteCalls).toHaveLength(0);
  });
});

describe("login and logout", () => {
  it("sets the exact hardened cookie from verified expiry", async () => {
    const now = Math.floor(Date.now() / 1000);
    verifyAccessToken.mockResolvedValue({ sub: "operator-1", role: "admin", exp: now + 300 });
    const { actions } = await import("./routes/login/+page.server");
    const jar = cookies();
    const request = new Request("https://console.example/login", {
      method: "POST",
      body: new URLSearchParams({ token: "signed-token" }),
    });
    const result = await captureRedirect(
      () => actions.default({ request, cookies: jar } as never) as Promise<unknown>,
    );
    expect(result).toMatchObject({ status: 303, location: "/" });
    expect(jar.setCalls).toHaveLength(1);
    expect(jar.setCalls[0]).toMatchObject({
      name: "__Host-admin_session",
      value: "signed-token",
      options: { path: "/", secure: true, httpOnly: true, sameSite: "strict" },
    });
    expect(jar.setCalls[0].options).not.toHaveProperty("domain");
    expect(jar.setCalls[0].options.maxAge).toBeGreaterThanOrEqual(299);
    expect(jar.setCalls[0].options.maxAge).toBeLessThanOrEqual(300);
  });

  it("returns 401 and never persists verification failures", async () => {
    verifyAccessToken.mockRejectedValue(new Error("invalid"));
    const { actions } = await import("./routes/login/+page.server");
    const jar = cookies();
    const request = new Request("https://console.example/login", {
      method: "POST",
      body: new URLSearchParams({ token: "invalid-token" }),
    });
    const result = await actions.default({ request, cookies: jar } as never);
    expect(result).toMatchObject({ status: 401, data: { error: "Invalid token" } });
    expect(jar.setCalls).toHaveLength(0);
  });

  it("login load verifies existing cookies and clears invalid ones", async () => {
    verifyAccessToken.mockRejectedValue(new Error("invalid"));
    const { load } = await import("./routes/login/+page.server");
    const jar = cookies("invalid-token");
    await expect(load({ cookies: jar } as never)).resolves.toEqual({});
    expect(verifyAccessToken).toHaveBeenCalledWith("invalid-token");
    expect(jar.deleteCalls[0]).toMatchObject({
      name: "__Host-admin_session",
      options: { path: "/", secure: true, httpOnly: true, sameSite: "strict" },
    });
  });

  it("verified non-admin login follows denied flow without a cookie", async () => {
    verifyAccessToken.mockResolvedValue({ sub: "user-1", role: "user", exp: 2_000_000_000 });
    const { actions } = await import("./routes/login/+page.server");
    const jar = cookies();
    const request = new Request("https://console.example/login", {
      method: "POST",
      body: new URLSearchParams({ token: "signed-user-token" }),
    });
    expect(
      await captureRedirect(
        () => actions.default({ request, cookies: jar } as never) as Promise<unknown>,
      ),
    ).toMatchObject({ location: "/denied" });
    expect(jar.setCalls).toHaveLength(0);
  });

  it("logout deletes with matching host-cookie scope and attributes", async () => {
    const { load } = await import("./routes/logout/+page.server");
    const jar = cookies("signed-token");
    expect(
      await captureRedirect(() => load({ cookies: jar } as never) as Promise<unknown>),
    ).toMatchObject({ location: "/login" });
    expect(jar.deleteCalls[0]).toMatchObject({
      name: "__Host-admin_session",
      options: { path: "/", secure: true, httpOnly: true, sameSite: "strict" },
    });
    expect(jar.deleteCalls[0].options).not.toHaveProperty("domain");
  });
});
