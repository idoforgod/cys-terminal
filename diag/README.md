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

## 구성

- `.github/workflows/diag-win11-update.yml` — 잡 `e2e`(win11-arm · 2025 · 2022), 잡 `sacrules`(win11-arm · 2025). `diag/**` 브랜치 푸시로 돈다.
- `diag/lib.ps1` 공통 · `p0-facts.ps1` 환경 사실 · `p1-assets.ps1` 내려받기·추출·대조군 · `e2e-update.ps1` + `cdp-update.mjs` QA ·
  `replica.ps1` + `replica/*.rs` QF · `sacrules.ps1` QB·QC·QD · `sac-real.ps1` QE · `publish-results.mjs` 결과 업로드.

## 결과 읽는 법

1. 결과는 저장소의 `diag-results/<run_id>-<attempt>-<job>-<os>` 브랜치(부모 없는 커밋 1개)에 있다. 같은 파일이 Actions 아티팩트(`diag-<job>-<os>`)에도 올라간다.
2. 먼저 `_publish-manifest.json`(올라간 파일 목록) → `syntax-check.txt`(윈도우 5.1 파서가 본 스크립트 문법 오류) → `p0-facts.txt`(러너 사실) → 위 표의 주 결과 파일 순서로 본다. sacrules 잡에는 `ensure-policy-removed.txt`(정책이 남지 않았다는 확인)도 있다.
3. `done-<스크립트>.json` 이 없으면 그 스텝은 끝까지 가지 못했다(타임아웃 등). `log-<스크립트>.txt` 에 진행 로그가 있다.
4. 모르는 필드명은 추측하지 않고 원본(XML·JSON·텍스트·`.evtx`)을 통째로 남겼다. 이벤트는 `sac-*-events.json` 의 `xml` 필드가 원본이다.
5. 대조군 해석 줄이 `sac-*-matrix.txt` 머리에 있다 — ISG 가 작동하는지(미서명 신규 exe 는 걸리고 널리 쓰이는 미서명 설치기는 안 걸리는지)를 먼저 확인한 뒤 나머지 표를 읽는다.

## 주의

- 측정 스텝은 실패해도 잡을 빨갛게 하지 않는다(결과가 데이터). 업로드(publish) 실패만 빨강이다.
- `sacrules` 는 한 PowerShell 프로세스에서 감사 → 강제 → 제거까지 가고, `finally` 에서 정책 제거와 이벤트 수집을 반드시 시도한다.
- 강제 정책이 러너를 죽일 수 있어서, `sacrules` 는 강제 직전(감사 결과)과 제거 직후에 같은 업로더로 중간 결과를 먼저 올린다(`diag-results/<run>-<attempt>-sacrules-audit-<os>` 등). e2e 잡의 `windows-11-arm` 은 `sac-real`(진짜 SAC 전환 시도) 직전에 한 번 올린다.
- SAC·Defender 를 우회하는 코드는 없다. 관측만 한다.
