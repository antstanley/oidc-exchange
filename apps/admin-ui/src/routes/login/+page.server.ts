import { fail, redirect } from "@sveltejs/kit";
import { hasAdminClaim, verifyAccessToken } from "$lib/auth";
import type { Actions, PageServerLoad } from "./$types";

const SESSION_COOKIE = "__Host-admin_session";
const COOKIE_OPTIONS = {
  path: "/",
  secure: true,
  httpOnly: true,
  sameSite: "strict" as const,
};

export const load: PageServerLoad = async ({ cookies }) => {
  const token = cookies.get(SESSION_COOKIE);
  if (!token) return {};
  try {
    const claims = await verifyAccessToken(token);
    if (hasAdminClaim(claims)) throw redirect(303, "/");
    throw redirect(303, "/denied");
  } catch (error) {
    if (error && typeof error === "object" && "status" in error) throw error;
    cookies.delete(SESSION_COOKIE, COOKIE_OPTIONS);
    return {};
  }
};

export const actions: Actions = {
  default: async ({ request, cookies }) => {
    const data = await request.formData();
    const tokenValue = data.get("token");
    if (typeof tokenValue !== "string" || tokenValue.length === 0) {
      return fail(400, { error: "Token is required" });
    }

    try {
      const claims = await verifyAccessToken(tokenValue);
      if (!hasAdminClaim(claims)) throw redirect(303, "/denied");
      const maxAge = claims.exp! - Math.floor(Date.now() / 1000);
      if (maxAge <= 0) return fail(401, { error: "Invalid token" });
      cookies.set(SESSION_COOKIE, tokenValue, { ...COOKIE_OPTIONS, maxAge });
      throw redirect(303, "/");
    } catch (error) {
      if (error && typeof error === "object" && "status" in error) throw error;
      return fail(401, { error: "Invalid token" });
    }
  },
};
