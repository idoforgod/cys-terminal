---
status: accepted
---

# Use an exact, generation-bound cast embed protocol

Cast HTML is served from a tokenized loopback origin while its parent is a Tauri application origin. We will emit `frame-ancestors` only on the cast HTML route and only for the exact origin selected for the current platform/build: `tauri://localhost` on macOS/Linux, `http://tauri.localhost` on Windows, or one explicitly configured loopback development origin. RPC, state, and audit routes do not inherit this policy.

Every live iframe URL carries protocol version, positive embed generation, exact parent origin, and a 256-bit one-time embed ticket. Child-to-parent messages carry the same version/generation/ticket and use that parent as `targetOrigin`; the parent requires the actual iframe source, the exact current iframe origin, the supported version, the current generation, and the exact ticket. Browserd issues the ticket on the token-protected app GET, binds it to the process runtime identity/context/generation/origin with a 15-second TTL, and consumes it exactly once at WS upgrade. Wildcard `postMessage` and wildcard/self-only `frame-ancestors` are not permitted.

## Consequences

- A packaged-origin change must update the platform policy and packaged-app tests together.
- Development embedding is fail-closed unless its single exact loopback origin is configured.
- `load` remains insufficient as readiness evidence; the cast child reports `SHELL_READY` only after authenticated WS open and `FRAME_READY` only after a valid canvas draw crosses paint boundaries.
