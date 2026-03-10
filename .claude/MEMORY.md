# Delta Project Memory

## Project Identity
- Delta: code hosting platform (VCS, CI/CD, artifact registry) for the AGNOS ecosystem
- Language: Rust (edition 2024)
- Repo: git@github.com:MacCracken/delta.git

## User Preferences
- Documents are the source of truth (not CLAUDE.md)
- No CLAUDE.md — use docs/, README, and project files for project knowledge
- Rust for all components
- Follow AGNOS ecosystem conventions (named subsystems, TOML config, structured logging)

## Conventions
- CHANGELOG.md tracks completed work
- docs/development/roadmap.md tracks work to be done
- Documents are source of truth (no CLAUDE.md)

## Key Paths
- Architecture: docs/architecture/overview.md
- Roadmap: docs/development/roadmap.md
- Contributing: docs/development/contributing.md
- Config example: config/delta.example.toml

## Build Commands
- `cargo build` — build all
- `cargo test --workspace` — run all tests
- `cargo clippy --workspace --all-targets -- -D warnings` — lint
- `cargo fmt --all` — format
- `cargo run --bin delta-api` — run server (port 8070)
