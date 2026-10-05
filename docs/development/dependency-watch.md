# Dependency Watch

Tracking dependency advisories, upgrade blockers, and security items that require upstream fixes or design decisions before resolution.

Last reviewed: 2026-10-05

## Active Advisories

### RUSTSEC-2023-0071 — rsa timing sidechannel (Marvin Attack)

- **Crate:** `rsa` 0.10.0-rc (via `russh` 0.64 / `ssh-key` 0.7)
- **Severity:** Medium (5.9)
- **Issue:** Potential key recovery through timing sidechannels in RSA private-key operations
- **Impact:** None in practice — the SSH server only *verifies* RSA client signatures (public-key operations) and its host key is Ed25519, so no RSA private-key operation ever runs. (`sqlx` 0.9 no longer pulls `rsa` into the active dependency tree.)
- **Fix:** No upstream fix available.
- **Status:** No action needed; re-check when `russh`/`ssh-key` move to a patched `rsa`.
- **Tracking:** <https://rustsec.org/advisories/RUSTSEC-2023-0071>

## Pending Security Work

Items identified in security scan that require design decisions or larger implementation effort.

### ~~Secret encryption at rest~~ (Resolved 2026-03-10)

Implemented BLAKE3 stream cipher encryption in `delta-core/src/crypto.rs`. Key derived from `auth.secrets_key` config.

### Rate limiting on auth endpoints (Medium)

- **Issue:** No rate limiting on `/api/v1/auth/login` and `/api/v1/auth/register`
- **Required:** Choose rate limiting strategy (tower-governor, custom middleware)
- **Phase:** 9 (Scale and Hardening) — roadmap item "Rate limiting and abuse prevention"

### Collaborator access control (Medium)

- **File:** `crates/delta-api/src/routes/git.rs:186`
- **Issue:** Push access only checks repo owner, no collaborator support
- **Required:** Collaborator model, invitation system, permission levels
- **Phase:** Needs design — cross-cutting feature

### Webhook HTTPS enforcement (Low)

- **Issue:** Webhooks accept HTTP URLs, only HTTPS should be allowed in production
- **Required:** Config flag to control HTTP vs HTTPS-only webhook URLs
- **Phase:** Could be done anytime, needs config option

## CI/CD Notes

- `cargo audit` in `.github/workflows/ci.yml` runs with `continue-on-error: true` to prevent known advisories from blocking builds/releases. It emits a GitHub Actions warning instead.
- `cargo deny` and `cargo outdated` also run with `continue-on-error: true`.
- When all active advisories above are resolved, consider removing `continue-on-error` from `cargo audit` to enforce a clean audit gate.

## Resolved

### 2026-10-05 — RUSTSEC-2025-0140 (gix-date non-utf8 string)

- Resolved by upgrading `gix` 0.72 → 0.88, which depends on `gix-date` 0.17 (fixed in >= 0.12)

### 2026-03-10 — SSRF protection for webhooks

- Added private IP validation to webhook dispatch (git.rs)
- Blocks localhost, RFC1918, link-local, .local, .internal

### 2026-03-10 — Audit log authorization

- Users now restricted to viewing only their own audit logs (audit.rs)

### 2026-03-10 — CORS middleware

- Added CorsLayer to API router (routes.rs)

### 2026-03-10 — Status check visibility

- Private repo status checks now return 404 for unauthenticated requests (status_checks.rs)

### 2026-03-10 — reqwest native-tls to rustls-tls

- Switched to rustls-tls to fix aarch64 cross-compilation (no system OpenSSL needed)

### 2026-03-10 — HTTP client error handling

- Replaced `unwrap_or_default()` with proper error propagation in webhook dispatch
