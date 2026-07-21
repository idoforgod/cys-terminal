---
status: accepted
---

# Let the lazy cysd authority broker own the self-contained Browser Runtime

Production Browser v2 ships a standalone browser engine, its pinned Playwright-compatible Chromium, licenses, and a runtime manifest inside the signed application. The sole lifecycle authority is the optional, lazy `cysd` `BrowserAuthorityExtension`. The extension owns compatibility evaluation and single-flight start; its private native supervisor owns the per-`EngineKey` process lock, engine process group, private endpoint state, and cleanup. Tauri, `cys`, and `javis_browser.py` are adapters only. They submit logical intent and never select an executable, mutate runtime state, hold the process lock, or spawn browserd/Chromium. User Bun, system Node packages, system Chrome, `PATH` discovery, and production fallback to the source TypeScript server are forbidden.

The shared engine key is exactly `shared-default/headless`. A GUI start requires a peer-verified `cys-app` process plus an exact `{window_label, pane_nonce, gesture_id}` user-gesture contract. Restored panes may request only a passive embed descriptor; they cannot call start. The broker returns an `EmbedDescriptor` URL after binding the private engine endpoint to runtime id, pid, port, instance and generation. It never exposes the long-lived control token as a DTO field.

Development may opt into source browserd only through an explicit development authority and never from a production fallback. Browser startup remains lazy: app boot and layout restore can inspect state or request an embed descriptor but cannot spawn; the globe button and explicit reconnect can request a bounded, single-flight start. The extension is initialized only when a `browser.runtime.*` RPC is dispatched. Missing/corrupt Browser assets therefore produce `DISABLED_SAFE` Browser errors and cannot block cysd construction, PTY service, onboarding, or organization bootstrap.

## Consequences

- GUI version, protocol version, browserd build, Chromium build, hashes, licenses, and security floor must be recorded in one manifest.
- Runtime failure is isolated to Browser and must never mutate global PATH or block cysd, PTY, onboarding, or organization boot.
- `src/bin/cysd/authority_broker` is the public lifecycle choke point; `browser-runtime/supervisor` is private and rejects direct launch without a parent-bound authenticated broker channel.
- `CYS_BROWSER_V2_ENGINE_ROOT`, `CYS_BROWSER_RUNTIME_ID`, and `CYS_BROWSER_STRICT_RUNTIME=1` bind the packaged engine endpoint to a supervisor-owned private directory and the signed runtime identity.
- This ADR supersedes its earlier Tauri-owner wording. Any comment, test, or release workflow that assigns process ownership to Tauri/Python is a release-blocking drift.
