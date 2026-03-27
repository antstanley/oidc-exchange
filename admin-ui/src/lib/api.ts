import { env } from '$env/dynamic/private';

const INTERNAL_API_URL = env.INTERNAL_API_URL || 'http://localhost:8081';
const INTERNAL_API_SECRET = env.INTERNAL_API_SECRET || '';

async function api(path: string, options: RequestInit = {}): Promise<Response> {
	const url = `${INTERNAL_API_URL}${path}`;
	const headers = new Headers(options.headers);
	headers.set('Authorization', `Bearer ${INTERNAL_API_SECRET}`);
	if (options.body && !headers.has('Content-Type')) {
		headers.set('Content-Type', 'application/json');
	}

	return fetch(url, { ...options, headers });
}

export async function getStats() {
	const res = await api('/internal/stats');
	if (!res.ok) throw new Error(`Stats failed: ${res.status}`);
	return res.json();
}

export async function listUsers(offset = 0, limit = 50) {
	const res = await api(`/internal/users?offset=${offset}&limit=${limit}`);
	if (!res.ok) throw new Error(`List users failed: ${res.status}`);
	return res.json();
}

export async function getUser(id: string) {
	const res = await api(`/internal/users/${id}`);
	if (!res.ok) {
		if (res.status === 404) return null;
		throw new Error(`Get user failed: ${res.status}`);
	}
	return res.json();
}

export async function updateUser(id: string, patch: Record<string, unknown>) {
	const res = await api(`/internal/users/${id}`, {
		method: 'PATCH',
		body: JSON.stringify(patch),
	});
	if (!res.ok) throw new Error(`Update user failed: ${res.status}`);
	return res.json();
}

export async function deleteUser(id: string) {
	const res = await api(`/internal/users/${id}`, { method: 'DELETE' });
	if (!res.ok) throw new Error(`Delete user failed: ${res.status}`);
}

export async function getUserClaims(id: string) {
	const res = await api(`/internal/users/${id}/claims`);
	if (!res.ok) throw new Error(`Get claims failed: ${res.status}`);
	return res.json();
}

export async function setClaims(id: string, claims: Record<string, unknown>) {
	const res = await api(`/internal/users/${id}/claims`, {
		method: 'PUT',
		body: JSON.stringify(claims),
	});
	if (!res.ok) throw new Error(`Set claims failed: ${res.status}`);
}

export async function mergeClaims(id: string, claims: Record<string, unknown>) {
	const res = await api(`/internal/users/${id}/claims`, {
		method: 'PATCH',
		body: JSON.stringify(claims),
	});
	if (!res.ok) throw new Error(`Merge claims failed: ${res.status}`);
}

export async function clearClaims(id: string) {
	const res = await api(`/internal/users/${id}/claims`, { method: 'DELETE' });
	if (!res.ok) throw new Error(`Clear claims failed: ${res.status}`);
}
