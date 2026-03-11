# Contributing to Delta

## Development Setup

```bash
# Clone
git clone git@github.com:MacCracken/delta.git
cd delta

# Build
cargo build

# Run tests
cargo test --workspace

# Run linter
cargo clippy --workspace --all-targets -- -D warnings

# Format
cargo fmt --all

# Run the server
cargo run --bin delta-api -- --port 8070
```

### Docker Development

The dev container mounts the workspace and uses `cargo-watch` for live reload:

```bash
docker compose -f docker/docker-compose.yml --profile dev up --build
```

This starts the server on `localhost:8070` with auto-rebuild on code changes.
The dev config is at `config/delta.dev.toml`.

## Conventions

### Code Style
- Use `tracing` for all logging (not `log` or `println!`)
- Error handling: `thiserror` for library errors, `anyhow` at binary boundaries
- Tests live alongside code in `#[cfg(test)]` modules
- Follow standard Rust naming: `snake_case` functions, `CamelCase` types

### Configuration
- All configuration is TOML-based
- Default config path: `/etc/delta/config.toml`
- Dev config: `config/delta.dev.toml` (used by Docker dev container)
- Example config: `config/delta.example.toml`
- Key production settings: `auth.secrets_key` (encrypt pipeline secrets), `server.cors_origins` (restrict CORS)

### Version Scheme
- AGNOS CalVer: `YYYY.M.D` (e.g. `2026.3.10`)
- Same-day patches: `YYYY.M.D-N` (e.g. `2026.3.10-1`)
- See [versioning.md](versioning.md) for details

### Commit Messages
- Use conventional commit style: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`
- Keep the first line under 72 characters

### Adding a New Crate
1. Create `crates/<name>/Cargo.toml` with `version.workspace = true`, `edition.workspace = true`
2. Add to workspace members in root `Cargo.toml`
3. Add workspace dependency entry if other crates will depend on it
