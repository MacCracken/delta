#!/usr/bin/env bash
set -euo pipefail

echo "Building Delta..."
cargo build --release --workspace
echo "Build complete."
