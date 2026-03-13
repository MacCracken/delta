# Delta Development Roadmap

Delta is a code hosting platform providing version control, CI/CD, and artifact registry — built for the AGNOS ecosystem.

All planned phases (1–9) and AGNOS integration are complete. Only demand-gated items remain.

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
