export interface User {
	id: string;
	external_id: string;
	provider: string;
	email: string | null;
	display_name: string | null;
	metadata: Record<string, unknown>;
	claims: Record<string, unknown>;
	status: 'Active' | 'Suspended' | 'Deleted';
	created_at: string;
	updated_at: string;
}

export interface Stats {
	users: {
		total: number;
		active: number;
		suspended: number;
		deleted: number;
	};
	sessions: {
		active: number;
	};
}
