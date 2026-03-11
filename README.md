# Delta

A code hosting platform with version control, CI/CD pipelines, and artifact registry — built for the [AGNOS](https://github.com/agnostos/agnos) ecosystem.

Delta is designed to be private, fast, and natively accessible to both humans and AI agents.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      Delta Platform                       │
├──────────────┬──────────────┬──────────────┬─────────────┤
│  delta-api   │  delta-vcs   │  delta-ci    │ delta-web   │
│  REST API    │  Git hosting │  CI/CD       │ Web UI      │
│  server      │  & protocol  │  engine      │             │
├──────────────┴──────────────┴──────────────┴─────────────┤
│  delta-registry                                          │
│  Artifact & package storage                              │
├──────────────────────────────────────────────────────────┤
│  delta-core                                              │
│  Shared types, config, models                            │
└──────────────────────────────────────────────────────────┘
```

### Crates

| Crate | Description |
|-------|-------------|
| `delta-core` | Core types, configuration, error handling, models |
| `delta-vcs` | Git-compatible repository hosting and protocol |
| `delta-api` | REST API server (axum) |
| `delta-ci` | CI/CD pipeline engine with workflow definitions |
| `delta-registry` | Artifact and package registry (BLAKE3 content-addressed) |
| `delta-web` | Web frontend |

## Quick Start

```bash
# Build
cargo build

# Run the server
cargo run --bin delta-api -- --port 8070

# Run tests
cargo test --workspace

# Docker (development)
docker compose -f docker/docker-compose.yml --profile dev up --build

# Docker (production)
docker compose -f docker/docker-compose.yml --profile prod up --build
```

## Configuration

Delta uses TOML configuration. Default location: `/etc/delta/config.toml`

See `config/delta.example.toml` for all options.

```toml
[server]
host = "127.0.0.1"
port = 8070
api_prefix = "/api/v1"
# Restrict CORS origins in production (empty = allow any)
cors_origins = ["https://delta.example.com"]

[storage]
repos_dir = "/var/lib/delta/repos"
artifacts_dir = "/var/lib/delta/artifacts"
db_url = "sqlite:///var/lib/delta/delta.db"

[auth]
enabled = true
token_expiry_secs = 86400
# IMPORTANT: Change this in production — used to encrypt pipeline secrets
secrets_key = "your-strong-random-passphrase"
```

## API

Base URL: `http://localhost:8070/api/v1`

| Endpoint | Description |
|----------|-------------|
| `POST /auth/register` | Register a user |
| `POST /auth/login` | Login and get a token |
| `GET /auth/tokens` | List API tokens |
| `POST /auth/tokens` | Create an API token |
| `DELETE /auth/tokens/{id}` | Delete an API token |
| `GET /repos` | List repositories |
| `POST /repos` | Create a repository |
| `GET /repos/{owner}/{name}` | Get a repository |
| `GET /repos/{owner}/{name}/branches` | List branches |
| `GET /repos/{owner}/{name}/pulls` | List pull requests |
| `GET /repos/{owner}/{name}/pipelines` | List CI pipelines |
| `GET /repos/{owner}/{name}/artifacts` | List artifacts |
| `GET /audit` | View audit log |

Git smart HTTP transport: `/{owner}/{repo}.git/info/refs`

## Design Principles

- **Privacy-first** — self-hosted, no telemetry, user owns their data
- **AI-native** — agents are first-class users with structured APIs
- **Simple** — clean UX, sensible defaults, fewer concepts
- **Fast** — Rust backend, minimal overhead
- **Compatible** — standard git protocol, existing clients work
- **AGNOS-integrated** — native ark packages, daimon agents, sigil trust

## License

GPL-3.0
