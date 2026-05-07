#!/bin/sh
# NVIDIA device metrics script.
# Outputs TSV: STATUS line followed by zero or more METRIC lines.
#
# Requires: nvidia-smi

set -e

COLLECTOR="nvidia"
VENDOR="nvidia"

if ! command -v nvidia-smi >/dev/null 2>&1; then
    printf 'STATUS\t1\tnot_available\t%s\t%s\tnvidia-smi not found\n' "$VENDOR" "$COLLECTOR"
    exit 0
fi

printf 'STATUS\t1\tok\t%s\t%s\t\n' "$VENDOR" "$COLLECTOR"

# Query metrics fields.
# Fields: uuid, memory.total, memory.used, memory.free, utilization.gpu,
#         utilization.memory, temperature.gpu, power.draw
nvidia-smi \
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

    # Build stable device_key: prefer nvidia:<uuid>, fall back to nvidia:<uuid>
    if [ -n "$uuid" ] && [ "$uuid" != "[N/A]" ]; then
        device_key="nvidia:$uuid"
    else
        device_key="nvidia:unknown"
    fi

    # Health: ok unless GPU has fallen off the bus or has errors.
    health="ok"
    if [ "$gpu_util" = "[N/A]" ] && [ "$mem_util" = "[N/A]" ]; then
        health="unknown"
    fi

    printf 'METRIC\t1\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t\n' \
        "$device_key" "$mem_total" "$mem_used" "$mem_free" \
        "$gpu_util" "$mem_util" "$temp" "$power" "$health"
done
