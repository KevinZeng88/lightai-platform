#!/usr/bin/env bash
set -euo pipefail

# ── verify-local-deployment.sh ──────────────────────────────────────────────
# Smart local deployment verification for LightAI Platform.
#
# Default: auto-detect fresh vs update + restart, then verify, then stop.
#
# Usage:
#   bash scripts/verify-local-deployment.sh                        # auto
#   bash scripts/verify-local-deployment.sh --keep-running         # auto + keep
#   bash scripts/verify-local-deployment.sh --fresh --keep-running # force fresh
#   bash scripts/verify-local-deployment.sh --update --no-restart  # sync only
#   bash scripts/verify-local-deployment.sh --workdir /path/to/dir
#   bash scripts/verify-local-deployment.sh --help

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# ── Defaults ──
WORKDIR="/tmp/lightai-local-deployment"
FORCE_FRESH=false
FORCE_UPDATE=false
NO_RESTART=false
KEEP_RUNNING=true   # default: keep running for manual dev workflow
AUTO=false          # --auto: create admin + test login session
STOP=false          # --stop: stop services after verification
YES=false

usage() {
    cat << 'HELP'
lightai-platform local deployment verification

Usage:
  bash scripts/verify-local-deployment.sh [FLAGS]

Flags:
  --fresh          Force full re-initialization (certs, config, db).
  --update         Force incremental update (fail if not initialized).
  --no-restart     Only sync files, do not restart services.
  --stop           Stop services after verification (default: keep running).
  --auto           Auto-create admin + test login session (for CI/tools).
  --workdir <DIR>  Work directory (default: /tmp/lightai-local-deployment).
  --yes            Skip confirmation prompts for directory operations.
  --help           Show this message.

Examples:
  # Daily dev: first time auto-init, later auto-update+restart, keep running
  bash scripts/verify-local-deployment.sh

  # Force rebuild but keep first-setup page (no auto-admin)
  bash scripts/verify-local-deployment.sh --fresh --yes

  # CI / automated full test (create admin, verify login, stop when done)
  bash scripts/verify-local-deployment.sh --fresh --yes --auto --stop

  # Only sync files, don't restart
  bash scripts/verify-local-deployment.sh --update --no-restart
HELP
    exit 0
}

# ── Parse args ──
while [ $# -gt 0 ]; do
    case "$1" in
        --fresh) FORCE_FRESH=true; shift ;;
        --update) FORCE_UPDATE=true; shift ;;
        --no-restart) NO_RESTART=true; shift ;;
        --restart) NO_RESTART=false; shift ;;
        --auto) AUTO=true; shift ;;
        --stop) STOP=true; shift ;;
        --workdir) WORKDIR="$2"; shift 2 ;;
        --yes) YES=true; shift ;;
        --help|-h) usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

SERVER_DIR="${WORKDIR}/server"
AGENT_DIR="${WORKDIR}/agent"
SERVER_BIN="${SERVER_DIR}/bin/lightai-server"
AGENT_BIN="${AGENT_DIR}/bin/lightai-agent"
SERVER_CFG="${SERVER_DIR}/lightai-server.toml"
AGENT_CFG="${AGENT_DIR}/lightai-agent.toml"
SERVER_HEALTH="https://127.0.0.1:18443/health"

# ── Safety checks ──
if [ "$WORKDIR" = "/" ] || [ "$WORKDIR" = "$HOME" ] || [ "$WORKDIR" = "$PROJECT_ROOT" ]; then
    echo "ERROR: workdir cannot be /, \$HOME, or project root."
    exit 1
fi

# ── Determine mode ──
is_initialized() {
    [ -f "$SERVER_CFG" ] && [ -f "$SERVER_DIR/certs/ca.crt" ] && [ -f "$AGENT_CFG" ] && [ -f "$AGENT_DIR/certs/ca.crt" ]
}

MODE=""
if $FORCE_FRESH; then
    MODE="fresh"
elif $FORCE_UPDATE; then
    if ! is_initialized; then
        echo "ERROR: --update specified but workdir is not initialized."
        echo "Run without --update for auto-detection, or use --fresh first."
        exit 1
    fi
    MODE="update"
elif is_initialized; then
    MODE="update"
else
    MODE="fresh"
fi

echo "=== LightAI local deployment verification ==="
echo "  Mode:             ${MODE}"
echo "  Auto auth:        ${AUTO}"
echo "  Stop when done:   ${STOP}"
echo "  Workdir:          ${WORKDIR}"
echo ""

# ── fresh: ask before removing existing ──
if [ "$MODE" = "fresh" ] && [ -d "$WORKDIR" ]; then
    echo "Workdir already exists: ${WORKDIR}"
    if $YES; then
        echo "Removing (--yes)..."
    else
        read -r -p "Remove and re-initialize? [y/N] " confirm
        if [ "${confirm,,}" != "y" ] && [ "${confirm,,}" != "yes" ]; then
            echo "Aborted."
            exit 0
        fi
    fi
    BACKUP="${WORKDIR}.backup.$(date +%Y%m%d%H%M%S)"
    echo "Moving to ${BACKUP} ..."
    mv "$WORKDIR" "$BACKUP"
fi

# ── Build ──
echo "[build] cargo build --workspace..."
cargo build --workspace

echo "[build] cd web && npm run build..."
( cd "$PROJECT_ROOT/web" && npm run build )

# ── Assemble server ──
echo "[assemble] server..."
mkdir -p "${SERVER_DIR}"/{bin,web/dist,collectors/gpu,scripts,certs,data,logs,run}
cp "${PROJECT_ROOT}/target/debug/lightai-server" "${SERVER_BIN}"
cp -r "${PROJECT_ROOT}/web/dist/"* "${SERVER_DIR}/web/dist/"
cp -r "${PROJECT_ROOT}/deploy/collectors/"* "${SERVER_DIR}/collectors/"
cp "${PROJECT_ROOT}/scripts/init-server.sh" "${SERVER_DIR}/scripts/"

# start/stop scripts for server
cat > "${SERVER_DIR}/scripts/start-server.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p run logs data
if [ -f run/lightai-server.pid ] && kill -0 "$(cat run/lightai-server.pid)" 2>/dev/null; then
    echo "Server already running (pid $(cat run/lightai-server.pid))"
    exit 0
fi
nohup bin/lightai-server --config lightai-server.toml > logs/lightai-server.log 2>&1 &
echo $! > run/lightai-server.pid
echo "Server started (pid $!)"
SCRIPT

cat > "${SERVER_DIR}/scripts/stop-server.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -f run/lightai-server.pid ]; then
    pid=$(cat run/lightai-server.pid)
    if kill -0 "$pid" 2>/dev/null; then
        echo "Stopping lightai-server (pid $pid)..."
        kill "$pid"
        for i in $(seq 1 20); do
            if ! kill -0 "$pid" 2>/dev/null; then echo "Server stopped."; break; fi
            sleep 0.5
        done
        kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f run/lightai-server.pid
fi
SCRIPT
chmod +x "${SERVER_DIR}/scripts/start-server.sh" "${SERVER_DIR}/scripts/stop-server.sh"

# ── Assemble agent ──
echo "[assemble] agent..."
mkdir -p "${AGENT_DIR}"/{bin,collectors/gpu,scripts,certs,data,logs,run}
cp "${PROJECT_ROOT}/target/debug/lightai-agent" "${AGENT_BIN}"
cp -r "${PROJECT_ROOT}/deploy/collectors/"* "${AGENT_DIR}/collectors/"
cp "${PROJECT_ROOT}/scripts/init-agent.sh" "${AGENT_DIR}/scripts/"

cat > "${AGENT_DIR}/scripts/start-agent.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p run logs data
if [ -f run/lightai-agent.pid ] && kill -0 "$(cat run/lightai-agent.pid)" 2>/dev/null; then
    echo "Agent already running (pid $(cat run/lightai-agent.pid))"
    exit 0
fi
nohup bin/lightai-agent --config lightai-agent.toml > logs/lightai-agent.log 2>&1 &
echo $! > run/lightai-agent.pid
echo "Agent started (pid $!)"
SCRIPT

cat > "${AGENT_DIR}/scripts/stop-agent.sh" << 'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -f run/lightai-agent.pid ]; then
    pid=$(cat run/lightai-agent.pid)
    if kill -0 "$pid" 2>/dev/null; then
        echo "Stopping lightai-agent (pid $pid)..."
        kill "$pid"
        for i in $(seq 1 20); do
            if ! kill -0 "$pid" 2>/dev/null; then echo "Agent stopped."; break; fi
            sleep 0.5
        done
        kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f run/lightai-agent.pid
fi
SCRIPT
chmod +x "${AGENT_DIR}/scripts/start-agent.sh" "${AGENT_DIR}/scripts/stop-agent.sh"

# ── fresh mode: init server + agent ──
if [ "$MODE" = "fresh" ]; then
    echo ""
    echo "=== Fresh initialization ==="

    echo "[server] init-server.sh --host 127.0.0.1 --yes ..."
    ( cd "$SERVER_DIR" && bash scripts/init-server.sh --host 127.0.0.1 --yes )

    # Start server temporarily so agent can download CA.
    echo "[server] starting temporarily for agent init..."
    ( cd "$SERVER_DIR" && bash scripts/start-server.sh )
    sleep 4

    echo "[agent] init-agent.sh --server https://127.0.0.1:18443 --name local-agent --yes ..."
    ( cd "$AGENT_DIR" && bash scripts/init-agent.sh --server https://127.0.0.1:18443 --name local-agent --yes )

    # Stop server so main start flow can restart cleanly.
    ( cd "$SERVER_DIR" && bash scripts/stop-server.sh ) || true
    sleep 2
fi

# ── Stop existing services ──
echo ""
echo "=== Stopping existing services ==="
( cd "$AGENT_DIR" && bash scripts/stop-agent.sh ) || true
( cd "$SERVER_DIR" && bash scripts/stop-server.sh ) || true
sleep 1

# Fallback: check if ports are still occupied by lightai processes.
for port in 18081 18443; do
    if fuser "${port}/tcp" 2>/dev/null | grep -q .; then
        echo "  Port ${port} still occupied:"
        fuser -v "${port}/tcp" 2>/dev/null || true
        echo "  Cleaning up leftover lightai processes on port ${port}..."
        fuser -k "${port}/tcp" 2>/dev/null || true
    fi
done
sleep 2

# Start server
echo ""
echo "=== Starting Server ==="
( cd "$SERVER_DIR" && bash scripts/start-server.sh )
sleep 4

# ── Verify Server ──
echo ""
echo "=== Server verification ==="

check() { local desc="$1" url="$2" expect_code="$3" expect_body="$4"
    local code body
    code=$(curl -sk -o /dev/null -w "%{http_code}" --connect-timeout 5 "$url" 2>/dev/null || echo "000")
    # Normalize 000000 → 000 (connection refused variants)
    code=$(echo "$code" | sed 's/^000000$/000/')
    body=$(curl -sk --connect-timeout 5 "$url" 2>/dev/null | head -c 200 || echo "")
    if [ "$code" = "$expect_code" ]; then
        if [ -n "$expect_body" ] && ! echo "$body" | grep -q "$expect_body"; then
            echo "  WARN  $desc: code $code OK but body missing '$expect_body'"
        else
            echo "  PASS  $desc ($code)"
        fi
    else
        echo "  FAIL  $desc: got $code, expected $expect_code"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

FAIL_COUNT=0

echo "  -- Server endpoints --"
check "/health"              "$SERVER_HEALTH"                                 "200" '"status":"ok"'
check "/ (index.html)"       "https://127.0.0.1:18443/"                       "200" "<!doctype html"
check "/nodes (SPA)"         "https://127.0.0.1:18443/nodes"                  "200" "<!doctype html"
check "/api/nonexistent"     "https://127.0.0.1:18443/api/nonexistent-ep"     "404" "not_found"
check "/assets/missing.js"   "https://127.0.0.1:18443/assets/missing.js"      "404" ""
check "HTTP 18080 closed"    "http://127.0.0.1:18080/health"                   "000" ""

echo "  -- Well-known --"
check "/.well-known/ca.crt"       "https://127.0.0.1:18443/.well-known/lightai/ca.crt"        "200" "BEGIN CERTIFICATE"
check "/.well-known/ca-fingerprint" "https://127.0.0.1:18443/.well-known/lightai/ca-fingerprint" "200" "sha256"

echo "  -- CA fingerprint match --"
CA_FP_SERVER=$(cargo run -p lightai-server -- cert fingerprint --file "${SERVER_DIR}/certs/ca.crt" 2>/dev/null || echo "err")
CA_FP_WELLKNOWN=$(curl -sk https://127.0.0.1:18443/.well-known/lightai/ca-fingerprint 2>/dev/null | grep -o '"sha256":"[^"]*"' | cut -d'"' -f4 || echo "")
if [ "$CA_FP_SERVER" = "$CA_FP_WELLKNOWN" ]; then
    echo "  PASS  CA fingerprint match: ${CA_FP_SERVER}"
else
    echo "  FAIL  CA fingerprint mismatch: server=$CA_FP_SERVER wellknown=$CA_FP_WELLKNOWN"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

echo "  -- Web assets check --"
INDEX_HTML=$(curl -sk https://127.0.0.1:18443/ 2>/dev/null)
MISSING_ASSETS=0
for src in $(echo "$INDEX_HTML" | grep -oP 'src="[^"]*\.js"' | sed 's/src="//;s/"//'); do
    code=$(curl -sk -o /dev/null -w "%{http_code}" "https://127.0.0.1:18443${src}" 2>/dev/null)
    if [ "$code" != "200" ]; then
        echo "  WARN  asset ${src}: ${code}"
        MISSING_ASSETS=$((MISSING_ASSETS + 1))
    fi
done
if [ "$MISSING_ASSETS" -eq 0 ]; then
    echo "  PASS  All web assets reachable"
else
    echo "  WARN  ${MISSING_ASSETS} assets not reachable"
fi

# ── Collector sync (idempotent, safe for both fresh and update) ──
echo ""
echo "=== Collector sync ==="
COLLECTOR_SYNC_OUT=$(cargo run -p lightai-server -- --config "${SERVER_CFG}" collector sync --root "${SERVER_DIR}/collectors/gpu" 2>&1) || true
if echo "$COLLECTOR_SYNC_OUT" | grep -q -E "registered|updated"; then
    echo "  PASS  Collector sync succeeded: $(echo "$COLLECTOR_SYNC_OUT" | grep -E 'registered|updated')"
else
    echo "  FAIL  Collector sync failed: ${COLLECTOR_SYNC_OUT}"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Start Agent ──
echo ""
echo "=== Starting Agent ==="
( cd "$AGENT_DIR" && bash scripts/start-agent.sh )
sleep 4

# ── Verify Agent (with retry) ──
echo ""
echo "=== Agent verification ==="

# Agent writes to agent.log (platform_log) AND lightai-agent.log (nohup redirect).
AGENT_LOG1="${AGENT_DIR}/logs/lightai-agent.log"
AGENT_LOG2="${AGENT_DIR}/logs/agent.log"  # legacy compat
AGENT_REGISTERED=false
AGENT_HEARTBEAT=false

# Agent communication evidence: any of these patterns in log.
AGENT_CONNECT_EVIDENCE="Agent registered successfully\|registered successfully\|config_version=\|config updated\|GPU collector registry fetch\|GPU probe:"

grep_agent_logs() {
    local pattern="$1"
    (grep -q "$pattern" "$AGENT_LOG1" 2>/dev/null || grep -q "$pattern" "$AGENT_LOG2" 2>/dev/null)
}

# Also check Server DB for node registration (fallback evidence).
check_db_for_node() {
    local db="${SERVER_DIR}/data/lightai.db"
    if [ -f "$db" ] && command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$db" "SELECT COUNT(*) FROM nodes WHERE name='local-agent'" 2>/dev/null | grep -q '1' && return 0
    fi
    return 1
}

AGENT_LOG_FOUND=false
for attempt in $(seq 1 15); do
    if grep_agent_logs "$AGENT_CONNECT_EVIDENCE"; then
        AGENT_REGISTERED=true
        AGENT_HEARTBEAT=true
        AGENT_LOG_FOUND=true
        break
    fi
    if check_db_for_node; then
        AGENT_REGISTERED=true
        AGENT_HEARTBEAT=true
        echo "  (agent confirmed via DB, attempt ${attempt}/15)"
        break
    fi
    if [ $attempt -lt 15 ]; then
        echo "  (waiting for agent... attempt ${attempt}/15)"
        sleep 2
    fi
done

if $AGENT_REGISTERED; then
    echo "  PASS  Agent connected (log evidence)"
else
    echo "  FAIL  Agent not connected (waited 30s)"
    echo "  Agent log dir:"
    ls -la "${AGENT_DIR}/logs/" 2>/dev/null || echo "  (no logs dir)"
    echo "  --- lightai-agent.log (last 100 lines) ---"
    tail -100 "$AGENT_LOG1" 2>/dev/null || echo "  (not found)"
    echo "  --- lightai-server.log (last 100 lines) ---"
    tail -100 "${SERVER_DIR}/logs/lightai-server.log" 2>/dev/null || echo "  (not found)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

if [ -f "$AGENT_LOG1" ] || [ -f "$AGENT_LOG2" ]; then
    if grep_agent_logs 'bearer [a-f0-9]\{32,\}'; then
        echo "  FAIL  Bearer token found in agent log"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    else
        echo "  PASS  No token in agent log"
    fi
else
    echo "  FAIL  Agent log not found"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# Check config
echo "  -- Agent config check --"
if grep -q 'url = "https://127.0.0.1:18443"' "$AGENT_CFG" 2>/dev/null; then
    echo "  PASS  server.url = https://127.0.0.1:18443"
else
    echo "  FAIL  server.url mismatch"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi
if grep -q 'ca_cert_path = "./certs/ca.crt"' "$AGENT_CFG" 2>/dev/null; then
    echo "  PASS  ca_cert_path = ./certs/ca.crt"
else
    echo "  FAIL  ca_cert_path mismatch"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi
if grep -q 'insecure_skip_tls_verify = false' "$AGENT_CFG" 2>/dev/null; then
    echo "  PASS  insecure_skip_tls_verify = false"
else
    echo "  FAIL  insecure_skip_tls_verify != false"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Server log check ──
echo ""
echo "  -- Server log check --"
SERVER_LOG="${SERVER_DIR}/logs/lightai-server.log"
if [ -f "$SERVER_LOG" ] && [ -s "$SERVER_LOG" ]; then
    echo "  PASS  Server log non-empty ($(wc -c < "$SERVER_LOG") bytes)"
else
    echo "  FAIL  Server log is empty or missing: ${SERVER_LOG}"
    echo "  Logs directory contents:"
    ls -la "${SERVER_DIR}/logs/" 2>/dev/null || echo "  (no logs dir)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Auth/session cookie jar test (only in --auto mode) ──
if $AUTO; then
    echo ""
    echo "  -- Auth session test (--auto) --"
    SETUP_TOKEN=$(grep -oP 'setup_token = "\K[^"]+' "${SERVER_CFG}" 2>/dev/null || echo "")
    COOKIE_JAR="/tmp/lightai-verify-cookies.txt"
    rm -f "$COOKIE_JAR"

    if [ -n "$SETUP_TOKEN" ]; then
        SETUP_RESP=$(curl -sk -X POST https://127.0.0.1:18443/api/setup/admin \
            -H "Content-Type: application/json" \
            -d "{\"username\":\"admin\",\"password\":\"test-admin-pw-123\",\"setup_token\":\"${SETUP_TOKEN}\"}" 2>/dev/null)
        if echo "$SETUP_RESP" | grep -q '"username":"admin"'; then
            echo "  PASS  Admin created via setup token"
        elif echo "$SETUP_RESP" | grep -q "already completed\|already"; then
            echo "  (admin already exists, continuing)"
        else
            echo "  WARN  Setup response: $(echo "$SETUP_RESP" | head -c 80)"
        fi

        LOGIN_RESP=$(curl -sk -i -c "$COOKIE_JAR" -X POST https://127.0.0.1:18443/api/auth/login \
            -H "Content-Type: application/json" \
            -d '{"username":"admin","password":"test-admin-pw-123"}' 2>/dev/null)
        if echo "$LOGIN_RESP" | grep -qi "set-cookie"; then
            echo "  PASS  Login returned Set-Cookie"
        else
            echo "  FAIL  Login did not return Set-Cookie"
            FAIL_COUNT=$((FAIL_COUNT + 1))
        fi

        ME_RESP=$(curl -sk -b "$COOKIE_JAR" https://127.0.0.1:18443/api/auth/me 2>/dev/null)
        if echo "$ME_RESP" | grep -q '"username":"admin"'; then
            echo "  PASS  Session cookie valid (/api/auth/me returned user)"
        else
            echo "  FAIL  Session cookie invalid (/api/auth/me: $(echo "$ME_RESP" | head -c 80))"
            FAIL_COUNT=$((FAIL_COUNT + 1))
        fi
    else
        echo "  WARN  Could not read setup_token from config; skipping auth test"
    fi
    rm -f "$COOKIE_JAR"
else
    echo ""
    echo "  SKIP  Auto setup/login verification (manual mode; use --auto for automated test)"
fi

# ── Collector check ──
echo ""
echo "  -- Collector check --"
COLLECTOR_OUT=$(cargo run -p lightai-server -- --config "${SERVER_CFG}" collector inspect --root "${SERVER_DIR}/collectors/gpu" 2>/dev/null || echo "")
if echo "$COLLECTOR_OUT" | grep -q 'nvidia'; then
    echo "  PASS  Collector nvidia-wsl present"
else
    echo "  FAIL  Collector nvidia-wsl not found in registry"
    echo "  output: $(echo "$COLLECTOR_OUT" | head -3)"
    FAIL_COUNT=$((FAIL_COUNT + 1))
fi

# ── Final ──
echo ""
echo "============================================"
if [ "$FAIL_COUNT" -eq 0 ]; then
    echo "  VERIFICATION PASSED"
else
    echo "  VERIFICATION FAILED (${FAIL_COUNT} failures)"
fi
echo "  Workdir:  ${WORKDIR}"
echo "  Web:      https://127.0.0.1:18443/"
echo "  Logs:     ${SERVER_DIR}/logs/ ${AGENT_DIR}/logs/"
echo "============================================"

if $STOP; then
    echo ""
    echo "Stopping services (--stop)..."
    ( cd "$AGENT_DIR" && bash scripts/stop-agent.sh ) || true
    ( cd "$SERVER_DIR" && bash scripts/stop-server.sh ) || true
else
    echo ""
    echo "Services kept running (default)."
    echo "Stop manually:"
    echo "  cd ${AGENT_DIR} && bash scripts/stop-agent.sh"
    echo "  cd ${SERVER_DIR} && bash scripts/stop-server.sh"
fi

exit $FAIL_COUNT
