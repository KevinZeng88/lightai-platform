#!/bin/sh
# NVIDIA device discovery script.
# Outputs TSV: STATUS line followed by zero or more DEVICE lines.
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

# Query device identity fields.
# Fields: index, name, uuid, pci.bus_id, driver_version
nvidia-smi \
    --query-gpu=index,name,uuid,pci.bus_id,driver_version \
    --format=csv,noheader,nounits 2>/dev/null \
    | while IFS=',' read -r idx name uuid pci driver; do
    idx=$(echo "$idx" | tr -d ' ')
    name=$(echo "$name" | tr -d ' ')
    uuid=$(echo "$uuid" | tr -d ' ')
    pci=$(echo "$pci" | tr -d ' ')
    driver=$(echo "$driver" | tr -d ' ')

    # Build stable device_key: prefer nvidia:<uuid>, fall back to nvidia:index-<idx>
    if [ -n "$uuid" ] && [ "$uuid" != "[N/A]" ]; then
        device_key="nvidia:$uuid"
    else
        device_key="nvidia:index-$idx"
    fi

    printf 'DEVICE\t1\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t\n' \
        "$device_key" "$VENDOR" "$idx" "$name" "$uuid" "$pci" "$driver"
done
