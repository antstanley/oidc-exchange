# Implementation Plan: Service Roles & Admin Web UI

## Track 1: Rust Server — Service Roles (can be done first, admin UI depends on it)

### Step 1: Add role config
- Add `role: String` to `ServerConfig` with default `"all"`
- Update `config/default.toml`

### Step 2: Add stats endpoint
- Add `count_users_by_status()` to `UserRepository` trait
- Add `count_active_sessions()` to `SessionRepository` trait
- Implement on all adapters (DynamoDB, Postgres, SQLite, MockRepository; Valkey/LMDB for session count)
- Add `admin_stats()` method to `AppService`
- Add `GET /internal/stats` route
- Add to tests

### Step 3: Conditional route mounting
- Refactor main.rs to check `config.server.role`
- Mount routes based on role
- Skip unnecessary adapter construction per role
- Health always mounted

### Step 4: Build and test
- Verify `cargo check --workspace --tests`
- Verify existing tests still pass

## Track 2: SvelteKit Admin UI

### Step 5: Scaffold SvelteKit app
- Create `admin-ui/` with SvelteKit, TailwindCSS 4, Node adapter
- Add LayerChart dependency
- Configure for server-side rendering

### Step 6: Auth system
- Login page with provider redirect
- OAuth callback handler
- JWT verification via JWKS
- Role claim check
- Session cookie management
- Auth guard hook

### Step 7: Dashboard page
- Stats cards (users total, active, suspended; sessions active)
- Registration line chart (LayerChart)
- Session activity chart (LayerChart)

### Step 8: Users list page
- Paginated table
- Search and filter
- Status badges

### Step 9: User detail page
- User info display
- Edit form (email, display_name, status)
- Claims JSON editor
- Session list with revoke
- Danger zone (suspend/delete)

### Step 10: Polish and commit
