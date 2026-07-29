# 증거 수집 런북 — Windows 0.14.4 두 결함

> **짝 문서**: `2026-07-29-win-two-defects-plan.md` (기획서 — 배경·가설·코드 지도)
> **목적**: master surface exit의 **원인 규명(U2)** + 파이썬 import 제보 **판별(U1)** 을 한 패스로 끝낸다.
> **작성 근거**: 플래그는 2026-07-29 `--help` 실측 확인.

## 절대 규칙 (모든 수행자 공통)

1. **기계를 재시작·종료하지 않는다.** 데몬도 끄지 않는다.
2. **읽기 위주.** 상태를 바꾸는 명령(`kill`·`close-surface`·`restore`·`boot`)은 이 런북에 없다. 임의 추가 금지.
3. **증거 확보 전 재현 시도 금지.** 재현은 수집 완료 후 별도 승인.
4. 각 단계의 **출력 전문을 그대로 보존**한다(요약 금지 — 요약은 판정 후에).
5. 판정은 **가설표(§4)로만** 한다. 인상·추론으로 결론 내지 않는다.

---

## A. 오너용 — 직접 실행 (PowerShell)

> **A-0을 가장 먼저.** 화면 잔상(scrollback)은 가장 먼저 사라지는 증거다.

### A-0. 대상 식별
```powershell
cys list
cys status --json > $env:USERPROFILE\Desktop\cys-status.json
```
- `role=master`인 surface 번호를 확인한다. 이하 `<M>`으로 표기.
- **판정 포인트**: 그 항목이 `agent=null` · `agent_alive=null` 이면 **반쪽 좌석 확정**(기획서 §2.5).

### A-1. ★죽기 직전 화면 (최우선)
```powershell
cys read-screen --surface surface:<M> --max-lines 2000 > $env:USERPROFILE\Desktop\master-screen.txt
```
- 여기서 볼 것: ①claude의 마지막 출력 ②오너가 치지 않은 입력이 있는가 ③종료 직전 에러 문구.

### A-2. 이벤트 되감기
```powershell
cys events --after-seq 0 --max-events 5000 2>$null > $env:USERPROFILE\Desktop\cys-events.txt
```
> `--max-events`가 없으면 그냥 `cys events --after-seq 0` 로 실행하고 몇 초 뒤 Ctrl+C.

- 여기서 볼 것: `agent.exited` · `watchdog.duplicates_killed` · `watchdog.proc_count_high` · `health.alert` · `surface.exited` · `role.claim_denied`.

### A-3. 부트 기록
```powershell
type $env:USERPROFILE\.cys\state\boot-last.json > $env:USERPROFILE\Desktop\boot-last.json
```

### A-4. ★U1 판별 (제보건 — 10초)
```powershell
# 설치 폴더 확인
Get-ChildItem "$env:LOCALAPPDATA" -Recurse -Filter cysd.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty DirectoryName
```
그 경로를 `<DIR>`이라 하고:
```powershell
& "<DIR>\runtime\python\python3.exe" "$env:USERPROFILE\.cys\pack\bin\javis_event.py" --help
& "<DIR>\runtime\python\python3.exe" -c "import sys; print(sys.path)"
```
- **`ModuleNotFoundError: javis_scrub`** → **U1 참** (제보 확정 · 티켓A 착수 가능)
- **정상 도움말 출력** → **U1 거짓** (티켓A 폐기 · 제보자 회신)
- 두 번째 명령의 출력에 **팩 `bin` 경로가 있는지**도 함께 기록.

### A-5. claude 자체 로그
```powershell
# 어느 계정 디렉터리를 썼는지 먼저 확인
Get-ChildItem "$env:USERPROFILE\.claude\projects","$env:USERPROFILE\.cys\claude\projects" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending | Select-Object -First 5 FullName,LastWriteTime
```
- 가장 최근 세션 `.jsonl`의 **마지막 20줄**을 보존한다.

### A-6. OS 수준 종료 흔적
```powershell
Get-EventLog -LogName Application -Newest 200 |
  Where-Object { $_.Message -match "claude|node|cys" } |
  Format-List TimeGenerated,EntryType,Source,Message |
  Out-File $env:USERPROFILE\Desktop\oslog.txt
```

### A-7. 환경 확인 (H1 판별용)
```powershell
$env:CYS_AUTOKILL_DUP
$env:CYS_AGENT_AUTORESTART
```

**A-0 ~ A-7 완료 후 데스크탑의 파일 6개를 회수한다.**

---

## B. CSO용 — 붙여넣기 지시서

> 아래 블록을 **그 기계의 CSO pane에 그대로 붙여넣는다.**

```
[오너 지시 · 증거 수집 티켓 — CSO 전담]

■ 임무
부트스트랩 중 종료된 master surface의 exit 원인 증거를 수집한다. 원인 판정은 하지 말고
"수집과 사실 기술"만 한다. 추론·처방 금지.

■ 절대 제약 (위반 시 즉시 중단·보고)
- 기계·데몬을 재시작하지 않는다.
- 상태 변경 명령 금지: kill / close-surface / restore / boot / launch-agent / cycle-agent.
- 재현 시도 금지(별도 승인 사항).
- 살아있는 노드(워커·리뷰어)에 지시를 전파하지 않는다. 이 티켓은 너 혼자 수행한다.

■ 수집 항목 (순서대로 · 각 출력 전문 보존)
1) cys list
2) cys status --json          → master의 role/agent/agent_alive/seat 값을 그대로 인용
3) cys read-screen --surface surface:<master> --max-lines 2000
4) cys events --after-seq 0   → 다음 이벤트 유무만 사실 기술:
   agent.exited / surface.exited / watchdog.duplicates_killed /
   watchdog.proc_count_high / health.alert / role.claim_denied
5) ~/.cys/state/boot-last.json 전문
6) 환경변수 CYS_AUTOKILL_DUP, CYS_AGENT_AUTORESTART 값

■ 보고 형식 (이 형식 그대로)
- 수집 항목별: [항목] / [명령] / [출력 요약 3줄 이내] / [원문 보존 위치]
- 마지막에 "관측되지 않은 것" 목록을 명시한다(없음을 없음으로 보고 — 침묵 금지).
- 판정·원인 추정은 쓰지 않는다. master가 판정한다.

■ 완료 신호
cys set-status --state waiting --context <실제%> --task "증거수집 완료·판정 대기"
```

---

## C. 판정표 — 수집 후 master가 사용

| 가설 | **확정** 증거 | **기각** 증거 |
|---|---|---|
| **H1** watchdog 중복 kill | `watchdog.duplicates_killed`에 master 계열 pid | 이벤트 없음 **또는** `CYS_AUTOKILL_DUP` 미설정 |
| **H2** 자원 게이트 차단 | `boot-last.json`에 resource_gate 차단 판정 | 통과 기록 |
| **H3** claude 자체 종료 | claude 세션 로그 말미에 종료 사유·에러 | 로그에 흔적 없음 |
| **H4** 외부 텍스트 주입 | scrollback에 오너 미입력 텍스트가 제출된 흔적 | 없음 |
| **H5** OS 크래시/OOM | OS 이벤트 로그 항목 | 없음 |

**보조 판정(구조 확인)**
- `agent.exited` **부재** + `agent=null` → **반쪽 좌석 확정**. 이 경우 "데몬은 죽음을 감지조차 못 했다"가 사실로 확정되며, **H1~H5 모두 이벤트 근거가 남지 않았을 수 있음**을 전제로 해석해야 한다(로그 부재 ≠ 사건 부재).

---

## D. 수집 직후 조치 (증거 회수가 끝난 뒤에만)

1. **오너**: `▶CEO` 버튼으로 master 복구.
   - 근거: `launch-agent`가 `takeover_empty_seat: true`를 보내고 데몬이 좌석을 재판정해 죽은 좌석의 role을 회수한다.
   - 효과: 정식 좌석이 되어 **다음 사고부터 `agent.exited` 증거가 남는다**.
2. U1 결과에 따라:
   - **참** → 기획서 §6.2 티켓A(워커) 착수 승인 요청
   - **거짓** → 티켓A 폐기 + 제보자 회신

---

## E. 이 런북이 하지 않는 것 (명시)

- 원인 **판정**을 하지 않는다(수집만). 판정은 master, 승인은 오너.
- 코드를 고치지 않는다.
- `._pth` 파일을 열지 않는다(내용 동결 대상).
- 살아있는 노드의 상태를 바꾸지 않는다.
