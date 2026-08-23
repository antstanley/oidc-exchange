// GENERATED FILE — do not edit.
// Source: schemas/internal-api.schema.json (run `pnpm generate`).

export type UserStatus = "active" | "suspended" | "deleted";
export interface User {
  /** Internal user ID (e.g., usr_01ARZ3NDEK...) */
  id: string;
  /** Provider subject claim or DID */
  external_id: string;
  /** Provider identifier (google, apple, atproto) */
  provider: string;
  email?: string | null;
  display_name?: string | null;
  /** Extensible key-value pairs for sync data */
  metadata?: Record<string, unknown>;
  /** Per-user private claims added to access token JWT, managed via the internal API */
  claims?: Record<string, unknown>;
  status: UserStatus;
  /** Optimistic-concurrency counter */
  version: number;
  created_at: string;
  updated_at: string;
}
export interface NewUser {
  external_id: string;
  provider: string;
  email?: string;
  display_name?: string;
}
export interface UserPatch {
  email?: string | null;
  display_name?: string | null;
  metadata?: Record<string, unknown>;
  status?: UserStatus;
}
export type ClaimsMap = Record<string, unknown>;
export interface UserPage {
  users: Array<User>;
  next_cursor: string | null;
}
export interface Stats {
  users: { total: number; active: number; suspended: number; deleted: number };
  sessions: { active: number };
}
export type OperatorAuthMechanism = "mtls" | "operator_token" | "shared_secret";
export interface OperatorPrincipal {
  /** Certificate subject, operator-token sub, or the reserved literal 'unattributed' */
  id: string;
  mechanism: OperatorAuthMechanism;
}
export interface ErrorResponse {
  error: string;
  error_description?: string;
}
