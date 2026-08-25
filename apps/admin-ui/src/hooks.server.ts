import { redirect, type Handle } from "@sveltejs/kit";
import { hasAdminClaim, verifyAccessToken } from "$lib/auth";

const SESSION_COOKIE = "__Host-admin_session";
const COOKIE_OPTIONS = {
  path: "/",
  secure: true,
  httpOnly: true,
  sameSite: "strict" as const,
};

export const handle: Handle = async ({ event, resolve }) => {
  const path = event.url.pathname;
  if (path === "/login" || path === "/denied") return resolve(event);

  const token = event.cookies.get(SESSION_COOKIE);
  if (!token) throw redirect(303, "/login");

  try {
    const claims = await verifyAccessToken(token);
    if (!hasAdminClaim(claims)) throw redirect(303, "/denied");
    event.locals.userId = claims.sub!;
    event.locals.claims = claims;
  } catch (error) {
    if (error && typeof error === "object" && "status" in error) throw error;
    event.cookies.delete(SESSION_COOKIE, COOKIE_OPTIONS);
    throw redirect(303, "/login");
  }

  return resolve(event);
};
