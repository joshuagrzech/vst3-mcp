#!/usr/bin/env bash
# Bundle the agentaudio-wrapper-vst3 cargo build into a VST3 directory layout.
# Run from repo root. Usage:
#   ./scripts/bundle-vst3.sh [debug|release] [install-dir]
# Examples:
#   ./scripts/bundle-vst3.sh                    # release, print path only
#   ./scripts/bundle-vst3.sh release            # same
#   ./scripts/bundle-vst3.sh release ~/.vst3    # release + install to ~/.vst3
#   ./scripts/bundle-vst3.sh debug              # use target/debug build

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PROFILE="${1:-release}"
INSTALL_DIR="${2:-}"
BUNDLE_NAME="AgentAudio Wrapper.vst3"
ARCH="$(uname -m)-linux"

if [[ "$PROFILE" == "debug" ]]; then
  SO="target/debug/libagentaudio_wrapper_vst3.so"
else
  PROFILE=release
  SO="target/release/libagentaudio_wrapper_vst3.so"
fi

if [[ ! -f "$SO" ]]; then
  echo "Build not found: $SO"
  echo "Run: cargo build --$PROFILE --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml"
  exit 1
fi

# Build scanner binary for out-of-process plugin discovery (crash isolation)
SCANNER="target/$PROFILE/agent-audio-scanner"
if [[ ! -f "$SCANNER" ]]; then
  echo "Building scanner binary..."
  cargo build --"$PROFILE" --bin agent-audio-scanner
fi

rm -rf "$BUNDLE_NAME"
mkdir -p "$BUNDLE_NAME/Contents/$ARCH"
mkdir -p "$BUNDLE_NAME/Contents/Resources"
cp "$SO" "$BUNDLE_NAME/Contents/$ARCH/AgentAudio Wrapper.so"
cp "$SCANNER" "$BUNDLE_NAME/Contents/Resources/agent-audio-scanner"
chmod +x "$BUNDLE_NAME/Contents/Resources/agent-audio-scanner"
echo "Created $BUNDLE_NAME"

if [[ -n "$INSTALL_DIR" ]]; then
  mkdir -p "$INSTALL_DIR"
  cp -r "$BUNDLE_NAME" "$INSTALL_DIR/"
  echo "Installed to $INSTALL_DIR/$BUNDLE_NAME"
fi
