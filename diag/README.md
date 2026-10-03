# diag — 윈도우 11 인앱 업데이트 실측 하네스 (진단 전용)

제품 코드는 하나도 바꾸지 않는다. 이 고아 브랜치(`diag/win11-update`)에는 이 하네스만 있다.

## 무엇을 재는가 (질문 → 결과 파일)

| 질문 | 내용 | 주 결과 파일 |
|---|---|---|
| QA | 설치된 0.14.37 앱이 `install_update` 로 실제 0.14.42 가 되는가 (win11-arm · 2025 · 2022 대조) | `e2e-verdict.json` · `e2e-timeline.txt` · `e2e-cdp.json` · `e2e-screen-*.png` |
| QB | SAC 와 같은 규칙(신뢰 CA 서명 또는 ISG 평판)에서 무엇이 막히는가 (설치기 3종 · 앱 exe · 동봉 exe 전부 · 대조군) | `sac-audit-matrix.txt` · `sac-enforce-matrix.txt` (+ `.json`) |
| QC | 막힐 때 호출자가 받는 값 (CreateProcessW · ShellExecuteW · ShellExecuteExW) | `sac-qc.json` · `sac-enforce-probe.json` |
| QD | 평가(감사)에서 0.14.37 설치 → 강제로 전환 → 앱이 뜨는가, 업데이트 버튼은 | `sac-e2e-verdict.json` · `sac-enforce-events.json`(3077 정책명·경로) |
| QE | 진짜 SAC 를 레지스트리 + `CiTool -r` 로 켤 수 있는가 (값 1·2 를 모두 시험 — 값의 뜻이 자료마다 달라 `citool` 정책명·`SmartAppControlState` 로 판정) | `sac-real.json` |
| QF | `ShellExecuteW` 직후 `exit(0)` 가 설치기 기동을 놓치는가 (40회 반복) | `replica.json` · `replica-summary.txt` |
| QG (2차) | **진짜 SAC 를 켠 채로** 전체 재현: 켠 뒤 설치기·앱 exe·설치된 exe 가 무엇이 막히는가, 이미 떠 있던 앱의 `install_update` 는 어떻게 되는가(A), 설치된 앱을 새로 띄우면(B) | `sacreal-summary.txt`(10줄) · `sacreal-matrix.txt`(+`.json`) · `sacreal-e2e-A-verdict.json` · `sacreal-e2e-B.json` |

## 구성

- `.github/workflows/diag-win11-update.yml` — 잡 `e2e`(win11-arm · 2025 · 2022), 잡 `sacrules`(win11-arm · 2025), 잡 `sacreal-e2e`(win11-arm, 2차). `diag/**` 브랜치 푸시로 돈다.
  커밋 메시지에 `[sacreal-only]` 가 있으면 앞의 두 잡은 건너뛰고(`if:` 조건) `sacreal-e2e` 만 돈다. `sacreal-e2e` 는 조건 없이 항상 돈다.
- `diag/lib.ps1` 공통(+ 2차에서 `sacrules.ps1` 에서 글자 그대로 옮긴 매트릭스·이벤트 도우미 10개) · `p0-facts.ps1` 환경 사실 · `p1-assets.ps1` 내려받기·추출·대조군 ·
  `e2e-update.ps1` + `cdp-update.mjs` QA · `replica.ps1` + `replica/*.rs` QF · `sacrules.ps1` QB·QC·QD · `sac-real.ps1` QE ·
  `sacreal-e2e.ps1` QG · `publish-results.mjs` 결과 업로드.

## 결과 읽는 법

1. 결과는 저장소의 `diag-results/<run_id>-<attempt>-<job>-<os>` 브랜치(부모 없는 커밋 1개)에 있다. 같은 파일이 Actions 아티팩트(`diag-<job>-<os>`)에도 올라간다.
2. 먼저 `_publish-manifest.json`(올라간 파일 목록) → `syntax-check.txt`(윈도우 5.1 파서가 본 스크립트 문법 오류) → `p0-facts.txt`(러너 사실) → 위 표의 주 결과 파일 순서로 본다. sacrules 잡에는 `ensure-policy-removed.txt`(정책이 남지 않았다는 확인), sacreal-e2e 잡에는 `ensure-sac-restored.txt`(값 0 확인·`CiTool -r`)도 있다.
3. `done-<스크립트>.json` 이 없으면 그 스텝은 끝까지 가지 못했다(타임아웃 등). `log-<스크립트>.txt` 에 진행 로그가 있다.
4. 모르는 필드명은 추측하지 않고 원본(XML·JSON·텍스트·`.evtx`)을 통째로 남겼다. 이벤트는 `sac-*-events.json` 의 `xml` 필드가 원본이다.
5. (2차 `sacreal-e2e`) `sacreal-summary.txt`(10줄) → `sacreal-state-*.json`(켜기 전·10초 뒤·복구 뒤의 3상태) → `sacreal-matrix.txt` → `sacreal-e2e-A-verdict.json`(+ `sacreal-e2e-A-timeline.txt` · `sacreal-e2e-A-screen-*.png`) → `sacreal-e2e-B.json` → `ensure-sac-restored.txt`. 체크포인트 브랜치는 `…-sacreal-e2e-pre-sac-…`(켜기 직전)·`…-sacreal-e2e-after-A-…`(A 직후).
6. 대조군 해석 줄이 `sac-*-matrix.txt` 머리에 있다 — ISG 가 작동하는지(미서명 신규 exe 는 걸리고 널리 쓰이는 미서명 설치기는 안 걸리는지)를 먼저 확인한 뒤 나머지 표를 읽는다.

## 주의

- 측정 스텝은 실패해도 잡을 빨갛게 하지 않는다(결과가 데이터). 업로드(publish) 실패만 빨강이다.
- `sacrules` 는 한 PowerShell 프로세스에서 감사 → 강제 → 제거까지 가고, `finally` 에서 정책 제거와 이벤트 수집을 반드시 시도한다.
- 강제 정책이 러너를 죽일 수 있어서, `sacrules` 는 강제 직전(감사 결과)과 제거 직후에 같은 업로더로 중간 결과를 먼저 올린다(`diag-results/<run>-<attempt>-sacrules-audit-<os>` 등). e2e 잡의 `windows-11-arm` 은 `sac-real`(진짜 SAC 전환 시도) 직전에 한 번 올린다.
- SAC·Defender 를 우회하는 코드는 없다. 관측만 한다.

## 2차: `sacreal-e2e` (진짜 SAC 를 켠 채로 전체 재현)

한 PowerShell 프로세스에서 순서대로 간다. `Add-Type`(네이티브 도우미·스크린샷 어셈블리)과 각종 워밍업은 SAC 를 켜기 **전에** 끝낸다.

1. SAC 꺼진 상태에서 `inst-0.14.37` 을 `/S` 로 설치, 버전 마커 확인, 설치된 `cys-app.exe`·`cysd.exe`·`cys.exe` 의 `fsutil file queryEA` 기록, 설치가 띄운 프로세스는 설치 폴더 경로로 종료.
2. (A) 설치된 앱을 **먼저** CDP 플래그 + HKLM 정책으로 띄워 attach 확인, `check_update` 값 기록(`sacreal-e2e-A-pre-cdp.json`). 아직 `install_update` 는 부르지 않는다.
3. 켜기 직전 체크포인트 게시 → 레지스트리 값 1 + `CiTool -r` → 10초 뒤 3가지 상태(레지스트리 · `citool -lp -json` 의 `VerifiedAndReputable*` · `Get-MpComputerStatus`) 기록(`sacreal-state-*.json`).
   켜짐이 확인되지 않으면 그대로 기록하고 `measurable:false` 로 끝낸다.
4. 판정 매트릭스: 설치기 3 · 앱 exe 9 · 대조군(NEG · POS-UNSIGNED · POS-SIGNED) · 설치된 0.14.37 exe 3 · 러너 node.exe 를 `CreateProcessW(CREATE_SUSPENDED)` 로만 시험(코드 실행 없음) → `sacreal-matrix.json/.txt`(열은 1차 enforce 매트릭스와 같음), 3076/3077/3089/309x 원본은 `sacreal-3077.json`·`sacreal-events-matrix.json`·`sacreal-ci-matrix.evtx`.
5. (A 계속) 이미 떠 있는 앱에 CDP 로 `install_update {force:true}` 호출, 최대 6분을 2초 간격 타임라인으로 관찰(다운로드 → 임시 인스톨러 → 인스톨러 기동 여부 → 앱 종료 → 마커 → 재기동). 스크린샷은 종료 직후(`-screen-exit.png`) · 20초 · 60초 · 20초마다(`-screen-tNNNNs.png`). → `sacreal-e2e-A-verdict.json`, `sacreal-e2e-A-events.json`, 임시 인스톨러에 대한 지금의 `CreateProcessW` 결과·EA·SHA256, 알림 DB 복사본(`sacreal-A-wpndatabase.db*`).
6. (B) 남은 cys 프로세스를 설치 폴더 경로로 정리한 뒤 설치된 `cys-app.exe` 를 `Start-Process` 로 진짜 실행. 막히면 원시 오류와 4551 여부, 실행되면 60초 뒤 프로세스·창·CDP attach 여부. 이어서 `cysd.exe --version`·`cys.exe --version` → `sacreal-e2e-B.json`.
7. **복구**(값 0 + `CiTool -r`, 최대 3회) → 상태 재확인 + NEG 대조군이 다시 *실제로 실행*되는지 → 전체 SAC 창 동안의 이벤트(`sacreal-events-final.json`·`sacreal-3077-final.json`·`sacreal-ci-final.evtx`·`sacreal-final-matrix.*`) 수집 → `sacreal-summary.txt`(10줄 이내: SAC 켜짐 확인 · 종류별 막힘/허용 수 · A 결과 · B 결과 · 복구 확인).

안전장치: `finally` 에서 항상 복구(값 0 + `CiTool -r` + 재확인), 워크플로의 `ensure-sac-restored`(`if: always()`, PowerShell 없이 `reg.exe`·`CiTool.exe` 만)가 값이 0 이 아니면 0 으로 되돌리고 `CiTool -r` 로 다시 적용한다. 스텝마다 언어 모드를 로그에 남긴다.
SAC 를 켠 뒤에는 서명된 `node.exe` 와 시스템 도구만 새로 실행한다(러너 node 가 막히면 cys 런타임의 서명된 node 로 대체하고 그 사실을 `notes` 에 적는다).

읽을 때 주의:
- 같은 브랜치에 연달아 푸시하면 앞 런이 취소된다(워크플로의 `concurrency`). SAC 를 켠 구간에서 취소되면 스크립트의 `finally` 는 돌지 못하고 `ensure-sac-restored`(`if: always()`)가 복구한다. 그 사이 결과는 마지막 체크포인트까지만 남는다 — 푸시는 한 번만.
- 지시된 순서(매트릭스 → A) 때문에 0.14.42 설치기는 A 의 `install_update` 보다 먼저 `CreateProcessW` 시험을 받는다. 평판(ISG) 판정이 그 사이 캐시됐을 수 있으니 A 결과는 그 점을 감안해 읽는다.
- 이벤트(`sacreal-events-final.json`·`sacreal-3077-final.json`·`sacreal-final-matrix.*`)는 **복구 뒤에** 지속 로그에서 읽는다(복구 시작 이후 이벤트는 개수만 적고 목록에서 뺀다). `sacreal-ci-final.evtx` 는 복구 이벤트까지 전부 담는다.
