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

## Conventions

### Code Style
- Use `tracing` for all logging (not `log` or `println!`)
- Error handling: `thiserror` for library errors, `anyhow` at binary boundaries
- Tests live alongside code in `#[cfg(test)]` modules
- Follow standard Rust naming: `snake_case` functions, `CamelCase` types

### Configuration
- All configuration is TOML-based
- Default config path: `/etc/delta/config.toml`
- Example config: `config/delta.example.toml`

### Version Scheme
- Semver (`0.x.y`) during development
- AGNOS ecosystem version alignment after 1.0

### Commit Messages
- Use conventional commit style: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`
- Keep the first line under 72 characters

### Adding a New Crate
1. Create `crates/<name>/Cargo.toml` with `version.workspace = true`, `edition.workspace = true`
2. Add to workspace members in root `Cargo.toml`
3. Add workspace dependency entry if other crates will depend on it
