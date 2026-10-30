# 0.14.56 homepage preparation

Status: local preparation only. No release assets or homepage publication are certified here.
Basis: release-notes draft at 04fd9913, S10 (reviewed download skeletons), S1
(Windows unsigned/SAC notice), S9 (checksum handling), and the pending asset checks.
The release-day instruction requires SIZE slots to remain explicit until the published
assets have been measured. This document does not change the reviewed page copy.

## Prepared change

Render the existing private, hash-pinned skeletons for version `0.14.56`. Keep the
four following values unresolved in preparation; never copy a previous release's size:

| Asset | Pending value |
|---|---|
| cys_0.14.56_aarch64.dmg | {{cys:SIZE:_aarch64.dmg}} |
| cys_0.14.56_x64.dmg | {{cys:SIZE:_x64.dmg}} |
| cys_0.14.56_x64-setup.exe | {{cys:SIZE:_x64-setup.exe}} |
| cys_0.14.56_x64-setup.zip | {{cys:SIZE:_x64-setup.zip}} |

The private skeletons remain outside git. Their names, hashes, version-slot counts,
size suffixes, required guidance and JavaScript closure are defined only in
`scripts/download_page_check.py::PROFILES`. Do not duplicate or loosen those pins.
The website is `www.cysinsight.com`; `docs/index.html` is a separate GitHub Pages
surface and is not the release download-page source. Do not bump that stale page.

## Release-day fill and refusal sequence

1. Bind the final published release tag and commit, the published `SHA256SUMS.txt`
   digest and the downloaded asset bytes. Verify every checksum and the complete
   release bundle. Record each of the four files' measured byte sizes; compare
   against the published asset metadata. Missing or inconsistent evidence stops here.
2. Load the private skeletons and verify their PROFILES hashes. Fetch the two current
   pages and check their fixed bytes against those skeletons, so a live page edit is
   not silently reverted. Preserve Windows Defender/SAC guidance and the pinned JS.
3. Fill version `0.14.56` and SIZE values with `download_page_check.render` using
   `download_page_check.mib(measured_bytes)` (integer MiB floor). Never substitute
   guessed numbers. The prepared pages are not publishable while any slot remains.
4. Use the existing staged SSH release procedure: upload both checker modules and
   both skeletons alongside the staged pages, retain backup and journal evidence,
   then run `homepage-switch.py verify-stage` before `switch`. Both commands already
   call the shared byte checker. An unresolved SIZE token yields PAGE-DIFF and exit
   3 before the first plan or public move. The regression test
   `PageCheckTests.test_each_unfilled_release_size_refuses_before_any_public_move`
   checks all four slots individually, then fills the same slot and proves the
   same fixture can switch. No replacement gate or bypass is introduced.
5. After the authorized switch, run `verify-release-remote.py 0.14.56 <measured-old-version>
   --skeleton-dir <private-skeleton-folder>`. Check both pages, four download links
   and sizes, checksums for every asset, guidance and the complete pinned JS closure.
   Preserve the full output and switch journal. A failed check is not completion.

The SSH release wrapper is a release-step artifact outside this repository; verify
that it uploads `download_page_check.py` and the skeletons before using it. Do not
use `deploy-homepage.py --apply`: that older route deletes old remote assets.
The final release notes must describe the delta from the latest public release,
including retained changes from the unpublished 0.14.55; this preparation does not
certify that editorial reconciliation or authorize a public write.

## Change boundary

This preparation changes documentation and a local-fixture regression test only.
No shipped Windows executable, DLL, installer or updater replacement code changes.
The release still needs actual Authenticode/SAC status and honest release notices.
No daemon, queue, restoration, context-cycle or live-pane operation is added.
