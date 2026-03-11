# Changelog

All notable changes to Delta are documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows [AGNOS CalVer](docs/development/versioning.md): `YYYY.M.D`.

## Unreleased

## 2026.3.10-1

### Added
- API token listing endpoint (`GET /api/v1/auth/tokens`)
- API token deletion endpoint (`DELETE /api/v1/auth/tokens/{id}`)
- Audit logging wired to key operations (register, login, repo create/delete)
- Configurable CORS origins via `server.cors_origins` config field
- Startup warnings for insecure defaults (secrets_key, CORS allow-any)
- Secret creation now returns a JSON response body (was 204 No Content)
- Docker dev container with cargo-watch live reload (`--profile dev`)
- Dev config file (`config/delta.dev.toml`)

### Fixed
- SQLite database auto-creation (`create_if_missing`) — no longer requires pre-existing DB file
- Git smart HTTP routes panic — Axum single-param-per-segment constraint
- CI workflow: cross-compiled binary artifact path missing `/release/` directory
- Release workflow: replaced unsafe `sed -E /e` shell execution with portable bash
- Release workflow: changelog generation on first-ever tag
- Release workflow: SBOM file collection from per-crate output
- Docker dev container: AGNOS entrypoint `ulimit -v 2097152` causing rustup OOM

### Changed
- Docker compose: production service now requires `--profile prod`
- Docker dev: set `AGNOS_ULIMIT_VMEM=unlimited` for next AGNOS version compatibility
- Docker dev: `RUSTUP_UPDATE_MODE=no-self-update` to avoid toolchain re-download on restart

## 2026.3.10

### Added

#### Phase 1 — Foundation
- Project scaffold with Rust workspace (6 crates: delta-core, delta-api, delta-vcs, delta-ci, delta-registry, delta-web)
- Core types and configuration loading (TOML)
- Database schema and migrations (SQLite via sqlx)
- User and authentication system (Argon2 passwords, BLAKE3 tokens)
- Agent identity and API key management (scoped tokens, `is_agent` flag)
- Repository CRUD via REST API
- Bare git repository initialization and storage (gix)
- Health check and status endpoints
- Structured logging and tracing

#### Phase 2 — Git Protocol
- Smart HTTP transport (info/refs, upload-pack, receive-pack)
- Ref advertisement and negotiation
- Push authorization (Basic auth with API tokens)
- Branch protection rules (pattern matching, PR requirement, force push prevention)
- Webhook dispatch on push events (async delivery with recording)
- Webhook SSRF protection (private IP/IPv6 rejection)
- Branch and tag listing via API
- Webhook CRUD API (create, list, delete per repo)

#### Phase 3 — Code Review
- Pull request model (create, update, merge, close, reopen)
- PR numbering (auto-incrementing per repo)
- Diff generation between branches (unified diff, stat, file list)
- Commit listing between base and head
- File-level inline comments (path, line, side)
- General conversation comments
- Review states (approve, request changes, comment)
- Merge strategies (merge commit, squash, rebase) via git worktree
- Status checks (create/update per commit, block merge on failure)
- Status check visibility enforcement (private repo protection)
- Audit log with user-scoped access
- CORS middleware on API router
- Branch protection enforcement on merge (required approvals + status checks)

#### Phase 4 — CI/CD Engine
- Workflow definition format (`.delta/workflows/*.toml`)
- Trigger system (push, PR, tag, schedule, manual)
- Job DAG scheduling with dependency resolution
- Pipeline runner with end-to-end orchestration
- Secret management (encrypted at rest via BLAKE3 stream cipher, scoped per repo)
- Push event to pipeline trigger integration (automatic on git push)
- Step log capture and storage
- CI step timeout enforcement

#### Phase 5 — Artifact Registry (partial)
- Content-addressable blob storage (BLAKE3 hashing, dedup)
- Release management (tags, changelogs, draft/prerelease flags)
- Generic artifact upload/download API (100 MB limit, hash verification)

### Security
- Input validation across all user-facing endpoints (usernames, PR titles/bodies, comments, secret names, release tags, branch names)
- Error response sanitization — generic messages instead of leaking internal details (~80 instances fixed)
- SSRF protection for webhooks (IPv4/IPv6 private ranges, link-local, mDNS, .internal)
- Artifact upload size limits (100 MB)
- Git ref and name validation (prevents option injection, path traversal)
- Webhook secret HMAC signatures (BLAKE3-keyed)
- Silent error logging replaced with `tracing::error!` throughout pipeline runner

### Infrastructure
- GitHub Actions CI/CD pipeline (clippy, tests, coverage)
- Release automation with CalVer tagging (no `v` prefix)
- Test coverage at 62.74% (target: 50%+)
