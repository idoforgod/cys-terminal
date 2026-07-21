---
status: accepted
---

# Package browserd and Chromium as one self-contained runtime

Production Browser v2 will ship a standalone browserd executable, its pinned Playwright-compatible Chromium, licenses, and a runtime manifest inside the signed application. A Tauri `BrowserRuntimeManager` will be the sole production lifecycle owner and will resolve only verified absolute bundle paths; user Bun, system Node packages, system Chrome, `PATH` discovery, and production fallback to the source TypeScript server are forbidden. `javis_browser.py` remains a CLI compatibility adapter that calls the same manager contract instead of owning a second spawn path.

Development may opt into source browserd explicitly. Browser startup remains lazy: app boot and layout restore can inspect state but cannot spawn; the globe button and an explicit reconnect can request a bounded, single-flight start. This choice increases package size but makes clean-machine behavior, version compatibility, signing, and supportability properties of one release rather than properties of the user's development environment.

## Consequences

- GUI version, protocol version, browserd build, Chromium build, hashes, licenses, and security floor must be recorded in one manifest.
- Runtime failure is isolated to Browser and must never mutate global PATH or block cysd, PTY, onboarding, or organization boot.
- This ADR is implementation authority for a later packet; the current protocol packet does not modify Tauri runtime or release files.
