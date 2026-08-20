# DESIGN — cys-terminal 무중단(재시작 0) 팩 업데이트 채널

> **목표**: 오너이 새 기능(디렉티브·스킬·스크립트·워크플로우 = "팩")을 개발하면, cys-terminal을
> **끄지 않고·재시작 없이·24/365 가동 중에** 그 새 기능을 살아있는 노드에 적용한다.
>
> **R0 = 설계 전용.** 본 문서는 코드 0줄이다. 모든 단정은 코드 실측(file:line)에 근거하며,
> repo = `~/dev/cys-terminal`, 브랜치 = `feat/multi-master-formalization`.
>
> **선행 문서**: `docs/DESIGN-seamless-update.md`(업데이트-버튼 = Tauri 재시작 모델)의 **보완 채널**이다.
> 그 문서는 팩을 바이너리에 묶어 재시작으로 반영한다 — 본 설계는 **팩만의 변경을 재시작 없이** 푸는
> 별도 경로를 추가한다. 두 문서는 충돌하지 않고 공존한다(§3 라우팅 참조).

---

## 0. 한 줄 요약 (TL;DR)

현재 팩은 **cys 바이너리에 `include_str!`로 embed**(`src/pack.rs:10-358`)되어, 새 디렉티브/스킬을
배포하려면 **반드시 새 바이너리 릴리스 → Tauri updater → `app.restart()`**(`src-tauri/src/main.rs:1126`)를
거친다. 이 재시작이 무중단의 적이다.

본 설계는 embed 경로를 **그대로 둔 채**(첫 설치·오프라인·복원의 바닥), 팩을 **별도 다운로드
아티팩트**(`pack.tar.gz` + `pack-manifest.json`)로도 배포하고, **이미 실행 중인 cys 바이너리**가
그것을 받아 검증→디스크 반영(`install`의 preserve-gate 재사용)→**살아있는 노드 reinject**까지
하는 경로를 추가한다. cysd·cys-app·세션은 단 한 번도 죽지 않는다.

---

## 1. 현황 실측 — 팩이 디스크에 닿는 모든 경로

| # | 경로 | 코드 | 재시작? |
|---|---|---|---|
| A | 빌드타임 embed (`PACK` const + skills walk) | `src/pack.rs:10-358`, `src/pack.rs:362`(`include!(…/pack_skills.rs)`) | — (컴파일) |
| B | 디스크 반영 코어 `install(force)` | `src/pack.rs:467-611` | 없음 (파일 쓰기만) |
| C | cysd 첫 기동 자동 설치 `install(false)` | `src/bin/cysd/main.rs:59` | 데몬 기동 시 1회 |
| D | `cys init-pack` CLI | `src/bin/cys.rs:1372-1373`, `run_init_pack` `src/bin/cys.rs:2270-2321` | 없음 |
| E | Tauri 업데이트 재시작 후 `cys init-pack --no-install-hook` | `src-tauri/src/main.rs:489-494`(`maybe_apply_pending_update`) | **있음** (`app.restart()` `main.rs:1126` 직후) |
| F | 살아있는 노드에 디렉티브 재주입 `cys reinject` | `src/bin/cys.rs:1215-1217`, `run_reinject` `src/bin/cys.rs:3889-3937` | 없음 |

**핵심 실측 결론 3가지**

1. `install(force)`(B)는 **embed 상수 `PACK`/`PACK_SKILLS`만 순회**한다(`src/pack.rs:496`).
   즉 디스크 반영 자체는 재시작이 필요 없지만, **반영할 내용이 현재 바이너리에 박힌 것뿐**이다.
   새 팩을 무중단으로 풀려면 "embed가 아닌 **외부 디렉터리**에서 같은 로직으로 반영"하는 입력
   교체가 유일한 신규 요소다. (→ §7-⑤ `install_from_iter`)
2. `install`의 **preserve-gate·prune·다운그레이드 차단·매니페스트** 로직은 입력원과
   무관한 순수 로직이다(`src/pack.rs:494-611`). 입력만 바꾸면 그대로 재사용된다. ★단 현재 파일
   본문 쓰기는 `std::fs::write`(`src/pack.rs:538`)로 **원자적이 아니다** — 무중단 채널은 진짜
   atomic write(temp+fsync+rename) helper를 신규 도입해야 한다(§7-⑤).
3. `reinject`(F)는 **디스크의 디렉티브를 `compose_directive`로 다시 읽어 주입**한다
   (`src/bin/cys.rs:3922`). 즉 B로 디스크만 갱신하면 F로 살아있는 노드를 각성시킬 수 있다 —
   **무중단 2단(디스크 반영 → 노드 재주입)이 이미 부품으로 존재**한다.

---

## 2. 설계 — 5요소

### ① 팩 아티팩트 분리 (하이브리드 — embed 보존)

- **embed는 불가침**: `src/pack.rs:10-358`의 `include_str!`, `build.rs` skills walk(`src/pack.rs:362`),
  `install(false)`의 cysd 첫 기동 자동설치(`src/bin/cysd/main.rs:59`)는 **그대로 둔다**. 이는
  ⓐ신규 머신 첫 설치 ⓑ오프라인 ⓒ다운로드 실패 시의 **바닥(floor)**이다.
- **추가 아티팩트(릴리스에 동봉)**:
  - `pack.tar.gz` — `cysjavis-pack/` 트리(=embed 입력원과 **동일 소스**)의 tarball.
  - `pack-manifest.json` — `{ pack_version, min_binary_version, key_id, signed_at, expires_at,
    files: { "<rel>": "<sha256>" } }`.
    - `pack_version` = 릴리스 태그(§3에서 단일 버전선 유지).
    - `min_binary_version` = 이 팩이 의존하는 **최소 cys/cysd 바이너리 버전**(새 RPC 의존 시 상향).
    - `key_id` = 서명 키 식별자(§7-⑩ B 키링 대조) · `signed_at`/`expires_at` = 서명 신선도 유효창
      (§7-⑩ A — Replay 차단·전건 필수 검증). 폐기는 키링의 `revoked_key_ids[]`(바이너리 측, manifest 아님).
    - ★**`key_id`·`signed_at`·`expires_at` 중 하나라도 부재 = 검증 실패(verification failure)·반영 0**
      (fail-closed — 구 2필드 manifest는 무중단 채널에서 거부됨). 본문 §7-⑩과 동일 계약.
    - 파일 해시는 `pack.rs`의 `content_hash`(`src/pack.rs:458-461`, sha256(content bytes))와
      **동일 산식**으로 계산 → 기존 매니페스트 의미론(설치-당시 해시 = 비수정 판정)이 그대로 승계.
  - **로컬 replay 상태 파일**(다운로드 아티팩트 아님 · 클라이언트 측 on-disk): `~/.cys/.pack-accepted.json`
    `{pack_version, signed_at}` — 마지막 수용 팩 기록. 단조 수용 게이트가 이보다 오래된 서명을 거부한다
    (§7-⑩ A 필수2). 키링(`{key_id, pubkey, not_after}[]` + `revoked_key_ids[]`)은 바이너리 embed(§7-⑩ B).
  - `pack-manifest.json.minisig` — **서명 대상은 manifest 단 하나**(tar가 아님). manifest가 전
    파일 sha256을 봉인하므로 manifest만 서명하면 tarball·각 파일 무결성이 전이적으로 보장된다
    (tar 별도 재서명 불요). Tauri updater와 **동일 서명 루트**(minisign, `release.yml:63`
    `TAURI_SIGNING_PRIVATE_KEY`)로 서명 — 신뢰 경계 일원화. CLI 단독 서명검증 아키텍처는 §7-①
    (현재 `Cargo.toml`엔 검증 크레이트 부재 — agy blocking).
- **동일성 보증**: 같은 태그의 embed 팩과 standalone 팩은 **같은 `cysjavis-pack/` 소스에서**
  CI가 동시 생성하므로 바이트 동일(byte-identical by construction). 이 불변식을 release.yml에
  결정론 게이트로 박는다(§5 검증).

### ② 무중단 채널 (재시작 0)

신규 흐름 — **Tauri updater를 경유하지 않는다**:

```
[폴링]   pack-manifest.json 조회 → parse_semver(remote.pack_version) 성공 시에만 디스크 .pack-version과
         strictly-greater 비교(파싱 실패=신버전 아님, 반영 안 함). ★version_gt(src/pack.rs:434)는 disk-vs-embed
         다운그레이드 가드 전용 — remote 비교에 재사용 금지(fail-CLOSED라 malformed remote를 newer로 오판, §7-④)
[가드]   parse 성공한 remote.min_binary_version ≤ 실행 cys --version 이어야 진행, 초과면 무중단 거부→바이너리(재시작) 경로(§3)
[다운로드] pack.tar.gz + pack-manifest.json + pack-manifest.json.minisig → 임시 스테이징(~/.cys/.pack-staging/)
[검증]   ⓐ manifest를 .minisig로 minisign 검증  ⓑ 검증된 manifest의 파일별 sha256 == 압축해제 트리  (하나라도 불일치=전량 폐기)
[반영]   install_from_iter로 staging 트리 조립 → post-verify → ★디렉토리 일괄 atomic 스왑(per-file rename 아님·집합 부분반영 0·prev 트리=rollback 백업) ← §7-⑤/⑧
[각성]   디렉티브 해시가 바뀐 노드만, idle 게이트 통과 후 강제 주입 ← §7-② pack-update 전용 게이트(기존 cys reinject --check 단독 사용 금지)
```

cysd·cys-app·세션 프로세스는 **그 무엇도 종료/재시작되지 않는다**. `app.restart()`(`main.rs:1126`)는
**호출되지 않는다**.

- **트리거 주체**: UI "업데이트 버튼"이 `cys` 사이드카(`resolve_sidecar` `src-tauri/src/main.rs:460-466`)의
  신규 서브커맨드(가칭 `cys pack-update`)를 spawn. 오케스트레이션은 **Tauri 계층이 아니라 `cys`(Rust)에**
  둔다 — 테스트 가능성·기존 `run_init_pack`/`run_reinject` 재사용 때문(기존 패턴: UI는 사이드카를
  shell-out, `main.rs:489-509`·`make_ticket`/`run_skill` `main.rs:543-579`).
- **실행 바이너리 제약 충족(★정정 — codex MEDIUM)**: 경계는 **"다운로드되는 팩 자체엔 새 Rust 코드가
  불요"**다 — 팩은 디렉티브·스킬·스크립트(데이터)다. **단 현재 설치된 바이너리는 `pack-update` 구현
  (install_from_iter·서명검증·staging 스왑·reinject 게이트·상태 영속)과 `min_binary_version` 게이트를
  이미 포함**해야 한다(이게 §3 라우팅·§7 전체의 전제). 즉 무중단 채널 자체는 **R2에서 바이너리에
  한 번 추가**되고(재시작 채널로 배포), 그 후의 **팩 콘텐츠 갱신**이 무중단이다. 새 팩이 새 **Rust RPC**를
  요구하면 그건 무중단 대상이 아니다(§2-③ `min_binary_version`).
- **UI 갱신 브리지(중요 — cys CLI엔 AppHandle 없음)**: 오케스트레이션이 `cys` 사이드카에 있으므로
  CLI는 Tauri `app.emit`을 직접 못 한다. → **Tauri command가 사이드카 실행을 래핑**한다:
  `install_pack_update` Tauri command(가칭)가 `cys pack-update`를 spawn→종료코드/새 pack_version을
  받아 **자신이 `app.emit("pack-updated", …)`** 한다(기존 `make_ticket`/`run_skill`이 사이드카를
  shell-out하는 패턴 `src-tauri/src/main.rs:543-579`과 동형). UI 버튼은 이 command를 호출. 상세 §7-③.

### ③ 팩/바이너리 구분 — 라우팅 (브리프의 "둘 다면?" 해소)

| 릴리스 종류 | 판정 | 경로 |
|---|---|---|
| **팩만 변경** (디렉티브·스킬·스크립트, 새 RPC 불요) | remote.pack_version > 디스크 .pack-version **그리고** min_binary_version ≤ 실행 cys | **무중단 채널**(§2-②) |
| **바이너리 변경** (cysd/cys/cys-app, 새 RPC 포함) | Tauri updater가 latest.json로 신버전 감지(기존 `check_update` `main.rs:1050`) | **기존 Tauri 경로**(재시작, 불가침) |
| **둘 다** | 아래 결정 규칙 | 둘 다 — 단 **상호 멱등** |

**"둘 다"의 결정 규칙** (단조 버전 + min_binary_version으로 결정론화):

1. 단일 버전선 유지: 팩만 바뀌어도 태그·`CARGO_PKG_VERSION`을 범프한다. 따라서
   **어떤 바이너리의 embed 팩 버전 = 그 바이너리 버전**이고, 같은 태그의 standalone 팩과 동일.
2. 새 RPC를 요구하는 팩 변경은 `min_binary_version`을 그 태그로 올린다 → 무중단 채널이
   **스스로 거부**하고 사용자에게 "바이너리 업데이트 필요(재시작)"를 안내. RPC 불요 변경은
   `min_binary_version`을 그대로 둬 무중단으로 즉시 반영.
3. **상호 멱등(핵심 안전)**: 두 경로 모두 같은 파일을 **같은 preserve-gate·같은 버전 스탬프**로
   쓴다. 무중단으로 팩 vX를 먼저 깔면 디스크 `.pack-version=vX`. 이후 사용자가 바이너리 vX로
   업데이트→재시작하면 `maybe_apply_pending_update`의 `init-pack`(`main.rs:489`)이 도는데,
   embed==디스크라 **0 written(no-op)**. 반대로 바이너리부터 깔리면 무중단 폴링이 "이미 최신"으로
   no-op. **충돌 불가**.
4. **기존 불변식이 무중단 팩을 보호**: 무중단으로 새 팩(vX > 실행 바이너리 버전)을 깐 뒤
   구 바이너리 cysd가 재기동되면, `install(false)`의 **다운그레이드 차단**(`src/pack.rs:480-492`,
   디스크 .pack-version > embed → `(0,0)` 반환·보존)이 구 embed 팩의 회귀를 결정론 차단한다.
   → **무중단 팩은 데몬 재기동을 견딘다**(공짜로 얻는 안전).

### ④ reinject 자동화 — 안전 경계 + 중복주입 회피

- **무엇을 reinject하나(범위 최소화)**:
  - **디렉티브/soul/메모리 색인 변경** → reinject 필요(노드 컨텍스트에 박혀 있어 디스크만 바꿔선
    안 깨어남). `compose_directive`(`src/bin/cys.rs:2533`,3922)가 디스크에서 재조립.
  - **스킬·스크립트·워크플로우 변경** → **reinject 불요**. 이들은 파일로 사용 시점에 읽힌다
    (`bin/*.py` 실행, `skills/*/SKILL.md` 호출). 디스크만 갱신되면 다음 호출에 자동 반영.
  - → 설계 규칙: **무중단 반영 후, 합성 디렉티브 해시가 바뀐 surface만 reinject**.
- **안전 경계(언제)**: 생성/툴콜 도중 주입 금지. 기존 `reinject --check`의 ACK-핑은
  **단독으로는 위험**하다 — Busy(추론 중) 노드가 핑에 응답 못 하면 timeout을 드리프트로 **오판**해
  실행 중 컨텍스트에 전문 디렉티브를 강제 주입, 대화 이력을 파괴한다(`src/bin/cys.rs:3920`,
  agy major). → **Busy 가드 + 디렉티브 해시 선검사를 ACK 핑 앞에 둔다**(상세 §7-②).
- **중복주입 회피(dedup)**: `boot_agent_on_surface`의 resume 분기 패턴 차용
  (`src/bin/cys.rs:2665-2675` — resume 노드엔 전문 디렉티브 **재주입 안 함**, 짧은 복귀 가드만).
  무중단 reinject도 **surface별 "마지막 주입 팩 버전" 마커**를 두고, 마커 < 새 pack_version인
  surface만 주입. 한 번 vX를 주입한 노드는 같은 vX로 다시 주입하지 않는다.
- **컨텍스트 오염 완화**: 전문 디렉티브 재주입은 토큰 2배·컨텍스트 임계(clear) 유발 위험이
  명시돼 있다(`src/bin/cys.rs:2666`). 완화: 디렉티브가 **실제로 바뀐 경우에만** 전문 재주입,
  그 외엔 짧은 델타 통지("팩 vX 반영됨 — `~/.cys/pack/directives/<역할>` 재숙지")만. 스킬/스크립트
  변경은 통지조차 불요(위 범위 규칙).

### ⑤ 무중단 검증 기준 (cys-app·cysd·세션 전부 생존 실측)

결정론 검증 스크립트가 **반영 전/후 스냅샷 동등성**을 비교(브리프의 "어떻게 실측하나" 답):

| 생존 대상 | 측정 | 합격 |
|---|---|---|
| **cysd** | `cys identify` → `daemon_pid`(`src/bin/cys.rs` identify; 실측 예 `daemon_pid:92226`) | 전/후 **동일 pid** |
| **cys-app** | Tauri 앱 프로세스 pid | 전/후 **동일 pid** (`app.restart()` 미호출) |
| **세션** | `surface.list` → surface_id 집합·`exited` 플래그 (★session_id는 응답에 없음 `handlers.rs:678` — 사용 금지) | 집합 불변·전부 `exited:false` |
| **팩 반영** | 디스크 `.pack-version` == 새 버전 + 비수정 파일 sha256 == manifest | 일치 |
| **노드 각성** | §7-② pack-update 전용 게이트: idle 확인(`control.dashboard` state) **후 강제 주입** → 주입 **이후** 새 디렉티브에만 있는 사실 인용 확인 (★`cys reinject --check`는 ACK 수신 시 주입을 **생략**하므로 `src/bin/cys.rs:3916` 각성검증으로 부적합) | idle에서 주입됨 + 인용 성공 |

**불합격 조건(hard fail)**: daemon_pid 또는 app pid가 바뀌면 = 재시작 발생 = 무중단 위반 = FAIL.

---

## 3. 불가침 (건드리지 않는다)

| 대상 | 코드 | 이유 |
|---|---|---|
| 첫 설치 embed 경로 | `src/pack.rs:10-358,362`, `build.rs` | 오프라인·신규 머신·복원의 바닥 |
| preserve-gate | `src/pack.rs:502-533` | 사용자 수정 파일 불가침 계약 |
| 기존 Tauri updater(바이너리) | `src-tauri/src/main.rs:1082-1127` | 바이너리 교체는 재시작이 정상 |
| pack.rs version/prune 로직 | `src/pack.rs:434-456,543-589` | 다운그레이드·기능제거 배포의 검증된 로직 (★파일쓰기 자체는 `std::fs::write` 비원자 — §7-⑤에서 공용 atomic helper로 격상) |

신규 코드는 전부 **가산적(additive)**: `install_from_iter`(입력만 (rel,content) 이터레이터), `cys pack-update`
서브커맨드, manifest 신규 필드(`min_binary_version`·`key_id`·`signed_at`·`expires_at`). `install(force)`의
**시그니처·동작은 불변**(embed `PACK`를 넘기는 얇은 래퍼로 남김 — C/D/E 호출처 무영향, 하위호환).

---

## 4. 리스크별 완화책 (브리프 필수)

| 리스크 | 완화책 | 근거 |
|---|---|---|
| **embed 첫설치 보존** | embed·`install`·cysd 자동설치 불가침. 무중단 채널은 가산·온라인 옵트인. 다운로드 실패 시 embed가 바닥. | `src/pack.rs:10-358`, `src/bin/cysd/main.rs:59` |
| **다운로드 무결성** | minisign 서명(Tauri와 동일 키) + 파일별 sha256(manifest). 임시 스테이징에 받아 **전체 검증 후** 반영. 하나라도 불일치 = 전량 폐기. | `release.yml:63`, `src/pack.rs:458` 동일 산식 |
| **reinject 컨텍스트 오염** | 디렉티브 변경 시에만 전문 재주입·그 외 델타통지/무통지. ACK 게이트. surface별 버전 마커로 반복 차단. | `src/bin/cys.rs:2666,2665-2675,3902-3920` |
| **팩 vs 바이너리 버전 정합** | 단일 버전선 + `min_binary_version` 거부 게이트. **버전 비교 3축 정책분리(§7-④)**. version_gt fail-CLOSED(파싱 실패=보존). embed≥standalone(동일 태그)이라 바이너리 업데이트가 팩을 supersede. | `src/pack.rs:434,453`, §3-③, §7-④ |
| **부분 적용(다운로드 중 실패)** | **staging 트리 조립→전수검증→디렉토리 일괄 atomic 스왑**(per-file rename 아님·집합 부분반영 0). prev 트리=rollback 백업·swap 저널로 half-swap 복구. 미커밋 시 `.pack-version` 불변→다음 폴링 재시도(§7-⑤/⑧). | `src/pack.rs:538-589`, §7-⑤·⑧ |
| **force 다운그레이드 사일런트 롤백** | `init-pack --force`는 다운그레이드 차단을 우회(`src/pack.rs:480`) → 무중단 상위 팩이 구 embed로 사일런트 롤백 위험(agy minor). **force 다운그레이드 가드(§7-⑦)**. | `src/pack.rs:480-492`, §7-⑦ |
| **메모리 상주 stale (런타임 캐시)** | 디스크 갱신돼도 Tauri UI·MCP·노드가 구 캐시 고수(agy major). **리로드 트리거(§7-③)**. | §7-③ |

---

## 5. 출시 전 검증 (end-to-end)

1. **동일성 게이트(CI)**: release.yml에서 `pack.tar.gz`의 파일별 sha256 == embed `PACK`의
   `content_hash`임을 결정론 비교. 불일치면 빌드 FAIL(embed/standalone 드리프트 차단).
2. **무중단 실측 1회**: 살아있는 노드 1개 상태에서 `cys pack-update` →
   §2-⑤ 표의 daemon_pid·app pid·surface 집합 **전부 불변** + `.pack-version` 범프 +
   노드 각성은 §7-② 게이트(idle 확인 후 강제 주입 → **주입 이후** 새 디렉티브 사실 인용)로 확인
   (★`reinject --check` 단독 사용 금지 — ACK 수신 시 주입 생략 `cys.rs:3916`).
3. **min_binary_version·서명 만료 폴백**: `min_binary_version` > 실행 cys면 무중단 **거부**→바이너리
   경로 안내 emit 확인. ★만료된 서명(`expires_at` 경과)·만료 key_id 팩 주입 시 **검증 거부**(§7-⑩) 확인.
4. **다운그레이드 견딤**: 무중단 vX 반영 후 구 바이너리 cysd 재기동 → `install(false)`가
   `(0,0)`으로 디스크 vX를 보존하는지(`src/pack.rs:485-491`) 확인.
5. **부분실패 복구(staging 스왑)**: 다운로드 중단/해시 위변조/swap 중 crash 시 → 스왑 미커밋·
   reader는 prev 트리만 봄(혼재 0)·`.pack-version` 불변·swap 저널로 포인터 복구·다음 폴링 재시도(§7-⑤).
6. **마커 영속**: pack-update 후 cysd 재기동 → surface별 `pack_reinject` 마커가 topology에서 복원되어
   **동일 버전 재주입이 발생하지 않음**(§7-⑪) 확인.

---

## 6. 오너 결정 사항 — ★확정(R2 구현 반영 완료)

> 4자 수렴 ACCEPT 후 오너 5건 확정. 아래는 결정 + as-built 반영(file:line).

1. **버전선 정책 → 단일 버전선**(태그 범프). min_binary: **embed≥standalone**(동일 태그 임베드 팩이 standalone과 동일). `cys pack-manifest`/version_gates(`src/bin/cys.rs`)·release.yml 동일성 게이트로 보장.
2. **트리거 UX → ★처음 수동 트리거**: `cys pack-update` 명시 호출(자동 폴링 X). UI는 `install_pack_update` Tauri command(`src-tauri/src/main.rs`)가 사이드카 래핑·`pack-updated` emit. "검증 후 자동 전환"은 후속.
3. **reinject → pack-update가 apply 후 자동**(idle·dedup 게이트). 단일 write path=`reinject.mark` RPC(`src/bin/cysd/handlers.rs:1980`).
4. **★심링크 마이그레이션 → 안 함**. ⑤ apply-lock+epoch 폴백을 **최종 채택**(live `round/` 무접촉). `with_apply_lock`+`install_from_iter`(.pack-version=epoch 맨 마지막). ★한계 정직 문서화: 외부 동시 reader(compose_directive·read_board_catalog)에 대한 multi-file SET 일관성은 writer-side 배타로만 보장(sub-second·수동트리거라 노출 창 희소·reinject는 apply 후)(`src/bin/cys.rs:4193` 독스트링). 강화 옵션(reader shared-flock)은 가용성 트레이드오프로 보류·master 판단 시 추가.
5. **서명 유효창 → 기존 Tauri 키 재사용**(신규키 X·`build.rs` embed 단일SOT key_id=39E60A702949D6C3). per-manifest `expires_at`=릴리스 CI 90일(`release.yml` EXPIRES_DAYS)·키 `not_after` 필수(`src/packsig.rs`).

**구현 상태: R2 P1~P7 전부 완료**(11 tracked +1763/-17·신규 packsig.rs·trusted-keys.json·noshutdown_verify.py·격리 E2E PASS·미커밋). 발행(git push·gh release·cysd 재시작)은 오너 승인 게이트.

---

## 7. R1 감사 보강 (REVISE 7종) — 적대검증 반영

> agy verdict(`_round/REVIEWER_AGY_VERDICT.json`, REVISE) + codex(surface:281) 지적을 코드 실측으로
> 보강한다. 각 항목은 **결함 → 실측 근거 → 설계 보강**. 코드 0줄(설계 명세).
>
> **R3 정합 패스**(codex `_round/REVIEWER_CODEX_VERDICT2.json` REVISE 반영): §7만이 아니라 본문
> §1·§2·§3·§5/검증표를 §7과 정합시켰다 — ①서명 대상 manifest 단일화(tar 아님) ②remote 비교에서
> `version_gt` 재사용 제거(parse_semver 전용·다운그레이드 가드만 version_gt) ③§2-⑤/§5 노드 각성검증을
> pack-update 전용 게이트로 교체(`reinject --check`는 ACK 수신 시 주입 생략 `cys.rs:3916`이라 부적합)
> ④app.emit UI 브리지(Tauri command가 사이드카 래핑) ⑤`std::fs::write` 비원자 명시·atomic helper 신규
> ⑥검증표 session_id 제거(`surface.list`에 없음 `handlers.rs:678`) ⑦release.yml 아티팩트 생성·서명·동일성
> 게이트를 R2 표에 추가.
>
> **R4 통합 보완**(폐기된 master v2 R1 검증서 4결함 + codex BLOCK `_round/REVIEWER_CODEX_VERDICT2.json:307`):
> ⑧multi-file 집합 원자성(per-file atomic만으론 reader가 신/구 혼재 — apply-lock+epoch+apply후emit, `cys.rs:2533-2602`·`main.rs:521`)
> ⑨queued/quiet 게이트≠turn-boundary(`governance.rs:1341-1370` quiet 3초 — ②에 진짜 idle 다중신호 통합)
> ⑩서명키 유출 대비 키링·회전·만료(min_binary 불충분 — key_id+not_after, 신뢰근원 변경은 바이너리 채널)
> ⑪pack-reinject 마커 지속성(Surface 신규 필드+topology 영속 `state.rs:40-105`, `agent_session_id` 패턴 차용)
> + **codex BLOCK 해소**: rollback 저널이 sha256만으론 복원 불가 → 백업 바이트 저장·atomic rename 복원(§7-⑤).
>
> **R5 종합 통합**(master가 codex R0+v2+R2 BLOCK을 단일 8항목으로 종합 + agy R1재감사 2 major):
> ③의 **전체 atomic을 staging 디렉토리 일괄 스왑으로 격상**(master 지시 — per-file in-place rename 폐기,
> reader 집합 혼재 0). `install_from_dir`→**`install_from_iter`**(embed/staged 공용 추출). prev 트리=rollback
> 백업이라 codex BLOCK 백업바이트 요구가 더 견고히 해소. live `round/`(`cys.rs:3628-3643`) carve-out +
> 심링크 1회 마이그레이션은 §6 오너 결정(거부 시 apply-lock+epoch 폴백). ⑩에 **per-manifest 서명 만료
> `signed_at`/`expires_at` 추가(agy major — Replay Attack 차단**, 키 `not_after`와 층위 구분). ⑪ "세션 원장
> 영속"으로 명문화(agy major). agy minor(백업 용량) → stale purge 명시. 본문 §2 흐름·§5 검증·R2 표를 전부 정합.
>
> **R6 정밀 보완**(codex 7종 닫힘 확인 + agy R3 REVISE 확정 2건): ⑤서명 만료 가드에 **명시적 Key Revocation
> 2층**(키링 `revoked_key_ids[]` 긴급 폐기 + 짧은 `expires_at` 시간봉인 중첩방어, not_after 만료 대기 불요) 추가.
> ⑧reinject 마커를 **메모리 아닌 on-disk topology 영속**으로 명문화(cysd 기동 시 로드 복원·마커 유실로 인한
> 전 노드 중복주입·컨텍스트 오염 차단). 나머지(codex 7종)는 R3~R5에서 닫힘.
>
> **R7 구현등급 보강**(codex 정식 REVISE `_round/REVIEWER_CODEX_VERDICT3.json` 4종 — 실행 계약화):
> [HIGH]①서명 fail-closed **완성** — 키 `not_after` **전 서명키 필수**(optional 제거)·`signed_at`/`expires_at`
> **전 manifest 필수 검증**·**persisted accepted-pack(`{pack_version,signed_at}`) 단조 수용 게이트 mandatory**
> (구버전 서명 거부)·신규머신은 time-window 한계 명시. [HIGH]②reinject **단일 write path 확정** — `status.set`
> 확장 폐기, **전용 `reinject.mark` RPC(주입 성공 후에만 호출)** + Surface 필드·init·`persist_topology`
> (`governance.rs:703`)/`load_topology` 직렬화·restore seeding 명세. [MEDIUM]③codex **`ready_marker` 부재**
> (`agents.json:46-52`) → **어댑터별 fallback predicate**(idle+quiet)로 영구 deferral 방지. [MEDIUM]④"no new
> Rust code" stale **정정**(다운로드 팩엔 불요, 단 설치 바이너리는 pack-update 구현+min_binary_version 게이트 포함).

### ① [BLOCKING] minisign 서명검증 아키텍처 — CLI 단독 검증

**결함**(agy blocking, `Cargo.toml:23`): cys `Cargo.toml` 의존성은 `sha2`까지뿐, **minisign 검증
크레이트가 없다**(`Cargo.toml:23-38` 실측). 서명 검증은 현재 **Tauri updater 플러그인**
(`src-tauri/Cargo.toml:13` `tauri-plugin-updater`)만 할 수 있고, 그 공개키는 **Tauri 측에만** 있다
(`src-tauri/tauri.conf.json:44` `updater.pubkey` = base64 minisign 공개키). 무중단 채널은 Tauri를
우회해 `cys` CLI가 직접 검증해야 하므로, **현 상태로는 CLI 서명검증이 물리적으로 불가능**.

**설계 보강**:
- **크레이트 추가(R1 구현 대상)**: `Cargo.toml [dependencies]`에 검증 전용 순수-Rust 크레이트
  `minisign-verify`(jedisct1, verify-only·서명키 미포함이라 공격면 최소)를 추가. Tauri와 **같은
  minisign 형식**이라 서명 자산(`*.minisig`) 재사용 가능.
- **공개키 단일 SOT**: 키를 두 곳에 두면 드리프트(검증 실패·우회)의 원천. `src-tauri/tauri.conf.json:44`의
  `pubkey`를 **빌드타임에 cys 바이너리로 embed**한다 — `build.rs`가 `tauri.conf.json`을 읽어
  `OUT_DIR`에 상수로 방출(skills walk가 이미 `build.rs`에서 도는 패턴 `src/pack.rs:362`과 동형).
  → 키가 1곳(tauri.conf.json)에서만 관리되고 양쪽이 동일 보장. CLI 인자 `--pubkey`는
  **테스트/회전 오버라이드용 옵션**으로만(기본=embed).
- **검증 순서(fail-closed)**: ⓐ`pack-manifest.json`을 `pack-manifest.json.minisig`로 minisign 검증
  → ⓑ검증된 manifest의 파일별 sha256으로 압축해제 트리 대조(§2-②). 서명 대상은 **manifest 단 하나**이고,
  manifest가 전 파일 해시를 봉인 → tar 자체 재서명 불요·부분 위변조 차단. 어느 단계든 실패 =
  스테이징 폐기·반영 0.

### ② reinject Busy 가드 + 디렉티브 해시 선검사

**결함**(agy major, `src/bin/cys.rs:3920`): ACK-핑은 Busy(추론 중) 노드를 idle과 구분 못 해
timeout→드리프트 오판→전문 주입으로 **대화 컨텍스트 파괴**. 또 매 업데이트마다 핑 텍스트가
**토큰 영구 누적**.

**실측 — 이미 있는 부품**:
- 노드 상태 판정 `derive_node_state`(`src/bin/cysd/handlers.rs:436-463`): 스크롤백 키워드
  ("esc to interrupt"·"working"·"generating"·"thinking" `handlers.rs:438`) + `idle_secs` 폴백으로
  working/idle 산출. `control.dashboard` RPC가 이를 노출(`handlers.rs:2317-2355`).
- 출력 정지 시간 `last_output.elapsed()`(`src/bin/cysd/governance.rs:496,971`).
- 노드 자기보고 상태 `status.set`(`handlers.rs:1879-1973`, working/done/waiting/blocked) →
  `control.dashboard`가 병합 노출(`handlers.rs:2221`).
- 합성 디렉티브는 `compose_directive`(`src/bin/cys.rs:2533`)로 결정론 산출 → 해시 가능.

**설계 보강 — reinject 게이트 3단(순서 고정)**:
1. **디렉티브 해시 선검사**: 새로 합성한 `compose_directive(role)` 해시 == 해당 surface의
   "마지막 주입 디렉티브 해시" 마커면 **주입 자체를 스킵**(불필요 핑·토큰 0). 스킬/스크립트만
   바뀐 릴리스는 디렉티브 해시 불변 → 전 노드 reinject 스킵(§2-④ 범위 규칙의 결정론 집행점).
2. **Busy 가드(★진짜 idle — §7-⑨와 통합)**: `last_output` quiet 단독은 **turn-boundary가 아니다**
   (busy 노드가 무출력 3초여도 주입됨 — §7-⑨). → **다중 신호 AND**: ⓐ`derive_node_state`==`idle`
   (스크롤백에 "esc to interrupt"·"generating"·"thinking" 키워드 부재 `handlers.rs:438`) **AND**
   ⓑ자기보고 `agent_status` ≠ `working`(`status.set`) **AND** ⓒ**prompt-ready predicate** 충족일 때만 진행.
   `working`이면 **주입 보류(Deferred)** → 다음 idle 신호(`pane.idle` push `governance.rs:975`)에서 재시도.
   ACK-핑은 이 가드 통과 후 **확인용**으로만(주입 여부 판단엔 미사용).
   - ★**어댑터별 prompt-ready predicate(codex MEDIUM — 영구 deferral 방지)**: ⓒ는 `agents.json`의
     `ready_marker`(claude `❯` `agents.json:7`·gemini `? for shortcuts` `:27`)를 쓴다. 그러나 **codex 어댑터엔
     `ready_marker`가 없다**(`cysjavis-pack/agents.json:46-52` — cmd·resume_arg만). 마커 미정의 어댑터는
     ⓒ가 영원히 거짓→**영구 deferral**이 된다. → **fallback predicate**: `ready_marker` 부재 어댑터는
     ⓒ를 **`derive_node_state`==idle AND `last_output` quiet ≥ 임계(예 ACK timeout)** 로 대체한다(ⓐ와
     사실상 합치되 quiet 임계로 turn-boundary 근사). ★권장: agents.json에 codex `ready_marker`(또는
     `ready_predicate`)를 추가해 결정론화(R2 옵션) — 둘 중 하나는 **모든 어댑터에 필수**.
3. **버전 마커 dedup**: surface별 "마지막 주입 pack_version" < 새 버전인 노드만(§2-④ 기존).

→ Busy 노드 컨텍스트 파괴 0, 디렉티브 무변경 시 핑·토큰 0.

### ③ 메모리 상주 리로드 트리거 (런타임 캐시 동기화)

**결함**(agy major): 디스크 팩이 갱신돼도 **메모리 상주 소비자가 구 캐시 고수** → 갱신 유실(stale).
대상: ⓐTauri UI(예: `read_board_catalog`가 `board-catalog.json`을 호출 시 읽음
`src-tauri/src/main.rs:521-527` — UI가 한 번 읽고 캐시하면 stale) ⓑMCP 서버 ⓒ워커/리뷰어 노드.

**설계 보강 — 소비자별 리로드 계약**:
| 소비자 | 캐시 성격 | 리로드 트리거 |
|---|---|---|
| **Tauri UI** | 보드 카탈로그·설정 메모리 캐시 | ★cys CLI엔 AppHandle 없음 → **Tauri command가 `cys pack-update` 사이드카를 래핑**해 종료 후 **자신이** `app.emit("pack-updated", {pack_version})`(브리지, §2-② "UI 갱신 브리지"). 프런트가 `read_board_catalog`(`main.rs:521`) 등 재호출. 대안: cysd event bus로 push 후 UI 구독(emit 패턴 `main.rs:498,1095` 동형) |
| **디렉티브 의존 노드**(master·worker·reviewer) | 컨텍스트 상주 디렉티브 | §7-② reinject(디렉티브 변경 시·idle에서) |
| **스크립트·스킬**(`bin/*.py`·`skills/*/SKILL.md`) | **무캐시**(사용 시점 파일 읽기) | 트리거 불요 — 다음 호출에 자동 신버전 |
| **MCP 서버**(serena 등 scoped 데몬) | 프로세스 메모리 | idle 시점 graceful: `cys run`(scoped 실행 · `src/bin/cys.rs`)로 띄운 것은 생명주기 강제종료가 보장되므로, 팩이 MCP 설정/바이너리를 바꾼 경우 **해당 scoped 프로세스만** idle에서 재기동(전체 무중단 유지). 설정만 바뀌고 서버가 핫리로드 지원하면 시그널, 아니면 idle graceful restart. |

원칙: **무캐시 자산(스크립트·스킬)은 트리거 불요, 캐시 자산만 명시 리로드**. 어느 소비자도
cys-app/cysd 재시작을 요구하지 않는다(§5 무중단 불변).

### ④ 버전 비교 3축 정책분리

**결함**(codex missing): `remote`·`disk`·`min_binary` 세 버전의 비교 의미가 §2-②에 섞여 있다.
명시 분리:

| 비교 | 대상 | 연산 | 실패/경계 처리 |
|---|---|---|---|
| **반영 판정**(remote→disk) | `remote.pack_version` vs 디스크 `.pack-version`(`src/pack.rs:429`) | **신규 `parse_semver(remote)` 성공 시에만** strictly-greater 비교. ★`version_gt`(`src/pack.rs:434`) 재사용 금지 — 그건 첫 인자 파싱 실패를 true로 처리하는(`src/pack.rs:451-453`) **디스크 보존용**이라 malformed remote를 newer로 오판한다 | 파싱 실패·같거나 낮음 = 반영 안 함(멱등) |
| **호환 게이트**(remote→running) | `remote.min_binary_version` vs 실행 `cys --version`(`CARGO_PKG_VERSION`) | parse 성공 후 `min_binary ≤ running` 이어야 반영 | 파싱 실패·초과 = 무중단 **거부**→바이너리(재시작) 경로 안내(§3) |
| **다운그레이드 차단**(disk→embed) | 디스크 `.pack-version` vs embed `CARGO_PKG_VERSION` | `install` 기존 `version_gt` 가드 그대로(`src/pack.rs:480-492`) — 이 축은 fail-CLOSED 보존이 **옳다** | 디스크 > embed = 보존(§3-④) |

- **API 분리 명세**: remote 비교는 **신규 `parse_semver()→Option<(u32,u32,u32)>` + 명시 compare**(파싱 실패=None=반영 안 함, fail-CLOSED **거부** 방향). disk-vs-embed 다운그레이드 가드만 기존 `version_gt`(파싱 실패=보존 방향) 유지. **둘은 실패 시 안전 방향이 반대**(remote=반영거부 / disk=보존)라 같은 함수로 묶으면 안 된다.
- **단조성 불변**: 어떤 축도 버전을 낮추지 않는다. 세 비교가 모두 참일 때만 무중단 반영.

### ⑤ 전체 atomic 적용 + rollback (staging 디렉토리 일괄 교체 — ⑧과 단일 계약)

**결함**(codex BLOCK `_round/REVIEWER_CODEX_VERDICT2.json:307` + master 종합 ③): per-file in-place
쓰기는 ⓐ쓰는 도중 crash 시 부분 파일(`std::fs::write` 비원자 `src/pack.rs:538`) ⓑ다중 파일 집합의
신/구 혼재(§7-⑧)를 못 막는다. 또 저널에 sha256만 기록하면 복원 소스가 없어 rollback 불가.

**설계 보강 — staging 일괄 atomic 스왑(★per-file in-place rename 아님)**:
1. **`install_from_iter(items)`**: `install`의 코어(preserve-gate·prune·버전 판정 `src/pack.rs:496-589`)를
   **`(rel, content)` 이터레이터**를 받는 순수 함수로 추출 — embed `PACK` iter(기존 경로)와 staged-tree
   iter(무중단)가 같은 로직을 공유(중복 0·회귀 0). 기존 `install(force)`는 이를 embed iter로 호출하는
   얇은 래퍼로 남김(하위호환·§3).
2. **staging build**: 다음 상태 콘텐츠 트리 **전체**를 `~/.cys/.pack-next/`에 조립 — ⓐ보존 대상
   (사용자 수정본·preserve-gate 통과분) copy-forward + ⓑ새 팩 overlay(staging 내부는 reader가 없으니
   per-file 원자쓰기로 충분) + ⓒ`.pack-version`·매니페스트 포함.
3. **post-verify**: staging 트리를 manifest sha256으로 **전수 대조** → 실패면 staging 폐기·반영 0.
4. **atomic swap(전체 일괄)**: `pack_dir` 콘텐츠를 새 트리로 **단일 atomic 연산**(버전 디렉토리
   심링크 flip — `ln -sfn` 한 syscall, 또는 동일 FS rename)으로 교체. reader는 항상 **all-old 또는
   all-new**(혼재 0·multi-file 부분반영 0). **직전 트리를 `~/.cys/.pack-prev`로 보존 = rollback 백업**
   → codex BLOCK#1 해소(백업 바이트 = prev 트리 통째, sha256만 기록하는 문제 원천 제거).
5. **swap 저널**: 어느 트리가 authoritative인지 1줄(`~/.cys/.pack-swap.json`, 원자 쓰기). crash가
   swap 도중이면 포인터로 결정론 복구.
6. **rollback / commit**: 검증·swap 실패 = 포인터를 `.pack-prev`로 atomic 되돌림. commit 성공 =
   `.pack-prev` 삭제·`.pack-next` 정리. ★**stale purge(agy minor)**: commit·다음 pack-update 착수
   시점에 `~/.cys/.pack-prev`·`.pack-next`·잔존 staging의 옛 트리를 **명시적 일괄 퍼지**(디스크 누적
   방지) — 단 직전 1개(롤백용)는 보존.
- **live 상태 carve-out(★필수)**: `round/`(노드 TODO·SESSION_STATE 실시간 기록 — cycle 저장 타깃
  `src/bin/cys.rs:3628-3643`)·사용자 오버라이드는 **스왑 트리에서 제외**(스왑이 라이브 쓰기를 덮지
  않게). → `round/`는 안정 실디렉토리로 두고 **콘텐츠 트리만 버전닝**. 이를 위해 pack_dir을 버전
  심링크로 두는 **1회 마이그레이션** 필요 → §6 오너 결정.
- **시작 시 회수(순서 고정)**: cysd 기동 시 swap 저널 잔존이면 **cysd `install(false)`
  (`src/bin/cysd/main.rs:59`)보다 먼저** 포인터를 확정(half-swap 자가 치유).
- **마이그레이션 거부 시 폴백**: 심링크 전환이 부담이면 **apply-lock(flock)+epoch+apply후 emit**(§7-⑧
  폴백)으로 강등 — 단 이 경우 "부분반영 0"은 락 보유 중 reader 차단으로만 보장(약함). 우선안은 스왑.

### ⑥ 무중단 검증 — 측정 가능한 RPC shape

**결함**(codex missing): §2-⑤ 검증표가 "어떤 RPC로 무엇을 읽나"가 추상적. 실측 RPC로 고정:

| 검증 | RPC(실측) | 읽는 필드 | 합격 조건 |
|---|---|---|---|
| cysd 생존 | `system.identify`(`handlers.rs:485`) | `daemon_pid` | 전/후 동일 |
| cys-app 생존 | OS pid (Tauri 앱) | 프로세스 pid | 전/후 동일 (`app.restart()` 미호출) |
| 세션 생존 | `surface.list`(`handlers.rs:641`) | `surfaces[].surface_id`·`exited` | 집합 불변·전부 `exited:false` |
| 노드 상태(주입 안전) | `control.dashboard`(`handlers.rs:2317`) | `state`(derive_node_state) | reinject는 `idle`에서만 |
| ACK | `surface.read_text`(`handlers.rs:976`)+`surface.wait_for` | `latest_cursor`·`matched` | `DIRECTIVE-ACK-<pid>` 수신(`src/bin/cys.rs:3905-3916`) |
| 팩 반영 | 파일 | `.pack-version`·파일 sha256 | 새 버전·manifest 일치 |

검증 스크립트 = (반영 전 스냅샷) → `cys pack-update` → (반영 후 스냅샷) **동등성 diff**.
`daemon_pid` 또는 app pid 변동 = 재시작 = **hard fail**.

### ⑦ force 다운그레이드 가드

**결함**(agy minor, `src/pack.rs:480`): `init-pack --force`가 다운그레이드 차단을 우회 →
자동화/hook이 force 남용 시 무중단 상위 팩(예 v0.4.2)이 구 embed(v0.4.1)로 **사일런트 롤백**.

**설계 보강**:
- **force ≠ 다운그레이드 허용 분리**: `force`는 "사용자 수정 파일 덮어쓰기" 의미로 국한하고,
  **다운그레이드(디스크 > embed)는 force여도 기본 거부**한다. 진짜 롤백이 필요하면 별도 명시
  플래그(`--allow-downgrade`)로만 — 의도적·가시적.
- **구현 위치**: `src/pack.rs:480` 가드를 `if !force` → `if !allow_downgrade`로 좁히고, force는
  preserve-gate 우회만 담당. 무중단 채널은 `--allow-downgrade`를 **절대 전달하지 않는다**.
- **하위호환**: 기존 `init-pack --force`(같은/상위 버전 재설치) 동작은 불변 — 다운그레이드일 때만
  거동이 바뀐다(사일런트 롤백 → 명시 거부). cysd 자동설치(`install(false)`)는 영향 없음.

### ⑧ multi-file 집합 일관성 — 왜 staging 일괄 스왑인가 (⑤와 단일 계약)

**결함**(폐기된 master v2 R1 검증서 발견): per-file temp+rename은 **각 파일이 통째임**만 보장,
**집합**은 반영 도중 신/구 혼재 가능. reader 증거:
- `compose_directive`(`src/bin/cys.rs:2533-2602`)는 한 번 호출에 **디렉티브+RSI_DIRECTIVE+soul.md+
  MEMORY.md+각 SKILL.md**를 차례로 읽는다(`cys.rs:2540,2548,2554,2561,2572`) → 새 디렉티브 + 옛 soul
  조합으로 노드가 각성하는 비일관 위험.
- `read_board_catalog`(`src-tauri/src/main.rs:521-526`)는 호출 시 `board-catalog.json`을 읽는다 →
  다른 파일과 버전 불일치(per-file atomic은 '부분 파일'만 막고 '집합 불일치'는 못 막음).

**해결 = §7-⑤의 staging 디렉토리 일괄 atomic 스왑**(master 종합 ③): 콘텐츠 트리를 통째로 단일
연산으로 flip하므로 reader는 **스왑 전/후 한 트리만** 본다 → 집합 혼재 0·multi-file 부분반영 0.
디렉토리 단위 단일 flip이 per-file in-place rename보다 강하다. 보조로 reinject(§7-②)는 스왑 완료·
idle에서만 발동하고, UI emit(§7-③/④)도 스왑 후에만 보내 reader 재호출이 항상 완결 상태를 읽게 한다.

**폴백(심링크 마이그레이션 거부 시)**: `~/.cys/.pack-apply.lock`(flock) 임계영역 + `.pack-version`
epoch(seqlock: reader가 version→파일→version 재확인) + apply 후 emit. 단 "부분반영 0"이 락 보유 중
reader 차단으로만 보장돼 약하다 — **우선안은 스왑**(§7-⑤).

> ⚠**구현 현황(정직 명시 · R2)**: 현재 코드(`with_apply_lock`, `src/bin/cys.rs`)는 §6-4 심링크
> 마이그레이션(우선안=디렉터리 일괄 atomic 스왑)이 보류돼 이 §7-⑧ 폴백을 ship하되, **flock을
> writer(pack-update)만 취득**한다. load-bearing 리더(`compose_directive` `cys.rs` · `read_board_catalog`
> `src-tauri/src/main.rs`)는 공유 flock(LOCK_SH)을 취하지 않으므로, 위 폴백이 약한 수준에서 요구한
> **reader-측 차단조차 실현되지 않는다.** 따라서 §6-4 도입 전까지 **외부 동시 리더에 대한 multi-file
> 집합 일관성은 보장되지 않으며**(1초 미만 apply 창에 신규-directive + 구-soul 혼재 관측 가능), 현재
> 보호는 **writer 측 상호배제까지**다. pack-update 자신의 reinject는 apply 이후라 안전하고, 노출
> 대상은 외부 동시 리더뿐이다. 진짜 집합 원자성은 §6-4 심링크 스왑(또는 리더 측 LOCK_SH 추가)으로만 확보된다.

### ⑨ queued/quiet 게이트는 turn-boundary가 아니다 (②와 통합)

**결함**(폐기된 master v2 R1 검증서 발견): 기존 큐 배달 `deliver_queued`는 **turn-boundary가 아니라
`last_output` quiet ≥ 3초**(`src/bin/cysd/governance.rs:1341-1370`, `quiet_for < queue_quiet_secs()`)로만
게이트한다. **추론 중(busy)인데 출력이 3초 멎은** 노드(긴 사고·도구 대기)에 디렉티브가 끼어들어
컨텍스트를 오염시킨다.

**설계 보강**: §7-② Busy 가드는 **quiet 단독을 신뢰하지 않는다**. 진짜 idle/turn-boundary 신호 =
ⓐ`derive_node_state`==idle(생성 키워드 부재 `handlers.rs:438`) ⓑ`agent_status`≠working ⓒprompt-ready
마커 가시 — 셋의 AND(§7-② step2). 또한 reinject 주입은 **큐(`--queued`) 경로를 쓰지 않고** 전용
게이트를 통과한 직접 주입으로 한다(큐의 quiet-3초 오배달 회피). 향후 결정론 강화: cysd가 prompt-ready
edge를 1회성으로 노출하는 신호(예 `pane.ready` push)를 도입하면 turn-boundary가 결정론화된다(R2 옵션).

### ⑩ 서명키 유출·회전·만료 + 서명 만료(Replay 차단) (min_binary_version 불충분)

**결함**(폐기된 master v2 R1 검증서 + agy R1재감사 major): `min_binary_version`은 **버전 정합**만
막을 뿐, ⓐ**서명 개인키가 유출**되면 공격자가 임의 팩에 유효 서명을 붙일 수 있고(키는 단일 pubkey
`src-tauri/tauri.conf.json:44`·회전/폐기 수단 없음), ⓑ**서명에 시간적 유효성이 없어 Replay Attack에
취약**하다 — 과거에 정당 서명됐으나 이후 취약점이 발견된 **구버전 팩을 재전송**하면, 디스크
`.pack-version`이 없는 신규/초기화 머신은 그 구 팩을 정당한 신버전으로 받아들인다(다운그레이드
가드는 디스크 버전이 있어야 작동 `src/pack.rs:480`).

**설계 보강 A — 서명 fail-closed 완성(★Replay 차단, agy major + codex HIGH)**: 아래 3가지는 **전부
필수(mandatory)** — 선택·조건부가 아니다(fail-closed 완성).
- **(필수1) `signed_at` + `expires_at` 전 manifest 검증**: `pack-manifest.json`(서명 대상)에 발행·만료
  시각을 넣어 **함께 서명**하고, **모든 manifest 검증에서** 현재 시각이 `[signed_at, expires_at]` 밖이면
  **거부**한다(필드 부재도 거부 = fail-closed). 짧은 유효창(예 N일)으로 오래전 서명된 구 팩 재전송 차단.
  → 키 만료(`not_after`, B)와 **층위가 다름**: B=키 수명, A=이 아티팩트 한 건의 신선도.
- **(필수2) persisted accepted-pack replay 계약**: 마지막으로 **수용한** 팩의 `{pack_version, signed_at}`을
  디스크(예 `~/.cys/.pack-accepted.json`, 원자 쓰기)에 기록하고, 이후 검증에서 **`signed_at ≤ 수용본.signed_at`
  (또는 pack_version ≤ 수용본.pack_version)이면 거부**한다(단조 수용 게이트 — 조건부 아님·항상 적용).
  → 정당 서명됐던 구 팩의 재전송을, 만료창 안이어도 차단.
- **(필수3·한계 명시) 신규 머신**: 디스크에 수용 기록이 **없는** 초기 상태에서는 (필수2)가 비교 기준이
  없어 작동 못 한다 → **신규 머신 replay는 ⓐ`expires_at` 만료창 + ⓑ`min_binary_version` 하한으로만
  time-window-bounded 차단**된다(★이 한계를 명시 — 유효창 내 구 팩 1회 수용 가능성). 첫 수용 즉시
  (필수2) 기준선이 생겨 이후 봉인.

**설계 보강 B — 키링 + 회전 + 키 만료**:
- **단일 pubkey → 신뢰 키링**: cys에 embed하는 것을 단일 키가 아니라 **trusted-keys 목록**
  (`{key_id, pubkey, not_after}[]` — ★`not_after`는 **모든 팩 서명키에 필수**, optional 아님: 만료 없는
  키는 영구 유효라 fail-closed 위반)으로 둔다. `build.rs`가 `tauri.conf.json` pubkey + 회전용 키링
  파일에서 생성(§7-①의 embed 경로 확장).
- **manifest에 key_id**: `pack-manifest.json`에 서명 `key_id`를 명시 → 검증 시 **해당 key_id의
  非만료(now < not_after)·非폐기 키로만** 대조. 알 수 없는/만료/폐기/`not_after` 없는 key_id = 거부(fail-closed).
- **회전 절차**: 새 키를 키링에 추가한 **바이너리 릴리스**를 먼저 배포(중첩 윈도) → 신키로 서명
   시작 → 구키 `not_after` 도달 후 폐기. ★키링 자체가 바이너리에 embed라 **키 회전은 바이너리
   업데이트(재시작 채널)를 통해서만** 가능(닭-달걀: 무중단 채널이 자기 신뢰근원을 바꿀 순 없음).
   이 제약을 문서화 — 무중단은 **콘텐츠** 채널이고, **신뢰근원 변경은 바이너리 채널** 소관.
- **긴급 폐기(★Key Revocation — agy R3 확정)**: `not_after` 만료를 기다리지 않는 **명시적 폐기 수단**
   2층:
  - ⓐ**키링에 `revoked_key_ids[]`**: 유출 키 key_id를 폐기 목록에 올린 **긴급 바이너리 릴리스** →
    해당 key_id 서명 **즉시 전량 거부**(만료 시각과 무관). minisign 자체엔 폐기 목록이 없으므로
    **키링의 revoked 목록 + not_after 만료가 폐기 수단**이다.
  - ⓑ**짧은 `expires_at`(설계 보강 A)**: 폐기 릴리스 배포 전 공백을 덮는다 — 서명 유효창이 짧으면
    유출 키로 만든 위조 팩도 **유효창 경과 후 자동 거부**(시간 기반 봉인). A(아티팩트 신선도)와
    B(키 폐기)가 **중첩 방어**.
  - ★키링 변경(폐기 반영)은 바이너리 채널을 통해서만 — 무중단은 자기 신뢰근원을 못 바꾼다(위 회전 제약).

### ⑪ pack-reinject 추적 필드 — 지속성 (Surface state)

**결함**(폐기된 master v2 R1 검증서 발견): §7-② step1·step3의 "마지막 주입 디렉티브 해시 / pack_version"
마커가 **어디에 사는지·재기동을 견디는지** 미설계. 메모리에만 두면 cysd 재기동 시 전 마커 소실 →
다음 pack-update가 **전 노드 일괄 재주입**(토큰 폭증·컨텍스트 파괴).

**설계 보강 — 단일 확정 write path(★codex HIGH — 대안 제거, 실행 계약 1개)**:
1. **Surface 신규 필드**: `pack_reinject: Mutex<Option<PackReinject>>`에 `PackReinject{pack_version:String,
   directive_hash:String}`. 위치 = `src/bin/cysd/state.rs:40-105`의 Surface 구조체, `agent_session_id`
   (`state.rs:85-87`) 옆. **초기화**: surface 생성 시 `Mutex::new(None)`(미주입).
2. **단일 write path = `reinject.mark` RPC(cysd 매개)**: `status.set` 확장이 아니라 **전용 RPC로 확정**한다
   (혼선 제거). 호출 주체는 **pack-update/reinject 컨트롤러뿐**이고, **노드 주입이 성공한 직후에만** 호출
   (`reinject.mark {surface_id, pack_version, directive_hash}`) → cysd가 해당 Surface의 `pack_reinject`를
   set. self-declared 신뢰 금지(`state.rs:97`)에 맞춰 노드 자기보고로는 갱신 불가(cysd-인증 발신만).
3. **persist 직렬화**: `persist_topology`(`src/bin/cysd/governance.rs:703` 부근 — 현재 entry는
   `{role, agent, agent_bin, cwd, title, session_id}`만 직렬화)에 **`"pack_reinject": s.pack_reinject.lock()...clone()`
   한 키 추가**. 쓰기는 기존 `write_json_atomic`(temp+fsync+rename+fsync dir — 실측 helper)로 그대로.
4. **load/restore seeding**: `load_topology`가 topology.json에서 `pack_reinject`를 읽어, **restore/launch가
   Surface 재구성 시 해당 필드를 seed**(session_id를 resume 핀으로 seed하는 경로와 동일 지점). →
   cysd 재기동·노드 복원 후에도 마커 생존.
- **dedup 사용**: pack-update는 `pack_reinject.pack_version < 새 버전` **또는** `directive_hash 상이`인
  surface만 재주입(§7-② step1·step3) → 마커 유실로 인한 전 노드 일괄 재주입·컨텍스트 오염 차단.
- **하위호환**: 필드 부재(구 topology)·`None` = "미주입" 안전 폴백(첫 pack-update에서 1회 주입).
- **system.topology 노출**: 디버깅·검증(§5-6)이 필요하면 `system.topology`/`control.dashboard`에 읽기 전용
  으로 노출(옵션 — write는 오직 `reinject.mark`).

### R1 감사 보강 — 코드 변경 예고 (R2 구현 대상, 본 R0/R1은 0줄)

| # | 파일 | 변경(설계) |
|---|---|---|
| ①⑩ | `Cargo.toml`, `build.rs` | `minisign-verify` 의존 + **trusted-keys 키링 embed**(`{key_id, pubkey, not_after(필수)}` + **`revoked_key_ids[]`**) + manifest `signed_at`/`expires_at` **전건 필수 검증** + **`~/.cys/.pack-accepted.json` 단조 수용 게이트** |
| ②③⑨ | `src/bin/cys.rs`(신규 `pack-update`·reinject 3단 게이트·진짜 idle 다중신호·**어댑터별 ready predicate**), `src-tauri/src/main.rs`(신규 `install_pack_update` command — `pack-updated` emit 브리지) | Busy 가드·해시 선검사·UI 리로드·codex deferral 방지 |
| ④⑤⑦⑧ | `src/pack.rs` | **`install_from_iter` 추출**(embed/staged 공용)·remote/disk/embed 3축 비교 분리·**staging 일괄 atomic 스왑(prev=백업)·swap 저널·stale purge**·force/allow_downgrade 분리 |
| ⑪ | `src/bin/cysd/state.rs`(신규 `pack_reinject` 필드+init)·`governance.rs:703`(persist_topology에 `pack_reinject` 키 추가)·`load_topology`+restore seeding·**신규 `reinject.mark` RPC**(주입 성공 후에만 호출) | reinject 마커 단일 write path·재기동 복원 |
| ⑥ | `docs`/검증 스크립트 | RPC 기반 동등성 diff(`system.identify`·`surface.list`·`control.dashboard`) |
| ①⑦⑩ | `.github/workflows/release.yml` | **신규**: `pack.tar.gz` + `pack-manifest.json`(+`key_id`·`signed_at`·`expires_at`) 생성·업로드, `pack-manifest.json.minisig` 서명, **embed `PACK` 해시 vs standalone manifest 동일성 게이트**(불일치=빌드 FAIL, §5-1) |

---

## 부록 A — 실측 근거 인덱스 (file:line)

- embed `PACK`/`include_str!`: `src/pack.rs:10-358` · skills walk: `src/pack.rs:362`
- `content_hash`(sha256): `src/pack.rs:458-461` · `version_gt`(fail-CLOSED): `src/pack.rs:434-456`
- `install(force)` 코어·preserve-gate·prune·다운그레이드차단·버전기록: `src/pack.rs:467-611`
- cysd 첫 기동 자동설치: `src/bin/cysd/main.rs:59`
- `run_init_pack`: `src/bin/cys.rs:2270-2321`
- `run_reinject`(ACK-핑·재조립): `src/bin/cys.rs:3889-3937` · `compose_directive`: `src/bin/cys.rs:2533`
- `boot_agent_on_surface` resume 중복주입 분기: `src/bin/cys.rs:2665-2675`
- Tauri `check_update`/`install_update`/`app.restart()`: `src-tauri/src/main.rs:1050,1082-1127`
- `maybe_apply_pending_update`(재시작 후 init-pack): `src-tauri/src/main.rs:483-510`
- `resolve_sidecar`(UI→cys shell-out): `src-tauri/src/main.rs:460-466`
- 릴리스 2-repo·minisign·아티팩트: `.github/workflows/release.yml:4-9,22,56-105`
- **R1 보강 근거**: cys deps(minisign 부재): `Cargo.toml:23-38` · Tauri updater 플러그인: `src-tauri/Cargo.toml:13` · minisign pubkey SOT: `src-tauri/tauri.conf.json:44`
- 노드 상태 판정 `derive_node_state`: `src/bin/cysd/handlers.rs:436-463`(키워드 438) · `control.dashboard`: `handlers.rs:2317-2355`(상태병합 2221)
- idle 출력정지 `last_output.elapsed`: `src/bin/cysd/governance.rs:496,971` · `pane.idle` push: `governance.rs:975`
- 자기보고 상태 `status.set`: `handlers.rs:1879-1973` · `system.identify`: `handlers.rs:485` · `surface.list`: `handlers.rs:641` · `surface.read_text`: `handlers.rs:976-1031`
- UI 카탈로그 읽기 `read_board_catalog`: `src-tauri/src/main.rs:521-527` · UI emit 패턴: `main.rs:498,1095` · scoped 생명주기 `run_scoped`: `src/bin/cys.rs:3942`
- **R7 보강 근거(codex VERDICT3)**: `agent_session_id` Surface 필드: `src/bin/cysd/state.rs:85-87` · `persist_topology` 직렬화(session_id만): `src/bin/cysd/governance.rs:703` 부근 · `write_json_atomic`(실측 atomic helper temp+fsync+rename+fsync dir): `governance.rs`(persist_topology 직후 정의)
- **어댑터 `ready_marker`**: claude `❯` `cysjavis-pack/agents.json:7` · gemini `? for shortcuts` `:27` · **codex 부재** `:46-52`(cmd·resume_arg만)
