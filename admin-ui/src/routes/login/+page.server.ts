import { redirect, fail } from '@sveltejs/kit';
import { decodeJwtPayload, hasAdminClaim, isExpired } from '$lib/auth';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ cookies }) => {
	const token = cookies.get('access_token');
	if (token) {
		try {
			const payload = decodeJwtPayload(token);
			if (!isExpired(payload) && hasAdminClaim(payload)) {
				throw redirect(303, '/');
			}
		} catch (e) {
			if (e && typeof e === 'object' && 'status' in e) throw e;
		}
	}
	return {};
};

export const actions: Actions = {
	default: async ({ request, cookies }) => {
		const data = await request.formData();
		const token = data.get('token')?.toString();

		if (!token) {
			return fail(400, { error: 'Token is required' });
		}

		try {
			const payload = decodeJwtPayload(token);

			if (isExpired(payload)) {
				return fail(401, { error: 'Token is expired' });
			}

			if (!hasAdminClaim(payload)) {
				throw redirect(303, '/denied');
			}

			// Set httpOnly cookie
			const exp = payload.exp as number;
			const maxAge = exp - Math.floor(Date.now() / 1000);

			cookies.set('access_token', token, {
				path: '/',
				httpOnly: true,
				secure: false, // set to true in production
				sameSite: 'lax',
				maxAge: maxAge > 0 ? maxAge : 3600
			});

			throw redirect(303, '/');
		} catch (e) {
			if (e && typeof e === 'object' && 'status' in e) throw e;
			return fail(400, { error: 'Invalid token' });
		}
	}
};
