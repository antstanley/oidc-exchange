export interface User {
  id: string;
  external_id: string;
  provider: string;
  email: string | null;
  display_name: string | null;
  metadata: Record<string, unknown>;
  claims: Record<string, unknown>;
  /**
   * The service's wire spelling of `UserStatus` (serde `snake_case`), not a
   * display label — the canonical schema and every status comparison in the
   * UI must use these exact values.
   */
  status: "active" | "suspended" | "deleted";
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
