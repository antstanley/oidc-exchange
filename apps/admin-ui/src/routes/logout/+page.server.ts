import { redirect } from "@sveltejs/kit";
import type { PageServerLoad } from "./$types";

const SESSION_COOKIE = "__Host-admin_session";
const COOKIE_OPTIONS = {
  path: "/",
  secure: true,
  httpOnly: true,
  sameSite: "strict" as const,
};

export const load: PageServerLoad = async ({ cookies }) => {
  cookies.delete(SESSION_COOKIE, COOKIE_OPTIONS);
  throw redirect(303, "/login");
};
