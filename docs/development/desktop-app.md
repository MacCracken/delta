# Desktop App (Tauri)

Demand-gated. Only implement if users request a native desktop experience.

## Approach

- **Tauri v2** — wraps the existing server-rendered web UI in a system webview
- No bundled Chromium — uses WebKit (macOS/Linux) and WebView2 (Windows)
- Binary size: ~3-5MB (vs ~150MB for Electron)
- The app connects to a running Delta server, same as the browser

## Architecture

```
delta-desktop/
  src-tauri/
    src/main.rs       # Tauri entry point, configures window + URL
    Cargo.toml        # Tauri deps
    tauri.conf.json   # Window size, title, permissions
  src/
    index.html        # Thin shell that redirects to configured server URL
```

## Key decisions

- **Thin client** — no embedded server, no offline mode. The app is a branded browser window pointed at the user's Delta instance.
- **Config** — on first launch, prompt for server URL (`https://delta.example.com`). Store in OS config dir.
- **Auth** — use the same token-based auth. Tauri can store tokens in the OS keychain via `tauri-plugin-store`.
- **Deep links** — register `delta://` protocol handler so CLI tools can open specific pages (`delta://owner/repo/-/pipelines/123`).

## Build targets

- Linux: AppImage, .deb
- macOS: .dmg
- Windows: .msi

All built via `tauri build` in CI.

## When to build

Only if there are concrete requests from users who need:
- System tray notifications for pipeline status
- OS-level keyboard shortcuts
- Deep link integration with other tools
- A window that stays separate from their browser tabs
