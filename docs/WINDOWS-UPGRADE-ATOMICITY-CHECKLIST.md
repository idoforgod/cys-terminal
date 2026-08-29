# Windows 업그레이드 원자성 — Parallels 실기 체크리스트

> 2026-08-01 윈도우 실사고(0.14.8→0.14.9 업그레이드 후 `cys.exe`·`cysd.exe` 동시 소실, `.prev.exe`만
> 잔존 → 사용자가 정지 명령조차 실행 불가) 의 근본 수리(`src-tauri/nsis-hooks.nsh`)를 **실기에서**
> 확인하기 위한 절차. mac 개발기에서는 윈도우 설치기를 실행할 수 없어 코드 판독·모델 검증까지만
> 마쳤다 — 아래는 그 **미검증분**을 사람이 1회 클릭해 닫는 목록이다.
>
> ★2026-08-29(v0.14.28 · W4) 전면 개정. 2026-08-28 실사고(잠긴 구 `cys.exe` 위로 재설치 →
> 영원한 exit 4 루프)의 재설계를 반영한다 — 지문(크기+mtime) 비교와 같은버전 면제 기계는
> **제거**됐고, 판정은 **절대 신선도 오라클**(정식 자리 파일의 VERSIONINFO DWORD 2개 ==
> 이번 빌드 원본에서 컴파일 시 뽑은 상수 — 훅 컴파일 게이트 `!getdllversion /packed` · 런타임 `CYS_ORACLE`)
> 이며, 배치는 `<bin>.new.exe` 추출→검증→**rename 2단**의 3종 트랜잭션(`CYS_PLACE` ·
> 순서 cys→cysd→cys-app = POSTINSTALL 의 `!insertmacro CYS_PLACE` 3행)이다. 기대값의 동결 스키마는
> `_work/win-installer-fix-20260829/NSIS-CONTRACT.md`(이름·토큰·종료코드 계약)다.
> 아래 P1/P2/P5/P6/P9/P10/P11 의 기대값은 이 개정 기준이다.
>
> ★훅 인용 규약(R3 확립): 이 문서는 훅을 **라인 번호가 아니라 앵커**(매크로명 `CYS_*` ·
> 본문 라벨(`cys_pl_*`·`cys_ld_*`·`cys_post_*` 류) · 콜백 함수명 · ①~⑧ = NSIS-CONTRACT §2 의
> 동결 단계번호)로 인용한다. 위치 찾기는 `grep -n '<앵커>' src-tauri/nsis-hooks.nsh`.
> 라인 번호 인용은 훅 편집마다 썩는 것이 두 번 실측됐고(R2·R3), 앵커의 실재는 컴파일
> 하네스(`scripts/tests/nsis-hook-compile/run.sh` **N6**)가 매 브랜치 상시 단언한다.
>
> 준비물: Parallels Windows 11 VM · 직전 버전 설치본 · 신 버전 `cys_x.y.z_x64-setup.exe`.
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
| False 줄이 아예 없음 | **PASS** (오라클 단락·rename 창이 표집 사이에 지나감 — 정상 다수) |
| 연속 False 최대 **2줄 이하**(≈0.4초)이고 곧바로 True 로 복귀 | **PASS** — 설계상 남는 전이 창이다(아래 설명) |
| 연속 False 최대 3~9줄(0.6~1.8초) | **조사 필요** — 설계 창의 상한을 넘었다(HDD·AV 스캔 지연 의심). 로그 첨부 보고 |
| 연속 False **10줄 이상**(≥2초)인데 설치기가 **성공(exit 0)** 으로 끝남 | **FAIL** — 즉시 보고(수리 무효) |
| 설치 종료 후에도 False | **FAIL** — 즉시 보고. 단 재부팅으로 cysd 가 뜨면 부팅 가드(P11)가 복구해야 한다 — 복구돼도 보고 대상 |

> ★기준의 근거(2026-08-29 W4 개정): 배치는 `<bin>.new.exe` 에 신본을 먼저 추출·검증해 두고
> **rename 2회**(정식→prev 슬롯 `cys_pl_vacate` → `.new`→정식 `cys_pl_fill`)
> 로 교체한다. 두 rename 은 모두 메타데이터 연산이라 정식 이름이 비는 창은 **수 ms** 다
> (훅 침불변 L2). 종전(0.14.27)의 `Rename → cmd copy` 2단이 만들던 수십~수백 ms 창보다 더
> 좁아졌고, 같은 버전 재설치는 오라클 단락(`CYS_PLACE` ①)으로 **아예 스왑을 하지
> 않으므로** 창 자체가 없다. 200ms 표집에서 False 0~2줄은 정상 범위다.
>
> 채우기 rename 이 일시 거부되면(AV 가 `.new` 를 쥔 경우) 유한 재시도(5회×1초 —
> `CYS_RENAME_RETRY`)가 도는 동안 창이 최대 ~5초까지 늘 수 있으나, 그 종단은
> 반드시 **prev 슬롯 복귀**(`CYS_RESTORE_SLOT`) 후 `placement-refused` 유음 실패다
> — 즉 "창이 길었는데 설치는 성공" 조합은 존재하지 않는다. 그래서 FAIL 판정은 "긴 창 +
> exit 0" 조합에 걸며, 설치기가 실패(exit 3/4 + `cys-install-failure.txt`)로 끝났다면 그것은
> 설계된 유음 경로다(P5/P6 로 이동해 토큰을 판독하라).
>
> 수리 전(0.14.8) 코드였다면 `cys=False` 가 **수십 초~수 분**(= 수백~수천 줄) 연속으로 찍힌다
> (사이드카는 NSIS `File` 목록의 맨 마지막에 풀리기 때문). 즉 이 항목의 핵심 신호는
> "False 유무" 가 아니라 **"연속 창의 자릿수"** 다 — 수백 줄 vs 0~2줄.
> 마지막 안전선: 어떤 실패로든 정식이 빈 채 남으면 콜백(`.onInstFailed`/`.onUserAbort`
> — 본체 `CYS_ABORT_RESCUE`)과 최종 바닥 점검(`CYS_LASTDITCH`), 그리고 다음 cysd 부팅의 회수 가드
> (P11)가 `.new`(신본) → prev 체인(구본) 순으로 자리를 닫는다. 콜백·부팅 가드는 정식을
> '존재'가 아니라 **크기(≥64KiB)** 로 판정한다(R1-r1) — 취소가 템플릿 File 의 truncate-write
> 도중이어서 남은 **절단 정식**은 부재로 취급해 자리를 비운 뒤 같은 순서로 복구한다.

---

## P2 — 업그레이드가 완료되면 신본이 실제로 들어와 있다

```powershell
$INST = "$env:LOCALAPPDATA\cys"
& "$INST\cys.exe" --version
(Get-Item "$INST\cys-app.exe").VersionInfo.FileVersion
Test-Path "$INST\cys-install-failure.txt"      # False 여야 한다
Get-Content "$INST\cys-installed-version.txt"  # 신 버전이어야 한다(전 게이트 통과 후에만 갱신됨)
Get-ChildItem $INST -Filter '*.prev*.exe'      # lame-duck 잔해(있어도 정상 — 새 cysd 기동이 청소)
Get-ChildItem $INST -Filter '*.new.exe'        # ★비어 있어야 한다(성공 배치는 rename 으로 소멸 + Delete 벨트 · `CYS_PLACE` ⑧ `cys_pl_commit`)
```

PASS 조건: `cys --version` 이 **신 버전** · `cys-install-failure.txt` 부재 ·
`cys-installed-version.txt` 가 신 버전 · `*.new.exe` 잔존 0.
(구 버전이 찍히면 = 추출이 조용히 스킵된 반쪽 업그레이드 → FAIL, 보고.)

> ★2026-08-29 개정(W4): "반쪽 업그레이드" 판정 방식이 바뀌었다. 종전(0.14.27)의
> 지문(크기+최종수정시각) 전후 대조와 같은버전 면제 기계는 **제거**됐다 — 그 조합이
> "같은 버전 재설치 → 영원한 exit 4 루프"(2026-08-28 실사고)를 낳았기 때문이다.
> 지금은 **절대 오라클**이 판정한다: 정식 자리 파일의 VERSIONINFO DWORD 2개를
> 이번 빌드 원본에서 컴파일 시 뽑아 둔 상수와 비교한다(기대값 `!getdllversion /packed`
> 컴파일 게이트 · 런타임 `CYS_ORACLE` · 배치 후 재검증 ⑦ `cys_pl_rv`). 읽기 실패·리소스
> 부재는 **fail-closed**(`CYS_ORACLE` 의 IfErrors 갈래 = notfresh 간주)다. 따라서:
> - 신본이 실제로 안 들어왔는데 성공(exit 0)으로 끝나는 조합은 원리적으로 없다 — 보이면
>   오라클 자체 결함이니 **최우선 보고**.
> - **같은 버전 재설치는 오라클 단락**(`CYS_PLACE` ①)으로 아무 스왑 없이 exit 0 이다 — prev
>   슬롯이 새로 생기지 않는 것이 정상이다(면제 기계 불요의 근거 · NSIS-CONTRACT §4).
> `cys-installed-version.txt` 는 이제 판정 재료가 아니라 **정보성 마커**다(성공 종단 `cys_post_ok` 의 마커 쓰기) —
> 모든 게이트를 통과한 뒤에만 쓰이므로, 실패·중단한 설치로는 갱신되지 않는다
> (= 이 파일의 값이 '마지막으로 실제 반영된 버전').

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
| 둘 다 True 이고 `cys --version` 이 정상 출력 | **PASS** — 중단이 '빈 자리'가 아니라 '동작본'을 남겼다(콜백 `CYS_ABORT_RESCUE` 가 `.new` 신본 → prev 구본 순으로 자리를 닫으므로 신·구 어느 쪽이어도 정상) |
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
- 설치기가 마지막에 **실패**로 끝난다(빨간 중단 · GUI 면 `cys installation did not complete correctly.` 메시지 상자 · 훅 `cys_post_fail` 경로의 MessageBox).
- `%LOCALAPPDATA%\cys\cys-install-failure.txt` 가 생긴다(`cys_post_fail` 경로가 쓴다). 1행은 종전과 같은
  형식 `cys installer: critical executable verification FAILED (exit N)` 이고, 그 아래
  **토큰 4줄이 항상 존재**한다(해당 없으면 접두어 뒤 빈 값 — 동결 스키마 NSIS-CONTRACT §3):
  ```
  unrecoverable:  ← 이 시험에서는 cys.exe 가 여기에 적힌다(0바이트 자리를 어떤 재료로도 못 메움 · `CYS_LASTDITCH` 의 `cys_ld_fatal` 종단)
  rolled-back-to-previous:
  not-updated:
  placement-refused:  ← cys.exe(vacate-locked) 도 함께 적힌다(읽기전용 ACL 이 rename 대피를 거부 · `cys_pl_stick` 거부 종단)
  ```
- cysd.exe·cys-app.exe 는 토큰 목록에 **없다** — 배치는 cys 부터의 고정 순서 트랜잭션이라
  (POSTINSTALL `!insertmacro CYS_PLACE` 3행 · fail 시 `cys_txn_undo`) 첫 바이너리 거부 시 나머지는 시도조차 하지 않는다(시도 안 한 것은 목록에 없음 —
  NSIS-CONTRACT §3). 단 이 시험에서 cysd/cys-app 파일 자체는 잠겨 있지 않았으므로 템플릿
  추출이 이미 신본으로 덮었을 수 있다 — P5 의 판정 대상이 아니다.
- 무인(silent) 설치라면 종료 코드가 **3**이다:

> 종료코드 규약(훅 `cys_post_lvl` 분기 · NSIS-CONTRACT §4): **3 = 정식이 없(었)거나 구본으로 비상
> 복구됨**(`unrecoverable:`/`rolled-back-to-previous:` 비어있지 않음) · **4 = 거부된
> 바이너리는 구본 무손상·동작, 그 신본 미반영**(`not-updated:`/`placement-refused:` 만
> 비어있지 않음 — P6/P10 이 이 쪽을 시험한다. ★D11 정직 스코프(2026-08-29): "구본 무손상"
> 은 지배 거부 레인(`vacate-locked` 등)의 서술이다 — `reverify-failed` 절단 창(NSIS-CONTRACT
> §9-3)에서는 거부 명명 바이너리의 정식 자리에 '크기 통과 절단본'이 남을 수 있다(항상
> 유음·토큰 명명 · 동작본으로 시작했다면 동작 사본이 가족 이름에 잔존, cysd 부팅 가드가
> 다음 부팅에 수리). 실패 파일의 Note 2행("old build is still in place…")도 같은 지배-레인
> 기준 안내문이다 — 기계 판정은 **토큰 4줄·exit 코드로만** 한다. 형제 바이너리는 템플릿
> 추출로 이미 신본일 수 있고, LASTDITCH `.new` 승격이 만든 세대 분할은 exit-4 레인에서도
> 도달한다 — 전 분할 종단 유음(모델 실측 5,008/5,008 · 무음 0) · 잠금 해제 후 재실행 치유 ·
> 근본 봉합(PREINSTALL `.old` 대피)은 D9-b Release B 이월. 도달 조합·치유 상세 =
> NSIS-CONTRACT §9-3(승격 경로)·§9-5(템플릿 경로)). 둘 다 사용자 안내는 "당신의 cys 는
> 여전히 동작하고 세션도 안 죽었다" 를 전제로 적혀 있다(지배 레인 기준 · 위 D11 스코프) —
> 재실행이 정답(안내 문구도 그렇게 적힌다: "Do NOT uninstall. Quit cys from the app
> (it saves sessions), wait 10 s, run this installer again and choose 'Do not uninstall'."
> · 실패 파일 Action 2행 = NSIS-CONTRACT §3 동결문).
  ```powershell
  $p = Start-Process "<...>-setup.exe" -ArgumentList '/S' -PassThru -Wait; $p.ExitCode   # 3 기대
  ```

**복구(필수)**

```powershell
icacls "$INST\cys.exe" /reset
Remove-Item "$INST\cys.exe","$INST\cys-install-failure.txt" -Force -ErrorAction SilentlyContinue
# cys.new.exe 가 남아 있어도 정상이다(거부된 배치의 스테이징 잔존 — NSIS-CONTRACT §1).
# 다음 완주가 스스로 지우고 새로 추출하므로(`CYS_PLACE` ②) 손대지 않아도 된다.
# 설치기를 다시 완주 → P2 로 정상 확인
```

| 결과 | 판정 |
|---|---|
| 실패 + 흔적 파일(1행 + 토큰 4줄) + exit 3 | **PASS** — 조용한 반쪽 설치가 불가능해졌다 |
| 설치기가 "성공" 으로 끝남 | **FAIL** — 게이트 미작동, 즉시 보고 |

---

## P6 — 잠금 두 계급을 정확히 가른다: 핸들 잠금 = exit 4 거부 · 이미지 잠금 = 성공

★2026-08-29 기대값 반전(W4). 종전 이 항목은 "배타 핸들이어도 성공" 을 기대했지만, 새 설계에서
잠금은 **두 계급**이고 정답이 서로 다르다(훅 침불변 L2 · NSIS-CONTRACT §4):

- **핸들 잠금(delete 비공유)** — rename(대피)에 필요한 DELETE 접근까지 거부한다. 훅은 정식을
  건드릴 수 없으므로 **거부**한다: `exit 4` + **구본 무손상·동작** + `placement-refused:
  cys.exe(vacate-locked)`(vacate 전 슬롯 거부 = `cys_pl_stick` 종단). "성공" 주장은 없다.
- **이미지 잠금(실행 중 프로세스)** — 덮어쓰기·삭제는 거부되지만 **rename 은 허용**된다
  (로드된 PE 이미지의 Windows 특성). 훅은 정식을 prev 슬롯으로 rename 해 비우고 검증된
  `.new` 를 세운다 → **성공**. 이것이 2026-08-28 실사고(잠긴 구 CLI 위로 영원한 exit 4)의
  수리 표적이며, CI 회귀는 `windows-build.yml` **T4-14** 가 상시로 잰다.

**P6-a. 핸들 잠금 ⇒ exit 4 + 구본 무손상**
(★R2 라운드2: 이 항목은 CI 로도 기계화됐다 — `windows-build.yml` **T4-15** 가 FileShare Read
핸들로 cysd 를 잠근 업그레이드에서 exit 4 + §3 4토큰 + 트랜잭션 undo(`not-updated: cys.exe`)
+ 구본 무손상 + 해제 후 재실행 치유까지 상시로 잰다. 아래 수동 절차는 실기 검증용으로 유지.)

```powershell
$INST = "$env:LOCALAPPDATA\cys"
# 읽기는 공유하되 쓰기·삭제(rename)는 비공유하는 핸들 — AV/백업류가 쥐는 전형적 잠금
$fs = [IO.File]::Open("$INST\cys.exe",'Open','Read','Read')
$p = Start-Process "<...>-setup.exe" -ArgumentList '/S' -PassThru -Wait; $p.ExitCode   # 4 기대
Get-Content "$INST\cys-install-failure.txt"    # placement-refused: cys.exe(vacate-locked)
& "$INST\cys.exe" --version                     # ★구 버전 그대로 + 정상 동작(무손상)
$fs.Close()
Start-Process "<...>-setup.exe" -ArgumentList '/S' -Wait   # 핸들 해제 후 재실행 = 성공해야 한다
& "$INST\cys.exe" --version                     # 신 버전
```

| 결과 | 판정 |
|---|---|
| exit **4** + `placement-refused:` 에 `cys.exe(vacate-locked)` + 구본 `--version` 정상 + 해제 후 재실행 성공 | **PASS** |
| 잠긴 채로 "성공(exit 0)" | **FAIL** — 성공 위장(구본 잔존을 성공으로 보고), 즉시 보고 |
| 구본이 동작 불능 | **FAIL** — 거부가 기계를 망가뜨렸다, 즉시 보고 |

> ★공유 모드 주의: 위 명령의 셋째 인자(FileShare)가 `Read` 인 것이 요점이다 — 오라클의
> 읽기 프로브(`CYS_ORACLE` 의 FileOpen)는 통과시키고 rename 만 거부하는, 실세계 AV 잠금의 재현이다.
> 완전 배타(`'None'`) 핸들로 시험하면 오라클·최종 바닥 점검의 **읽기 프로브 자체가 실패**해
> fail-closed 규약(측정 불능 ≠ 통과)에 따라 `unrecoverable:` 보고(exit 3)로 끝난다 —
> 파일은 그대로 있으니 기계는 무손상이지만, 판정 코드가 3 으로 달라지는 것이 정상이다.

**P6-b. 이미지 잠금 ⇒ 성공(실사고 재현 — 라이브 CLI 위로)**

```powershell
$INST = "$env:LOCALAPPDATA\cys"
# 구 cys.exe 를 실행 상태로 유지(이미지 잠금) — 이벤트 구독은 장수 프로세스다
Start-Process "$INST\cys.exe" -ArgumentList 'events','--reconnect' -WindowStyle Minimized
# 이 상태로 신 버전 설치기 완주
& "$INST\cys.exe" --version                    # 신 버전 (구독 프로세스는 prev 슬롯 이름으로 계속 산다)
Test-Path "$INST\cys-install-failure.txt"      # False
```

PASS: 설치 성공 + `cys --version` 신 버전 + 구독 프로세스 무사망.

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

설정 → 앱 → cys 제거 → 다음이 남지 않아야 한다. (제거는 의도적 전면 종료다 — 세션 보존
대상이 아니며, PREUNINSTALL 이 잔해를 **이름 스코프 와일드카드**로 정리한다: `.new.exe` 3종 ·
`<bin>.prev*.exe`(고정 슬롯 + tick 슬롯) · 마커 2종 · `runtime\` 트리(`RMDir /r`) —
PREUNINSTALL 의 taskkill 9행 뒤 Delete 블록. `$INSTDIR` 에는 사용자 데이터가 있어 광범위 와일드카드는 쓰지 않는다.)

```powershell
Test-Path "$env:LOCALAPPDATA\cys\runtime"
Get-ChildItem "$env:LOCALAPPDATA\cys" -Filter '*.prev*' -ErrorAction SilentlyContinue
Get-ChildItem "$env:LOCALAPPDATA\cys" -Filter '*.new.exe' -ErrorAction SilentlyContinue   # ★신설 이름도 0
Test-Path "$env:LOCALAPPDATA\cys\cys-install-failure.txt"
Test-Path "$env:LOCALAPPDATA\cys\cys-installed-version.txt"
```

다섯 줄 모두 비어 있거나 False = PASS.

> 알려진 한계(무해·차기): tick 슬롯 이름(`cys.prev<숫자>.exe`)으로 **실행 중**인 lame-duck
> 프로세스는 제거기의 고정 kill 목록(PREUNINSTALL 의 taskkill 9행) 밖이라, 그 파일 하나가 삭제를 거부하고 남을
> 수 있다(NSIS-CONTRACT §8). 보이면 프로세스 종료 후 수동 삭제 — FAIL 이 아니라 기록 대상.

---

## P10 — ★오라클 시험: '구본 잔존 성공 위장' 불가 + '같은 버전 재설치' 는 조용히 성공

★2026-08-29 전면 재작성(W4). 종전 P10 의 주입법(레지스트리 `DisplayVersion` 위장)은 시험할
기계가 사라졌다 — 지문 비교와 같은버전 **면제 기계 자체가 제거**됐고(훅에 `DisplayVersion`/
마커 읽기가 더는 존재하지 않는다 — `cys-installed-version.txt` 는 쓰기 전용 정보성 마커
— `cys_post_ok` 에서만 쓴다), 판정은 정식 파일의 VERSIONINFO 를 직접 묻는 절대
오라클(`CYS_ORACLE`)이다. 그 설계에서
"구본 잔존 + 성공 보고" 조합은 종단이 없다: 오라클이 notfresh 로 판정한 바이너리는
성공(스왑 완료·⑦ `cys_pl_rv` 재검증 통과)하거나, 토큰과 함께 exit 3/4 로 거부된다. 이 항목은
그 오라클의 **양방향**을 실기로 잰다.

**P10-a. 같은 버전 재설치 = 조용한 성공(0.14.27 "영원한 exit 4 루프" 회귀 핀)**

```powershell
$INST  = "$env:LOCALAPPDATA\cys"
$setup = "<방금 설치한 것과 같은 -setup.exe 전체 경로>"
# 정상 설치가 끝난 상태(P2 PASS 직후)에서, 같은 설치본을 무인으로 한 번 더:
$before = @(Get-ChildItem $INST -Filter '*.prev*.exe').Count
$p = Start-Process $setup -ArgumentList '/S' -PassThru -Wait; $p.ExitCode   # ★0 기대(오라클 단락 `CYS_PLACE` ①)
Test-Path "$INST\cys-install-failure.txt"                                   # False
@(Get-ChildItem $INST -Filter '*.prev*.exe').Count                          # $before 보다 늘지 않아야 한다(스왑 미발생 = prev 신규 0 · cysd 스윕이 줄이는 것은 정상)
& "$INST\cys.exe" --version                                                 # 같은(신) 버전
```

| 결과 | 판정 |
|---|---|
| exit **0** + 실패 파일 없음 + prev 신규 생성 0 | **PASS** — 같은 버전 재설치가 면제 기계 없이 단락된다(NSIS-CONTRACT §4) |
| exit 4 | **FAIL** — 0.14.27 계열 회귀(재설치 루프), 즉시 보고 |

**P10-b. 구본이 자리에 있으면(잠금 없음) 반드시 갈아치운다 — 성공 위장이 아니라 실제 스왑**

```powershell
$INST  = "$env:LOCALAPPDATA\cys"
$setup = "<신 버전 -setup.exe 전체 경로>"
Get-Process cys,cysd,cys-app -ErrorAction SilentlyContinue | Stop-Process -Force
# '옛날 파일' 재현: 다른 VERSIONINFO 의 자기완결 실행 PE(cmd.exe 사본)를 정식 자리에 세운다
Copy-Item "$env:SystemRoot\System32\cmd.exe" "$INST\cys.exe" -Force
(Get-Item "$INST\cys.exe").VersionInfo.FileVersion          # 우리 버전이 아님을 확인

$p = Start-Process $setup -ArgumentList '/S' -PassThru -Wait; $p.ExitCode   # 0 기대(notfresh → 스왑 `cys_pl_need` 이후 ②~⑧)
& "$INST\cys.exe" --version                                                 # ★신 버전(스탠드인이 아니라)
Test-Path "$INST\cys-install-failure.txt"                                   # False
```

| 결과 | 판정 |
|---|---|
| exit 0 + `cys --version` 신 버전 | **PASS** — notfresh 정식을 실제로 교체했다 |
| exit 0 인데 `--version` 이 우리 버전이 아님(스탠드인 잔존) | **FAIL** — 성공 위장, 최우선 보고 |
| exit 3/4 | **조사 필요** — 무잠금 스왑이 거부됐다. `cys-install-failure.txt` 첨부 보고 |

> ⚠ 잠금 하 구본 시나리오(실사용의 실제 형태)는 P6 이 담당한다 — 이미지 잠금(P6-b)은 성공,
> 핸들 잠금(P6-a)은 exit 4 유음 거부. 어느 경로에도 "구본인데 성공 보고" 는 없다.
> CI 상시 회귀: `windows-build.yml` T4-14(이미지 잠금) · T4-4/T4-12(단락·바이트 동일성).

---

## P11 — ★전원 차단 주입: 두 rename 사이에서 죽어도 다음 부팅이 정식을 수리한다 (신설 · W4)

배치의 정식 부재 창은 "정식→prev rename(⑤ `cys_pl_vacate`)" 과 "`.new`→정식 rename(⑥ `cys_pl_fill`)" 사이다.
정확히 그 순간 설치기가 죽으면(전원 차단·강제 종료) 디스크에는 **정식 없음 + `.new`(신본) +
prev(구본)** 가 남는다 — 훅의 콜백(`.onInstFailed`/`.onUserAbort`)은 프로세스가 죽으면 돌 수 없으므로, 이 상태의
복구선은 **cysd 부팅 회수 가드**(Wave 1 W1 · `src/bin/cysd/main.rs` sweep — 정식 부재 시
잔해를 지우는 대신 ① `<bin>.new.exe`(크기 ≥ 64KiB) rename 승격 ② 없으면 최신 mtime 의
`<bin>.prev*` 승격)다. 훅은 그 재료를 지우지 않고 남긴다(NSIS-CONTRACT §9-1: 무음 소실
경로 없음). 이 항목이 그 이어달리기를 실기로 잰다.

ms 단위 창을 손으로 맞출 수는 없으므로 **그 순간의 디스크 상태를 그대로 재현**한다:

```powershell
$INST = "$env:LOCALAPPDATA\cys"
Get-Process cys,cysd,cys-app -ErrorAction SilentlyContinue | Stop-Process -Force
# '두 rename 사이에서 전원이 나간' 디스크 상태 재현:
Copy-Item "$INST\cys.exe" "$INST\cys.new.exe"          # 신본이 .new 스테이징에 있고
Rename-Item "$INST\cys.exe" 'cys.prev.exe'             # 정식은 prev 로 대피된 채 = 정식 부재
Test-Path "$INST\cys.exe"                              # False — 사고 상태 확인

Start-Process "$INST\cysd.exe" -WindowStyle Hidden     # 다음 부팅(가드 발동)
Start-Sleep 5

Test-Path "$INST\cys.exe"                              # ★True — 자리가 수리됐다
& "$INST\cys.exe" --version                            # 정상 동작(.new 승격 = 신본)
Get-Process cys,cysd -ErrorAction SilentlyContinue | Stop-Process -Force
```

변형(재료가 prev 뿐일 때 — `.new` 없이 위 재현에서 `Copy-Item` 줄을 빼고 반복):
가드는 최신 prev 를 승격해야 하며, 이때 `cys --version` 은 **구 버전**이어도 PASS 다
(동작본 복원이 목적 — 신본 반영은 설치기 재실행의 몫).

| 결과 | 판정 |
|---|---|
| 부팅 후 `cys.exe` 존재 + `--version` 정상(재료가 `.new` 면 신본 우선) | **PASS** — 전원 차단이 CLI 를 소실시키지 못한다 |
| 부팅 후에도 정식 부재, 또는 가드가 유일한 재료를 **삭제** | **FAIL** — T3 사고 계열 재발, 즉시 보고 |

---

## 롤백 — 이전 버전으로 되돌리기 (R3 신설 · 절차 없이 실행 금지)

> ★**prev 슬롯은 롤백 수단이 아니다.** `<bin>.prev*.exe` 는 배치 트랜잭션의 undo 재료이자
> 위생 잔해이며, 성공한 업그레이드 뒤에는 두 겹으로 소멸한다: ①훅 커밋 직후의 best-effort
> 정리(`CYS_SLOT_CLEANUP` — 3종 전부 성공한 뒤) ②다음 cysd 부팅 스윕(정식이 동작본이면 가족
> prev 전량 정리 — `src/bin/cysd/main.rs` `plan_leftover_action` 의 SweepAll). 업그레이드 뒤
> prev 가 남아 있대도 그것은 우연(잠금 거부·lame-duck 점유)이지 보장이 아니다.
>
> 유일한 실제 롤백 = **이전 버전 setup.exe 재실행**이다. 단 이전 설치기(v0.14.27 이하)는 이번
> 릴리스가 제거한 두 kill 경로를 그대로 싣고 있다: PREINSTALL 의 `taskkill /F /T /IM
> cys-app.exe`(트리 kill — GUI 가 떠 있으면 자식 cysd 와 전 pane 이 함께 죽는다) · 구 스왑
> 매크로의 `taskkill /F /T` 최후 폴백(prev 3칸이 전부 막혔을 때 발화 — 업그레이드 직후
> lame-duck 프로세스가 슬롯 파일을 점유 중인 기계가 정확히 그 상태다). **라이브 세션 위로
> 그냥 돌리면 0.14.27 kill 의미론으로 되돌아간다.**

**절차 — quiesce 선행(실패 파일 안내문과 같은 문구·순서):**

1. 앱에서 cys 를 종료한다 — "Quit cys from the app (it saves sessions)".
2. **10초 기다린다**("wait 10 s") — cysd 와 lame-duck 프로세스가 내려가 슬롯 점유·잠금이 풀린다.
3. 이전 버전 `cys_<구버전>_x64-setup.exe` 를 실행한다. **제거(uninstall)는 하지 않는다** —
   세션·데이터까지 걷어낸다.
4. `cys --version` 으로 구버전 복귀를 확인한다.

on-disk 문법은 하위 안전이다: 구 cysd(v0.14.27)의 잔해 문법은 "마지막 `.prev` 뒤 = 숫자 0개
이상(+선택적 `.exe`)"라 이번 릴리스가 만드는 prev 슬롯 이름 전부(고정 `.prev[2|3].exe` ·
tick `.prev<숫자>.exe`)와 잠금 스윕의 `.prev<rand>` 를 잔해로 정리하고, `<bin>.new.exe` 는 구
소비자 전부에게 비활성이다(잔존해도 무해 — 다음 0.14.28+ 설치의 stale-`.new` Delete ② 가
정리한다).

---

## 결과 기록 양식

| 항목 | PASS/FAIL | 메모 |
|---|---|---|
| P1 CLI 무소실 창 | | **연속 False 최대 N줄**(≈N×200ms) · 설치 후 존재 여부 |
| P2 신본 반영 | | 설치 후 `cys --version` · `cys-installed-version.txt` · `*.new.exe` 0 |
| P3 세션 무손실 | | |
| P4 중단 주입 | | 취소/강제종료 어느 쪽 |
| P5 검증 게이트 음성(파일 없음) | | exit code (3 기대) · `unrecoverable:`+`placement-refused:` |
| P6-a 핸들 잠금(비공유 delete) | | exit code (**4** 기대) · `placement-refused: cys.exe(vacate-locked)` · 구본 무손상 |
| P6-b 이미지 잠금(실행 중 구 CLI) | | 성공 + 신 버전 (2026-08-28 실사고 재현 · CI T4-14 짝) |
| P7 훅 bash | | |
| P8 인앱 업데이터 | | |
| P9 제거 | | `*.prev*`·`*.new.exe`·마커 2종·runtime 잔존 0 |
| P10-a 같은 버전 재설치 | | exit code (**0** 기대) · prev 신규 0 (오라클 단락) |
| P10-b 구본 잔존 교체 | | exit 0 + `--version` 신 버전 (성공 위장 불가) |
| P11 전원 차단 주입(rename 사이) | | 부팅 가드 수리 — `cys.exe` 복원 · 재료 무삭제 |

FAIL 이 하나라도 나오면 `%LOCALAPPDATA%\cys\cys-install-failure.txt` 와
`cys-upgrade-watch.txt` 를 그대로 첨부해 보고한다.

---

## 부록 — 이 문서가 닫는 갭과 남는 갭 (2026-08-29 W4 재정산)

| 갭 | 상태 | 근거 |
|---|---|---|
| 정식 이름이 **빈 채로 남는 종단** | **닫힘(코드·3중)** | ①배치 실패 전수 슬롯 복귀(`CYS_RESTORE_SLOT`·undo `CYS_UNPLACE`) ②콜백+최종 바닥 점검(`CYS_ABORT_RESCUE`·`CYS_LASTDITCH`) ③cysd 부팅 회수 가드(Wave 1 W1 — 설치기 밖 2차 복구선). P1·P4·P11 이 실기로 잰다 |
| **구본 잔존 미탐**(반쪽 업그레이드를 성공 보고) | **닫힘(코드·설계 교체)** | 지문 대조 폐기 → 절대 오라클(VERSIONINFO == 컴파일 상수 = `CYS_ORACLE`) + 배치 후 재검증(⑦ `cys_pl_rv`). notfresh 종단은 전부 exit 3/4 + 토큰. P10 이 양방향을 잰다 |
| **같은 버전 재설치 → 영원한 exit 4 루프**(2026-08-28 실사고) | **닫힘(코드)** | 면제 기계 제거 + 오라클 단락(`CYS_PLACE` ①) — "이미 이번 빌드" 는 무행동 성공. P10-a·CI T4-4 |
| **이미지 잠금 하 업그레이드 실패**(잠긴 구 CLI 위로) | **닫힘(코드)** | `.new` 추출→rename 2단은 잠긴 정식을 덮지 않고 대피시킨다(③ 추출 → ⑤ `cys_pl_vacate`). P6-b·CI T4-14 |
| **전이 창 자체** | **좁힘(rename 2회 사이 수 ms)** | copy 채움 폐기 — 채움도 rename(⑥ `cys_pl_fill`). copy 는 undo 복귀 경로에만 잔존(`CYS_RESTORE_SLOT` 의 copy 강등). 잔여 창은 P11 의 부팅 가드가 받친다 |
| 핸들 잠금(delete 비공유) 기계 | **의도된 거부(exit 4)** | rename 조차 불가한 잠금은 우회 불가 — 구본 무손상으로 거부하고 재실행 안내(`cys_pl_stick` 거부 종단 · 안내문 = NSIS-CONTRACT §3 Action 동결문). P6-a |
| 스테이징 사본이 **추출 후에 손상**되는 경우(AV 절단 등) | **좁힘** | `.new` 검증(존재·≥64KiB·버전 = ④) + 배치 후 오라클 재검증(⑦)이 잡는다. VERSIONINFO 리소스가 온전한 채 본문만 손상된 파일은 여전히 통과 가능 — 잔여 갭(정직 고지) |
| **실기 실행 검증** | **부분 닫힘(CI)** | `windows-build.yml` T4 레인(T4-1~14)이 실기 1차 검증 — 단 CI 는 같은 버전 재설치라 실제 구→신 업그레이드·GUI 경로는 이 문서(P1~P11) 1회 완주가 여전히 필요 |
