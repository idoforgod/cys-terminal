# cys 0.14.56 — release-notes draft

**DRAFT — not a release-completion statement.** Scope below is the local integration at `f6429140`, plus pre-tag preparation. P1 integration and final-candidate native CI are **PENDING**. The presence of a change in this tree does not certify a published installer. [S0]

## Windows Smart App Control and code signing

<!-- cys-win-sac: unsigned -->
**Windows 스마트 앱 컨트롤(SAC):** 이 판의 cys 설치 파일과 cys 실행파일은 코드 서명이 없습니다. 스마트 앱 컨트롤이 켜진 PC에서는 실행이 막힐 수 있으며, 이 판은 스마트 앱 컨트롤 적용 PC에서 검증하지 않았습니다.
**Windows Smart App Control:** this release's cys installer and cys executables are not code-signed. They may be blocked on PCs with Smart App Control on, and this release was not verified on a Smart App Control PC.

The candidate workflow permits only unsigned Windows builds. This notice describes that build policy; checking the actual installer and every shipped Windows executable/DLL remains **PENDING** until those assets exist. The release verifier must compare the asset signature state with this text in both the release body and `latest.json` notes. [S1]

## Changes present in the preparation tree

- **First department ticket:** manifest application queues the first ticket without holding the caller in a delivery wait. Status reports delivery separately from queue acceptance. Reapplying a task does not resend a pending or delivered first ticket; unreadable evidence produces an unknown state. Follow-up changes handle queue digests and ledger rotation. A daemon receipt proves delivery, not that the receiving agent has read the ticket. A real department end-to-end run and Windows runtime validation are not claimed here. [S2]
- **Event reporting:** a failed event-spool append now returns a nonzero status. Phoenix's old `_emit_evt` path and its nine calls were removed; the later restoration work supplies its own result handling. [S3]
- **Task addressing:** the optional `task-prompt --surface` form checks the addressed surface and tells its recipient to resolve its own TODO. The no-flag form preserves the earlier output and behaviour, including its known limitation for numbered/fresh workers. Missing or cross-daemon identity is stated rather than guessed. [S4]
- **Report timestamps and deadlock diagnostics:** timestamp parsing honours UTC offsets. Task writers publish their roots and lane stamps; deadlock evaluation records per-root evidence in shadow mode. Missing evidence is not reported as an empty task set. This is **not a claim that deadlock paging is enabled**. Lexically recognized cloud-managed roots are not read by this path; symlink aliases and redirected Windows cloud folders remain limitations. [S5]
- **Socket framing and attach cleanup:** a nonblank request ending at EOF without its terminating newline is refused instead of executed. A stalled attach can be released after its surface exits or leaves the daemon map; the change is designed to preserve live-surface streams and a reading client's final output. Windows named-pipe/ConPTY behaviour remains subject to native checks. [S6]
- **Event-spool transport:** producers and the bridge use one pack-relative spool-path definition. The bridge reads lanes separately, preserves rotated history, and reports failures to the owning lane's master. Returning files are not reread as new input. Rotation waits during a detected pack-install window; a spool over 8 MiB can rotate inside the startup grace once a poll is drained. This is not a hard bound on storage: repeated installs, an unreadable parent, a bridge dying before a drained poll, or a continuously busy spool can still prevent rotation. [S7]
- **Owned child processes and hook startup:** the C-03 work restores owned child-lifecycle handling and bounded skillscan pool stopping. Follow-up changes end the cooperative grace when the leader is gone, resolve hook paths with either slash form, relay phase bytes unchanged, and separate managed Windows input from the private lifeline. **C-03 native results at the final candidate are PENDING**; cross-typechecking cannot validate Job Objects, process teardown or Windows hook execution. [S8]
- **Release preparation:** post-processing preserves an existing published checksum file, distinguishes draft and public mismatches, requires a matching asset digest for cache reuse, and uploads the exact candidate bytes it checked. A public checksum mismatch stops the operation before remote writes. [S9]
- **Download-page checks:** both download pages must match reviewed skeletons with their version/size slots filled. The homepage JavaScript import closure is pinned by names and hashes. Staging and remote verification use the shared checker; missing skeletons or unreadable inputs fail the check. This draft does not claim the live homepage has been updated. [S10]
- **Regression tests:** selected PTY, writer-handoff and recorder tests now observe the event being tested instead of relying on a short sleep. The bash-specific parked-reader hang has a bounded, named fixture reap. A wall-clock-premise ratchet checks newly introduced timing assumptions. These are test changes, not a claim that every Windows failure has been repaired. [S11]

## Optional script before automatic restoration

Before restoring seats, the daemon can run one user-selected Python script whose bytes match the `default` entry in `~/.cys/local/phoenix-allow.sha256`. It checks a read-only restoration plan before and after the hook, bounds the hook, and skips built-in restoration only when the post-hook plan certifies that nothing remains. Without the allow file, the existing built-in restoration path remains the default. [S12]

The same work adds the read-only Phoenix `plan` command, result-file handling, and a single later recheck when restoration yields to another active restoration. See `docs/INSTALL.md`, “사전 복원 스크립트”, for the configuration and exit contract. [S12]

**Residual risk:** on macOS/Linux, a helper spawned by the hook can retain `hook.run.lock` through inherited stdin after the hook exits, preventing automatic restoration on the next boot until that helper ends; hook authors must give helpers `</dev/null` or `stdin=DEVNULL`. Removing the allow-file entry disables the optional hook. [S12]

Windows hook execution under Smart App Control, task-scheduler environment, Job nesting/spawn-to-assignment behaviour, stall cadence, and final-candidate native hook tests remain **UNMEASURED/PENDING** in this draft. The lane's reported macOS tests do not replace those checks. [S12]

## Still pending before this draft can become final

- **P1:** not part of the `f6429140` preparation baseline; no P1 outcome is asserted. Incorporate the reviewed final P1 report and rerun the affected checks after integration. [S0]
- **C-03 and other native CI:** collect same-candidate `ci-branch`, `windows-build`, and `windows-health` evidence, including inner test failures rather than only run colours. [S8, S13]
- Regenerate the previous-vendor table after all pack changes land; repeat version, secret and Windows typechecking gates at the final candidate. Synchronize the six copy-drift watched files against remote main before tagging. [S13, S14]
- Produce and verify both notarized macOS DMGs, Windows setup EXE/ZIP and updater/pack assets, full checksums, signature status, protected publication, and the homepage's remote checks. None is declared complete by these notes. [S1, S9, S10, S13]

## Sources and evidence scope

References name commits present in the preparation baseline, or repository documents. Confidence is High for the existence and content of the cited changes; runtime coverage is limited to the measurements explicitly attributed above. Independent release-wide validation remains pending.

- **S0:** integration commit `f6429140`; its first-parent merge history defines this draft's scope. P1 pending is the PRETAG-PREP assignment boundary.
- **S1:** `.github/workflows/release.yml`, “Windows signing mode (dormant - unsigned only in this release)”; `scripts/release-verify.py`, `check_sac_text` and stage 8. Policy, not an asset measurement.
- **S2:** `c72262e5`, superseded in its waiting/retry behaviour by `54373fe5`; receipt follow-ups `78de37c3`, `84c9fdd5`; event-based test `21d0d395`. Commit descriptions retain the native-validation limits.
- **S3:** `9254af1a` (spool failure status), `0ae1eb09` (old emitter removal).
- **S4:** `86ec053e`, `e2df32cc`, `b38123fa`; PACKTOOLS phase6c report, sections 1 and 5.
- **S5:** `6d1696c6`, `611cea8c`, `f5cdccc5`, `5be14116`; PACKTOOLS phase4 report, sections 1–2.
- **S6:** `494f5be2`, `c980ede0`; SOCKET-P8 implementation report, sections 1, 6–7.
- **S7:** `972ffa3b`, `576773dc`, `99916589`, `e59aae92`, `93d6df29`, `75b31682`, `14e32a88`; CARRIER implementation report and addendum 5, sections 1 and 3.
- **S8:** `7e3d771a`, `b4af461f`, `2b9897bd`, `07214604`, `af092542`, `f64f990b`; C03-WIN-FIXES implementation report and final stdin-fix commit description. Native validation remains pending at the final candidate.
- **S9:** `a6fc78ea`, `e4c588e5`, `6a529c14`; P9 H-05 report, sections 1–2.
- **S10:** `eab9a144`, `0be7aaf5`; H04R implementation report, sections 3–4.
- **S11:** `102654b5`, `a9122bf0`, `cfc08f4a`, `91cd57d2`, `c3d9de8d`, `16552bc8`, `20ce7526`, `d3f17a90`; SLEEP-PREMISE and SLEEP-BASH-HANG reports. The remaining foreign-write refusal is not declared fixed.
- **S12:** `bba00fa5`, `6081e83a`, `678c1f54`, `7c053eee`, `4fb246f5`, `376d90c4`, `0c248a10`, `6635e817`; `docs/RELEASE_NOTES_0.14.56-PHX.md` and PHX S6 report, sections 2–6.
- **S13:** `docs/RELEASE.md` sections 0, 0-C and release checklist; release, postprocess and protected-publish workflows at `f6429140`.
- **S14:** `scripts/gen_prev_vendor_hashes.py` and `src/pack.rs::classify_stale_entry_with`; preparation commits `fead28c5` (version) and `a9fd6950` (vendor table). This generated table must be refreshed at the final candidate.
