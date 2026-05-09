#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Use local binary if available (for deployment verification), else cargo run.
if [ -x bin/lightai-agent ]; then
    LIGHTAI_AGENT="bin/lightai-agent"
else
    LIGHTAI_AGENT="cargo run -p lightai-agent --"
fi

# ── Parse flags ──
SERVER_URL=""
AGENT_NAME=""
YES=false
while [ $# -gt 0 ]; do
    case "$1" in
        --server) SERVER_URL="$2"; shift 2 ;;
        --name) AGENT_NAME="$2"; shift 2 ;;
        --yes) YES=true; shift ;;
        --help|-h) echo "Usage: $0 [--server <URL>] [--name <NAME>] [--yes]"; exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

echo "=== LightAI Agent deployment init ==="

mkdir -p certs data logs run

HOSTNAME=$(hostname)
AGENT_NAME="${AGENT_NAME:-${HOSTNAME}}"

# Prompt for server URL if not provided.
if [ -z "${SERVER_URL}" ]; then
    read -r -p "Server URL (e.g. https://172.19.168.153:18443): " SERVER_URL
    if [ -z "${SERVER_URL}" ]; then
        echo "ERROR: Server URL is required."
        exit 1
    fi
fi

echo "[1/3] Downloading CA certificate from ${SERVER_URL}..."

if [ "$YES" = true ]; then
    # Non-interactive: auto-confirm CA fingerprint.
    ${LIGHTAI_AGENT} ca fetch --server "${SERVER_URL}" --out ./certs/ca.crt --yes
else
    ${LIGHTAI_AGENT} ca fetch --server "${SERVER_URL}" --out ./certs/ca.crt
fi

# If name not set, prompt.
if [ -z "${AGENT_NAME}" ] || [ "$AGENT_NAME" = "$HOSTNAME" ] && [ "$YES" != true ]; then
    read -r -p "Agent name [${HOSTNAME}]: " input
    AGENT_NAME="${input:-${HOSTNAME}}"
fi

echo "[2/3] Generating lightai-agent.toml..."
cat > lightai-agent.toml << TOML
[agent]
listen_addr = "127.0.0.1:18081"
node_name = "${AGENT_NAME}"
state_path = "data/agent-state.toml"

[server]
url = "${SERVER_URL}"
ca_cert_path = "./certs/ca.crt"
insecure_skip_tls_verify = false

[gpu_collectors]
root = "./collectors/gpu"
mode = "explicit"
enabled = ["nvidia-wsl"]
TOML

echo "[3/3] Initialization complete."
echo "  Start Agent:  ${LIGHTAI_AGENT} --config ./lightai-agent.toml"
