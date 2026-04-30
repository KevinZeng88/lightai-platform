CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    hostname TEXT NOT NULL,
    agent_version TEXT,
    os TEXT,
    arch TEXT,
    token_hash TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    registered_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_heartbeat_at INTEGER
);

CREATE TABLE IF NOT EXISTS node_status (
    node_id TEXT PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
    cpu_usage_percent REAL,
    memory_total_bytes INTEGER,
    memory_used_bytes INTEGER,
    disk_total_bytes INTEGER,
    disk_used_bytes INTEGER,
    collector_errors_json TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS gpu_status (
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    gpu_key TEXT NOT NULL,
    gpu_index INTEGER,
    vendor TEXT NOT NULL,
    name TEXT NOT NULL,
    uuid TEXT,
    driver_version TEXT,
    memory_total_bytes INTEGER,
    memory_used_bytes INTEGER,
    utilization_percent REAL,
    temperature_celsius REAL,
    power_watts REAL,
    collector TEXT NOT NULL,
    raw_json TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (node_id, gpu_key)
);

CREATE TABLE IF NOT EXISTS node_metric_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    sampled_at INTEGER NOT NULL,
    cpu_usage_percent REAL,
    memory_total_bytes INTEGER,
    memory_used_bytes INTEGER,
    disk_total_bytes INTEGER,
    disk_used_bytes INTEGER
);

CREATE INDEX IF NOT EXISTS idx_node_metric_samples_node_time
ON node_metric_samples(node_id, sampled_at);

CREATE TABLE IF NOT EXISTS gpu_metric_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    gpu_key TEXT NOT NULL,
    sampled_at INTEGER NOT NULL,
    vendor TEXT NOT NULL,
    memory_total_bytes INTEGER,
    memory_used_bytes INTEGER,
    utilization_percent REAL,
    temperature_celsius REAL,
    power_watts REAL
);

CREATE INDEX IF NOT EXISTS idx_gpu_metric_samples_node_gpu_time
ON gpu_metric_samples(node_id, gpu_key, sampled_at);
