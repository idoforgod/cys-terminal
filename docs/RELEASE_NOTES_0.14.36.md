# v0.14.36 — 컨텍스트 숫자는 실측을 먼저 보고, 못 잰 것은 못 쟀다고 말합니다

> **먼저 읽어 주세요 — 이 판의 번호가 0.14.33 다음인데 0.14.36 인 이유.**
>
> `v0.14.34` 와 `v0.14.35` 는 2026-09-12 에 GitHub release·태그·홈페이지 파일에서 **삭제**됐습니다
> (`releases/latest` = `v0.14.33`). 0.14.35 는 공개됐다가 삭제돼 이미 받은 설치본이 있을 수
> 있으므로, 다음 발행 번호는 그보다 큰 **0.14.36** 입니다(범프 커밋 `fdd45f3` 본문).
>
> 이 문서가 다루는 범위는 **범프 커밋 `fdd45f3` 이후의 9개 커밋(`967e202` … `d979ac4`)** 입니다 —
> 2026-09-06 전수 감사의 후속 회차(WP-2 · WP-6)와 그 적대 검증(성찰 1회) 반영분입니다.
> `v0.14.33` 과 `fdd45f3` 사이의 13개 커밋(온보딩 재설치 감지·윈도우 빌드 수리 등 0.14.34/0.14.35
> 에 실렸던 내용)은 **같은 트리에 포함되지만 이 문서에서 정리하지 않았습니다** — 그 판들의
> 노트는 이 저장소에 없고, 여기서 새로 쓰면 태그 대조 없이 쓰는 글이 되기 때문입니다.
>
> 이 판의 팩 하한(`PACK_MIN_BINARY`)은 **0.14.31 그대로**입니다(아래 「업그레이드하실 때」).

이번 판을 한 문장으로 줄이면 이렇습니다 — **"모른다"를 값으로 접지 않습니다.** 컨텍스트 사용률을
읽는 자리가 여섯 군데였고 그 여섯이 서로 다른 규칙을 썼습니다. 어떤 자리는 27시간 묵은
자기보고를 "지금 값"으로 읽어 5분마다 경보를 울렸고, 어떤 자리는 결측을 `0%` 로 접어 경보
목록에서 조용히 빠뜨렸습니다. 이번 판은 그 규칙을 한 벌로 통일하고, **실측을 먼저 보되 실측이
없으면 신선한 자기보고만, 그것도 없으면 "판정 불가"** 를 그대로 화면에 씁니다.

같은 원칙을 감사가 짚어 둔 fail-open 세 자리(데드맨·가림 플래그·staging 삭제)에도 적용했습니다 —
못 읽으면 통과가 아니라 **거절이나 보호** 쪽으로 무너집니다.

---

## 1. 컨텍스트 축 — 실측 우선 · 경보가 줄 수 있습니다(의도)

**무엇이 문제였나.** 좌석의 컨텍스트 사용률은 두 축으로 들어옵니다.

| 축 | 어디서 오나 | 성격 |
|---|---|---|
| 실측 `usage.ctx_pct` | 데몬이 claude 상태줄에서 잰 값 | 정확 · agy/gemini 좌석에는 **구조적으로 없음** |
| 자기보고 `status.context_pct` | 좌석이 `cys set-status --context N` 으로 신고한 값 | 추정 · 신고를 멈추면 **낡은 채 남음** |

종전에는 `cys status` 표는 자기보고만 읽고 HUD 는 실측을 읽어 **같은 좌석에 다른 숫자**가
떴고, 5분 주기 보고 게이트는 자기보고의 나이를 보지 않아 **27시간 묵은 값으로 매 5분 WARN** 을
냈습니다. 그 WARN 이 `consecutive_quiet` 를 리셋해 조용한 세션을 주차시키는 경로(`quiet_cycles`
→ master-park)에 **영영 도달하지 못하는** 부작용까지 있었습니다(성찰 1회 ③ · hazard medium).

### ⓐ 규칙을 한 벌로 — 네 언어, 같은 수

`실측 > 신선한 자기보고(마지막 신고 후 300초 이내) > 판정 불가`. 이 규칙이 네 자리에 같은 모양으로
있습니다 — Rust `src/bin/cys.rs` `ctx_cell` · TS `ui/src/ctxpick.ts` `pickCtx` · Python
`javis_report.py` `pick_node_ctx` · `javis_hud_bridge.py` `pick_ctx`. 상수 300 은 네 파일에 흩어져
있으므로 검체가 **네 파일의 리터럴을 정규식으로 뽑아 같은지 단언**합니다(추출 실패 = hard fail).
종전 검체는 파이썬 상수만 300 과 비교해 변별력이 0 이었습니다(성찰 ⑤).

### ⓑ `cys status` · `cys fleet` 의 CTX 칸 — 네 가지 표기

| 표기 | 뜻 |
|---|---|
| `78%` | **실측**(데몬이 잰 값 · 표식 없음) |
| `78%~` | **자기보고·추정** — 실측은 없고 자기보고가 신선(≤300초)할 때. `~` 가 붙습니다 |
| `?` | **판정 불가** — 자기보고가 낡았거나 나이를 모름, 또는 **좌석 종료·에이전트 사망**(`exited == true` ∨ `agent_alive == false`)으로 동결된 값이라 읽지 않음 |
| `-` | **없음** — 두 축 다 없음(60% 판정의 입력이 되지 않습니다) |

**agy/gemini 좌석의 `?` 나 `~` 는 정상입니다** — 그 어댑터에는 실측이 없어 자기보고가 신선할 때만
`~` 로 뜨고, 신고가 끊기면 `?` 로 내려갑니다. 칸 폭이 4→5 로 늘었습니다(`100%~` 5자). `--json`
스키마는 바뀌지 않았습니다.

- 실패 방향(사망 분기): `exited`/`agent_alive` 키 부재·`null` 은 게이트를 열지 **않습니다** — "모른다"는
  "죽었다"가 아닙니다(구버전 데몬 페이로드 무해).
- 실패 방향(자기보고 분기): `age_secs` 결측은 "방금"이 아니라 "모른다" → `?`.
- **실측에 나이 게이트를 걸지 않기로 했습니다(마스터 결정).** 데몬이 낡은 실측을 지우는 범위는
  휴리스틱 매핑뿐이고 등록 매핑·statusline 값은 나이로 지워지지 않는데, 그렇다고 300초 게이트를
  걸면 **유휴 좌석 전부가 `?` 가 되어 오경보**가 됩니다 — 유휴 좌석의 실측은 여전히 정확합니다.
  대신 페이로드에 있는 사망 신호 두 축으로 막습니다(위 표의 `?`).

### ⓒ 60% 임계 소비자 전부를 실측 우선 축으로 — 경보가 줄 수 있습니다

UI(`ui/src/main.ts` 소비자 8곳)·보고기(`javis_report.py` 60% 판정선)·게이트(`javis_report_gate.py`)가
전부 위 규칙을 씁니다. 종전 UI 는 `context_pct ?? 0` 으로 **결측을 0% 로 접어** 60% 목록에서 조용히
빠뜨렸고(80% 배너도 같은 형태), 보고기는 낡은 자기보고를 값으로 읽어 60% 를 울렸습니다.

- **운영 고지: 낡은 자기보고가 더 이상 60% 를 울리지 못하므로 경보 건수가 줄 수 있습니다. 의도한
  감소입니다.** 실측이 있는 좌석은 실측으로, 없는 좌석은 신선한 자기보고로만 판정합니다.
- 화면: 자기보고 값은 `≈78%` 로 그리고 툴팁 "노드 자기보고(추정)" · 판정 불가는 **`?` 칩으로 보이게**
  그립니다(없으면 없다고 표시 — 숨기지 않습니다). 60% 판정 문구에는 출처가 붙습니다
  (`role(78% 실측)` / `role(65% 추정)`).
- 실패 방향: 결측·낡음·나이 미상 → 60% 목록에서 **빠집니다**(0% 로 위장하지 않음). CTX 헬퍼
  import 가 실패하면(팩 부분 갱신 스큐) 컨텍스트 경보 **없음** + 5분 보고 대장 `reasons` 에
  `report_module_missing` 이 남습니다 — 측정 수단 부재를 "이상 없음"으로 읽지 않습니다.
- `live_nodes` 엔트리에 `usage_ctx_pct` 가 **추가**됐습니다(기존 키 삭제 0). 이 키는 시간 파생값이라
  게이트의 diff 블랙리스트에 넣어 매 주기 DELTA 가 폭주하지 않게 했습니다.

### ⓓ 두 축의 괴리를 상시 측정 — 5분 보고 게이트의 `ctx_divergence`

같은 좌석에서 실측과 자기보고가 **둘 다 있고 둘 다 신선할 때** 그 차이를 잽니다.
`|자기보고 − 실측| > 8` 이면 대장·배지에 경보가 남습니다(push 없음) — 문구는 부호를 포함합니다:

```
worker: 자기보고 95 vs 실측 78 = +17
```

부호를 남기는 이유는, 편차가 한 방향으로만 며칠 쌓이면 **체계적 오차**(신고 습관·창 크기 가정)가
드러나기 때문입니다. 구조화 기록은 게이트 관측 저장부 `last_measurement.json` 의 `ctx_divergence`
와 `ctx_divergence_stats{compared, skipped_stale, skipped_missing, skipped_dead, skipped_missing_module}`
입니다 — "괴리 0" 과 "비교한 게 0건" 을 이 계수로 가릅니다.

**괴리는 판정을 움직이지 않습니다.** 괴리 행은 verdict 를 WARN 으로 바꾸는 `warns` 에 들어가지 않고,
대장 `reasons`(`ctx_divergence:<문구>`) · 배지(`gate-ctx-divergence-<role>`) · 관측 JSON 에만 매 주기
보존됩니다. WARN·quiet 카운터·DELTA 라우팅은 그대로이므로 **지속 괴리가 조용한 세션의 주차
(`quiet_cycles`)를 막지 않습니다** — 성찰 ③ 이 짚은 "27시간 묵은 값으로 매 5분 WARN → 주차 도달 불능"
경로를 신선도 게이트와 이 분리, 두 겹으로 닫았습니다.

- 실패 방향: 한 축이라도 결측 → 판정 제외·경보 없음(0 으로 접지 않음 · `skipped_missing`). 자기보고가
  낡음(>300초)·나이 미상 → 비교 제외(`skipped_stale`). **사망 좌석**(`exited == true` ∨
  `agent_alive == false`)은 동결값이라 비교 제외(`skipped_dead` · `null`/부재는 사망이 아님). CTX 헬퍼
  import 실패 → 전 노드 비교 안 함(`skipped_missing_module` = 노드 수 · 대장 `report_module_missing`).
- 실환경 5분 주기 경보량은 **아직 관측하지 않았습니다**(첫날 임계 조정 여지 — 그래서 임계를 env 로
  뺐습니다, 아래).

### ⓔ 착수 전 대조 프로브 `ctx-compare` — 기본 임계 15 → 8

`javis_actprobe.py ctx-compare` 의 기본 임계가 **15 에서 8** 로 내려갔습니다. 인수인계 기록에 남은
가장 작은 사고가 9pt 였고, 옛 기본값은 그것을 통과시켰습니다. 영수증/JSON 에 `diff` · `measured` ·
`reported` · `usage_source` 네 필드가 실립니다 — 종전에는 exit 코드만 남아 사후에 "얼마나
벌어졌나"를 읽을 수 없었습니다.

- 실패 방향: 한 축이라도 못 재면 **exit 3**(판정 불가 = 행동 금지 + 수동 확인 · 착수 허가 아님).
  임계 자체가 무효(nan·inf·음수·비숫자)여도 **exit 3** + 영수증 사유 `threshold_env_invalid` —
  종전에는 무검증 float 라 nan/inf 에서 **임의 괴리가 PASS** 로 통과했습니다(성찰 ① · block).

### ⓕ 새 환경변수 `CYS_CTX_DIVERGENCE_PCT` (기본 8)

ⓓ 와 ⓔ 두 소비자가 같은 이름을 읽습니다. 무효 값의 처리는 소비자마다 **다르게 무너지고, 둘 다
소리를 냅니다** — 둘 중 어느 쪽도 조용히 기본값으로 돌아가지 않습니다.

| 소비자 | 유효 범위 | 무효 값(빈 값·비숫자·nan·inf·범위 밖)이면 |
|---|---|---|
| `javis_report_gate.py`(5분 보고) | 유한·양수 | 기본 **8.0** 으로 동작하되 5분 보고 대장 `reasons` 와 배지에 `ctx_divergence_env_invalid` 를 남김 |
| `javis_actprobe.py ctx-compare` | 유한·비음수 | **exit 3** + 영수증 사유 `threshold_env_invalid` |

게이트 쪽을 죽이지 않는 이유: 종전 코드는 모듈 최상위에서 `float(env)` 를 그대로 불러 **빈 값
하나에 import 가 죽고 게이트 전체가 침묵**했습니다("항상 exit 0" 계약 위반). 게이트는 죽지 않고
사유를 남기는 쪽이고, 프로브는 못 재면 행동을 막는 쪽입니다.

---

## 2. fail-open 세 자리 수리 (WP-2) — 못 읽으면 거절·보호

감사 REPORT 의 H-14 계열입니다. 셋 다 "조회 실패 = 통과" 로 접혀 있던 자리입니다.

### ⓐ 데드맨 — 미래 mtime 은 stale 입니다

`src/bin/cysd/deadman.rs` `heartbeat_stale`: heartbeat 파일의 mtime 이 **미래**라(시계 역행 ·
타임스탬프 보존 복원) 경과 시간을 못 구하는 칸이 `unwrap_or(false)` = 신선 으로 접혀 있었습니다.
독 코멘트("조회 실패 = stale")와 코드가 어긋난 자리입니다. 이제 `unwrap_or(true)` — 못 재면 stale.
동작이 바뀌는 칸은 **[pid 알려짐 ∧ 무응답 ∧ 미래 mtime]** 하나로, 종전 Healthy(회수 불가·crashloop)
→ Dead 입니다. 응답하는 홀더는 `judge_holder` 가 `responded` 를 먼저 보므로 미래 mtime 이어도
오살되지 않습니다.

> **킬 창(정직).** 그 `responded` 는 단발 1회·2초 프로브 값입니다. 따라서 산 홀더가 SIGTERM→SIGKILL
> 대상이 되는 창은 정확히 **[단발 2초 프로브 실패(부하·락 경합 등 일시 지연 포함)] × [미래 mtime]**
> 입니다. 이 수리는 "hung 홀더 영구 wedge" 를 없애는 대가로 "일시 무응답 산 홀더 오살" 가능성을
> 딱 이 창만큼 엽니다. "오살 없음"이 아닙니다. 완화 후보 두 가지(회수 전 프로브 2차 시도 · 미래
> skew 가 임계를 넘을 때만 stale)는 주석으로 예약했고 이번 판에서는 로직을 바꾸지 않았습니다.

### ⓑ 가림 플래그 `redact` — 비-bool 은 조용한 off 가 아니라 거절

`control.sessions` · `control.session_detail` 의 `redact` 파라미터는 종전 `as_bool().unwrap_or(false)`
라 `"true"` · `1` · `["true"]` 가 전부 **조용한 off** 로 접혔습니다 — 가림 요청이 소리 없이 무시되면
PII 가 새고도 아무도 모릅니다. 이제 두 RPC 가 같은 판독기 `parse_redact_flag` 를 씁니다.

| 값 | 판정 |
|---|---|
| `true` / `false` | 그대로 |
| `null` · 키 부재 | **off** — tauri 브리지가 `Option<bool>::None` 을 `null` 로 싣는 정상 트래픽 |
| 그 밖의 타입(`"true"` · `1` · 배열 …) | **`invalid_params` 거절** |

`CYS_CONTROL_REDACT=1` 은 종전대로 파라미터와 OR 입니다. 비-bool 을 보내던 기존 호출자는 저장소
전수 grep 0건입니다(회귀 없음).

**가림 범위(있는 그대로).** `control.sessions` 는 `session_id` 를 해시(`sess-` + 8hex)로 바꾸고
집계 지표는 보존합니다. `control.session_detail` 은 `session_id` 해시 + 전사 `transcript` 를 `[]`
로 비웁니다. **`star_note`(즐겨찾기 노트 · 사용자 자유 텍스트)는 `redact=true` 에서도 raw 입니다** —
PII 가 들어갈 수 있는 필드라 오너 결정 항목으로 이월했습니다(아래 「미착수·이월」 #31).

### ⓒ `control.session_detail` 이 `redacted: bool` 을 에코합니다

`control.sessions` 와 같은 모양입니다(필드 추가 = 하위 호환). 가림 여부를 **응답만 보고** 판정할 수
있어야 반출 파이프라인이 그것을 게이트로 쓸 수 있습니다. `docs/CONTROL_CENTER_DESIGN.md` 의 RPC
계약 한 줄을 함께 갱신했습니다. 검체는 실재하는 `.jsonl` 픽스처로 `redact=false` 에서 전사 ≥ 1 ·
`redact=true` 에서 `[]` 를 실제로 재고, 가림 줄을 지우면 두 턴이 응답에 누출됨을 변이 대조로
확인했습니다(종전 픽스처는 경로가 실재하지 않아 어느 쪽이든 `[]` 라 판별력이 없었습니다).

### ⓓ `cys doctor --fix` staging 삭제 보호 — 측정 불능은 삭제가 아니라 보호

`diag_staging_residue` 의 L5 가드는 "최근 N초 안에 수정된 staging 은 지우지 않는다"인데, **idle 을
못 재는 경우**(metadata/modified 실패 · **미래 mtime**)를 종전(`d422e0b`)은 "삭제 진행"으로
접었습니다. 비가역 `remove_dir_all` 앞의 차단 조건이 측정 불능에서 열리는 구조입니다. 설계 결정을
뒤집었습니다.

| 측정 결과 | 종전 | 이 판 |
|---|---|---|
| idle < 보호창 | 보호(진행중) | 보호(진행중) |
| idle ≥ 보호창 | 삭제 | 삭제 |
| **측정 불능**(None) | **삭제** | **보호** — `unmeasurable` 로 따로 셉니다 |

doctor 출력이 둘을 구별합니다 — `"N건 진행중 보호"` 와 **`"N건 측정불능 보호(mtime 미상·미래)"`**
(status Warn · 종료 코드 불변). 운영자는 "아직 쓰는 중"과 "잴 수 없음"을 출력에서 가릅니다.

- 보호창은 `CYS_DOCTOR_STAGING_MIN_IDLE_SECS`(기본 **60**초)입니다. **`0` 은 보호 off 로, 진행중 보호와
  측정불능 보호를 함께 해제해 종전대로 항상 삭제합니다 — 탈출구입니다.** 측정 불능이 계속 남아
  치워야 할 때 쓰십시오 — doctor 출력의 측정불능 조각에도 그 길이 병기됩니다
  (`… 측정불능 보호(mtime 미상·미래 — 영구면 CYS_DOCTOR_STAGING_MIN_IDLE_SECS=0 으로 보호 해제 후 --fix)`).
- 무효 값(`off` · `-1` · 빈 값 — 비음수 정수만 유효)은 **stderr 경고 1줄 + 기본 60(보호 on)** 입니다.
  탈출구가 이 env 하나뿐이라, 무효 값이 조용히 60 으로 떨어지면 탈출구가 막힌 채 침묵하기 때문입니다.
  실패 방향: 못 읽으면 보호 on 쪽 — 삭제로 미끄러지지 않고, 침묵하지도 않습니다.
- 실패 방향: 못 재면 **보호(skip)** — 잔재는 다음 라운드 또는 보호 off 로 넘깁니다. 운영 비용은
  닫혀 있습니다(pack-update 는 전개 직전 `.pack-staging` 을 스스로 비우고, init-pack 은 pid 접미
  이름이라 후속 명령 차단 0 · 디스크 ≤ 팩 1벌 × 잔재 수 · doctor WARN 1행).
- 기각한 안: "2단 임계"(측정 불능이면 더 긴 창으로 삭제) — 못 잰 값으로 파괴를 정당화하는 구조라
  택하지 않았습니다. 격리(`remove` → `cys-trash` 로 `rename`) 전환은 별도 후보로 이월했습니다.

---

## 3. `cys pause` — 사유 결측을 결측으로, 누가 걸었는지를 함께

**무엇이 문제였나.** `system.pause` 의 빈/공백 사유가 `"reason": ""` 으로 디스크에 영속됐고, 화면은
그것을 사유처럼 그렸습니다. 누가 걸었는지는 어디에도 남지 않았습니다.

- 데몬 `pause_info` 가 `(f64, String)` 튜플에서 `PauseInfo { since, reason: Option, actor_pid,
  actor_surface, actor_role }` 로 바뀌었습니다. 빈/공백 사유는 값이 아니라 **결측(None)** 입니다.
  설정자는 dispatch 가 이미 쥔 `caller_pid` 와 그 pid 가 속한 좌석·역할입니다(해소 실패 = 정상
  결측 · pid 는 남습니다).
- `cys status` 의 PAUSED 줄:
  `⛔ PAUSED — (사유 미기재) · 설정자 worker(pid 4321) (cys resume로 해제; …)`. 설정자는
  `설정자 <role>(pid N)` → `설정자 pid N` → `설정자 미상` 순으로 내려갑니다. `cys gate-check` 도
  `PAUSED (reason: (사유 미기재))` 로 그립니다. `--json` 경로는 건드리지 않았습니다.
- **`autopilot.json` 포맷**: 결측 키는 **생략**합니다(`skip_serializing_if`). 부팅 복원은 **구 포맷의
  `"reason": ""` 을 None 으로 승격**합니다 — 마이그레이션 없이 구 파일을 그대로 읽습니다.
  `org.status` 의 `pause_info` · `system.gate_check` · `autopilot.paused` 이벤트(사유 결측 = `null`)가
  같은 포맷입니다.
- 순서 수리(성찰 ⑨): 종전에는 `set_paused(true)` **뒤에** 설정자를 해소해 `paused=true ·
  pause_info=None` 창이 넓었습니다 — 사유 있는 pause 가 그 창에서 "미기재"로 보였습니다. 이제
  설정자 해소·`PauseInfo` 완성이 플래그 전환 **앞**입니다.
- 실패 방향: 사유·설정자가 결측이어도 **동결은 유지**됩니다 — kill-switch 는 사유 부재로 풀리지
  않습니다. `cys pause` 수락 계약과 `cys resume` 의미는 불변입니다. **빈 사유를 거부하지 않은
  이유**: 복구 하네스가 kill-switch 를 못 걸게 됩니다.
- `--reason` 필수화(CLI 계약 변경)는 **미착수** — phoenix 하네스 2곳 동반 수정이 필요해 별도 결정
  항목입니다(#32).

---

## 4. 부서 편성 심박 — 실패가 보입니다

**무엇이 문제였나.** 데몬 내장 잡 `formation-heartbeat`(10분 주기)는 부서마다
`javis_formation.py ensure … --json || true` 를 돌렸습니다. `|| true` 때문에 **어느 부서가
실패해도 잡은 항상 성공**이었고, 마지막 부서 결과만 남았습니다. 자동 복구 경로의 유일한 관측
수단이 침묵하고 있었습니다.

- 집행 전 안전 검증: `schedule.rs` 주기 발화는 `interval_due → last_fired 기록 → spawn(fire)` 이고,
  `fire_command` 의 Err 는 호출자가 `schedule.error` 를 발행할 뿐 **잡 비활성화·백오프·실패 카운터가
  없습니다**. 즉 exit 비0 을 표면화해도 다음 10분 주기를 막지 않습니다. 이것을 코드로 확인한 뒤에
  집행했습니다(자가치유 전멸 방지).
- ⓐ 명령 문자열: `… --json || true; done` → **`rc=0; … --json || rc=1; done; exit $rc`**. 한 부서
  실패가 마지막 부서 결과에 가려지지 않고, 실패한 부서 **뒤의 부서도 계속** 돕니다. `ensure` 는
  failed 일 때만 1 이라 평시 소음이 없습니다(검체가 팩 exit 규약을 함께 핀). `cys-dept` 호출의
  `2>/dev/null` 은 유지했습니다 — 풀면 정상 잡음이 `schedule.error` 로 승격됩니다.
- ⓑ 기존 설치본에 닿는 법: `BUILTIN_COMMAND_MIGRATIONS` 맨 뒤에 (`formation-heartbeat`, 종전
  문자열 바이트 그대로) 를 **append** 했습니다. `BUILTIN_JOBS_VERSION = 2` 는 그대로입니다 —
  버전을 올리면 운영자가 손으로 고친 예약이 무언 소실됩니다.
- ⓒ 상태파일에 **심박 도달 사실**만 남깁니다: `last_tick` · `last_tick_epoch` · `last_tick_note`.
  판정 키는 불변이고, 상태파일이 없으면 만들지 않으며, 정상 경로에는 이 키를 넣지 않습니다 —
  호출은 싱글플라이트 락 실패(`partial:inflight`) 와 `ensure` 최후 예외, 두 곳뿐입니다.
  "`last_tick` 만 움직인다" 는 곧 "심박은 왔는데 새 판정은 기록되지 않았다" 입니다.
  실패 방향: 기록 실패·잠금 경합은 조용히 무시(fail-open) — 관측 보강이 복구 경로를 막으면 안 됩니다.
- 워커(codex)가 찾은 경쟁: `os.replace` 단독으로는 읽기→교체 사이에 정상 writer 가 쓴 최신
  `complete` 를 낡은 `partial:booting` 으로 되돌릴 수 있음을 재현 → 두 기록 경로에 **공통 쓰기
  잠금 + 원자 교체** 헬퍼 + 고유 임시파일명.
- 성찰 ⑧: 그 쓰기 잠금이 무기한 대기(POSIX 무기한 vs Windows ≈10초로 갈림)라 싱글플라이트를 쥔 채
  멈출 수 있었습니다 → **50ms 폴링 × 2.0초 상한**, 초과 시 loud WARN 1줄 + 그 판정 1회 미영속
  (다음 심박이 재기록). 실패 방향: 상한 초과 = 침묵 드롭이 아니라 무엇을 버렸는지까지 stderr 에
  남기고 반환.
- 미착수: cys-dept 미등재 부서 고지(ⓓ · #33).

---

## 5. 자원 게이트 부트 유예 — mtime 이 아니라 내용을 근거로

**무엇이 문제였나.** 부팅 직후 300초 CPU 유예의 유일한 근거가 `boot-epoch` 파일의 **mtime** 이었습니다.
백업 복원·타임스탬프 보존 복사로 mtime 만 현재로 밀리면 **없는 부팅에 유예가 섰습니다**.

`_boot_elapsed` 의 근거 우선순위가 바뀌었습니다.

| 순위 | 근거 | `boot_grace_reason` |
|---|---|---|
| ① | 이미 받아 둔 `cys status --json` 응답의 `daemon.started_at`(새 왕복 0) | `daemon_started_at` |
| ② | `boot-epoch` **내용**(u64 nonce)의 세대 대조 — 게이트 상태 `<CYS_STATE_DIR>/resource-gate/boot-nonce-<레인키>.json {boot_nonce, first_seen}` · 새 세대의 `first_seen = min(now, mtime)` | `nonce` |
| ③ | mtime — ② 가 불능(내용이 nonce 가 아님 · 상태 I/O 불능)일 때만 · **폴백 사용 사실을 표기** | `mtime_fallback` |

- 근거 부재(`epoch_missing` · `epoch_unreadable` · `clock_backwards`)는 종전대로 **유예 없음**
  (fail-closed). `BOOT_GRACE_REASONS` 는 닫힌 집합 7종(`override` 포함)이고 사용자 출력에
  `근거 <reason>` 이 병기됩니다. `"ok"` reason 은 소비처가 0 이라 폐지했습니다.
- 실패 방향: ① 이 부재·비숫자·미래 → 유예 없음. ② 는 같은 nonce 면 mtime 이 밀려도 유예가 재개되지
  않음. ③ 은 방향이 종전과 같되 **표기가 계약**입니다(조용한 완화 금지).
- **한계(설계상 잔존 · docstring 명시)**: ② 에서 세대의 첫 관측이 늦으면 유예가 그만큼 길어집니다 —
  fail-open 성분이며 세대당 최대 1회입니다.
- 지시서 반박(기록): "이 파일은 이미 status 를 부른다"는 부서 소켓 glob 에만 참이라, base 레인은
  ① 이 구조적으로 불가 → base 는 ② 가 실효 1차입니다. mtime 을 전면 제거하지 않은 이유: TTL·나이
  축은 mtime 이 옳은 축입니다. WP6-6 (2)~(5)(mtime 을 내용의 대리값으로 쓰는 나머지 자리)는
  미착수입니다(#34).

---

## 이번 판에서 새로 생긴 축

이 문서 범위의 9개 커밋에는 **새 CLI 명령이 없습니다.** 새로 생긴 것은 환경변수 2종과 기존 명령·
파일·RPC 의 축입니다. 구 팩·구 바이너리에서는 축이 없으므로 존재를 먼저 확인하십시오.

| 어디 | 새 축 | 무엇을 하나 |
|---|---|---|
| env | `CYS_CTX_DIVERGENCE_PCT`(기본 8) | 실측·자기보고 괴리 임계 — 5분 보고 게이트와 `ctx-compare` 프로브가 읽음(무효 값 처리는 §1-ⓕ) |
| env | `CYS_DOCTOR_STAGING_MIN_IDLE_SECS`(기본 60) | `doctor --fix` staging 보호창 — `0` 은 측정불능 보호까지 해제하는 탈출구 |
| `cys status` · `cys fleet` | CTX 칸 `78%` / `78%~` / `?` / `-` | 실측 · 자기보고(추정) · 판정 불가 · 없음 |
| `cys status` · `cys gate-check` | `(사유 미기재) · 설정자 …` | pause 사유 결측·설정자 표시 |
| `cys doctor --fix` 출력 | `N건 측정불능 보호(mtime 미상·미래)` | 진행중 보호와 별도 계수 |
| RPC `control.session_detail` | 응답 `redacted: bool` | 가림 적용 사실 에코(`control.sessions` 와 같은 모양) |
| RPC `control.sessions` / `session_detail` | `redact` 비-bool → `invalid_params` | 타입 혼동 거절 |
| RPC `org.status` · `system.gate_check` · 이벤트 `autopilot.paused` · `autopilot.json` | `pause_info{since, reason?, actor_pid?, actor_surface?, actor_role?}` | 결측 키 생략 · 구 포맷 `""` 승격 |
| `org.status` `surfaces[]` | `exited` · `agent_alive` 를 CTX 판정에 소비 | 사망 좌석 `?` |
| 5분 보고 `live_nodes[]` | `usage_ctx_pct` 추가 | 실측 축(기존 키 삭제 0) |
| 5분 보고 대장 `reasons` | `ctx_divergence_env_invalid` · `report_module_missing` | env 무효 · CTX 헬퍼 부재의 가청화 |
| 게이트 `last_measurement.json` | `ctx_divergence[]` · `ctx_divergence_stats{}` | 괴리 기록·계수 |
| `javis_actprobe.py ctx-compare` | 기본 임계 8 · 영수증 `diff/measured/reported/usage_source` · 무효 임계 exit 3 `threshold_env_invalid` | 착수 전 대조 |
| 편성 상태파일 | `last_tick` · `last_tick_epoch` · `last_tick_note` | 심박 도달 사실(inflight·예외 경로만) |
| 데몬 내장 잡 `formation-heartbeat` | `exit $rc`(부서별 rc 누적) · 이관표 1행 | 실패 부서 표면화 |
| 자원 게이트 `measured` | `boot_grace_reason`(닫힌 집합 7종) | 부트 유예 근거 표기 |
| CI | 팩 검체 4종을 이름 루프 5곳에 등재 | `test_report_ctx_axis` · `test_formation_tick_visibility` · `test_resource_gate_boot_anchor` · `test_ctx_divergence` |

---

## 업그레이드하실 때 알아두실 것

### 경보가 줄어 보이면 — 먼저 CTX 칸을 보십시오

이 판을 올린 뒤 60% 컨텍스트 경보가 줄면, 그것은 **낡은 자기보고가 더 이상 경보의 입력이 되지
않기 때문**입니다(§1-ⓒ). `cys status` 의 CTX 칸이 `?` 인 좌석은 판정에서 빠진 것이지 0% 가 아닙니다.
실측이 없는 어댑터(agy·gemini)의 좌석은 신고가 신선할 때만 `~` 로 목록에 듭니다.

### 팩 하한 — 0.14.31 그대로

두 발행 레인의 값은 바꾸지 않았습니다(실측 · 통합 브랜치 `fix/0.14.31-audit` · 행번호는 이 트리 값):

| 레인 | 키 | 위치 | 값 |
|---|---|---|---|
| `release.yml` | `PACK_MIN_BINARY` | `:1065` | `0.14.31` |
| `pack-release.yml` | `PACK_MIN_BINARY_OVERRIDE` | `:79` | `0.14.31` |

이 문서 범위의 팩 변경(보고기·게이트·프로브·편성·자원 게이트)은 새 CLI 표면에 의존하지 않습니다 —
`cys status --json` 의 `usage.ctx_pct` · `exited` · `agent_alive` · `daemon.started_at` 은 구 데몬에
없어도 **폴백**(자기보고 · 게이트 안 염 · nonce/mtime)으로 무너지도록 만들었습니다. 하한을 올릴
근거가 없어 올리지 않았습니다. `v0.14.33..fdd45f3` 구간(이 문서 범위 밖)이 하한 상향을 요구하는지는
여기서 판정하지 않았습니다.

### 버전

버전 문자열은 게이트가 강제하는 8곳 전부 **0.14.36** 입니다(범프 커밋 `fdd45f3` · 수동 6곳 +
Cargo.lock 2패키지). 이 문서 작성 시점 재확인(`sh scripts/version-check.sh v0.14.36` · rc=0):

```
버전 SOT 8곳:
  Cargo.toml                     0.14.36
  src-tauri/Cargo.toml           0.14.36
  src-tauri/tauri.conf.json      0.14.36
  ui/package.json                0.14.36
  dist-win/cys.wxs               0.14.36
  dist-win/cys-x64.wxs           0.14.36
  Cargo.lock [cys-terminal]      0.14.36
  Cargo.lock [cys-app]           0.14.36

✅ 8곳 일치: 0.14.36
✅ 기대 버전 일치: 0.14.36
```

### 검사 실측(커밋에 기록된 값 · 통합 트리 `d979ac4`)

- `cargo test --bin cysd --skip hwmon::` → **1293 / 0**
- `cargo test --bin cys` → **276 / 0**
- `cd ui && bun test` → **772 pass 0 fail** · `bunx tsc -p tsconfig.check.json` → 오류 0
- 팩 검체 17종(신규 4 + 형제·게이트 13) → 전건 rc 0 · `bash scripts/secret-scan.sh --all` → clean
- CI 등재 검증: `ruby -ryaml YAML.load_file` 3파일 ok · 열거 자리 5곳 전부 4종 포함 · CI 형태
  (`CYS_PACK_DIR=$(mktemp -d)` + 격리 HOME/CYS_STATE_DIR)로 4종 rc 0
- 변이 대조(되돌리면 붉어짐 실측): 데드맨 `unwrap_or(false)` · redact `Some(_) => false` · `redacted`
  에코 제거 · staging `d422e0b` 원표현 · session_detail 가림 줄 제거 — 전부 해당 검체 FAILED.

**돌리지 않은 것**: GitHub Actions 실행(push 는 릴리스 단계) · 실좌석 5분 주기 경보량 · 실좌석
심박 틱·부트 유예 관측(데몬 실행 금지 회차) · Windows 러너 실기 · 앱 번들 빌드 · GUI 시각 확인.

---

## 정직하게 — 이번 판의 한계

- **데드맨 킬 창.** §2-ⓐ 의 [단발 2초 프로브 실패] × [미래 mtime] 창에서는 산 홀더가 회수될 수
  있습니다. 미래 mtime 은 heartbeat 신선도 증거를 통째로 버리므로(1초 전에 touch 했어도 stale)
  그 창에서는 45초 임계가 보호하지 못하고 프로브 하나만 남습니다. 완화 두 가지는 다음 라운드입니다.
- **관측 전에 죽은 좌석은 잡히지 않습니다.** CTX `?` 의 사망 게이트는 페이로드의 `exited`/`agent_alive`
  만 봅니다. 에이전트가 한 번도 관측되지 않은 채(`agent_alive=null`) 죽은 좌석과, 사망 뒤 워치독
  틱이 돌기 전의 창은 동결된 실측이 값으로 보일 수 있습니다 — `null` 은 "모른다"라 게이트를 열지
  않는 것이 의도입니다.
- **실측 축에 나이 게이트가 없습니다(결정).** 등록 매핑·statusline 값은 나이로 지워지지 않으므로,
  좌석이 살아 있되 실측 갱신이 멈춘 형상(데몬 usage 수집기 쪽 문제)은 이 판의 CTX 칸이 잡지
  못합니다. 데몬 측(`usage.rs` · WP6-7)은 오너 결재 선행 항목이라 손대지 않았습니다(#35).
- **괴리 경보량은 실환경에서 재지 않았습니다.** 임계 8 은 인수인계 기록의 최소 사고(9pt)를 잡도록
  고른 값이고, 첫날 조정 여지가 있어 env 로 뺐습니다. 60% 블록의 Infinity 서식 예외는 별건으로
  미수정입니다.
- **부트 유예 ② 의 fail-open 성분.** 세대의 첫 관측이 늦으면 유예가 그만큼 길어집니다(세대당 1회).
- **편성 상태파일 쓰기 잠금 상한 초과 시 판정 1회가 영속되지 않습니다**(WARN 1줄 · 다음 심박이
  재기록). Windows `msvcrt` 경로는 문서 근거로만 확인했습니다(실기 없음).
- **`star_note` 는 가림 밖입니다**(#31). **tauri `control_session_detail` 은 `redact` 를 데몬에
  전달하지 않습니다** — GUI 상세는 `CYS_CONTROL_REDACT=1` 이 아니면 미가림입니다(#30). 둘 다 오너
  결정 항목입니다.
- **미추적 파일 `cysjavis-pack/state/probe_runs.jsonl`** 이 트리에 남아 있습니다(프로브 영수증 ·
  보존·미커밋 · 오너 판단).
- **이 문서의 범위 밖 구간**(`v0.14.33..fdd45f3` 13커밋)은 정리되지 않았습니다(머리말).
- **이전 판(v0.14.33)에 적어 둔 한계는 이번 회차가 다루지 않은 항목에 한해 그대로 유효합니다.**

---

## 미착수·이월 — 백로그 #30 ~ #37

저장소 밖 PREP.md(오너 작업 디렉터리 · 커밋 대상 아님)에 등재한 항목의 요약입니다. #27~#29(데드맨 ·
staging 보호 · redact)는 이 판에 착지했습니다.

| # | 무엇 | 자리 | 상태 |
|---|---|---|---|
| 30 | tauri `control_session_detail` 이 `redact` 를 데몬에 전달하지 않음(GUI 상세는 env 아니면 미가림) | `src-tauri/src/main.rs` | P2 · 오너 결정(배선 보강 여부) |
| 31 | `control.sessions` 의 `star_note` 가 `redact=true` 에서도 raw | `src/bin/cysd/analytics.rs` `redact_sessions` | P2 · 오너 결정(PII 가능) |
| 32 | `cys pause --reason` 필수화(CLI 계약 변경 · phoenix 하네스 2곳 동반) | `src/bin/cys.rs` · `javis_phoenix_harness.py` | P2 · 오너 결정 |
| 33 | 심박 레인에서 cys-dept 미등재 부서 고지가 버려짐(`cys feed` 1회 고지 필요) | `cysjavis-pack/bin/cys-dept` `dept_warn_unregistered_socks` | P2 · 미착수 |
| 34 | mtime 을 내용의 대리값으로 쓰는 잔여 자리(actprobe `--since` · javis_task E1 ③ · state_ledger 진단) | `javis_actprobe.py` · `javis_task.py` · `javis_state_ledger.py` | P2 · 미착수(E1 ③ 은 baseline sha 자리 설계 선행) |
| 35 | codex 세션 매핑 재결속(lsof 첫 매치 선택 → 세션 교체 후 공회전 가설) | `src/bin/cysd/usage.rs` | P2 · **오너 결재 선행**(착수 금지) |
| 36 | 빌드 기계의 `/usr/bin/cc`·git 이 Xcode 라이선스 미동의로 사망(툴체인 직접 지정으로 우회 중) | 환경 | P1 · 오너 손 조치 |
| 37 | codex 사용자 설정이 삭제된 로컬 모델 서버를 `base_url` 로 가리킴 — 대화형 codex 좌석 연결 거부 가능(헤드리스는 `--ignore-user-config` 로 우회) | 사용자 홈의 codex 설정 | P1 · 오너 확인 |

그 밖에 이 회차에서 **기각**한 안(기록): 실측 축 300초 나이 게이트(유휴 좌석 거짓 `?`) ·
`Bun.file` 소스 핀(tsc 선언 부재) · 데몬 `idle_stale_transition` 수정(WP6-7 결재 항목) · 전역
`BUILTIN_JOBS_VERSION` 범프 · `|| true` 단순 삭제 · 팔레트/토스트 문구 추정 표식 · 낡은 자기보고
막대 숨김 · CTX SRC 열 신설(외부 파서 파괴 위험 → `~` 표식) · redact null 거절(UI 정상 호출 파괴).

---

## 다음에 하는 일 — 재측정 계획

- **설치 후 하루**: 5분 보고 대장의 `ctx_divergence_stats` 를 읽어 `compared` 가 0 이 아닌지(0 이면
  비교 자체가 안 된 것 — `skipped_*` 로 어느 갈래인지), 괴리 경보 건수와 부호 분포. 임계 8 조정
  여부는 이 값으로 정합니다.
- **60% 경보 감소분의 내역**: 줄어든 경보가 전부 "낡은 자기보고" 갈래였는지 `reasons` 와 CTX 칸
  `?` 로 대조합니다. 실측이 있는 좌석에서 경보가 빠졌다면 그것은 의도한 감소가 아닙니다.
- **편성 심박**: `schedule.error` 발생 건과 상태파일 `last_tick_note` 를 대조해 실패 부서가 실제로
  드러나는지, 다음 10분 주기가 계속 도는지.
- **부트 유예**: 실좌석 재기동에서 `boot_grace_reason` 이 `daemon_started_at` 또는 `nonce` 로 찍히는지 —
  `mtime_fallback` 이 상시라면 ② 경로가 죽어 있는 것입니다.

**측정하지 못하면 통과가 아닙니다.** 위 항목 중 측정에 실패한 것은 "미측정"으로 보고하고 통과로
세지 않습니다.

---

## Windows Defender / SmartScreen 안내 (유지)

Windows용 설치 파일은 Authenticode 서명이 없어(인증서 미보유) 실행 시 SmartScreen
"알 수 없는 게시자" 경고가 뜰 수 있습니다(**추가 정보 → 실행**으로 진행).
홈페이지의 `SHA256SUMS.txt` 와 대조해 무결성을 확인할 수 있습니다.
Defender 오탐은 WDSI 신고로 낮춰 갑니다(`docs/WDSI_SUBMISSION.md`).

### Defender가 `cys.exe` 를 격리한 경우 — 복구는 **순서**가 생명입니다

```powershell
# ① 제외 먼저 (복원 즉시 재격리 방지)
Add-MpPreference -ExclusionPath "$env:LOCALAPPDATA\cys"

# ② 그 다음 복원
& "$env:ProgramFiles\Windows Defender\MpCmdRun.exe" -Restore `
    -Name "Program:Win32/Contebrew.A!ml" -All
```

순서를 바꾸면 복원하자마자 다시 격리됩니다.

## Windows 설치 전 — `claude` 는 따로 설치해야 합니다

```powershell
irm https://claude.ai/install.ps1 | iex
```

자세한 절차는 `docs/INSTALL-Windows-KR.md` 를 보시면 됩니다.

## macOS 설치

`.dmg` 를 열고 **"Install cys"** 를 더블클릭하면 설치가 원자적으로 끝납니다.

- Apple Silicon: `cys_0.14.36_aarch64.dmg`
- Intel: `cys_0.14.36_x64.dmg`
