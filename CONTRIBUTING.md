# Contributing

Thanks for your interest in cys-terminal.

## Ground rules

- **Before large PRs, open an issue first** — the project has strong conventions
  (deterministic gates, fail-closed guards, Korean-first docs) and we want to
  align direction before you invest time.
- Match the existing style of the file you touch (comment density, naming, 한국어 주석 유지).
- Every changed line should be traceable to the issue/PR intent (surgical diffs).

## Checks that must pass

```bash
cargo test --bin cysd            # daemon unit tests
cargo check -p cys-app           # desktop app
bash ui/build.sh                 # UI bundle
bash scripts/secret-scan.sh --all  # secret/PII gate (fail-closed)
sh scripts/version-check.sh      # version SOT consistency (release PRs only)
```

## Directive edits

- **`cysjavis-pack/directives/CEO_TEMPLATE.md` is a generated file — never edit it
  by hand.** It is synthesized from the non-shipped fragment
  `scripts/ceo_template_header.md` + a separator + the byte-identical
  `MASTER_DIRECTIVE.md` body. Whenever you edit the MASTER body (or the fragment),
  you **must** re-run `python3 scripts/gen_ceo_template.py` — otherwise the drift
  gate `python3 scripts/gen_ceo_template.py --check` goes red (exit 1) in CI.
- Never write the guarded lifecycle verbs
  (`launch`|`allocate`|`create`|`down`|`down-sock`|`rotate`|`reap`|`promote-ceo`)
  in **call form** — i.e. `cys-dept <verb>`, backticked or not — inside directives.
  The single-owner guard rejects those calls from the CEO (exit 7), so a directive
  instructing them would contradict its own enforcement; both the H-DOC-3 health
  specimen and `gen_ceo_template.py --check` fail on it. Prose mentions of a bare
  verb (`launch`) are fine — just don't prefix it with `cys-dept `.
- After resynthesis, run the part-cap pin locally:
  `cargo test --bin cysd deployed_ceo` (CEO synthesized payload must fit the
  delivery part cap with 4x headroom).

## Test isolation — pack sandbox (W0)

Tests must never touch the live pack at `~/.cys/pack`. `cargo test`/`cargo run`
that exercise pack install/update code would otherwise write to the live pack and
can corrupt it. Three structural seals enforce this — you normally do nothing:

- **`.cargo/config.toml [env]`** injects `CYS_PACK_DIR=target/test-pack-sandbox`
  into every `cargo test`/`cargo run`, so even a bare `cargo test` writes to a
  repo-local sandbox, not `~/.cys/pack`. Set your own `CYS_PACK_DIR=$(mktemp -d)`
  to override it per run (honored — `force = false`).
- **fail-closed `pack_dir()`** — in test builds, if no `CYS_PACK_DIR` (or legacy
  `JAVIS_/AITERM_`) is set, `pack_dir()` panics instead of falling back to the live
  path. Tests that manipulate these env vars must use the `EnvGuard` RAII helper
  (restores the previous value on drop, incl. on panic) rather than bare
  `set_var`/`remove_var`, so no "env-is-empty" window is left for a sibling test.
- **positive write authorization** — pack write paths hard-refuse (`Err`) to write
  the live default path unless given a `PackWriteAuth` token, granted only by the
  production entry points (`cys init-pack`, pack-update/downgrade, cysd boot).

Release binaries are unaffected: the sandbox env only exists under cargo, so a
shipped `cys` still resolves `~/.cys/pack` normally.

## Licensing

By contributing you agree your contributions are licensed under the MIT License.
Third-party code must be MIT/Apache-2.0-compatible and attributed in `NOTICE.md`
(and `cysjavis-pack/skills/THIRD_PARTY.md` for pack skills).
