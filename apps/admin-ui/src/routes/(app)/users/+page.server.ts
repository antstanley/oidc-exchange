import { listUsers } from '$lib/api';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ url }) => {
	const page = parseInt(url.searchParams.get('page') || '1');
	const limit = 25;
	const offset = (page - 1) * limit;
	const users = await listUsers(offset, limit);
	return { users, page, limit };
};
