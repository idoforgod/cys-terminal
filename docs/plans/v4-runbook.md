# v4 수리 릴리스 런북 — v0.14.16 (스펙 §A12 순서 A)

> 정본 스펙: `docs/plans/v4-repair-spec.md` (§A12 · 신설 절 · 부록 G0 결정 레지스터).
> 판번: **단일 v4 릴리스 = v0.14.16 으로 통일**(분할 출하 금지 — 원자 묶음 (T1 개정+완화)·
> (T2 재합성+위생)·(T7c+T10)은 같은 릴리스에 실린다).
> 실행 주체: 오너(부재 시 마스터 대리 판단 — 품질 최우선). 각 단계는 체크 후 다음으로.

---

## 0. 사전 조건 (릴리스 시퀀싱 — 스펙 신설 절)

태그 전·업로드 전 순서(6항 원격검증까지가 릴리스다 — 코드 완료≠릴리스):

1. **win T3 green 증적** — D3 거부 4종(launch `--help` 기대값 = *name 위치=exit 2·부작용 0*) ·
   P0 전 PASS · CEO_TEMPLATE 바이트 대조 · 승격 스모크(마커 시드→promote→cmp→down 원복) ·
   alt_screen 관측 · win-parity 바이트 핀 · agents.json `"0"` override. 이 증적이 릴리스
   차단 게이트다(태그 파이프라인과의 결합은 본 런북의 수동 순서가 정본).
2. 태그(mac) → 3. win 빌드 → 4. **WDSI 신고**(Defender 오탐 최소화) → 5. **애플 공증**

> ★2026-08-20 실측(v0.14.20): WDSI 익명 제출은 Microsoft가 전면 차단(전 트랙 OAuth 로그인 필수 — 페이지 소스에 'TODO: uncomment when we re-enable anonymous submissions' 주석 실재). 제출에는 Microsoft 계정 로그인 1회 필요. 폼 셀렉터 지도·기입값·자동 제출 스크립트(fill.js — CAPTCHA 감지 시 exit 42 중단)는 준비됨: 분류=#userOpinionClean(Incorrectly detected)·탐지명=#detectionName·업로드=#filePicker. 오너 로그인 후 약 2분 소요.
   (`~/.cys/apple-notary.env` 자격 — 무공증이면 Gatekeeper 차단) →
6. **전 자산 일괄 업로드** → `SHA256SUMS.txt` 전 자산 재생성(`scripts/release-postprocess.py` —
   구버전 줄 잔존 금지 · ★백업 DMG 2종에 Gatekeeper 게이트 자동 실평가, rc≠0 이면 `--apply`
   거부=fail-closed) → **원격검증 6항**(`scripts/verify-release-remote.py` — 다운로드 4URL
   200 · Defender/SmartScreen 안내 섹션 잔존 grep ≥1 포함).

- updater 채널 공개는 **본 머신 런북(아래 ①~⑥) 완주 후**. 앞당기려면 'WARN 창 수용'을
  결정으로 기록하고 진행한다.
- 팩은 양 OS 동일 임베드 — 머신 간 팩 스큐는 저위험(스펙 명기)이라 OS별 순차 업로드 중
  혼재 창을 별도 통제하지 않는다.
- 다운그레이드 설치 리허설 1회(§7 롤백 리허설 · R-4)가 릴리스 게이트에 포함된다.

## 1. 순서 A — 본 머신 마이그레이션 (전문)

**⓪ 채택·정리는 릴리스 설치 후에만 시작한다.** 선채택 금지 — 구 릴리스에서 `.new` 를
먼저 채택하면 이중 채택 + `.pre-ceo` 구본화가 된다. 설치 후 `.new` 가 신본으로
재기록됐는지 `cmp` 로 확인하고 나서 ③으로 간다.

**① v0.14.16 설치** (본 머신).

**② 같은 세션에서 `cys-dept promote-ceo` 재실행** — 오너 role-less 셸 또는 CSO CLI 에서.
CEO pane 자신의 실행은 exit 7 거부가 **계약**이다(단일소유 강제). GUI 팔레트의 '승격
재실행' 항목(결정 D4 채택 — `[.pre-ceo 존재 ∧ md≠라이브 템플릿]` 게이트)도 같은 경로다.

**③ D1(a) — 디렉티브 채택** (P0 택(a): 개정 4종 동봉 릴리스이므로 전 역할 신본 채택):

```sh
# base 팩 — WORKER/CSO/REVIEWER = take-new
cys pack-merge --file directives/WORKER_DIRECTIVE.md   --take-new
cys pack-merge --file directives/CSO_DIRECTIVE.md      --take-new
cys pack-merge --file directives/REVIEWER_DIRECTIVE.md --take-new
# base MASTER (CEO 승격 중) — ①② 순서 역전 금지: keep-mine 이 .new 를 삭제한다
cp  ~/.cys/pack/directives/MASTER_DIRECTIVE.md.new \
    ~/.cys/pack/directives/MASTER_DIRECTIVE.md.pre-ceo   # ① 복원 백업을 신본으로 갱신
cys pack-merge --file directives/MASTER_DIRECTIVE.md --keep-mine   # ② 승격본 유지(.new 해소)
```

코드 가드가 실수를 기계 거부한다: `MASTER_DIRECTIVE.md` 가 승격 중(`.pre-ceo` 실재)이면
`pack-merge --take-new` 와 `pack-rollback --file` 은 거부된다(승격 파괴 승인 시에만
`--force-vendor` — 결정 D8). 채택 후, 기동 중인 WORKER 노드는 재각성한다(javis_boot_node).

**④ dept-1 팩** — 승격 개념 없음: 디렉티브 전부 live `take-new`. `acl.json.new` 는 오너가
diff 검토 후 처분한다(결정 D6 — 실측 후 결정, 자동 채택 금지).

**⑤ D1(b)(c)(d)** — 아래 §2~§4 상세 절차.

**⑥ 부트 4신호(§5) + 최종 체크리스트(§6).**

## 2. D1(b) — 유령 부서(`--help`) 격리

각 단계 **실행 전** `ls -d` 로 정확 경로를 눈으로 확인한다. **glob 금지.**

1. `cys-dept down-sock <유령 sock 정확 경로>` — 데몬 소등.
2. 상태 디렉터리는 `rm` 대신 **`mv`** 로 `~/.local/state/cys-trash/--help-<ts>/` 에
   격리한다(TTL 14일 — `cys-dept-*` 글롭 무매치 경로라 복원 루프 재발견이 절단된다).
3. 사후 확인 = sock 부재 + `pgrep` 0.
4. `--help` 문자 이름은 down-by-name 불가가 정상이다(v4 name 검증이 exit 2 로 거부) —
   **수동 격리가 정본 절차**다. 묘비는 불요(복원 루프가 미등재+死 소켓을 드롭한다).

## 3. D1(c) — feed 잔건 9건 처분

- stale 7건 = **deny**.
- 승격 알림 2건 = **allow** — 반드시 **cys pane 또는 GUI 에서** 처리한다.
  sid 앨리어싱으로 거부되면 GUI 재시도.

## 4. D1(d) — fullscreen 정책 SOT 정규화 (결정 D5 승인)

- **settings 정규화(SOT)**: 전 계정 dir 의 claude settings 에서 `tui` 키 제거/inline 정규화
  + preflight ensure. 수정 전 반드시 같은 자리에 `.bak-*` 백업(관례 — 매뉴얼 §4.6b).
  오너 승인 1회 후 실행.
- **D5 env 방어층 병행**: mac·claude 기동 시 `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` 기본
  주입(**키 부재 시에만** — `src/lib.rs inject_claude_alt_screen_default`, append 후 재정렬
  금지 계약). 계정별 옵트인은 팩 `agents.json` env 에 `"0"`(사용자 값 절대 불가침).
- **관측**: launch-agent ready 직후 mac ∧ claude ∧ `alt_screen=true` → stderr **WARN**
  (`cys.rs alt_screen_notice` — env 방어층 우회 감지). win 은 힌트 1줄(경보 아님).
  `alt_screen` 필드 부재(구 데몬)=판정 불가·생략(FAIL 격상 금지). 일 집계는 fleet digest 1줄.

## 5. 부트 4신호 (⑥ 전반부 — 결정 D7: 이번엔 런북 수동, actprobe 프로브는 후속)

- ⓐ 델타 marker readiness ⓑ 8s 내 `awakened_at` ⓒ 2차 에코 ⓓ `alt_screen=false`.
- 추가 실기: '부서장 빈 셸에서 수동 `claude`' 1건 + env×settings 4조합 중 2조합(R-1 겸용).

## 6. 최종 체크리스트 (⑥ 후반부)

- [ ] 팩 등가 — 설치본 팩 == 임베드(양 OS 동일 임베드).
- [ ] **`.pre-ceo` 핀 전수 스팟체크** — master 핀 42건 재산정본이 `.pre-ceo` 에서 전수 통과.
- [ ] 원장 0 (미해소 잔건 없음).
- [ ] feed 0 (pending 없음 — §3 처분 완료 상태 유지).
- [ ] preflight 전 PASS — C03 두 축(핀 축·demote-guard) 포함, WARN 은 사유 확인 후 기록.

## 7. 롤백 절 (스펙 신설 절)

| 대상 | 절차 |
|---|---|
| **D2 (CEO 템플릿)** | 「직전 릴리스 재설치 → **같은 세션에서 promote-ceo 재실행**」. **md 파일 수동 복귀 금지**(비정형 승격 FAIL 자가 제조 경로 — pack-rollback 도 승격 중엔 기계 거부). 다운그레이드 설치 리허설 1회를 릴리스 게이트에 편입(R-4). |
| **D4 (마우스 정합기 · macOS)** | `localStorage.cysMouseReconcilerOff="1"` — 정합기 소비 전부 비활성(win 비활성 코드 경로 재사용 · 새 pane 부터). ⚠**이 스위치는 Windows 휠 가드를 끄지 못한다**(아래 행) — 종전에는 Windows 에 휠 핸들러가 없어 이 구분이 없었다. |
| **D4-W (Windows 휠 가드)** | PowerShell `New-Item -ItemType File -Force $HOME\.cys\win-wheel-guard-off` 후 **새 pane**(env `CYS_WIN_WHEEL_GUARD_OFF=1` 동등하나 GUI 가 상속한 값만 읽으므로 GUI 재시작 필요 — 정본 수단은 파일. `touch` 는 PowerShell 에 없다). ⚠**결함 복원 스위치**다 — 끄면 claude fullscreen 휠→방향키 합성이 되살아나 프롬프트 입력창이 다시 오염될 수 있다. `allow-app-mouse` 로 대신하지 마라(ConPTY 결함 1호 재발). |
| **D5 (env 게이트)** | 전 계정 팩 `agents.json` env 에 `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN: "0"` **일괄** 기입 — 주입 계약이 '키 부재 시에만'이라 `"0"` 이 항상 이긴다. ※**Windows 는 v0.14.19 부터 기본 미주입(옵트인)** 이라 되돌릴 것이 없다 — 옵트인한 사용자는 `~/.cys/win-no-alt-screen` 삭제로 즉시 복귀한다(`src/lib.rs` 의 `d5_gate_for_os` doc 이 정본). |
| **릴리스 전체** | 이전 자산 재게시 → `SHA256SUMS.txt` 재갱신 → 원격검증 재실행 → updater 채널 처분(구버전 재공개 또는 잠금). |

## 8. 사후 검증 14일

- **지표 4종**(매일): ① C03 FAIL 0 · 상태 지문 변화 0 ② 미등재 state dir 신규 0
  ③ 프롬프트 히스토리 오염 재현 0 ④ override(`--force-vendor` 등) 발동 0.
- fleet digest 에 C03 요약 + `alt_screen` WARN 건수 1줄 편입.
- **오너 확인 문답 — D+1·D+7 각 4항**: ⑴ fullscreen 휠 = 트랜스크립트 스크롤인가
  ⑵ 히스토리 오염 0인가 ⑶ Option+드래그 복사가 되는가 ⑷ vim/less 휠 무회귀인가.
- 차기 릴리스에서 C03 '구본화' 분류 발화를 E1 재발 검증으로 예약(S-4).

## 9. 실기 이관 목록 (시뮬레이션 한계 보상 — 스펙 §5)

**릴리스 전 게이트**: R-1 claude env×settings 4조합(스크롤백 오염 관찰 포함) ·
R-2 휠 체감 QA(트랙패드+마우스 × fullscreen claude·vim·less·codex) · R-3 win T3 실기 ·
R-4 다운그레이드 설치 리허설 · R-5 팔레트 클릭 경로(승격 재실행 항목).

**사후**: S-1 승격 CEO 1일 소크 + bench 재채점 · S-2 재attach·ssh 절단 재현 ·
S-3 updater 경유 설치 관찰 · S-4 차기 릴리스 '구본화' 발화 확인.

## 각주 — 부서명 slug 충돌 쌍 개명 절차 (D3(i))

기존 등재명 중 fold(소문자화+'.' 제거) 동일 쌍이 preflight WARN 으로 가시화되면(신규
생성은 이미 기계 거부 — "거부(개명 후 재시도)"), 오너 감독 하에: ① 새 **정형** 이름
(`^[A-Za-z0-9][A-Za-z0-9_-]*$`·≤40자)으로 `cys-dept launch` ② 진행 중 작업·필요 상태를
이관·확인 ③ 구 부서 `cys-dept down` ④ 잔존 상태 디렉터리(`~/.local/state/cys-dept-<구명>`)는
§2 관례대로 `mv` 격리(rm 금지). 자동 개명·자동 삭제는 없다.
