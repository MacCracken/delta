# Delta Architecture Overview

## Platform Components

Delta is a Rust workspace with six crates, each responsible for a distinct layer of the platform.

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

## Crate Responsibilities

### delta-core
Shared foundation — types, configuration, error handling, and data models used across all other crates.

- `DeltaConfig` — TOML-based configuration
- `DeltaError` / `Result` — unified error types via `thiserror`
- Models: `Repository`, `User`

### delta-vcs
Git-compatible version control backend. Manages bare repositories on disk and implements git transport protocols.

- `RepoHost` — bare repo lifecycle (init, delete, list)
- Smart HTTP transport (info/refs, upload-pack, receive-pack)
- SSH transport (planned)
- Uses `gix` (gitoxide) for git operations

### delta-api
HTTP API server built on `axum`. The primary interface for clients, agents, and the web UI.

- REST endpoints for all platform operations
- Binary entry point: `delta-api`
- Default port: 8070

### delta-ci
CI/CD pipeline engine. Parses workflow definitions and executes build/test/deploy pipelines.

- Workflow format: `.delta/workflows/*.toml`
- Triggers: push, PR, tag, schedule
- Sandboxed execution (Landlock + seccomp on AGNOS)

### delta-registry
Artifact and package storage with content-addressable integrity.

- BLAKE3 content hashing for deduplication
- Supports: binaries, archives, containers, `.ark` packages
- Release management with signed artifacts

### delta-web
Browser-based frontend for repository browsing, code review, CI dashboards, and administration.

## Tech Stack

| Component | Choice |
|-----------|--------|
| Language | Rust (edition 2024) |
| HTTP | axum |
| Database | SQLite (dev) / Postgres (prod) via sqlx |
| Git | gix (gitoxide) |
| Content hash | BLAKE3 |
| Serialization | serde, serde_json, toml |
| Logging | tracing + tracing-subscriber |
| CLI | clap |
| Errors | thiserror (libraries), anyhow (binaries) |
| Crypto | ed25519-dalek, sha2 |

## Port Assignments

| Service | Port |
|---------|------|
| Delta API | 8070 |

## AGNOS Integration

Delta is designed as part of the AGNOS ecosystem:

- **ark packages** — native `.ark` package registry support
- **daimon agents** — agent identity and API access as first-class users
- **sigil trust** — ed25519 artifact signing compatible with AGNOS trust chain
- **takumi recipes** — Delta itself is buildable as an ark package
