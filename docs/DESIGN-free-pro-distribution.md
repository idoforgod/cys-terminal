# DESIGN — cys-terminal free/pro 이원 배포 체계 (통합 최종 설계안)

> 2026-07-02 박사님 확정 결정 전부를 통합한 우산 설계서. 하위 상세: `DESIGN-pro-license.md`.
> **v2 (2026-07-02)**: R1 리뷰 라운드(agy·codex 각 BLOCK) 결착 반영 — 버전 계약
> channel·pro_revision 재설계(§3)·다운그레이드 가드 완전 명세(§5)·폐기 명단 내장 단일
> SOT(§7·라이선스 설계 §5)·회전 runbook 참조(§7).
> **v3 (2026-07-02)**: R2 라운드(agy REVISE·codex BLOCK) 결착 반영 — `.pack-state.json`
> 영속 상태 계약(§3)·손상 진단+복구 명령(§5)·license-aware downgrade(§5)·pro_revision
> 단조 빌드 가드(§3)·revocation-only 긴급 릴리스(§7).
> **v4 (2026-07-02)**: R3 라운드(agy REVISE·codex BLOCK) 결착 반영 — 트랜잭션 계약 완결
> (state journal 편입·정합 검사·AcceptedPack post-commit 재배치·crash 4케이스·self-heal — §3)·
> 손상 상태 × pack-update 시맨틱(§5)·repair 권위 규칙(§5).
> **v5 (2026-07-02)**: R4 라운드(agy REVISE·codex REVISE) 결착 반영 — self-heal 4조건 강화
> (디스크 트리 해시 대조 — §3)·내장 free 경로 checked 쓰기 순서·fault-injection(§3)·
> channel=free 한정 자가치유(§5)·post-commit accepted 실패 시맨틱(§3)·낡은 accepted 창
> 무해 증명 명기(§3).
> **v6 (2026-07-02)**: R5 라운드(agy **ACCEPT**·codex REVISE) 결착 반영 — 자가치유에
> **음성 pro 증거 검사**(accepted=pro·pro 파일 실재 시 금지 — §5)·self-heal 락 규칙(§3).
> 상태: ★**R6 수렴 종결(agy·codex 양측 ACCEPT — 2026-07-02)** — 구현 착수 가능(박사님 승인 대기).

## 0. 결정 대장 (박사님 확정 — 전 항목 이 설계의 불변 전제)

| # | 결정 | 일자 |
|---|---|---|
| D1 | free = 지금까지 만든 모든 것(현행 전체 팩 포함) · pro = 이후 추가분 | 2026-07-02 |
| D2 | 깃·로컬 폴더 분리(repo 2개·개발폴더 2개), 런타임 팩 폴더는 단일 유지 | 2026-07-02 |
| D3 | 앱(설치 파일)은 1종 — free/pro 사용자 동일 | 2026-07-02 |
| D4 | pro는 앱 고급기능도 제공 → **3층 구조: B(단일 바이너리+라이선스 게이트) 기본 + C(팩 동봉 헬퍼) 탈출구** | 2026-07-02 |
| D5 | 라이선스: **만료 열쇠 기본 + `never` 옵션 + 폐기 명단** (상세 `DESIGN-pro-license.md`) | 2026-07-02 |
| D6 | 구현 전 리뷰 라운드 선행(옵션 2) | 2026-07-02 |

## 1. 전체 구조 — 3층

```
[층 1] 앱 (1종 · 공개 채널 · 자동 업데이트)
       = free 기능 + pro 고급기능 코드(라이선스 없으면 휴면·미노출) + free 팩 내장
[층 2] 라이선스 파일 ("열쇠" · 고객별 minisign 서명 · ~1KB)
       = client_id 워터마크 + tier + 만료(기본)/never + 폐기 명단 대조
       → 내장 키링 오프라인 검증 → pro 앱 기능 활성화
[층 3] pro 팩 (고객별 서명 tarball = free 팩 ⊕ pro overlay)
       = 신규 스킬·지침·스크립트 + C-탈출구 헬퍼
```

- **B 기본**: pro 앱 기능은 단일 바이너리에 탑재하되 `license::is_pro()` 단일 게이트 뒤 휴면.
  근거: CI·서명·mac 공증·자동업데이트 1벌 유지(1인 운영 최대 비용 레버) · 검증 인프라
  (packsig) 재사용 · 위협모델(계약된 소수 신뢰 고객)에 부합.
- **C 탈출구**: "코드가 공개 바이너리에 실리는 것 자체가 수용 불가"한 극비 기능만,
  앱에 넣지 않고 pro 팩에 스크립트/헬퍼로 동봉. **판정권 = 박사님, 건별.** 기본값은 B.
  C 채택 시 플랫폼별 바이너리 헬퍼가 필요해지면 그 기능 한정 빌드 매트릭스 비용을 티켓에 명시.

## 2. 저장소·폴더 경계

| 구분 | free | pro |
|---|---|---|
| 저장소 | `idoforgod/cys-terminal` (private·현행) — 앱 소스(pro 기능 코드 포함)+free 팩 | `idoforgod/cysjavis-pack-pro` (private·**신규**) — pro 팩 overlay·병합빌드·license-issue·발급대장 |
| 개발 폴더 | `~/dev/cys-terminal/` (현행) | `~/dev/cysjavis-pack-pro/` (신규 클론) |
| 런타임(고객) | `~/.cys/pack` **단일** | 동일 단일 — pro 번들이 통째 교체(overlay 탐색경로 도입 금지: pack_dir 단일 가정 소비처 전체 리팩터+무음실패 표면 회피) |

- 별도 repo 근거: 공개 릴리스 CI가 pro 콘텐츠를 **물리적으로 볼 수 없음** — 빌드 실수에 의한
  pro 유출 사고 경로 자체를 제거(build.rs `git ls-files` SOT 철학의 연장).
- pro repo는 **overlay만** 보유(free 사본 없음 — 드리프트 원천 차단). 병합 빌드가 free를
  태그 핀(`FREE_BASE=vX.Y.Z`)으로 참조.

## 3. 산출물·버전 체계

| 산출물 | 채널 | 버전 표기 |
|---|---|---|
| 설치 파일(DMG/setup.exe) | 공개(cys-terminal-releases) — 현행 무변경 | `X.Y.Z` |
| free 팩 | 바이너리 내장(현행) | `X.Y.Z` (앱과 동행) |
| pro 팩 번들(3파일) | Drive 고객별 폴더(0단계) | base semver + `pro_revision` (표기 예: `0.8.0 pro.2`) |
| 라이선스 파일(+sig) | Drive 고객별 폴더 · 계약 시/갱신 시만 | license_id 일련 |

- **버전 계약 v2 (R1 결착 — `-pro.N` 문자열 접미 폐기)**: 기존 파서(parse_semver·version_gt)의
  '-' 이후 절단은 테스트로 핀된 의도 사양이라(pack.rs:1154) 문자열 접미는 ①replay 2차 게이트
  거부(packsig.rs:145 — pro.1→pro.2가 동급 판정) ②version_gates UpToDate(cys.rs:4105·
  pack.rs:171)의 **이중 차단**을 유발함이 실증됐다. 교정:
  - `pack_version`은 **순수 semver 유지**(기존 파서·테스트 핀 무변경).
  - manifest에 `channel`("free" 기본 / "pro") + `pro_revision`(u32 · pro 전용 단조 증분) 필드 추가.
  - 비교기는 **(base semver, pro_revision) 튜플** — free 경로는 pro_revision=0 동치로 무회귀.
    packsig replay 2차·version_gates 양쪽에 channel-aware 비교 적용.
  - pro manifest는 `min_binary_version` **필수**(구 바이너리 × 신 pro 팩 호환 파손 차단 — R1 누락 보강).
    **v3 검증 규칙(R2 codex)**: channel=pro에서 min_binary_version 비어있음·파싱불가 =
    version_gates **이전** 거부(fail-closed). channel=free는 기존 default 하위호환 유지.
  - **v3→v4 영속 상태 계약 (R2·R3 codex blocking 결착 — 비교기의 디스크 근거 + 트랜잭션 완결)**:
    디스크 측 튜플 SOT = `pack_dir/.pack-state.json` {channel, base_version, pro_revision}
    **단일 파일**(§5의 채널 마커를 이 파일로 통합 — 별도 `.pack-channel` 파일 폐기:
    channel 이중 기록 드리프트 차단). 부재 = free/0(구 설치 자연 마이그레이션) ·
    손상 = 보존 방향(§5: pro 간주 + loud 진단 + pack-update 거부→repair 선행).
    - **`.pack-version`은 최종 커밋 마커로 유지** — agy의 "파일 병합(단일 write_atomic)" 제안은
      **변형 수용**: 병합 대신 ①`.pack-state.json`을 트랜잭션 journal 백업 집합(pack.rs:588)에
      **편입**(rollback 시 함께 복원) ②**정합 검사** — 읽기 시 state.base_version ≠
      `.pack-version` 이면 state를 손상으로 간주(보존+loud+repair 요구). 사유: `.pack-version`은
      recover_pack_journal(pack.rs:539)·기존 판독처·구설치 마이그레이션의 정박점이라 병합은
      검증된 복구 기계의 재작성 = 더 큰 회귀 표면. journal 편입+정합 검사로 동일한 원자성
      보장을 외과적으로 달성한다.
    - **AcceptedPack 커밋 재배치 (R3 codex blocking)**: 현행 record_accepted는 commit_extra로
      `.pack-version` **이전**에 실행되고 journal 밖(parent dir)이라, crash 시 accepted만
      앞서가 동일 서명 번들 재시도가 replay 거부되는 교착이 실증됐다(cys.rs:4508·pack.rs:613).
      → record_accepted를 `.pack-version` 커밋 **이후 post-commit 단계**로 이동. 이 순서에서
      "팩은 커밋·accepted는 낡음" 창이 생기나, 낡은 accepted는 **안전 방향**(replay 기준선이
      뒤처질 뿐 — 구번들 재생 공격은 디스크 버전 게이트(version_gates)와 signed_at 신선도
      유효창이 여전히 차단. **낡은 accepted 창의 무해 증명(R4 agy 반박 결착)**: 이 창에서
      공격자가 이득을 보려면 디스크보다 "새" 번들이 필요한데 그것은 replay가 아니고, 디스크
      이하 버전·튜플은 version_gates가 전부 거부한다 — 따라서 agy의 "accepted를
      .pack-state.json에 병합해 단일 트랜잭션화" 제안은 **기각(사유 명기)**: 창 자체가
      무해하며, accepted는 팩 전체 교체·재설치를 **견뎌야 하는** anti-replay 기준선이라
      pack_dir 밖(parent)에 두는 현행 배치가 의도적 설계다 — pack_dir 내 병합 시 재설치가
      기준선을 소실시킨다).
      **self-heal 규정(v5 강화 — R4 codex major 결착)**: 검증 통과 manifest에 대해
      ①manifest 튜플(base, channel, pro_revision) == `.pack-state.json` 튜플 ②state.base ==
      `.pack-version` ③**설치된 디스크 트리 해시 == manifest.files 전항 일치**(기존
      verify_files는 staging만 검증 — 디스크 대조가 필수) ④signed_at > accepted.signed_at —
      **4조건 전부** 충족 시에만 파일 반영 없이 accepted 갱신. ③ 불일치 = typed
      "same-version content mismatch" 거부(동일 버전 재서명 콘텐츠 드리프트를 기준선에
      기록하는 것 차단) → 새 pro_revision 발급 또는 repair 요구. 회귀 테스트: 동일
      버전·동일 pro_revision·더 새 signed_at·다른 파일 해시 번들.
      **락 규칙(v6 — R5 codex minor)**: self-heal의 state/accepted 판독 → 디스크 트리
      해시 대조 → accepted 갱신은 pack-update apply lock **보유 중** 수행(경합 감지 시
      typed abort·재시도) — 대조 중 파일 변경 경합 차단.
      **post-commit accepted 쓰기 실패 시맨틱(R4 agy missing 수용)**: 디스크 반영은 이미
      성공이므로 롤백하지 않되 성공으로 침묵 포장 금지 — loud stderr + 전용 경고 exit
      code(EXIT_REINJECT_DEGRADED 동형 패턴)로 구분 보고, 다음 pack-update의 self-heal이
      기준선을 수렴시킨다.
    - **crash 4케이스 명세(테스트 의무)**: ①state 쓰기 전 → journal rollback 전체 복원
      ②state 후 `.pack-version` 전 → rollback이 state 포함 복원(전원단절 시 recover가 미커밋
      판정→rollback) ③`.pack-version` 후 accepted 전 → 커밋 유효·accepted 낡음(안전)·
      self-heal로 수렴 ④accepted 후 journal 정리 전 → 정상 커밋. + `.pack-version` 쓰기
      실패·crash 복구에서 accepted/state 정합 회귀 테스트.
    - `.pack-accepted.json`(AcceptedPack) 확장: {pack_version, signed_at}에 channel·
      pro_revision을 `#[serde(default)]`로 추가 — **명세(R3 agy missing)**: default =
      channel="free"·pro_revision=0, 미지 필드 무시, 구 포맷 파일 판독 마이그레이션 테스트 의무.
    - **내장 install(free 경로)의 state 동기 갱신(v4 자체 발견 — 정합 검사 오탐 차단)**:
      marker=free에서 내장 install이 `.pack-version`을 올릴 때, `.pack-state.json`이
      **존재하면** {free, target_version, 0}으로 함께 갱신한다(부재면 생성하지 않음 —
      부재=free/0 규칙 유지). 미갱신 시 pack-update 이력이 있는 free 사용자가 앱 업데이트
      직후 정합 검사(state.base ≠ .pack-version)에 걸려 오탐 동결되는 회귀를 차단.
      **v5 순서·실패 명세(R4 codex major 결착 — 현행 내장 경로는 비트랜잭션·best-effort
      쓰기(pack.rs:366·389)임이 실증됨)**: `.pack-version`을 **checked 쓰기로 먼저**(실패 =
      loud + state 미갱신 — 불일치 미생성) → 성공 후에만 `.pack-state.json` 갱신(이 갱신
      실패 = loud 경고만, 다음 기동 자가치유(§5)가 수렴). fault-injection 테스트 의무:
      내장 free 경로에서 .pack-version 쓰기 실패 / .pack-state.json 쓰기 실패 각각.
  - **v3 pro_revision 단조 빌드 가드 (R2 agy)**: pro repo 병합 빌드가 발급·배포 대장으로 강제 —
    base 동일 시 직전 배포 pro_revision보다 단조 증가 아니면 **빌드 실패** / base 변경 시에만 1로 리셋.
  - 마이그레이션·전이 테스트 의무(v3 확장): 구 free 상태 / 구 pro 상태 / 손상 상태 /
    free→pro / pro.N replay / pro.N+1 적용 / base rebase 각 케이스.

## 4. pro 고객 생애주기

```
온보딩(계약 1회): 공개 설치 파일 설치(free와 동일)
  → cys license install <열쇠>       (명령 1)
  → cys pack-update --from <번들>    (명령 2)
평시: 앱 = 자동 업데이트(행동 0) · pro 팩 = 번들 수령 시 명령 1줄 · 라이선스 = 갱신 시만
종료: 만료 도래 → pro 앱 기능 자동 휴면(free로 우아한 강등·데이터 무손상)
  + pro 팩 업데이트 중단 + Drive 권한 회수 (+ 필요 시 license_id 폐기 명단 등재)
```

## 5. 다운그레이드 가드 (앱 업데이트 × pro 팩 공존 — v2 완전 명세)

위험(R1 실측 확정): 현행 내장 install은 `.pack-version` 비교뿐이고 channel 비인지라,
동일 base의 pro 팩 위에 내장 free 팩 install이 그대로 진행되어 **prune이 pro 전용 파일을
'폐기된 옛 파일'로 오판·자동 삭제**한다(pack.rs:121 version_gt 접미 무시 → :268 가드
미발동 → :326 prune). v1의 "실측 후 구현" 가정이 파괴 실증으로 확정 — 가드는 필수다.

- **마커 (v3: 상태 파일로 통합)**: `pack_dir/.pack-state.json` {channel, base_version,
  pro_revision} — channel 값은 `free`|`pro` 단 둘. pack-update가 팩 반영과 함께 원자 기록
  (§3 트랜잭션 순서). 별도 `.pack-channel` 파일은 두지 않는다(이중 기록 드리프트 차단).
- **fail-closed 방향**: 상태 손상·미지 값 = `pro`로 간주(보존 — 무음 파괴 차단이 최우선).
  **부재 = `free`**(현행 전 설치 기기는 free이므로 자연 마이그레이션).
- **손상 진단·복구 (v3 — R2 양 리뷰어 합의 보강)**: 손상 시 무음 동결 금지 — cysd 기동·
  init-pack·status가 "channel 상태 손상 → 보존 모드(pro 간주)·내장 팩 자동 갱신 정지"를
  **명시 출력**(정상 멱등 설치와 구분되는 안정 토큰). 복구 = `cys pack-repair-channel`.
  ※ agy의 "유효 라이선스 부재면 자동 free 간주" 제안은 **사유 명기 기각** —
  라이선스 일시 무효(만료·갱신 지연)와 pro 팩 실재는 공존 가능하며, 자동 free 판정 시
  prune이 pro 파일을 삭제하는 **비가역 파괴**가 된다. 손상 비용은 "갱신 일시정지(가역)"로
  제한하고 loud 진단으로 무음성만 제거한다(codex 방향 채택).
- **손상 상태 × pack-update 시맨틱 (v4 — R3 codex major)**: `.pack-state.json` 손상 시
  pro-간주는 내장 install 차단(보존)에는 충분하나 서명 팩 적용의 튜플 판정에는 불충분 —
  **pack-update는 typed 진단과 함께 거부**하고 repair 선행을 요구한다. 단, AcceptedPack
  (서명 검증 이력 기록)에서 튜플을 재구성해 정합이 증명되면 자동 복구 제안. 손상 상태
  pack-update 테스트 의무.
- **정합 불일치의 제한적 자가치유 (v5 — R4 agy major② 변형 수용 · v6 음성 증거 가드 보강)**:
  state.base ≠ `.pack-version` 불일치라도 state가 **정상 파싱되고 channel=free**이면
  cysd 기동 시 base_version만 `.pack-version`으로 자동 동기화(loud 로그)해 free 사용자의
  수동 repair 강요를 제거한다. **손상(파싱 불가) 또는 channel=pro의 불일치는 자가치유
  금지** — 기존 보존+repair 경로 유지.
  **v6 음성 pro 증거 검사(R5 codex major 결착 — "정상 JSON이지만 거짓 free" 차단)**:
  channel=free 자가치유는 **pro 증거 부재까지 확인해야** 발동한다 — ①accepted.channel=pro
  또는 ②pro 전용 파일 실재 증거가 있으면(state가 free를 자칭해도 실체는 pro 설치) 자가치유
  **금지** + typed 진단 + `pack-repair-channel` 경로로 유도. 미검사 시 오염된 valid-JSON
  free state가 자가치유 문을 통과해 내장 install·prune이 pro 파일을 삭제하는 원 재앙이
  되살아난다. 회귀 테스트 의무: pro 설치 후 state만 valid free로 오염 → cysd/init-pack이
  내장 install·prune을 수행하지 않고 repair를 요구해야 한다.
  ※ agy의 "유효 pro 라이선스 부재 시 자가치유" 판정 기준은 **3차 기각(동일 사유)**:
  라이선스 상태는 팩 파괴 안전성의 근거가 될 수 없다(만료 라이선스 × 정당 pro 팩 공존).
  판정 기준은 라이선스가 아니라 **state의 channel 값 + 음성 pro 증거**다.
- **repair 권위 규칙 (v4 — R3 agy major·codex minor 합의 결착)**: `pack-repair-channel`의
  재기록 권위 = **AcceptedPack 기록(서명 검증 이력) + pro 전용 파일 실재 증거**. 규칙:
  ①pro로 재기록은 accepted.channel=pro 증거가 있을 때만 허용(라이선스는 정보로 표시하되
  단독 권위 아님 — 만료 라이선스 × 정당 pro 팩 공존 케이스 보호) ②증거 없는 free→pro
  전환 = 기본 거부·명시 expert override + loud 경고만 ③free로 재기록은 §5 downgrade
  명령과 동일한 license-aware 확인 경유. → agy가 지적한 악용 경로(free 사용자가 pro로
  마킹해 내장 갱신을 자가 차단·지원 부담 유발)는 ②로 차단된다(순수 free 설치는
  accepted 기록 자체가 없다 — pack-update 이력 전무).
- **경로별 동작 표**:

  | 경로 | marker=free | marker=pro |
  |---|---|---|
  | cysd 기동 install(false) | 현행 동일 | **전체 생략 (쓰기 0 + prune 0)** |
  | `cys init-pack` | 현행 동일 | 전체 생략 + 안내 출력 |
  | `cys init-pack --force` | 현행 동일 | **여전히 생략** — 습관적 force가 pro 팩을 죽이는 사고 차단. 복귀는 아래 전용 명령만 |
  | `cys pack-update` | 서명·채널·버전(§3 튜플) 검증 후 반영 + 마커 갱신 | 동일 |

- **복귀 전용 명령 (v3: license-aware — R2 양 리뷰어 독립 합의)**: `cys pack-downgrade-to-free` —
  실행 시 현재 라이선스 상태를 먼저 표시. **유효 pro 라이선스 실재 시 기본 거부**(팩만 free로
  강등되면 pro 앱 기능 ↔ 팩 콘텐츠 불일치 유발 — 명시 override 플래그로만 통과). 라이선스
  부재·만료·폐기 시 확인 후 진행(channel free 전환 + 내장 팩 재설치). pro→free 전환은 이
  명령 하나뿐이다(우회 경로 없음).
- **회귀 핀**: "앱 업데이트(내장 버전 상승) 후 pro 팩 파일 전수 생존" 테스트 의무.
- 운영 결과: pro 고객은 free X.Y+1 수정사항을 박사님의 rebase 번들(X.Y+1 / pro_revision=1)로
  받는다(§7).

## 6. 배포 채널

- **free: 현행 완전 무변경** (공개 releases + latest.json + 내장 팩).
- **pro 0단계(즉시)**: Drive 고객별 폴더(기본값·박사님 변경 가능) — 고객 계정 권한 부여·회수
  가능·이력 누적·번들 ~0.9MB. 고객별 client_id 워터마크 서명 번들.
  채널 무결성은 서명이 담당(위변조 번들은 설치 거부) — 채널 요건은 접근통제뿐.
- **pro 1단계(고객 10+ 시)**: pro repo Releases + 고객별 read-only fine-grained 토큰 +
  `pack-update --manifest-url` 인증 헤더 지원(코드 변경 1건) + GUI pro 채널 등록(버튼 1클릭).
  고객별 워터마크 ↔ 공용 번들 트레이드오프는 1단계 진입 시 결정.

## 7. 운영 규칙 (박사님 정례 업무)

| 트리거 | 박사님 행동 |
|---|---|
| 신규 pro 스킬/기능 완성 | pro repo 커밋 → 병합빌드 1회 → Drive 업로드 → 고객 알림 1줄 |
| free 새 버전 발행 | pro 번들을 새 base로 rebase 재빌드·배포 (pro 고객의 free 수정사항 수령 경로) |
| 계약 체결/갱신 | `license-issue.sh --client … --expires …`(never는 명시 옵션) → 파일 전달 |
| 유출·해지 | 발급 대장에서 license_id → 폐기 명단 등재 → **다음 앱 릴리스에 내장 전파**(단일 SOT — 위반자 기기에 실제로 닿는 유일 채널이 앱 자동 업데이트임이 R1 결착. 라이선스 설계 §5). **긴급(유출 확인) 시 revocation-only 최소 릴리스**(폐기 명단만 갱신) 즉시 발행으로 지연 상한 제거(R2 codex 보강) |
| 서명키 회전(비상 포함) | **회전 runbook**(라이선스 설계 §9): 대장 기반 유효 고객 인벤토리 → dry-run 커버리지 리포트 → 일괄 재발급 → 전달 ack 추적. 평시 회전은 구키 중첩 창(+90일) · 즉시 폐기는 키 유출 비상 전용 |
| 경계 판정 | 버그수정·호환성 = free / 신규 기능 = pro / 애매 = 박사님 건별. C 탈출구 채택도 건별 |

## 8. 위협 모델 (정직 기재 — 박사님 고지 완료)

| 시나리오 | 평가 | 방어 |
|---|---|---|
| 열쇠 위조 | 사실상 불가 | minisign(Ed25519) — 팩 채널과 동일 기술·동일 키링 |
| 열쇠 복사·공유 | 가능 | client_id 추적 + 만료 + 계약 + 폐기 명단 |
| 게이트 역공학 우회 | 어렵지만 가능 | 계약·워터마크·심층 IP는 층3(팩)·C 탈출구 |
| 서명 개인키 도난 | 최대 급소 | 로컬 단독 보관 + 키 폐기·회전(기구현) = 비상 kill |
| 한계(수용) | 중도해지 즉시회수 불가(완화: 짧은 만료 분할 발급) · 시계 rollback · 업데이트 미수신 기기 폐기 미전파 | — |

## 9. 구현 티켓 (의존 순)

| # | 티켓 | 위치 | 의존 |
|---|---|---|---|
| T1 | 다운그레이드 가드 + 버전 계약 v5: `.pack-state.json` 상태 계약(§3 — journal 편입·정합 검사·AcceptedPack post-commit 재배치·self-heal 4조건(디스크 트리 대조)·crash 4케이스·checked 쓰기 순서)·§5 동작 표·channel=free 자가치유·`pack-downgrade-to-free`(license-aware)·`pack-repair-channel`(권위 규칙)·channel/pro_revision 비교기·channel=pro min_binary_version 검증·마이그레이션+전이+crash+fault-injection 테스트·회귀 핀 | cys-terminal | — |
| T2 | 라이선스 모듈(`DESIGN-pro-license.md` §12 전체 — 검증 ⓐ~ⓕ·typed status·내장 폐기 명단 빌드타임 검증·정적 핀·인벤토리 테스트) | cys-terminal | — |
| T3 | 기능 게이트 배선: pro 기능 레지스트리 + `is_pro()` 단일 진입 + 미노출 인벤토리 | cys-terminal | T2 |
| T4 | pro repo 골격: overlay 규약·병합빌드(충돌 loud·**pro_revision 단조 빌드 가드**)·서명·워터마크·license-issue.sh(dry-run·커버리지·ack)·발급대장 | pack-pro(신규) | ★repo 생성 = 외부발행 → 박사님 집행 |
| T5 | 배포 규약 문서 + 고객 온보딩 가이드 1장 | pack-pro | T4 |
| T6 | (1단계 보류) manifest-url 인증 헤더 + GUI pro 채널 | cys-terminal | 고객 10+ 시 |

## 10. 리뷰 라운드 이력·계획 (D6 — 구현 전)

- **R1 (2026-07-02) 완료**: agy·codex 각 **BLOCK** (격리 독립 도달·핵심 좌표 일치·master
  독립 실측 3자 수렴). 결착: R2·R4 blocking(prune 파괴 실증·버전 이중 차단), R3 논리모순
  (디스크 폐기 채널은 위반자에게 실효 0), R5 커버리지 증명 부재, R6 기계 강제 요구.
  verdict: `_round/FREEPRO_{agy,codex}_R1_verdict.md`.
- **v2 교정**: 버전 계약(channel·pro_revision 튜플 — §3) · 가드 완전 명세(§5) · 폐기 내장
  단일 SOT · status typed 진단 + 키 수명 경고 · 회전 runbook · 게이트 기계 강제 4겹 ·
  issued_at 미래 검사 · min_binary_version 의무화 (하위 상세는 라이선스 설계 v2).
- **R2 (2026-07-02) 완료**: agy **REVISE** · codex **BLOCK**. 합의: R1 지적 전건 해소 확인 +
  양측이 **동일 신규 결함 2건 독립 발견**(①상태 손상 시 free 사용자 무음 동결 ②downgrade
  명령의 라이선스 비인지). codex 잔여 blocking = pro_revision **영속 상태 계약 부재**
  (디스크 위치·AcceptedPack 스키마·커밋 순서). verdict: `_round/FREEPRO_{agy,codex}_R2_verdict.md`.
- **v3 교정**: `.pack-state.json` 단일 상태 계약(AcceptedPack serde-default 확장·커밋 순서·
  마이그레이션 테스트) · 손상 loud 진단 + `pack-repair-channel` · license-aware downgrade ·
  pro_revision 단조 빌드 가드 · channel=pro min_binary_version 검증 · revocation-only 긴급
  릴리스. **기각 2건(사유 명기)**: agy "자동 free 폴백"(§5 — 비가역 파괴 위험) ·
  agy "never 보증 만료"(라이선스 설계 §6 — 서명키 not_after와 중복·D5 충돌).
- **R3 (2026-07-02) 완료**: agy **REVISE** · codex **BLOCK**. 합의: R2 지적 전건 해소 +
  기각 2건(자동 free 폴백·never 보증 만료) 논리 양측 수용. 신규: agy=상태 2파일 원자성·
  repair 악용 경로 / codex=**AcceptedPack이 journal 밖에서 최종 마커 이전에 기록되는
  crash 교착**(코드 실증 — 동일 번들 재시도가 replay 거부) + 손상 상태 pack-update 미정의.
  verdict: `_round/FREEPRO_{agy,codex}_R3_verdict.md`.
- **v4 교정**: `.pack-state.json` journal 편입 + `.pack-version` 정합 검사(agy 병합안은
  변형 수용 — 검증된 복구 기계 보존) · record_accepted post-commit 재배치 + self-heal ·
  crash 4케이스 명세·테스트 · 손상 상태 pack-update 거부→repair 선행 · repair 권위 규칙
  (accepted 증거 기반 — 양측 지적 통합 결착) · AcceptedPack serde 마이그레이션 명세.
- **R4 (2026-07-02) 완료**: agy **REVISE** · codex **REVISE** (codex BLOCK 해제 — 수렴 접근).
  합의: R3 지적 전건 해소·변형수용 논리 수용. 신규: codex=**self-heal이 디스크 트리 대조
  없이 기준선을 전진시켜 동일 버전 재서명 드리프트를 은닉**(verify_files는 staging만 검증
  — 코드 실증) + 내장 경로 best-effort 쓰기 순서 미정의 / agy=post-commit accepted 창
  재이의 + free 오탐 자가치유 요구. verdict: `_round/FREEPRO_{agy,codex}_R4_verdict.md`.
- **v5 교정**: self-heal 4조건(튜플 동일·state=version·**디스크 트리 해시=manifest.files**·
  signed_at 단조) + "same-version content mismatch" typed 거부 · 내장 free 경로 checked
  쓰기 순서(.pack-version 먼저·실패 시 불일치 미생성)·fault-injection 테스트 ·
  channel=free 한정 정합 자가치유 · post-commit accepted 실패 exit 시맨틱.
  **기각 2건(사유 명기)**: agy accepted 병합안(낡은 창 무해 증명 + accepted는 팩 교체를
  견뎌야 하는 parent-dir 기준선) · agy 라이선스 기반 자가치유 판정(3차 기각 — 판정 기준은
  state의 channel 값).
- **R5 (2026-07-02) 완료**: agy **ACCEPT**(이슈 0 — 수렴 선언) · codex **REVISE**(국소 1건).
  codex 발견: channel=free 자가치유가 "정상 JSON이지만 거짓 free" state를 구분 못함 —
  accepted=pro·pro 파일 증거 실재 시 자가치유가 prune 재앙의 뒷문(master 독립 검증 = 타당).
  + minor: self-heal 락 범위. verdict: `_round/FREEPRO_{agy,codex}_R5_verdict.md`.
- **v6 교정**: 자가치유 발동 조건에 **음성 pro 증거 검사** 추가(accepted.channel=pro 또는
  pro 전용 파일 실재 → 금지·typed 진단·repair 유도) + 오염 회귀 테스트 · self-heal은
  apply lock 보유 중 수행(경합 typed abort).
- **R6 (2026-07-02) 완료 — ★수렴 종결**: agy **ACCEPT** · codex **ACCEPT** (양측 이슈 0·
  누락 0). codex 명시: "잔여 리스크는 T1 구현 테스트 범위에 흡수 가능 — 구현 단계로 보내도
  된다." 판정 궤적: R1 BLOCK+BLOCK → R2 REVISE+BLOCK → R3 REVISE+BLOCK → R4 REVISE+REVISE
  → R5 ACCEPT+REVISE → **R6 ACCEPT+ACCEPT**. verdict: `_round/FREEPRO_{agy,codex}_R6_verdict.md`.
  **본 설계(v6)는 구현 착수 가능 상태다** — 착수는 박사님 승인 후(T1·T2부터).

## 11. 정지 경계 (자율 진행 금지 — 박사님 보유)

- pro repo 생성(`gh repo create`) · 모든 릴리스/태그/외부 발행 · 계약 문구 확정 ·
  C 탈출구 채택 판정 · 0단계 채널 최종 확정(Drive 기본값 변경 여부).
