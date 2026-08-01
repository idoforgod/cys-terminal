# Windows 업그레이드 원자성 — Parallels 실기 체크리스트

> 2026-08-01 윈도우 실사고(0.14.8→0.14.9 업그레이드 후 `cys.exe`·`cysd.exe` 동시 소실, `.prev.exe`만
> 잔존 → 사용자가 정지 명령조차 실행 불가) 의 근본 수리(`src-tauri/nsis-hooks.nsh`)를 **실기에서**
> 확인하기 위한 절차. mac 개발기에서는 윈도우 설치기를 실행할 수 없어 코드 판독·모델 검증까지만
> 마쳤다 — 아래는 그 **미검증분**을 사람이 1회 클릭해 닫는 목록이다.
>
> 준비물: Parallels Windows 11 VM · 0.14.8(또는 직전 버전) 설치본 · 신 버전 `cys_x.y.z_x64-setup.exe`.
> 모든 PowerShell 은 **관리자 아님**(설치 모드가 currentUser)으로 연다.

---

## 0. 사전 상태 고정

```powershell
$INST = "$env:LOCALAPPDATA\cys"
Get-ChildItem $INST -Filter '*.exe' | Select-Object Name, Length, LastWriteTime
(Get-Item "$INST\cys.exe").VersionInfo.FileVersion
```

기대: `cys.exe`·`cysd.exe`·`cys-app.exe` 3종이 존재하고 이전 버전이다.
`.prev*.exe` 가 이미 있으면 지우고 시작한다(직전 lame-duck 잔해).

---

## P1 — ★핵심: 업그레이드 내내 CLI 가 한 번도 사라지지 않는다

이번 사고의 1차 증상("`cys: not recognized`")을 직접 반증하는 항목이다.

**① 감시 창**(PowerShell 창 A) — 설치기를 띄우기 **전에** 먼저 돌린다.

```powershell
$INST = "$env:LOCALAPPDATA\cys"
$log  = "$env:USERPROFILE\Desktop\cys-upgrade-watch.txt"
Remove-Item $log -ErrorAction SilentlyContinue
1..6000 | ForEach-Object {
  $t = Get-Date -Format 'HH:mm:ss.fff'
  "$t cys=$(Test-Path "$INST\cys.exe") cysd=$(Test-Path "$INST\cysd.exe") app=$(Test-Path "$INST\cys-app.exe")" |
    Add-Content $log
  Start-Sleep -Milliseconds 200
}
```

**② 라이브 부하**(창 B) — 실사고와 같은 조건(세션이 살아 있는 상태)을 만든다.

```powershell
& "$env:LOCALAPPDATA\cys\cys.exe" list
& "$env:LOCALAPPDATA\cys\cys.exe" new-surface --role master
& "$env:LOCALAPPDATA\cys\runtime\git\usr\bin\bash.exe" -c "sleep 900"   # msys DLL 잠금 재현
```

**③** 신 버전 `*-setup.exe` 를 더블클릭해 **끝까지** 진행(업그레이드 페이지는 기본 선택 그대로).

**④ 판정**(창 A 중지 후)

```powershell
$log = "$env:USERPROFILE\Desktop\cys-upgrade-watch.txt"
$max = 0; $cur = 0
foreach ($l in Get-Content $log) { if ($l -match 'False') { $cur++; if ($cur -gt $max) { $max = $cur } } else { $cur = 0 } }
"연속 False 최대 = $max 줄 (약 $($max*200) ms)"
Select-String -Path $log -Pattern 'False' | Select-Object -First 5      # 있으면 앞부분만 눈으로
Test-Path "$env:LOCALAPPDATA\cys\cys.exe"; Test-Path "$env:LOCALAPPDATA\cys\cysd.exe"   # 설치 종료 후: 둘 다 True
```

| 결과 | 판정 |
|---|---|
| False 줄이 아예 없음 | **PASS** |
| 연속 False 최대 **2줄 이하**(≈0.4초)이고 곧바로 True 로 복귀 | **PASS** — 설계상 남는 전이 창이다(아래 설명) |
| 연속 False 최대 3~9줄(0.6~1.8초) | **조사 필요** — 설계 창의 상한을 넘었다(HDD·AV 스캔 지연 의심). 로그 첨부 보고 |
| 연속 False **10줄 이상**(≥2초) 또는 설치 종료 후에도 False | **FAIL** — 즉시 보고(수리 무효) |

> ★기준이 바뀐 이유(2026-08-01 적대 검증): 종전 기준은 "False 가 한 줄이라도 있으면 FAIL"
> 이었는데, 이는 코드 주석의 **"한 순간도 비우지 않는다"** 라는 부정확한 문언에 기대고 있었다.
> 실제 구현은 `Rename`(이름 비움) → `cmd copy`(자리 채움) 2단이라 **그 사이 수십~수백 ms 의
> 전이 창이 실재한다**(cmd 기동 + 복사 시간). 200ms 간격 감시라면 정상 동작에서도 False 가
> 0~2줄 찍힐 수 있으므로, 1줄을 FAIL 로 읽으면 멀쩡한 빌드를 반려하게 된다.
> 이 항목이 실제로 반증하는 것은 **"자리가 빈 채로 남지 않는다"** 이며, 그 판정은
> ① 연속 창의 길이(≤0.4초)와 ② 설치 종료 후 존재(무조건 True) 둘로 한다.
>
> 수리 전 코드였다면 `cys=False` 가 **수십 초~수 분**(= 수백~수천 줄) 연속으로 찍힌다
> (사이드카는 NSIS `File` 목록의 맨 마지막에 풀리기 때문). 즉 이 항목의 핵심 신호는
> "False 유무" 가 아니라 **"연속 창의 자릿수"** 다 — 수백 줄 vs 0~2줄.

---

## P2 — 업그레이드가 완료되면 신본이 실제로 들어와 있다

```powershell
$INST = "$env:LOCALAPPDATA\cys"
& "$INST\cys.exe" --version
(Get-Item "$INST\cys-app.exe").VersionInfo.FileVersion
Test-Path "$INST\cys-install-failure.txt"      # False 여야 한다
Get-Content "$INST\cys-installed-version.txt"  # 신 버전이어야 한다(게이트 통과 후에만 갱신됨)
Get-ChildItem $INST -Filter '*.prev*.exe'      # lame-duck 잔해(있어도 정상)
```

PASS 조건: `cys --version` 이 **신 버전** · `cys-install-failure.txt` 부재 ·
`cys-installed-version.txt` 가 신 버전.
(구 버전이 찍히면 = 추출이 조용히 스킵된 반쪽 업그레이드 → FAIL, 보고.)

> ★2026-08-01 추가: 이제 이 "반쪽 업그레이드" 는 **설치기 자신이 판정**한다(신선도 게이트 R4).
> 설치기는 추출 직전 3종의 지문(크기+최종수정시각)을 적어두고 끝에서 다시 재어, 지문이 그대로면
> "추출이 그 파일을 못 건드렸다" 로 보고 **exit 4 로 실패**시킨다. 따라서 이 P2 가 조용히
> 구 버전을 보여주는 상황은 원리적으로 설치 성공과 공존할 수 없다 — 만약 그런 조합을 보면
> 게이트 자체의 결함이니 **최우선 보고**다.
> `cys-installed-version.txt` 는 그 게이트를 통과한 뒤에만 쓰이므로, 실패·중단한 설치로는
> 갱신되지 않는다(= 이 파일의 값이 '마지막으로 실제 반영된 버전' 이다).

---

## P3 — 세션 무손실(회귀 확인)

```powershell
& "$env:LOCALAPPDATA\cys\cys.exe" list        # role=master 세션이 그대로 보여야 한다
Get-Process cysd | Select-Object Id, Path      # Path 가 cysd.prev.exe = lame-duck(정상)
```

PASS: 설치 전 만든 세션이 살아 있다. (P1-② 의 bash 프로세스도 살아 있어야 한다.)

---

## P4 — ★중단 주입: 설치를 중간에 끊어도 CLI 가 남는다

수리의 두 번째 축(중단 안전성). 아래 둘 중 하나만 해도 된다.

**P4-a 사용자 취소** — 설치기 진행 막대가 도는 도중(runtime 추출 중) **취소**를 누른다.
**P4-b 강제 종료** — 설치기 실행 중 다른 창에서:

```powershell
Get-Process | Where-Object { $_.Name -like '*setup*' } | Stop-Process -Force
```

**판정**

```powershell
$INST = "$env:LOCALAPPDATA\cys"
Test-Path "$INST\cys.exe"; Test-Path "$INST\cysd.exe"     # 둘 다 True 여야 한다
& "$INST\cys.exe" --version                                # 구 버전이어도 OK(동작만 하면 됨)
```

| 결과 | 판정 |
|---|---|
| 둘 다 True 이고 `cys --version` 이 정상 출력 | **PASS** — 중단이 '빈 자리'가 아니라 '구버전 동작본'을 남겼다 |
| 하나라도 False | **FAIL** — 즉시 보고 |

정리: 다시 설치기를 완주해 정상 상태로 되돌린다.

---

## P5 — ★검증 게이트 음성 시험: 신본이 못 들어오면 '크게' 실패한다

POSTINSTALL 검증 게이트가 실제로 물리는지 확인한다. **고의 고장 주입**이라 반드시 복구까지 한다.

```powershell
# 준비: 데몬 정지 + cys.exe 자리를 '쓰기 금지 0바이트' 로 만든다
Get-Process cys,cysd,cys-app -ErrorAction SilentlyContinue | Stop-Process -Force
$INST = "$env:LOCALAPPDATA\cys"
Remove-Item "$INST\cys.exe" -Force
New-Item -ItemType File "$INST\cys.exe" | Out-Null
icacls "$INST\cys.exe" /inheritance:r /grant:r "$($env:USERNAME):(R)"
```

이 상태로 설치기를 **완주**시킨다.

**기대 동작**
- 설치기가 마지막에 **실패**로 끝난다(빨간 중단 · GUI 면 `cys installation did not complete correctly.` 메시지 상자).
- `%LOCALAPPDATA%\cys\cys-install-failure.txt` 가 생기고 `unrecoverable:` 줄에 `cys.exe` 가 적혀 있다.
- 무인(silent) 설치라면 종료 코드가 **3**이다:

> 종료코드 규약: **3 = 실행물이 없거나 구본으로 원복됨**(`unrecoverable:`/`rolled-back-to-previous:`) ·
> **4 = 파일은 멀쩡하나 신본이 안 들어옴**(`not-updated:` · P10 이 이 쪽을 시험한다).
> 둘 다 "당신의 cys 는 여전히 동작하고 세션도 안 죽었다" 가 전제다 — 재실행이 정답.
  ```powershell
  $p = Start-Process "<...>-setup.exe" -ArgumentList '/S' -PassThru -Wait; $p.ExitCode   # 3 기대
  ```

**복구(필수)**

```powershell
icacls "$INST\cys.exe" /reset
Remove-Item "$INST\cys.exe","$INST\cys-install-failure.txt" -Force -ErrorAction SilentlyContinue
# 설치기를 다시 완주 → P2 로 정상 확인
```

| 결과 | 판정 |
|---|---|
| 실패 + 흔적 파일 + exit 3 | **PASS** — 조용한 반쪽 설치가 불가능해졌다 |
| 설치기가 "성공" 으로 끝남 | **FAIL** — 게이트 미작동, 즉시 보고 |

---

## P6 — 잠긴 파일을 들고 있어도 업그레이드가 성공한다

수리의 첫 축(rename → 사본 → 추출)이 잠금을 우회하는지 본다.

```powershell
$fs = [IO.File]::Open("$env:LOCALAPPDATA\cys\cys.exe",'Open','ReadWrite','None')   # 배타 핸들
# 이 상태로 설치기 완주
$fs.Close()
```

PASS: 설치 성공 + `cys --version` 이 신 버전.
(핸들은 rename 된 `cys.prev.exe` 를 따라가고, 정식 자리에는 잠기지 않은 사본이 서므로 추출이 통과한다.)

---

## P7 — ★훅 bash 경로(Claude Code 훅 생명선)

CI 에 하드 단언을 넣었지만(`windows-build.yml`·`release.yml`), 실기에서 한 번 눈으로 확인한다.

```powershell
$INST = "$env:LOCALAPPDATA\cys"
Test-Path "$INST\runtime\git\bin\bash.exe"          # True 여야 한다  ← Claude Code 훅 실행 경로
Test-Path "$INST\runtime\git\usr\bin\bash.exe"      # True (MSYS 실 바이너리)
& "$INST\runtime\git\bin\bash.exe" -c "echo hook-bash-ok"
```

PASS: 세 줄 모두 정상(`hook-bash-ok` 출력).
그 뒤 cys 창에서 Claude Code 를 띄워 훅이 실제로 도는지(세션 시작 배너·role bootstrap) 확인한다.

---

## P8 — 인앱 업데이터 경로

앱 UI 의 업데이트 알림으로 설치했을 때도 P1·P2 가 성립하는지 확인한다.
(NSIS 는 `/S`·`/UPDATE` 로 조용히 돌기 때문에, 실패해도 화면에 안 뜬다 → **반드시 아래를 본다**.)

```powershell
Test-Path "$env:LOCALAPPDATA\cys\cys-install-failure.txt"    # False 여야 한다
& "$env:LOCALAPPDATA\cys\cys.exe" --version                   # 신 버전
```

---

## P9 — 제거(uninstall) 회귀

설정 → 앱 → cys 제거 → 다음이 남지 않아야 한다.

```powershell
Test-Path "$env:LOCALAPPDATA\cys\runtime"
Get-ChildItem "$env:LOCALAPPDATA\cys" -Filter '*.prev*' -ErrorAction SilentlyContinue
Test-Path "$env:LOCALAPPDATA\cys\cys-install-failure.txt"
Test-Path "$env:LOCALAPPDATA\cys\cys-installed-version.txt"
```

네 줄 모두 비어 있거나 False = PASS.

---

## P10 — ★신선도 게이트 음성 시험: '구본 잔존' 을 성공으로 위장하지 못한다

P5 가 "파일이 없을 때" 를 시험한다면, P10 은 **"파일은 멀쩡한데 옛날 것일 때"** 를 시험한다.
2026-08-01 적대 검증이 지적한 잔여 구멍(존재+64KB 만 보면 R1 이 세운 구버전 사본이 그대로
통과 = 반쪽 업그레이드를 설치기가 성공 보고)을 직접 반증하는 항목이다.

**원리**: 설치기는 '직전 설치 버전'(레지스트리 `DisplayVersion` 또는 마커 파일)이 이번 버전과
같으면 신선도 판정을 면제한다(같은 버전 재설치는 지문이 같아도 정상이므로). 그래서 **같은
설치본을 두 번 돌리되 레지스트리에 '직전은 구버전이었다' 고 적어두면**, 설치기는 업그레이드를
기대하게 되고 — 실제로는 같은 파일이 들어오므로 지문이 그대로다 — **신본 미반영으로 판정**해야
한다. 파일을 망가뜨리지 않는 안전한 주입이다.

```powershell
$INST  = "$env:LOCALAPPDATA\cys"
$K     = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\cys"
$setup = "<신 버전 -setup.exe 전체 경로>"

# 0) 정상 설치가 끝난 상태에서 시작한다(P2 PASS 직후가 이상적)
(Get-ItemProperty $K).DisplayVersion            # 현재 버전이 보여야 한다
Remove-Item "$INST\cys-installed-version.txt" -Force -ErrorAction SilentlyContinue   # 마커 면제도 해제
Set-ItemProperty $K -Name DisplayVersion -Value '0.0.1'    # '직전 설치는 구버전' 으로 위장

# 1) 같은 설치본을 무인으로 다시 돌린다
$p = Start-Process $setup -ArgumentList '/S' -PassThru -Wait; $p.ExitCode              # 4 기대

# 2) 판독
Get-Content "$INST\cys-install-failure.txt"     # not-updated: 에 cys.exe/cysd.exe/cys-app.exe
& "$INST\cys.exe" --version                     # 여전히 정상 동작해야 한다(기계는 살아 있다)
```

| 결과 | 판정 |
|---|---|
| exit **4** + `not-updated:` 목록 + `cys --version` 정상 | **PASS** — 구본 잔존을 성공으로 위장하지 못한다 |
| exit 0(성공) | **FAIL** — 게이트 미작동, 즉시 보고 |
| `cys --version` 이 실패 | **FAIL** — 게이트가 기계를 망가뜨렸다(있어서는 안 되는 일), 즉시 보고 |

**복구(필수)** — 설치기를 정상으로 한 번 완주시키면 레지스트리·마커가 모두 제 값으로 돌아온다.

```powershell
Start-Process $setup -ArgumentList '/S' -Wait
(Get-ItemProperty $K).DisplayVersion ; Get-Content "$INST\cys-installed-version.txt"   # 둘 다 신 버전
Test-Path "$INST\cys-install-failure.txt"                                              # False
```

> ⚠ 이 시험이 **PASS 여야 정상**이라는 점을 기억한다 — 여기서 설치기가 "성공" 이라고 말하면,
> 실사용에서도 잠긴 CLI 위로 업그레이드가 조용히 스킵된 채 성공 보고가 나간다는 뜻이다.

---

## 결과 기록 양식

| 항목 | PASS/FAIL | 메모 |
|---|---|---|
| P1 CLI 무소실 창 | | **연속 False 최대 N줄**(≈N×200ms) · 설치 후 존재 여부 |
| P2 신본 반영 | | 설치 후 `cys --version` · `cys-installed-version.txt` |
| P3 세션 무손실 | | |
| P4 중단 주입 | | 취소/강제종료 어느 쪽 |
| P5 검증 게이트 음성(파일 없음) | | exit code (3 기대) |
| P6 잠금 보유 업그레이드 | | |
| P7 훅 bash | | |
| P8 인앱 업데이터 | | |
| P9 제거 | | |
| P10 신선도 게이트 음성(구본 잔존) | | exit code (4 기대) · `not-updated:` 목록 |

FAIL 이 하나라도 나오면 `%LOCALAPPDATA%\cys\cys-install-failure.txt` 와
`cys-upgrade-watch.txt` 를 그대로 첨부해 보고한다.

---

## 부록 — 이 문서가 닫는 갭과 남는 갭 (2026-08-01 적대 검증 정산)

| 갭 | 상태 | 근거 |
|---|---|---|
| 정식 이름이 **빈 채로 남는 종단** | **닫힘(코드)** | copy 실패 시 rename 되돌리기(3 슬롯 전수). 정적 전개 768 경로에서 '완주한 설치'의 빈 자리 종단 **0건** |
| **구본 잔존 미탐**(반쪽 업그레이드를 성공 보고) | **닫힘(코드)** | 추출 전후 지문 대조 게이트. 정적 전개에서 '구본 + 성공' 조합 **0건**(종전 코드에서는 144경로) |
| 주석 문언과 코드 불일치 | **닫힘(문서)** | `nsis-hooks.nsh` R1 조문을 실제 동작(전이 창 잔존)으로 진실화 · P1 판정 기준 동반 수정 |
| **전이 창 자체**(rename→copy 사이 수십~수백 ms) | **남음(설계상 수용)** | 없애려면 '사본 선행 + 2단 rename' 재설계가 필요. 종전 대비 4~5 자릿수 축소라 현 릴리스에서는 수용하고 문언으로 명시 |
| 지문 판정의 **면제 구멍**(직전 버전 증거가 둘 다 없는 기계) | **남음(의도적)** | 증거 없이 고발하면 레지스트리를 지운 기계가 영구 실패 루프에 빠진다. 마커 파일 도입 이후 버전부터 사실상 닫힌다 |
| 스테이징 사본이 **복사 후에 손상**되는 경우(AV 절단 등) | **남음** | 64KB 미만이면 R2 가 잡지만, 중간 크기 손상은 지문이 '바뀐 것' 으로 보여 통과할 수 있다. 실기 P6·P10 으로 감시 |
| **실기 실행 검증 전무**(mac 개발기) | **남음** | 이 문서의 P1~P10 이 그 격차를 닫는 유일한 수단 — 1회 완주 필요 |
