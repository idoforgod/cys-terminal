# 완전 초기화(팩토리 리셋) — 3라운드 가상 시뮬레이션 결과·수정 실행계획

> 생성: 2026-08-16 · 대상: 브랜치 `feat/factory-reset` 워킹트리
> 방법: 12개 시나리오(정상 실사용 4 · 이상 엣지 4 · 리셋 이후 4) 코드 추적 시뮬레이션 →
> 발견 96건(중대 57건) → 반박 전담 적대 검증 24건 실행
> → 확정 20건 · 반박 4건.
> ⚠ 검증 상한(24건) 때문에 중대 발견 중 33건은 **미검증**이다 — 이 문서의
> 확정 목록은 하한이지 전부가 아니다. 수정 착수 전 미검증분 재확인 필요.

# 완전 초기화(팩토리 리셋) — 결함 병합·수정 실행계획

작성 기준: `feat/factory-reset` 미커밋 워킹트리 실코드 재확인(2026-08-16). 아래 모든 file:line 은 내가 직접 열어 확인한 값이다. 코드는 수정하지 않았다.

우선순위 정의
- **배포 차단(P0)** — 이 항목이 남아 있으면 기능을 출하할 수 없다. 사용자가 **동의 없이 데이터를 잃거나, 거짓 상태 보고를 받는** 계급.
- **배포 전 필수(P1)** — 같은 릴리스에 반드시 포함. P0와 병렬 진행 가능. 고장은 나지만 데이터 손실이 아니거나, 조건부로 발생.
- **배포 후 개선(P2)** — 릴리스 노트 고지로 넘길 수 있는 인지·정합 결함.

원 결함 41건(번호 20 + 무번호 21) → **12 클러스터**로 병합. 중복 제거 내역은 각 클러스터 머리에 표시했다.

---

## P0 — 배포 차단 (6건)

### P0-1. 리셋 센티널이 프로덕션에서 무장되지 않는다 (RAII 계약 미배선)
병합: #1 + 무번호 "센티널 파일이 성공 후에도 남는다" + 무번호 "자기 흔적 센티널" (동일 뿌리 3건)

**피해** 리셋이 정지·격리 게이트에서 실패하면 데몬은 이미 죽었는데 UI는 `resetCompleted` 래치를 안 걸어 앱이 정상처럼 보인다. `↻ 재시작`을 누르면 `완전 초기화가 진행 중 — 데몬 기동을 보류한다`만 뜨고 최대 900초 복구가 막힌다. 성공해도 `~/.local/state/.cys-factory-reset-in-progress` 가 남아 "설치 초기 상태" 계약을 자기 손으로 깬다.

**근거** `src/factory_reset.rs:658` 이 `write_sentinel()` 을 직접 호출. 해제자 `clear_sentinel`(565)은 `impl Drop for ResetSentinel`(579-583)에서만 불린다. `ResetSentinel::arm()`(574)의 유일한 호출부는 테스트뿐 — `src/bin/cys.rs:4440-4594`, `src-tauri/src/main.rs:2916-2964` 어디에도 없다. 소비자는 `src/bin/cys.rs:1552`, `src-tauri/src/main.rs:1998`. `docs/DESIGN-factory-reset.md §7` 은 "RAII Drop 해제"를 트립와이어로 선언한다.

**수정 지시**
1. `src/factory_reset.rs:658` 의 `write_sentinel();` 삭제(주석 ★A3는 arm 배선부로 이동).
2. `src/bin/cys.rs::run_factory_reset` — 확인 문구 통과 직후, `stop_daemons_and_unregister` 호출 **직전**에 `let _sentinel = cys::factory_reset::ResetSentinel::arm();`.
3. `src-tauri/src/main.rs::factory_reset_execute` — `spawn_blocking` 클로저 최상단(plan 생성 직후)에 동일 1줄.
4. `write_sentinel`/`clear_sentinel` 을 `arm`/`Drop` 외부에서 못 부르게 private 유지 확인(현재 이미 private).

**회귀 테스트**
- `factory_reset.rs` 유닛: `arm()` 후 `reset_in_progress()==true`, 스코프 이탈 후 파일 부재.
- 신규 트립와이어: `include_str!("../bin/cys.rs")` 와 `src-tauri` 소스에 `ResetSentinel::arm()` 문자열이 존재함을 단언(purge D2a 관례와 동일한 소스 트립와이어). 배선이 빠지면 CI가 잡는다.
- e2e(temp home): 성공 경로 종료 후 센티널 파일 0건.

**2차 파급** 부트 체인: 센티널은 `cysd` 자체 기동에는 안 걸리고 CLI autostart(cys.rs:1552)·GUI ensure_daemon(main.rs:1998)만 보므로 arm 배선은 부트 표면을 넓히지 않는다 — 다만 Drop 이 프로세스 kill(SIGKILL) 시엔 안 돌므로 fail-open TTL 900초는 **반드시 유지**할 것.

---

### P0-2. GUI 확인 모달이 격리 경로·report_only·영향 범위를 하나도 보여주지 않는다
병합: #2 + #15 + #11 + #12(고지 축) + #10(14일 축) + 무번호 "모달 격리 경로가 실제와 다름"×2 + 무번호 "보존 N건 오표기" + 무번호 "프리뷰 동기 커맨드 창 굳음"×2 (총 8건)

**피해** 사용자가 보는 것은 `격리 대상 34건 · 총 2.2GB — 모든 부서·세션·대화기억·작업기억·설정이 초기화됩니다` 한 줄뿐이다. 그 34건 안에 `~/Desktop/CYSjavis/cys-homepage/_round`(실측 279MB, 사용자 프로젝트 폴더 **안**)가 들어 있는데 경로가 한 줄도 안 뜬다. CLI `--plan` 은 34줄 전부 찍는다 — 같은 기능에서 GUI 사용자만 눈이 가려진 정보 비대칭이다. 여기에 ①`report_only` 3건(zshrc·codex·프로젝트 파일)이 통째로 버려지고 ②"지금 돌고 있는 세션이 저장 없이 즉사한다"는 고지가 없고(덜 위험한 `↻ 재시작` 툴팁은 drain·미저장 손실을 고지한다) ③모달의 복구 경로는 프리뷰 시각 stamp라 실제 격리 폴더와 **항상 다르다** ④"약 14일 후 소거"가 툴팁·CLI·부서삭제 모달엔 있는데 정작 확정 버튼 화면에만 없다.

**근거** `src-tauri/src/main.rs:2899-2906` 프리뷰 반환에 격리 항목 배열 자체가 없음(`quarantine_count`·`total_bytes`·`trash_dir`·`kept`·`strip_profiles`·`report_only`). `ui/src/main.ts:5134-5140` 의 타입에 `report_only` 필드가 없어 백엔드가 보낸 값을 버림. `ui/src/resetconfirm.ts:35-52` `resetNoticeLines` 는 집계 5개 필드만 받음. 대비: `src/bin/cys.rs:4478-4495`. stamp 불일치: `src/factory_reset.rs:316-317`(build_plan 이 매 호출 새 UTC stamp) vs `src-tauri/src/main.rs:2932`(실행이 계획을 새로 생성). 프리뷰 동기 실행: `factory_reset_preview`(2891)는 `fn`, `factory_reset_execute`(2916)는 `async fn + spawn_blocking` — 비대칭. `ui/index.html:22` vs `:23` 툴팁 고지 격차.

**수정 지시**
1. `factory_reset_preview` 를 `async fn` + `tokio::task::spawn_blocking` 으로 바꾸고(실행 커맨드와 대칭), 반환 JSON에 추가:
   - `quarantine: [{path,label,size_bytes,outside_state:bool}]` — `outside_state` = `~/.cys`·`~/.local/state`(윈도 `%LOCALAPPDATA%\cys`) 밖 경로 표식.
   - `live_sessions`, `dept_count` (기존 `list_depts`·`live_session_count` 재사용).
   - `stamp` 는 **내보내지 말 것**(아래 3번으로 대체).
2. `ui/src/main.ts:5134-5153` 타입·전달을 확장하고, `ui/src/resetconfirm.ts` 에 `resetHighlightLines(info)` 추가:
   - 첫 줄에 `지금 실행 중인 세션 N개·부서 M개가 저장(drain) 없이 즉시 종료됩니다 — 중요한 작업은 먼저 마무리하세요.`
   - `outside_state` 항목은 **별도 강조 블록**으로 전 경로·용량 노출(`~/Desktop/CYSjavis/cys-homepage/_round (279 MB)`).
   - 전체 목록은 스크롤 영역(크기 내림차순, 상위 5건 펼침 + `…외 N건`), 카테고리 소계(대화기억·DB / 작업기억 / 팩·조직 / GUI 저장값).
   - `report_only` 를 `자동 정리하지 않음 — 직접 확인하세요` 블록으로 그대로 표시.
   - 복구 줄에서 구체 경로를 빼고 `~/.local/state/cys-trash/factory-reset-<실행시각>` 형태 + `약 14일 후 자동 소거될 수 있습니다`(부서 삭제 모달 `ui/src/main.ts:4963` 문구와 동일 어휘)로 교체. 실제 경로는 완료 모달에서만 확정 고지.
   - `keptCount` 문구를 `라이선스·오버레이 포함 보존 N건` 으로 정정(현재는 라이선스·`~/.cys/local` 까지 "직접 넣은 파일"로 합산됨).
3. 모달 표시 전 스피너 토스트(`stickyToast("reset-preview", …, "격리 대상 계산 중…")`) 추가 — 콜드 캐시에서 수 초 굳는 구간을 사용자에게 설명.
4. `ui/index.html:23` 툴팁에 `실행 중인 세션은 저장 없이 즉시 종료됩니다` 추가(재시작 툴팁과 대칭).

**회귀 테스트** `ui/src/resetconfirm.test.ts` 확장 — (a) `outside_state` 항목이 있으면 강조 블록이 반드시 포함, (b) `report_only` 비어있지 않으면 문장화, (c) `14일` 문자열 존재, (d) 세션 수 0일 때 첫 줄이 오도하지 않음. Rust 쪽: `factory_reset_preview` 반환 키 집합 스냅샷 테스트(누락 회귀 차단).

**2차 파급** Windows: `%LOCALAPPDATA%\cys` 트리에 PortableGit·embeddable Python 전개본이 포함돼 `total_bytes` 대부분이 **프로그램 파일**이므로, 강조 블록 도입과 함께 "프로그램 파일 포함" 주석을 윈도우 분기에 붙이지 않으면 용량 오인이 그대로 남는다.

---

### P0-3. `_round` 범위가 자의적이고, 프로젝트 폴더 안 작업기억이 무고지로 이동된다 — 아키텍트 판정 필요
병합: #13 + #12 + #7(강조 축)

**충돌** #13은 "워크스페이스 하위 15곳(122.5MB)이 남으니 순회를 추가해 **더 많이** 격리하라", #12는 "프로젝트 폴더 안 `_round` 격리가 가이드 단언(`일반 폴더 파일은 안 지워진다`)과 정반대이니 **고지·축소**하라"로 정면 충돌한다.

**판정(아키텍트)** — **범위를 넓히지 말고, 순회는 채택하되 결과를 격리가 아니라 고지로 쓴다.**
- 기본 격리: `$HOME/_round`, `<workspace_root>/_round` **만**.
- `ACTIVE_PROJECT` 포인터가 가리키는 프로젝트의 `_round` 는 **기본 격리에서 제외**하고 `report_only` 로 강등.
- `round_candidates` 에 workspace_root 직하(+1단계) 순회를 추가하되, 발견된 모든 프로젝트 `_round` 를 `report_only` 로 열거(`이 N곳은 남습니다 — 필요 시 직접 정리`).
- 전면 격리는 `--purge-round` opt-in(CLI) + GUI 모달 체크박스(경로 전량 표시)로만.

근거 3가지: ①격리본은 `cys-dept reap` 가 mtime 14일 초과 시 `rm -rf` 한다(`cysjavis-pack/bin/cys-dept:1175-1185`) — 동의 없이 옮긴 사용자 저작물이 **영구 소거**된다. ②`~/.cys/local` 을 기본 보존으로 되돌린 A8 결정(`src/factory_reset.rs:74-80`)과 동일한 논리다 — 사용자 저작 영역은 opt-in. ③"왜 이 프로젝트만 지워지고 저 15곳은 남지"라는 설명 불가능한 비대칭이 사라진다(둘 다 report_only 로 대칭).

**수정 지시** `src/factory_reset.rs:292-312 round_candidates` 를 `(quarantine: Vec<PathBuf>, report: Vec<PathBuf>)` 반환으로 변경; `build_plan:385-394` 에서 report 분기 추가. `ResetOptions` 에 `purge_round: bool` 추가 → `src/bin/cys.rs` 플래그, `src-tauri` 커맨드 인자, `ui` 체크박스. `docs/GUIDE-clean-reset-KR.md:28-30` 의 "일반 폴더 파일은 안 지워진다" 단언과 `docs/DESIGN-factory-reset.md §3` 인벤토리를 이 판정에 맞춰 동시 수정.

**회귀 테스트** temp 홈에 `home/_round`·`workspace/_round`·`workspace/projA/_round`·`workspace/projB/_round` + `ACTIVE_PROJECT`→projA 를 심고: 기본 옵션에서 quarantine 2건·report 2건(projA 포함), `purge_round=true` 에서 quarantine 4건. `ACTIVE_PROJECT` 가 홈 밖을 가리키면 여전히 게이트 거부(`outside-home`).

**2차 파급** 오케스트레이션 앵커: `_round/SESSION_STATE.md`·`tasks/` 가 남으면 재온보딩 후 옛 세션 상태가 되살아날 수 있으므로, report_only 문구에 "여기 남은 SESSION_STATE.md 가 새 조직과 충돌할 수 있습니다"를 반드시 포함해야 기능 목적(#13의 문제의식)이 보전된다.

---

### P0-4. 부분 실패인데 화면은 "완전 초기화 완료" — 실패 내역이 모달 뒤에 가려지고 앱을 끄면 영구 소멸
병합: #8 + #3 + #5(보고 축) + 무번호 "예고 34건 vs 완료 31건 불일치"

**피해** 격리 실패·훅 해제 실패가 있어도 정면에 뜨는 것은 제목 `완전 초기화 완료` 모달과 `사용 흔적 N건이 격리 보관되었습니다`. 실패 토스트는 반투명 오버레이 뒤에서 흐리고 클릭 불가(클릭하면 모달이 '아니오'로 닫힌다). 안내대로 `앱 종료`를 누르면 60초 TTL 토스트도, 메모리에만 있는 알람 이력도 함께 사라진다. 초보는 "깨끗해졌다"고 믿고 종료하지만, 정작 초기화하려던 증상(부서 잔존·Claude Code 훅 오류)이 그대로 남는다. 예고 건수(34)와 완료 건수(31)의 차이도 아무도 설명하지 않는다.

**근거** `ui/src/main.ts:5185-5207` — `rep.ok===false` 여도 완료 모달을 무조건 표시하고 본문은 `deferred` 만 반영, `rep.failed`·`rep.revived_warning` 제외. `ui/src/style.css:539-541`(overlay z-index 1000) vs `:492-494`(#toasts z-index 99). `ui/src/toastttl.ts:16`(60초)·`:46`(`BANNER_ON_EXPIRY_PREFIXES=["purge-fail-"]` — `reset-fail` 미포함). `ui/src/main.ts:5219` 알람 이력은 메모리 배열.

**충돌 판정** "TTL을 늘리거나 배너 접두를 추가하라"는 수정안은 `ui/src/toastttl.ts:3-6` 에 적힌 오너 요구("모든 에러 알람은 종류 불문 일정 시간 뒤 꺼진다")와 부딪힌다. → **정본은 모달 본문 + 디스크 영속**이고, 배너 보강은 보조로만 채택한다.

**수정 지시**
1. `factoryResetFlow`(main.ts:5199-5207): `rep.ok===false || rep.revived_warning` 이면 제목을 `완전 초기화 부분 완료 — 정리되지 않은 항목이 있습니다`, 본문 최상단에 실패 경로 전량 + 부활 경고를 싣는다. 확인 버튼 라벨은 `앱 종료` 유지, 부정 버튼 라벨을 `아니오` 고정값에서 `나중에`로 바꿀 수 있게 `confirmModal` 에 선택적 `noLabel` 인자 추가.
2. **디스크 영속**: `execute_quarantine` 이 `<trash_dir>/REPORT.txt`(사람용 요약 — 이동 N건·실패 목록·이연·부활 경고·report_only·복구 절차)를 항상 쓰게 한다(`src/factory_reset.rs:1028` 반환 직전). 완료 모달에 그 경로를 명시.
3. 건수 정합: 완료 모달 문구를 `예고 34건 중 이동 31건(이미 없음 2 · 이연 1 · 실패 0)` 형태로 분해해 표기. `ResetReport` 에 `skipped_absent: usize` 필드 추가(현재 `factory_reset.rs:925-927` 에서 조용히 `continue`).
4. 보조로 `toastttl.ts:46` 에 `"reset-fail"`, `"reset-revived"` 를 추가(만료 시 OS 배너 1회) — TTL 자체는 건드리지 않는다.

**회귀 테스트** `ui` 단위: 실패 배열이 비어있지 않을 때 모달 제목/본문에 실패 경로가 포함됨을 단언(순수 함수로 본문 조립기를 `resetconfirm.ts` 로 분리해 테스트 가능하게). Rust: `REPORT.txt` 가 실패·성공 양쪽에서 생성되고 실패 목록을 포함함.

**2차 파급** 부트 체인: 완료 모달을 '나중에'로 닫아도 `resetCompleted` 래치는 이미 걸려 있으므로(main.ts:5184) 데몬 소생은 계속 차단된다 — 이 래치와 P0-1의 센티널 Drop 이 서로 다른 시점에 풀린다는 사실을 문구에 반영해야 한다.

---

### P0-5. 격리 원자성 — manifest 실패가 "격리 미진입"으로 거짓 보고되고, 저널 실패는 통째로 무시된다
병합: #17 + #18 + 무번호 "중도 사망 시 journal 안내 순환" + 무번호 "중단 후 재실행이 두 번째 격리 폴더를 판다"

**피해** (a) 디스크가 꽉 차면 mv 는 공간을 안 쓰므로 2.4GB 전부가 이미 trash 로 옮겨진 뒤 manifest 기록만 실패한다. 사용자가 받는 문구는 `완전 초기화 실패(격리 미진입): manifest 기록 실패: No space left on device` — '아직 안 옮겼다'는 정반대 정보다. 복구 지도도 없고, 격리 폴더 경로도 에러 어디에도 없고, 훅 해제 루프는 실행조차 안 돼 사라진 pack 을 가리키는 훅이 그대로 남는다. (b) "중도 사망 시 유일한 복구 지도"라던 `journal.ndjson` 의 write·flush·fsync 결과가 셋 다 버려진다 — 저널 줄이 빠진 채 mv 가 진행되면 `042-_round`, `043-_round` 중 어느 것이 자기 프로젝트 279MB 인지 영영 알 수 없다.

**근거**
```rust
// src/factory_reset.rs:938-945  (주석 936-937은 "이 줄이 디스크에 닿은 뒤에만 mv 한다(순서가 계약이다)")
let _ = writeln!(journal, "{line}");
let _ = journal.flush();
let _ = journal.sync_data();
```
`src/factory_reset.rs:987-989` — `write_atomic(manifest).map_err(...)?` 가 **모든 rename 이후**에 위치해 `moved` 벡터를 폐기하고 Err 전파. `:992-1006` 훅 해제는 그 뒤라 미실행. `src/bin/cys.rs:4589-4592`, `ui/src/main.ts:5176-5180` 이 그 Err를 "미진입"으로 문장화.

**충돌 판정** `docs/DESIGN-factory-reset.md §6` 은 "격리 미진입 = 부분 이동 0"을 실패 모델로 선언한다. manifest 실패를 Ok 로 강등하면 그 모델과 충돌한다. → **모델을 고친다.** 근거: 실패 시점이 rename 이후이므로 "부분 이동 0"이 사실이 아니고, 거짓 문구가 사용자를 복구 불가능한 오판(재실행 → 두 번째 격리 폴더)으로 몬다. exit code 1과 "부분 완료" 등급은 그대로 유지해 계약의 관측 가능한 부분은 보존한다.

**수정 지시**
1. 저널 선기록을 **계약으로 강제**: `writeln/flush/sync_data` 중 하나라도 실패하면 그 항목의 `rename` 을 건너뛰고 `failed.push((path, "복구 지도 기록 실패 — 이동하지 않음"))`. 첫 실패 시 이후 전 항목 중단(fail-closed)도 옵션이나, 기본은 항목 단위 스킵을 권장한다(단일 항목 오류가 전체를 막지 않도록).
2. manifest 실패를 `failed` 에 담고 `Ok(report)` 로 반환. 훅 해제·심링크 제거·temp sweep 는 **반드시 이어서 실행**한다.
3. `ResetReport` 에 `manifest_written: bool` 추가 → CLI/GUI 문구: `N건이 <trash_dir> 로 이동됐으나 복구 지도(manifest.json)를 쓰지 못했습니다 — journal.ndjson 이 유일한 지도입니다`.
4. **중단 감지**: `execute_quarantine` 진입 시 `trash_root` 하위에 `manifest.json` 없는 `factory-reset-*` 폴더가 있으면 `report_only` 에 `이전 초기화가 중단된 흔적: <경로>(복구 지도 journal.ndjson)` 를 싣고, 프리뷰·완료 화면 양쪽에 노출한다. 이것이 "두 폴더의 관계를 아무도 안 알려준다" 결함의 단일 해법이다.
5. 복구 안내에서 `manifest.json` 단독 지시를 `manifest.json(없으면 journal.ndjson)` 으로 교체 — `src/bin/cys.rs:4497,4583`, `ui/src/main.ts:5201`, `USER-MANUAL.md`, `docs/GUIDE-clean-reset-KR.md`.

**회귀 테스트** 저널 파일을 읽기전용으로 만들어 write 실패를 유도 → 해당 항목이 원위치에 남고 `failed` 에 기록됨. `trash_dir` 를 읽기전용으로 만들어 manifest 실패 유도 → `Ok(report)` 이고 `stripped` 가 채워지며 `manifest_written==false`. 중단 잔재 폴더 감지 테스트.

**2차 파급** Windows: `rename` 이 공유 위반으로 실패하는 GUI 웹층은 `best_effort` 라 이 경로에 안 걸리지만, `%LOCALAPPDATA%\cys` 는 정규 격리라 저널 강제화 후 실패율이 그대로 `failed` 로 드러난다 — 윈도우 첫 릴리스에서 "부분 완료"가 늘어 보일 수 있음을 릴리스 노트에 미리 적을 것.

---

### P0-6. 정지 실패·중단 시 이미 끝난 파괴적 부수효과를 숨긴다 + 격리 디렉토리 사전 점검 부재
병합: #4 + #20 + 무번호 "정지 폴링 구간 진행 표시 없음" + 무번호 "cysd N개 정상 종료 신호는 윈도우에서 거짓"

**피해** `정지 실패: cysd 1개가 종료되지 않았다(pid [72358])` 한 줄만 뜨지만 실제로는 launchd 등록 해제와 **plist 파일 삭제**가 이미 끝났고 마스터·워커·부서 세션은 전부 죽었다. 사용자는 "실패했으니 원래대로겠지"라고 읽는다. `~/.local/state/cys-trash` 가 파일인 기계에서는 프리뷰·문구 입력까지 다 통과한 뒤 데몬 전멸·launchd 해제가 끝나고 나서야 `격리 디렉토리 생성 실패 …: Not a directory` 가 뜬다 — 리셋 전보다 더 망가진 시스템만 남는다. TERM 후 최대 12초 무출력 구간에서 사용자가 Ctrl-C 를 누르면 아무 메시지도 없이 같은 상태가 된다. 윈도우에서는 `정상 종료 신호(TERM)` 라고 말하면서 실제로는 `taskkill /T /F` 로 pane 자식(claude·python·git-bash)까지 즉사시킨다.

**근거** `src/factory_reset.rs:664-697`(bootout → unload → `std::fs::remove_file(plist)` at :677, 격리 성공과 무관하게 선행) · `:712-770`(TERM→80회 폴링→KILL→40회, 그 사이 progress 호출 없음) · `:724-728`(윈도우 `taskkill /PID /T /F` 인데 문구는 :715 의 "정상 종료 신호(TERM)") · `src/factory_reset.rs:905-907`(`create_dir_all` 이 stop 이후 첫 동작) · `src/bin/cys.rs:4532-4535`, `:4589-4592`(부수효과 미고지) · cys.rs 전체에 factory-reset 용 SIGINT 핸들러 없음.

**수정 지시**
1. **순서 교정**: `plan.launchd_plist` 삭제를 `stop_daemons_and_unregister` 에서 빼고 `execute_quarantine` 성공 직후로 이동(bootout/unload 는 그대로 stop 에 유지 — KeepAlive 부활 방지가 목적이라 파일 삭제는 불요).
2. **사전 점검**: `build_plan` 에 `trash_root_ready: Result<(),String>` 필드 추가(디렉토리인지·쓰기 가능한지 — `create_dir_all` 대신 `metadata` + 임시 파일 생성 시도). CLI `--plan`·GUI 프리뷰에서 부적격이면 **모달을 아예 띄우지 않고** `~/.local/state/cys-trash 가 파일입니다 — 그 파일을 지운 뒤 다시 시도하세요` 로 사전 거부. 실집행 경로에서는 격리 디렉토리를 **stop 이전에 선생성**.
3. **부수효과 고지 공통 함수** `stop_side_effects_note()` 신설: `데몬은 이미 정지되었고 launchd 자동시작 등록이 해제되었습니다(격리는 진행되지 않았습니다). 복구: cys daemon install`. `src/bin/cys.rs:4533`·`:4590` 와 GUI 실패 토스트(main.ts:5178)에 모두 삽입.
4. **SIGINT 핸들러**: run_factory_reset 실행 구간에서 `ctrlc`(리포에 이미 `cys run --scoped` 용 핸들러 존재, cys.rs:10972 참조)를 걸어 같은 안내를 출력하고 exit 130.
5. **진행 표시**: 폴링 루프에서 1초마다 `progress("stop", "cysd 종료 대기 N/12초")`.
6. **문구 정직화**: 윈도우 분기의 `정상 종료 신호(TERM)` 를 `프로세스 트리 강제 종료(taskkill /T /F — 미저장분 손실)` 로 교체.

**회귀 테스트** `trash_root` 를 파일로 만든 뒤 `build_plan` 이 `trash_root_ready.is_err()` 를 내는지. plist 삭제가 `stop_*` 에 없음을 소스 트립와이어로 단언. CLI 실패 문구에 `cys daemon install` 문자열 포함 단언.

**2차 파급** 부트 체인: plist 삭제를 뒤로 미뤄도 `bootout` 으로 미적재는 확정되므로 KeepAlive 부활 위험은 없고, 리셋 실패 시 `cys daemon install` 없이도 다음 로그인에 자동 복구되는 이득이 생긴다.

---

## P1 — 배포 전 필수 (4건)

### P1-1. 훅 해제 실패의 사후 조치가 없다 (심링크·파싱·CLAUDE_CONFIG_DIR·`pack*` 오탐)
병합: #5 + 무번호 "CLAUDE_CONFIG_DIR 설정 파일 미대상" + 무번호 "사용자 `~/.cys/pack*` 폴더의 자기 훅만 해제" + 무번호 "chmod 444 잠금이 조용히 풀림" + 무번호 "`.bak-factory-reset` 백업 5개 신설"(#16)

**피해** 도트파일 저장소로 `~/.claude/settings.json` 을 심링크한 사용자는 초기화 후 **매 Claude Code 세션마다** 18개 훅이 `sh …/.cys/pack/hooks/*.sh: No such file or directory` 로 실패하는 것을 본다. 화면 안내는 `실패 /Users/…/settings.json: 훅 제거 실패(파일 무변경): … is a symlink — refusing` 한 줄뿐이고 "손으로 지우세요"라는 조치가 없다. 게다가 그 한 줄 때문에 전체가 exit 1 · "부분 완료"가 된다. 반대 방향 오류도 있다 — 사용자가 `~/.cys/pack-notes/` 를 두고 훅으로 등록해 뒀다면 파일은 남는데 훅만 조용히 해제된다(A8 대칭 계약의 정확한 역전). 리셋은 `~/.claude*` 에 cys 훅 전문이 든 백업 파일을 5개 더 만든다 — "cys 흔적은 격리 폴더 한 곳"이라는 계약 위반이고, 되돌리면 죽은 훅이 부활한다.

**근거** `src/factory_reset.rs:1101-1106`(symlink → Err) · `:1112-1113`(parse → Err) · `:998`(Err→failed) · `:1193-1196`(`.bak-factory-reset` 을 프로필 디렉토리에 생성, 정리 코드 없음 — 리포 전수 grep 결과 이 문자열은 구현·테스트·문서에만 등장) · `src/pack.rs:486-503`(대상은 `~/.claude`·`.claude-*` 디렉토리뿐) vs `cysjavis-pack/bin/javis_preflight.py:654,682`(등록기는 `CLAUDE_CONFIG_DIR` 을 **최우선**으로 넣는다) · `src/factory_reset.rs:1047-1049`(`command_points_into_pack` 이 `<home>/.cys/pack` 문자열 **포함**만 보므로 `pack-notes` 도 매칭).

**충돌 판정** "심링크면 realpath 대상을 고쳐라" vs "심링크 거부는 설정 클로버 금지 계약" → **부분 채택**. `canonicalize` 결과가 `$HOME` 아래의 일반 파일이면 **그 실파일**을 대상으로 재시도한다(링크 자체는 rename 하지 않으므로 계약 정신 보존). realpath 가 홈 밖이거나 파일이 아니면 기존대로 거부.

**수정 지시**
1. `strip_cys_from_settings_scoped` 심링크 분기: `canonicalize` → 홈 아래 일반 파일이면 그 경로로 재귀 1회.
2. `command_points_into_pack` 의 needle 을 `<cys_base>/pack` **경계 일치**로 강화(`/pack` 뒤가 `/` 또는 문자열 끝). `pack-dept-` 는 별도 needle 로 명시 추가.
3. 백업 위치를 `<trash_dir>/settings-backups/<프로필명>.settings.json` 으로 이동(`:1193`). 기존 `.bak-*` 21개는 `report_only` 로 `cys 훅이 든 옛 백업 N개가 남아 있습니다 — 되돌리면 죽은 훅이 부활합니다` 고지. `~/.claude-2/settings.json.cys-lock` 은 cys 산물이므로 strip 단계에서 제거.
4. `personal_profile_dirs_under` 소비 지점에 `CLAUDE_CONFIG_DIR`(설정돼 있고 `~/.cys` 밖이면) 을 추가 대상으로 포함.
5. 파일 권한 보존: `write_atomic` 전에 원본 `permissions` 를 읽어 교체 후 복원(0444 잠금 유지). 최소한 `읽기 전용이던 파일이 쓰기 가능해졌습니다` 고지.
6. **조치 블록 승격**: strip 실패 항목은 CLI 완료 출력 말미와 GUI 완료 모달에 `직접 지워야 하는 훅` 섹션으로 승격 — 어떤 이벤트·어떤 명령이 남았는지 열거 + 백업 경로.

**회귀 테스트** 심링크 settings(대상 홈 안/홈 밖) 두 케이스, `~/.cys/pack-notes` 훅이 보존되는 대칭 테스트, 0444 권한 보존 테스트, 백업이 `trash_dir` 아래에 생기는지.

**2차 파급** 오케스트레이션 앵커: `~/.cys/claude*`(cys 전용 CLAUDE_CONFIG_DIR, `src/pack.rs:580`)는 통째 격리되므로 표준 설치는 무영향 — 4번 수정은 비표준 CLAUDE_CONFIG_DIR 사용자 전용 경로임을 주석에 못 박을 것.

---

### P1-2. 복구 경로가 존재하지 않는다 ("되돌릴 수 있습니다"의 미이행)
병합: #10 + #14 + 무번호 "journal 안내 순환" + 무번호 "두 격리 폴더 관계 미고지"(P0-5와 분담)

**피해** 모달·가이드·CLI 모두 "격리 보관되어 되돌릴 수 있다"고 안심시키지만 앱에도 CLI에도 복구 명령·버튼이 없다. 실제 복구는 터미널로 manifest.json 을 열어 수십 건을 손으로 `mv` 하는 것뿐이고 그 절차는 생초보 가이드 어디에도 없다. 동시에 CLI는 `14일 후 reap 자동 소거`라고 **단정**하는데, 그 소거를 실행하는 유일한 코드(`cysjavis-pack/bin/cys-dept:1175-1185`)는 이번 리셋이 격리하는 `~/.cys/pack` 안에 있고 `depts.json` 도 격리된다 — 2.29GB 가 무기한 남는다. 설계 문서 자신이 "사용자에게 '14일 후 자동 소거'로 **단정하지 않는다**"(`docs/DESIGN-factory-reset.md §2.1`)고 못 박았는데 CLI 문구가 그 계약을 어긴다.

**수정 지시**
1. **`cys factory-reset --undo <trash-dir>`** 신설(`src/bin/cys.rs`): manifest(없으면 journal) 를 읽어 `to→from` 역방향 mv, 목적지 존재 시 스킵·보고, `--plan` 대칭 프리뷰 제공. 1콜 복구가 있어야 "복구 가능" 주장이 성립한다.
2. GUI 완료 모달에 `격리 폴더 열기` 버튼(Finder/탐색기 reveal — Tauri `opener`).
3. 문구 정합: `src/bin/cys.rs:4497`·`:4583` 의 "14일 후 reap 자동 소거"/"14일 내"를 GUI와 같은 비단정 표현(`정리 작업에서 소거될 수 있습니다 · 직접 지우려면 rm -rf <trash_dir>`)으로 교체. 반대로 GUI 모달에는 `약 14일` 수치를 넣는다(P0-2 3번) — **수치는 알리되 자동 실행은 단정하지 않는다**가 판정.
4. `docs/GUIDE-clean-reset-KR.md` 에 `되돌리기` 절 신설(`--undo` 1줄 + 수동 절차).

**2차 파급** 오케스트레이션 앵커: `--undo` 도 `CYS_OWNER_ONLY_SUBS` 판정 대상이어야 한다 — `guard.sh:265` 는 서브커맨드 `factory-reset` 단위로 막으므로 `--undo` 는 자동으로 DENY 에 포함된다(추가 작업 없음, 단 테스트로 핀 박을 것).

---

### P1-3. 리셋 중/후 UI 상태가 서로 모순되고, 파괴적 단축키가 모달을 관통한다
병합: #19 + #9 + 무번호 "완료 모달 '아니오' 후 버튼 무반응" + 무번호 "⌘W 가 모달 관통" + 무번호 "리셋 중 탭 × 가 무관한 경고" + 무번호 "재실행 시 '직원 복귀 중' 토스트" + 무번호 "온보딩이 안 보인다" + 무번호 "부서 5개가 라벨 없이 동시 종료" + 무번호 "`cys events --reconnect` 영구 종료"

**피해** ①`완전 초기화 완료 — 지금 앱을 종료하세요` 모달과 `로그인 항목에서 cys 백그라운드 항목을 허용한 뒤 앱을 다시 여세요` 배너가 같은 화면에 공존한다(부트 재시도 루프가 리셋을 모른다). ②확인 모달이 떠 있어도 ⌘W/⌘T/⌘D 가 살아 있어, 모달을 닫으려던 사용자가 뒤의 pane 을 확인 없이 죽인다. ③완료 모달에서 '아니오'를 고르면 `+ New`·`Split`·사이드바 `＋` 가 토스트 하나 없이 완전 무반응이 된다(가드 미배선). ④`종료 후 다시 실행하면 정리됩니다`(이연 안내)를 실제로 이행하는 코드가 없다 — 윈도우에서는 이 경로가 사실상 항상 발생하므로 모든 윈도우 사용자가 거짓 안내를 본다. ⑤재실행 첫 화면이 `👥 직원 복귀 중 → ✅ 직원 복귀 완료` 로, 방금 조직을 지운 사용자에게 부활을 알린다.

**근거** `src-tauri/src/main.rs:3701-3722`(재시도 루프·최종 로그인 항목 문구, 리셋 인지 없음) · `ui/src/main.ts:6170`(전역 키 핸들러는 `paletteOpen` 만 가드) · `:5897-5901`·`:5984`(btn-new/split/close/ws-new 에 `daemonActionBlocked` 없음 — 가드 호출부는 `:4100,4376,4487,4758,5009,5133,6092` 뿐) · `:5195-5198`(이연 안내) vs `src-tauri` 전체에 deferred 재처리 경로 없음 · `src/factory_reset.rs:192-197`(코드 주석이 이 실패 양상을 스스로 인정).

**수정 지시**
1. 리셋 시작 시 `reset-progress` 첫 이벤트에서 부트 재시도 루프에 취소 플래그(`AtomicBool`)를 세우고, `ensure_daemon` 이 `reset_in_progress` 사유로 실패한 경우엔 로그인 항목 문구 대신 `초기화 중/완료 — 앱을 종료한 뒤 다시 실행하세요` 를 emit. `daemon-hint` sticky 도 리셋 시작 시 dismiss.
2. `ui/src/main.ts:6170` 전역 키 핸들러 최상단에 `if (document.querySelector(".modal-overlay")) return;` 추가. 확인 모달에 Esc = 취소 핸들러 추가.
3. `actionNew`·`actionSplit`·`addWorkspace`·부서 탭 × 핸들러에 `daemonActionBlocked()` 배선(`main.ts:3383` 주석이 이미 "신규 호출부에 반드시 배선하라"고 규정한다). 부서 탭 × 확인창의 `재시작 시 부활할 수 있습니다` 경고는 `resetCompleted` 시 `완전 초기화가 진행/완료됨 — 이 부서는 이미 격리 대상입니다` 로 교체.
4. **이연 안내 정직화**: 문구를 `앱을 종료한 뒤 외부 터미널에서 cys factory-reset 을 한 번 더 실행하면 정리됩니다` 로 교체. 추가로 종료 직전 `localStorage.clear()`(LAYOUT_KEY `cys-layout-v2`, main.ts:85)를 실행해 유령 워크스페이스 재현을 차단한다 — 이건 재시도 구현 없이도 가장 큰 증상을 없앤다.
5. 재실행 첫 화면: `.gui-onboarded` 부재 + 이전 리셋 마커가 있으면 `직원 복귀` 토스트를 억제하고 `설치 초기 상태입니다 — ①에이전트 로그인 ②▶CEO 기동 ③＋부서` 3단계 온보딩 카드를 띄운다.
6. `cys events --reconnect` 는 데몬 부재 시 재연결 백오프를 유지하되 `완전 초기화가 진행 중 — 끝나면 자동 재연결합니다` 를 1회 출력(현재는 조용히 종료).

**2차 파급** 부트 체인: 1번의 취소 플래그가 리셋 실패 후 정상 부팅까지 막지 않도록 **리셋 프로세스 종료 시 반드시 해제**해야 한다(P0-1 센티널 Drop 과 같은 생명주기에 묶을 것).

---

### P1-4. 릴리스 게이트가 Windows 회귀를 구조적으로 못 잡는다
병합: 무번호 "윈도우 전용 테스트가 윈도우에서 실패" + 무번호 "윈도우 사용자는 CLI 리셋에 도달 못 함" + 무번호 "새 guard.sh 가 구버전 바이너리에 배달"

**피해** `windows_roots_are_planned_when_present` 는 문자열 단언 `p.ends_with("AppData/Local/cys")` 를 쓰는데 윈도우의 `PathBuf::join` 은 백슬래시로 이어 붙이므로 실제 값은 `…AppData/Local\cys` 가 되어 깨진다. 그런데 CI 는 macOS 레그만 돈다 — 윈도우 경로 규약 회귀는 릴리스 게이트에서 **절대** 잡히지 않는다. 동시에 윈도우 사용자는 `cys` 가 PATH 에 없어 CLI `--plan`(경로 목록을 볼 수 있는 유일한 화면)에 사실상 도달할 수 없고, GUI 모달은 경로를 안 보여준다(P0-2) — 무엇이 사라지는지 확인할 방법이 0이 된다. 팩 채널의 새 `guard.sh` 는 `min_binary_version`(정책상 `security-floor.txt` = 0.12.48 이상, 현 본체 0.14.16)보다 훨씬 낮은 버전에도 배달되므로, `factory-reset` 서브커맨드도 '완전 초기화' 버튼도 없는 구버전에서 `주인님이 직접 GUI '완전 초기화' 버튼 또는 에이전트 pane 밖 터미널에서 실행한다` 는 안내만 도착한다.

**근거** `src/factory_reset.rs:1269-1285`(문자열 `ends_with`) · `.github/workflows/ci-branch.yml:29`(`runs-on: macos-latest`)·`:527-531`(`cargo test --bin cys` 도 macOS) · `security-floor.txt:5`(0.12.48) · `Cargo.toml:8`(0.14.16) · `cysjavis-pack/hooks/guard.sh:265-268`.

**수정 지시**
1. 테스트 단언을 `PathBuf` 비교로 교체: `q.iter().any(|p| p == &lad.join("cys"))`. 같은 파일의 `".cys/pack"` 계열 문자열 단언도 전수 교정.
2. CI 에 `windows-latest` 레그를 추가하되 최소 범위로: `cargo test --lib factory_reset::` 만(전체 빌드 비용 회피). 이번 릴리스에 레그 추가가 부담이면 **최소한 로컬 윈도우 기계에서 1회 수동 실행 + 결과 첨부**를 릴리스 체크리스트에 강제한다.
3. `guard.sh` DENY 문구에 버전 조건 추가: `이 기계의 cys 가 0.14.16 미만이면 이 기능 자체가 없습니다 — 본체를 먼저 업데이트하세요`. 또는 `CYS_OWNER_ONLY_SUBS` 판정 전에 바이너리 버전을 읽어 미지원이면 일반 unknown-subcommand 경로로 흘린다.
4. 윈도우 도달성: GUI 거부 메시지와 `docs/GUIDE-clean-reset-KR.md` 에 `%LOCALAPPDATA%\cys\cys.exe factory-reset --plan` 전체 경로를 명시.

**2차 파급** 윈도우: 2번 레그 추가 시 `sysinfo`·`taskkill` 의존 테스트가 CI 샌드박스에서 실패할 수 있으므로 `stop_daemons_and_unregister` 는 테스트 대상에서 제외(현재도 "부수효과 크므로 호출하지 않는다"고 `factory_reset.rs:607` 에 선언돼 있다).

---

## P2 — 배포 후 개선 (2건)

### P2-1. CLI UX — 프리뷰 마무리·목록 가독성·문구 확인의 관용성
병합: #6 + #7(목록 축) + 무번호 "문구 한 번 틀리면 즉시 종료·따옴표 유도" + 무번호 "유니코드 정규화 없는 바이트 동등" + 무번호 "GUI 실행 중 거부가 끄는 법을 안 알려줌"

**피해** `--plan` 만 본 사용자는 (a) 에이전트 재로그인이 필요하다는 사실을 끝내 못 보고(그 문장은 `cys.rs:4511-4515` 실집행 분기 전용), (b) 52줄 스크롤 뒤 마지막 줄이 `복구 …manifest.json 의 to→from 역방향 mv` 라서 **이미 실행된 것처럼 읽히며**, (c) 다음에 뭘 해야 하는지 안내가 없다. 확인 프롬프트는 `실행하려면 "완전 초기화" 를 정확히 입력:` 이라 화면에서 드래그 복사하면 큰따옴표가 딸려오고 `trim()` 은 따옴표를 안 깎아 즉시 exit 1, 재입력 기회 없음. NFD 자모 분리·NBSP·전각 공백·ZWSP 는 육안상 동일한데 전부 거부된다.

**수정 지시** `src/bin/cys.rs:4501` 의 `plan_only` return 직전에 요약 블록 추가(⚠ 재로그인·비가역·보존 3줄 + `쓰기 0 — 아무것도 변경되지 않았습니다` + `실제로 실행하려면: cys factory-reset (앱을 먼저 종료)`). 목록은 카테고리 소계 + 크기 내림차순 + 상위 5건 후 `…외 N건 (--verbose)`. 문구 비교를 `unicode-normalization`(NFC) + 양끝 따옴표 제거 + 공백류(NBSP/전각/ZWSP) 정규화 후 비교로 바꾸고, **최대 3회 재입력** + 불일치 사유 표시(GUI 와 동등한 체감). GUI 실행 중 거부 메시지에 `⌘Q(맥) / 창 닫기 후 Dock 우클릭 종료` 를 명시하고, "지금 앱 안에 있다면 topbar 버튼" 대안을 조건절이 아니라 **첫 줄**로 올린다.

**2차 파급** 오케스트레이션: `--verbose` 신설 시 `guard.sh` 의 서브커맨드 판정(`cys_sub_strict`)은 플래그 무관이라 DENY 가 그대로 유지된다 — 무영향.

---

### P2-2. 잔재·정합 (cys 흔적이 "격리 폴더 한 곳"이 아니다)
병합: #16 + 무번호 "`~/.gemini` 배선이 report_only 에서 빠짐" + 무번호 "`--purge-local` 시 codex skills 58개 링크 절단 무고지" + 무번호 "`claude-` 접두 규칙이 미등록 보존 규칙을 이긴다" + 무번호 "TMPDIR 잔재 1,103건" + 무번호 "cys-trash 기존 항목이 보존 목록에도 안 나옴" + 무번호 "리셋 직후 주기 잡 4개 동시 만기" + 무번호 "수동 등록 opt-in 훅이 사라지고 목록이 없음" + 무번호 "`~/.cys` 직하 오너 설정 3종(policy/accounts/profile.json) 격리"

**수정 지시(우선순위 순)**
1. `~/.gemini/settings.json`(`trustedWorkspaces`)·`~/.gemini/config/skills.json` 을 `report_only` 에 추가 — `~/.codex/config.toml` 과 대칭(`src/factory_reset.rs:472-481` 바로 아래).
2. `~/.cys/policy.json`·`accounts.json`·`profile.json` — 재온보딩이 재생성하지 않으므로 **격리 전 `<trash_dir>/owner-settings/` 사본 + 완료 안내에 "되돌리려면 이 3파일을 제자리로"** 를 추가하거나, `deny_self_approve:false` 같은 오너 판단은 보존 목록으로 옮긴다. 최소한 모달의 `직접 만든 설정은 삭제되지 않습니다` 문장과의 모순을 문구로 해소할 것.
3. `--purge-local` 사용 시 `~/.codex*/skills` 에서 끊길 링크 수를 프리뷰가 미리 세어 고지.
4. `~/.cys` 직하 `claude-` 접두 규칙에 예외를 추가(정확히 `claude-<계정슬러그>` 디렉토리만 — 일반 파일 `claude-*.env`·`claude-notes.md` 는 보존).
5. `cys-trash` 기존 항목(`--help-20260816T103649` 등)을 `보존` 줄에 열거 — 설계는 Keep 으로 선언했는데 화면에 한 글자도 안 나온다.
6. 리셋 직후 첫 부팅의 주기 잡 4개 동시 만기: 재온보딩 시 `schedule_state.json` 부재를 감지하면 `last_fired = now`(=다음 주기부터)로 초기화. 마스터 폭탄과 무발화 침묵 양쪽을 동시에 없앤다.
7. TMPDIR 1,103건은 전부 `#[cfg(test)]` 산물이므로 **일반 사용자 피해 0** — 릴리스 노트에 적지 말고 개발 기계 정리 스크립트로만 처리.

**2차 파급** 오케스트레이션 앵커: 6번은 `phoenix 세대 스냅샷`·`RSI 학습 TTL 감사`·`fleet digest` 의 첫 발화 시점을 바꾸므로, 조직 재구축 직후 6시간~1주 공백이 생긴다는 사실을 스케줄러 주석에 명시할 것.

---

## 아키텍트 판정 요약 (충돌 7건)

| # | 충돌 | 판정 | 근거 |
|---|---|---|---|
| 1 | #13(`_round` 범위 확대) vs #12(축소·고지) | **순회는 채택, 결과는 report_only. 전면 격리는 `--purge-round` opt-in** | 격리본은 14일 뒤 영구 소거 → 동의 없는 사용자 저작물 이동 금지(A8·D1a 계승). 비대칭도 동시 해소 |
| 2 | #17(manifest 실패 → Ok 강등) vs DESIGN §6("격리 미진입 = 부분 이동 0") | **설계 문서를 고친다. Ok(report) + exit 1 유지** | 실패 시점이 rename 이후라 "미진입"이 거짓. 훅 해제 스킵이라는 2차 피해가 더 크다 |
| 3 | #5(심링크 settings 수정 허용) vs "설정 클로버 금지" | **realpath 가 홈 아래 일반 파일일 때만 그 실파일을 수정. 링크 교체는 계속 금지** | 도트파일 저장소는 흔한 구성이고 실파일 편집은 클로버가 아니다 |
| 4 | #3/#8(토스트 TTL·배너 확장) vs 오너 요구("종류 불문 자동 소멸", toastttl.ts:3-6) | **정본은 모달 본문 + `<trash_dir>/REPORT.txt` 영속. 배너는 보조로만 추가** | 오너의 명시 요구를 뒤집지 않고 정보 소실만 막는다 |
| 5 | #10(모달에 "14일" 명시) vs #14(CLI 의 "14일 단정"이 계약 위반) | **수치는 알리되 자동 실행은 단정하지 않는다** — 양쪽 다 `약 14일 후 소거될 수 있습니다 · 직접 지우려면 rm -rf` | DESIGN §2.1 이 요구하는 것은 "자동 소거의 단정 금지"이지 "시한 은폐"가 아니다 |
| 6 | #11(stop 전 drain) vs 원칙 3(등록 해제 우선·전멸 실측) | **launchd bootout 이후 · cysd TERM 이전에 best-effort drain 1회 + 3초 유예** | 순서 계약을 깨지 않으면서 저장 신호 제공. 실패는 무시(격리 게이트에 영향 없음) |
| 7 | 프리뷰 동기 유지 vs 비동기화 | **비동기화(async + spawn_blocking)** | 실행 커맨드가 이미 그 패턴이라 비대칭이 결함이다(`main.rs:2916` vs `:2891`) |

---

## 배포 판정

### **NO-GO (현행 워킹트리 기준)**

이유 3가지 — 어느 하나도 문서·릴리스노트로 흡수할 수 없다.
1. **동의 없는 데이터 이동**: GUI 사용자는 자기 프로젝트 폴더 안 279MB 가 옮겨진다는 사실을 승인 전에 알 방법이 전혀 없고(P0-2), 14일 뒤 영구 소거되며(P1-2), 복구 수단이 제품에 없다.
2. **거짓 상태 보고**: 부분 실패가 "완전 초기화 완료"로 표시되고(P0-4), 2.4GB 를 다 옮긴 뒤 "격리 미진입"이라고 말한다(P0-5). 사용자가 화면을 믿고 내린 결정이 손해로 직결된다.
3. **자기 계약 위반**: "설치 초기 상태로 되돌린다"는 명령이 자기 센티널을 남기고(P0-1), 설계 문서가 트립와이어로 선언한 RAII 계약이 프로덕션에 배선돼 있지 않다.

### 승격 조건
**P0-1 ~ P0-6 + 회귀 테스트 전건 통과 시 → GO-WITH-FIXES.**
- P1-1 ~ P1-4 는 같은 릴리스에 포함하되, P1-2 의 `--undo` 만은 "복구 가능" 문구를 쓰는 이상 **P0 로 승격 가능**하다 — 문구에서 "되돌릴 수 있습니다"를 빼는 대안을 택하면 P1 로 남겨도 된다(오너 선택 사항).
- P2 는 릴리스 노트 고지 후 다음 패치.

예상 작업량(참고): P0 6건 ≈ Rust 4파일·TS 3파일 수정 + 신규 회귀 테스트 12개. P1 4건 ≈ + CI 워크플로 1건.

---

## 릴리스 전 실기기 검증 체크리스트

전 항목 **실측 확인** 원칙 — 화면에 실제로 뜬 문구를 그대로 기록해 첨부한다. 검증은 반드시 **스냅샷/복제 가능한 기계**에서, `~/.cys`·`~/.local/state`·`~/Desktop/CYSjavis` 백업 후 수행한다.

### 공통 사전 준비
- [ ] `~/.cys/apple-notary.env`·`hostinger-ftp.env`(오너 배치 파일) 존재 상태로 시작 → 리셋 후 **제자리 보존** 확인
- [ ] `~/.claude/settings.json` 에 팩 훅 18개 + statusLine 1개 등록 상태 확인(`grep -c '.cys/pack'`)
- [ ] 프로젝트 `_round` 최소 3곳(홈·워크스페이스·프로젝트) 준비, 각 용량 기록

### macOS
1. [ ] `cys factory-reset --plan` — 마지막 화면에 `쓰기 0 — 아무것도 변경되지 않았습니다` + `실제로 실행하려면` 안내가 보이는가. 실행 전후 `~/.cys` mtime 불변(쓰기 0 실증)
2. [ ] GUI `완전 초기화` 클릭 → **모달이 뜨기 전 스피너**가 보이는가(창 굳음 0). 모달에 `~/Desktop/CYSjavis/<프로젝트>/_round` 경로와 용량, `report_only` 3건, `실행 중인 세션 N개 즉시 종료`, `약 14일`이 모두 보이는가
3. [ ] 모달 떠 있는 상태에서 **⌘W·⌘T·⌘D** → 뒤의 pane 이 죽거나 새 pane 이 생기지 않는가. **Esc** 로 취소되는가
4. [ ] 정상 실행 완료 후 `ls -la ~/.local/state/.cys-factory-reset-in-progress` → **파일 없음**(P0-1)
5. [ ] 완료 모달에서 `나중에` 선택 → `+ New`·`Split`·사이드바 `＋` 클릭 시 **차단 토스트**가 뜨는가(무반응 0)
6. [ ] 실패 주입 A: `~/.claude/settings.json` 을 심링크로 만든 뒤 실행 → 완료 모달 제목이 `부분 완료`인가, 본문에 `직접 지워야 하는 훅` 목록이 있는가, `<trash_dir>/REPORT.txt` 가 존재하는가
7. [ ] 실패 주입 B: `~/.local/state/cys-trash` 를 **파일**로 만든 뒤 실행 → 데몬이 죽기 **전에** 사전 거부되는가(P0-6)
8. [ ] 실패 주입 C: 정지 단계에서 `Ctrl-C` → `데몬은 이미 정지되었고 launchd 자동시작 등록이 해제되었습니다 … cys daemon install` 이 출력되는가
9. [ ] 리셋 후 `launchctl list | grep cysjavis` 미적재 확인 → 앱 재실행 시 `register_if_absent` 로 재등록되는가
10. [ ] 재실행 첫 화면: `직원 복귀 중` 토스트가 **뜨지 않고**, 온보딩 카드가 보이는가(※괄호 예시 「에이전트 로그인 → ▶CEO → ＋부서」는 2026-08-20 P2 이후 구판 — 현행 온보딩은 pane 마스터 선언·`cys launch-agent` 경로다. ▶CEO·▶부서장 버튼은 제거됨). `로그인 항목을 허용하세요` 배너가 공존하지 않는가
11. [ ] 재실행 후 테마·창 배치·워크스페이스가 **초기값**인가(localStorage `cys-layout-v2` 소거 확인)
12. [ ] `cys factory-reset --undo <trash-dir>` → 부서·대화기억·`_round` 가 원위치로 돌아오고 앱이 정상 기동하는가
13. [ ] 에이전트 pane 안에서 `cys factory-reset` → guard.sh DENY 문구 + exit 2 (오케스트레이션 앵커 보전)

### Windows
1. [ ] `%LOCALAPPDATA%\cys\cys.exe factory-reset --plan` — GUIDE 에 적힌 그 경로로 실제 도달 가능한가
2. [ ] 모달 용량 표기에 `프로그램 파일 포함` 주석이 있는가(PortableGit·Python 전개본이 총량 대부분)
3. [ ] 정지 단계 문구가 `프로세스 트리 강제 종료(taskkill /T /F — 미저장분 손실)` 로 정직 표기되는가
4. [ ] `%LOCALAPPDATA%\cys` 격리 성공, `%LOCALAPPDATA%\com.cysjavis.terminal`(WebView2)은 `이연`으로 보고되는가 — **부분 실패로 세지 않는가**
5. [ ] 이연 안내 문구가 `앱을 종료한 뒤 외부 터미널에서 한 번 더 실행하세요` 로 바뀌었는가(거짓 약속 제거)
6. [ ] `schtasks /query /TN cysd` 미등록 확인 → 앱 재실행 시 온보딩이 재등록하는가
7. [ ] `cysd.prev*.exe` 세대 잔재를 띄워 둔 상태에서 전멸 실측이 그것도 잡는가(`scan_cysd_pids` W4 경로)
8. [ ] 윈도우 기계에서 `cargo test --lib factory_reset::` 전건 green (P1-4 — 경로 단언 교정 확인)

### 릴리스 게이트 (기계 증명)
- [ ] 소스 트립와이어: `ResetSentinel::arm()` 이 `src/bin/cys.rs` 와 `src-tauri/src/main.rs` 양쪽에 존재
- [ ] 소스 트립와이어: `stop_daemons_and_unregister` 에 `write_sentinel`·`remove_file(plist)` 부재
- [ ] `resetconfirm.test.ts` 문구 계약 테스트 green (강조 블록·report_only·14일)
- [ ] `cargo test --lib` / `cargo test --bin cys` macOS 레그 green + Windows 레그(신규) green
- [ ] `docs/DESIGN-factory-reset.md` §3(인벤토리)·§6(실패 모델)이 위 판정 1·2와 동기화됐는가