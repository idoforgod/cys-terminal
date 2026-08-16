# DESIGN — factory-reset (완전 초기화 · 팩토리 리셋)

> 목적: 연습(부서·워커 생성, 팀 가동, 업무, 에러 수정)으로 쌓인 **모든 사용 흔적**을
> 한 번에 제거하고 "자비스 설치 초기 단계"로 되돌린다. 앱(설치본)은 남는다 —
> 완전 제거는 docs/GUIDE-clean-reset-KR.md(수동)가 담당하고, 이 기능은 그 리셋 부분의
> **내장·원버튼·복구가능** 판이다.

## 1. 문제 (오너 보고 2026-08-16)

사용자가 연습 후 정식 사용으로 전환할 때 연습 잔재(세션 기억·부서·큐·묘비·스케줄·
장기기억·훅)가 살아 있어 충돌한다. 앱 안에서 "초기화해줘"라고 시켜도 부서·상태가
남는다(GUIDE-clean-reset-KR.md 서두가 기록한 실사용 불만과 동일). 수동 가이드는
생초보에게 부담이고, `rm -rf ~/.cys`는 오너 배치 파일(apple-notary.env·
hostinger-ftp.env·license.json)까지 파괴한다.

## 2. 원칙 (기존 교리 계승)

1. **격리(quarantine), 즉시 삭제 아님** — 전 항목 `~/.local/state/cys-trash/
   factory-reset-<UTC>/` 로 mv + `manifest.json`(원위치→격리위치 전수 기록). 복구 =
   역방향 mv 또는 **`cys factory-reset --undo <격리폴더>`**(제품 내 복구 경로 — "되돌릴 수
   있습니다"라는 고지는 실행 수단이 있어야 참이다). 영구 소거는 기존 유일 경로(`cys-dept reap`
   의 TTL sweep — 조직이 다시 가동돼야 돌기 때문에 "14일 후 자동 소거"로 **단정하지 않는다**).
   ("삭제는 전부 mv로 통일" — javis_org.py 교리)
2. **오너 데이터 보존** — `~/.cys` 는 통째 삭제 금지. **알려진 항목 열거**로만 격리하고
   미등록 파일(오너 배치 추정: *.env 등)은 제자리 보존·보고. license.json(+.minisig)은
   기본 보존, `--purge-license` opt-in 격리.
3. **정지 확인 후 이동** — launchd bootout/unload(★KeepAlive라 kill보다 먼저) → cysd 전
   프로세스 TERM→대기→KILL → **전멸 실측 확인이 하드 게이트**(살아있는 데몬 밑에서 DB를
   옮기지 않는다). ★plist **파일 삭제는 격리 성공 후**다(P0-6) — 정지 후 격리가 실패하면
   자동시작 등록만 잃은 채 "실패했으니 원래대로"라는 오해를 남기기 때문이다. 실패·중단
   경로는 `stop_side_effects_note()` 로 이미 일어난 비가역 부수효과를 반드시 고지한다.
3b. **사전 점검**(P0-6) — 격리 목적지(`cys-trash`)가 디렉토리이고 쓰기 가능한지를 **계획
   단계에서** 실측한다(`trash_root_ready`). 부적격이면 모달을 띄우지도, 데몬을 건드리지도 않는다.
4. **자기 살해 방지** — `CYS_SURFACE_ID` 환경(=cys surface 안)에서 CLI 실행 거부.
5. **쓰기 0 프리뷰 + 승인 전 전량 공개**(P0-2) — `--plan`(pack-plan 규약). GUI 확인 모달은
   집계가 아니라 **경로**를 보여준다: ①사용자 폴더 밖(`outside_state`) 항목을 맨 앞에 강조
   ②주요 항목 상위 N건 ③`report_only`(자동 정리 안 함) ④이전 중단 흔적 ⑤실행 중 세션·부서 수와
   "저장 신호 없이 즉시 종료" 고지. 프리뷰는 `spawn_blocking`(창 굳음 0) + 계산 중 토스트.
5b. **재기동 펜스(센티널)** — 정지~격리 구간에 `~/.local/state/.cys-factory-reset-in-progress`
   (`<ts> <pid>`)를 세우고 CLI autostart·GUI ensure_daemon 이 이를 보고 스폰을 보류한다.
   ★부트 체인 불가침: 판정은 **fail-open**(TTL 15분 초과·기록 pid 사망이면 무시하고 즉시 청소)
   이라 리셋이 죽어도 다음 기동을 영구히 막지 않는다. cysd 기동 자체에는 넣지 않는다(최소 침습).
   CLI 실집행은 GUI 앱(cys-app) 생존 시 거부한다 — 앱이 데몬을 되살리기 때문(프리뷰는 허용).
6. **입력 확인** — GUI: 고정 문구 "완전 초기화" 타이핑(purgeconfirm 규약·자동교정 차단).
   CLI: 대화식 같은 문구 입력, `--yes` 로 생략(스크립트용).
7. **팩 스크립트 무의존** — 리셋은 팩이 깨진 상태에서도 돌아야 하는 최후 수단이므로
   코어는 Rust lib(`src/factory_reset.rs`)에 있고 CLI·GUI가 같은 구현을 소비한다
   (부서 purge가 javis_org.py에 위임하는 것과 의도적으로 다른 결 — 근거가 위 문장).

## 3. 대상 인벤토리 (2026-08-16 소스·실기기 전수 조사 근거)

격리(Quarantine):
- `~/.cys/`: pack, pack.prev, .pack-staging*, .pack-download, .pack-journal,
  .pack-accepted.json, .pack-apply.lock, .pack-reinject-pending.json, pack-dept-*,
  claude, claude-*, state, state-generations, state-harness, _round, transfers,
  depts.json(+.lock), dept-catalog.json(+.lock), dept-missions, dept-snapshots,
  accounts.json, policy.json, profile.json, approvals.json, .approval-secret,
  .master-bootstrapped*, .gui-onboarded, .last-app-version, .pending-restore,
  ime-debug, allow-app-mouse, url-allow-hosts, harness-creator
- `~/.local/state/`: cys, cys-dept-* (등록·고아 불문. cys-trash 제외 — 격리 목적지)
- 프로젝트 작업기억: `~/_round`, `${CYS_ROOT:-~/Desktop/CYSjavis}/_round`, 그리고 그
  둘의 `_round/ACTIVE_PROJECT` 가 가리키는 프로젝트의 `_round` — **_round 하위
  디렉토리만**, D1a식 게이트(realpath가 $HOME 아래·$HOME 자신 아님·보호 루트 아님·
  심링크 루트 거부) 통과 시에만. 작업 폴더 본체는 절대 건드리지 않는다.
- macOS GUI 층: `~/Library/WebKit/com.cysjavis.terminal`,
  `~/Library/Caches/com.cysjavis.terminal`, `$(getconf DARWIN_USER_CACHE_DIR)/
  com.cysjavis.terminal`, `defaults delete com.cysjavis.terminal`(+plist 격리)
- Windows 층: `%LOCALAPPDATA%\cys`(메인+부서 데몬 상태 — cysd 슬러그 규약),
  `%LOCALAPPDATA%\com.cysjavis.terminal`(WebView2 데이터)
- ★GUI 웹층은 **이연(best_effort) 등급**이다: 실행 중인 자기 앱이 점유하면 Windows 는 rename 이
  항상 실패한다(공유 위반). 이 실패는 `failed`(부분 실패)가 아니라 `deferred` 로 보고하고
  "앱 종료 후 재실행하면 정리됨"을 안내한다 — 맥 정상/윈도 매번 부분실패 비대칭 제거.

해제(Strip — 격리가 아니라 외부 등록의 외과적 제거):
- 대상 프로필: `~/.claude*` **+ `CLAUDE_CONFIG_DIR`**(등록기 preflight 가 최우선으로 훅을 심는
  경로 — 빠지면 그 프로필의 훅만 깨진 채 남는다 · P1-1 ④). `~/.cys` 안의 격리형 프로필은 제외
  (파일째 사라지므로 무의미).
- 심링크 settings: 거부가 아니라 **실파일 추종**(대상이 홈 아래 일반 파일일 때만 · P1-1 ①).
  도트파일 저장소 구성이 흔하고, 거부하면 그 사용자는 리셋 후 매 세션 훅 오류를 본다.
  홈 밖·비파일·끊어진 링크는 기존대로 거부(설정 클로버 금지).
- 백업 위치: `<trash_dir>/settings-backups/<프로필>.settings.json` — 프로필 옆에 백업을
  새로 만들지 않는다(P1-1 ③: "cys 흔적은 격리 폴더 한 곳"). 원본 권한(0444 잠금)은 복원한다(⑤).
- needle 은 **경계 일치**(`<base>/pack` 정확 또는 `pack-dept-*`) — 사용자가 만든 `~/.cys/pack-notes`
  훅까지 해제하던 A8 대칭 역전을 막는다(P1-1 ②).
- `~/.claude*/settings.json`: 명령 문자열이 **격리로 사라질 경로**(`<home>/.cys/pack`·부서팩,
  그리고 `--purge-local` 일 때만 `<home>/.cys/local`)를 가리키는 훅만 제거 — 보존되는 local 을
  가리키는 훅은 리셋 후에도 유효하므로 건드리지 않는다(사용자 훅 불가침·merge_desired_hooks 의 역연산),
  statusLine 이 cys-statusline 이면 제거하되 `CYS_PREV_STATUSLINE=<prev>` 보존분은
  원복. 변경 시에만 `.bak-factory-reset` 백업 후 write_atomic.
- `~/.claude*/skills/<n>`: 대상이 `~/.cys/pack/skills` 인 **심링크만** 제거(실디렉토리
  불가침 — preflight 심링크 파밍의 역연산).

정지·등록해제: launchd `com.cysjavis.cysd`(bootout+unload+plist 삭제 — 다음 앱 실행이
register_if_absent 로 재등록 = 신규 설치 동등), Windows schtasks `cysd` 삭제, cysd 전
프로세스(이름 `cysd`, Windows 는 `cysd.exe`·세대 잔재 `cysd.prev*.exe` 포함 — 놓치면 "전멸 확인"이
거짓이 된다). launchd 해제는 **plist 존재와 무관하게 라벨 대상 bootout** 후 `launchctl list` 미적재를
실측해야 한다(plist 만 지워진 기계에서 KeepAlive 부활 실사고 방지).

임시 소거(rm — 캐시 등급·OS 관리 영역 한정): `$TMPDIR`의 cys-paste/, cys-ime.log,
cys-pack-guard/, cys_chan_test_*, cycverftest-*, cso_watch_prev.*,
.cys-guard-timeout-absent-*.

보존(Keep) + 보고: 라이선스(+서명), `~/.cys/local`(사용자 오버레이), `~/.cys` 미등록 파일 전부
(오너 배치 — `claude-` 접두 규칙은 **계정 디렉토리만** 겨냥하므로 `claude-*.env` 같은 파일은 보존 · P2-2 ④),
`cys-trash` 기존 보관본(화면에 명시 열거 · P2-2 ⑤),
`~/.local/state/claude`(Claude Code 소유 — cys 아님), cys-trash 기존 항목.

보고만(ReportOnly — 자동 수정 금지): `~/.zshrc` cys alias 블록(오너 저작),
`~/.codex/config.toml`·`~/.gemini/*`의 cys 경로 항목(서드파티 설정 · P2-2 ①), 임의 프로젝트의
CLAUDE.md/.mcp.json/.vibecoding(작업 폴더 불가침 — 경로 안내만), 프로필에 남은 옛 settings 백업
개수(되돌리면 죽은 훅이 부활 · P1-1 ③), `--purge-local` 시 끊길 외부 스킬 링크 수(P2-2 ③),
프로젝트 `_round` 목록(P0-3).

오너 판단 설정 사본(P2-2 ②): `policy.json`·`accounts.json`·`profile.json` 은 재온보딩이 재생성하지
않으므로 격리 전 `<trash_dir>/owner-settings/` 로 따로 복사하고 REPORT.txt 에 위치를 적는다.

## 4. 종료 상태 계약

리셋 후 = **설치 직후**: 앱·심링크만 존재, `.gui-onboarded` 부재 → 다음 GUI 실행이
온보딩(팩 시드·훅 등록·launchd 등록)을 처음처럼 수행. CLI 재구성은 `cys init-pack` +
`cys daemon install`. 에이전트 계정 로그인(~/.cys/claude*)은 격리되므로 재로그인 필요
— 프리뷰·완료 화면에 명시.

## 5. 표면

- CLI: `cys factory-reset [--plan] [--yes] [--json] [--verbose] [--purge-license] [--purge-local]`
  `[--purge-round]`(프로젝트 작업기억까지) · `--undo <격리폴더>`(복구, `--plan` 과 조합 가능)
  exit: 0=성공(또는 --plan) · 1=부분 실패(failed 존재)·확인 문구 불일치·정지 실패·격리 미진입
  (quiescent 게이트 거부) · 2=가드 거부(surface 내부 실행·GUI 앱 실행 중·홈 미해석·--json 단독·
  격리 폴더 부적격).
- GUI: topbar "완전 초기화" 버튼 → `factory_reset_preview` → 타이핑 확인 모달
  (resetconfirm.ts 순수 판정) → `factory_reset_execute`(`reset-progress` 단계 이벤트)
  → 완료 모달 → `factory_reset_quit_app`(app.exit). **재시작(app.restart)을 쓰지 않는 이유**:
  single-instance 락 레이스(install_update 주석의 재활성화 경고)를 피하고, 종료가 라이브
  WebView·cfprefsd 의 재기록 창을 닫는 유일한 확실한 방법이다.
- GUI 완료 래치: 격리가 시작된 순간부터 이 앱 프로세스는 반쪽 상태이므로 데몬·부서를 만드는
  모든 경로(재시작·스큐 교대·＋부서·팔레트·업데이트·팩 설치·새 pane·분할·새 워크스페이스)를
  종료 전까지 영구 차단한다(무반응 아님 — 항상 사유 토스트 · P1-3).
- 리셋 중 UI 정합(P1-3): ①부트 재시도 루프가 `reset_in_progress` 를 보고 "로그인 항목 허용"
  대신 초기화 안내를 낸다 ②확인 모달이 떠 있으면 전역 파괴 단축키(⌘W/T/D)를 관통시키지 않고
  Esc 로 취소된다 ③종료 직전 `localStorage.clear()` 로 화면 저장값을 직접 비운다(이연 여부와
  무관하게 다음 실행이 초기 화면) ④리셋 후에는 "직원 복귀" 복원 토스트를 띄우지 않는다.

## 6. 실패 모델

- 데몬 전멸 실패 → **격리 진입 전 중단**(부분 이동 0) + 부수효과 고지(정지·등록해제는 이미 됨).
- 개별 mv 실패 → failed 기록·계속(리포트에 정직 표기, exit 1).
- **저널 기록 실패 → 그 항목을 옮기지 않는다**(P0-5). 복구 지도를 못 남기면 동명 항목의
  원위치를 영영 알 수 없으므로 이동 자체를 포기하는 것이 유일하게 안전한 선택이다.
- **manifest 기록 실패 → Err 가 아니라 `failed` 기록 + 나머지 단계 계속**(P0-5). 실패 시점이
  전 항목 rename **이후**라 "격리 미진입"은 거짓이고, Err 전파는 훅 해제까지 건너뛰어
  사라진 팩을 가리키는 훅을 남겼다. `manifest_written=false` 로 보고하고 journal 을 지도로 안내.
- 훅 strip 실패(파싱·심링크) → 해당 프로필 스킵·보고(설정 파일 클로버 금지).
- 재실행 안전: 이미 없는 항목은 건너뛴다(가이드 §마지막 FAQ와 동일 계약).
- 중도 사망: `journal.ndjson`(이동 **직전** 선기록·fsync)이 유일한 복구 지도로 남는다 —
  manifest 는 완료 요약이고, 동명 항목(`_round` 최대 3곳)은 저널 없이는 역산이 불가능하다.
- 격리 도중 데몬 부활: 사후 재실측해 `revived_warning` 으로 보고한다(침묵 금지).
- **결과 영속**(P0-4): `<trash_dir>/REPORT.txt` 에 이동/이미없음/이연/실패·보존·미정리 안내·
  복구 절차를 항상 쓴다. 화면 토스트는 60초 뒤 사라지고 모달을 닫으면 실패 내역이 소멸하므로,
  디스크의 이 파일이 사후 확인의 단일 진실이다. 완료 화면은 건수를
  `이동 N · 이미 없음 M · 이연 K · 실패 L` 로 **분해**해 예고 건수와의 차이를 설명한다.

## 7. 트립와이어 (실사고 재발 방지 핀)

- plan 이 $HOME 자신·보호 루트·미등록 ~/.cys 파일·(기본값) license 를 절대 포함하지
  않음을 단언하는 회귀 테스트(src/factory_reset.rs).
- GUI 커맨드 소스 구간에 `rm -rf`·`--purge-workdir` 문자열 부재 + lib 위임 단언
  (src-tauri include_str! 트립와이어 — purge D2a 관례).
- 센티널 fail-open 회귀(TTL 초과·고아 pid·형식 불량은 차단하지 않고 자동 청소, RAII Drop 해제)
  — 잔존 센티널이 데몬 기동을 영구히 막는 '전 pane 사망' 등급 사고의 방지 핀.
- **센티널 배선 트립와이어**(P0-1): CLI·GUI 실행 경로에 `ResetSentinel::arm()` 이 존재하고,
  stop 단계가 `write_sentinel()`·`remove_file(plist)` 를 **하지 않음**을 소스로 단언한다.
  이 결함(무장은 있는데 해제자가 안 도는 배선)이 실제로 시뮬레이션에서 발견됐다.
- 저널 실패 시 이동 금지 · manifest 실패 시 계속 진행 + REPORT.txt 생성 · undo 왕복(원위치
  점유 시 덮어쓰기 금지) · 프로젝트 `_round` 기본 보고/opt-in 격리 · 격리 폴더 사전 거부.
- local 오버레이 보존/격리와 훅 strip 범위의 **대칭** 회귀(보존 시 훅 잔존·격리 시 훅 제거).
- Windows statusLine 원복(`bash "C:/…"` 형식) 회귀.

## 8. 오케스트레이션 안전(LLM Orchestration 앵커)

`cys factory-reset` 은 **오너 전용**이다. 에이전트(마스터·워커·리뷰어)가 실행할 수 있으면
전 데몬 종료 + 팩(guard.sh 포함)·부서·대화기억 전량 격리가 무승인으로 일어난다.
3중 방어:
1. CLI 자체가 cys surface 안(`CYS_SURFACE_ID`) 실행을 거부(exit 2).
2. `guard.sh` 의 `CYS_OWNER_ONLY_SUBS` — STRICT·LOOSE **모드 무관 DENY**(R-02b).
   `env -u CYS_SURFACE_ID` 류 우회도 명령 문자열 판정이라 함께 막힌다.
   자기잠금 없음 — 오너의 경로는 GUI 버튼과 pane 밖 터미널이다.
3. `approval_risk::derive_risk` 는 미지 요청을 fail-closed HighRisk 로 판정하므로 승인 우회 불가
   (별도 denylist 추가는 하지 않았다 — "초기화" 같은 흔한 낱말을 전역 denylist 에 넣으면
   무관한 요청까지 자동화가 죽는 샷건 서저리다).

## 9. 이 기능 밖의 동반 수리(P2-2 ⑥)

`cysd` 스케줄러는 상태 파일이 없는 **첫 가동**에서 전 주기 잡의 만기가 동시에 성립했다
(`last_fired=0`). 마스터가 있으면 6h·24h·주간 잡이 한꺼번에 주입돼 갓 각성한 마스터를 큐로
덮치고(폭주 결함군), 없으면 `if_absent: skip` 으로 전부 소인돼 다음 주기까지 침묵한다.
완전 초기화 직후가 정확히 그 상태라 이 브랜치에서 함께 고쳤다 — 첫 가동에는 기준 시각을
`now` 로 시드한다(`src/bin/cysd/schedule.rs`). ⚠**적용 범위 고지**: 이 수정은 리셋 기계뿐 아니라
**모든 신규 설치**의 첫 부팅에 적용된다(첫 주기 1회가 늦어질 뿐이라 실패 방향은 무해).
