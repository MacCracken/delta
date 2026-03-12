# Delta Development Roadmap

Delta is a code hosting platform providing version control, CI/CD, and artifact registry — built for the AGNOS ecosystem.

Phases 1–5 are complete. Only remaining work is listed below.

## Phase 2 — Git Protocol (Remaining)

- [ ] Collaborator access control (push permissions beyond owner)
- [ ] SSH transport via built-in SSH server
- [ ] Large file support (LFS-compatible)
- [ ] Shallow clone and partial clone support
- [ ] Repository forking and mirroring
- [ ] Webhook HTTPS-only enforcement (config flag)

## Phase 3 — Code Review (Remaining)

- [ ] AI-assisted code review summaries
- [ ] Agent-authored PRs with provenance tracking
- [ ] Inline suggestions with one-click apply

## Phase 4 — CI/CD Engine (Remaining)

- [ ] Sandboxed step execution (Landlock + seccomp on AGNOS)
- [ ] Container-based runners (fallback for non-AGNOS hosts)
- [ ] Log streaming (real-time)
- [ ] Reusable workflow templates
- [ ] Matrix builds (multiple OS/arch/toolchain)
- [ ] Pipeline status badges

## Phase 5 — Artifact Registry (Complete)

- [x] `.ark` package registry (AGNOS native packages)
- [x] Container image registry (OCI-compatible)
- [x] Artifact retention policies and cleanup
- [x] Signed artifacts with ed25519 verification
- [x] Download statistics and audit trail

## Phase 6 — Web Interface

- [ ] Repository browser (file tree, blame, history)
- [ ] Commit and diff viewer
- [ ] Pull request UI (conversation, diff, checks)
- [ ] CI/CD dashboard (pipeline list, log viewer)
- [ ] User/org profile pages
- [ ] Settings and administration panels

## Phase 7 — AI-Native Features

- [ ] Structured API responses optimized for LLM consumption
- [ ] Agent-scoped API tokens with fine-grained permissions
- [ ] Code search with semantic indexing
- [ ] AI-generated PR descriptions and commit summaries
- [ ] Natural language query interface for repos

## Phase 8 — Federation and Privacy

- [ ] Instance-to-instance federation protocol
- [ ] Cross-instance forking and mirroring
- [ ] Private instance deployment (single binary, minimal config)
- [ ] End-to-end encrypted repositories
- [ ] Audit log export for compliance
- [ ] AGNOS integration (daimon agent registry, sigil trust chain)

## Phase 9 — Scale and Hardening

- [ ] Horizontal scaling (stateless API, shared storage)
- [ ] Rate limiting and abuse prevention
- [ ] Performance benchmarks and optimization
- [ ] Backup and disaster recovery
- [ ] Monitoring and alerting integration

---

## AGNOS Integration

- [ ] Takumi recipe for building Delta as an .ark package
- [ ] MCP server — expose Delta API as MCP tools for agnoshi shell
- [ ] Hoosh provider — LLM gateway for AI-powered code review
- [ ] Daimon agent registration on startup
- [ ] Sigil trust — ed25519 artifact signing
- [ ] Argonaut service target and dependency declaration
- [ ] Structured JSON logging for AGNOS journald
- [ ] `.ark` registry support in artifact storage
