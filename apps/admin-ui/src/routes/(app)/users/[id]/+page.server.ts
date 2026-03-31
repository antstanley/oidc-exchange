import { getUser, getUserClaims, updateUser, setClaims, deleteUser } from '$lib/api';
import { error, redirect, fail } from '@sveltejs/kit';
import type { PageServerLoad, Actions } from './$types';

export const load: PageServerLoad = async ({ params }) => {
	const user = await getUser(params.id);
	if (!user) throw error(404, 'User not found');
	const claims = await getUserClaims(params.id);
	return { user, claims };
};

export const actions: Actions = {
	update: async ({ params, request }) => {
		const data = await request.formData();
		const patch: Record<string, unknown> = {};

		const email = data.get('email')?.toString();
		if (email !== undefined) patch.email = email || null;

		const displayName = data.get('display_name')?.toString();
		if (displayName !== undefined) patch.display_name = displayName || null;

		const status = data.get('status')?.toString();
		if (status) patch.status = status;

		await updateUser(params.id, patch);
		return { success: true };
	},

	updateClaims: async ({ params, request }) => {
		const data = await request.formData();
		const claimsJson = data.get('claims')?.toString();
		if (!claimsJson) return fail(400, { claimsError: 'Claims JSON is required' });

		try {
			const claims = JSON.parse(claimsJson);
			await setClaims(params.id, claims);
			return { claimsSuccess: true };
		} catch {
			return fail(400, { claimsError: 'Invalid JSON' });
		}
	},

	delete: async ({ params }) => {
		await deleteUser(params.id);
		throw redirect(303, '/users');
	},
};
