# Versioning

Delta follows the AGNOS calendar versioning (CalVer) convention.

## Format

- **Source version**: `YYYY.M.D` (e.g. `2026.3.10`)
- **Same-day patch**: `YYYY.M.D-N` (e.g. `2026.3.10-1`)
- **Build number**: `YYYYMMDDN` (e.g. `20260310`, `202603101`)

The source version is stored in `VERSION` (root) and `Cargo.toml` workspace version.
The build number is derived automatically during release — no parentheses or special
characters in filenames.

## Release process

Releases are triggered by pushing a git tag matching the version:

```bash
# Tag a release
git tag 2026.3.10
git push origin 2026.3.10

# Same-day patch
git tag 2026.3.10-1
git push origin 2026.3.10-1
```

Only tagged commits are released. The CI pipeline runs the full CI gate before
building release artifacts.

## Artifacts

Release archives are named: `delta-{version}-linux-{arch}.tar.gz`

Container images are tagged with both the source version and the build number:
- `ghcr.io/agnostos/delta:2026.3.10`
- `ghcr.io/agnostos/delta:20260310`
- `ghcr.io/agnostos/delta:latest`

## Updating the version

When bumping version, update both files:
1. `VERSION` — read by CI/CD
2. `Cargo.toml` — `[workspace.package] version`
