# 컨텍스트 사이클 전자동화(fullauto-cycle) — 운영자 가이드

> v0.14 동봉. builtin 잡 2종(`cycle-autopilot-tick` 매분 · `cycle-verifier-watchdog` 10분)이 데몬 부트 시 **자동 배선**된다 — 단 **shadow 기본**: live 승격(§3-5 파일 채널) 전에는 clear 발화도, 검증자 pane 상주(자동 기동)도 집행하지 않는다(원장에 would_fire 기록뿐).
> 대상 독자: cys 터미널로 멀티에이전트 플릿을 운영하는 관리자.

## 1. 무엇인가

"컨텍스트 60% 도달 시 저장→clear 사이클"과 "주요 이벤트 기계 원장 기록"을 사람·LLM 판단 없이 결정론 코드로 수행하는 외곽 자동화다. `cys cycle-agent`(5단계 집행기)와 데몬은 수정하지 않는다 — 개시·검증·사후검증을 코드가 담당한다.

| 구성물 | 역할 |
|---|---|
| `bin/javis_cycle_autopilot.py` | 1분 틱 상태기계: 측정(statusline만)→안전 게이트 7종→선통보→cycle-agent 실행→사후검증(토큰 급락+nonce)→원장 기록. 실패=clear 미실행(fail-closed) |
| `bin/javis_cycle_verifier.py` | 전용 pane 상주 결정론 검증자: 사이클 직전 baseline과 전 파일 대조·유휴 재확인 후 feed reply. 모호=deny |
| `bin/javis_state_ledger.py` | 기계 원장(`STATE_LEDGER.jsonl`): 커밋·task done·handoff·사이클을 훅이 자동 기록. O_APPEND+flock 동시성 계약 |
| `hooks/fullauto/*.sh` 4종 | PostToolUse 오버레이(이벤트 감지)·SessionStart(원장 요약 주입)·Stop(staleness 기록)·UserPromptSubmit(오너 존재 신호) — **템플릿**(여기 있는 채로는 발동하지 않음) |

## 2. 안전 불변식 (설계 계약)

- 자동 clear의 유일 경로는 `cys cycle-agent --verifier`(2-phase handshake) — self-clear 코드 차단 불변.
- kill-switch 4중: ①`cys pause`(스케줄 동결) ②`cys gate-check` ③`$CYS_PACK_DIR/AUTOPILOT_PAUSED` 또는 `<프로젝트>/_round/AUTOPILOT_PAUSED` 파일(하나라도 존재=무집행) ④집행 중 1~5s 폴링·감지 시 SIGTERM.
- 검증자는 반드시 **별도 pane 포그라운드**로 상주(`bootstrap-verifier`가 생성). detached·데몬 스폰은 데몬의 self-approval 게이트가 범주적으로 거부한다. 맨 셸·LLM pane 금지.
- 측정은 statusline 서버 진실만 판정 투입(claude 노드 한정). 자기보고·transcript 추정은 판정 금지.
- **실패**(`failed`·`failed_preclear`·`indeterminate`) 종결 후 재발화는 운영자 `reset` + 쿨다운(180s)으로만. **비파괴 보류**(`held_noop` · cycle-agent exit 84/85)는 실패가 아니라 자동 재시도 대상이다 — §4-b 표. `--force-no-verify`는 어떤 경로로도 사용되지 않는다.
- 즉시 전체 무력화: 환경변수 `CYS_STATE_LEDGER_DISABLE=1`(원장 훅) + 스케줄 잡 제거 또는 `cys pause`.

## 3. 활성화 절차 (단계적 — 건너뛰지 말 것)

1. **배치**: `bin/` 3종은 팩 경로 `${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/`의 동봉본을 그대로 쓴다 — builtin 잡·워치독이 이 경로를 호출하므로 **팩 밖 사본(`~/.cys/local/bin` 등) 운용 금지**(사본 드리프트 = 잡과 수동 절차가 서로 다른 코드를 돈다). 원장 훅 `hooks/fullauto/50-state-ledger.sh`를 `$HOME/.cys/local/hooks/PostToolUse.d/`에 복사하면 이벤트 기록이 즉시 살아난다(무해·기록만).
2. **훅 등록**: `state-ledger-inject.sh`(SessionStart)·`state-staleness.sh`(Stop)·`owner-active.sh`(UserPromptSubmit)를 `$HOME/.cys/local/hooks/`에 복사 후 각 프로필 settings.json 해당 이벤트 배열 **끝에** `{"hooks":[{"type":"command","command":"sh <경로>"}]}` 블록으로 가산(백업 필수). 새 세션부터 유효.
3. **검증자 기동(S0 관측용 수동)**: `python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_cycle_autopilot.py" bootstrap-verifier`. S0 shadow 에서 would_fire 를 보려면 검증자 heartbeat 게이트(게이트6) 때문에 이 **수동 기동**(운영자 명시)이 필요하다. live 승격 후에는 워치독 잡(10분 주기 `--ensure`)이 자동 유지·재기동한다 — shadow 에서 `--ensure` 는 무집행 shadow-noop(pane 생성 0)이 계약이다.
4. **S0 shadow 관측**: 스케줄 잡은 **등록하지 않는다** — builtin 잡 2종(`cycle-autopilot-tick`·`cycle-verifier-watchdog`)이 데몬 부트 시 자동 upsert 된다. 같은 id 를 손으로 등록하면 사용자 선점으로 오인돼 conflict 경고만 만든다(schedule.rs apply_builtin_jobs). 기본 shadow 모드로 원장(`cycle_autopilot_log.jsonl`)에 would_fire만 쌓인다. `audit` 서브커맨드로 오탐 0을 확인하라. PAUSED 파일 생성→다음 틱 skip 확인(음성대조)도 필수.
5. **S1 live 승격**: 잡 문자열이 아니라 **STATE_DIR 파일 채널**로 승격한다(잡 command 의 env 접두는 builtin 버전 범프 때 코드 정의로 통째 교체돼 live 가 shadow 로 무언 회귀한다):
   ```sh
   mkdir -p ~/.local/state/cys/cycle_autopilot
   printf 'live' > ~/.local/state/cys/cycle_autopilot/mode
   printf 'worker' > ~/.local/state/cys/cycle_autopilot/roles   # 대상 역할(콤마 구분)
   ```
   강등 = mode 파일 삭제. **Windows 주의**: PowerShell 의 `>`·`Set-Content` 기본 인코딩은 UTF-16(BOM)이다 — `Set-Content -Path $env:USERPROFILE\.local\state\cys\cycle_autopilot\mode -Value live -Encoding ascii -NoNewline` 으로 쓴다(코드에 utf-16 재시도 내성이 있으나 ascii 가 정본). claude 워커 노드 한정(codex·agy는 측정원이 달라 대상 아님).
6. **master 확대(S2)**: 워커 사이클 성공 이력 확보 후에만 roles 파일에 `worker,master` 로 추가.

## 4. 관측·트러블슈팅

- 원장: `<프로젝트>/_round/cycle_autopilot_log.jsonl`(사이클 phase 전이 전부)·`STATE_LEDGER.jsonl`(이벤트). `status`·`audit` 서브커맨드.
- 사이클이 안 돈다 → 원장의 skip reason이 사실이다(측정 source·유휴·쿨다운·짝짓기·검증자 heartbeat). 직전 종결이 failed면 `reset --role <r> --reason "<사유>"` 후 180s. 직전 종결이 `held_noop` 이면 reset 하지 마라 — 아래 §4-b 의 쿨다운이 지나면 스스로 다시 발화한다(구조적 보류 상한 도달만 예외).

### 4-b. cycle-agent 종료코드 84·85·86 과 `held_noop` (v0.14.39 · WP-D)

| exit | 뜻 | clear 송신 | autopilot 종결 | 운영자 조치 |
|---|---|---|---|---|
| 84 | 대상이 `--timeout` 안에 유휴(턴 종료·빈 composer)가 되지 않음 | **0건** | `held_noop` | 없음 — 자동 재시도 |
| 85 | 사람 초안·미제출 입력 보호(사전 확인) 또는 데몬 타이핑 가드가 `/clear` 를 거부 | **0건**(타이핑 가드 거부 경로는 `C-u` 1키가 선행할 수 있다) | `held_noop` | 없음 — 자동 재시도 |
| 86 | clear 는 **이미 실효**(session_file 교체 확인)했으나 재주입 직전 대상이 유휴가 안 됨 · RESUME 은 최선노력 송신 | **1건(발효)** | 사후검증(held 아님) | 손으로 다시 clear **금지** — 좌석에 [RESUME] 이 없으면 재주입만 |

- **자동 재시도 규칙(게이트5)**: `held_noop` 뒤 쿨다운은 연속 보류 횟수에 따라 지수 증가 — 1회 300s → 2회 600s → 3회 이상 1200s(성공 사이클 쿨다운과 같은 상한). 대상이 살아서 턴을 도는 한(rc84 비구조 · rc85) **하드 정지 없음**. 다른 게이트(유휴·임계·오너 부재·single-flight·검증자 heartbeat)는 그대로 겹쳐 잡는다.
- **구조적 보류 상한**: rc84 문면에 `[diag=quiet_secs_unreported]` 가 붙으면(데몬이 `quiet_secs` 를 보고하지 않는 구 데몬 · 재시도가 원리적으로 무의미) 그 보류만 세어 연속 3회(`HELD_RETRY_MAX`)에 도달하면 자동 재시도를 멈추고 `autopilot-held-limit` 통지 1회를 낸다. 해제 = 데몬 갱신(`cys daemon restart` 또는 팩 업그레이드) 후 `reset --role <r>`.
- **통지는 보류 연속 구간당 유한**: `autopilot-held` 는 1회째와 `HELD_NOTIFY_EVERY`(=3)의 배수 회(3·6·9…)에서만, `autopilot-held-limit` 는 도달 순간 1회. tick 은 통지하지 않는다(javis_wakeup 멱등키는 배달 뒤 소멸하므로 tick 재통지 = 매분 홍수). ★통지 주기(`HELD_NOTIFY_EVERY`)와 구조적 보류 하드 상한(`HELD_RETRY_MAX`)은 **서로 다른 노브**다 — digest 가 잦아 주기를 늘려도 사람 개입 시점(상한)은 밀리지 않는다. 예외로 종결 뒤 원장 재조회 값이 예측과 어긋나면(경합 · 원장 손상) 주기와 무관하게 `autopilot-held` 1건을 반드시 내고 문면에 `원장 재조회 불일치(예측 N != 원장 M)` 를 싣는다(침묵 금지). 원장 `detail`: `held_streak`·`held_structural_streak`·`structural`·`alive_evidence`·`keys_sent`·`cooldown_secs`·`retry_after_ts`·`residual_window_secs`(검증자 allow→clear 실측 · 자식 stderr 파싱 · 미보고면 null).
- **`keys_sent` 는 어댑터와 무관하게 읽는다**: rc85 타이핑 가드 거부 경로의 `C-u` 1건 선행 여부는 자식 문면의 `C-u 1건은 선행 송신됨`·`[cycle 5/7] 입력 버퍼 정리 + '` 로 판정한다 — `agents.json` 의 `clear_cmd` 가 `/clear` 든 `/new` 든 같게 기록된다(종전에는 `/clear` 좌석에서만 맞았다).
- 84~86 을 **실패로 읽고 손으로 강제 clear 를 치는 것**이 이 장치가 막는 사고다. 원장 `phase` 가 `held_noop` 이면 기다려라.
- 검증자 deny가 잦다 → 대상 턴 종료 리듬 대비 대기창(기본 108s·유휴 하한 5s) 점검.
- 스케줄 `schedule.error`가 매분 뜬다 → 틱 자체 오류(정상 skip은 exit 0)다. 원장과 py_compile 확인.

## 5. 제거(롤백)

settings.json 가산 블록 제거(백업 복원) → `$HOME/.cys/local/hooks/PostToolUse.d/50-state-ledger.sh` 및 훅 3종 삭제 → mode 파일 삭제(shadow 강등) + 정지가 필요하면 PAUSED 파일 또는 `cys pause`(builtin 잡은 데몬 부트 시 재-upsert 되므로 잡 삭제만으로는 정지가 아니다) → 검증자 pane close. 원장 파일은 감사 기록이므로 삭제하지 말고 보관 이동만.
