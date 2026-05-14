#!/usr/bin/env bash
set -euo pipefail

# ── Usage ──
#   bash scripts/package-release.sh [VERSION] [SUFFIX]
#
# SUFFIX defaults to "native".  Use "glibc2.28" for the RHEL 8 compatible build.
#
# Examples:
#   bash scripts/package-release.sh v0.1.0
#   bash scripts/package-release.sh v0.1.0 native
#   bash scripts/package-release.sh v0.1.0 glibc2.28

VERSION="${1:-v0.1.0}"
SUFFIX="${2:-native}"
RELEASE_NAME="lightai-platform-${VERSION}-linux-x86_64-${SUFFIX}"
RELEASE_DIR="release/${RELEASE_NAME}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${PROJECT_ROOT}"

echo "=== Building release ${VERSION} (${SUFFIX}) ==="

# ── 1. Build Rust binaries ──
echo "[1/7] Building Rust release binaries..."
cargo build --workspace --release

# ── 2. Build Web frontend ──
echo "[2/7] Building Web frontend..."
( cd "${PROJECT_ROOT}/web" && npm run build )

# ── 3. Assemble release directory ──
echo "[3/7] Assembling release directory..."
rm -rf "${RELEASE_DIR}"
mkdir -p "${RELEASE_DIR}"/{bin,web/dist,config,scripts,systemd,logs,run,data}

cp "${PROJECT_ROOT}/target/release/lightai-server" "${RELEASE_DIR}/bin/"
cp "${PROJECT_ROOT}/target/release/lightai-agent" "${RELEASE_DIR}/bin/"
cp -r "${PROJECT_ROOT}/web/dist/"* "${RELEASE_DIR}/web/dist/"
cp -r "${PROJECT_ROOT}/deploy/collectors" "${RELEASE_DIR}/"
cp "${PROJECT_ROOT}/deploy/server.example.toml" "${RELEASE_DIR}/config/"
cp "${PROJECT_ROOT}/deploy/agent.example.toml" "${RELEASE_DIR}/config/"
cp "${PROJECT_ROOT}/deploy/lightai-server.service" "${RELEASE_DIR}/systemd/"
cp "${PROJECT_ROOT}/deploy/lightai-agent.service" "${RELEASE_DIR}/systemd/"
cp "${PROJECT_ROOT}/INSTALL.md" "${RELEASE_DIR}/"
cp "${PROJECT_ROOT}/scripts/init-server.sh" "${RELEASE_DIR}/scripts/"
cp "${PROJECT_ROOT}/scripts/init-agent.sh" "${RELEASE_DIR}/scripts/"

# Ensure executables are marked +x (source perms may vary).
chmod +x "${RELEASE_DIR}/bin/lightai-server"
chmod +x "${RELEASE_DIR}/bin/lightai-agent"
chmod +x "${RELEASE_DIR}/scripts/init-server.sh"
chmod +x "${RELEASE_DIR}/scripts/init-agent.sh"
chmod +x "${RELEASE_DIR}/collectors/gpu/nvidia-wsl/discover.sh"
chmod +x "${RELEASE_DIR}/collectors/gpu/nvidia-wsl/metrics.sh"

# ── 4. Generate start/stop scripts ──
echo "[4/7] Generating start/stop scripts..."

cat > "${RELEASE_DIR}/scripts/start-server.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p run logs data

if [ -f run/lightai-server.pid ]; then
    old_pid=$(cat run/lightai-server.pid)
    if kill -0 "$old_pid" 2>/dev/null; then
        echo "Server is already running (pid $old_pid)"
        exit 0
    fi
    rm -f run/lightai-server.pid
fi

CONFIG_FILE="${LIGHTAI_SERVER_CONFIG:-lightai-server.toml}"
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Config file '$CONFIG_FILE' not found."
    echo "Copy config/server.example.toml to lightai-server.toml and edit it first."
    exit 1
fi

nohup bin/lightai-server --config "$CONFIG_FILE" > logs/server-stdout.log 2> logs/server-stderr.log &
echo $! > run/lightai-server.pid
echo "Server started (pid $!)"
SCRIPT

cat > "${RELEASE_DIR}/scripts/start-agent.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p run logs data

if [ -f run/lightai-agent.pid ]; then
    old_pid=$(cat run/lightai-agent.pid)
    if kill -0 "$old_pid" 2>/dev/null; then
        echo "Agent is already running (pid $old_pid)"
        exit 0
    fi
    rm -f run/lightai-agent.pid
fi

CONFIG_FILE="${LIGHTAI_AGENT_CONFIG:-lightai-agent.toml}"
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Config file '$CONFIG_FILE' not found."
    echo "Copy config/agent.example.toml to lightai-agent.toml and edit it first."
    exit 1
fi

nohup bin/lightai-agent --config "$CONFIG_FILE" > logs/lightai-agent.log 2>&1 &
echo $! > run/lightai-agent.pid
echo "Agent started (pid $!)"
SCRIPT

cat > "${RELEASE_DIR}/scripts/stop.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

stopped=0

for service in lightai-agent lightai-server; do
    pidfile="run/${service}.pid"
    if [ -f "$pidfile" ]; then
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then
            echo "Stopping $service (pid $pid)..."
            kill "$pid"
            for i in $(seq 1 20); do
                if ! kill -0 "$pid" 2>/dev/null; then
                    echo "$service stopped."
                    break
                fi
                sleep 0.5
            done
            if kill -0 "$pid" 2>/dev/null; then
                echo "$service did not stop; force killing..."
                kill -9 "$pid" 2>/dev/null || true
            fi
            stopped=1
        else
            echo "$service pidfile exists but process not running; removing pidfile."
        fi
        rm -f "$pidfile"
    fi
done

if [ "$stopped" -eq 0 ]; then
    echo "No running services found."
fi
SCRIPT

chmod +x "${RELEASE_DIR}/scripts/start-server.sh"
chmod +x "${RELEASE_DIR}/scripts/start-agent.sh"
chmod +x "${RELEASE_DIR}/scripts/stop.sh"

# ── Generate default config with web serving enabled ──
cat > "${RELEASE_DIR}/lightai-server.toml" << 'TOML'
# HTTP listener (default for initial setup).
# Run scripts/init-server.sh to enable HTTPS with self-signed certificates.
[http]
enabled = true
listen_addr = "0.0.0.0:18080"

[web]
dist_dir = "web/dist"

[database]
url = "sqlite://./data/lightai.db"

[metrics]
retention_days = 7

[auth.password]
min_length = 12
complexity_required = false
expires_days = 0
force_change_after_reset = true

[auth.session]
ttl_secs = 43200
idle_timeout_secs = 7200
secure_cookie = false

[logs]
dir = "logs"
level = "info"
max_file_bytes = 10485760
retention_files = 5
retention_days = 30
TOML

# ── 5. Check dynamic library dependencies ──
echo "[5/7] Checking dynamic library dependencies..."
{
    echo "=== lightai-server ==="
    ldd "${RELEASE_DIR}/bin/lightai-server" 2>&1 || true
    echo ""
    echo "=== lightai-agent ==="
    ldd "${RELEASE_DIR}/bin/lightai-agent" 2>&1 || true
} | tee "${RELEASE_DIR}/ldd-check.txt"

# ── 6. Check GLIBC symbols ──
echo "[6/7] Checking GLIBC symbols..."
GLIBC_LOG="${RELEASE_DIR}/glibc-symbols.txt"
{
    echo "=== lightai-server GLIBC symbols ==="
    strings "${RELEASE_DIR}/bin/lightai-server" | grep '^GLIBC_' | sort -u || echo "(none)"
    echo ""
    echo "=== lightai-agent GLIBC symbols ==="
    strings "${RELEASE_DIR}/bin/lightai-agent" | grep '^GLIBC_' | sort -u || echo "(none)"
} | tee "${GLIBC_LOG}"

# Strict checks for glibc2.28 package.
if [ "${SUFFIX}" = "glibc2.28" ]; then
    echo ""
    echo "  (glibc2.28 strict mode: checking for GLIBC_2.29+ symbols)"

    HIGHER_SYMS=$(strings "${RELEASE_DIR}/bin/lightai-server" "${RELEASE_DIR}/bin/lightai-agent" \
        | grep '^GLIBC_' | sort -u | grep -E 'GLIBC_2\.(29|[3-9][0-9]|[0-9]{3,})' || true)
    if [ -n "${HIGHER_SYMS}" ]; then
        echo "ERROR: binaries contain GLIBC symbols newer than 2.28:"
        echo "${HIGHER_SYMS}"
        exit 1
    fi

    if ldd "${RELEASE_DIR}/bin/lightai-server" 2>&1 | grep -qi 'libsqlite3'; then
        echo "ERROR: lightai-server depends on libsqlite3.so"
        exit 1
    fi
    if ldd "${RELEASE_DIR}/bin/lightai-agent" 2>&1 | grep -qi 'libsqlite3'; then
        echo "ERROR: lightai-agent depends on libsqlite3.so"
        exit 1
    fi

    echo "  GLIBC 2.28 check: PASSED"
    echo "  libsqlite3 check: PASSED"
fi

# ── 7. Package tar.gz ──
echo "[7/7] Creating tar.gz..."
cd release
tar czf "${RELEASE_NAME}.tar.gz" "${RELEASE_NAME}"
cd ..

echo ""
echo "=== Release package created ==="
echo "  release/${RELEASE_NAME}.tar.gz"
echo "  (${SUFFIX}, SQLite bundled, no libsqlite3.so required on target)"
echo ""
echo "To install:"
echo "  tar xzf release/${RELEASE_NAME}.tar.gz"
echo "  cd ${RELEASE_NAME}"
echo "  cat INSTALL.md"
