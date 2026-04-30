ALTER TABLE node_status ADD COLUMN agent_config_version INTEGER;
ALTER TABLE node_status ADD COLUMN heartbeat_interval_secs INTEGER;
ALTER TABLE node_status ADD COLUMN metrics_sample_interval_secs INTEGER;
ALTER TABLE node_status ADD COLUMN task_poll_interval_secs INTEGER;
ALTER TABLE node_status ADD COLUMN config_refresh_interval_secs INTEGER;
ALTER TABLE node_status ADD COLUMN command_timeout_secs INTEGER;
ALTER TABLE node_status ADD COLUMN environment_check_timeout_secs INTEGER;
ALTER TABLE node_status ADD COLUMN last_config_updated_at INTEGER;

ALTER TABLE model_instances RENAME TO model_instances_old;

CREATE TABLE IF NOT EXISTS model_instances (
    id TEXT PRIMARY KEY,
    model_id TEXT NOT NULL REFERENCES models(id),
    node_id TEXT REFERENCES nodes(id),
    runtime_environment_id TEXT REFERENCES runtime_environments(id),
    name TEXT NOT NULL,
    backend TEXT NOT NULL,
    deploy_type TEXT NOT NULL,
    status TEXT NOT NULL,
    base_url TEXT,
    endpoint_url TEXT,
    health_url TEXT,
    runtime_version TEXT,
    model_name TEXT,
    description TEXT,
    params_json TEXT,
    last_checked_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO model_instances (
    id, model_id, node_id, runtime_environment_id, name, backend,
    deploy_type, status, endpoint_url, health_url, runtime_version,
    params_json, last_checked_at, last_error, created_at, updated_at
)
SELECT
    id, model_id, node_id, runtime_environment_id, name, backend,
    deploy_type, status, endpoint_url, health_url, runtime_version,
    params_json, last_checked_at, last_error, created_at, updated_at
FROM model_instances_old;

DROP TABLE model_instances_old;

CREATE INDEX IF NOT EXISTS idx_model_instances_model_status
ON model_instances(model_id, status);

CREATE INDEX IF NOT EXISTS idx_model_instances_node_environment
ON model_instances(node_id, runtime_environment_id);
