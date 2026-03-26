CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,
    external_id     TEXT NOT NULL,
    provider        TEXT NOT NULL,
    email           TEXT,
    display_name    TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    claims          JSONB NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_external_id ON users (external_id);

CREATE TABLE IF NOT EXISTS sessions (
    refresh_token_hash TEXT PRIMARY KEY,
    user_id            TEXT NOT NULL REFERENCES users(id),
    provider           TEXT NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    device_id          TEXT,
    user_agent         TEXT,
    ip_address         TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
