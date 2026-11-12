# ROUTE_CONTRACT — Level 판정의 결정론 계약 (§C4)

> SOT: `_research/vibecoding-mastery/PROPOSAL-jarvis-vibecoding-system-v3.md` §C4 (L164-225).
> 집행: `bin/javis_viberoute.py` (순수 함수 + append-only ledger). 이 문서는 신호당 true/false
> 판정 기준·판정표·재분류 경로·`_route.py`와의 합성 규칙을 고정한다. 계약 개정은 정지 경계
> (soul/CLAUDE 변경 층위) — 신호 정의·enum 신설은 문서 개정 + 토큰 게이트를 거친다.

Level 판정은 **오직 이 계약만이** 산출·기록한다(단일 SOT). 판정은 순수 함수다 — 같은 입력은
항상 같은 Level을 낸다. 판정 근거는 append-only ledger(`_round/vibecoding-ledger/route-log.jsonl`)에
남고, 기록 없는 Level 변동은 그 자체로 게이트 위반이다.

---

## 1. 신호 판정 기준 (§C4.1 — 신호당 1절)

각 신호는 `{"value": "true|false|unknown", "evidence": "파일:줄 또는 명령 출력"}`. evidence는
판정 근거(diff 경로·명령 출력)를 담는다. 판정 불가·근거 없음은 `false`로 낮추지 말고 `unknown`
으로 표기한다(§4의 보수적 격상이 받는다).

### 1.1 `persistent_data` — 영속 데이터 쓰기
- **true ⇔** DB 스키마·마이그레이션·영속 스토리지(서버 DB·파일·객체스토어) **쓰기 경로가 diff에
  존재**. 거래·사용자·상태를 프로세스 수명 밖으로 저장하면 true.
- **false ⇔** 읽기 전용, 또는 프로세스/클라이언트 휘발(메모리 캐시·browser `localStorage`·
  `sessionStorage`). 예: localStorage만 쓰는 데모는 false(GT-4).
- 예시 evidence: `migrations/003_add_orders.sql`, `db.execute("INSERT ...")`.

### 1.2 `external_integration` — 외부 시스템 연동
- **true ⇔** 외부 서비스·서드파티 API·결제/인증 provider·외부 큐/웹훅과의 **네트워크 연동이
  과제에 포함**. analytics 태그·결제 SDK·OAuth provider 모두 true.
- **false ⇔** 자기 코드베이스 내부 호출만.
- 예시 evidence: `stripe.charges.create(...)`, `<script src="analytics">`.

### 1.3 `deploy_exposure` — 배포·외부 노출
- **true ⇔** 산출물이 **배포되어 외부(사용자·인터넷)에 노출**되거나 프로덕션 실행 경로에 오른다.
  feature flag 뒤라도 배포 대상이면 true.
- **false ⇔** 로컬 전용·미배포 데모·내부 스크립트.
- 예시 evidence: `release.yml`, `vercel deploy`, 프로덕션 라우트 추가.

### 1.4 `scale_modules` — 규모·구조 작업
- **true ⇔** 다수 모듈·대규모 리팩터·아키텍처 재편 등 **규모·구조가 큰 작업**(spec이 필요하되
  영속데이터·외부연동·배포 근거는 별도).
- **false ⇔** 국소·단일 파일 수준.
- 사용 규칙: `scale_modules=true` 단독(타 신호 false)은 **최소 L3**(행3). 규모 작업이라 spec은
  필요하되 L4 근거(데이터·연동·배포)가 없으면 L3이 상한이다.

### 1.5 `brownfield` — 기존 코드베이스 개입
- **true ⇔** **기존 서비스·레거시 코드베이스**를 수정(회귀 위험·기존 계약 준수 부담).
- **false ⇔** 완전 신규(greenfield).
- `brownfield=true` 단독은 행3 → **최소 L3**(GT-15).

### 1.6 `new_service` — 신규 서비스·앱 신설 (v3.1 신설)
- **true ⇔** 기존 코드베이스에 속하지 않는 **신규 서비스·앱의 신설이 과제 정의에 존재**.
- **false ⇔** 기존 서비스 내 변경(정적 페이지 추가 등은 서비스 신설이 아니다 — GT-1).
- 신설 배경(v3.1): 구 L5 조건의 "신규 서비스"가 입력 스키마 밖 술어였던 결함(codex R3 issue 1)의
  폐쇄. 이제 판정표 안의 1급 신호다.

---

## 2. 판정표 (§C4.2 — total decision table)

### 2.0 정규화 (0단계)
각 신호의 `unknown`은 **true로 간주**(보수적 격상 — §4). 정규화 후 6신호는 true/false 2치이고
2^6 = **64조합 전체**가 아래 표의 판정 대상이다.

### 2.1 판정 규칙 — 위에서 첫 일치 행 적용 (first match wins)

| 순위 | 조건(정규화 후) | Level |
|---|---|---|
| 1 | `deploy_exposure` ∧ (`new_service` ∨ (`persistent_data` ∧ `external_integration`)) | **L5** |
| 2 | `persistent_data` ∨ `external_integration` ∨ `new_service` ∨ `deploy_exposure` | **L4** |
| 3 | `scale_modules` ∨ `brownfield` | **L3** |
| 4 | 전 신호 false | **L1-2** |

### 2.2 전칭성 (coverage)
true 신호가 하나라도 있으면 행1~3 중 하나에 반드시 걸리고(행2가 pd/ei/ns/de를, 행3이 sm/bf를 전부
수용), 전부 false면 행4 — 64조합 전체가 정확히 한 행에 떨어진다(미산출 0). 이 전칭성은
`tests/test_viberoute.py`가 64조합 전수 기계 검증으로 고정한다.

### 2.3 fail-closed (미분류 입력)
스키마 위반 입력(신호 누락·enum 밖 값·미지 신호·input_hash 직렬화 불일치)은 Level을 "낮게 추정"
하지 않는다 — 판정 자체를 **차단**한다(exit 4·Level 미산출·보고). 통과가 아니라 차단이 기본값이다.
`input_hash`가 제공되면 `sha256(signals 정규직렬화)`와 대조하고 불일치 시 차단한다.

### 2.4 needs-grill (§C4.3)
unknown이 **2개 이상**이면 판정 결과에 `needs_grill=true`를 달아 의도 합의를 요구한다. 단 합의
불가·응답 지연 시 폴백은 **"격상된 Level로 진행"**이다 — grill-me가 결정론 폴백을 대체하지 않는다.

---

## 3. critic·재분류 경로 (§C4.4 · §C4.5)

### 3.1 critic은 advisory 한정 (§C4.4)
순수 함수 판정 **후** critic(이종모델 1턴)이 의미론 검산(주석뿐인 델타의 과잉격상 의심, 문자열
래핑으로 은폐된 DB/API 연동의 격하 의심 등)을 한다. critic 출력은 advisory finding만이다:
`{suspected_direction: up|down, evidence, confidence}`. **critic은 어떤 경우에도 Level을 변경할
수 없다.** finding은 별도 레코드로 기록되고 Level은 불변이다(GT-6).

### 3.2 재분류 = 유일 변경 경로 (§C4.5)
Level 변경은 오직 이 경로만:

> critic finding 또는 인간 이의 → **master 또는 owner(사용자)의 명시 승인(`approval_id` = `APR-YYYYMMDD-NNN`)
> + `reason_code`** 기록 → 새 Level로 재판정 기록.

reason code enum:

| code | 의미 | 방향 |
|---|---|---|
| `RC-01` | 은폐된 연동 발견 (격상) | up 전용 |
| `RC-02` | 주석·포맷뿐인 델타 (격하) | down 전용 · **기계증거 필수** |
| `RC-03` | 승인된 scope 변경 (§C2.2 연동) | up/down |
| `RC-04` | pilot 롤백 조치 (§C6.4 연동) | up/down |

enum 밖 사유는 재분류 불가(사유 신설 = 계약 개정). **격하(RC-02)는 격상보다 엄격** — diff가 실행
경로 무변경임을 기계 증거(AST/diff 분석 출력, `--machine-evidence`)로 첨부해야 승인 가능하다.

### 3.3 silent 변경 차단
순수 함수 결과·`input_hash`·critic finding·재분류 승인 기록은 **각각 별도 레코드**로 append-only
기록된다(`route-log.jsonl`). `verify`는 판정 base에서 유효 재분류만 재생해 effective Level을 산출
하고, 승인 없는 Level 변동·미지 레코드·사슬 단절을 **게이트 위반(exit 6)**으로 탐지한다(GT-8).

---

## 4. `_route.py`와의 합성 (§C4.7)

- `_route.py`(= `javis_route.py`, fast/deliberate/slow)는 **응답 모드** 라우터.
- 본 계약(`javis_viberoute.py`, L1-2/L3/L4/L5)은 **구현 절차 강도** 라우터.
- 두 라우터는 **층위가 다르다(직교)**. 합성 순서:

  > `_route.py`가 **slow**로 판정한 **구현 작업에만** §C4가 발동한다.

- 단일 SOT 원칙: 응답 모드는 `javis_route.py`만이, Level(절차 강도)은 `javis_viberoute.py`만이
  산출·기록한다. 두 산출을 섞어 하나가 다른 하나를 덮어쓰지 않는다.

### 4.1 viberoute Level ↔ vibecheck `--level` 매핑 (M-2 어휘 통일)

viberoute는 하위 tier를 **`L1-2`** 한 토큰으로 산출하는데, 하류 `javis_vibecheck.py`의 `--level`
인자는 `L1|L3|L4|L5`만 받는다(`LEVEL_DOCS` 키). 합성 파이프라인 파단을 막기 위해 viberoute 출력
토큰은 `L1-2`로 유지하되(4개 Level 어휘 안정), vibecheck에 넘길 때 아래 단방향 매핑을 적용한다.
`judge` 출력 JSON은 `vibecheck_level` 필드로 이 매핑값을 함께 낸다(`javis_viberoute.to_vibecheck_level`).

| viberoute `level` | vibecheck `--level` | 근거 |
|---|---|---|
| `L1-2` | `L1` | vibecheck `LEVEL_DOCS["L1"]` 주석 = "L1~L2 스크립트·데모: 필수 문서 없음" — L1이 L1~L2를 포괄 |
| `L3` | `L3` | 항등 |
| `L4` | `L4` | 항등 |
| `L5` | `L5` | 항등 |

> vibecheck 코드(어휘·인자)는 **w-vibecheck 소유**다. 위 매핑은 viberoute 쪽 단방향 정합만 제공
> 하며, vibecheck가 향후 `L2`를 분리 신설하면 이 표와 `VIBECHECK_LEVEL_MAP`을 함께 개정한다.

---

## 5. CLI 요약 (`bin/javis_viberoute.py`)

```bash
python3 javis_viberoute.py judge --input task.json     # 6신호 JSON → Level 판정·ledger 기록
python3 javis_viberoute.py hash  --input signals.json  # input_hash(sha256) 산출
python3 javis_viberoute.py critic --task-id T1 --input-hash <h> \
    --direction down --evidence "주석뿐" --confidence low     # advisory·Level 불변
python3 javis_viberoute.py reclassify --task-id T1 \
    --from-level L4 --to-level L1-2 --approval-id APR-20260718-001 \
    --reason-code RC-02 --machine-evidence "AST diff → 실행경로 무변경"
python3 javis_viberoute.py verify --task-id T1         # effective Level + silent 변경 게이트
```

exit codes: `0` ok · `2` usage · `3` JSON 파싱 실패 · `4` fail-closed(스키마/enum/hash) ·
`5` 재분류 무효(승인/사유/증거 결여) · `6` verify 게이트 위반(silent 변경/변조).

ledger 경로는 `$JAVIS_ROOT/_round/vibecoding-ledger/route-log.jsonl`(배포 시 JAVIS_ROOT=워크스페이스
루트면 정본 경로로 해소). 테스트·격리는 `CYS_VIBEROUTE_LEDGER` 또는 `--ledger`로 override.
