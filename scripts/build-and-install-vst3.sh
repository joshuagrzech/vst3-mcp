#!/usr/bin/env bash
# Single pipeline: build wrapper, bundle as VST3, optionally install.
# Run from repo root. Usage:
#   ./scripts/build-and-install-vst3.sh [release|debug] [install-dir]
# Examples:
#   ./scripts/build-and-install-vst3.sh              # build release + bundle only
#   ./scripts/build-and-install-vst3.sh release       # same
#   ./scripts/build-and-install-vst3.sh release ~/.vst3   # build + bundle + install to ~/.vst3

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PROFILE="${1:-release}"
INSTALL_DIR="${2:-}"
CRATE="crates/agentaudio-wrapper-vst3"

echo "Building wrapper ($PROFILE)..."
cargo build --"$PROFILE" --manifest-path "$CRATE/Cargo.toml"

echo "Bundling and installing..."
./scripts/bundle-vst3.sh "$PROFILE" $INSTALL_DIR

echo "Done. Insert 'AgentAudio Wrapper' in your DAW and open its editor to get the MCP endpoint."
