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
| QH (3차) | **제품 코드의 설치 파일 실행 모듈(`src/update_launch.rs`)을 진짜 SAC 에서 그대로** 시험: 켜기 전 성공 경로(서명된 cmd.exe 사본) · 켠 뒤 진짜 0.14.42 설치기는 `Err 4551 / shell_ret 5 / AppControl` · `remove_installer` 정리 · 서명된 대조는 통과 | `product-launch-summary.txt`(10줄) · `product-launch-x64.jsonl`(본 판정) · `product-launch-aarch64.jsonl`(보조) · `product-launch-verdict.json` · `product-3077.json` |

## 구성

- `.github/workflows/diag-win11-update.yml` — 잡 `e2e`(win11-arm · 2025 · 2022), 잡 `sacrules`(win11-arm · 2025), 잡 `sacreal-e2e`(win11-arm, 2차), 잡 `product-launch`(win11-arm, 3차). `diag/**` 브랜치 푸시로 돈다.
  **커밋 메시지 태그로 잡을 고른다**: `[product-only]` → `product-launch` 만 · `[sacreal-only]` → `sacreal-e2e` 만 · 태그 없음 → 전부 · 두 태그 → 그 둘. `e2e`·`sacrules` 는 두 태그가 모두 없을 때만 돈다(`if:` 조건).
- `diag/lib.ps1` 공통(+ 2차에서 `sacrules.ps1` 에서 글자 그대로 옮긴 매트릭스·이벤트 도우미 10개) · `p0-facts.ps1` 환경 사실 · `p1-assets.ps1` 내려받기·추출·대조군 ·
  `e2e-update.ps1` + `cdp-update.mjs` QA · `replica.ps1` + `replica/*.rs` QF · `sacrules.ps1` QB·QC·QD · `sac-real.ps1` QE ·
  `sacreal-e2e.ps1` QG · `sac-lib.ps1`(sacreal-e2e 에서 글자 그대로 옮긴 진짜 SAC 켜기·복구·체크포인트·이벤트 도우미 — 3차가 같이 쓴다) ·
  `product-launch.ps1` + `product/`(`update_launch.rs` = 제품 파일 사본 · `main.rs` = 드라이버) QH · `publish-results.mjs` 결과 업로드.

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

## 3차: `product-launch` (제품의 설치 파일 실행 모듈을 진짜 SAC 에서 그대로)

제품 쪽 `src/update_launch.rs`(커밋 `aa0b1adb` · std 만 쓰는 독립 모듈)를 **한 글자도 고치지 않고** `diag/product/update_launch.rs` 에 복사해 두고(sha256 **`570476c28df10e9280036cee4da37301175d2fd215a49f5cb16627fd00e88ddc`** · `product-launch.ps1` 이 실행 때 다시 계산해 결과 파일에 적는다 · `.gitattributes` 로 줄바꿈 변환을 막았다), `diag/product/main.rs`(드라이버)가 `mod update_launch;` 로 그 파일을 그대로 포함해 앱과 **같은 호출**(`write_installer` → `launch_installer(file, nsis_update_params(&[]))` → `remove_installer`)을 한다.

- 드라이버 `sac-launch.exe <work_dir> <cmd_exe_path> <installer_path>` 는 **SAC 를 켜기 전에** 떠서 신호 파일을 기다린다(켠 뒤에 새로 뜨는 미서명 exe 는 막히므로).
  - OFF 단계(뜨자마자): ① `write_installer(work_dir, "cys", "0.0.0-diag", <cmd.exe 바이트>)`(경로 꼴 기록) → `launch_installer(그 파일, "/c exit 0")` 기대 **Ok**(서명된 cmd.exe 사본 = 성공 경로) ② `product-launch.jsonl`(한 줄 한 JSON)에 기록하고 `ready` 파일 생성.
  - `go` 파일이 생길 때까지 대기(최대 10분 · 0.5초 폴링 · `abort` 파일이면 일찍 끝냄).
  - ON 단계(PowerShell 이 그 사이 진짜 SAC 를 켠 뒤): ③ `write_installer(…, "0.14.42", <진짜 설치 파일 바이트>)` → `launch_installer(…, nsis_update_params(&[]))` 기대 **Err · os_code 4551 · shell_ret 5 · block()=AppControl · Display `installer_launch_failed:4551:5`** → `remove_installer` 뒤 파일·폴더 소멸 기록 ④ 대조: cmd.exe 사본 다시 `write_installer` + `launch_installer(…, "/c exit 0")` 기대 **Ok** ⑤ 호출마다 소요 ms(5초 초과면 `slow`)·`driver_alive_through_on_stage` 기록, `done` 파일, 종료 코드 0.
  - 실제 반환값을 그대로 적는다(`expect`/`matches` 가 옆에 붙지만 **판정은 PowerShell 요약**이 한다). MZ 머리가 없는 파일과 호출 직전에 사라진 파일(검사기가 치웠다면)은 띄우지 않고 기록한다(Windows 가 모달 오류 창을 띄움). 각 `launch_installer` 호출은 자기 스레드에서 돌고 40초 안에 안 돌아오면 `timed_out` 으로 적고 파일은 그대로 둔 채 계속한다(모달 창에 붙잡혀도 `done` 은 쓴다). 서명 대조(`cmd.exe` 사본)가 실패하면 원본 `System32\cmd.exe` 를 한 번 더 띄워 비교한다(진단용 · 기대 아님).
- `product-launch.ps1`: 한 PowerShell 프로세스 — 빌드(`rustc --edition 2021 -O -C target-feature=+crt-static`, **x64**(`--target x86_64-pc-windows-msvc` · 본 판정)와 **aarch64**(호스트 · 보조); 한쪽이 안 지어지면 사유 기록) → 두 드라이버 기동·`ready` 확인(OFF 결과) → 체크포인트 게시 → 진짜 SAC 켬(2차와 같은 3상태 확인 · 안 켜지면 `measurable:false`) → `go` → `done` 대기(최대 3분) → 출력·jsonl 수집 → 스크린샷 1장 → 복구(값 0 + `CiTool -r` + 확인 + NEG 대조군 실행) → 3077 이벤트(경로에 `cys-0.14.42-updater-` 가 있는 것 표시) → 판정 → 요약 `product-launch-summary.txt`(10줄).
- **판정(PASS)** = x64 드라이버(없으면 aarch64 만 · 근거 표시)의 ① OFF 성공 경로 Ok ② ON 차단이 정확히 `Err 4551 / 5 / AppControl` ③ 정리 완료 ④ 서명 대조 Ok ⑤ ON 단계 끝까지 도달, 그리고 SAC 켜짐 확인. 하나라도 어긋나면 FAIL + 이유. 5초 넘은 호출 · 경로가 플러그인 꼴이 아님 · 즉시는 안 지워졌지만 3초 안에 사라짐 · 복구 미확인은 판정을 바꾸지 않는 NOTE.
- 결과 읽는 순서: `product-launch-summary.txt` → `product-launch-x64.jsonl`(호출마다 한 줄 · SAC 켜기 전 체크포인트 때의 OFF 단계 기록은 `product-launch-<arch>-off.jsonl`) → `product-launch-verdict.json` → `product-state-*.json`(3상태) → `product-3077.json`(이벤트 원본) → `ensure-sac-restored.txt`.
- 로컬 확인(맥): `rustc --edition 2021 --test diag/product/main.rs` — 드라이버 시험 14개 + 제품 모듈 자체 시험 17개 = 31개 통과(비윈도우 갈래의 시험용 대역 사용), `--target x86_64-pc-windows-msvc --emit=metadata` 로 윈도우 갈래 타입 체크. 윈도우 실기는 러너에서 처음 돈다.
