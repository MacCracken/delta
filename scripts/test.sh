#!/usr/bin/env bash
set -euo pipefail

echo "Running Delta tests..."
cargo test --workspace
echo "All tests passed."
