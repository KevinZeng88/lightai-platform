#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Use cargo run from project root.
LIGHTAI_SERVER="cargo run -p lightai-server --"

echo "=== LightAI local development init ==="

mkdir -p certs data logs run

HOSTNAME=$(hostname)

# Generate certificates
echo "[1/4] Generating certificates..."
$LIGHTAI_SERVERcert init --host 127.0.0.1 --out ./certs

# Generate setup token
echo "[2/4] Generating setup token..."
SETUP_TOKEN=$($LIGHTAI_SERVERcert setup-token)

# Generate server config
echo "[3/4] Generating lightai-server.toml..."
cat > lightai-server.toml << TOML
setup_token = "${SETUP_TOKEN}"

[https]
listen_addr = "127.0.0.1:18443"
cert_path = "./certs/server.crt"
key_path = "./certs/server.key"

[http]
enabled = false
listen_addr = "127.0.0.1:18080"

[web]
dist_dir = "./web/dist"

[database]
url = "sqlite://./data/lightai.db"

[metrics]
retention_days = 7
cleanup_interval_hours = 6

[auth.password]
min_length = 12
complexity_required = false
expires_days = 0
force_change_after_reset = true

[auth.session]
ttl_secs = 43200
idle_timeout_secs = 7200
secure_cookie = true

[logs]
dir = "logs"
level = "info"
max_file_bytes = 10485760
retention_files = 5
retention_days = 30
TOML

# Register collectors
echo "[4/4] Registering GPU collectors..."
$LIGHTAI_SERVER--config ./lightai-server.toml collector sync --root ./deploy/collectors/gpu

# Generate agent config
cat > lightai-agent.toml << TOML
[agent]
listen_addr = "127.0.0.1:18081"
state_path = "data/agent-state.toml"

[server]
url = "https://127.0.0.1:18443"
ca_cert_path = "./certs/ca.crt"
insecure_skip_tls_verify = false

[gpu_collectors]
root = "./deploy/collectors/gpu"
mode = "explicit"
enabled = ["nvidia-wsl"]
TOML

echo ""
echo "=== Local dev environment ready ==="
echo ""
echo "  Start Server:  $LIGHTAI_SERVER--config ./lightai-server.toml"
echo "  Start Agent:   cargo run -p lightai-agent -- --config ./lightai-agent.toml"
echo "  Web:           cd web && npm run dev"
echo ""
echo "  CA fingerprint: see certs/ca.crt (display with: $LIGHTAI_SERVERcert fingerprint --file ./certs/ca.crt)"
echo "  Setup token:    ${SETUP_TOKEN}"
echo ""
echo "  certs/ca.crt        — can be distributed to Agents"
echo "  certs/ca.key        — do NOT distribute"
echo "  certs/server.key    — do NOT distribute"
echo "  lightai-server.toml — contains setup_token, treat as sensitive"
