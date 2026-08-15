# cys-terminal 수리 설계 — 3라운드 가상 시뮬레이션 최종 보고 (설계 v4)

> 2026-08-16. 대상: 문제1(유령부서發 껍데기 CEO 승격) · 문제2(맥 fullscreen 휠→프롬프트 히스토리 오염) 수리 설계.
> 방법: 구현 전 가상 시뮬레이션 3라운드(총 13개 시뮬레이터, 도구 호출 576회, 전부 읽기 전용 실코드 대조).
> 결론: **설계 v4 확정 — 조건부 구현 착수 가능** (조건 = 오너 결정 레지스터 G0 확정, 최소 P0 택1).

---

## 1. 라운드 구성과 수치

| 라운드 | 내용 | 발견(비-PASS) |
|---|---|---|
| R1 | 설계 항목별 데스크체크(D1~D5, 5개 병렬) | DESIGN_DEFECT 9 · SPEC_GAP 15 · RISK 8 (critical 2) |
| R2 | 교차 여정(win 온보딩·mac 마이그레이션)·앵커 스트레스·오케스트레이션 시맨틱·회귀 매트릭스 | DESIGN_DEFECT 8 · SPEC_GAP 14 · RISK 9 (critical 0, 신규 major 다수) |
| R3 | v3 적대 재검증·완전성 비평·추적성 전수 대조 | v3 신규 결함 5(critical 1) · 프로그램 결여 5절 · 추적성 96건 중 UNADDRESSED **0** |

추적성 최종: **ADDRESSED 78 / PARTIAL 15(→v4에서 문안 반영) / DEFERRED_OK 3 / UNADDRESSED 0**.

## 2. "구현했으면 사고났을 것" 대표 10건

1. **[R3·critical] A3×A7 결합 모순**: fan-out 루프를 `sock -- "$d"`로 위생 정정하면 sock 동사가 `--`를 해석하지 못해 **전부서 방송이 전 함대에서 항상 실패**하고, 드리프트 게이트가 그 문안을 바이트 봉인. → v4: 루프는 `sock "$d"` 유지(name 검증만으로 안전), 재합성 전 스니펫 실행 스모크를 gen --check에 추가.
2. **[R1·critical] 핀-디렉티브 패리티 붕괴**: preflight의 2026-08-01 재산정 핀 42건(master 6·worker 7·cso 17·reviewer 12)이 현행 출하본과 8/14 `.new` 어느 쪽에도 없음 — **신선 설치조차 C03 4종 FAIL**이고 'D1(a) 채택 후 녹색' 전제가 무효. → P0 선행조건 신설.
3. **[R2·major] `.pristine`은 '갱신 후' 미러**: 구판 승격 판정을 `.pristine` 등가로 하면 판정이 설계된 유일한 상황(템플릿 전진 릴리스 직후)에서 항상 거짓 — **전 기승격 머신 위경보**. → 머리글 핀 표지+승격 영수증으로 교체.
4. **[R3·major] 표지 핀 4번째('직접 구현 금지')는 구 템플릿에 부재**(grep 0건 실측) — 그대로 쓰면 3번 수리가 재발. → 표지 핀은 구·신 공통 3핀({master of master·단일소유 강제·exit 7})으로 한정, "표지 핀은 구·신 양쪽 템플릿에 실존해야 한다"를 CI 불변식으로 gen --check에 단언.
5. **[R1·major] rotate 반파괴 half-op**: 정리 동사 rotate의 꼬리가 생성 동사 launch 재귀라, 비정형 등재명에서 **kill 후 재기동 거부** — 데몬 죽고 등재 남는 상태. → 진입부(kill 이전) 사전 검증.
6. **[R2·major] 화이트리스트 '.' × Windows pipe_slug 점 소거** = 점만 다른 부서의 상태 디렉토리 병합(격리 붕괴). → '.' 제외: `^[A-Za-z0-9][A-Za-z0-9_-]*$`.
7. **[R3·major] A2 자기 충돌**: slug-fold 중복 거부에 자기 제외 조항이 없으면 **모든 기존 부서의 재기동(GUI 복원·rotate)이 거부**됨. → `fold(신)==fold(기존) ∧ 신≠기존(바이트)`일 때만 거부 + '기존명 재-launch 통과' 회귀 핀.
8. **[R2·major] forward 마우스 보고의 human 위장**: 오너가 pane을 스크롤해 읽는 동안 `--queued` 배달이 무기 연기(큐 적체 앵커 위반)·seat 판정 오염 — 기존 mac 클릭 경로에도 있던 잠복 결함. → 데몬측 last_human_input 면제(A9).
9. **[R3·major] A8 DECSTR 오판**: xterm에서 DECSTR은 버퍼·마우스 프로토콜을 건드리지 않는데(벤더 InputHandler.ts 실측) alt 복귀로 처리하면 **정당한 트래킹 앱이 마우스를 잃고 원 결함 부활**. → 전이 감시에서 DECSTR 제거, RIS만 유지(crash-reset 동선은 RIS가 전담).
10. **[R2·major] C03 ① 조기 PASS가 강등-불능 검출을 영구 차폐** + ceo_demote 무음 no-op = **비가역 상태 잠복**. → 핀 축과 독립인 `C03.demote-guard`(WARN) 신설 + demote 무음 실패에 경보.

또한 R1이 P0 택(a)/(b)의 상충을 발견(택(a)=§7 점수 루프↔REVIEWER verdict 계약 모순까지 해소하는 유일안 / 택(b)=갱신 함대 즉시 PASS이나 모순 화석화)해 오너 결정 D1으로 상정했다.

## 3. 최종 설계 v4 (v3 + R3 교정 전부 반영)

### P0 — 핀·디렉티브 패리티 복원 (선행조건 · 크리티컬 패스)
- 택(a) **[권고]** 재산정 핀 문안을 담은 디렉티브 개정 4종을 같은 릴리스에 동봉 — §7 점수 루프 vs REVIEWER §3 score 금지 모순까지 해소. 이행 완화 동봉 필수: C03 FAIL detail에 ".new 병치 대기 — `cys pack-merge --file <f> --take-new`(무수정본 한정)" 분기 문안. (선택·오너 결정 D2) constitution 무수정본(.pristine==live) 한정 자동 전진 예외 — 이번엔 미채택 권고, 후속 티켓.
- 택(b) 핀을 현행 출하 문안 리터럴로 정정 — 채택 시 §7 점수 문안 최소 정정을 같은 릴리스에 동봉(원자 묶음) + FAIL 문안에 pack-merge 안내.
- 공통: `bin/tests/test_content_pins_parity.py`(CONTENT_PINS⊆임베드 디렉티브·CEO 핀 포함) + RULE_MARKERS·FALLBACK_RULES↔worker 핀 상호 포함 CI 상주 + FAIL detail 인용 규범 실재 단언 + encoding='utf-8' 명시. 한계 주석: 패리티=존재 검사(모순 검사 아님).

### D1 — 운영 절차 (코드 0 · 오너 승인)
(a) **v4 릴리스 설치 후에만 실행**(선채택 금지 — 이중 채택+.pre-ceo 구본화. 확인: `.new`가 신본으로 재기록됨을 cmp로 확인). base: WORKER=`pack-merge --take-new`, MASTER=①`cp MASTER_DIRECTIVE.md.new → .pre-ceo` ②`pack-merge --keep-mine`(**역전 금지** — keep-mine이 .new를 삭제). P0(a) 시 CSO/REVIEWER도 take-new(base·dept-1 공통). dept-1 팩: 승격 개념 없음 — 전부 live take-new + acl.json.new 오너 diff 검토(결정 D6). 채택 후 기동 중 WORKER 노드 재각성(javis_boot_node).
(b) 유령: `down-sock <유령 sock 경로>` → **rm 대신 mv**로 `~/.local/state/cys-trash/--help-<ts>/` 격리(TTL 14일) → 각 단계 전 `ls -d` 정확 경로·glob 금지·사전확인=sock 부재+pgrep 0. `--help` 문자 이름은 down-by-name 불가(name 검증 exit 2) — 수동 격리가 정본, 묘비 불요(복원 루프가 미등재+死 소켓 드롭).
(c) feed 9건: stale 7건=deny, 승격 알림 2건=allow(**cys pane 또는 GUI에서** — sid 앨리어싱 거부 시 GUI 재시도).
(d) fullscreen 정책 SOT=settings 정규화(전 계정 dir tui 키 제거/inline·preflight ensure+.bak 관례·오너 승인 1회 — 결정 D5) + D5 env 방어층 병행. 매뉴얼 4.x에 .bak 관례 1줄.

### D2 — CEO_TEMPLATE 합성 확장
- repo-side generator `scripts/gen_ceo_template.py`가 [CEO 머리글 fragment(원문·선두 — delivery.rs `CEO_TEMPLATE.md:22` 좌표·H-DOC-3 토큰 보존)] + [합성 서문(R2 산출 600자 실문안 — 서열 머리글>서문>본문·직할/부서 위임 판단 트리·"직접 구현은 §1-A 사소 예외 없이 금지"·하트비트 병기 3항·자원 관할 분리·RSI 범위·§11 유효·리뷰어1=적대/리뷰어2=감사·verdict 계약)] + [구분선] + [MASTER 본문 바이트 무수정]을 연접·커밋(빌드타임 변환 금지). 머리글 todo 절→§9 포인터 1줄 물리 치환.
- **fan-out 루프는 `s=$(cys-dept sock "$d")` 유지('-- ' 넣지 않음)** — A3 검증만으로 안전. gen --check에 ①드리프트(재합성==커밋본) ②표지 핀 구·신 템플릿 공통 실존 ③템플릿 내 bash 스니펫 실행 스모크를 단언, ci-branch·release CI에 배선.
- fragment 소스 위치 택1 명기(scripts/ 비출하 권고), check_manifest.py 오참조 삭제, manifest·서명 추가 조치 불요 명기.
- CONTENT_PINS["CEO_TEMPLATE.md"]에 CEO 핀 신설: {master of master·단일소유 강제·exit 7·직접 구현 금지} — **이 중 md 승격 '표지' 술어에는 구·신 공통 3핀만 사용**('직접 구현 금지'는 라이브 템플릿 파일 검사 전용). 각주: 주입 시 compose_directive가 RSI지침·soul.md·메모리·스킬 색인 후첨(드리프트 게이트는 파일만 비교). part-cap CEO 케이스(기대 ≈780 제출단위·3,120≤4,000) + CONTRIBUTING 'Directive edits' 절(재합성 필수·차단 동사 백틱 호출형 금지·part-cap 로컬 실행). 주입 예산 표기 "총 ≈120KB 중 디렉티브 델타 +6KB". P0(a) 시 개정 MASTER 자원 게이트 조항↔서문 중복 점검을 게이트 체크리스트에 1항.

### D3(i) — cys-dept 이름 검증
- 화이트리스트 `^[A-Za-z0-9][A-Za-z0-9_-]*$`('.' 제외 — win pipe_slug 충돌 봉합)·≤40자·LC_ALL=C.
- slug-fold 중복 거부: **거부 = fold(신규)==fold(기존) ∧ 신규≠기존(바이트)** — 자기 제외. 검사 지점=reg_upsert와 allocate 예약 블록 공통 헬퍼. 기존 충돌 쌍은 preflight WARN 가시화+개명 절차 runbook. 회귀 핀: 충돌 쌍 거부 + **기존명 재-launch/rotate 통과**.
- `--help`/`-h`: **cmd 위치만 usage(stdout·exit 0·가드 이전·역할 무관)**. name 위치 '-' 선두는 전부 exit 2(stderr — 진단에 해당 동사 usage 1줄 병기). name = **각 동사의 플래그 해석 완료 후 첫 위치 인자**($2 직검증 금지 — down --purge-state 보존, 회귀 핀 1건).
- 생성 동사(launch/create/allocate): name 확보 직후·일체 부작용 이전 검증. create는 verb/name 분해 직후(3분기 공통 관문) + **카탈로그 key도 동일 화이트리스트**. REUSE_* 불합격='비정형 등재' stderr 보고 exit 2(자동 삭제 금지).
- rotate: 진입부(등재 게이트 직후·kill 이전) 사전 검증 — 불합격 시 kill 없이 exit 2. launch는 CYS_DEPT_ROTATE=1이어도 검증 유지. passthrough arm: 정리 동사형(실존 허용·비실존+비정형 exit 2) + 비실존·정형 이름에 CYS_NO_AUTOSTART=1 동봉 여부는 결정 D10(동봉 권고).
- 인자 위생: `cys tombstone --dept [--remove] -- "$name"`, 모든 grep 멤버십 `grep -Fxq -- "$name"`.
- exit 계약: 검증 거부=2(기존 usage 계열과 동일). **재사용 금지 집합={1,3,4,5,6,7,8,9}**(전 기사용 코드 실측 반영). 가드(exit 7)는 검증보다 선행 유지, cmd 위치 --help만 가드 이전.

### D3(ii)+A6 — preflight C03 재구성 (두 축 분리)
- [핀 축] ① md에서 master 핀 전수 통과=핀 PASS(승격 무관·주권 편집 허용 — '개행 변경도 비정형 판정됨' 문안 포함). ② 승격 표지 감지 = md==라이브 템플릿 등가 **또는** (영수증 해시 등가) **또는** (md가 표지 3핀 전수 통과 ∧ .pre-ceo 존재): md≠라이브 템플릿이면 '정상 승격(구판)' WARN+promote-ceo 재실행 안내. master 핀은 .pre-ceo에 검사(검사 표면 파일 명시). ③ .pre-ceo 존재 ∧ master 핀 실패 ∧ 표지 핀도 실패='비정형 승격 상태' FAIL(파괴 금지·cmp 첨부·promote-ceo 분기 안내). ④ md==템플릿 ∧ .pre-ceo 부재='복원 백업 소실(강등 불능)' FAIL+.pristine 재건 안내.
- [건전성 축] `C03.demote-guard`(WARN·핀 결과와 독립): 승격 표지 시 항상 — .pre-ceo 존재(부재면 후속 단락·④와 중복 발화는 의도) / cmp(.pre-ceo,템플릿) **퇴화 검출 시 ②의 '구본화' 분류 억제·'승격 백업 파괴' 전용 분류로 승격**(WARN 등급 유지 근거=부트 비치명 계약 명기) / .pre-ceo 핀 전수(부족 ∧ 동일 핀이 .new에 존재 → '구본화—신본 채택 대기').
- 승격 영수증(결정 D3·채택 권고): 위치=**pack 트리 내 `directives/.ceo-template-applied`**(Tier1 백업 원자성 — ~/.cys/state 금지). _swap이 기록·**ceo_demote가 삭제**. 판정 우선=영수증 해시 등가 > 표지 핀 폴백, stale 영수증(부등)=무시.
- 상태 지문 = (md 해시, 주축 분류, **.pre-ceo 해시/부재, demote-guard 3항 비트**) — demote-guard도 dedupe 편입하되 지문 변화 시 즉시 전문 재발화. 오너 통보는 지문 변화 시에만.
- ceo_demote의 .pre-ceo 부재 무음 return 0 → stderr 경보+feed push. 전 역할 공통 원인 분류 1줄(live==.pristine → '무수정 배포본—핀 기준 선행/.new 병치 대기'). C03 무-fix 회귀 테스트. dept 면제 early-return 선행 불변.

### D4+A8+A9 — trackfilter 정합기·휠 억제·human 면제
- 장부=앱 희망 트래킹 상태(앱의 명시 DECSET/DECRST로만 갱신). **alt 전이 감시 집합={47,1047,1049}h/l + RIS(ESC c)**(alt 복귀+장부 소거 동일 처리). **DECSTR(CSI ! p)은 제거** — xterm 의미론상 버퍼·마우스 무접촉(역회귀 핀: alt+트래킹 중 CSI ! p 후 상태 불변). 결합 CSI=재조립분 방출 직후 주입.
- (mac 소비) alt 진입 시 희망 트래킹 재생 주입(1049h 방출 직후 위치)·alt 중 통과·복귀 시 MOUSE_PARAMS 8종 전 집합 상수 DECRST·스트리핑 재개. **상수 주입은 필터 우회 term.write 직접**(자기 스트리핑 차단). Windows=장부 기록만 공통·소비 전부 비활성(win-parity 바이트 핀+휠 핸들러 win 미등록 단언). OS는 trackfilter 생성자 opt로 캡처 주입.
- 휠 억제(mac·attachCustomWheelEventHandler): alt buffer ∧ 장부상 트래킹 요청 ∧ xterm 트래킹 미진입 ∧ pane 캡처 allowAppMouse=false → 소비. less/man 방향키 보존.
- 리셋 훅: socket_slug 실해석 성공 pane만(기본 데몬 폴백 금지·main.ts:5194 선례)·해석 실패 시 생략·exited_event(:2128)=최종 안전망(flush→리셋 주입→배너 순서 고정)·에포크 가드(이벤트 후 앱 명시 전이 관측 시 장부 소거 생략).
- **롤백 스위치**: 정합기 전체 비활성 localStorage 플래그 1개(win 비활성 코드 경로 재사용 — 신규 로직 0).
- A9(데몬측·D4 DoD에 "데몬측 예외 1건" 명시): 수신 텍스트 **전체가 마우스 보고 시퀀스의 연접일 때만** last_human_input 갱신 생략(classifyMouseReport 동형·`\x1b[200~` 접두는 무조건 비면제). TS 코퍼스를 공유 픽스처로 추출해 Rust 매처 패리티 핀. 회귀 핀='순수 보고(휠·클릭·모션 전 인코딩) 미갱신·비순수 갱신'.
- E2E 수용 기준: 단일 attach 세션 내(재attach 모드 복원=후속 게이트). 트레이드오프 명기: alt 트래킹 중 일반 드래그 선택→Option+드래그. 'vim→reset(RIS)→less' 케이스 + codex pane 케이스.

### D5 — env 게이트·관측
- 주입: agent_env_pairs 단일 SOT — 키 부재 시에만 (CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN,"1") 삽입(contains — append+sort로 사용자 "0" 뒤집기 금지). 게이트=cfg!(macos) ∧ extract_bin(cmd)=='claude'. 회귀 핀은 두 소비처(인라인 문자열·surface.create env 맵) 모두. **핀 활성화 순서**: 주입 로직 lib 헬퍼화→기존 --lib 레인 편승을 1순위, `cargo test --bin cys` 전체 활성화는 ci-branch 선행 1사이클 green 후 release 승격(112개 휴면 테스트 직행 금지·--skip 패턴 예약).
- 관측: 데몬 vt100 alternate_screen() → surface `alt_screen` bool — **surface.list·org.status 동시 노출+동형성 핀**. launch-agent ready 직후 mac ∧ claude ∧ alt_screen=true → stderr WARN. 필드 부재=판정 불가·WARN 생략(FAIL 금지). win은 힌트 1줄(경보 아님). 일 집계는 fleet digest에 1줄 편입.
- 부트 E2E 4신호(ⓐ델타 marker readiness ⓑ8s 내 awakened_at ⓒ2차 에코 ⓓalt_screen=false) — 자동화 배치처=릴리스 런북 수동(이번)+actprobe 프로브(후속·결정 D7). E2E에 '부서장 빈 셸 수동 claude' 1건 + env×settings 4조합 중 2조합 실기.

### A11~A16 (유지·정정)
- A11: ⑦ request-only feed dedupe(미해결 동종 존재 시 push 생략).
- A12 runbook(**단일 v4 릴리스로 판번 통일**): ⓪채택·정리는 릴리스 설치 후 → ①설치 → ②같은 세션 promote-ceo 재실행(오너 role-less 셸/CSO CLI — CEO pane exit 7은 계약·팔레트 확장은 결정 D4) → ③D1(a) → ④dept-1 → ⑤D1(b)(c)(d) → ⑥부트 4신호+최종 체크리스트(팩 등가·**.pre-ceo 핀 전수 스팟체크**·원장 0·feed 0·전 PASS). 코드 가드: pack-merge **및 pack-rollback**이 rel==MASTER ∧ .pre-ceo 존재 시 --take-new/후진 거부(오너 override 별도).
- A13(win): mkdir 원자 락 폴백. Windows 검증 절 — T3 확장(D3 거부 4종 — **launch --help 기대값은 'name 위치=exit 2·부작용 0'으로 갱신**·P0 전 PASS·CEO_TEMPLATE 바이트 대조·**승격 스모크(마커 시드→promote→cmp→down 원복)**·alt_screen·win-parity·agents "0" override)를 릴리스 차단 게이트로 하되 **태그 파이프라인과의 기계 결합(또는 런북 수동 순서)을 명시 배선**. win fullscreen 옵트인=비지원 선언+힌트.
- A14(테스트·CI): R2 SIM-J 배치 목록 ①~⑪ + R3 정정(cys-dept 테스트 기대값 갱신·A9 패리티 픽스처·3레인 allowlist·레인 대조 ALLOWED 갱신).
- A15(문서 DoD): 4.6b 재작성·env 표·릴리스 노트 알려진 제한(win fullscreen·win vim·재attach·ssh 절단 reset)·CONTRIBUTING·D1(d) .bak 1줄 — D4 DoD(미개정=미완). 대상 일반화("마우스를 요청하는 모든 전체화면 TUI").
- A16: ssh 유휴 소등 휴리스틱=후속. 교차 레인 원장 접두=runbook 각주. depts.json mission 필드=중기. 스킬 색인 다이어트=백로그.

### 신설 절 (R3 완전성 비평 반영)
- **롤백**: D2=「직전 릴리스 재설치→같은 세션 promote-ceo 재실행」(md 수동 복귀 금지 명문)+다운그레이드 설치 리허설 1회를 릴리스 게이트에. D4=정합기 비활성 플래그. D5=전 계정 agents.json "0" 일괄 스크립트. 릴리스=이전 자산 재게시→SHA256SUMS 재갱신→원격검증 재실행→updater 채널 처분.
- **릴리스 시퀀싱**: win T3 green 증적→태그(mac)→win 빌드→WDSI→공증→**전 자산 일괄 업로드**→SHA256SUMS→원격검증 6항. updater 공개는 본 머신 runbook 완주 후(또는 WARN 창 수용을 결정 기록). 팩=양 OS 동일 임베드(머신 간 스큐 저위험) 근거 명기.
- **사후 검증(14일)**: C03 FAIL 0·지문 변화 0 / 미등재 state dir 신규 0 / 히스토리 오염 재현 0 / override 발동 0. fleet digest에 C03 요약+alt_screen WARN 건수 1줄. 오너 확인 D+1·D+7 문답 4항(fullscreen 휠=트랜스크립트 스크롤·오염 0·Option드래그 복사·vim/less 무회귀). 차기 릴리스에서 '구본화' 분류 발화를 E1 재발 검증으로 예약.
- **WBS**: G0(결정 레지스터) → Phase1 병렬 [T1 P0(원자: 개정+완화 문안) / T2 D2(원자: 재합성+fan-out 문안+게이트) / T3 D3(i) / T4 C03(의존 T1·T2) / T5 A9 / T6 D5 / T7 D4 3분할(a 정합기 코어→b 리셋 훅→c 휠 억제) / T8 dedupe+가드+영수증 / T9 win / T10 문서(=T7 DoD)] → G1(전 테스트 green+3레인 등재) → T11 릴리스(+롤백 리허설) → G2(원격검증) → T12 runbook 완주 → T13 사후 14일. 크리티컬 패스=G0→T1→T2→T4. 원자 묶음 분리 출하 금지: (T1 개정+완화), (T2 재합성+위생), (T7c+T10). 역할=master 위임만/worker 구현/reviewer1 적대(flip 검증)/reviewer2 감사(레인 등재·DoD·원자성).

## 4. 오너 결정 레지스터 (G0 — 구현 착수 전 확정)

| ID | 결정 | 권고 | 시급도 |
|---|---|---|---|
| D1 | P0 택(a) 개정 동봉 / 택(b) 핀 정정 | **택(a)** — §7↔REVIEWER 모순까지 해소(택(b)도 §7 정정 동봉이 필요해 비용 수렴) | **착수 차단**(T1·T2·T4 종속) |
| D2 | constitution 무수정본 자동 전진 예외 | 이번 미채택(FAIL 문안 갈음)·후속 티켓 | 중 |
| D3 | 승격 영수증 채택 | 채택(+부재 시 3핀 폴백 명세) | 중 |
| D4 | GUI 팔레트 '승격 재실행' 확장 | 채택(미채택 시 CLI 유일 경로 명기) | 중 |
| D5 | settings 정규화 실행 승인(전 프로필 tui 제거·.bak) | 승인 | 마이그레이션 시 |
| D6 | dept-1 acl.json.new 처분 | diff 검토 후 결정 | 마이그레이션 시 |
| D7 | E2E 4신호 자동화 배치처 | 런북 수동(이번)+actprobe(후속) | 릴리스 전 |
| D8 | pack-merge/rollback 가드 override 플래그 UX | 구현 재량 | minor |
| D9 | A16 후속 4건+재attach 스냅샷 백로그 승인 | 승인 | 백로그 |
| D10 | passthrough 비실존·정형 이름에 CYS_NO_AUTOSTART=1 동봉 | 동봉 | 소 |

## 5. 시뮬레이션의 정직한 한계 (실기 이관)

릴리스 전 실기 게이트: R-1 claude env×settings 4조합(스크롤백 오염 관찰 포함) · R-2 휠 체감 QA(트랙패드+마우스 × fullscreen claude·vim·less·codex) · R-3 win T3 실기 · R-4 다운그레이드 설치 리허설 · R-5 팔레트 클릭 경로. 사후: S-1 승격 CEO 1일 소크+bench 재채점 · S-2 재attach·ssh 절단 재현 · S-3 updater 경유 설치 관찰 · S-4 차기 릴리스 '구본화' 발화 확인. 3라운드는 동일 정보원(repo+라이브)을 공유하며 LLM 행동·실기 GUI·장기 소크는 미커버 — 위 목록이 그 보상이다.

## 6. 보호 기능 3종 무훼손 재확인 (R2 실코드 대조)

- CLI 설치기(번들 runtime·install_cli_to_path·install_hint)·claude PATH 자동수정(레지스트리 재합성+user_bin_dirs+B8 3중 방어): v4 전 변경과 **무접점**. 부수 이득 — P0가 신선 설치 C03 FAIL(온보딩 첫 인상 오염)을 제거.
- 부트 앵커: ①폭주=⑦ dedupe·지문 축약으로 봉합 ②무clear=델타 +1.5~2%p(60% 경로 무접촉) ③자가치유=파괴 신규 경로 0(user-owned 보존 확증) ④백지=주입 어휘 한정+ESC 문자열 종단 성질+fail-open으로 구조 봉인, 정합기 비활성 플래그가 최종 탈출구.

## 7. 최종 판정

**설계 v4 — 조건부 구현 착수 가능.** 조건: ①G0 결정 레지스터 확정(최소 D1=P0 택1), ②본 문서의 v4 문안을 구현 티켓의 정본 스펙으로 채택. 3라운드에 걸쳐 발견 106건(초기 96+R3 신규 10)이 설계에 반영됐고 추적성 UNADDRESSED 0을 확인했다.

---

## 부록 G0 — 결정 레지스터 확정 (2026-08-16, 오너 위임 하 마스터 대리 판단 · 품질 최우선)

| ID | 확정 | 근거 |
|---|---|---|
| D1 | **P0 택(a)** — 재산정 핀 42건을 담은 디렉티브 개정 4종을 같은 릴리스에 동봉 + 이행 완화 문안(C03 FAIL detail의 pack-merge 안내 분기) | §7 점수 루프↔REVIEWER verdict 계약 모순까지 해소하는 유일안(오케스트레이션 절대규칙 정합). 택(b)도 §7 정정 동봉이 필요해 편집 비용 수렴 |
| D2 | constitution 무수정본 자동 전진 예외 — **이번 미채택**(FAIL 문안 갈음), 후속 티켓 | 헌법 자동 변경 원칙 보존 |
| D3 | 승격 영수증 **채택** — directives/.ceo-template-applied, _swap 기록·ceo_demote 삭제, 영수증>표지 3핀 폴백 | 판정 결정론화 |
| D4 | GUI 팔레트 '승격 재실행' 확장 **채택** — [.pre-ceo 존재∧md≠라이브 템플릿] 게이트, approve_ceo_promotion 재사용 | 오너 막다른 흐름 제거 |
| D5 | settings 정규화(전 프로필 tui 키 제거·.bak) **승인** — 런북 단계에서 실행 | fullscreen SOT 단일화 |
| D6 | dept-1 acl.json.new — 런북 단계에서 diff 검토 후 처분 | 실측 후 결정 |
| D7 | E2E 4신호 — **런북 수동(이번)**, actprobe 프로브는 후속 | 반경 통제 |
| D8 | pack-merge/rollback 승격 가드 override 플래그명 `--force-vendor` | 구현 재량 확정 |
| D9 | A16 후속 4건+재attach 스냅샷 — **백로그 승인** | — |
| D10 | passthrough 비실존·정형 이름에 CYS_NO_AUTOSTART=1 **동봉** | 유령 생성 봉합 완결 |

추가 범위(오너 지시 2026-08-16): ①번들 Python 캐시의 번들 내 기록 원천 차단 감사·보완(PYTHONDONTWRITEBYTECODE — 기존 SEAL 계층·precompile-bundled-python.sh 실측 후 공백만 봉합) ②릴리스 게이트(격리 속성+Gatekeeper 실평가 — 기존 release-gate-gatekeeper.sh 실측 후 공백만 봉합) ③설치 후 자가진단(codesign --verify 자가 실행·정직 안내·재설치 유도).
