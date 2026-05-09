#!/usr/bin/env bash
set -euo pipefail

# ── Usage ──
#   bash scripts/package-release.sh [VERSION]
#
# Examples:
#   bash scripts/package-release.sh v0.1.0
#   bash scripts/package-release.sh v0.1.0-alpha

VERSION="${1:-v0.1.0}"
RELEASE_NAME="lightai-platform-${VERSION}-linux-x86_64"
RELEASE_DIR="release/${RELEASE_NAME}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${PROJECT_ROOT}"

echo "=== Building release ${VERSION} ==="

# ── 1. Build Rust binaries ──
echo "[1/6] Building Rust release binaries..."
cargo build --workspace --release

# ── 2. Build Web frontend ──
echo "[2/6] Building Web frontend..."
( cd "${PROJECT_ROOT}/web" && npm run build )

# ── 3. Assemble release directory ──
echo "[3/6] Assembling release directory..."
rm -rf "${RELEASE_DIR}"
mkdir -p "${RELEASE_DIR}"/{bin,web/dist,config,scripts,systemd,logs,run,data}

cp "${PROJECT_ROOT}/target/release/lightai-server" "${RELEASE_DIR}/bin/"
cp "${PROJECT_ROOT}/target/release/lightai-agent" "${RELEASE_DIR}/bin/"
cp -r "${PROJECT_ROOT}/web/dist/"* "${RELEASE_DIR}/web/dist/"
cp "${PROJECT_ROOT}/deploy/server.example.toml" "${RELEASE_DIR}/config/"
cp "${PROJECT_ROOT}/deploy/agent.example.toml" "${RELEASE_DIR}/config/"
cp "${PROJECT_ROOT}/deploy/lightai-server.service" "${RELEASE_DIR}/systemd/"
cp "${PROJECT_ROOT}/deploy/lightai-agent.service" "${RELEASE_DIR}/systemd/"
cp "${PROJECT_ROOT}/INSTALL.md" "${RELEASE_DIR}/"

# ── 4. Generate start/stop scripts ──
echo "[4/6] Generating start/stop scripts..."

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

nohup bin/lightai-agent --config "$CONFIG_FILE" > logs/agent-stdout.log 2> logs/agent-stderr.log &
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
            # Wait up to 10 seconds for graceful shutdown.
            for i in $(seq 1 20); do
                if ! kill -0 "$pid" 2>/dev/null; then
                    echo "$service stopped."
                    break
                fi
                sleep 0.5
            done
            # Force kill if still running.
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
[server]
listen_addr = "0.0.0.0:10080"

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
echo "[5/6] Checking dynamic library dependencies..."
LDD_LOG="${RELEASE_DIR}/ldd-check.txt"
{
    echo "=== lightai-server ==="
    ldd "${RELEASE_DIR}/bin/lightai-server" || true
    echo ""
    echo "=== lightai-agent ==="
    ldd "${RELEASE_DIR}/bin/lightai-agent" || true
    echo ""
    echo "SQLite is statically linked (libsqlite3-sys bundled)."
    echo "Target server does NOT need libsqlite3-dev or libsqlite3.so."
} > "${LDD_LOG}"
cat "${LDD_LOG}"

# ── 6. Package tar.gz ──
echo "[6/6] Creating tar.gz..."
cd release
tar czf "${RELEASE_NAME}.tar.gz" "${RELEASE_NAME}"
cd ..

echo ""
echo "=== Release package created ==="
echo "  release/${RELEASE_NAME}.tar.gz"
echo "  (SQLite bundled, no libsqlite3.so required on target)"
echo ""
echo "To install:"
echo "  tar xzf release/${RELEASE_NAME}.tar.gz"
echo "  cd ${RELEASE_NAME}"
echo "  cat INSTALL.md"
