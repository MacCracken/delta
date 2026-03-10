# Delta Development Roadmap

Delta is a code hosting platform providing version control, CI/CD, and artifact registry — built for the AGNOS ecosystem. Designed to be clean, private, and natively accessible to both humans and AI agents.

## Phase 1 — Foundation (Complete)

Core infrastructure, project scaffold, and basic repo hosting.

- [x] Project scaffold (Rust workspace, crate structure)
- [x] Core types and configuration (`delta-core`)
- [x] Database schema and migrations (SQLite)
- [x] User and authentication system (argon2 passwords, BLAKE3 tokens)
- [x] Agent identity and API key management (scoped tokens, is_agent flag)
- [x] Repository CRUD via REST API
- [x] Bare git repository initialization and storage
- [x] Health check and status endpoints
- [x] Configuration file loading (TOML)
- [x] Structured logging and tracing

## Phase 2 — Git Protocol (Complete)

Full git push/pull support over HTTP.

- [x] Smart HTTP transport (info/refs, upload-pack, receive-pack)
- [x] Ref advertisement and negotiation
- [x] Push authorization (Basic auth with API tokens)
- [x] Branch protection rules (pattern matching, PR requirement, force push prevention)
- [x] Webhook dispatch on push events (async delivery with recording)
- [x] Webhook SSRF protection (private IP rejection)
- [x] Branch and tag listing via API (gix)
- [x] Webhook CRUD API (create, list, delete per repo)
- [ ] Webhook HTTPS-only enforcement (config flag)
- [ ] Collaborator access control (push permissions beyond owner)
- [ ] SSH transport via built-in SSH server
- [ ] Large file support (LFS-compatible)
- [ ] Shallow clone and partial clone support
- [ ] Repository forking and mirroring

## Phase 3 — Code Review (Complete)

Pull/merge request workflow with review tooling.

- [x] Pull request model (create, update, merge, close, reopen)
- [x] PR numbering (auto-incrementing per repo)
- [x] Diff generation between branches (unified diff, stat, file list)
- [x] Commit listing between base and head
- [x] File-level inline comments (path, line, side)
- [x] General conversation comments
- [x] Review states (approve, request changes, comment)
- [x] Merge strategies (merge commit, squash, rebase) via git worktree
- [x] Status checks (create/update per commit, block merge on failure)
- [x] Status check visibility enforcement (private repo protection)
- [x] Audit log authorization (user-scoped access)
- [x] CORS middleware on API router
- [x] Branch protection enforcement on merge (required approvals + status checks)
- [ ] AI-assisted code review summaries
- [ ] Agent-authored PRs with provenance tracking
- [ ] Inline suggestions with one-click apply

## Phase 4 — CI/CD Engine

Workflow execution engine for build, test, and deploy pipelines.

- [ ] Workflow definition format (`.delta/workflows/*.toml`)
- [ ] Trigger system (push, PR, tag, schedule, manual)
- [ ] Job DAG scheduling with dependency resolution
- [ ] Sandboxed step execution (Landlock + seccomp on AGNOS)
- [ ] Container-based runners (fallback for non-AGNOS hosts)
- [ ] Log streaming and artifact upload from jobs
- [ ] Secret management (encrypted at rest, scoped per repo)
- [ ] Reusable workflow templates
- [ ] Matrix builds (multiple OS/arch/toolchain)
- [ ] Pipeline status badges

## Phase 5 — Artifact Registry

Package and release artifact storage with integrity verification.

- [ ] Content-addressable blob storage (BLAKE3)
- [ ] Release management (tags, changelogs, asset uploads)
- [ ] Generic artifact upload/download API
- [ ] `.ark` package registry (AGNOS native packages)
- [ ] Container image registry (OCI-compatible)
- [ ] Artifact retention policies and cleanup
- [ ] Signed artifacts with ed25519 verification
- [ ] Download statistics and audit trail

## Phase 6 — Web Interface

Browser-based UI for humans and structured views for agents.

- [ ] Repository browser (file tree, blame, history)
- [ ] Commit and diff viewer
- [ ] Pull request UI (conversation, diff, checks)
- [ ] CI/CD dashboard (pipeline list, log viewer)
- [ ] User/org profile pages
- [ ] Settings and administration panels
- [ ] Responsive design, keyboard navigation
- [ ] Dark/light theme support

## Phase 7 — AI-Native Features

First-class AI agent integration across the platform.

- [ ] Structured API responses optimized for LLM consumption
- [ ] Agent-scoped API tokens with fine-grained permissions
- [ ] Code search with semantic indexing
- [ ] AI-generated PR descriptions and commit summaries
- [ ] Automated issue triage and labeling
- [ ] Training data export (opt-in, per-repo, privacy-preserving)
- [ ] Agent activity dashboard and audit log
- [ ] Natural language query interface for repos

## Phase 8 — Federation and Privacy

Multi-instance federation and private deployment features.

- [ ] Instance-to-instance federation protocol
- [ ] Cross-instance forking and mirroring
- [ ] Private instance deployment (single binary, minimal config)
- [ ] End-to-end encrypted repositories
- [ ] Audit log export for compliance
- [ ] AGNOS integration (daimon agent registry, sigil trust chain)
- [ ] Air-gapped deployment support

## Phase 9 — Scale and Hardening

Production readiness, performance, and security hardening.

- [ ] Horizontal scaling (stateless API, shared storage)
- [ ] Repository sharding and replication
- [ ] Rate limiting and abuse prevention
- [ ] Security audit and penetration testing
- [ ] Performance benchmarks and optimization
- [ ] Backup and disaster recovery
- [ ] Monitoring and alerting integration
- [ ] Documentation and API reference
- [ ] Dependency vulnerability tracking (see [dependency-watch.md](dependency-watch.md))

---

## AGNOS Integration Checklist

Items Delta needs to complete for full AGNOS ecosystem integration.
These cut across phases and should be prioritized alongside phase work.

### Delta-Side (this repo)

- [x] Package manifest (`deploy/agnosticos/delta.pkg.toml`) — ark install metadata
- [x] Systemd service file with security hardening (`deploy/delta.service`)
- [x] System user creation hooks (pre/post install)
- [x] Capability declaration (code-hosting, git-http endpoints)
- [ ] **Takumi recipe** — `recipes/marketplace/delta.toml` in agnosticos repo for building Delta as an .ark package from source
- [ ] **MCP server** — expose Delta API as MCP tools (create-repo, list-repos, create-pr, search-code) for agnoshi shell integration
- [ ] **Hoosh provider** — optional LLM provider config for AI-powered code review (use local hoosh gateway at port 8088)
- [ ] **Daimon agent registration** — on startup, register with daimon at port 8090 (`/v1/agents/register`) with capabilities
- [ ] **Sigil trust** — sign artifacts and releases with ed25519 keys compatible with AGNOS sigil trust chain
- [ ] **Argonaut service target** — declare service dependencies (requires: postgres/sqlite, network; wanted-by: agnos-core)
- [ ] **Health endpoint for daimon** — `/health` already exists, ensure it returns JSON format expected by daimon heartbeat
- [ ] **Structured logging** — output JSON logs compatible with AGNOS journald integration
- [ ] **`.ark` registry support** — Phase 5 artifact registry should natively store/serve .ark packages

---

## Design Principles

1. **Privacy-first** — self-hosted by default, no telemetry, user owns their data
2. **AI-native** — agents are first-class users, not afterthoughts
3. **Simple** — fewer concepts, cleaner UX, sensible defaults over configuration
4. **Fast** — Rust backend, minimal overhead, responsive UI
5. **Compatible** — standard git protocol, works with existing git clients
6. **AGNOS-integrated** — native support for ark packages, daimon agents, sigil trust
