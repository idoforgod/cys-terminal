# 부서 한정 키 모델 — 월드 계약 v2 설계 (v2.1 · 2026-07-12 적대검증 반영)

> 원천: v1 잠복 결함(surface_ref 전역 비유일) 근본 수리. 오너 승인: "근본 수리 승인·설계안부터".
> v2.1: 적대검증 2렌즈(correctness·ops) 각 REVISE 판정의 blocking 1·major 7·minor 4 전건 반영.
> 리뷰 요지 — 핵심 전제(--socket 실재·/command 오배달 실재·프론트 키 불투명성 골격·Phase 분리)는
> 코드 실선 검증 통과. 결함은 전부 명세 공백이었고 아래에 닫는다.

## 1. 문제 (전부 실측)

1. surface 번호는 부서 데몬별 발급 — 전역 비유일(실측 9개 ref 충돌·surface:7 ×5).
2. **diff 추적 오염**: prev_nodes/seen이 ref 단독 키 — 동번호 부서끼리 매 틱 상호 덮어씀.
3. **프론트 엔티티 충돌**: office3d nodes Map 키 충돌 — 뒤 부서가 앞 부서 아바타를 덮음.
4. **★/command 오배달(심각·실선 확인)**: 브리지가 소켓 미지정 `cys send --surface key` —
   부서 아바타 지시가 본부 데몬 동번호 surface로 배달(bridge:1260, 방어 없음 확인).
5. 귀속 커버리지 0: f62aea1 유일 게이트가 오귀속은 봉쇄했으나 6부서 전중복 체제에서 사실상 전부 미귀속.
6. (관련) `cys events`는 본부 단일 구독 — 부서 노드 훅 연출 원래 부재(→Phase 2).

## 2. 목표 / 비목표 / 이득의 정직한 범위

- 목표: ①전역 유일 노드 키 ②/command·/peek 정확 라우팅 ③spool 정밀 귀속 **토대** ④기존 화면 무회귀.
- **Phase 1 확정 체감 이득 = ②오배달 수리 + 엔티티/diff 충돌 수리** (ops-m7 반영).
  ③의 실이득은 방출자가 정식 키를 emit해야 발생 — §4d emitter 이관을 **완료 게이트**로 두고
  그 전까지 귀속은 점진(기존 유일 게이트 하위호환 경로 유지).
- 비목표: cysd 서버 프로토콜 변경 0(fleet 집계는 cys CLI 클라이언트 구조 — 실선 확인) ·
  CC(ui/src) 변경 0 · 음성/상위 소비자 변경 0.

## 3. 키 스킴 (v2.1 개정)

- **정식 키**: `<dept_slug>@surface:<N>` — 예: `dept-1@surface:5`, 본부 `main@surface:11`.
- **구분자 `@` (v2.0의 `#`에서 교체)**: correctness-blocking — `#`는 URL fragment 구분자라
  `/peek?key=<키>` GET에서 서버에 키가 잘려 도달(office3d:2177 raw 연결 실측). `@`는 RFC 3986
  query 허용 문자(pchar). **방어 이중화**: 프론트는 그래도 모든 URL 삽입점에 encodeURIComponent
  적용(키는 불투명 토큰 — 포맷 재변경 대비).
- slug = depts.json 레지스트리 키(리네임 불변), **본부 = 고정 slug `"main"`** (ops-M4: run_fleet의
  하드코딩 본부 타깃에도 명시 부여 — fleet 키와 이벤트 키가 반드시 일치해야 함, e2e 단정 대상).
- slug 문자셋 계약: `^[a-z0-9_-]{1,32}$`. 비정합 레지스트리 키는 브리지가 정규화
  (소문자화·허용 외 문자 `-` 치환·32자 절단, 충돌 시 `-2` 서픽스) 후 사용 — fail-open 금지.
- 노드 뷰에 표시 전용 `dept_label`(display_name) 별도 필드. 패널 타이틀은 dept_label 사용
  (correctness-minor: raw 키 노출 제거).

## 4. 변경 목록

### 4a. cys CLI (`run_fleet`) — additive (~25줄)
- depts 루프: 각 항목에 `"dept"`(slug=레지스트리 키)·`"socket"`(경로 문자열) 추가.
- **하드코딩 본부 타깃에도 `"dept":"main"`,`"socket":null` 명시**(M4). surfaces 불변.

### 4b. 브리지 (javis_hud_bridge.py)
- merge_fleet: dept마다 slug(+정규화)·socket 채택. 노드 정식 키 생성 + `dept_label`.
  `dept` 필드 부재(구 cys 조합) 시 display_name 정규화 폴백 + 1회 경고 로그.
- **소켓 캐리(M1 — depts.json 재독 기각)**: 노드별 socket을 World에 스냅샷 캐리
  (`self.sockets[full_key] = socket|None`). /command·/peek는 **스냅샷의 socket**으로
  `cys [--socket <path>] send --surface surface:N` 실행(--socket은 clap 전역 플래그 — 서브커맨드 앞 배치, 구현 정합화 v2.1.1) — 재독 레이스·3중 파서·부재 fail모드
  3건 동시 소거. 키에 대응 socket 미존재 = unknown_target 거부(fail-closed).
- 키 전면 전환: prev_nodes·known_keys·heat_acc·progress/run·blocked[].key.
- **CMD_KEY_RE 교체(C2/M5)**: `^[a-z0-9_-]{1,32}@surface:\d{1,8}$` — gate_command(199)·/peek(1218)
  **두 사용처 동시** 갱신 + 미등록 키 deny 유지 + 음성 테스트(구 bare 키 거부·비정합 slug 거부).
- route_event: 본부 단일 구독이므로 key = `main@surface:<sid>`.
  **apply_usage 스코프 수정(C3)**: 전 부서 첫-매칭 순회(현행 260-268, departments[0]=본부 순서
  의존·무문서)를 본부 한정 조회로 교체 — 순서 의존 소거.
  **despawn/spawn fallback(C4)**: `surface_ref` 재조립 분기(588)도 `main@` 정식 키로 생성.
- route_spool 귀속 사다리: ①정식 키 명시=즉시(스냅샷 존재 검증) ②bare `surface:N`=기존 전역
  유일 게이트(하위호환) ③role 유일 ④미귀속.
  ★(0.14.56 X-8 CARRIER) spool 은 레인별 파일이고 레코드의 레인 = 그 파일의 레인이다: ②③은 그 레인의 노드로 한정되고,
  정식 키의 slug 가 그 레인 키와 다르면(다른 레인의 키) 사다리로 강등하지 않고 **버린다**(상태 변화·fx 없음 ·
  `/health` 의 레인별 `foreign_key_dropped` 로 센다). `blocked[]` 항목은 `lane` 을 갖고 (lane, task) 단위로 바뀐다.
- **presence_heat 지연 마이그레이션(M2)**: load_heat(부팅 직후, fleet 미수집)는 bare 키를
  `pending_heat`로 **보존만** 하고, 첫 merge_fleet 완료 시점에 1회 기회적 승격(유일 매칭 시
  정식 키로, 실패분만 폐기). 부팅 순서상 즉시 승격이 불가함(1333<1336)을 계약에 명시.
- fx_archive 리플레이 관용: bare 키 항목은 suffix 유일 매칭, 실패 시 티커만.

### 4c. 프론트 (office3d.html)
- 키 불투명성 유지 + **URL 삽입점 전수 인코딩**: `/peek?key=` (2177) 등 GET 경로
  encodeURIComponent — POST(/command body JSON)와 GET(URL)의 인코딩 요구 차이를 주석으로 계약화.
- 월드 `"v":2` 승격 + `w.v > 2 → location.reload()` self-heal 방어.
- 패널·근접 표기 dept_label. fx despawn/spawn 등 신규 키 그대로 Map 매칭(불투명).

### 4d. 방출자·문서 — **완료 게이트**
- javis_event.py `--surface` 정식 키 허용(검증 확장). EVENT_CONTRACT·office-detail-v11 v2 절 갱신.
- **emitter 이관 추적**: `--surface` 사용처 grep 목록화 → 정식 키 상향 완료를 goal③의
  완료 게이트로(그 전까지 귀속 점진임을 §2에 명시).

### Phase 2 — 부서 이벤트 멀티 구독 + emitter 이관 (2026-07-12 오너 승인 · 상세 스펙)

**P2-1 구독 슈퍼바이저**: events 구독을 (slug,socket)별 fan-out. fleet 폴 주기마다 타깃
집합({main:None} ∪ fleet의 dept/socket)과 실구독을 조정(reconcile) — 신규 부서 spawn·소멸
부서 reap(terminate). 타깃 상한 12(런어웨이 방지)·재수립 백오프 2s 유지·cursor는
`cursor-<slug>.seq` per-slug. 전 구독은 공유 Hub·공유 Coalescer(락 보유) 경유.
`cys [--socket S] events --reconnect --cursor-file …` (--socket 전역 플래그).

**P2-2 route_event slug 문맥**: `route_event(ev, world, coal, slug)` — 키 `f"{slug}@surface:{sid}"`.
main@ 고정을 일반화. apply_usage도 해당 slug 스코프(C3의 본부 한정을 일반화).

**P2-3 sid 단독 키 맵 정식화(★v1 잠복 결함 2호 수리)**: hooks·line_hist·line_rate·flags가
surface_id 단독 키 — 멀티 구독 전에도 line_hist/line_rate는 merge_fleet가 전 부서를 sid로
순회해 **부서 간 activity 오염이 현재도 실재**. 4맵 전부 정식 키로 전환(_node_view·
accumulate_heat·set_flag/live_flags 등 소비처 동기).

**P2-4 emitter 이관**: 생산자 호출부 grep 실측 0곳 — 이관 = ①`javis_event.py emit --surface auto`
신설: CYS_SOCKET env(부재=main) → depts.json socket 매칭으로 slug 해석 + `cys identify`로
surface_ref → 정식 키 자동 조립(해석 실패 시 미귀속 폴백·fail-open 금지) ②EVENT_CONTRACT
가이드에 `--spool --surface auto` 표준 방출 규약 추가(CYSjavis측 — master 직접).

**P2 검증**: 슈퍼바이저 reconcile(부서 추가/소멸 fixture)·동번호 hook 무충돌(두 데몬 sid 5 훅
→ 해당 노드만 active)·line_rate 격리·apply_usage slug 스코프·--surface auto 해석 3분기 —
음성검증: P1 코드에 신규 테스트 FAIL 실측. 실기: 부서 pane 도구 훅 → 해당 부서 아바타만
활동 연출(오피스 육안).

## 5. 호환성 매트릭스 (v2.1 — 전이 상태 2행 추가)

| 조합 | 거동 |
|---|---|
| 신 브리지 + 구 프론트(열린 iframe) | 키 불투명 → 렌더 지속·재오픈 시 정합 |
| 구 브리지 + 신 프론트 | v1 bare 키 무가정 — 불변 |
| 신 브리지 + 구 cys(dept 필드 없음) | display_name 정규화 폴백·경고 — 기능 유지 |
| **배포 스왑 전이(동일 브리지가 구→신 cys 연달아 호출)** | 키 포맷 불연속 → **1회 전량 churn(스폰/디스폰 플리커) 수용** — 직후 안정(M3) |
| **롤백(구 브리지 + v2 STATE_DIR)** | presence_heat의 정식 키가 팬텀 행 잔존 위험 → **롤백 절차에 presence_heat.json 삭제 1줄**(M6) |
| 구 spool 항목(bare key) | 유일 게이트 경유 — 오귀속 0 유지 |

## 6. 배포 런북 (M3 — 브리지 활성화 명시)

1. pack sync/설치(코드 교체) — 러닝 브리지는 구코드 유지(프로세스는 파일 재로드 안 함).
2. cys/cysd 교체(deploy_gate) → 데몬 재시작 → **cysd가 브리지 자식 재스폰 = 신코드 활성**.
   (브리지가 cysd 자식이 아닌 비상 수동 기동 상태면 명시 kill 후 재기동.)
3. **활성 검증 게이트**: `curl /world`에서 정식 키(`@surface:`) 등장 + `main@` 본부 키 확인 후 종료.
4. 롤백 시: pack install(force) + `STATE_DIR/presence_heat.json` 삭제(+선택 fx_archive) + 브리지 재기동.

## 7. 검증 계획 (v2.1 추가분 굵게)

- unit(~20): 동번호 2부서 fixture — diff 무간섭·patch 정확 타깃·귀속 사다리 4분기·
  **소켓 캐리 라우팅(부서→--socket·본부→생략·미지 키 deny)**·**CMD_KEY_RE 음성(구 bare 거부·
  비정합 slug 거부)**·**heat 지연 승격(부팅 순서 재현: 빈 fleet→보존, 첫 merge 후 승격)**·
  **apply_usage 본부 스코프**·**despawn fallback 정식 키**·리플레이 관용.
  음성검증: v1 코드에 신규 테스트 → FAIL 실측.
- e2e(office_detail_gate 확장): 합성 월드 v2 키 + 동번호 2부서에서 fx 정확 타깃 1개 +
  **mock /peek가 수신 키==정식 키 전체임을 단언 + 적대 키(`…#raw` fragment 문자 포함
  불투명 토큰) 왕복 단언(v2.1.1 — 정식 키의 @·:는 URL-safe라 정식 키만으론 인코딩 음성검증이
  공허함을 구현 중 실측, encodeURIComponent를 load-bearing으로 만드는 강화)** +
  **본부 fleet 키 == 이벤트 키(main@surface:N) 정합 단정**.
- 실기 결정타 2건: ①중복 번호 부서에 진행률 주입 → 그 부서 아바타에만 링
  ②부서 pane에 오피스 /command → **그 부서** 도달(input_injected 왕복) = 오배달 수리 입증.

## 8. 리스크·규모

- 리스크: heat 이력 부분 소실(승격 실패분·수용) · 스왑 전이 1회 churn(수용·§5) ·
  /command 경로 변경(실기 게이트).
- 규모: cys.rs ~25줄 · 브리지 ~150줄 · office3d ~20줄 · 테스트 ~250줄.
- 실행: **격리 워크트리 + `feat/dept-qualified-keys-v2` 브랜치**(타 터미널이 main에서 0.12.48
  릴리스 작업 중 — 파일 겹침 0 실측·버전 0.12.48 선점 확인). base=origin/main(push된 안정점).
  main 머지·릴리스는 타 터미널 0.12.48 발행 완료 재확인 후 0.12.49로.
