# Changelog

All notable changes to Delta are documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows [AGNOS CalVer](docs/development/versioning.md): `YYYY.M.D`.

## Unreleased

## 2026.3.13

### Added

#### Phase 9 — Scale and Hardening (Complete)
- Horizontal scaling support
  - Configurable database connection pool size (`scaling.db_pool_size`)
  - Configurable request timeout (`scaling.request_timeout_secs`)
  - `init_pool_sized()` for tunable SQLite/Postgres pool initialization
- Rate limiting and abuse prevention
  - In-memory token-bucket rate limiter keyed by IP address (DashMap-based)
  - Configurable limits: `rate_limit.requests_per_window`, `rate_limit.window_secs`
  - Separate auth endpoint limiter (`rate_limit.auth_requests_per_window`, default 10/min)
  - Background cleanup task for expired rate limit entries
- Performance and observability
  - Request metrics tracking (status code counts, total duration, request count)
  - `GET /health/metrics` — uptime, avg latency, status code distribution
  - `TraceLayer` for HTTP request/response tracing
  - `CompressionLayer` (gzip) for response compression
- Backup and disaster recovery
  - `GET /api/v1/backup/status` — repo/artifact counts, DB size, storage paths
  - `POST /api/v1/backup/snapshot` — consistent SQLite snapshot via `VACUUM INTO`
- Monitoring and alerting integration
  - `GET /health/ready` — readiness probe checking DB connectivity and storage availability
  - Enhanced health check with version info
  - Structured JSON logging (`--json-log`) for log aggregation

#### Phase 8 — Federation and Privacy (Complete)
- Instance-to-instance federation protocol with instance discovery, trust management, and public key exchange
  - `GET /api/v1/federation/info` — public endpoint for remote instance discovery
  - `POST/GET /api/v1/federation/instances` — register and list federated instances
  - `POST /api/v1/federation/instances/{id}/trust` — update trust status
  - `GET /api/v1/federation/instances/{id}/repos` — browse remote instance repos
- Cross-instance forking and mirroring
  - `POST /api/v1/federation/mirror` — create local mirror of a remote repo via `git clone --mirror`
  - New repo fields: `is_mirror`, `mirror_url`, `federation_instance_id`
  - `create_mirror()` DB function for federated repo tracking
- Private instance deployment (single binary, minimal config)
  - `--private` CLI flag: enables auth, disables federation, conservative defaults
  - `--data-dir` CLI flag: sets all storage paths to subdirectories of one directory
  - Auto-creation of storage directories on startup
  - SQLite `?mode=rwc` default for auto-creating database files
  - Example config: `config/delta.private.toml`
- End-to-end encrypted repositories
  - Per-repo encryption keys wrapped with user-derived keys
  - `repo_encryption_keys` table with per-user key wrapping
  - `encrypted` flag on repositories
  - `generate_repo_key()`, `wrap_repo_key()`, `unwrap_repo_key()` in crypto module
  - Database migration `012_encryption.sql`
- Audit log export for compliance
  - `GET /api/v1/audit/export` — export with date range filters (`since`/`until`), resource type filter
  - JSON and CSV output formats
  - BLAKE3 integrity hash on JSON exports for tamper detection
  - Pagination support with total count
  - `list_for_export()` and `count_for_export()` DB functions
- Database migration `011_federation.sql` — mirror and federation fields on repositories
- `FederationConfig` section in config (`[federation]` with `enabled`, `instance_url`, `instance_name`, `timeout_secs`)

#### Phase 6 — Web Interface (Complete)
- Repository file browser with branch selector, breadcrumb navigation, file/folder icons
- Code viewer with line numbers, anchor links, raw download, and blame view
- Blame view with grouped commit annotations and author display
- Commit log page with paginated history per branch
- Commit detail page with diff stats, unified diff rendering (colored add/del/context lines)
- Pull request list page with Open/Closed/All filter tabs and state badges
- Pull request detail page with Conversation/Diff/Checks tabs, comment form, merge/close buttons
- User profile page with avatar initial, bot badge, and repository list
- Repository settings page: general settings, collaborators, branch protection rules, danger zone
- 13 new route handlers in `web.rs` serving all UI pages
- Custom `render_diff_html()` parser for unified diff to HTML tables
- Askama templates with responsive CSS (mobile-friendly)

#### Phase 3 — Code Review AI (Complete)
- AI-assisted code review summaries via `/ai/review/{number}` endpoint
- Agent-authored PRs with `is_agent_authored` provenance tracking on PR responses
- Inline code suggestions with old/new code and explanations

#### Phase 7 — AI-Native Features (Complete)
- Structured API responses optimized for LLM consumption (`/structured`, `/structured/tree`, `/structured/pulls`)
- Agent-scoped API tokens with 13 fine-grained permission scopes (`ScopeSet` with wildcard, write-implies-read, admin-implies-all)
- Full-text code search with FTS5 and porter stemming (`/search`, `/search/index`)
- AI-generated PR descriptions (`/ai/describe-pr`) and commit summaries (`/ai/summarize-commit/{sha}`)
- Natural language query interface for repos (`/ai/query`)
- Database migration `010_search.sql` — FTS5 virtual table for code search

#### AGNOS Integration
- **Hoosh provider** — added Hoosh (AGNOS LLM gateway) as AI provider option; connects to local gateway on port 8088 with OpenAI-compatible API; API key optional for local use
- **Daimon registration** — Delta registers 6 capabilities (code-hosting, pull-requests, ci-cd, artifact-registry, code-search, ai-code-review) with the Daimon agent runtime on startup; registration failures are non-fatal
- **Sigil artifact signing** — added `sign_content()` for ed25519 artifact signing (previously verification only); `sigil_trust_level()` maps verification status to Sigil trust levels (system_core/verified/unverified)
- **MCP server** — 9 MCP tools exposed at `/v1/mcp/tools` for agnoshi shell integration: `delta_list_repos`, `delta_get_repo`, `delta_list_branches`, `delta_list_pulls`, `delta_get_pull`, `delta_list_pipelines`, `delta_search_code`, `delta_read_file`, `delta_list_tree`
- **Structured JSON logging** — `--json-log` CLI flag switches tracing output to JSON format compatible with AGNOS journald
- **AGNOS config section** — `[agnos]` config with `enabled` and `daimon_url` fields

### Changed
- Version bumped to 2026.3.13
- `AiConfig` now supports `endpoint` field for custom LLM gateway URLs
- `AiProvider` enum expanded: `Anthropic`, `OpenAI`, `Hoosh`
- Roadmap updated: Phases 1–7 and AGNOS integration marked complete

## 2026.3.11

### Added

#### Phase 5 — Artifact Registry (Complete)
- `.ark` package registry — publish, download, search, list versions for AGNOS native packages (`/api/v1/registry/ark/`)
- OCI container image registry — OCI Distribution Spec endpoints for push/pull images (`/v2/`)
  - Blob upload (monolithic and chunked), manifest push/pull, tag management
  - sha256 digest verification, content-addressable storage via BlobStore
- Artifact retention policies — per-repo configurable rules (max age, max count, max total size)
  - Cleanup endpoint (`PUT /{owner}/{name}/artifacts/cleanup`) with global config fallback
  - `[registry]` config section for global defaults
- Signed artifacts with ed25519 verification
  - User signing key management (`/api/v1/auth/signing-keys`)
  - Artifact signature upload and verification (`/artifacts/{id}/signatures`, `/artifacts/{id}/verify`)
  - Signatures verified against content hash before storage
- Download statistics and audit trail
  - Per-download event tracking with user-agent and IP logging
  - Daily download count aggregation endpoint (`/artifacts/{id}/stats`)
  - Audit log entries on artifact download
- Database migration `005_registry.sql` — tables for retention policies, download events, signing keys, artifact signatures, ark packages, OCI manifests/tags/blobs/uploads
- Dedicated Docker config file (`config/delta.docker.toml`) — binds `0.0.0.0`, SQLite under `/var/lib/delta`

### Changed
- Dockerfile: pinned `delta` user/group to UID/GID 1003 for consistent volume permissions
- Dockerfile: copies `delta.docker.toml` instead of `delta.example.toml`
- Documentation updates (README, architecture overview, contributing guide, roadmap)

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
