#!/usr/bin/env bash
set -euo pipefail

# ── Usage ──
#   bash scripts/package-release-docker.sh [VERSION]
#
# Builds a glibc 2.28 compatible release package inside a Rocky Linux 8 container.
# Requires: docker

VERSION="${1:-v0.1.0}"
SUFFIX="glibc2.28"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DOCKERFILE="${PROJECT_ROOT}/Dockerfile.release.glibc228"
IMAGE_TAG="lightai-release-glibc228:latest"

echo "=== Building glibc 2.28 release ${VERSION} ==="

# ── 1. Build Docker image ──
echo "[1/4] Building Docker build image (Rocky Linux 8 + Rust + Node.js)..."
docker build -t "${IMAGE_TAG}" -f "${DOCKERFILE}" "${PROJECT_ROOT}"

# ── 2. Run build + test + package inside container ──
echo "[2/4] Running build inside container..."
docker run --rm \
    -v "${PROJECT_ROOT}:/build" \
    -v "${CARGO_HOME:-${HOME}/.cargo}/registry:/usr/local/cargo/registry" \
    "${IMAGE_TAG}" \
    bash -c "
set -euo pipefail
echo '=== Build environment ==='
gcc --version | head -1
rustc --version
node --version
npm --version
ldd --version | head -1
echo ''

echo '=== cargo build --release ==='
cargo build --workspace --release

echo ''
echo '=== cargo test --workspace ==='
cargo test --workspace

echo ''
echo '=== cargo clippy ==='
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo ''
echo '=== web build ==='
cd web && npm ci && npm run build && cd ..

echo ''
echo '=== package ==='
bash scripts/package-release.sh ${VERSION} ${SUFFIX}
"

# ── 3. Verify glibc symbols ──
echo "[3/4] Verifying glibc symbols..."
RELEASE_NAME="lightai-platform-${VERSION}-linux-x86_64-${SUFFIX}"
STAGING="${PROJECT_ROOT}/release/${RELEASE_NAME}"

echo "  ldd lightai-server:"
ldd "${STAGING}/bin/lightai-server" 2>&1 | grep -v linux-vdso || true

echo ""
echo "  ldd lightai-agent:"
ldd "${STAGING}/bin/lightai-agent" 2>&1 | grep -v linux-vdso || true

echo ""
echo "  GLIBC symbols (lightai-server):"
SERVER_GLIBC=$(strings "${STAGING}/bin/lightai-server" | grep '^GLIBC_' | sort -u)
echo "${SERVER_GLIBC}"

echo ""
echo "  GLIBC symbols (lightai-agent):"
AGENT_GLIBC=$(strings "${STAGING}/bin/lightai-agent" | grep '^GLIBC_' | sort -u)
echo "${AGENT_GLIBC}"

# Check both binaries for forbidden symbols (GLIBC_2.29+).
MAX_ALLOWED="GLIBC_2.28"
ALL_GLIBC="${SERVER_GLIBC}
${AGENT_GLIBC}"
HIGHER_SYMS=$(echo "${ALL_GLIBC}" | grep -E 'GLIBC_2\.(29|[3-9][0-9]|[0-9]{3,})' || true)
if [ -n "${HIGHER_SYMS}" ]; then
    echo ""
    echo "ERROR: one or both binaries contain GLIBC symbols newer than ${MAX_ALLOWED}:"
    echo "${HIGHER_SYMS}"
    echo "The glibc2.28 build has failed. The container base may have been updated."
    exit 1
fi

# Check both binaries for libsqlite3.so dependency.
for bin in lightai-server lightai-agent; do
    if ldd "${STAGING}/bin/${bin}" 2>&1 | grep -qi 'libsqlite3'; then
        echo "ERROR: ${bin} depends on libsqlite3.so"
        exit 1
    fi
done

echo ""
echo "  GLIBC check: PASSED (max ${MAX_ALLOWED})"
echo "  libsqlite3 check: PASSED (not present)"

# ── 4. Done ──
echo ""
echo "=== Release package created ==="
echo "  release/${RELEASE_NAME}.tar.gz"
echo "  (glibc 2.28 compatible, SQLite bundled, Web self-hosted)"
echo ""
echo "To install on RHEL 8 / Rocky 8 / AlmaLinux 8:"
echo "  tar xzf release/${RELEASE_NAME}.tar.gz"
echo "  cd ${RELEASE_NAME}"
echo "  cat INSTALL.md"
