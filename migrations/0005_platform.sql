CREATE TABLE IF NOT EXISTS agent_config_policies (
    scope TEXT NOT NULL,
    node_id TEXT NOT NULL,
    policy_json TEXT NOT NULL,
    version INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (scope, node_id)
);

CREATE TABLE IF NOT EXISTS audit_events (
    id TEXT PRIMARY KEY,
    occurred_at INTEGER NOT NULL,
    actor_type TEXT NOT NULL,
    actor_id TEXT,
    actor_group_id TEXT,
    operation_type TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    node_id TEXT,
    instance_id TEXT,
    result TEXT NOT NULL,
    error_message TEXT,
    source TEXT NOT NULL,
    detail_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_events_filters
ON audit_events(occurred_at, operation_type, target_type, result);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    password_changed_at INTEGER NOT NULL DEFAULT 0,
    must_change_password INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_user_sessions_token ON user_sessions(token_hash);
CREATE INDEX IF NOT EXISTS idx_user_sessions_user_expires ON user_sessions(user_id, expires_at);

CREATE TABLE IF NOT EXISTS user_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_group_members (
    group_id TEXT NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_user_group_members_user ON user_group_members(user_id);

CREATE TABLE IF NOT EXISTS platform_settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS collector_registry (
    id TEXT NOT NULL,
    version TEXT NOT NULL,
    vendor TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    discover_sha256 TEXT NOT NULL,
    metrics_sha256 TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (id, version)
);
