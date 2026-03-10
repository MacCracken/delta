#!/usr/bin/env bash
set -euo pipefail

echo "Setting up Delta development environment..."

# Install git (needed for smart HTTP protocol)
if ! command -v git &>/dev/null; then
    echo "Installing git..."
    if command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm git
    elif command -v apt-get &>/dev/null; then
        sudo apt-get install -y git
    fi
fi

# Install cargo-watch for development
if ! command -v cargo-watch &>/dev/null; then
    echo "Installing cargo-watch..."
    cargo install cargo-watch --locked
fi

# Create local data directories
mkdir -p /tmp/delta/{repos,artifacts}

echo "Development environment ready."
echo ""
echo "Run the server:"
echo "  cargo run --bin delta-api -- --config config/delta.example.toml"
echo ""
echo "Or with auto-reload:"
echo "  cargo watch -x 'run --bin delta-api -- --config config/delta.example.toml'"
