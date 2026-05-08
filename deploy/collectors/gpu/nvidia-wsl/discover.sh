#!/bin/sh
# NVIDIA device discovery script.
# Outputs TSV: STATUS line followed by zero or more DEVICE lines.
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
#   env -i /bin/sh discover.sh

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

# Query device identity fields.
# Fields: index, name, uuid, pci.bus_id, driver_version
"$NVIDIA_SMI" \
    --query-gpu=index,name,uuid,pci.bus_id,driver_version \
    --format=csv,noheader,nounits 2>/dev/null \
    | while IFS=',' read -r idx name uuid pci driver; do
    idx=$(echo "$idx" | tr -d ' ')
    name=$(echo "$name" | tr -d ' ')
    uuid=$(echo "$uuid" | tr -d ' ')
    pci=$(echo "$pci" | tr -d ' ')
    driver=$(echo "$driver" | tr -d ' ')

    if [ -n "$uuid" ] && [ "$uuid" != "[N/A]" ]; then
        device_key="nvidia:$uuid"
    else
        device_key="nvidia:index-$idx"
    fi

    printf 'DEVICE\t1\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t\n' \
        "$device_key" "$VENDOR" "$idx" "$name" "$uuid" "$pci" "$driver"
done
