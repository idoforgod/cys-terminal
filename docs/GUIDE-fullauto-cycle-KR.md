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
- 실패 종결 후 재발화는 운영자 `reset` + 쿨다운(180s)으로만. `--force-no-verify`는 어떤 경로로도 사용되지 않는다.
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
- 사이클이 안 돈다 → 원장의 skip reason이 사실이다(측정 source·유휴·쿨다운·짝짓기·검증자 heartbeat). 직전 종결이 failed면 `reset --role <r> --reason "<사유>"` 후 180s.
- 검증자 deny가 잦다 → 대상 턴 종료 리듬 대비 대기창(기본 108s·유휴 하한 5s) 점검.
- 스케줄 `schedule.error`가 매분 뜬다 → 틱 자체 오류(정상 skip은 exit 0)다. 원장과 py_compile 확인.

## 5. 제거(롤백)

settings.json 가산 블록 제거(백업 복원) → `$HOME/.cys/local/hooks/PostToolUse.d/50-state-ledger.sh` 및 훅 3종 삭제 → mode 파일 삭제(shadow 강등) + 정지가 필요하면 PAUSED 파일 또는 `cys pause`(builtin 잡은 데몬 부트 시 재-upsert 되므로 잡 삭제만으로는 정지가 아니다) → 검증자 pane close. 원장 파일은 감사 기록이므로 삭제하지 말고 보관 이동만.
