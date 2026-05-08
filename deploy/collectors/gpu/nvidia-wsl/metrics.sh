#!/bin/sh
# NVIDIA device metrics script.
# Outputs TSV: STATUS line followed by zero or more METRIC lines.
#
# Requires: nvidia-smi at the hardcoded path below.
#
# This script is executed by the Agent inside an env_clear() sandbox.
# It does NOT depend on PATH or parent environment.
#
# To customise the nvidia-smi path for your environment:
#   1. Find the real path:  readlink -f "$(which nvidia-smi)"
#   2. Edit NVIDIA_SMI below.
#   3. Re-inspect and re-register:
#        lightai-agent collector inspect <dir>
#        lightai-server collector register --dir <dir>
#   4. Or update via Web → collector registry → edit/update.
#
# Testing in a clean environment:
#   env -i /bin/sh metrics.sh

set -eu

COLLECTOR="nvidia"
VENDOR="nvidia"

# ── Hardcoded absolute path ──
# Change this to match your environment (e.g. /usr/bin/nvidia-smi).
NVIDIA_SMI="/usr/lib/wsl/lib/nvidia-smi"

if [ ! -x "$NVIDIA_SMI" ]; then
    printf 'STATUS\t1\tnot_available\t%s\t%s\tnvidia-smi not executable: %s\n' \
        "$VENDOR" "$COLLECTOR" "$NVIDIA_SMI"
    exit 0
fi

printf 'STATUS\t1\tok\t%s\t%s\t\n' "$VENDOR" "$COLLECTOR"

# Query metrics fields.
# Fields: uuid, memory.total, memory.used, memory.free, utilization.gpu,
#         utilization.memory, temperature.gpu, power.draw
"$NVIDIA_SMI" \
    --query-gpu=uuid,memory.total,memory.used,memory.free,utilization.gpu,utilization.memory,temperature.gpu,power.draw \
    --format=csv,noheader,nounits 2>/dev/null \
    | while IFS=',' read -r uuid mem_total mem_used mem_free gpu_util mem_util temp power; do
    uuid=$(echo "$uuid" | tr -d ' ')
    mem_total=$(echo "$mem_total" | tr -d ' ')
    mem_used=$(echo "$mem_used" | tr -d ' ')
    mem_free=$(echo "$mem_free" | tr -d ' ')
    gpu_util=$(echo "$gpu_util" | tr -d ' ')
    mem_util=$(echo "$mem_util" | tr -d ' ')
    temp=$(echo "$temp" | tr -d ' ')
    power=$(echo "$power" | tr -d ' ')

    if [ -n "$uuid" ] && [ "$uuid" != "[N/A]" ]; then
        device_key="nvidia:$uuid"
    else
        device_key="nvidia:unknown"
    fi

    health="ok"
    if [ "$gpu_util" = "[N/A]" ] && [ "$mem_util" = "[N/A]" ]; then
        health="unknown"
    fi

    printf 'METRIC\t1\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t\n' \
        "$device_key" "$mem_total" "$mem_used" "$mem_free" \
        "$gpu_util" "$mem_util" "$temp" "$power" "$health"
done
