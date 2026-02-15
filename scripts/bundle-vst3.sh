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
ARCH="x86_64-linux"
CRATE="crates/agentaudio-wrapper-vst3"

if [[ "$PROFILE" == "debug" ]]; then
  SO="$CRATE/target/debug/libagentaudio_wrapper_vst3.so"
else
  PROFILE=release
  SO="$CRATE/target/release/libagentaudio_wrapper_vst3.so"
fi

if [[ ! -f "$SO" ]]; then
  echo "Build not found: $SO"
  echo "Run: cargo build --$PROFILE --manifest-path $CRATE/Cargo.toml"
  exit 1
fi

rm -rf "$BUNDLE_NAME"
mkdir -p "$BUNDLE_NAME/Contents/$ARCH"
cp "$SO" "$BUNDLE_NAME/Contents/$ARCH/AgentAudio Wrapper.so"
echo "Created $BUNDLE_NAME"

if [[ -n "$INSTALL_DIR" ]]; then
  mkdir -p "$INSTALL_DIR"
  cp -r "$BUNDLE_NAME" "$INSTALL_DIR/"
  echo "Installed to $INSTALL_DIR/$BUNDLE_NAME"
fi
