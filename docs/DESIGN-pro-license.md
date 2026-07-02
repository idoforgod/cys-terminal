# DESIGN — pro 라이선스 모듈 (열쇠·만료·폐기 명단)

> 2026-07-02 박사님 확정 스펙 반영: **만료 열쇠 기본 + `never`(영구) 옵션 + 폐기 명단**.
> 배경 결정(같은 날 박사님 승인): 앱 1종(설치 파일 단일) · pro 차별화 = 라이선스 파일(앱 고급기능 게이트) + pro 팩(콘텐츠) · repo 2개(cys-terminal=free+앱 / cysjavis-pack-pro=pro overlay·발급 스크립트).
> **v2 (2026-07-02)**: R1 리뷰 라운드(agy·codex 각 BLOCK) 결착 반영 — 폐기 명단 내장 단일 SOT(§5)·status typed 진단+키 수명 경고(§7)·게이트 기계 강제(§8)·키 회전 runbook(§9)·issued_at 미래 검사(§4ⓔ).
> **v3 (2026-07-02)**: R2 결착 반영 — revocation-only 긴급 릴리스(§5)·never '보증 만료' 제안 기각 사유 명기(§6).

## 1. 목적

pro 고객에게만 앱 내 고급기능을 활성화하는 **오프라인 검증 라이선스**("열쇠"). 서버 없음 —
검증 신뢰근원은 바이너리 내장 키링(packsig와 동일 SOT) 하나다.

## 2. 비목표 (요청되지 않은 것 — 구현 금지)

- 기기 바인딩(하드웨어 지문) 없음 — 필요 시 후속 결정.
- 온라인 활성화·라이선스 서버 없음.
- **발급(서명) 기능의 바이너리 탑재 없음** — 기존 철학(minisign은 verify-only, 서명키 미포함=공격면 최소) 유지. 발급은 pro repo 스크립트(§9).
- 앱 빌드 2종화 없음(cargo feature pro 빌드 금지 — 단일 바이너리 결정).
- 시계 조작(rollback) 방어 없음 — 알려진 한계로 문서화(§10).

## 3. 라이선스 파일 포맷

`license.json` + 분리서명 `license.json.minisig` (pack-manifest와 동형 — 파일 바이트 전체가 서명 대상).

```json
{
  "license_id": "PRO-2026-001",
  "client_id": "홍길동",
  "tier": "pro",
  "key_id": "39E60A70…",
  "issued_at": 1782000000,
  "expires": "2027-06-30T23:59:59Z"
}
```

| 필드 | 형 | 규칙 (전부 필수 — `#[serde(default)]` 금지 = 부재 시 파싱 거부, packsig ⓐ와 동형) |
|---|---|---|
| `license_id` | String | 발급 일련번호. **폐기 명단의 단위** — 같은 고객의 재발급본과 구분되어 구본만 폐기 가능 |
| `client_id` | String | 고객 식별(워터마크·유출 추적) |
| `tier` | String | `"pro"`만 유효. 그 외 값 = 거부(fail-closed) |
| `key_id` | String | 서명키 ID — 내장 키링 대조 |
| `issued_at` | i64 | 발급 시각(Unix epoch 초) |
| `expires` | String | **RFC3339 만료시각(기본) 또는 리터럴 `"never"`(영구)**. 그 외 파싱불가 문자열 = 거부 |

## 4. 검증 파이프라인 — 전건 fail-CLOSED (packsig §와 동형 서술)

`license::verify(license_bytes, sig_bytes, now_unix, keyring, revoked) -> Result<LicenseFile, String>`

- ⓐ JSON 파싱: 필수 필드 부재 = 거부.
- ⓑ 키링 대조: `key_id`가 revoked/미지/만료(now ≥ not_after)/not_after 부재 = 거부.
  — `packsig::embedded_keyring()` + 동일 키 검사 로직 재사용(신뢰근원 단일화).
- ⓒ minisign 서명 검증: `packsig::verify_minisign` 재사용. 실패 = 거부.
- ⓓ tier 검사: `"pro"` 아님 = 거부.
- ⓔ 시각·만료 검사: `now_unix < issued_at` = 거부(발급시각 미래 — 위조·시계 이상 신호, R1 보강).
  `expires == "never"` → 통과 / RFC3339 파싱(packsig `parse_rfc3339` 재사용) 실패 = 거부 /
  `now_unix >= expires` = 만료 거부.
- ⓕ **폐기 명단 대조**: `license_id ∈ 내장 폐기 명단(§5)` = 거부.

replay 단조 게이트는 두지 않는다(팩과 달리 라이선스는 구본 재설치가 위협이 아니다 —
구본은 이른 만료일 또는 폐기 명단이 처리한다).

## 5. 폐기 명단 — 바이너리 내장 단일 SOT (v2: 디스크 union 폐지)

파일(소스): `cys-terminal repo의 revoked-licenses.json` (팩 트리 밖 — pro 팩에 사본을 두지 않는다)

```json
{ "revoked_license_ids": ["PRO-2026-001"] }
```

- build.rs가 keyring(trusted-keys.json)과 동일 패턴으로 **바이너리에 embed**(코드젠 상수).
  **빌드 타임 JSON 형태 검증** — 파싱 불가·형태 불일치 = 빌드 실패(손상 폐기 명단의
  출하·런타임 도달 원천 차단, R1 codex 보강).
- 전파 = 공개 채널 앱 자동 업데이트 **단일**. 근거(R1 agy 결착): 폐기가 닿아야 할 기기는
  위반자(유출 열쇠 사용 기기)인데, 그들은 pro 팩 채널(Drive 권한 회수됨)에 접근이 없고
  **앱 자동 업데이트만 받는다** — 디스크 채널은 위반자에게 실효 0이면서
  ①삭제 우회(fail-open) ②손상 시 정당 고객 무음강등(fail-closed 부작용) ③경합의
  3중 실패면만 만들었다. → 채널 제거로 3문제 동시 소멸(단순성 우선).
- **긴급 폐기(유출 확인) = revocation-only 최소 앱 릴리스** — 폐기 명단만 갱신한 릴리스를
  즉시 발행해 "다음 정기 릴리스까지"라는 지연 상한을 제거한다(v3 — R2 codex 보강.
  운영 절차는 우산 설계 §7).

## 6. 서명키 만료·회전 × 영구 라이선스 (설계 결정)

키링 철학상 서명키는 `not_after` 필수(영구 서명키 금지). 따라서 `"never"` 라이선스도
**서명키가 만료·회전되면 검증 ⓑ에서 죽는다**. 이는 버그가 아니라 의도된 안전변이다:

- ④급 사고(개인키 도난) 시 키 폐기 한 방으로 그 키가 서명한 모든 라이선스 무력화(비상 kill).
- 운영 규칙: **키 회전 시 유효 고객 라이선스 전량 재발급**(발급 스크립트 batch 모드, §9).
  고객 체감 = 파일 1개 교체. 소수 컨설팅 고객 규모에서 수용 가능한 부담.
- `never`의 의미는 "라이선스 자체 만료 없음"이지 "서명키 수명 초월"이 아님을
  발급 스크립트 출력·계약 문구에 명시한다.
- **별도 '보증 만료' 정책은 두지 않는다**(R2 agy 제안 기각·사유 명기): never의 실효 상한은
  서명키 not_after가 이미 담당한다 — 중복 만료 정책은 D5(never 옵션) 결정과 충돌하고
  복잡성만 더한다. 유출 대응은 폐기 명단(§5) + revocation-only 긴급 릴리스가 담당.

## 7. CLI (shipped — verify-only)

- `cys license install <dir|file>`: manifest·sig 로드 → §4 전건 검증 → 통과 시에만
  `~/.cys/license.json`(+`.minisig`) 원자 기록(pack::write_atomic 재사용). 실패 = 기존
  라이선스 무손상 + 사유 출력.
- `cys license status`: **typed 진단**(R1 codex 결착) — 상태를 명시 사유로 구분 출력:
  `free`(라이선스 부재·에러 아님) / `pro`(유효) / `expired`(라이선스 만료) / `revoked`(폐기) /
  `invalid`(서명·형식·시각 검사 실패) / `key-expired`(서명키 not_after 경과).
  추가로 **서명키 잔여 수명을 항상 병기** — `never` 라이선스 포함 "서명키 만료까지 N일
  (도래 전 재발급 필요)" 경고(R1 agy 결착: 경고 없는 급정지 = 기만적 UX 차단).
- 층 분리: `is_pro()`는 제품 동작상 조용히 false(우아한 강등) 유지 — 사유 진단은 status 전담.

## 8. 기능 게이트 규약 — 기계 강제 (v2: 컨벤션 단독 불수용, R1 결착)

- 단일 진입점: `license::is_pro(now_unix) -> bool`. §4 전건 통과 = true, 그 외
  (파일 부재 포함) 전부 조용히 false = free 동작(우아한 강등 — 앱 파손·데이터 삭제 없음).
- 프로세스 내 1회 검증 후 캐시 가능(파일 ~1KB + ed25519 1회라 비캐시도 무방 — 구현 단순 우선).
- **기계 강제 4겹** (컨벤션은 유지하되 위반이 기계적으로 빨개지게):
  1. **은닉**: `LicenseFile` 등 세부 구조체·파싱·검증 함수는 `license.rs` 밖 비공개(pub 금지) —
     외부에는 `is_pro()`(+status용 진단 enum)만 노출.
  2. **pro 기능 레지스트리**: pro 기능은 중앙 레지스트리(상수 목록)에 등록해야만 존재 —
     게이트 분기·노출 판정이 레지스트리 경유로만 배선된다.
  3. **정적 핀 테스트**: `license.json` 문자열 리터럴·minisign 직접 사용이 `license.rs` 밖
     소스에 등장하면 fail하는 소스 스캔 테스트(기존 회귀 핀 스타일 — 독자 파싱 산재 차단).
  4. **인벤토리 테스트**: is_pro()=false 상태에서 레지스트리의 전 pro 기능이 CLI·메뉴·도움말에
     미노출임을 전수 확인.
- 미노출 원칙: is_pro()=false면 pro 기능은 메뉴·도움말에서 보이지 않는다(잠김 표시가 아니라 부재).

## 9. 발급 (pro repo — 바이너리 외부)

`cysjavis-pack-pro/bin/license-issue.sh` (박사님 전용·개인키 필요):

```
license-issue.sh --client 홍길동 --expires 2027-06-30    # 만료 열쇠(기본)
license-issue.sh --client 홍길동 --expires never          # 영구 열쇠(명시 옵션)
license-issue.sh --reissue-all --expires-shift …          # 키 회전 시 전량 재발급(§6)
```

- license_id 자동 채번(발급 대장 `ledger.json`에 append — 폐기·재발급 커버리지의 SOT).
- 서명 = minisign CLI(팩 서명과 동일 키·동일 절차).
- `--expires` 생략 = 에러(만료가 기본이되 **날짜는 박사님 명시 입력** — 무음 기본값 금지).
- 폐기 = 대장에서 license_id 찾아 `revoked-licenses.json`(cys-terminal repo)에 추가 →
  다음 앱 릴리스에 내장 전파(§5 단일 SOT).

### 키 회전 runbook (v2 — R1 결착: "전량 재발급"의 커버리지 증명 의무)

1. 발급 대장에서 **유효 고객 인벤토리** 도출(미만료·미폐기 전건 — 기계 산출).
2. `--reissue-all --dry-run` → 커버리지 리포트(대상 N건·누락 0 검증) → 박사님 확인 후 집행.
3. 일괄 재발급 → 고객별 전달 → **전달 확인(ack)을 대장에 기록** — 미확인 고객 잔존 시 경고.
4. 평시(비상 아닌) 회전: 구키 `not_after`를 즉시 만료가 아닌 **중첩 창**(예: +90일)으로 운영 —
   재발급 전달이 늦은 정당 고객의 서비스 단절 방지.
5. **즉시 폐기는 키 유출(비상) 전용** — 이때 미전달 고객의 일시 단절은 사고 대응 비용으로
   수용함을 명시(무음이 아니라 문서화된 결정).

## 10. 알려진 한계 (박사님 고지 완료 사항)

1. 중도 해지 즉시 회수 불가(오프라인) — 완화: 짧은 주기 발급 + 내장 폐기 명단(앱 업데이트 전파).
2. 시계 rollback으로 만료 연명 가능 — 신뢰 고객 위협모델상 수용.
3. 역공학으로 게이트 우회 이론상 가능 — 완화: 계약·워터마크·심층 IP는 pro 팩에 소재.
4. 폐기는 업데이트 미수신 기기에 미전파 — "시간이 지나면 대부분 죽는" 수준임을 인지.

## 11. 테스트 계획

- 단위(license 모듈): 정상 만료열쇠 통과 / `never` 통과 / 만료 경과 거부 / 필드 결손 각각 거부 /
  tier≠pro 거부 / 서명 불일치 거부 / 폐기 license_id 거부(내장 명단) /
  **발급시각 미래(now < issued_at) 거부** / 키 not_after 경과 시 `never` 라이선스 거부(§6 핀) /
  expires 쓰레기 문자열 거부.
- 빌드타임: **손상 revoked-licenses.json = 빌드 실패** 핀(build.rs 검증).
- roundtrip(dev-dep minisign): 발급 포맷 → 검증 통과 (packsig 기존 fixture 패턴 재사용).
- CLI e2e: install 실패 시 기존 라이선스 무손상 / status **typed 6상태**(free·pro·expired·
  revoked·invalid·key-expired) 각각 출력 + 서명키 잔여 수명 병기.
- 게이트 기계 강제: 정적 핀(라이선스 직접 파싱·minisign 사용이 license.rs 밖 등장 시 fail) /
  is_pro()=false에서 레지스트리 전 pro 기능 미노출 **인벤토리 전수** 테스트.

## 12. 구현 범위 (외과적 — 이 설계로 추적되는 변경만)

| 대상 | 변경 |
|---|---|
| `src/license.rs` (신규) | 포맷·검증 ⓐ~ⓕ·is_pro·진단 enum·pro 기능 레지스트리 — packsig 재사용 위주. 세부 구조체 비공개(§8-1) |
| `src/packsig.rs` | `verify_minisign`·`parse_rfc3339` 가시성 조정(pub(crate)) 필요 시에만 |
| `src/bin/cys.rs` | `license install/status` 서브커맨드 2개(status = typed 6상태+키 수명) |
| `build.rs` | revoked-licenses.json embed + **빌드타임 JSON 형태 검증**(키링 embed와 동형 코드젠) |
| `revoked-licenses.json` (신규·repo 루트) | 빈 폐기 명단 초기 파일 (팩 트리 밖 — pro 팩 사본 금지) |
| `tests/` | 정적 핀(게이트 산재)·인벤토리 테스트(§8-3·4) |
| `cysjavis-pack-pro/bin/license-issue.sh` | pro repo 생성 시(별도 티켓) — dry-run·커버리지·ack 추적 포함(§9 runbook) |

pro 기능 자체(게이트 뒤에 들어갈 고급기능)는 이 설계의 범위가 아니다 — 기능별 후속 티켓.
