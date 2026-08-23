import { getStats, listUsersPage } from "$lib/api";
import type { PageServerLoad } from "./$types";

/** How many recently-created users the dashboard preview shows. */
const RECENT_USERS_SHOWN = 10;

export const load: PageServerLoad = async () => {
  const [stats, recentPage] = await Promise.all([
    getStats(),
    listUsersPage({ limit: RECENT_USERS_SHOWN }),
  ]);

  return { stats, recentUsers: recentPage.users };
};
