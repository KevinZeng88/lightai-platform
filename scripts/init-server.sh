#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Use local binary if available (for deployment verification), else cargo run.
if [ -x bin/lightai-server ]; then
    LIGHTAI_SERVER="bin/lightai-server"
else
    LIGHTAI_SERVER="cargo run -p lightai-server --"
fi

# ── Parse flags ──
SERVER_HOST=""
YES=false
while [ $# -gt 0 ]; do
    case "$1" in
        --host) SERVER_HOST="$2"; shift 2 ;;
        --yes) YES=true; shift ;;
        --help|-h) echo "Usage: $0 [--host <IP_OR_DNS>] [--yes]"; exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

echo "=== LightAI Server deployment init ==="

mkdir -p certs data logs run

# Prompt for server host if not provided.
if [ -z "${SERVER_HOST}" ]; then
    read -r -p "Server IP or domain name: " SERVER_HOST
    if [ -z "${SERVER_HOST}" ]; then
        echo "ERROR: Server host is required."
        exit 1
    fi
fi

echo "[1/5] Generating certificates for ${SERVER_HOST}..."
${LIGHTAI_SERVER} cert init --host "${SERVER_HOST}" --out ./certs

echo "[2/5] Generating setup token..."
SETUP_TOKEN=$(${LIGHTAI_SERVER} cert setup-token)

echo "[3/5] Generating lightai-server.toml..."
cat > lightai-server.toml << TOML
setup_token = "${SETUP_TOKEN}"

[https]
listen_addr = "0.0.0.0:18443"
cert_path = "./certs/server.crt"
key_path = "./certs/server.key"

[http]
enabled = false
listen_addr = "127.0.0.1:18080"

[web]
dist_dir = "web/dist"

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

echo "[4/5] Registering GPU collectors..."
${LIGHTAI_SERVER} --config ./lightai-server.toml collector sync --root ./collectors/gpu

CA_FP=$(${LIGHTAI_SERVER} cert fingerprint --file ./certs/ca.crt)

echo "[5/5] Generating deployment-info.txt..."
cat > deployment-info.txt << EOF
LightAI Server Deployment Info
================================
Server host:      ${SERVER_HOST}
Web/API address:  https://${SERVER_HOST}:18443/
CA fingerprint:   SHA256:${CA_FP}
Setup token:      ${SETUP_TOKEN}

IMPORTANT:
- ca.crt can be distributed to Agent machines.
- ca.key and server.key must NOT be distributed.
- deployment-info.txt is sensitive — keep secure.
EOF
chmod 600 deployment-info.txt

echo ""
echo "=== Server deployment ready ==="
echo "  Web/API:        https://${SERVER_HOST}:18443/"
echo "  CA fingerprint:  SHA256:${CA_FP}"
echo "  Setup token:     ${SETUP_TOKEN}"
echo "  Sensitive info:  deployment-info.txt (chmod 600)"
