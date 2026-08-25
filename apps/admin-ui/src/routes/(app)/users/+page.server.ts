import { listUsersPage } from "$lib/api";
import type { PageServerLoad } from "./$types";

/**
 * The per-page bound the console requests; the service clamps whatever it
 * receives to its own MAX_ADMIN_PAGE_SIZE regardless.
 */
const USERS_PAGE_LIMIT = 25;

/**
 * Cursor-paginated user listing.
 *
 * Why a cursor stack in the URL: the internal API's pages are cursor-keyed
 * with no `previous` token, so backward navigation is expressed by replaying
 * the cursors visited so far — each "next" pushes the page's next_cursor,
 * each "back" pops one. The offset parameter is gone for good: an offset
 * this service cannot honour without a full scan is the defect, not a
 * compatibility surface.
 *
 * Full end-to-end pagination behaviour (including bounded reads server-side)
 * is task 08's slice; this loader already speaks only the published contract.
 */
export const load: PageServerLoad = async ({ url }) => {
  const limit = USERS_PAGE_LIMIT;
  const stack = decodeCursorStack(url.searchParams.get("stack"));
  const cursor = stack.at(-1) ?? null;

  const page = await listUsersPage({ cursor, limit });

  return {
    users: page.users,
    next_cursor: page.next_cursor,
    stack,
    limit,
  };
};

/** Read and validate the visited-cursor chain from the URL. */
function decodeCursorStack(raw: string | null): string[] {
  if (raw === null || raw === "") {
    return [];
  }
  // Bounded: a hand-edited URL can carry at most this many hops.
  const MAX_STACK_DEPTH = 1000;
  return raw
    .split(",")
    .slice(0, MAX_STACK_DEPTH)
    .map((part) => part.trim())
    .filter((part) => part !== "")
    .map((part) => safeDecode(part));
}

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    // A malformed escape is just a bad cursor; the service will reject it.
    return value;
  }
}
