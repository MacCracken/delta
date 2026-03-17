# Delta Development Roadmap

Delta is a code hosting platform providing version control, CI/CD, and artifact registry — built for the AGNOS ecosystem.

All planned phases (1–9) and AGNOS integration are complete. Only demand-gated items remain.

## Future / Demand-Gated

Items below are not planned for any phase. They will be prioritized if there is sufficient user demand.

- [x] Self-hosted CI runners (offload jobs to external machines via a runner agent that polls for work)
- [ ] Desktop app (Tauri webview shell wrapping the web UI)
- [ ] Email notifications for pipeline failures
- [ ] IDE extensions (VS Code, Zed)
- [ ] Enforce token scopes in AuthUser extractor (scopes stored but not checked)

## Engineering Backlog

Low-priority improvements identified during security audit (2026.3.16):

- [x] Cap pipeline step log accumulation (local: 2MB cap, remote: 1MB cap)
- [x] OCI tag/reference name validation (1-128 chars, alphanumeric)
- [x] Password max-length validation (1024 char cap)
- [x] Deduplicate constant_time_eq (shared from delta_core::crypto)
- [x] Loopback SSRF: block full 127.0.0.0/8, [::], IPv4-mapped IPv6
- [x] MCP workspace handlers: verify workspace creator ownership
- [x] Step log errors: propagate instead of silently swallowing in complete_job
- [ ] Propagate RNG errors in crypto.rs instead of panicking
- [ ] CSV formula injection protection in audit export (escape =, +, -, @)
- [ ] Account registration limits / email verification
- [ ] Request body size limits on Bytes endpoints (git, LFS, OCI)
- [ ] Pagination on runner list endpoint
- [ ] Runner stale-job cleanup (re-queue claimed jobs after heartbeat timeout)
- [ ] Seccomp filter support for aarch64 (currently x86_64 only)
- [ ] Migrate legacy encrypted secrets (no MAC) → new format with MAC
- [ ] Empty queue_id fallback: pre-allocate queue ID before building payload
- [ ] Runner shared-token model: consider per-runner unique tokens for isolation
- [ ] Pipeline finalization race: atomic check-and-update to avoid dropped events
- [ ] Admin promotion/demotion API endpoint (currently requires direct DB access)
- [ ] Deduplicate parse_repo_name and basic auth parsing (git.rs, lfs.rs)
- [ ] CORS: restrict allow_headers to specific headers when origins are configured
- [ ] Workspace lock DashMap cleanup on workspace expiry

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
