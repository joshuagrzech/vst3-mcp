#!/usr/bin/env bash
# Build a self-contained installer release bundle with precompiled artifacts.
# This is the single supported build/release pipeline for this repository.
# It performs a fresh clean build each run.
#
# Usage:
#   ./scripts/master-release-installer.sh [output-dir]
#
# Examples:
#   ./scripts/master-release-installer.sh
#   ./scripts/master-release-installer.sh ./dist

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This release script currently supports Linux only."
  exit 1
fi

OUT_DIR="${1:-$REPO_ROOT/dist}"
ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"

PACKAGE_NAME="agentaudio-master-installer-${OS}-${ARCH}-${GIT_SHA}-${STAMP}"
PACKAGE_DIR="$OUT_DIR/$PACKAGE_NAME"
PRECOMPILED_RELEASE_DIR="$PACKAGE_DIR/precompiled-target/release"

CARGO_CMD=(cargo)
if command -v rustup >/dev/null 2>&1 && cargo +stable --version >/dev/null 2>&1; then
  CARGO_CMD=(cargo +stable)
fi

if ! "${CARGO_CMD[@]}" metadata --format-version 1 --no-deps >/dev/null 2>&1; then
  echo "Cargo toolchain cannot parse this workspace."
  echo "Install a modern toolchain (Edition 2024 capable), then retry."
  echo "If rustup is installed, run: rustup toolchain install stable"
  exit 1
fi

echo "==> Using cargo command: ${CARGO_CMD[*]}"
echo "==> Cleaning previous build outputs..."
"${CARGO_CMD[@]}" clean

echo "==> Cleaning previous release bundles..."
mkdir -p "$OUT_DIR"
rm -rf "$OUT_DIR"/agentaudio-master-installer-*
rm -f "$OUT_DIR"/agentaudio-master-installer-*.tar.gz "$OUT_DIR"/agentaudio-master-installer-*.tar.gz.sha256

echo "==> Building release artifacts (fresh, no incremental cache)..."
export CARGO_INCREMENTAL=0
"${CARGO_CMD[@]}" build --locked --release --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
"${CARGO_CMD[@]}" build --locked --release \
  --bin agent-audio-scanner \
  --bin agentaudio-mcp-stdio \
  --bin agentaudio-mcp \
  --bin agentaudio-installer
"${CARGO_CMD[@]}" build --locked --release -p agentaudio-mcp-router --bin agentaudio-mcp-routerd

echo "==> Staging installer package..."
rm -rf "$PACKAGE_DIR"
mkdir -p "$PRECOMPILED_RELEASE_DIR"

copy_artifact() {
  local from="$1"
  local to="$2"
  if [[ ! -f "$from" ]]; then
    echo "Missing build artifact: $from" >&2
    exit 1
  fi
  cp "$from" "$to"
}

copy_artifact "target/release/agentaudio-installer" "$PACKAGE_DIR/agentaudio-installer"
copy_artifact "target/release/libagentaudio_wrapper_vst3.so" "$PRECOMPILED_RELEASE_DIR/libagentaudio_wrapper_vst3.so"
copy_artifact "target/release/agent-audio-scanner" "$PRECOMPILED_RELEASE_DIR/agent-audio-scanner"
copy_artifact "target/release/agentaudio-mcp-routerd" "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp-routerd"
copy_artifact "target/release/agentaudio-mcp-stdio" "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp-stdio"
copy_artifact "target/release/agentaudio-mcp" "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp"

chmod +x "$PACKAGE_DIR/agentaudio-installer"
chmod +x "$PRECOMPILED_RELEASE_DIR/agent-audio-scanner"
chmod +x "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp-routerd"
chmod +x "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp-stdio"
chmod +x "$PRECOMPILED_RELEASE_DIR/agentaudio-mcp"

cat > "$PACKAGE_DIR/run-installer.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/agentaudio-installer" "$@"
EOF
chmod +x "$PACKAGE_DIR/run-installer.sh"

cat > "$PACKAGE_DIR/README.txt" <<EOF
AgentAudio Master Installer Bundle
=================================

This package is for end users. No Rust toolchain is required.
The installer auto-detects ./precompiled-target and defaults to "Skip compile".
Built via a fresh clean build (cargo clean + non-incremental release build).

Contents:
- agentaudio-installer                 (GUI installer)
- run-installer.sh                     (launcher script)
- precompiled-target/release/
  - libagentaudio_wrapper_vst3.so
  - agent-audio-scanner
  - agentaudio-mcp-routerd
  - agentaudio-mcp-stdio
  - agentaudio-mcp

Quick start:
1. Unpack this directory on a Linux desktop machine.
2. Run: ./run-installer.sh
3. In the installer UI, keep defaults and click "Install Now".
4. Open your DAW, insert "AgentAudio Wrapper", load a plugin, then use MCP tools.

Expected installer behavior (no compile step):
- Place the VST3 wrapper in your chosen plugin directory.
- Install router/client binaries to ~/.local/bin.
- Enable/start agentaudio-mcp-routerd as a systemd --user service.
- Patch Claude/Gemini/Cursor MCP configurations.

Troubleshooting:
- If service setup fails, confirm systemd user services are available:
  systemctl --user status
- If your DAW cannot find the wrapper plugin, rescan VST3 directories.
- If MCP cannot connect, verify router URL is http://127.0.0.1:38765.
EOF

echo "==> Creating compressed archive..."
mkdir -p "$OUT_DIR"
TARBALL="$OUT_DIR/${PACKAGE_NAME}.tar.gz"
tar -C "$OUT_DIR" -czf "$TARBALL" "$PACKAGE_NAME"
sha256sum "$TARBALL" > "${TARBALL}.sha256"

echo ""
echo "Release bundle ready:"
echo "  Directory: $PACKAGE_DIR"
echo "  Archive:   $TARBALL"
echo "  SHA256:    ${TARBALL}.sha256"
