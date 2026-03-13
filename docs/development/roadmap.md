# Delta Development Roadmap

Delta is a code hosting platform providing version control, CI/CD, and artifact registry — built for the AGNOS ecosystem.

Phases 1–8 and AGNOS integration are complete. Only remaining work is listed below.

## Phase 9 — Scale and Hardening

- [ ] Horizontal scaling (stateless API, shared storage)
- [ ] Rate limiting and abuse prevention
- [ ] Performance benchmarks and optimization
- [ ] Backup and disaster recovery
- [ ] Monitoring and alerting integration

## Future / Demand-Gated

Items below are not planned for any phase. They will be prioritized if there is sufficient user demand.

- [ ] Desktop app (Tauri webview shell wrapping the web UI)
- [ ] Email notifications for pipeline failures
- [ ] IDE extensions (VS Code, Zed)

---

## AGNOS Integration (Complete)

- [x] MCP server — expose Delta API as MCP tools for agnoshi shell
- [x] Hoosh provider — LLM gateway for AI-powered code review
- [x] Daimon agent registration on startup
- [x] Sigil trust — ed25519 artifact signing
- [x] Structured JSON logging for AGNOS journald
- [x] `.ark` registry support in artifact storage

Items below are AGNOS-side configuration (not Delta code):
- Takumi recipe for building Delta as an .ark package
- Argonaut service target and dependency declaration
