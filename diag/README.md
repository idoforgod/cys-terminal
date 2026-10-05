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
| QI (4차) | **0.14.43 앱 전체**를 윈도우에서 **진짜 Update 버튼 클릭**으로 갱신: 꺼짐(성공 경로: 내려받기 → 설치 파일 기동 → 앱 종료 → 설치 → 재실행) · 진짜 SAC 켬(차단 경로: 앱이 살아 있고 화면에 「설치 파일 실행이 차단되었습니다」 알림 · 임시 폴더·시도 기록 정리) | `appe2e-summary.txt`(10줄) · `appe2e-off-verdict.json` · `appe2e-on-verdict.json` · `appe2e-*-ui-*.png`(앱 화면) · `appe2e-*-cdp.json` |
| QJ (7차) | **기존 사용자의 길(업그레이드)**: 공개 0.14.42 가 설치돼 떠 있는 PC 에 시험 대상 설치 파일(0.14.43)이 **0.14.42 의 업데이터가 띄우는 방식 그대로** 얹힐 때 — 설치 결과 · 다시 뜬 앱 · 새 앱과 옛 데몬의 만남 · 그 앱에서 팀 직접 만들기. 그리고 시험 대상 설치 파일을 **릴리스(draft 포함)** 에서 받는 입력 경로 | `appe2e-upgrade-summary.txt`(14줄) · `appe2e-upgrade-verdict.json` · `appe2e-upg-timeline.txt` · `appe2e-upg-after-obs-timeline.txt` · `appe2e-installer-source.json` |
| QH (3차) | **제품 코드의 설치 파일 실행 모듈(`src/update_launch.rs`)을 진짜 SAC 에서 그대로** 시험: 켜기 전 성공 경로(서명된 cmd.exe 사본) · 켠 뒤 진짜 0.14.42 설치기는 `Err 4551 / shell_ret 5 / AppControl` · `remove_installer` 정리 · 서명된 대조는 통과 | `product-launch-summary.txt`(10줄) · `product-launch-x64.jsonl`(본 판정) · `product-launch-aarch64.jsonl`(보조) · `product-launch-verdict.json` · `product-3077.json` |

## 구성

- `.github/workflows/diag-win11-update.yml` — 잡 `e2e`(win11-arm · 2025 · 2022), 잡 `sacrules`(win11-arm · 2025), 잡 `sacreal-e2e`(win11-arm, 2차), 잡 `product-launch`(win11-arm, 3차). `diag/**` 브랜치 푸시로 돈다.
  **커밋 메시지 태그로 잡을 고른다**: `[product-only]` → `product-launch` 만 · `[sacreal-only]` → `sacreal-e2e` 만 · `[appe2e-only]` → `app-e2e`(와 그 문지기 잡)만 · `[cysdwin-only]` → `cysd-win` 만 · 태그 없음 → `cysd-win` 을 뺀 전부(단 `app-e2e` 는 `diag/appe2e-input.json` 의 런 번호가 0 이면 건너뜀 · `cysd-win` 은 자기 태그가 있을 때만 돈다) · 태그 둘 → 그 둘. `e2e`·`sacrules` 는 네 태그가 모두 없을 때만 돈다(`if:` 조건). 4차에서 잡 `appe2e-gate`(ubuntu · 입력 파일만 읽음)와 `app-e2e`(win11-arm)가 늘었다.
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

## 4차: `app-e2e` (0.14.43 앱 전체를 진짜 Update 버튼으로 — 실행은 master 가 윈도우 빌드가 나온 뒤에)

만들어 두기만 한 하네스다(이 커밋은 `[skip ci]` · push 안 함). 0.14.43 앱 전체를 윈도우에서 돌려 본 적이 없다 — 성공 경로(설치 파일이 뜨고 → 앱 종료 → 설치 완료)와, 막혔을 때 **앱 화면에 알림이 뜨는 것**을 잰다.

- **재료**
  - 0.14.43 설치 파일 = 제품 브랜치 `fix/0.14.43-bugreport` 를 푸시하면 제품 저장소의 `windows-build.yml` 이 만드는 아티팩트 **`cys-windows-x64-nsis`**(`target/release/bundle/nsis/*.exe`). 잡 `app-e2e` 가 `actions/download-artifact@v4`(`run-id`+`github-token` · 잡에 `actions: read`)로 그 런의 아티팩트를 받는다. 런 번호·아티팩트 이름은 **`diag/appe2e-input.json`**(`{"windows_build_run_id": <숫자>, "artifact": "<이름>"}`)에서 읽는다 — **지금은 0** 이라 태그 없는 푸시에서는 `app-e2e` 가 건너뛰어진다(`[appe2e-only]` 로 돌리면 '입력 없음'을 결과에 적고 끝).
  - 업데이트 대상 = 앱의 시험용 환경변수 `CYS_UPDATE_MANIFEST_URL`(제품 `build_updater`)로 가리키는 매니페스트 **`diag/appe2e-manifest.json`**: `version` 0.14.99 · `platforms["windows-x86_64"]`(와 `-nsis`)의 `url` = 공개 0.14.42 설치 파일 · `signature` = 공개 `cys_0.14.42_x64-setup.exe.sig` 의 내용 그대로(공개 `latest.json` 과 글자 그대로 같음을 확인했다). 서명은 파일 바이트에 대한 것이라 버전 표기와 무관하게 통과한다 → "0.14.43 앱이 0.14.99 업데이트를 받아 실제로는 0.14.42 설치 파일을 띄운다". 재는 것은 0.14.43 의 **새 실행 경로**이지 설치 내용물이 아니다. 주소는 잡이 `https://raw.githubusercontent.com/<저장소>/<이 커밋 sha>/diag/appe2e-manifest.json` 으로 조립한다(저장소가 공개여야 한다).
- **`diag/app-e2e.ps1`**(한 PowerShell 프로세스) 흐름
  1. 입력 확인(설치 파일·매니페스트 주소가 열리는지·글자 그대로 이 커밋의 파일인지) → SAC 꺼짐 기준선
  2. **OFF(성공 경로)**: 0.14.43 `/S` 설치 → 버전 표식 확인 → 앱 기동(WebView2 CDP + 매니페스트 환경변수) → `node diag/cdp-update.mjs --mode ui` 가 **Update 버튼 클릭 → 업데이트 창의 「본체 패치 설치」 클릭 → 확인 창의 「설치」 클릭**(진짜 마우스 이벤트 · 가려지면 DOM 클릭 · 어느 쪽인지 기록) → 관측: 내려받기 · 임시 설치 파일(`cys-0.14.99-updater-*\cys-0.14.99-installer.exe`) · 설치 파일 프로세스 · 앱 종료 · 버전 표식(0.14.42 가 될 것) · 앱 재실행 → `appe2e-off-verdict.json`. (0.14.42 로 내려 설치라 NSIS 가 창을 띄워 멈추면 '설치 파일 기동·앱 종료까지 확인, 설치 완료는 미완'으로 사실대로 적고 스크린샷을 남긴다.)
  3. 0.14.43 다시 `/S` 설치 → 앱 기동 → `--mode uipre`(UI 준비 확인) → 체크포인트 → **진짜 SAC 켬**(값 1 + `CiTool -r` · 3상태 확인 · 안 켜지면 `measurable:false`) → 같은 클릭 흐름 → 기대: 설치 파일은 뜨지 않고 **앱이 살아 있고**(프로세스 + CDP 연결 60초 이상) 화면에 지속 알림 「설치 파일 실행이 차단되었습니다」(본문에 4551) · 임시 설치 폴더와 시도 기록(`~/.cys/.update-attempt.json`) 정리 → `appe2e-on-verdict.json`
  4. `finally`: **복구 먼저**(값 0 + `CiTool -r` + 확인) → 정리 → 3077 이벤트(경로에 `-updater-` 가 든 것 · 프로세스 `cys-app.exe`) → 판정 → `appe2e-summary.txt`
  - UI 흐름이 중간에 막히면(선택자가 달라졌거나 창이 안 뜸) 화면 요약(`appe2e-*-ui-dom-*.json`)을 저장하고 `invoke('install_update',{force:true})` 로 대체한다 — 결과에 **'버튼 클릭 아님'**(`fallback_invoke`)으로 표시되고 판정은 최대 PARTIAL 이다.
- **판정**: PASS / PARTIAL / FAIL(이유 포함). OFF PASS = 버튼 클릭 + 설치 파일 기동 + 앱 종료 + 버전 표식 변화 + 앱 재실행. ON PASS = SAC 켜짐 확인 + 버튼 클릭 + 알림(4551) + 앱·CDP 60초 생존 + 설치 파일 미기동 + 임시 폴더·시도 기록 정리 + 버전 표식 불변.
- **master 가 실행할 때 할 일**: ① 제품 브랜치를 푸시해 `windows-build.yml` 런이 끝나기를 기다린다 ② 그 런 번호를 `diag/appe2e-input.json` 에 적고 커밋한다(제목 끝 `[appe2e-only]`) ③ **푸시는 한 번만**(연속 푸시는 앞 런을 취소한다 — SAC 를 켠 구간이면 `ensure-sac-restored` 가 복구) ④ 결과: `diag-results/<런>-1-app-e2e-windows-11-arm` 브랜치 → `appe2e-summary.txt` 부터.
- 결과 읽는 순서: `appe2e-summary.txt` → `appe2e-off-verdict.json` · `appe2e-on-verdict.json`(근거 필드 `facts`) → `appe2e-off-cdp.json` · `appe2e-on-cdp.json`(`ui.clicks`·`ui.states`·`ui.recorder.events` = 화면에 뜬 알림·창의 전체 문구) → `appe2e-*-ui-*.png`(앱 화면) · `appe2e-*-screen-*.png`(바탕화면) → `appe2e-*-timeline.txt`(2초 간격 프로세스·임시 폴더·표식) → `appe2e-3077.json` · `appe2e-state-*.json` · `ensure-sac-restored.txt`.
- 로컬 확인(맥): 가짜 DevTools 서버 + 가짜 앱 페이지(작은 DOM)로 `cdp-update.mjs --mode ui|uipre` 의 흐름 전체(마우스 클릭·DOM 클릭 대체·버튼 없음 대체·앱 종료·차단 알림)를 시험했다 — 시험 도구는 저장소 밖.

## 5차 추가: TEAM 장면 (`app-e2e` 의 마지막 장면 — 「팀 직접 만들기」를 실제 앱에서)

`app-e2e` 의 같은 PowerShell 프로세스가 끝에 한 장면을 더 돈다. **진짜 SAC 장면 · 원복 · 정리 · 이벤트 수집이 모두 끝난 뒤**(스텝 `team` — 기존 `verdict`·`summary` 스텝이 결과 파일을 쓴 **뒤**라서 TEAM 이 멈추거나 죽어도 기존 결과는 남는다. 장면 안에서 `Get-SacRealState` 로 SAC 꺼짐을 한 번 더 확인하고 아니면 `NOT_RUN`) 0.14.43 앱을 `node cdp-update.mjs --mode uiteam` 으로 조작한다(앱이 살아 있으면 그대로, 보통은 정리 단계가 죽였으므로 WebView2 디버그 정책을 다시 쓰고 새로 띄운다). 사이드바의 「전문가용 ▸」 토글(`#btn-expert-toggle`) → 「팀 직접 만들기」(`#btn-ws-dept` → `openTeamCreateFlow`) → 확인 창의 실행 단추(`.confirm-overlay .modal-yes` — 메뉴가 먼저 뜨면 마지막 항목 = 번호 팀)를 **진짜 마우스 이벤트**로 누르고(가려져 있으면 DOM 클릭으로 하되 `ui.clicks[].method` 에 `dom_click` 으로 남는다), 실행 단추를 누른 뒤 **최대 6분**(`--team-observe-sec 360`)을 1초 간격으로 본다: 탭 이름·개수, 대기 화면 문구와 단계 줄, 화면에 뜬 모든 알림(클래스·제목·본문 원문), 메뉴·확인 창, Tauri 이벤트 `dept-create-progress`(팩의 `@stage` 표지 원문)와 `daemon-event`(원문 · 개수 상한) — 변화가 있을 때만 `appe2e-team-ui-timeline.txt` 에 적고, 화면은 확인 창 · 누른 직후 · 첫 단계 문구 · 새 탭이 선 직후 · 끝에서 찍는다. 끝에서 제품이 쓰는 `list_surfaces` 로 새 팀 소켓의 좌석 수·역할을, Control Center 알람 이력으로 알림 id 를 읽는다(읽기 전용 호출뿐이다 — **클릭이 안 되면 대체 호출 없이** DOM 요약 `appe2e-team-team-dom-*.json` 과 화면을 남기고 끝낸다). PowerShell 쪽은 2초 간격으로 `cysd.exe`·`cys.exe`·`bash.exe`·`sh.exe`·`python*.exe` 의 pid·부모·명령줄·생성 시각, 이름에 `cys` 가 든 파이프(`cys` · `cys-dept-<이름>`), 팀 데몬 폴더(`%LOCALAPPDATA%\cys\cys-dept-<이름>\` 의 `cysd.log`·`formation.log` 꼬리), `~/.cys/depts.json`, 앱 파일을 장면 전후로 남기고, `cys-dept` 가 어느 bash 를 쓰는지(동봉 `runtime\git` 인가 러너의 Git Bash 인가)를 실행 중 `bash.exe` 의 경로로 적는다. **판정**(`appe2e-team-verdict.json`): PASS = 단추 클릭으로 흐름이 시작됐고 · 새 팀 탭이 섰고(대기 화면 → 실제 탭) · **클릭 뒤에 생성된** `cysd.exe`(설치 폴더 · `CreationDate` ≥ 클릭 시각)가 있고 · 클릭 전부터 떠 있던 알림을 뺀 새 `watchdog`·`health` 알림이 탭이 서기 전에 없다 — 문구는 보지 않고 구조·사실로만 판정한다(탭이 선 뒤의 「팀원 켜기 — 확인 필요」 류는 참고로만 적는다). 하나라도 아니면 FAIL(무엇이 빠졌는지 · 알림 원문 · 데몬 로그 꼬리를 싣는다), 장면을 못 돌린 사정(입력 없음 · SAC 안 꺼짐 · 시간 부족 · 앱이 안 뜸)은 `NOT_RUN`. 단계 표지의 순서와 단계별 초 · 새 탭까지 걸린 시간 · 편성 안내 문구와 그 변화 · 6분 끝의 좌석 수는 판정에 쓰지 않는 참고 사실이다(러너에는 claude·codex·agy 가 없어 팀원은 안 뜰 것이다 — 그때 화면과 데몬이 무엇을 말하는지가 관찰 대상). 기존 두 장면과 원복의 판정·순서는 그대로이고, `appe2e-verdict.json` 에는 `team` 키와 `TEAM …` 줄 하나만, `appe2e-summary.txt` 에는 11·12번째 줄(TEAM 판정 · TEAM 관찰)만 스텝 `team-report` 가 더한다(기존 10줄은 다시 쓰지 않고 뒤에 덧붙인다 · TEAM 이 FAIL 이어도 OFF·ON 표기는 안 바뀐다). 시간: 잡 65분 · 스크립트 스텝 51분 · 환경변수 `DIAG_TEAM_EXTRA_MIN=15`(장면이 쓰는 추가 시간 — 기존 장면은 `DIAG_JOB_LIMIT_MIN` 40 을 그대로 쓴다). 결과 파일: `appe2e-team-verdict.json` · `appe2e-team-facts.json`(작은 사실 묶음) · `appe2e-team-cdp.json`(전체 · `team` 아래 시간표·알림·이벤트·좌석·알람) · `appe2e-team-ui-timeline.txt`(화면 시간표) · `appe2e-team-timeline.txt`(프로세스·파이프 시간표) · `appe2e-team-observe.json` · `appe2e-team-ui-*.png` · `appe2e-team-procs-{before,after}.txt` · `appe2e-team-pipes-{before,after}.txt` · `appe2e-team-<폴더>-<로그>-tail.txt` · `appe2e-team-depts.json` · `appe2e-state-before-team.json`. 로컬 확인(맥): 가짜 DevTools 서버 + 제품 DOM 을 본뜬 가짜 페이지(저장소 밖)로 `--mode uiteam` 시나리오 8개(성공 · 메뉴가 먼저 · 생성 실패 · 복원 중 차단 · 버튼 없음 · 구 팩(단계 표지 없음) · 느린 시작 · 표지 1개)와 순수 함수 자체 검사 11개가 통과했고, 옛 모드(`version`·`uipre`)는 HEAD 판과 결과 JSON 이 같다. PowerShell 은 맥에 없어 정적 린터와 잡의 `syntax-check` 로 점검했다 — 윈도우 실기는 러너에서 처음 돈다.

## 6차 추가: `cysd-win` (제품 데몬 단위 검체 `cargo test --bin cysd` 의 **실패 원문**을 윈도우에서 받는 잡)

제품 브랜치의 `windows-health` 가 `cargo test --bin cysd` 를 처음 끝까지 돌렸다(1681 통과 · 195 실패 · 비차단). 익명 API 로는 annotation 의 잘린 목록만 보이고 로그·아티팩트는 받을 수 없어, 이 잡이 **같은 명령을 같은 환경**으로 다시 돌려 실패 원문(panic 문구·파일:줄)을 결과 브랜치(`diag-results/<런>-<시도>-cysd-win-windows-latest`)에 올린다. 환경: 러너 `windows-latest`(x64 — windows-health 와 같은 라벨) · `bash`(Git Bash) · 잡 env `PYTHONUTF8=1` · Python 3.12(`actions/setup-python@v5`) · `dtolnay/rust-toolchain@stable`(캐시 스텝 없음 — windows-health 도 없다) · `CYS_PACK_DIR="$(mktemp -d)"` · `cargo test --bin cysd -- --test-threads=1 --nocapture --skip …`(skip 은 입력 파일에서). 앞선 windows-health 스텝들(다른 `cargo test` 필터 · `~/.cys` 에 남는 것)은 재현하지 않는다 — 실패 수가 195 와 크게 다르면 그 차이부터 본다.

- **태그**: 커밋 메시지에 `[cysdwin-only]` 가 있을 때만 돈다 — 이 태그가 있으면 이 잡만, 다른 태그만 있거나 태그가 없으면 안 돈다(기존 잡들의 `if:` 는 새 태그를 부정 조건으로 하나씩 더했을 뿐, 이 태그가 없으면 종전과 같게 평가된다). **준비 커밋에는 이 글자를 넣지 말고 `[skip ci]` 를 쓴다**(넣으면 그 푸시에서 돈다).
- **입력 `diag/cysdwin-input.json`**: `{"ref": "<제품 커밋 SHA>", "filter": "", "skip": ["<검체 이름>", ...]}`. `ref` 는 이 저장소 안의 커밋이어야 한다(`actions/checkout@v4` 의 `ref` · `path: p` · `fetch-depth: 0` — 진단 브랜치 체크아웃과 별도 폴더). `ref` 가 비었거나 글자가 허용 밖이면 아무것도 돌리지 않고 이유를 요약에 적는다(입력 없음 런).
- **제한**: cargo 스텝 45분(빌드 포함 · `continue-on-error`) · 잡 75분. 45분에 잘리면 rc 표지가 없고 요약이 **마지막으로 시작한 검체**를 적는다(`--nocapture` 라 이름이 실행 전에 찍힌다).
- **가공 `diag/cysdwin-digest.py`**(표준 라이브러리만 · 어떤 경우에도 종료 코드 0): `cysdwin-summary.txt`(`test result:` 줄 · 실패 수와 목록·결과줄의 일치 · 모듈별 수(이름의 `::tests::` 앞) · panic 위치(파일:줄)별 수 상위 30 · 메시지 유형별 수 — **분류 규칙(낱말·정규식)과 순서를 요약 안에 적는다** · 정규화한 첫 메시지 줄 상위 30) · `cysdwin-digest.tsv`(순번 · 검체 · `panicked at` 위치 원문 · 파일:줄 · 메시지 앞 300자 · 스레드 · panic 수 · 파일) · `cysdwin-failures/<순번>-<이름>.txt`(검체별 로그 조각 — `--nocapture` 이면 `test <이름> ... ` 줄부터 `FAILED` 까지, 캡처 모드면 `---- <이름> stdout ----` 블록 · 파일당 10KB 상한) · `cysdwin-cargo.log`(전체 로그 — 40MB 미만이면 그대로, 넘으면 앞뒤를 잘라 저장하고 그 사실을 요약과 로그 안에 적는다) · `cysdwin-digest.json` · `cysdwin-meta.json` · `cysdwin-env.txt`(러너 이미지·도구 버전·경로 규약).
- 요약의 앞 3500자와 실패 첫 40건은 `::notice` annotation 으로도 올린다(익명 API 로 읽히는 유일한 면).
- **master 가 실행할 때 할 일**: `diag/cysdwin-input.json` 의 `ref` 에 제품 커밋 SHA 를 적고 커밋 제목에 `[cysdwin-only]` 를 넣어 푸시한다(**푸시는 한 번만** — 같은 브랜치의 다음 푸시는 도는 런을 취소한다). 결과: 결과 브랜치의 `cysdwin-summary.txt` 부터. 로컬 확인(맥): 가짜 cargo 로그 시나리오 10개(`--nocapture` · CRLF · 캡처 모드 · 절단 · 컴파일 오류 · 전량 통과 · 큰 로그 절단 · 한글·이상한 이름 · 로그 없음 · annotation)와 195건 규모 시험으로 가공 스크립트를, 스텁 cargo 로 잡의 bash 스텝을 실제로 돌렸고, 워크플로 `if:` 를 16개 태그 조합 × 2(런 번호 있음·없음)로 이전·이후 비교했다. PowerShell 스텝은 맥에 PowerShell 이 없어 정적 린터로만 점검했다 — 러너에서 처음 돈다.

## 7차 추가: 릴리스 설치 파일 입력 · UPGRADE 장면 (`app-e2e` 가 장면별 매트릭스로 돈다)

(작업 지시서에는 「6차」로 적혀 있다. 이 문서에는 이미 「6차 추가: `cysd-win`」 절이 있어 번호만 7 로 적는다 — 내용은 같다.)

### 무엇을 재나

1. **기존 사용자의 길.** 지금까지 `app-e2e` 는 깨끗한 윈도우 11 에 0.14.43 을 **새로** 깔았다. 실제 사용자는 0.14.42 를 쓰다가 올린다. 설치 훅(`src-tauri/nsis-hooks.nsh`)은 0.14.42 와 같지만 실리는 바이너리·UI·팩이 다르고, 업데이터(NSIS) 경로에서는 앱이 `exit(0)` 로 먼저 나가서 **옛 데몬이 살아 남는다**(디스크는 새 버전 · 프로세스는 옛 버전). 새 앱·새 데몬이 그 옛 상태·옛 데몬과 만나는 지점을 잰다.
2. **실제로 발행될 바이트.** `windows-build.yml` 산출물이 아니라 `release.yml` 이 만든 **draft 릴리스의 설치 파일**을 공개 발행 전에 같은 장면으로 잰다(`scripts/release-verify.py` 가 보지 않는 "이 설치본이 실제로 설치되는가").

### 입력 키 (`diag/appe2e-input.json` — 전부 선택 · 없으면 아래 기본값)

| 키 | 값 | 기본 | 뜻 |
|---|---|---|---|
| `windows_build_run_id` · `artifact` | 숫자 · 이름 | 0 · `cys-windows-x64-nsis` | 종전 그대로(제품 저장소 `windows-build.yml` 런의 아티팩트) |
| `release_tag` | `"v0.14.43"` 꼴 · `""` = 안 씀 | `""` | 있으면 **이쪽이 이긴다**(런 번호는 기록만 되고 무시). 그 태그의 릴리스(**draft 포함**)에서 이름이 `_x64-setup.exe` 로 끝나는 자산 **정확히 1개**를 받는다 |
| `scenes` | `["fresh"]` · `["upgrade"]` · `["fresh","upgrade"]` | **`["fresh","upgrade"]`** | 돌릴 장면. **키가 없으면 둘 다 돈다** — 종전처럼 fresh 만 돌리려면 `["fresh"]` 를 적는다 |
| `upgrade_from` | `"0.14.42"` 꼴 | `"0.14.42"` | 먼저 깔아 둘 공개 버전(`p1-assets` 가 받아 둔 0.14.37·0.14.41·0.14.42 는 그 파일을 쓰고, 그 밖의 버전은 공개 릴리스에서 3회까지 받는다) |
| `upgrade_mode` | `"emulate"` · `"updater"` | `"emulate"` | emulate = 하네스가 시험 대상 설치 파일을 0.14.42 업데이터와 같은 인자로 띄움(발행 전) · updater = 0.14.42 앱이 **자기 업데이터로 진짜 공개 엔드포인트에서** 받음(발행 뒤) |
| `upgrade_expect` | `"0.14.43"` 꼴 | `""` | 올라가야 할 버전. emulate 에서 비면 시험 대상 설치 파일의 버전(적었는데 다르면 측정 불능) · **updater 에서는 필수** |

문지기 잡 `appe2e-gate` 가 값의 꼴을 검사한다(태그 `v숫자.숫자.숫자` · 버전 `숫자.숫자.숫자` · 모드 둘 중 하나 · 장면은 fresh/upgrade 만 · 중복 금지). 어긋나면 사유를 찍고(로그 + notice annotation) `go=false` 로 끝낸다. `go` = 꼴 오류 없음 **그리고** (런 번호 > 0 **또는** `release_tag` 있음 **또는** 장면이 `["upgrade"]` 뿐이고 모드가 `updater` — 이때는 시험 대상 설치 파일이 필요 없다). `go=false` 인데 커밋에 `[appe2e-only]` 가 있으면 종전처럼 러너 **한 대**(장면 fresh)가 떠서 '입력 없음' 또는 문지기의 거절 사유(`inputs.gate_why`)를 결과에 적고 끝난다.

### 설치 파일을 릴리스에서 받기 (`diag/fetch-release-asset.mjs` · 스텝 `download-release-installer`)

`GET /repos/{repo}/releases?per_page=50`(최대 3쪽)에서 `tag_name` 이 같은 릴리스를 찾는다(draft 는 태그 조회 엔드포인트로 안 나온다 — 잡 토큰의 `contents: write` 로 목록에는 나온다). 같은 태그가 둘 이상이면 **draft 를 우선**하고 그 사실을 `duplicates_note` 에 적는다. 자산은 `…/releases/{id}/assets` 로 전부 읽어 접미사가 맞는 것이 0개·2개 이상이면 이름 목록을 찍고 실패한다. 내려받기는 `GET …/releases/assets/{id}` + `Accept: application/octet-stream` 이고 fetch 가 서명된 저장소 주소로 리다이렉트를 따른다 — **교차 출처 리다이렉트에는 `Authorization` 이 실리지 않는다**(Node 22.23.3·24.20.0 에서 두 포트짜리 가짜 서버로 실측: 저장소 쪽이 받은 Authorization 0건). `.part` 로 받아 크기(`size`)·`digest`(있으면)를 대조한 뒤에만 `.exe` 로 바꾼다(반쯤 받은 파일을 설치 파일로 집지 않게). 보고서 `appe2e-installer-source.json` 은 실패해도 쓰인다(`ok:false` · `error` · `http_status` · `body_head`). 토큰과 서명 주소의 쿼리는 어디에도 찍히지 않는다. `app-e2e.ps1` 은 이 보고서를 `inputs.installer_source` 로 싣고(없으면 `{"kind":"windows-build-artifact","run_id":…}`), 직접 계산한 sha256 과 보고서 값이 같은지(`file_sha256_same`)도 적는다.

### 장면 UPGRADE (`diag/app-upgrade.ps1` — `app-e2e.ps1` 이 불러 쓰는 함수 묶음 · 잡은 같은 스크립트를 `APPE2E_SCENE=upgrade` 로 돌린다)

스텝(각각 `Invoke-Step` · 실패해도 다음 스텝은 돈다 · 스텝마다 `app-e2e.json` 저장):

1. `prepare` — 네이티브 도우미 · **SAC 꺼짐 확인**(켜져 있으면 측정 불능 · 이 장면은 SAC 를 켜지 않는다) · 입력.
2. `upg-base-installer` — 공개 `<upgrade_from>` 설치 파일(크기·sha256·`SHA256SUMS.txt` 대조값).
3. `upg-install-base` — `/S` 설치 → 표식 == from · 실패 파일 없음. 이어서 `upg-facts-base`(설치 사실 기록).
4. `upg-start-base` — WebView2 정책 + 앱 기동(**`CYS_UPDATE_MANIFEST_URL` 없이** — 진짜 엔드포인트) → `uipre` 로 붙어 앱 버전 == from(안 붙으면 `--mode version` 으로 한 번 더). 여기까지 못 가면 **측정 불능**.
5. `upg-base-state` — 최대 120초(3초 간격) 설치 폴더의 `cysd.exe` 와 팩(`%USERPROFILE%\.cys\pack\.pack-version`)을 기다리며 기록: 프로세스 표 · cys 파이프 · 팩 파일 수·버전 · `.cys` 최상위 이름 · 바탕화면 · 앱 화면(`#daemon-info` 글자 · `daemon_status`). 준비 미달은 NOTE.
6. `upg-apply`
   - **emulate**: 시험 대상 설치 파일을 `%TEMP%\cys-<to>-updater-diag<6자>\cys-<to>-installer.exe` 로 복사(sha256 대조) → `ShellExecuteW(open, 그 파일, "/P /R /UPDATE /ARGS")`(tauri-plugin-updater 2.10.1 의 호출 그대로 · 현재 폴더 = 설치 폴더) → 0.3초 뒤 **앱 프로세스 하나만**(pid · 트리 kill 아님) 끝낸다. 데몬은 건드리지 않는다.
   - **updater**: `--mode attach` 로 `check_update` 를 읽어 버전이 `upgrade_expect` 가 아니면 **「공개 엔드포인트가 … 를 내놓지 않는다」로 FAIL**, 맞으면 `--mode update`(`install_update {force:true}`).
   - 공통 관찰(최대 300초 · 2초 간격 · `appe2e-upg-timeline.txt`): 임시 설치 파일 · 설치 프로세스 · 옛 앱 종료 · 표식 · 새 앱. 성공 = 표식이 to 에 닿고 **새** `cys-app.exe` 가 떠 있음.
7. `upg-after` — (a) 표식 == to · `cys-install-failure.txt` 없음 · `cys-app.exe`·`cys.exe`·`cysd.exe` 파일 버전 == to (b) 다시 뜬 앱에 `uipre`: 앱 버전 == to · UI 준비 (c) **옛 데몬과의 만남**(최대 150초 · 5초 간격 · 데몬이 to 를 두 번 연속 답하면 일찍 끝): `node cdp-update.mjs --mode uiobs`(클릭 없음)가 `#daemon-info` · `.ver-skew-badge` · 알림 · 제품의 읽기 전용 `daemon_status` 를 보고, PowerShell 은 `cysd` 프로세스(pid·부모·생성 시각·이미지·파일 버전)를 본다 (d) 잔해 `*.prev*`·`*.new.exe` 개수(직후·관찰 끝·장면 끝) (e) 팩 전후. 이어서 `upg-facts-after`.
8. `team` — 기존 `Invoke-TeamScene` 을 **업그레이드된 채 떠 있는 그 앱 그대로** 쓴다(`team.app.reused`). 표식이 to 가 아니면 돌리지 않는다(`NOT_RUN`).
9. `upg-verdict` · `upg-summary` · `upg-cleanup`(node 정리 · 설치 폴더 프로세스 종료 · WebView2 정책 제거).

`#daemon-info` 는 `daemon pid=… sock=…` 만 보여 주고(제품 `ui/src/main.ts` `start()`) 버전이 없으며 데몬이 바뀌어도 다시 쓰이지 않는다. 그래서 데몬 버전은 제품 UI 가 스스로 쓰는 `invoke('daemon_status')`(= RPC `system.identify` · 읽기 전용)의 `version` 으로 읽는다. 제품 동작(`checkVersionSkew`): 지킬 세션(역할·에이전트가 붙은 좌석)이 0 이면 앱이 **스스로 데몬을 교대**하고, 있으면 `.ver-skew-badge` 배지와 「새 버전 준비」 알림을 띄운다 — 러너에는 claude·codex·agy 가 없으니 자동 교대가 예상이지만 **어느 쪽이든 판정을 바꾸지 않는 NOTE** 다.

**판정**(`appe2e-upgrade-verdict.json` · 표준 파일 `appe2e-verdict.json`·`app-e2e.json` 의 `verdict` 에도 `{result, upgrade, team, reasons}`):
- `PASS` = 전부 참일 때만: 3·4 성공 · 6 성공 · 7(a)(b) · TEAM PASS.
- `FAIL` = 제품이 잘못했다(표식 미도달 · 앱 미복귀 · 실패 파일 · 파일 버전 불일치 · 앱 버전/UI · TEAM FAIL · updater 에서 엔드포인트가 기대 버전을 안 내놓음). 적용 관찰이 300초를 다 쓰고도 끝나지 않은 것도 여기다.
- `NOT-MEASURABLE` = 하네스가 못 쟀다(SAC 가 켜져 있음 · 입력 없음 · 0.14.42 설치/기동 실패 · 시간 부족 · emulate 에서 하네스의 `ShellExecuteW` 가 설치 파일을 못 띄움(반환값·오류 번호·그때 화면을 남긴다) · 적용 관찰을 하네스가 중간에 그만둠(작업 시간 · 관찰 루프 오류 — `apply_observe.end_kind` = `harness` · 이때는 7(a)(b) 값도 판정에 쓰지 않는다) · 다시 뜬 앱의 디버그 포트가 안 열림 · 스텝이 오류로 멈춤 · TEAM NOT_RUN).
- `NOTE`(판정 불변 · `reasons` 에 `NOTE …` 로): 옛 데몬이 150초 안에 교대되지 않음/배지가 뜸 · 5번 준비 미달 · 잔해 · TEAM 이 앱을 새로 띄움.

`upgrade`(이 장면의 1~7번)와 `team` 을 따로 적고, `result` 는 둘 중 하나라도 FAIL 이면 `FAIL`, 아니고 둘 다 PASS 가 아니면 `NOT-MEASURABLE`, 둘 다 PASS 일 때만 `PASS` 다.

### UPG-2 보강 (13차 뒤 · upgrade 장면만 · 전부 기록과 NOTE — PASS/FAIL 규칙은 그대로)

13차에서 업그레이드된 앱 화면에 「업데이트 경고 :: 새 팩 반영(init-pack) 실패 …」 알림이 떴는데 판정에는 드러나지 않았다. 원인(업그레이드 직후 팩을 쓰는 주체가 겹친다는 가설)을 **재기** 위한 관측 세 가지를 더했다.

1. **프로세스 감사** — 스텝 `upg-audit-on`(prepare 바로 뒤): `auditpol /set /subcategory:{0CCE922B-69AE-11D9-BED3-505054503030} /success:enable`(Process Creation) · `{0CCE922C-…}`(Process Termination) · 정책 값 `ProcessCreationIncludeCmdLine_Enabled=1`(4688 에 명령줄) · Security 로그가 128MB 보다 작으면 키움. 켜기 전 값은 기록하고 `upg-cleanup` 에서 되돌린다(로그 크기는 그대로 둔다). 켠 직후 **자가 시험**(낱말을 실은 `cmd.exe … exit 7` 이 4688+명령줄 · 4689 Status 0x7 로 보이는가)을 통과해야 감사를 쓴다. 스텝 `upg-proc-audit`(upg-facts-after 뒤)이 적용 30초 전부터의 4688/4689 를 읽어 제품 이미지(설치 폴더 · 업데이터 임시 폴더 · 설치 파일 · cys-app/cys/cysd)와 **그것이 띄운 것**만 남긴다 → `appe2e-upg-proc-audit.json`(원본 필드 + 10진 pid) · `.txt`(시간순 한 줄씩). 장면 끝에 한 번 더(`…-proc-audit-end.*`). 생성↔종료는 (pid · 이미지 · 시간 순서)로 짝짓는다.
   - 거기서 뽑는 표(`app-e2e.json` 의 `upgrade.proc_audit.table`): 다시 뜬 뒤의 `cys.exe` 마다 인자 · 시작 · 끝 · 초 · 종료 상태, `init-pack` 의 수 · 겹침 · 0 아닌 종료 수, 새 `cysd.exe` 시작 시각, `restore`/`drain`. 요약 한 줄 `init-pack after the upgrade (…): N process(es) [pid … 시작..끝 exit=…] overlap=…; new daemon started at …`.
   - **감사를 못 쓰면**(auditpol 실패 · 이벤트가 안 보임 · 명령줄이 안 실림) 새 앱이 보이는 순간 `diag/proc-poll.ps1` 을 두 번째 PowerShell 로 띄워 30초 동안 200ms 간격으로 `Win32_Process` 를 훑는다(`appe2e-upg-proc-poll.json` · **종료 코드 없음** · 한 간격보다 짧게 산 프로세스는 놓칠 수 있다). 그 사실은 NOTE 로 남는다.
2. **알림을 사실로** — `uiobs` 가 데몬 목표 버전을 확인해도 **최소 45초**(`--obs-min-sec`)는 계속 보고, 화면은 1초마다 읽는다. 알림마다 처음 본 시각 · 사라진 시각 · 시작 때 이미 있었는지 · 끝까지 있었는지를 적는다. 판정 파일 `facts` 에 `toasts_after_upgrade` · `update_error_toast_seen`(종류 health + 본문에 `init-pack`) · `restore_done_toast_seen`(제목 「직원 복귀 완료」) · `status_bar_pid_differs_from_daemon_status`. 업데이트 경고 알림과 상태바 pid 불일치는 **NOTE**(FAIL 아님). 요약 한 줄 `notifications after the upgrade (…)`. updater 방식도 같은 경로다.
3. **표식 파일** — `%USERPROFILE%\.cys\.pending-restore`(유무) · `.last-app-version` · `pack\.pack-version` · `pack.prev`(유무) · `.pack-staging*`(이름)을 적용 직전 · 재기동 감지 직후 · 관찰 끝 · 장면 끝에, 그리고 데몬 관찰 틱(5초)마다 바뀔 때 적는다(`upgrade.stamps`). 요약 한 줄 `stamp (.last-app-version) 0.14.42 -> 0.14.43 (first seen at …); .pending-restore …; staging leftovers …`.

요약은 14줄에서 **17줄**이 됐다(데몬 줄 뒤에 위 세 줄). 0 아닌 종료 상태의 `init-pack` 이 있으면 그것도 NOTE 다. 맥에서는 감사 도구·Security 로그를 가짜로 바꿔 모의 실행했을 뿐이다 — `auditpol`·`Get-WinEvent` 는 러너가 첫 실행이다. 13차의 실제 화면 기록을 새 로직에 다시 넣어 보면 `update-error (init-pack) seen=True; restore done seen=True; status bar pid differs=True` 가 나온다.

### UPG-3 보강 (14차 뒤 · upgrade 장면만 · 기록과 NOTE — PASS/FAIL 규칙은 그대로)

14차의 감사 기록으로 드러난 것: 업그레이드 직후 `cys.exe init-pack` 이 겹쳐 돌고 **지는 쪽이 그때그때 다르다**. 13차에서는 `--no-install-hook` 쪽(→ 업데이트 경고 알림), 14차에서는 온보딩 쪽(`init-pack` 단독 · exit 0x1 · 알림 없음)이 졌다. 온보딩이 지면 `~/.cys/.gui-onboarded` 가 안 쓰이고 제품은 **다음 기동**에 온보딩을 다시 한다 — 그 다음 기동을 잰다.

1. **표식에 온보딩 파일 둘** — `.gui-onboarded`(내용) · `.gui-onboard-attempts`(유무 + 앞 200자)를 네 시점과 데몬 관찰 틱의 변화 기록에 더했다. 표식 요약 줄 끝에 `.gui-onboarded before=… right after the relaunch=… after the page watch=… at the end=…; onboard-attempts at the end=…`.
2. **기준 상태가 "가라앉은" 뒤에 업그레이드** — `upg-base-state` 가 종전 준비 조건(데몬 + 팩) 뒤에 **최대 90초 더**(2초 간격) 기다린다: `.last-app-version` == 기준 버전 · `.gui-onboarded` == 기준 버전 · `.pack-staging*` 없음 · `init-pack`/`restore` 인자의 `cys.exe` 없음(틱마다 `Win32_Process` 한 번). `base_state.settled` · `settled_after_sec` · `settle_missing` 에 적고, 요약의 「state before the upgrade」 줄에 `settled=True after +N s` / `settled=False (…)`. 못 가라앉아도 장면은 그대로 가고 NOTE 만 남는다(「the upgrade was applied while the older app was still in its first-run work: …」).
3. **두 번째 기동** — 스텝 `upg-second-start`(TEAM 뒤 · 판정 앞). TEAM 의 정리가 앱·데몬을 끝내고 WebView2 정책을 지운 상태가 보통이므로 **콜드 스타트**다(앱이 남아 있으면 `cys-app.exe` 본체만 끝내고 데몬은 둔다). 표식 → 정책을 다시 걸고 `Start-CysApp`(매니페스트 env 없음) → `uipre` 로 버전 → `uiobs` 최소 30초(알림 · `daemon_status` · 상태바) → 그 구간의 프로세스 감사(`appe2e-upg-second-proc-audit.json/.txt` · 감사가 안 되면 대체 관측 `…-second-proc-poll.json`) → 표식. 요약 한 줄 `second start (cold start; …): app version …; cys.exe init-pack N (onboarding a, --no-install-hook b) [pid … "인자" 시작..끝 exit=…]; daemon install=…; update-error toast seen=…; .gui-onboarded 전 -> 후; stamp …; toasts: …`. NOTE 가 되는 것: 두 번째 기동의 init-pack 이 0 아닌 종료 · 업데이트 경고 알림 · 끝난 뒤에도 `.gui-onboarded` 가 목표 버전이 아님 · 앱이 안 뜸. 남은 작업 시간이 4분 미만이거나 업그레이드가 목표에 못 닿았으면 건너뛰고 NOTE 한 줄. 이 스텝에서 무슨 일이 나도 판정·요약·정리는 그대로 돈다.

`init-pack` 줄은 이제 프로세스마다 인자를 적는다(`[pid 1560 "init-pack" …]` = 온보딩 · `[pid 6880 "init-pack --no-install-hook" …]` = 팩 반영). 요약은 **18줄**이다.

### 설치 사실 기록 (`Get-InstallFacts` · 두 장면 공통 · 읽기 전용 · 판정에 안 넣음)

HKCU `…\Uninstall\cys` 의 DisplayName·DisplayVersion·UninstallString · 시작 메뉴·바탕화면 바로가기(대상 경로) · 설치 폴더 파일 수·바이트 · 세 실행 파일(크기·버전·sha256) · `cys.exe --version`·`cysd.exe --version`(각 15초 · 둘 다 데몬을 띄우지 않는다 — 근거는 스크립트 주석 · `cysd.exe` 는 인자 판정이 생긴 0.14.41 이상에서만 부른다) · 동봉 `bash.exe`·`python3.exe` 의 존재와 `--version` rc. fresh 는 `install-off` 바로 뒤 스텝 `install-facts` 에서 한 번(`appe2e-install-facts.json`), upgrade 는 0.14.42 설치 뒤(`upg-facts-base` → `…-base.json`)와 업그레이드 뒤(`upg-facts-after` → `…-after.json` — 7번 전체가 끝난 다음이다. 새 앱과 옛 데몬이 만나는 순간의 관찰을 늦추지 않으려고 7(a) 바로 뒤가 아니라 7번 뒤에 둔다).

### fresh 장면에서 달라진 것 (이 세 가지뿐 — 측정·판정·순서는 그대로)

1. 스텝 `install-facts` 하나(위) — 기존 스텝 사이에 끼지만 설치된 것을 읽기만 한다. 다만 `--version` 을 물으려고 `cys.exe`·`cysd.exe`·동봉 `bash.exe`·`python3.exe` 를 **한 번씩 실행**한다(데몬은 뜨지 않는다). 그래서 앱이 처음 뜨기 전에 이 네 파일이 한 번 읽히고 검사된 상태가 된다 — 측정값에 닿는 차이는 그것뿐이다.
2. `appe2e-summary.txt` **첫 줄**에 `installer source: …`(출처 · 이름 · 크기 · sha256 전체)가 더해졌다. 종전 10줄은 그 뒤에 글자 그대로 이어진다(2~11줄 · TEAM 줄은 12·13줄).
3. `app-e2e.json` 의 `inputs` 에 `release_tag`·`gate_why`·`installer_source` 키가 더해졌다.

잡 이름은 `app-e2e (windows-11-arm, fresh)` / `app-e2e (windows-11-arm, upgrade)` 로 바뀌었다. 결과 브랜치·아티팩트 이름은 fresh 가 종전 그대로(`…-app-e2e-windows-11-arm` · `diag-app-e2e-windows-11-arm` · 체크포인트 `…-app-e2e-pre-sac-…`), upgrade 가 `…-app-upgrade-windows-11-arm` · `diag-app-upgrade-windows-11-arm` 이다.

### 결과 읽는 순서 (upgrade)

`diag-results/<런>-1-app-upgrade-windows-11-arm` 브랜치 → `syntax-check.txt`(새 파일 `app-upgrade.ps1` 이 `ok` 인지 **먼저**) → `appe2e-upgrade-summary.txt`(14줄: 실행 · 시험 대상 설치 파일 · 0.14.42 설치 파일 · 0.14.42 설치/기동 · 업그레이드 전 상태 · 적용 방법 · 적용 관찰 · 적용 뒤 · 데몬 · 잔해/팩 · 설치 사실 · 시간 · TEAM · VERDICT) → `appe2e-upgrade-verdict.json`(`failures`·`not_measurable`·`notes`·`facts`) → `appe2e-upg-timeline.txt`(적용 2초 간격) · `appe2e-upg-screen-*.png` → `appe2e-upg-after-pre-cdp.json`·`…-ui-ready.png`(다시 뜬 앱) → `appe2e-upg-after-obs-timeline.txt`(화면: 데몬 버전·상태바·배지·알림) · `appe2e-upg-daemon-timeline.txt`(프로세스) · `appe2e-upg-after-ui-obs-*.png` → `appe2e-upg-base-state-timeline.txt` · `appe2e-upg-base-obs-*`(업그레이드 전) → `appe2e-install-facts-base.json` · `…-after.json` → `appe2e-team-*`(5차와 같음) → `app-e2e.json`(`upgrade` 아래 전부 · `steps`). 릴리스에서 받았으면 `appe2e-installer-source.json` 도.

### master 가 실행할 때 할 일

커밋 제목 끝에 **`[appe2e-only]`**(문지기 + `app-e2e` 매트릭스만 돈다). **푸시는 한 번만**(같은 브랜치의 다음 푸시는 도는 런을 취소한다). 장면마다 러너 한 대 · upgrade 는 15~25분쯤(추정 — 10차 fresh 실측: `/S` 설치 53초 · 업데이터식 설치 50초 · TEAM 6.5분).

런 A — 태그 전 · 최종 커밋의 `windows-build` 산출물 · fresh + upgrade(emulate):

```json
{
  "windows_build_run_id": 37261026255,
  "artifact": "cys-windows-x64-nsis",
  "release_tag": "",
  "scenes": ["fresh", "upgrade"],
  "upgrade_from": "0.14.42",
  "upgrade_mode": "emulate",
  "upgrade_expect": "0.14.43"
}
```

런 B — 발행 전 · draft 릴리스 `v0.14.43` 의 설치 파일 · fresh + upgrade(emulate):

```json
{
  "windows_build_run_id": 0,
  "artifact": "cys-windows-x64-nsis",
  "release_tag": "v0.14.43",
  "scenes": ["fresh", "upgrade"],
  "upgrade_from": "0.14.42",
  "upgrade_mode": "emulate",
  "upgrade_expect": "0.14.43"
}
```

런 C — 발행 뒤 · 공개 0.14.42 앱이 자기 업데이터로 진짜 엔드포인트에서 0.14.43 이 되는가 · upgrade 만:

```json
{
  "windows_build_run_id": 0,
  "artifact": "cys-windows-x64-nsis",
  "release_tag": "",
  "scenes": ["upgrade"],
  "upgrade_from": "0.14.42",
  "upgrade_mode": "updater",
  "upgrade_expect": "0.14.43"
}
```

런 B 에서 fresh 의 OFF 장면은 종전처럼 가짜 0.14.99 매니페스트(공개 0.14.42 설치 파일)로 0.14.43 의 새 실행 경로를 잰다 — 릴리스 설치 파일이라고 달라지는 것은 없다. 런 C 는 `releases/latest/download/latest.json` 이 0.14.43 을 가리킨 **뒤에** 돌려야 한다(그 전에는 `check_update` 가 비어 FAIL 로 나온다 — 측정이 아니라 순서 문제).

### 로컬 확인(맥) · 첫 실기 런에서 볼 것

로컬에서 돌린 것(원문은 저장소 밖 `W11/UPG/WORKLOG.md` 와 `W11/UPG/tools/out/`):

- `node --check` 전 `.mjs`(Node 22.23.3 · 24.20.0) · `.ps1`/`.mjs`/`.yml` 16개 파일 비ASCII·CR·TAB·제어문자 0.
- `fetch-release-asset.mjs`: 가짜 GitHub 서버(출처 둘) 시나리오 21개가 두 Node 에서 전부 통과 — 저장소 쪽 출처가 받은 요청 12건 중 Authorization 0건(같은 출처 리다이렉트에서는 실리는 것도 대조로 쟀다).
- `cdp-update.mjs`: `uiobs` 순수 함수 5개 + 가짜 DevTools 시나리오 6개 + 옛 모드 5개(version · uipre · attach · update 2갈래)의 결과 JSON 이 HEAD 판과 동일 — 두 Node 에서. `ui`·`uiteam` 모드는 로컬에서 돌리지 않았다(고친 줄 없음 — diff 는 추가뿐이다).
- 문지기 파이썬 조각: 입력 26개 + 종전 꼴 입력 9개가 HEAD 조각과 같은 `run_id`/`artifact`/`go`.
- 워크플로: YAML 파싱·이름 대조 0건(다른 잡 5개는 HEAD 와 동일) · `actionlint` 1.7.12 0건(이름 오타 돌연변이 7개는 전부 잡음).
- PowerShell: 정적 검사 2종이 `diag/*.ps1` 12개에서 0건(돌연변이 24개 전부 검출 · `-f` 줄 86개 민감도 확인) · 키/필드 이름 대조 0건 · **PowerShell 7.6.6 파서**로 12개 파일 파싱 오류 0 · 구문 나무에서 7 전용 문법 0건 · **가짜 환경 모의 실행 53개 시나리오**(UPGRADE 45 + fresh 에 더한 조각 8 — 새 PowerShell 코드를 그대로 돌리고, 윈도우 전용·이미 검증된 부품만 가짜로 바꿨다)가 전부 기대한 판정·파일을 냈다.

**Windows PowerShell 5.1 과 실제 윈도우에서는 `app-upgrade.ps1` 이 한 번도 돌지 않았다 — 러너가 첫 실기 실행이다**(맥에서 한 것은 PowerShell 7 파싱과 가짜 환경 모의 실행뿐이다). 레지스트리 읽기 · 바로가기 대상 읽기 · 진짜 `ShellExecuteW` · 진짜 프로세스 표는 모의 실행에서도 돌지 않았다. 첫 런에서는 `syntax-check.txt` 부터 본다. `app-upgrade.ps1` 이 파싱되지 않아도 fresh 장면은 영향이 없게 해 두었다(`install-facts` 스텝만 실패로 남고, upgrade 잡은 `NOT-MEASURABLE` + 사유를 표준 파일에 적는다 — 파싱 오류가 try/catch 에 잡히는 것은 PowerShell 7 에서만 확인했다).
