CREATE TABLE IF NOT EXISTS runtime_environments (
    id TEXT PRIMARY KEY,
    node_id TEXT REFERENCES nodes(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    backend TEXT NOT NULL,
    deploy_type TEXT NOT NULL,
    version TEXT,
    base_url TEXT,
    health_url TEXT,
    binary_path TEXT,
    docker_image TEXT,
    working_dir TEXT,
    log_dir TEXT,
    allowed_model_dirs_json TEXT,
    config_json TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_checked_at INTEGER,
    check_status TEXT,
    check_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runtime_environments_node_backend_deploy
ON runtime_environments(node_id, backend, deploy_type);

CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    display_name TEXT,
    model_type TEXT NOT NULL,
    model_path TEXT,
    description TEXT,
    default_backend TEXT,
    config_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_models_deleted_at
ON models(deleted_at);

CREATE TABLE IF NOT EXISTS model_instances (
    id TEXT PRIMARY KEY,
    model_id TEXT NOT NULL REFERENCES models(id),
    node_id TEXT REFERENCES nodes(id),
    runtime_environment_id TEXT NOT NULL REFERENCES runtime_environments(id),
    name TEXT NOT NULL,
    backend TEXT NOT NULL,
    deploy_type TEXT NOT NULL,
    status TEXT NOT NULL,
    endpoint_url TEXT,
    health_url TEXT,
    runtime_version TEXT,
    params_json TEXT,
    last_checked_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_model_instances_model_status
ON model_instances(model_id, status);

CREATE INDEX IF NOT EXISTS idx_model_instances_node_environment
ON model_instances(node_id, runtime_environment_id);

CREATE TABLE IF NOT EXISTS model_file_trash (
    id TEXT PRIMARY KEY,
    model_id TEXT REFERENCES models(id),
    node_id TEXT REFERENCES nodes(id),
    path TEXT NOT NULL,
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
