---
status: accepted
---

# Enforce the HTTP/HTTPS navigation boundary inside browserd

The address bar is not the security boundary. Browserd will enforce the product's HTTP/HTTPS-only rule for every top-level navigation path, including initial open/goto, address entry, redirects, history/reload outcomes, popup or `window.open` adoption, external protocols, and navigation-triggered downloads. Rejected navigation fails closed with a typed error while observation/logging failures remain fail-open for the rest of the application.

The existing v0.13.1 helper intentionally permits `data:` and `about:blank`; that is a baseline implementation detail, not the Browser v2 product policy. Internal blank-page creation must use a private engine-only operation rather than widening the public navigation allowlist. External websites are never loaded directly into a Tauri iframe, and `file:`, privileged schemes, local application protocols, and unsupported external handlers remain denied.

## Consequences

- All navigation entry points must converge on one browserd policy module; UI-only validation is defense in depth.
- Redirect, popup, download, and external-protocol tests are release-blocking, not optional address-bar tests.
- Private-network policy requires a separate explicit product decision; it must not be silently inferred while implementing this ADR.
