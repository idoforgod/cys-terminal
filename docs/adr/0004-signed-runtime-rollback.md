---
status: accepted
---

# Roll back only to sealed, compatible runtime units

A production rollback must never replace files inside a signed macOS app bundle, reuse an arbitrary old browserd state, or fall back to external Bun/system Chrome. The preferred browser-only rollback is selection among immutable runtime units already sealed by the same signed application and admitted by its compatibility allowlist and security floor. Selection is journaled and atomic; health must pass before commit.

If the current signed app contains no qualified previous runtime, Browser enters a disabled-safe state while terminals, cysd, packs, and user profiles remain available, and recovery uses a previously notarized/signed full-app release through the authenticated updater. A feature flag may route the globe entry point to a qualified prior implementation, but it cannot weaken protocol, signing, self-contained-runtime, or external-content isolation requirements.

## Consequences

- Runtime manifest identity and rollback journal are part of release compatibility evidence.
- macOS notarization/stapling and Windows artifact integrity are tested after rollback as well as fresh install.
- Rollback preserves browser profile and terminal/organization state; destructive reset is not a recovery mechanism.
