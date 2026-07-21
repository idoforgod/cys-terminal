# Browser Runtime local package smoke — 2026-07-22

Scope: local `aarch64-apple-darwin` staging at source revision
`c4c803af0e32b8e509a791fb1bda523e24b85588`. This is reproducible
pre-release evidence, not a notarized or promoted release artifact.

## Result

- `cys-browserd` built in the locked release profile with Rust 1.95.0.
- The pinned Bun 1.3.8 compiler, Chromium 131.0.6778.33 archive, and Chromium
  license passed the source manifest byte-count and SHA-256 checks.
- `runtime-stage.py` produced the expected arm64 supervisor, engine, Chromium,
  Playwright notice, and lock layout without linked files.
- Developer ID inside-out signing found and verified six Mach-O files. The
  engine signature authority was `Developer ID Application: yoonsik choi
  (Q43YA2NMF9)` with Team ID `Q43YA2NMF9` and hardened runtime enabled.
- The signed standalone engine started in strict-runtime mode, used the staged
  absolute Chromium path, opened a local HTTP page, and returned a snapshot
  beginning with the untrusted-web-content boundary.
- The same engine denied an `about:` navigation with `SCHEME_DENIED`, as the
  public navigation policy requires.
- Browser Runtime metadata unit tests passed 3/3.

## Verified input and signed-output digests

| Item | SHA-256 |
|---|---|
| pinned Bun compiler archive | `672a0a9a7b744d085a1d2219ca907e3e26f5579fca9e783a9510a4f98a36212f` |
| pinned Chromium archive | `ee37fe50d12019704f2b71091acea563519830a027df973ee58c03fd08103bc2` |
| pinned Chromium license | `368cca1106be99d39ecd32a38d8305585d802a475effb66380b91ffc9bcf709b` |
| signed supervisor | `e2bc425f8face4e21192ab62804d0e1d299ab86045a8bf7b689a6983053578e6` |
| signed engine | `23dbdb554039c80bcf285fd863810373547f3e7b166fa8625e4af80975cec3ed` |
| signed Chromium tree | `4b2ca223a9a40c20102e4e641c2aa60a1edc522e9f86f5a94fd1b8e99935e979` |

The stage lock initially contains pre-signing executable/tree hashes. This is
intentional: `browser-runtime-metadata.py prepare` recalculates them from the
signed target tree before deriving the final runtime identity and signing the
attestation and policy.

## Deliberate boundary

The workstation did not expose `minisign` or any Browser Runtime release trust
environment variables. No secret key was requested or materialized. Therefore
the final attestation/policy, notarization, stapling, DMG/update archives,
x86_64 macOS target, Windows Authenticode target, upload, and remote promotion
were not attempted. Those remain release gates and require the protected
release environment and owner-controlled credentials.
