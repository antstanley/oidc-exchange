import { redirect, type Handle } from '@sveltejs/kit';
import { decodeJwtPayload, hasAdminClaim, isExpired } from '$lib/auth';

export const handle: Handle = async ({ event, resolve }) => {
	const path = event.url.pathname;

	// Public paths that don't require auth
	if (path === '/login' || path === '/login/callback' || path === '/denied') {
		return resolve(event);
	}

	// Check for access token in cookie
	const token = event.cookies.get('access_token');
	if (!token) {
		throw redirect(303, '/login');
	}

	try {
		const payload = decodeJwtPayload(token);

		if (isExpired(payload)) {
			event.cookies.delete('access_token', { path: '/' });
			throw redirect(303, '/login');
		}

		if (!hasAdminClaim(payload)) {
			throw redirect(303, '/denied');
		}

		// Attach user info to locals
		event.locals.userId = payload.sub as string;
		event.locals.token = token;
	} catch (err) {
		if (err && typeof err === 'object' && 'status' in err) throw err; // re-throw redirects
		event.cookies.delete('access_token', { path: '/' });
		throw redirect(303, '/login');
	}

	return resolve(event);
};
