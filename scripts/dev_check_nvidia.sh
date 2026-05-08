#!/usr/bin/env bash
set -euo pipefail

SERVER_URL="${LIGHTAI_SERVER_URL:-http://127.0.0.1:8080}"
AGENT_URL="${LIGHTAI_AGENT_URL:-http://127.0.0.1:8081}"
CONTROL_TOKEN="${LIGHTAI_EMERGENCY_CONTROL_TOKEN:-}"

echo "Checking nvidia-smi..."
if ! command -v nvidia-smi >/dev/null 2>&1; then
  echo "nvidia-smi not found in PATH" >&2
  exit 1
fi

echo "Checking nvidia-smi query fields..."
nvidia-smi \
  --query-gpu=index,name,uuid,driver_version,memory.total,memory.used,utilization.gpu,temperature.gpu,power.draw \
  --format=csv,noheader,nounits >/tmp/lightai-nvidia-smi-check.txt
head -n 5 /tmp/lightai-nvidia-smi-check.txt

echo "Checking Server /health..."
curl -fsS "$SERVER_URL/health"
echo

echo "Checking Agent /health..."
curl -fsS "$AGENT_URL/health"
echo

echo "Checking Server /api/nodes..."
if [[ -z "$CONTROL_TOKEN" ]]; then
  echo "LIGHTAI_EMERGENCY_CONTROL_TOKEN is required for this API check, or inspect nodes through the logged-in Web UI" >&2
  exit 1
fi
curl -fsS -H "X-LightAI-Control-Token: $CONTROL_TOKEN" "$SERVER_URL/api/nodes"
echo

echo "NVIDIA local development check completed."
