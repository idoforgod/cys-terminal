# Phoenix 완전판 설계안 — 재발 원천 차단 (2026-07-06)

> 발주: 오너 — "이 문제가 절대 재발하지 않도록. 윈도우·맥 배포용에서도 수정. 숨은 오류·약점·잠재 리스크 전부 발굴, 이번에 전부 고치자."
> 방법: 6방향 병렬 적대 감사(phoenix.py 전 2027줄 정독 · cysd Rust 체인 · 팩 배포 스큐 · Windows 동작성 · 상태파일 계약 · 격리 실테스트/재현) + master 직접 표본 검증(P0급 4건 원문 대조 확정) + 하네스 실증(손상 시나리오 FAIL 재현).
> 기준선: cys-terminal HEAD = a07cc7f (CloseCause 도입). 오늘 아침 잔재 묘비 4개는 수동 해제 완료.

---

## 0. 원인 성찰 — 왜 피닉스 오류가 반복되는가 (감사로 확정)

가설 3개를 세우고 감사로 검증했다. 셋 다 코드 실물로 확정됐다.

**① 계약 없는 경계.** 피닉스는 `cysd 바이너리 → 팩 python 스크립트 → 상태파일 JSON → 다시 cys 바이너리`의 4단 체인인데, 어느 경계에도 버전 핸드셰이크·스키마 검증·exit code 계약이 없다. 실증: 스폰 커맨드는 `["python3", <path>, "restore", "--auto"]` 고정에 유일 전제조건이 `phoenix.exists()`(main.rs:363) — "이 스크립트가 --auto를 아는가"를 실행 전엔 알 수 없다. 역방향(phoenix→`cys restore` 재호출, javis_phoenix.py:551-556)도 동일 무계약. 훅→python 8종도 같은 클래스.

**② 침묵 실패 문화.** 실패 신호가 체계적으로 소거된다. 실증 3건: (a) 스폰 자식의 stderr까지 `Stdio::null()`(main.rs:397-399) — argparse 오류 메시지 소실. (b) `cmd_restore`는 결과가 FAILED여도 dict를 return만 하고 sys.exit를 안 함 → **완전실패도 exit 0** — exit=1은 오히려 미포착 예외 crash일 때만 나와 신호가 뒤집혀 있다. (c) `except Exception: pass` 폴백이 손상·쓰기실패를 삼키고, 손상 시 빈 상태를 되써서 **소실을 디스크에 확정**한다(자가치유가 아니라 자가감염 — 하네스 재현으로 실증).

**③ E2E 부재.** 피닉스의 존재 이유가 "데몬 교체 생존"인데 그 시나리오를 검증하는 자동 장치가 없다. a07cc7f조차 Not-tested에 명시. Windows CI 스모크는 python3.exe 절대경로+PHOENIX_CYS를 주입해 실 스폰 경로를 우회 — **결함을 은폐하는 테스트**였다.

여기에 감사가 4번째 구조 원인을 추가했다:

**④ 묘비 이중 저장소 + 비대칭 수명주기.** 묘비가 topology.json(데몬)과 desired_roster.json(phoenix) 두 곳에 살고, 데몬은 "역할 재등장 시 해제"(state.rs:1396, handlers.rs:1332), phoenix는 "관측으로 늘고 수동으로만 준다"(monotone) — 해제가 phoenix로 흐르지 않는다. 부활 결정권은 phoenix가 쥐므로 데몬의 해제는 무의미해진다. 오늘의 제3겹은 이 비대칭의 한 단면이다.

---

## 1. 결함 대장 (6감사 통합 · 26건 · 전건 file:line 근거)

### P0 — 재발·원칙붕괴 직결 (7건)

| # | 결함 | 위치 | 실패 시나리오 |
|---|---|---|---|
| P0-1 | restore 전 결과 exit 0 — cysd가 완전실패와 성공 구분 불가. 신호 역전(crash만 exit 1) | javis_phoenix.py:1000-1006, 2016-2022 (디스패치가 반환 폐기, sys.exit는 deploy만) + main.rs:402(로그만) | 콜드부트 4/5역할 미부활(FAILED)이 "finished (exit=Some(0))" — 성공과 문자 동일 |
| P0-2 | desired_roster 손상 → 묘비 빈집합 리셋 + 즉시 되써서 확정 → 폐역 역할 부활 (1급 원칙 붕괴) | javis_phoenix.py:295-303, 337-339, 351-356 | **하네스 실증 FAIL**: 1바이트 손상 → tombstone 소실=True, 폐역 부활=True. phoenix-only 묘비는 재구성 소스 자체가 없음. dept는 glob 재발견이라 100% 재부활 |
| P0-3 | topology.json 손상 → close-묘비 영구 소각 (데몬측 대칭 결함) | governance.rs:779-791 `unwrap_or_default()` + 첫 persist가 빈 묘비 완본 덮어쓰기 | 손상 부팅 → in-memory 묘비 공백 → 의도삭제 역할 auto-restore 부활 |
| P0-4 | preserve-gate `_ =>` catch-all — 사용자수정·매니페스트부재·읽기실패 무구분 동결 (소유권 축 부재) | src/pack.rs:450-472 | 원인 ① 실증 경로: 신 cysd embed에 신 phoenix 있어도 매니페스트 불일치면 "사용자 편집"으로 오판·영구 동결. `cys init-pack`(force 없음, 문서 기본형)도 동일 게이트 → 수동 복사 외 복구 불가 |
| P0-5 | 스폰 계약·폴백·관측 전무 — `python3` 리터럴 + stdin/out/err 전부 null + exit≠0 분기 없음 + 임베드 폴백 없음 | main.rs:358-373, 392-406 | 스큐·인터프리터 부재 시 침묵사. 조합 수 무한(전 릴리스 cysd × 디스크의 전 phoenix 버전) |
| P0-6 | surface.close RPC 고정 OwnerClose → **launch-agent 실패 롤백이 역할을 오묘비화** (a07cc7f가 못 막는 우회로) | handlers.rs:1188 (cause 무구분) ← cys.rs:4276 (롤백 발신) | worker launch 1회 실패 → topology 묘비 → phoenix desired에 영구 박제 → 이후 정상 재기동해도 크래시·콜드부트 부활 영구 배제. master도 동일 경로 가능. **오늘 아침 묘비 4개의 유력 생성 경로** |
| P0-7 | Windows: auto-restore 첫 스폰에서 체인 단절 + CI가 결함 은폐 | main.rs:367(`python3` 리터럴)·394(PATH 무보강) — 동봉 `runtime\python\python3.exe`는 데몬 PATH에 없음. windows-build.yml:371-382(절대경로+PHOENIX_CYS 주입으로 실경로 우회) | 순정 Windows에 python3 없음(Store 스텁 포함) → CreateProcess 실패 → 콜드부트 부활 전무. mac은 CLT shim 우연 의존(미설치 소비자 맥도 동일 위험) |

### P1 — 조건부 무력화·오판 (10건)

| # | 결함 | 위치 | 요지 |
|---|---|---|---|
| P1-1 | 데몬↔phoenix 묘비 수명주기 비대칭 (P0-6의 일반화 = 제3겹 근본) | state.rs:1396·handlers.rs:1332(데몬 해제) vs javis_phoenix.py:331-356(monotone) | 정당한 탭 닫기→재기동 역할도 desired 묘비 잔존 → 부활 영구 배제. 해제 유일 경로=수동 CLI |
| P1-2 | phoenix 부활 master는 approval.sign 동결 상태로 깨어남 | create_surface(state.rs:1157/1173/1388)가 master_claimed_at 미스탬프 + handlers.rs:3166-3172 "master_unstable" 거부 | **자율주행 게이트 전면 마비**: 부활 master가 승인 서명 불능 — 과거 "--role master 영구동결" 함정이 자동 부활 경로에서 재현 |
| P1-3 | 회로차단기가 NOOP까지 카운트 + NOOP은 리셋 안 함 | javis_phoenix.py:723-736, 930-937, 512-525 | 300초 내 3회 재기동(전부 NOOP)→BREAKER_OPEN→진짜 부활 차단. OPEN 경로는 P1-5로 crash 연쇄 |
| P1-4 | production 백엔드(`cys restore`)가 desired 묘비 무시 | javis_phoenix.py:550-557, 1046-1071 | 묘비 저장소 이원화의 또다른 단면: desired에만 폐역된 역할이 일괄 restore로 부작용 부활 |
| P1-5 | raw subprocess.run TimeoutExpired 미포착 → exit 1 crash — **오늘 로그 exit=1의 정체(유력)** | javis_phoenix.py:539(rollback_proposal)·1385·1695·1889 | BREAKER_OPEN + 스냅샷 도구 15초 초과 → traceback → 이유 없는 exit=1 |
| P1-6 | master liveness를 `cys list` 화면 정규식 파싱 — 미claim(`role=-`) master를 죽음 오판 | javis_phoenix.py:437-446, 751, 1000 | --auto=master 포함 강제 → 살아있는 master와 중복 스폰. 포맷 변경 시 전 역할 죽음 오판→대량 재스폰 |
| P1-7 | desired/dept RMW 프로세스간 락 부재 — lost update | javis_phoenix.py:328-359, 419-434 (lease는 restore만: 683-703) | reconcile·roster·inherit 동시 실행 시 방금 병합된 묘비 유실 → P0-2 합류 |
| P1-8 | Windows lease 전면 fail-open (fcntl 부재) → auto+수동 restore 이중 스폰 | javis_phoenix.py:694 | mac의 TOCTOU 방어가 Windows에선 0 |
| P1-9 | auto-restore가 PHOENIX_CYS 미전달 → Windows에서 cys.exe 미해석 | main.rs:392-400 + javis_phoenix.py:1981 | P0-7 수리 후에도 이 지점에서 재차 단절 (schtasks/GUI 데몬 PATH에 설치 디렉터리 부재가 표준) |
| P1-10 | 하네스 격리 fail-open-to-LIVE + readiness 배너 리터럴 취약 | javis_phoenix.py:69·145(격리 미주입 시 LIVE write) / 609-619·943-961("bypass permissions on" 부재 시 산 노드를 INCOMPLETE 오판) | 테스트가 라이브 오염(과거 팩 삭제 사고와 동형) / 허위 실패 escalation |

### P2 — 마모·표류 (9건)

| # | 결함 | 위치 | 요지 |
|---|---|---|---|
| P2-1 | fresh-push 유일 역할명(`worker-fresh-<epoch>`)이 desired를 무한 오염 — 유령 부활 | schedule.rs:465·474 + phoenix 관측 | 일회성 역할이 상설로 축적·부활 시도 |
| P2-2 | persist_topology 라이브-온리 침식 — 순수 콜드부트(사전 watch 없음)에서 reap된 역할 부활 누락 | governance.rs:706 | phoenix 단조 desired가 완화하나 watch 상시 가동은 무보장 |
| P2-3 | verify를 결과 무관 done=True → 동일 부트세대 내 UNVERIFIED 고착 | javis_phoenix.py:918, 755 | 재검증 영구 skip |
| P2-4 | 세대 스냅샷 deploy-트리거 전용 → stale/부재 (현재 1일 stale 실측) | javis_phoenix.py:306-325 | 손상 치유 핵심 소스가 신규설치·장기 무배포 시 부재 |
| P2-5 | 시계 역행 시 스냅샷 lexical sort 오선택·GC 오삭제 | javis_phoenix.py:314-322, javis_state_snapshot.py:317-338 | stale 치유·최근 세대 오삭제 |
| P2-6 | breaker.json 손상 → 카운트 침묵 리셋 → 크래시루프 감지 무력화 | javis_phoenix.py:517-522 | 부활 폭풍 무제동 |
| P2-7 | restore lease atexit 부재 + deploy 내부 restore가 stale lease에 LEASE_HELD→허위 FAILED | javis_phoenix.py:683-703, 714-721, 1939-1959 | 조직 절반 미부활 |
| P2-8 | 원자쓰기 실패 침묵 (`except: pass`) + topology 손상 시 신규 묘비 미병합 | javis_phoenix.py:354-358, 432-433 / 270-281 | 영속 실패 무보고·방금 close한 역할 부활 |
| P2-9 | Windows Job Object 부재 — ConPTY 자식이 데몬 사후 생존 → "동반사망" 의미론 플랫폼 분기 | 전 소스 AssignProcessToJobObject 0건, channels.rs:457-458(CREATE_NEW_PROCESS_GROUP=역방향) | Reap 분류 어긋남·잔존/중복 노드 (kill-on-close 부재는 확정, 런타임 거동은 Windows 실기 미검증) |

### 감사로 반증·정상 확정된 항목 (오탐 방지 기록)

- **"죽어가는 구 데몬이 셧다운 순간 오분류 묘비를 쓴다" 가설 → 반증.** shutdown_cleanup(main.rs:184)은 close_surface·persist_topology를 호출하지 않는다. 배포 교체도 SIGTERM만. 오늘 묘비 4개는 구 데몬 **생전**(구 close_surface 코드)에 이미 기록된 것 + P0-6 롤백 경로가 유력.
- a07cc7f의 CloseCause 분류는 P0-6(surface.close 롤백 오용) 제외 전 call site 의미 정합. reap↔claim TOCTOU는 `roles.get(role)==Some(id)` 가드로 안전. 락 순서 AB-BA 없음.
- 원자쓰기 자체(torn write 방지)는 py·rust 양쪽 견고 — 결함은 원자성이 아니라 **손상 폴백 의미론**.
- governance 테스트 25/25 PASS (격리 실측). 동시 observe 경합은 파일 무손상(원자 replace) 실측.
- phoenix.py의 Windows 대응(경로·홈·소켓 경유·schtasks·launchd 분기)은 양호 — 끊기는 곳은 그것을 띄우는 Rust 트리거다.

---

## 2. 설계 — 5축 (각 축이 성찰 ①~④를 하나씩 원천 차단)

### 축 A. 묘비 소유권 일원화 (성찰 ④ 제거 — 제3겹·P0-6·P1-1·P1-4의 공통 근본)

**결정 확정(2026-07-06 오너 승인권 위임 → master 확정): 옵션 A + R1 안전조건 3종.**

- **옵션 A (확정): 데몬 topology.json = 묘비 유일 진실. phoenix desired 묘비 = 미러(조건부 replace 의미론).**
  - `observe_and_persist_roster`가 topology 묘비 집합을 **add-merge가 아니라 그대로 대입**(있으면 유지·없으면 해제). 데몬의 claim/create 해제가 자동으로 phoenix에 흐른다 → 제3겹 영구 소멸, 수동 `tombstone --remove` 불요화.
  - `phoenix tombstone <role>` CLI는 desired 직접 쓰기 대신 **데몬 RPC 경유**로 topology 묘비를 심는다(단일 작성자 원칙이 오히려 강화됨: 묘비 작성자=데몬 유일).
  - phoenix-only 묘비의 "topology 침식 면역" 요구는 소멸한다 — 묘비는 topology.json의 tombstones **필드**에 있고 이 필드는 라이브 엔트리 침식과 무관하게 영속된다(governance.rs:724-731 실측). 손상 대비는 축 C의 hard-fail+세대 스냅샷이 담당.
- **R1 안전조건 + R2 정밀화 (gemini Issue 1·codex Issue 4 → R2 공격 반영)**:
  - **A-S1 검증된-건강 replace (R2 정밀화)**: topology에 스키마 마커 `{schema_version, tombstones_rev}` 도입 — tombstones_rev는 데몬이 묘비 변경마다 올리는 **단조 카운터**. replace 조건 = 파싱 성공 + 마커 실존 + rev ≥ phoenix가 마지막으로 본 rev. (a) "파싱은 되지만 부분 절단·조작으로 묘비만 빈" 파일은 rev 부재/역행으로 걸러진다(gemini R2 공격 1). (b) **구버전 topology(마커·tombstones 키 부재) = 손상이 아니라 legacy**: replace 생략(기존 desired 묘비 유지)하되 **부활은 정상 진행 + 경고 1회** — 무기한 보류·escalation 오판 방지(gemini R2 공격 2). 손상(파싱 실패)만 C2 폴백 체인. **(c) rev 역행·초기화 강제 rebase (gemini R3)**: `.bak` 폴백·fresh install(rev=0)·데몬 epoch(started_at) 변경으로 topology의 rev가 phoenix 인지 rev보다 작아질 수 있다 — 이때 모든 replace가 데드락되지 않도록 **데몬 epoch 변경 또는 rev 명시 0 리셋을 감지하면 phoenix 인지 rev를 그 값으로 강제 rebase**한다(정당한 역행을 손상으로 오판하지 않음 — W2 게이트에 시나리오 박제).
  - **A-S2 격리 태그 = 정규화 상태 디렉터리 (R2 정밀화)**: 태그는 가변 소켓 경로가 아니라 **canonical state dir 경로**로 기록(하네스 임시 소켓의 태그 불일치 DoS 회피 — gemini R2 공격). desired_roster는 `phoenix_home(socket)` 격리 실측 확정(javis_phoenix.py:145·156·291, gemini R2가 반박 타당 ACCEPT).
  - **A-S3 다운타임 폴백 = intent 저널 (R2 재설계)**: 파일 직접 수정 대신 **append-only intent 저널**(`phoenix/tombstone-intents.jsonl`)에 기록 — observe/restore가 replace **이전에** 미소화 intent를 적용(멱등)하므로 "데몬이 구 topology를 먼저 읽어 CLI 수정을 덮는" TOCTOU 소멸(gemini R2 공격). intent 저널 쓰기에도 flock + **C2 1단계 동일 정책(health check·원자쓰기·`.bak` 유지·corrupt 시 `.corrupt-<ts>` isolate·canonical state dir 태그 검증)** 적용 — 직접 파일 폴백이 A-S1 검증된-건강 replace를 우회하는 무규율 두 번째 writer가 되지 않도록(codex R2 Issue 6 수용).  **A-S3-w(Windows msvcrt 주의 — W5 검증 발견 2026-07-06)**: msvcrt는 mandatory lock이라 write μs창의 동시 read가 그 사이클 intent를 빈 리스트로 볼 수 있다(flock advisory와 상이). 소비 규약: **빈 read = '이번 사이클 신규 intent 없음'으로만 해석 — 빈 read를 근거로 절단·prune 금지**(다음 read가 자연 회복·topology가 보조 진실이라 영속 소실 아님). W5 게이트에 빈 read 비파괴 단언 케이스. 데몬 복귀 시 RPC 재동기화 후 소화분 절단 — 단 **절단은 RPC 성공 응답 시점이 아니라, 갱신된 tombstones_rev가 담긴 topology.json이 디스크에 영속된 것을 phoenix가 로드·확인한 후에만** 수행한다(gemini R3: RPC 응답 직후 데몬이 topology 영속 전 크래시하면 묘비가 소실되는 TOCTOU 차단 — W2·W3 게이트에 순서 박제).
  - **A-S4 묘비 = 보존+제외, 삭제 아님 (W2 게이트 발견 · codex W2 BLOCK 수리 의미론 · 2026-07-06 설계 확정)**: 묘비 적용을 desired roster 엔트리 **삭제(`roster.pop`)로 구현하면 부활 원본이 파괴**돼 untomb 후 죽은 역할을 되살릴 소스가 없다(스냅샷·topology entries·live 병합 모두 죽은 역할엔 무효 — master 실물 확정). 확정 의미론: **묘비된 역할의 엔트리·메타는 desired에 보존하고, restore target 산정 시에만 제외**(tombstones 집합이 제외 필터) — untomb=즉시 부활 가능(별도 복원 소스 불요·의미론 자명). 게이트 subcase는 실제 상태(`roster 엔트리 보존+tombstones=['worker']` → untomb → target에 worker 복귀·부활)로 검증하며, 종전 fixture(pop 후 상태)를 쓰는 false-green을 금지한다. 부수 규칙: 묘비 보존 엔트리의 무한 누적 방지는 명시 purge(운영자 CLI) 또는 W3 GC 정책으로만 — 자동 삭제 금지(보존이 기본).
- 옵션 B (기각): phoenix desired = 진실, 데몬이 claim 시 "untomb" 이벤트 방출. — 이벤트 유실 시 비대칭 재발, 작성자 이원화 유지, 계약 표면 증가.

동반 수리: **surface.close RPC에 cause 파라미터 추가**(기본 OwnerClose·launch 롤백 발신처는 Reap 명시, P0-6) · fresh/ephemeral 역할은 topology/phoenix 관측 제외 플래그(P2-1) — 단 **플래그는 미래 유입만 차단**하므로 이미 desired_roster에 병합된 legacy `*-fresh-*` 오염분(현행 `observe_and_persist_roster` live-role 병합 javis_phoenix.py:348-350 경유)은 **W2에서 생성원천(source flag)·관찰근거·생성시각 대조로 quarantine해 부활 대상에서 제외**한다(legacy 마이그레이션 — codex R2 Issue 4) · `create_surface(role=master)` 시 master_claimed_at 스탬프 또는 phoenix 부활 직후 claim_role RPC 명시 발행(P1-2 — 자율주행 마비 해제).

### 축 B. 실행 신뢰원 일체화 — 배포 스큐 원천 소멸 (성찰 ① 절반 제거)

- **B1. 시스템 스크립트는 임베드 직접 실행** (R1 수용조건 4종 명문화 — codex Issue 3·gemini Issue 3): cysd가 phoenix를 디스크 팩이 아니라 바이너리 임베드본(PACK_ALL, build.rs 자동 스캔 확인됨)을 추출해 실행. 바이너리와 스크립트가 **같은 커밋임을 하드 보장** — 스큐가 존재할 수 없는 구조. 디스크 팩 사본은 가시성·디버깅용으로 유지.
  - **수용조건**: ① embedded primary — 추출 경로는 stale 권한 충돌을 피해 `<state>/phoenix-embed/<version>-<uuid>/`(버전+고유 ID 격리, 실행 후 정리) ② 추출 실패(공간·권한·noexec) 시 **manifest-해시 검증된 디스크 팩 폴백** ③ 추출 직후 self-test(`--pack-version` 응답 확인) ④ 폴백·실패 전부 stderr 로그+feed 이벤트로 보고(침묵 금지).
  - **명시된 tradeoff**: 임베드 일체화는 "팩 파일 즉시 교체" 방식의 핫픽스를 바이너리 릴리스 필요로 바꾼다 — 디스크 폴백 경로(②)가 긴급 우회 수단으로 남는 것이 이 tradeoff의 보상이다.
- **B2. 팩 소유권 매니페스트**: 팩 파일에 system|user 축 도입. install_into는 system 파일(bin/*.py, hooks/*)을 해시 불일치 시 무조건 갱신, user 파일(디렉티브·헌법·CLAUDE.md)만 preserve. P0-4의 `_ =>` catch-all을 소유권 분기로 대체 — 훅 경유 8종·`cys restore` 역호출까지 커버. 분류 기본값=system, user는 화이트리스트(조용한 탈락 방지).
  - **B2-1. schedule.json 소유권 판정 (W6 발견 텐션 · 2026-07-06 설계 확정)**: schedule.json은 사용자가 `cys schedule add`로 편집하는 혼합 파일(팩 기본 잡+사용자 잡 동거)이라 system 강제갱신 시 사용자 잡이 소실된다 — 사용자 잡 소실(사실상 비가역 데이터 손실)이 기본 잡 드리프트(부트 ensure로 복구 가능)보다 상위 리스크. **①즉시: schedule.json을 user 화이트리스트에 편입**(팩 갱신이 덮지 않음) **②근본(W3 스코프)**: built-in 잡은 팩 파일 배달이 아니라 **데몬 부트 시 코드가 idempotent ensure**(누락 시 생성·버전업 시 갱신·중복 생성 0), 사용자 잡은 schedule.json 단독 소유로 분리 — 기본 잡 진화와 사용자 잡 보존을 동시에 확보.
- **B3. 인터프리터 절대경로 해석**: `decide_auto_restore`가 동봉 runtime python 절대경로 우선(`runtime\python\python3.exe` / mac 동봉본) → 시스템 python3 폴백. 스폰 env에 `PATH`(runtime_prefixed_path)·`PHOENIX_CYS`(exe_dir의 cys 절대경로) 주입. mac·Windows 대칭(P0-7·P1-9·mac CLT 의존 동시 해소).

### 축 C. 침묵 실패 전폐 (성찰 ② 제거)

- **C1. exit code 계약** (R1 보강 — codex Issue 5 수용: "additive 아님"을 인정하고 이행 계획 동봉): run_restore 최종 판정→exit 매핑(VERIFIED/NOOP=0 · DEGRADED/UNVERIFIED=3 · FAILED/INCOMPLETE=1 · BREAKER_OPEN=5 · 손상감지=6)을 `cmd_restore`가 sys.exit로 방출. **이행 조건**: 전 CLI 소비자 목록 + exit 호환성 매트릭스를 W1 산출물에 포함(deploy 경로의 기존 sys.exit 계약과 충돌 0 확인). cysd 재시도 정책: 비0 시 **1회 지연 재시도(60s), 단 BREAKER_OPEN(5)·손상(6)은 재시도 금지**(폭주 방지) + EVT 이벤트+feed 경고. **재시도 직전 반드시 구조화 liveness(`cys status --json`)·restore lease를 재획득하고 target role을 재산정**한다 — 라이브 사건(§5-1)처럼 master가 수동 복원에 성공한 직후 뒤늦게 도는 60s 재시도가 산 노드를 중복 스폰하지 않도록(수동 복원 완료분은 재산정에서 NOOP로 귀결, codex R2 Issue 5). 스폰 stderr는 null 대신 전용 `phoenix-restore.log`.
- **C2. 손상 대응 — 2단계 계층화** (R1 수용 + R2 정밀화): "파일 없음(fresh install 정상)"과 "파싱 실패(손상)"를 전 로드에서 구분 — **부재는 hard-fail 아님**(빈 초기 상태로 정상 부팅, codex R2 Issue 3 수용). hard-fail은 retention-critical **손상**에만:
  - **1단계 retention-critical**(topology tombstones·desired/dept roster): 손상 시 `.corrupt-<ts>` rename 보존(**최근 3개만 유지·초과 prune** — inode DoS 차단, gemini R2 수용) → **폴백 체인**: 직전 유효본(.bak, 매 성공 write마다 유지) → 세대 스냅샷 tombstones → 전부 불가 시에만 부활 중단+escalation(exit 6). **폴백 복원 = degraded 모드**(gemini R2 "stale-bak 묘비 leak" 수용): 백업은 과거 시점이라 최근 폐역이 빠졌을 수 있으므로, degraded에서는 묘비 상태가 불확실한 역할의 부활을 **보류+escalation**(불확실 시 부활하지 않는 쪽이 fail-safe — 묘비 일원화 원칙과 모순 없게). `unwrap_or_default()`(P0-3)도 동일 체인. **C2-1. degraded 보류 = 영속 sentinel (W3 게이트 발견 · codex W3 blocking · 2026-07-06 설계 확정)**: 폴백 복원이 primary를 유효본으로 교체하고 DEGRADED(exit 3)를 반환하면, C1의 비0 1회 재시도가 2차 실행에서 유효 파일만 보고 보류 없이 부활을 진행하는 **웨이브 간 상호작용 우회**(W1 재시도 × W3 보류 — 보류가 1회성 휘발 상태로 전락)가 생긴다. 확정: **degraded 보류는 프로세스 상태가 아니라 디스크 영속 sentinel**(`pending-degraded` 마커 — 복원 파일과 함께 원자 기록)로 유지하며, auto-retry·수동 restore를 불문하고 sentinel 존재 시 동일 보류를 적용한다. 해제는 명시 ack(`phoenix roster --rebase` 등 운영자 확인)로만 — 시간·재시도 횟수에 의한 암묵 해제 금지. 게이트 subcase: corrupt+.bak → attempt1 DEGRADED → auto-retry 2차도 보류 유지·부활 0 → 명시 ack 후에만 부활 재개.
  - **2단계 보조 상태**(breaker·journal·lease): isolate+warn 또는 안전측 reset — 부활을 막지 않는다.
- **C3. 설명-가능-축소 불변식** (R1 개정 + R2 정밀화): write 거부는 **설명 불가능한 축소**에만 — 감소분이 묘비(OwnerClose)·ephemeral 플래그로 설명되면 정당한 스케일다운. 설명 불가 축소는 영구 교착이 아니라 해당 write 1회 거부+EVT, **다음 관측 사이클 재평가**(gemini R2 TOCTOU 수용: 데몬의 entries 제거↔묘비 등록 사이 지연이 오판을 만들 수 있으므로 1사이클 유예가 자연 해소) + 운영자 명시 재기반(`phoenix roster --rebase`). **데몬 측 대응**: close의 엔트리 제거와 묘비 삽입을 단일 persist로 원자화(W2). 쓰기 실패는 `except: pass` 제거 → log+EVT.
- **C6. 죽은 surface 잔재 자동 정리** (2026-07-06 오너 신규 지시 — "죽은 화면 잔재 제거, 데몬엔 없으므로 안전"): phoenix restore 파이프라인에 **S0 정리 단계** 추가 — ①데몬 관측에서 `exited=true` surface와 데몬에 실체 없는 GUI pane 잔재를 열거 ②**반드시 Reap 사유로 회수**(기존 reap_exited 경로 재사용 또는 W2의 cause 파라미터 — 현행 surface.close RPC는 고정 OwnerClose라 이 기능이 P0-6 함정을 그대로 밟아 역할을 묘비화한다: **절대 금지 경로**) ③라이브(exited=false) surface는 절대 비대상 ④정리 결과를 restore 저널에 기록(몇 개 회수·묘비 0 확인). 수용 게이트: "잔재 정리가 묘비를 1개도 만들지 않음" + "라이브 surface 오회수 0".
- **C4. 전 raw subprocess에 try/except**(TimeoutExpired→rc=124 정직 강등, P1-5) · breaker는 실제 spawn 시도에만 기록+NOOP 리셋(P1-3) · verify done은 verified/fresh만(P2-3) · lease atexit 해제+deploy 내부 lease 우회(P2-7).
- **C5. liveness를 구조화 소스로**: 화면 정규식 대신 `cys status --json`(P1-6). readiness는 배너 리터럴 대신 구조화 ack(P1-10). 하네스는 `CYS_PHOENIX_ALLOW_LIVE=1` 명시 opt-in 없으면 LIVE write 거부(P1-10).

### 축 D. Windows 동등화 (P0-7·P1-8·P1-9·P2-9)

- D1. 축 B3의 절대경로·PATH·PHOENIX_CYS 주입 (첫 스폰 단절 해소).
- D2. lease를 msvcrt.locking/O_EXCL+heartbeat로 — fcntl fail-open 차단(이중 스폰 방지).
- D3. PTY 자식 스폰을 Job Object(KILL_ON_JOB_CLOSE)로 — 동반사망 의미론 mac과 대칭화. CREATE_SUSPENDED→Assign→resume. Job Object 전역 부재는 master 전 소스 grep 재유도로 확정(0건 — R1에서 codex 의심 제기·독립 재검증 종결. windows-audit 전역 검색과 일치).
- D4. CI를 우회 없는 실경로로: 설치된 cysd.exe를 직접 기동해 죽은 역할 부활을 관측(PHOENIX_CYS/절대 python 주입 금지). decide_auto_restore의 인터프리터 해석 Rust 단위 테스트(현 main.rs:1428은 문자열 `"python3"`만 단언 — 결함 통과 테스트). **+R1 보강(gemini Missing 1)**: ConPTY 자식 트리(에이전트 하위 프로세스 포함)까지 Job close 시 종료가 전파되는지 관측하는 실기 E2E 하네스를 W5 수용조건에 포함 — 고아 누적(리소스 폭발) 회귀 차단.

### 축 E. 데몬 교체 시뮬레이션 상시 게이트 (성찰 ③ 제거)

- E1. **격리 E2E 통합 테스트**: 임시 상태 디렉터리에서 cysd 기동 → stub 역할 스폰 → cysd kill(교체 모사) → 신 cysd 기동 → auto-restore가 역할을 부활시키는지 exit code·roster로 관측. mac·Windows CI 필수 게이트. a07cc7f의 Not-tested를 영구 봉인.
- E2. **정기 phoenix drill**: 스케줄 잡으로 저빈도(주 1회) restore --stub 드릴 + 세대 스냅샷 정기화(6h, P2-4) — "실전이 첫 테스트"인 상태 종료.
- E3. 시계 역행 대비: 세대 선택에 mtime·내부 updated_at 병행, GC에 monotonic 시퀀스(P2-5).

---

## 3. 실행 웨이브 (R1 재배치 — codex Issue 2 수용: "관측·검증을 갖추기 전에 실패를 더 세게 만들지 않는다")

| 순서 | 웨이브 | 내용 | 결함 해소 | 위험 |
|---|---|---|---|---|
| 1 | W1 최소 신호 | C1(exit 계약+호환성 매트릭스+재시도 정책) · stderr 로그 · C4(subprocess 가드·breaker·lease) | P0-1·P1-3·P1-5·P2-3·P2-6·P2-7 | 중하(exit 계약은 소비자 매트릭스로 통제) |
| 2 | W4 배포 일체화 | B1(수용조건 4종)·B2·B3 | P0-4·P0-5·P0-7·P1-9 | 중(pack.rs 수술 — 팩 채널 회귀 주의) |
| 3 | W6 E2E 게이트 골격 | E1(교체 시뮬레이션 mac부터)·E2·E3 | P2-2·P2-4·P2-5 + 성찰③ | 낮음 |
| 4 | W2 묘비 일원화 **+ topology 손상 정책 결합** | 축 A 전체(A-S1~S3 포함) + C2 1단계 중 topology tombstones 폴백 체인 | 제3겹·P0-3·P0-6·P1-1·P1-2·P1-4·P2-1 | 중(의미론 변경 — W6 게이트 위에서 검증) |
| 5 | W3 단계적 손상 내성 완성 | C2 잔여(desired/dept·2단계 보조상태)·C3·C5 | P0-2·P1-6·P1-7·P1-10·P2-8 | 중 |
| 6 | W5 Windows 격납 | D2·D3(+ConPTY 자식 트리 E2E)·D4 | P1-8·P2-9 + CI 은폐 | 중~높음(실기 검증 필요) |

**웨이브별 필수 수용 게이트 (R2 개정 — codex Issue 7 수용: wave-매핑 고정, "skeleton과 merge gate 혼동 금지")**

| 웨이브 | 커밋 전 필수 통과 게이트 |
|---|---|
| W1 | exit 매핑 전 경로 결정론(성공≠실패 코드) · TimeoutExpired 강등 · **비0 후 수동복원 상태에서 지연 재시도=NOOP·중복 스폰 0**(lease+liveness 재산정 선행, codex Issue 5) · PHOENIX_CYS 주입 실측 · 폴백 identity-check **3중 대조**(daemon build id+embedded pack manifest hash+phoenix protocol version — §5-1②, 불일치 exit 6+불일치 필드 feed·log, codex R4: generic 표기가 `--version` 단독 등 약한 검증 수용 여지를 남기지 않도록 게이트 행에도 명시) · **C6 탐지·보고 only: 회수 시도 0·묘비 변화 0·라이브 오탐 0**(실제 잔재 회수 게이트는 W2 cause 파라미터/Reap RPC 구현 후로 이동 — §8-2 정합·codex R3 blocking: W1엔 안전 Reap RPC 부재이므로 회수 요구 시 OwnerClose 오묘비 재발 위험) · **STRICT: `PHOENIX_STRICT_CYS=1` 하위케이스 required**(폴백 강제차단으로 Rust PHOENIX_CYS/PATH 주입 검증 — 폴백 정상동작은 별도 non-strict 케이스로 분리, codex R3) |
| W4 | 임베드 추출 성공 · 추출 실패→manifest 검증 디스크 폴백 성공 · stale 디스크 폴백 거부+보고 · self-test 실패 시 silent success 금지 · temp 누수 0 (codex Issue 2) · **missing vs corrupt 구분: fresh-install(전 상태파일 부재) 정상 부팅**(codex Issue 3) · **corrupt desired silent-empty 차단 sentinel**(손상 desired가 fresh-install missing으로 위장해 빈 상태로 통과하는 것 금지 — 현행 load_desired_roster는 corrupt를 missing과 동일 빈집합 처리 py:295-303, 이를 sentinel 테스트로 분리 차단, codex R3) · **STRICT: `PHOENIX_STRICT_CYS=1` 하위케이스 required**(B1 폴백이 근본수리 실패를 가리는 false-green 위험 최대 웨이브 — 폴백 정상동작은 별도 non-strict 케이스로 분리, codex R3) |
| W6 | 교체 시뮬레이션 E2E(mac): kill→재기동→부활 관측 · **STRICT: `PHOENIX_STRICT_CYS=1` required**(Rust 주입 검증 — 폴백이 B3 결함을 가리는 false-green 차단, codex R3) · no-target-roles NOOP · **cross-layer 재시도 증거(격리 하네스): attempt1 비0 → 라이브 상태 변경 → attempt2 실 run_restore가 NOOP·spawn 이벤트 0**(Rust 단위 seam 증명을 상위 체인으로 봉인 — codex W1-R2 minor #4 이월·구현 트랙 제안 2026-07-06 채택) |
| W2 | corrupt topology(A-S1 rev 가드) · legacy topology(키 부재)=경고+진행 · launch 롤백 close가 묘비 미생성(cause=Reap) · 부활 master approval.sign 정상 · **legacy migration 결정표**(codex R3 — metadata 無 `*-fresh-*`=quarantine 기본 · metadata 無 비-fresh=보존 · source/created_at 부재로 판정 애매=부활 보류+escalation): 현행 live-role 병합은 metadata 없이 `{'role':role}`만 저장(py:348)하므로 결정표로 판정 명문화 · **C6 실제 잔재 회수(Reap 사유 전용): 라이브 surface 오회수 0·회수분 묘비 0**(W1에서 이관 — cause 파라미터 구현 후 실행, codex R3 blocking) · **tombstones_rev 역행·초기화 강제 rebase**(gemini R3 — `.bak` 폴백·fresh install(rev=0)·데몬 epoch(started_at) 변경으로 rev가 phoenix 인지 rev보다 작아지면 replace 데드락 금지: 데몬 epoch 변경 또는 rev 명시 0 리셋 감지 시 phoenix 인지 rev를 강제 rebase) · **intent 저널 절단은 topology.json 디스크 영속 확인 후에만**(gemini R3 — RPC 응답 시점 절단 금지: 갱신 tombstones_rev 담긴 topology.json이 디스크에 기록된 것을 phoenix가 로드·확인한 후 절단, 데몬 크래시 시 묘비 소실 차단) · intent 저널 TOCTOU 시나리오 · **untomb 부활 복원(A-S4)**: 묘비 적용=엔트리 보존+target 산정 제외 → 해제 시 즉시 부활 — 실제 상태(엔트리 보존+tombstones=['worker']) subcase 필수·pop-후 fixture false-green 금지(codex W2 blocking) · STRICT: 해당 없음(묘비 의미론 — cys-해석 non-strict) |
| W3 | corrupt desired/dept(폴백 체인+degraded 부활 보류) · **corrupt와 missing이 다른 exit/event**(corrupt desired/dept가 missing과 달리 silent-empty로 통과하지 않음 — corrupt=이벤트+degraded, missing=정상 빈 부팅, codex R3 required) · corrupt breaker(리셋+경고) · .corrupt 3개 초과 prune · 설명-가능-축소(정당 스케일다운 통과·불가 축소 1회 거부 후 재평가) · **intent 저널 절단 순서 준수**(topology 디스크 영속 확인 후 절단 — gemini R3) · deploy 중첩 lease · **schedule 잡 분리(B2-1): 팩 강제갱신 후 사용자 잡 생존 · built-in 잡 부트 idempotent ensure(중복 생성 0·버전업 갱신)** · **B2-1 id 충돌 정책(codex W3): `_builtin` 마커 항목만 ensure 갱신 — 동일 id 비-builtin 사용자 잡=conflict 보존+경고, reserved id는 CLI add 단계 거부 — 동명 사용자 잡 테스트 필수** · **C2-1 degraded 영속 sentinel: attempt1 DEGRADED → auto-retry 2차도 보류 유지·부활 0 → 명시 ack 후에만 재개**(codex W3 blocking) · **C5 list 폴백 fail-closed: stdout 비어있지 않은데 파싱 0건=UNKNOWN_LIVENESS(DEGRADED) — 전 역할 죽음 오판·대량 restore 금지, malformed·컬럼 변경 케이스 게이트**(codex W3 · P1-6 재발 차단) · **손상 매트릭스: retention 파일(desired·dept)×상태(missing·corrupt+bak·corrupt+snapshot·unrecoverable) 전 셀의 exit/event/파일상태/차기 retry 동작 단언**(codex W3) · STRICT: 해당 없음(cys-해석 비의존 웨이브 — non-strict) |
| W5 | Windows 자식 고아(Job Object 전파, ConPTY 자식 트리 실기 E2E) · msvcrt lease 이중 스폰 차단 · CI 실경로(주입 우회 금지) · STRICT: 해당 없음(Windows 격납 웨이브 — cys-해석 non-strict, 단 CI 실경로의 주입 우회 금지는 유지, codex R3) |

*표 판독 규약(required vs defined-later — codex R2 Issue 7)*: 각 행의 게이트는 **해당 웨이브 커밋 전 반드시 통과해야 할 required subset**이다. 상위 웨이브에서만 실행 가능한 게이트(예: Windows 자식 고아=W5·corrupt desired=W3 구현 후)는 그 구현 웨이브에서 **정의(defined-later)** 되며 조기 웨이브 커밋의 green 기준에 포함되지 않는다 — skeleton(W6 골격)과 merge gate를 혼동해 green 기준이 흔들리는 것을 차단한다.

각 웨이브 종료 게이트: agy+codex 리뷰(verdict 계약) + master 독립검증 + 기계검증(격리 테스트) 4자수렴 → 로컬 커밋. 외부 발행(release·push)은 CI/CD 완주 명령(2026-07-06)에 따라 master가 집행하되 PII 하드게이트·가드 통과가 전제.

## 4. 즉시 조치 완료분 (이 설계와 별개로 이미 집행)

- 오분류 묘비 4개(cso·worker·reviewer-codex·reviewer-gemini) `tombstone --remove` 해제 → desired_roster tombstones=[] 검증 완료. 함대 5역할 전원 로스터 복귀·생존.

## 5. 결정 확정 (2026-07-06 오너 "모든 승인 사항 master 최고 선택" 위임 → master 확정)

1. **축 A = 옵션 A** (데몬=묘비 유일 진실) + R1 안전조건 A-S1~S3. 근거: 작성자 일원화·자동 화해·수동 해제 불요화. gemini의 싹쓸이 공격은 A-S1(검증된-건강 replace)로 무력화, 이원화 공격은 소켓별 격리 실측으로 반박 종결.
2. **B1 범위 = phoenix 우선** (실증 사고 지점). 훅 python 8종은 B2 소유권 매니페스트로 커버 후 관찰.

## 5-1. 라이브 사건 실증 (2026-07-06 09:38 — 설계 유효성의 실전 증명)

R1 진행 중 데몬이 재교체(pid 54067)되며 함대 전체가 전멸, auto-restore가 exit=1로 침묵 사망(묘비 무관 — tombstones=[] 확인). master가 stderr 캡처 수동 복원으로 함대 5/5 부활시키고 원인을 결정론 확정:
- **exit=1의 정체 = `FileNotFoundError: 'cys'`** — GUI 기동 데몬의 PATH가 `/usr/bin:/bin:/usr/sbin:/sbin`뿐이라 `/opt/homebrew/bin/cys`를 못 찾고, phoenix의 `CYS = PHOENIX_CYS or which("cys") or "cys"`(javis_phoenix.py:1981)가 리터럴 폴백 → 첫 subprocess에서 미포착 예외 → traceback → exit 1. **데몬 동등 env 재현으로 실증 완료**. 이는 windows 감사 P1(PHOENIX_CYS 미전달)의 mac 실증이자 cysd-rust 감사 P2-B(최소 PATH)의 적중 — B3(절대경로+PHOENIX_CYS 주입)가 고치는 바로 그 지점. (초기 "P1-5 TimeoutExpired 유력" 추정은 본 확증으로 정정 — P1-5는 별개 잠복 결함으로 유효.)
- 부수 실증: worker 부활 시 ready-marker 판정 흔들림(P1-10 readiness 취약)도 라이브 관측. stderr가 null이라 사후 진단이 불가능했던 것 자체가 P0-5의 실전 비용.
- **W1 임시 완화 항목 (R2 개정 — codex Issue 1·gemini side-effects 수용)**: ①**주 수단 = PHOENIX_CYS+PATH 주입 Rust 소패치를 B3에서 W1로 승격**(spawn_auto_restore에 env 2줄 — 소수술이므로 W1 적합) ②phoenix 파이썬 `_which` 표준경로 폴백은 **identity check 통과 시에만** — `--version` 단독이 아니라 **daemon build id + embedded pack manifest hash + phoenix protocol version 3중 대조**(같은 version 문자열의 다른 빌드·다른 embedded pack·다른 protocol 조합이 통과하는 구멍 차단 — B1 '같은 커밋 하드 보장' 원칙과 정합, codex R3). 불일치=exit 6 유지하되 **어떤 필드가 불일치했는지 feed·log에 명시**(다중 설치 스큐·구버전 실행 차단) ③하네스(--socket/격리 env) 실행 시 폴백 비활성(테스트 독립성) ④E2E 게이트는 `PHOENIX_STRICT_CYS=1`(폴백 강제 차단)로 실행해 Rust 주입 자체를 검증(폴백이 B3 결함을 가리는 false-green 차단 — gemini 지적 수용).

## 6. R1 라운드 기록 (2026-07-06)

- **codex: REVISE** — 표본 10건 전건 verified. 지적 7건 중 6건 수용(C2 계층화·웨이브 재배치·B1 수용조건·W2+topology 결합·W1 호환성 매트릭스·fresh ephemeral 플래그), 1건 반박 종결(P2-9 Job Object 전역 부재 — master 전 소스 grep 재유도 0건).
- **gemini: REVISE** — 표본 5건 전건 verified. Issue 1-시나리오1(topology 손상 replace 싹쓸이)=수용(A-S1), Issue 1-시나리오2(이원화 상호 덮어쓰기)=**반박 확정**(desired_roster는 phoenix_home(socket)별 격리 — javis_phoenix.py:145·156·291 실측), CLI 폴백 보존=수용(A-S3), Issue 2(C3 스케일다운 오작동)=수용(설명-가능-축소로 개정), Issue 3(B1 stale 권한)=수용(버전+uuid 격리 경로), Missing 1(ConPTY 자식 트리 E2E)=수용(D4 보강).
- 본 문서는 위 수용·반박을 전부 반영한 **v2**다. R2는 v2 재검증으로 진행.

## 7. R2 라운드 기록 + 종결 판단 (2026-07-06)

- **gemini R2: REVISE** — 단 R1 반박(이원화 격리)은 **명시 ACCEPT**. 잔여 공격 전건 수용: A-S1 부분손상 통과→tombstones_rev 단조 카운터 / legacy 키부재→무기한 보류 대신 경고+진행 / A-S2 태그=canonical state dir / A-S3→intent 저널 재설계(TOCTOU 소멸) / C2 stale-bak leak→degraded 부활 보류 / .corrupt DoS→3개 cap / C3 TOCTOU→1사이클 재평가+데몬 원자 persist / side-effects 3건(다중설치 스큐→identity check·하네스 오염→폴백 비활성·B3 은폐→PHOENIX_STRICT_CYS 게이트).
- **codex R2: REVISE** — R1 수용 6건의 v2 반영 **전건 verified**. 잔여 지적 7건 전건 수용: _which 스큐→W1 Rust 소패치 승격+identity check / W4 게이트 5종 추가 / missing-vs-corrupt fresh-install 게이트 / legacy fresh-* quarantine / 지연 재시도 중복스폰 가드 / A-S3 동일정책 / wave별 게이트 표 고정.
- **오너 추가 지시 반영**: C6 죽은 surface 잔재 자동 정리(Reap 사유 강제 — P0-6 함정 회피 명문화).
- **종결 판단(master)**: R1→R2에서 지적 고도가 아키텍처→정책→게이트로 단조 하강, 양 리뷰어가 상호 독립적으로 반영 정확성을 확인. 설계 라운드 **종결(v3 확정)** — 이후 검증은 웨이브별 4자수렴 게이트에서 계속(리뷰 루프는 구현 단계로 이관). 본 문서 = **v3(구현 기준본)**.

## 8. 구현 착수 전 최종 성찰 (오너 명령 이행 · 2026-07-06)

1. **원 결함 대비 완결성 재확인**: 3겹(팩 스큐→W4 / 오묘비→W2 / add-only→W2 옵션A) + 라이브 사건 2건(FileNotFoundError→W1 / 침묵실패→W1) + 26결함 전건이 웨이브에 매핑됨 — 대응 공백 0 확인.
2. **설계 자체의 잔여 리스크 2건과 대응**: ①이 리포는 병렬 세션이 공유한다(v0.12.15 태그 경합 사고 전례) → 구현은 **전용 브랜치(phoenix-hardening)+별도 worktree**에서 진행, 웨이브 게이트 통과 시에만 main 로컬 머지 — 트리 경합 원천 차단. ②C6(잔재 정리)은 W1 시점에 안전한 Reap RPC가 없으면 **탐지·보고만** 구현하고 실제 회수는 W2(cause 파라미터)와 함께 — OwnerClose 오묘비 함정(P0-6)을 새 기능이 밟지 않게 순서 강제.
3. **가장 경계할 자기기만**: 폴백·완화 장치가 근본수리를 가리는 false-green(리뷰어 양쪽이 독립 지적) — 전 웨이브 게이트에 STRICT 모드 검증을 유지한다.
5. **미결(open question · 결정=오너·master, gemini R3 advisory)**: degraded 모드(C2 폴백 후 묘비 불확실 역할 부활 보류)의 **자동 자가치유 복귀** 여부 — 주기적 attest 재시도로 정상 topology를 재확인하면 자동으로 degraded 해제·부활 재개할지, 아니면 운영자 `phoenix roster --rebase` 수동 복귀만 둘지. 자동 복귀는 무한 보류 DoS(gemini R3 minor)를 완화하나 손상 진동 시 부활 플랩 위험 — **현 설계 기본값 = 수동 `--rebase`(fail-safe)**, 자동 복귀는 미채택(오너·master 결정 대기).
4. 결론: 설계 고도에서 더 짜낼 것 없음 — 구현 단계의 4자수렴 게이트가 다음 방어선. **W1 착수.**
