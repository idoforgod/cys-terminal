# NOTICE — Third-Party Attributions

cys-terminal is licensed under the MIT License (see `LICENSE`).
This file consolidates third-party attributions for discoverability.

## Vendored code

| Component | Path | Upstream | License |
|---|---|---|---|
| portable-pty (patched) | `vendor/portable-pty/` | wezterm (Wez Furlong) | MIT — original copyright preserved in `vendor/portable-pty/LICENSE.md` |
| vt100 0.15.2 (patched) | `vendor/vt100/` | doy/vt100-rust (Jesse Luehrs) | MIT — original copyright preserved in `vendor/vt100/LICENSE` |
| insane-search | `cysjavis-pack/skills/insane-search/` | fivetaku/insane-search | MIT — see `cysjavis-pack/skills/THIRD_PARTY.md` |
| skill collections (32 skills) | `cysjavis-pack/skills/` | NomaDamas/k-skill · obra/superpowers · mattpocock/skills | MIT — commit-pinned attributions in `cysjavis-pack/skills/THIRD_PARTY.md` |

## Design-only references (no code vendored)

Voicebox (MIT) and TimesFM (Apache-2.0) informed designs; no code was copied.
Clean-room reimplementation notes are embedded at the referencing sites
(`cysjavis-pack/bin/javis_*.py` headers) and in `cysjavis-pack/skills/THIRD_PARTY.md`.

## Dependencies

Rust crate and npm dependencies are declared in `Cargo.toml` / `src-tauri/Cargo.toml` /
`ui/package.json`; direct dependencies are MIT or MIT/Apache-2.0 dual-licensed.
SQLite (bundled via rusqlite) is public domain.
