import { getStats, listUsers } from '$lib/api';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async () => {
    const [stats, recentUsers] = await Promise.all([
        getStats(),
        listUsers(0, 10),
    ]);

    return { stats, recentUsers };
};
