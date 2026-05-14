#!/usr/bin/env bash
set -euo pipefail

# ── verify-local-deployment.sh ──────────────────────────────────────────────
# Smart local deployment verification for LightAI Platform.
#
# Default: auto-detect fresh vs update, stop + reassemble + restart, then verify.
# Services stay running after verification for manual dev workflow.
#
# Usage:
#   bash scripts/verify-local-deployment.sh                        # auto
#   bash scripts/verify-local-deployment.sh --stop                 # stop only
#   bash scripts/verify-local-deployment.sh --stop-after-verify    # verify then stop
#   bash scripts/verify-local-deployment.sh --fresh                # force re-init
#   bash scripts/verify-local-deployment.sh --clean                # remove deploy dir
#   bash scripts/verify-local-deployment.sh --auto                 # CI auto-auth test
#   bash scripts/verify-local-deployment.sh --help
#
# Environment:
#   LIGHTAI_VERIFY_DIR    Override deployment directory
#                         (default: <project>/.local/lightai-deployment)

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# ── Defaults ──
WORKDIR="${LIGHTAI_VERIFY_DIR:-${PROJECT_ROOT}/.local/lightai-deployment}"
FORCE_FRESH=false
FORCE_UPDATE=false
NO_RESTART=false
AUTO=false
STOP=false
STOP_AFTER_VERIFY=false
CLEAN=false
YES=false

usage() {
    cat << 'HELP'
lightai-platform local deployment verification

Usage:
  bash scripts/verify-local-deployment.sh [FLAGS]

Flags:
  --fresh               Force full re-initialization (certs, config, db).
  --update              Force incremental update (fail if not initialized).
  --no-restart          Only sync files, don't restart services.
  --stop                Stop services and exit (no build, no verify).
  --stop-after-verify   Run full verify, then stop services.
  --clean               Remove the entire deployment directory and exit.
                        Prompts for confirmation unless --yes is also given.
  --auto                Auto-create admin + test login session (for CI/tools).
  --workdir <DIR>       Work directory (default: .local/lightai-deployment).
  --yes                 Skip confirmation prompts.
  --help                Show this message.

Environment:
  LIGHTAI_VERIFY_DIR    Override deployment directory.

Examples:
  # Daily dev: first time auto-init, later auto-update+restart, keep running
  bash scripts/verify-local-deployment.sh

  # Force fresh reinit
  bash scripts/verify-local-deployment.sh --fresh --yes

  # Stop all services without removing data
  bash scripts/verify-local-deployment.sh --stop

  # CI / automated full test (create admin, verify login, stop when done)
  bash scripts/verify-local-deployment.sh --fresh --yes --auto --stop-after-verify

  # Clean up everything
  bash scripts/verify-local-deployment.sh --clean --yes

  # Custom workdir
  LIGHTAI_VERIFY_DIR=/path/to/deploy bash scripts/verify-local-deployment.sh
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
        --stop-after-verify) STOP_AFTER_VERIFY=true; shift ;;
        --clean) CLEAN=true; shift ;;
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

# ═══════════════════════════════════════════════════════════════════════
# Helper functions
# ═══════════════════════════════════════════════════════════════════════

stop_by_pidfile() {
    local label="$1" pidfile="$2"
    if [ -f "$pidfile" ]; then
        local pid
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then
            echo "  Stopping ${label} (pid ${pid})..."
            kill "$pid"
            local i
            for i in $(seq 1 20); do
                if ! kill -0 "$pid" 2>/dev/null; then
                    echo "  ${label} stopped."
                    break
                fi
                sleep 0.5
            done
            if kill -0 "$pid" 2>/dev/null; then
                echo "  ${label} did not stop; force killing..."
                kill -9 "$pid" 2>/dev/null || true
                sleep 1
            fi
        else
            echo "  ${label} pidfile exists but process not running."
        fi
        rm -f "$pidfile"
    fi
}

port_is_free() {
    local port="$1"
    if command -v fuser >/dev/null 2>&1; then
        fuser "${port}/tcp" 2>/dev/null | grep -q . && return 1 || return 0
    fi
    if command -v ss >/dev/null 2>&1; then
        ss -tlnp 2>/dev/null | grep -q ":${port} " && return 1 || return 0
    fi
    return 0
}

stop_services() {
    echo "=== Stopping control-plane services ==="
    if [ -d "$AGENT_DIR" ]; then
        stop_by_pidfile "lightai-agent" "${AGENT_DIR}/run/lightai-agent.pid"
    fi
    if [ -d "$SERVER_DIR" ]; then
        stop_by_pidfile "lightai-server" "${SERVER_DIR}/run/lightai-server.pid"
    fi

    local waited=false
    for port in 18081 18443; do
        local attempt=0
        while ! port_is_free "$port"; do
            if [ $attempt -eq 0 ]; then
                echo "  Waiting for port ${port} to be released..."
                waited=true
            fi
            attempt=$((attempt + 1))
            if [ $attempt -gt 15 ]; then
                echo "  WARNING: Port ${port} still occupied after 15s."
                if command -v fuser >/dev/null 2>&1; then
                    echo "  Processes on port ${port}:"
                    fuser -v "${port}/tcp" 2>/dev/null || true
                fi
                echo "  Fallback: force-freeing port ${port}..."
                fuser -k "${port}/tcp" 2>/dev/null || true
                sleep 1
                break
            fi
            sleep 1
        done
    done
    if $waited; then
        sleep 1
    fi
}

assemble() {
    echo "[assemble] server..."
    mkdir -p "${SERVER_DIR}"/{bin,web/dist,collectors/gpu,scripts,certs,data,logs,run}
    cp "${PROJECT_ROOT}/target/debug/lightai-server" "${SERVER_BIN}"
    cp -r "${PROJECT_ROOT}/web/dist/"* "${SERVER_DIR}/web/dist/"
    cp -r "${PROJECT_ROOT}/deploy/collectors/"* "${SERVER_DIR}/collectors/"
    cp "${PROJECT_ROOT}/scripts/init-server.sh" "${SERVER_DIR}/scripts/"

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
}

# ═══════════════════════════════════════════════════════════════════════
# Helper: sync agent CA from server CA
# ═══════════════════════════════════════════════════════════════════════

sync_agent_ca() {
    local server_ca="${SERVER_DIR}/certs/ca.crt"
    local agent_ca="${AGENT_DIR}/certs/ca.crt"

    if [ ! -f "$server_ca" ]; then
        echo "  WARNING: server CA not found at ${server_ca}; skip agent CA sync"
        return 0
    fi

    if [ ! -f "$agent_ca" ]; then
        echo "  Syncing agent CA from server CA (agent CA missing)..."
        mkdir -p "$(dirname "$agent_ca")"
        cp "$server_ca" "$agent_ca"
        return 0
    fi

    if ! cmp -s "$server_ca" "$agent_ca"; then
        echo "  Syncing agent CA from server CA (fingerprint mismatch)..."
        cp "$server_ca" "$agent_ca"
    fi
}

# ═══════════════════════════════════════════════════════════════════════
# Helper: preflight TLS check — verify server cert against agent CA
# ═══════════════════════════════════════════════════════════════════════

preflight_tls_check() {
    local server_ca="${SERVER_DIR}/certs/ca.crt"
    local server_cert="${SERVER_DIR}/certs/server.crt"
    local agent_ca="${AGENT_DIR}/certs/ca.crt"

    if [ ! -f "$server_cert" ] || [ ! -f "$agent_ca" ]; then
        return 0
    fi

    if command -v openssl >/dev/null 2>&1; then
        if openssl verify -CAfile "$agent_ca" "$server_cert" >/dev/null 2>&1; then
            echo "  TLS preflight OK: agent CA validates server certificate"
        else
            echo "  TLS preflight FAILED: agent CA does NOT validate server certificate"
            echo "  Server CA fingerprint:"
            openssl x509 -in "${SERVER_DIR}/certs/ca.crt" -noout -fingerprint -sha256 2>/dev/null || true
            echo "  Agent CA fingerprint:"
            openssl x509 -in "${AGENT_DIR}/certs/ca.crt" -noout -fingerprint -sha256 2>/dev/null || true
            echo "  Agent CA will be re-synced from server CA."
            cp "${SERVER_DIR}/certs/ca.crt" "${AGENT_DIR}/certs/ca.crt"
        fi
    else
        # Fallback: compare SHA256 fingerprints.
        if command -v sha256sum >/dev/null 2>&1; then
            local s_fp a_fp
            s_fp=$(sha256sum "$server_ca" | awk '{print $1}')
            a_fp=$(sha256sum "$agent_ca" | awk '{print $1}')
            if [ "$s_fp" != "$a_fp" ]; then
                echo "  Agent CA fingerprint differs from server CA; syncing..."
                echo "    server CA sha256: ${s_fp}"
                echo "    agent  CA sha256: ${a_fp}"
                cp "$server_ca" "$agent_ca"
            fi
        fi
    fi
}

# ═══════════════════════════════════════════════════════════════════════
# Early-exit paths: --clean, --stop
# ═══════════════════════════════════════════════════════════════════════

# --clean: stop services, then remove entire deployment directory.
if $CLEAN; then
    if [ ! -d "$WORKDIR" ]; then
        echo "Deployment directory does not exist: ${WORKDIR}"
        exit 0
    fi
    echo "=== Clean deployment directory ==="
    echo "  Directory: ${WORKDIR}"
    if $YES; then
        echo "  Removing (--yes)..."
    else
        read -r -p "  Remove entire deployment directory? This deletes all data, certs, and configs. [y/N] " confirm
        if [ "${confirm,,}" != "y" ] && [ "${confirm,,}" != "yes" ]; then
            echo "  Aborted."
            exit 0
        fi
    fi
    stop_services
    rm -rf "$WORKDIR"
    echo "  Removed: ${WORKDIR}"
    exit 0
fi

# --stop: stop services only, then exit.  No build, no assemble, no verify.
if $STOP; then
    stop_services
    echo ""
    echo "Services stopped (--stop)."
    echo "  Workdir preserved: ${WORKDIR}"
    echo "  Data, certs, configs are NOT removed."
    echo "  Restart with: bash scripts/verify-local-deployment.sh"
    exit 0
fi

# ═══════════════════════════════════════════════════════════════════════
# Determine mode: fresh vs update
# ═══════════════════════════════════════════════════════════════════════
is_initialized() {
    [ -f "$SERVER_CFG" ] && \
    [ -f "$SERVER_DIR/certs/ca.crt" ] && \
    [ -f "$AGENT_CFG" ] && \
    [ -f "$AGENT_DIR/certs/ca.crt" ]
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
echo "  Mode:              ${MODE}"
echo "  Auto auth:         ${AUTO}"
echo "  Stop after verify: ${STOP_AFTER_VERIFY}"
echo "  No restart:        ${NO_RESTART}"
echo "  Workdir:           ${WORKDIR}"
echo ""

# ═══════════════════════════════════════════════════════════════════════
# Build
# ═══════════════════════════════════════════════════════════════════════
echo "[build] cargo build --workspace..."
cargo build --workspace

echo "[build] cd web && npm run build..."
( cd "$PROJECT_ROOT/web" && npm run build )

# ═══════════════════════════════════════════════════════════════════════
# fresh: ask before removing existing workdir
# ═══════════════════════════════════════════════════════════════════════
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

# ═══════════════════════════════════════════════════════════════════════
# update: stop services BEFORE copying binaries (fixes "Text file busy")
# ═══════════════════════════════════════════════════════════════════════
if [ "$MODE" = "update" ]; then
    stop_services
fi

# ═══════════════════════════════════════════════════════════════════════
# Assemble (copy binaries, scripts, web assets)
# ═══════════════════════════════════════════════════════════════════════
assemble

# ═══════════════════════════════════════════════════════════════════════
# fresh: init server + agent (certs, config, DB, token)
# ═══════════════════════════════════════════════════════════════════════
if [ "$MODE" = "fresh" ]; then
    echo ""
    echo "=== Fresh initialization ==="

    echo "[server] init-server.sh --host 127.0.0.1 --yes ..."
    ( cd "$SERVER_DIR" && bash scripts/init-server.sh --host 127.0.0.1 --yes )

    echo "[server] starting temporarily for agent init..."
    ( cd "$SERVER_DIR" && bash scripts/start-server.sh )
    sleep 4

    echo "[agent] init-agent.sh --server https://127.0.0.1:18443 --name local-agent --yes ..."
    ( cd "$AGENT_DIR" && bash scripts/init-agent.sh --server https://127.0.0.1:18443 --name local-agent --yes )

    ( cd "$SERVER_DIR" && bash scripts/stop-server.sh ) || true
    sleep 2
fi

# ═══════════════════════════════════════════════════════════════════════
# Sync agent CA from server CA (both fresh and update)
# ═══════════════════════════════════════════════════════════════════════
sync_agent_ca

# ═══════════════════════════════════════════════════════════════════════
# Start services (skip if --no-restart)
# ═══════════════════════════════════════════════════════════════════════
if ! $NO_RESTART; then
    if [ "$MODE" = "fresh" ]; then
        stop_services
    fi

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
    check "/.well-known/ca.crt"          "https://127.0.0.1:18443/.well-known/lightai/ca.crt"          "200" "BEGIN CERTIFICATE"
    check "/.well-known/ca-fingerprint"  "https://127.0.0.1:18443/.well-known/lightai/ca-fingerprint"  "200" "sha256"

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

    # ── Collector sync ──
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
    echo "=== Agent preflight ==="
    preflight_tls_check
    echo ""
    echo "=== Starting Agent ==="
    ( cd "$AGENT_DIR" && bash scripts/start-agent.sh )
    sleep 4

    # ── Verify Agent ──
    echo ""
    echo "=== Agent verification ==="

    AGENT_LOG1="${AGENT_DIR}/logs/lightai-agent.log"
    AGENT_REGISTERED=false

    AGENT_CONNECT_EVIDENCE="Agent registered successfully\|registered successfully\|config_version=\|config updated\|GPU collector registry fetch\|GPU probe:"

    grep_agent_logs() {
        local pattern="$1"
        grep -q "$pattern" "$AGENT_LOG1" 2>/dev/null
    }

    check_db_for_node() {
        local db="${SERVER_DIR}/data/lightai.db"
        if [ -f "$db" ] && command -v sqlite3 >/dev/null 2>&1; then
            sqlite3 "$db" "SELECT COUNT(*) FROM nodes WHERE name='local-agent'" 2>/dev/null | grep -q '1' && return 0
        fi
        return 1
    }

    for attempt in $(seq 1 15); do
        if grep_agent_logs "$AGENT_CONNECT_EVIDENCE"; then
            AGENT_REGISTERED=true
            break
        fi
        if check_db_for_node; then
            AGENT_REGISTERED=true
            echo "  (agent confirmed via DB, attempt ${attempt}/15)"
            break
        fi
        if [ $attempt -lt 15 ]; then
            echo "  (waiting for agent... attempt ${attempt}/15)"
            sleep 2
        fi
    done

    if $AGENT_REGISTERED; then
        echo "  PASS  Agent connected"
    else
        echo "  FAIL  Agent not connected (waited 30s)"
        echo "  -- TLS diagnostic --"
        if [ -f "${AGENT_DIR}/certs/ca.crt" ] && [ -f "${SERVER_DIR}/certs/server.crt" ]; then
            if command -v openssl >/dev/null 2>&1; then
                echo "  openssl verify result:"
                openssl verify -CAfile "${AGENT_DIR}/certs/ca.crt" "${SERVER_DIR}/certs/server.crt" 2>&1 || true
            fi
            echo "  Server CA sha256: $(sha256sum "${SERVER_DIR}/certs/ca.crt" 2>/dev/null | awk '{print $1}')"
            echo "  Agent  CA sha256: $(sha256sum "${AGENT_DIR}/certs/ca.crt" 2>/dev/null | awk '{print $1}')"
        fi
        echo "  Agent log dir:"
        ls -la "${AGENT_DIR}/logs/" 2>/dev/null || echo "  (no logs dir)"
        echo "  --- lightai-agent.log (last 50 lines) ---"
        tail -50 "$AGENT_LOG1" 2>/dev/null || echo "  (not found)"
        echo "  --- lightai-server.log (last 20 lines) ---"
        tail -20 "${SERVER_DIR}/logs/lightai-server.log" 2>/dev/null || echo "  (not found)"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi

    if [ -f "$AGENT_LOG1" ]; then
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
        ls -la "${SERVER_DIR}/logs/" 2>/dev/null || echo "  (no logs dir)"
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi

    # ── Auth test (--auto) ──
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
                echo "  FAIL  Session cookie invalid"
                FAIL_COUNT=$((FAIL_COUNT + 1))
            fi
        else
            echo "  WARN  Could not read setup_token from config; skipping auth test"
        fi
        rm -f "$COOKIE_JAR"
    else
        echo ""
        echo "  SKIP  Auto setup/login verification (manual mode; use --auto)"
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
fi

# ═══════════════════════════════════════════════════════════════════════
# Stop after verify (--stop-after-verify)
# ═══════════════════════════════════════════════════════════════════════
if $STOP_AFTER_VERIFY; then
    echo ""
    echo "Stopping services (--stop-after-verify)..."
    ( cd "$AGENT_DIR" && bash scripts/stop-agent.sh ) || true
    ( cd "$SERVER_DIR" && bash scripts/stop-server.sh ) || true
elif ! $NO_RESTART; then
    echo ""
    echo "Services kept running (default)."
    echo "Stop with:  bash scripts/verify-local-deployment.sh --stop"
fi

exit ${FAIL_COUNT:-0}
