; cys NSIS 설치 훅 — 업그레이드/재설치 시 잠긴 exe("Error opening file for writing") 문제를
; ★무중단(rename-swap · 2026-07-02)으로 푼다. 종전 taskkill cysd는 마스터·워커·부서 PTY가
; 전부 cysd 소유라 "업데이트 = 전 세션 사망"이었다(사용자 불안의 근원).
;
; 원리: Windows는 실행 중 exe의 '덮어쓰기'는 금지하나 '이름 변경(rename)'은 허용(NTFS 동일볼륨).
;   ① GUI(cys-app.exe)만 종료 — 얇은 클라이언트(main.rs:1 "UI가 죽어도 세션(PTY)은 데몬에
;      살아있다. UI 재시작 = 재attach") → 세션 무손실.
;   ② cysd.exe·cys.exe는 죽이지 않고 옆(.prev*.exe)으로 rename → 새 exe를 제자리에 설치.
;      구 데몬은 renamed 파일로 계속 봉사(lame-duck)하고, 새 cysd 기동이 잔해를 청소한다.
;   ③ rename이 전부 실패할 때만 구 방식(taskkill) 폴백 — 현재보다 나빠질 수 없는 우아한 강등
;      (그 경로는 기존 재시작 복원(maybe_apply_pending_update → cys restore)이 받친다).
; prev 체인(prev→prev2→prev3): 직전 lame-duck이 아직 살아 prev 파일을 점유 중일 수 있어
; 고정 3칸으로 우회한다(연속 lame-duck 2개 초과는 실사용상 희귀 — 초과 시 폴백 kill).
;
; ══════════════════════════════════════════════════════════════════════════════
; ★T3 원자화 (2026-08-01 · 윈도우 실사고 "CLI 소실" 근본 수리)
; ══════════════════════════════════════════════════════════════════════════════
; 실사고: 0.14.8→0.14.9 실행 중 업그레이드 후 %LOCALAPPDATA%\cys 에 cys.exe·cysd.exe 가
;        **둘 다 부재**하고 .prev.exe 사본만 남아, 사용자가 정지 명령조차 실행 불가했다.
;
; 코드 판독으로 확정한 구조적 원인 — 단일 버그가 아니라 '파괴 선행 + 무검증 + 무원복' 3중.
;
;  (A) 자리를 먼저 비우고 새 파일은 한참 뒤에 온다 (창(window)이 수십초~수분).
;      Tauri NSIS 템플릿 `Section Install` 의 실행 순서는 고정이다:
;         SetOutPath → **NSIS_HOOK_PREINSTALL** → CheckIfAppIsRunning
;         → File(메인 exe) → File(resources = runtime/ 수백 MB) → File(binaries = cys/cysd)
;         → 레지스트리 → 바로가기 → **NSIS_HOOK_POSTINSTALL**
;      즉 **사이드카(cys.exe·cysd.exe)는 맨 마지막에 풀린다**. 구 훅은 PREINSTALL 에서
;      원본을 .prev 로 rename 해 치워버리므로, 정상 설치에서도 그 사이 내내 cys.exe 가
;      디스크에 없다 → 그 창에서 `cys ...` 는 'cys: not recognized' 로 실패한다.
;
;  (B) 그 창 안에서 설치가 중단되면 **영구 소실**이다. 중단 경로는 템플릿 코드상 실재한다:
;      ① PREINSTALL 바로 다음의 `CheckIfAppIsRunning` 은 프로세스 kill 실패 시 `Abort` 한다
;         (우리 훅이 이미 taskkill 했더라도 종료 완료 전 레이스면 Find→Kill 이 실패할 수 있다)
;      ② 사용자 취소 ③ 디스크 부족 ④ AV 격리.
;      어느 경로든 **POSTINSTALL 은 실행되지 않는다** → 뒤늦은 검증·원복이 원리적으로 불가능.
;
;  (C) `SetOverwrite try` 는 추출 실패를 **조용히 스킵**한다(NSIS 정의: try = 덮어쓰기를 시도하고
;      실패하면 무통보 생략). 잠긴 파일에서 설치 전체가 Abort 되는 것을 막으려고 넣은 안전장치가,
;      "새 실행물이 안 들어왔다"는 사실까지 함께 삼켰다.
;
; ⇒ 수리 원칙 (이 파일의 계약):
;   R1. **정식 이름 자리를 '빈 채로' 끝내지 않는다.** 잠긴 원본은 .prev 로 rename 해 이름을
;       비운 **직후, 곧바로 그 .prev 를 정식 이름으로 복사해 되돌려 놓는다**. 복사본은 아무도
;       실행 중이 아니라 잠기지 않으므로, 뒤에 오는 File 추출이 그대로 덮어써 신본이 된다.
;       → 정상 설치에서도 CLI 부재 창이 사라지고(A 해소),
;       → 중간에 무엇이 Abort 되든 남는 것은 '빈 자리'가 아니라 '구버전 동작본'이다(B 해소).
;       ※ 정확한 계약 (2026-08-01 적대 검증 지적 반영 — 종전 문언 "한 순간도 비우지 않는다"는
;         코드와 불일치였다):
;         · **전이 창은 남는다.** Rename 반환 → cmd 기동 → 복사 완료 사이 수십~수백 ms 동안
;           정식 이름이 실제로 부재한다. 딱 그 순간에 새 `cys ...` 를 띄우면 not recognized 다.
;           종전 결함의 창(수십 초~수 분 · 사이드카가 File 목록 맨 끝이라)과는 4~5 자릿수 다르며,
;           '설치 중 어느 시점에 끊겨도 자리가 비어 남지 않는다'가 이 파일이 보증하는 성질이다.
;         · **복사가 실패하면 rename 을 되돌린다.** Rename 은 메타데이터 연산이라 디스크 부족으로
;           실패하지 않고 부분 파일도 남기지 않는다 → '자리가 빈 채 설치가 계속되는' 종단은
;           구조적으로 제거된다. 되돌린 뒤에는 원본(구본·잠김)이 자리에 있어 추출이 스킵될 수
;           있고, 그 '반쪽 상태'는 R4 가 잡아 크게 실패시킨다(조용한 통과 없음).
;   R2. **끝에서 필수 실행물을 검증한다.** POSTINSTALL 에서 cys.exe·cysd.exe·cys-app.exe 의
;       존재 + 최소 크기를 확인하고, 미달이면 .prev 사본에서 자동 원복한 뒤
;       **SetErrorLevel + 메시지 + Abort 로 크게 실패**한다(C 해소 · 조용한 성공 위장 금지).
;   R3. **잠금 처리 규약** (아래 "잠금 규약" 절) 을 명문화하고 스윕의 사정거리를 좁힌다.
;   R4. **신본이 실제로 들어왔는지를 결정론으로 판정한다** (2026-08-01 적대 검증 FAIL 봉합).
;       R2 의 '존재 + 64KB' 만으로는 **R1 이 세운 구버전 사본이 그대로 통과**한다 — 추출이
;       조용히 스킵되면(위 C) 설치기는 반쪽 업그레이드를 '성공'으로 보고한다. 그래서:
;         · PREINSTALL 이 **추출 직전 상태의 지문**(파일 크기 + 최종수정시각)을 3종 각각에 대해
;           $PLUGINSDIR 에 적어둔다.
;         · POSTINSTALL 이 다시 재어 비교한다. **지문이 바뀌었다 = 이번 실행의 File 추출이 그
;           파일을 썼다(신본 반영) · 그대로다 = 추출이 그 파일을 못 건드렸다(구본 잔존).**
;         · 근거: 지문을 **스테이징이 끝난 뒤에** 재기 때문에, 이 판정은 `cmd copy` 가 원본의
;           시각을 보존하는지 여부에 **의존하지 않는다**(복사가 시각을 보존하든 지금 시각으로
;           갱신하든, 그 결과값이 그대로 기준선이 된다). 비교가 묻는 것은 오직 "스냅샷 이후
;           이 파일이 바뀌었는가" 이고, 그 사이에 이 파일을 쓸 수 있는 주체는 File 추출뿐이다
;           (잠금 스윕은 이보다 먼저 끝나고, 필수 3종은 스윕 제외 대상이다 — L5).
;           그리고 NSIS 는 `SetDateSave` 기본 on 이라 추출이 **빌드 산출물의 최종수정시각을
;           복원**하므로, 다른 빌드가 들어오면 시각이 반드시 달라진다(같은 패키지를 두 번
;           설치하는 경우만 같아질 수 있고, 그건 아래 '면제' 가 걷어낸다).
;           판정은 전부 순수 NSIS(FileSeek·GetFileTime).
;         · **면제(오탐 차단)**: 같은 버전 재설치·복구설치는 지문이 같아도 정상이다. 직전 설치
;           버전을 PREINSTALL 에서 두 출처로 읽어(레지스트리 DisplayVersion + 지난 설치가
;           신선도 게이트를 통과한 뒤에만 남긴 마커 cys-installed-version.txt) 이번 ${VERSION}
;           과 같거나 **둘 다 알 수 없으면 판정을 면제**한다. 레지스트리를 PREINSTALL 에서만
;           읽는 이유: 템플릿이 DisplayVersion 을 추출 뒤에 쓰므로 POSTINSTALL 시점에는 이미
;           신 버전으로 덮여 있다(직전 값을 볼 수 있는 유일한 시점).
;         · **알려진 한계(의도적)**: 두 출처가 모두 비면 '증거 없음' 으로 보고 고발하지 않는다.
;           레지스트리를 지운 기계에서 같은 패키지를 재설치할 때 영구 실패 루프에 빠지는 쪽이
;           더 큰 해악이라 '증거 있을 때만 고발' 을 택했다. 이 경우에도 R2(존재·크기)는 그대로
;           작동하며, 마커가 도입된 이후 버전부터는 이 구멍이 사실상 닫힌다.
;       ─ 기각한 대안과 이유 (NSIS 제약 안에서의 택일 근거):
;         ① 버전 리소스 대조(GetDLLVersion): cys.exe·cysd.exe 는 winres/winresource 계열
;            빌드스크립트가 없어 **VERSIONINFO 자체가 없다**(Cargo.toml·build.rs 확인) →
;            항상 오류 플래그 = 전량 오탐. 리소스를 새로 심으려면 빌드 의존성 추가가 필요해
;            이번 수리의 사정거리를 넘는다.
;         ② 해시 매니페스트: NSIS 코어에 해시 명령이 없고, Tauri 번들 NSIS 에는 해시 플러그인이
;            없다. PowerShell Get-FileHash 로 대신하면 **기업 정책 PC 에서 검증 자체가 못 돌아
;            조용히 통과**하는 구멍이 생겨 R2 의 '판정은 순수 NSIS' 원칙과 정면 충돌한다.
;         ③ 새 exe 실행 후 --version 대조: 갓 쓰인 실행물을 설치 도중 실행해야 하고(AV 스캔·행),
;            타임아웃이 나면 '판정 불능' lane 이 생겨 게이트가 흐려진다. 또 cys-app.exe 에는
;            적용할 수 없다.
;         ④ witness 파일 동봉(작은 txt 를 같이 추출해 성공 여부 확인): 추출 스킵은 **파일별
;            잠금**에 좌우되므로, 잠기지 않는 txt 가 들어왔다는 사실은 exe 가 들어왔다는 증거가
;            전혀 되지 못한다(부당한 안심 = 가짜 게이트).
;
; ── 잠금 규약 (업그레이드 중 데몬·CLI 잠금 처리 · 이 절이 SOT) ────────────────
;   L1. 죽여도 되는 것은 GUI(cys-app.exe) 뿐이다. cysd.exe 는 마스터·워커·부서 PTY 전부의
;       소유자이므로 **정상 경로에서 절대 죽이지 않는다**(죽이면 전 세션 사망).
;   L2. 잠긴 실행 이미지는 '덮어쓰기' 불가·'rename' 가능이다. 따라서 교체는 항상
;       rename(이름 비우기) → 복사(자리 채우기) → 추출(덮어쓰기) 3단으로 한다. (R1)
;   L3. rename 대상 prev 슬롯은 3칸(prev·prev2·prev3). 직전 lame-duck 이 점유 중이면 다음 칸.
;       3칸 전부 실패했을 때만 taskkill 폴백 — 그 경우에도 **원복 재료(.prev 사본)를 먼저 확보**한다.
;   L4. 잠금 스윕(unlock-sweep)은 `runtime\` 하위 + 설치 루트 최상위 1단만 본다. 설치 루트는
;       기본 데몬의 상태 디렉터리(%LOCALAPPDATA%\cys)와 **같은 폴더**라 재귀 스윕은 세션 DB·로그
;       수천 개를 훑는 낭비이자 사고 위험이다.
;   L5. 스윕은 cys.exe·cysd.exe·cys-app.exe 를 **건드리지 않는다**(위 3단이 전담). 스윕이 이들을
;       .prev<rand> 로 밀어버리면 R1 이 무력화된다.
;   L6. 잔해(.prev*) 삭제 책임은 설치기가 아니라 **새 cysd 기동 시 자가청소(P1b)** 다. 설치 중
;       삭제 시도는 best-effort 이며 실패를 오류로 취급하지 않는다.
;
; ── 인코딩 주의 ───────────────────────────────────────────────────────────────
;   이 파일은 BOM 없는 UTF-8 이다. NSIS 는 BOM 없는 스크립트를 ACP 로 읽으므로 **한국어는
;   주석에만** 쓴다(주석은 오독돼도 무해). 사용자에게 보이는 문자열(MessageBox·DetailPrint·
;   FileWrite·콘솔)은 전부 ASCII 로 적는다 — 깨진 글자로 경고하면 경고가 아니다.
; ══════════════════════════════════════════════════════════════════════════════

; ── 매크로: 잠긴 실행물 1개를 '자리를 비우지 않고' 교체 준비 (R1·L2·L3) ──────
;   BIN = 확장자 없는 파일 이름(cys · cysd) / TAG = 라벨 접미(영숫자·밑줄만 — NSIS 라벨에
;   하이픈을 넣지 않기 위해 파일명과 분리한다. 예: BIN="cys-app" → TAG="cysapp").
!macro CYS_SWAP_IN_PLACE BIN TAG
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_swap_done_${TAG}

  ; ① prev 슬롯 확보 후 rename — 실행 중이어도 rename 은 허용된다(이름만 비운다).
  Delete "$INSTDIR\${BIN}.prev.exe"          ; 잔해 청소 시도(구 lame-duck 점유 시 실패해도 무시)
  ClearErrors
  Rename "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev.exe"
  IfErrors 0 cys_swap_back1_${TAG}
  Delete "$INSTDIR\${BIN}.prev2.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev2.exe"
  IfErrors 0 cys_swap_back2_${TAG}
  Delete "$INSTDIR\${BIN}.prev3.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev3.exe"
  IfErrors 0 cys_swap_back3_${TAG}

  ; ③ 최후 폴백 — prev 3칸 전부 실패. 프로세스를 죽여 잠금을 푼다(L3).
  ;    ★폴백에서도 원복 재료를 반드시 남긴다: 이게 없으면 POSTINSTALL 이 되돌릴 것이 없다.
  ;    ※ 이 경로에는 '되돌리기(undo rename)' 가 없다 — **rename 을 한 적이 없기 때문**이다.
  ;      원본은 정식 이름 그대로 자리에 있고, 여기의 copy 는 자리를 채우는 게 아니라 원복 재료를
  ;      뜨는 것이다. 따라서 이 copy 가 실패해도 '빈 자리' 는 생기지 않는다(경고만 남긴다).
  ;      실패의 결과는 "원복 재료 없음" 이며, 그 상태에서 추출까지 스킵되면 구본이 자리에
  ;      남는데 — 그 반쪽 상태는 R4(신선도 판정)가 잡는다.
  nsExec::Exec 'taskkill /F /T /IM ${BIN}.exe'
  Pop $R0
  Sleep 300
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_swap_done_${TAG}
  Delete "$INSTDIR\${BIN}.prev.exe"
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev.exe"'
  Pop $R0
  StrCmp $R0 "0" cys_swap_done_${TAG} 0
  DetailPrint "cys: WARNING - no rollback copy for ${BIN}.exe (kill fallback; freshness gate still applies)"
  Goto cys_swap_done_${TAG}

  ; ② 이름을 비운 즉시 정식 자리에 '잠기지 않은 동일 사본'을 세운다 (R1 핵심).
  ;    - 이 사본은 실행 중이 아니므로 뒤따르는 File 추출이 정상 덮어쓴다(신본 반영).
  ;    - 추출 전에 설치가 중단돼도 사용자에게는 구버전 CLI 가 그대로 남는다(소실 불가).
  ;    ★copy 성공 판정은 **cmd 종료코드**로 한다(IfFileExists 는 '부분 복사' 를 성공으로 오판한다 —
  ;      디스크가 도중에 차면 잘린 파일이 남고 copy 는 1 을 반환한다).
  ;    ★copy 가 실패하면 **rename 을 되돌린다**: Rename 은 메타데이터 연산이라 디스크 부족으로
  ;      실패하지 않고 부분 파일도 남기지 않으므로, 이 되돌리기로 '빈 자리 종단' 이 구조적으로
  ;      사라진다(3 슬롯 전수 적용 — 각자 자기가 쓴 슬롯에서만 되돌린다. 공용 되돌리기를 두고
  ;      prev→prev2→prev3 를 훑으면 이번에 밀어넣은 파일이 아니라 **직전 lame-duck 의 더 오래된
  ;      사본**을 자리에 세울 수 있다).
cys_swap_back1_${TAG}:
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R0
  StrCmp $R0 "0" cys_swap_check_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"                 ; 부분 사본 제거(Rename 은 대상이 존재하면 실패한다)
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"
  Goto cys_swap_undo_${TAG}
cys_swap_back2_${TAG}:
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R0
  StrCmp $R0 "0" cys_swap_check_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"
  Goto cys_swap_undo_${TAG}
cys_swap_back3_${TAG}:
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R0
  StrCmp $R0 "0" cys_swap_check_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"

cys_swap_undo_${TAG}:
  ; 되돌린 뒤에는 원본(잠긴 구본)이 정식 자리에 있다 → 추출이 스킵될 수 있고, 그 반쪽 상태는
  ; POSTINSTALL 의 신선도 판정(R4)이 잡아 크게 실패시킨다. 여기서 Abort 하지 않는 이유는 종전과
  ; 같다(설치 전체를 여기서 끊으면 다른 자산이 어중간하게 남는다).
  DetailPrint "cys: staging copy of ${BIN}.exe failed - rename undone, previous build kept in place"

cys_swap_check_${TAG}:
  ; 복사 실패(디스크 부족 등)를 여기서 가시화한다. 치명 판정은 POSTINSTALL 이 최종적으로 내린다
  ; (여기서 Abort 하면 이미 자리가 빈 채로 끝나 R1 을 스스로 어긴다).
  IfFileExists "$INSTDIR\${BIN}.exe" cys_swap_done_${TAG} 0
  DetailPrint "cys: WARNING - could not stage a working copy of ${BIN}.exe (checked again at end of install)"

cys_swap_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 파일 1개의 '지문' 산출 (R4) ───────────────────────────────────────
;   지문 = "<바이트 크기>/<최종수정시각 상위>/<최종수정시각 하위>" · 파일이 없으면 "ABSENT".
;   순수 NSIS(FileOpen/FileSeek/GetFileTime)만 쓴다 — PowerShell 이 막힌 기업 정책 PC 에서도
;   '측정 자체가 못 돌아 조용히 통과' 하는 일이 없어야 하기 때문(R2 와 동일 원칙).
;   OUT 에는 결과를 받을 레지스터를 넘긴다(예: $R1). 내부에서 $R2·$R3·$R4·$R5 를 덮어쓴다.
!macro CYS_FINGERPRINT BIN TAG OUT
  StrCpy ${OUT} "ABSENT"
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_fp_done_${TAG}
  ClearErrors
  FileOpen $R5 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_fp_done_${TAG}                  ; 열 수 없으면 ABSENT 로 둔다(= 비교에서 불일치 처리)
  FileSeek $R5 0 END $R4
  FileClose $R5
  StrCpy $R2 "x"                               ; GetFileTime 이 실패해도 직전 값이 새지 않게 선초기화
  StrCpy $R3 "x"
  ClearErrors
  GetFileTime "$INSTDIR\${BIN}.exe" $R2 $R3
  StrCpy ${OUT} "$R4/$R2/$R3"
cys_fp_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 추출 직전 지문을 $PLUGINSDIR 에 적어둔다 (R4 · PREINSTALL 전용) ────
;   $PLUGINSDIR 를 쓰는 이유: 설치기 프로세스 1회 실행 동안만 살고 종료 시 자동 삭제된다
;   (사용자 폴더에 잔해를 남기지 않는다). 레지스터로 넘기지 않는 이유: PREINSTALL 과
;   POSTINSTALL 사이에 템플릿 코드(CheckIfAppIsRunning·File·GetSize·바로가기 함수)가 돌아
;   $R* 생존을 보장할 수 없다.
!macro CYS_SNAPSHOT_PRE BIN TAG
  !insertmacro CYS_FINGERPRINT "${BIN}" "pre${TAG}" $R1
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\cys-pre-${BIN}.txt" w
  IfErrors cys_snap_done_${TAG}
  FileWrite $R0 "$R1"                          ; 개행 없이 딱 지문만 — POSTINSTALL 은 통째로 StrCmp
  FileClose $R0
cys_snap_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 신본 반영 판정 (R4 · POSTINSTALL 전용) ────────────────────────────
;   $R9 = "same"(같은 버전 재설치/증거 없음)이면 판정을 면제한다.
;   결과 누적: $R8 = 신본 미반영 목록.
!macro CYS_ENSURE_FRESH BIN TAG
  StrCmp $R9 "same" cys_fr_done_${TAG}
  !insertmacro CYS_FINGERPRINT "${BIN}" "post${TAG}" $R1
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\cys-pre-${BIN}.txt" r
  IfErrors cys_fr_unver_${TAG}
  FileRead $R0 $R2
  FileClose $R0
  StrCmp $R2 "" cys_fr_unver_${TAG}
  StrCmp $R2 $R1 0 cys_fr_done_${TAG}          ; 지문이 달라졌다 = 이번 추출이 썼다(신본) → 통과
  StrCpy $R8 "$R8 ${BIN}.exe(not-updated)"     ; 그대로다 = 추출이 못 건드렸다(구본 잔존)
  Goto cys_fr_done_${TAG}
cys_fr_unver_${TAG}:
  ; 추출 직전 지문을 못 남겼거나 못 읽었다 = '신본이 들어왔다'를 증명할 수 없다.
  ; 측정 불능을 통과로 취급하지 않는다(fail-closed).
  StrCpy $R8 "$R8 ${BIN}.exe(unverified)"
cys_fr_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 설치 종료 시 필수 실행물 1개 검증 + 자동 원복 (R2) ────────────────
;   결과 누적: $R6 = 복구 불가 목록 / $R7 = 원복은 됐으나 신본 미반영 목록.
;   두 목록 중 하나라도 비지 않으면 POSTINSTALL 이 크게 실패시킨다.
;   ★판정(존재·크기)은 순수 NSIS 명령(FileOpen/FileSeek/IntCmp)만 쓴다 — PowerShell 이 막힌
;   기업 정책 PC 에서도 '검증 자체가 못 돌아 조용히 통과'하는 일이 없게 한다. 복구 행위(copy)만
;   cmd 를 쓰며, 그 성공 여부도 다시 순수 NSIS 로 재검사한다(recheck).
!macro CYS_ENSURE_CRITICAL BIN TAG
  ClearErrors
  FileOpen $R5 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ens_bad_${TAG}
  FileSeek $R5 0 END $R4
  FileClose $R5
  ; 64KB 미만 = 추출 중단·절단·격리 잔해. 정상 Rust 바이너리는 수 MB 라 오탐 여지가 없다.
  IntCmp $R4 65536 cys_ens_done_${TAG} cys_ens_bad_${TAG} cys_ens_done_${TAG}

cys_ens_bad_${TAG}:
  ; 원복: prev → prev2 → prev3 중 첫 실재본을 정식 이름으로 되돌린다.
  ; ★copy 가 실패하면(디스크 부족·부분 복사) **rename 으로 승격**한다: 여기서의 최우선 목표는
  ;   "사용자 기계에 동작하는 실행물이 정식 이름으로 있다" 이고, Rename 은 메타데이터 연산이라
  ;   공간 부족으로 실패하지 않는다. 대가로 prev 슬롯을 소모하지만(사본이 아니라 이동),
  ;   그 파일로 실행 중인 lame-duck 프로세스는 열린 핸들을 그대로 따라가므로 죽지 않는다.
  ;   Delete 를 먼저 하는 이유: Rename 은 대상 파일이 존재하면 실패한다(잘린 사본 제거).
  IfFileExists "$INSTDIR\${BIN}.prev.exe" 0 cys_ens_p2_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R5
  StrCmp $R5 "0" cys_ens_recheck_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"
  Goto cys_ens_recheck_${TAG}
cys_ens_p2_${TAG}:
  IfFileExists "$INSTDIR\${BIN}.prev2.exe" 0 cys_ens_p3_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R5
  StrCmp $R5 "0" cys_ens_recheck_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"
  Goto cys_ens_recheck_${TAG}
cys_ens_p3_${TAG}:
  IfFileExists "$INSTDIR\${BIN}.prev3.exe" 0 cys_ens_fatal_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"'
  Pop $R5
  StrCmp $R5 "0" cys_ens_recheck_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  Rename "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"

cys_ens_recheck_${TAG}:
  ClearErrors
  FileOpen $R5 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ens_fatal_${TAG}
  FileSeek $R5 0 END $R4
  FileClose $R5
  IntCmp $R4 65536 0 cys_ens_fatal_${TAG} 0
  ; 원복 성공 = 기계는 살았지만 업그레이드는 반영되지 않았다. 성공으로 위장하지 않는다.
  StrCpy $R7 "$R7 ${BIN}.exe"
  Goto cys_ens_done_${TAG}

cys_ens_fatal_${TAG}:
  StrCpy $R6 "$R6 ${BIN}.exe"

cys_ens_done_${TAG}:
  ClearErrors
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; ① GUI만 종료(세션은 데몬 소유 — 무손실). updater 경로면 이미 종료 중이라 멱등. (L1)
  nsExec::Exec 'taskkill /F /T /IM cys-app.exe'
  Pop $R0

  ; ★잠금 스윕 일반화(2026-07-02 실장애: msys-2.0.dll Can't write → Installation Aborted).
  ; 라이브 세션 셸(claude의 bash 훅 등)이 로드한 runtime 이미지(.exe/.dll)는 '덮어쓰기'가 잠기지만
  ; 'rename'은 허용된다(로드된 PE 이미지의 Windows 특성 — rename-swap과 동일 원리의 전수 일반화).
  ; 잠긴 이미지 파일만 <이름>.prev<rand>로 밀어 이름을 비운다 → 추출이 전부 성공.
  ; 잔해(*.prev*)는 새 cysd 기동이 재귀 청소(P1b·L6). 스크립트는 $PLUGINSDIR에 생성(따옴표 지옥 회피).
  ; ★사정거리 축소(L4)+핵심 3종 제외(L5): 설치 루트는 기본 데몬 state dir 와 같은 폴더라
  ;   재귀 스윕이 세션 DB·로그 수천 개를 훑는다. runtime\ 재귀 + 루트 최상위 1단만 본다.
  ;   그리고 cys/cysd/cys-app 은 아래 rename-copy 3단이 전담하므로 스윕이 절대 만지지 않는다
  ;   (만지면 정식 자리를 비워 R1 이 무너진다 — 이번 실사고의 재발 경로).
  ; ★스윕이 rename 을 시도하려면 원본이 먼저 잠겨 있어야 하므로, 아래 rename-copy 3단보다
  ;   **먼저** 돈다(사본을 세운 뒤에 스윕하면 갓 만든 사본이 스윕 대상이 될 수 있다).
  FileOpen $R0 "$PLUGINSDIR\unlock-sweep.ps1" w
  FileWrite $R0 'param([string]$$Root)$\r$\n'
  FileWrite $R0 '$$ErrorActionPreference = "SilentlyContinue"$\r$\n'
  FileWrite $R0 '$$skip = @("cys.exe","cysd.exe","cys-app.exe")$\r$\n'
  FileWrite $R0 '$$targets = @()$\r$\n'
  FileWrite $R0 '$$rt = Join-Path $$Root "runtime"$\r$\n'
  FileWrite $R0 'if (Test-Path -LiteralPath $$rt) { $$targets += Get-ChildItem -LiteralPath $$rt -Recurse -File }$\r$\n'
  FileWrite $R0 '$$targets += Get-ChildItem -LiteralPath $$Root -File$\r$\n'
  FileWrite $R0 'foreach ($$f in $$targets) {$\r$\n'
  FileWrite $R0 '  if ($$f.Extension -ne ".exe" -and $$f.Extension -ne ".dll") { continue }$\r$\n'
  FileWrite $R0 '  if ($$f.Name -like "*.prev*") { continue }$\r$\n'
  FileWrite $R0 '  if ($$skip -contains $$f.Name) { continue }$\r$\n'
  FileWrite $R0 '  try { $$s = [IO.File]::Open($$f.FullName, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $$s.Close() }$\r$\n'
  FileWrite $R0 '  catch { try { [IO.File]::Move($$f.FullName, $$f.FullName + ".prev" + (Get-Random -Maximum 99999)) } catch {} }$\r$\n'
  FileWrite $R0 '}$\r$\n'
  FileClose $R0
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\unlock-sweep.ps1" "$INSTDIR"'
  Pop $R0

  ; ② cysd.exe·cys.exe 는 죽이지 않고 rename → 즉시 사본으로 자리 복원 (R1·L2)
  !insertmacro CYS_SWAP_IN_PLACE "cysd" "cysd"
  !insertmacro CYS_SWAP_IN_PLACE "cys" "cys"

  ; ③ ★추출 직전 지문 스냅샷 (R4) — 반드시 위 스테이징 **뒤**, File 추출 **앞** 이어야 한다.
  ;    여기서 재는 값이 "추출이 손대기 직전의 상태" 이고, POSTINSTALL 이 같은 자를 다시 대어
  ;    '이번 실행이 이 파일을 썼는가' 를 결정론으로 판정한다.
  !insertmacro CYS_SNAPSHOT_PRE "cys" "cys"
  !insertmacro CYS_SNAPSHOT_PRE "cysd" "cysd"
  !insertmacro CYS_SNAPSHOT_PRE "cys-app" "cysapp"

  ; ④ ★직전 설치 버전 증거 수집 (R4 면제 판정 · 같은 버전 재설치 오탐 차단).
  ;    출처 ① 레지스트리 DisplayVersion — 템플릿은 이 값을 **추출 뒤에** 덮어쓰므로,
  ;             '직전 설치 버전' 을 볼 수 있는 시점은 PREINSTALL 뿐이다.
  ;    출처 ② 마커 파일 — 지난 설치가 **신선도 게이트를 통과한 뒤에만** 남긴 기록이라
  ;             레지스트리보다 진실에 가깝다(중단·실패한 설치는 마커를 갱신하지 않는다).
  ;    판정: 어느 출처든 이번 ${VERSION} 과 같으면 "same"(면제) · 둘 다 비어 '증거 없음' 이어도
  ;          "same"(고발하지 않는다) · 구체적 구버전이 하나라도 있으면 "diff"(신본 반영 강제).
  StrCpy $R1 ""
  StrCpy $R2 ""
  !ifdef UNINSTKEY
    ReadRegStr $R1 SHCTX "${UNINSTKEY}" "DisplayVersion"
  !else
    ReadRegStr $R1 SHCTX "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "DisplayVersion"
  !endif
  ClearErrors
  FileOpen $R3 "$INSTDIR\cys-installed-version.txt" r
  IfErrors cys_pre_ver_judge
  FileRead $R3 $R2
  FileClose $R3
cys_pre_ver_judge:
  ClearErrors
  StrCpy $R4 "same"
  StrCmp $R1 "${VERSION}" cys_pre_ver_write 0
  StrCmp $R2 "${VERSION}" cys_pre_ver_write 0
  StrCmp $R1 "" 0 cys_pre_ver_diff
  StrCmp $R2 "" cys_pre_ver_write 0
cys_pre_ver_diff:
  StrCpy $R4 "diff"
cys_pre_ver_write:
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\cys-pre-version.txt" w
  IfErrors cys_pre_ver_done
  FileWrite $R0 "$R4"
  FileClose $R0
cys_pre_ver_done:
  ClearErrors

  ; 벨트-앤-브레이스: 스윕이 못 민 잔여 잠금 파일은 스킵하고 설치를 계속한다(Abort 금지).
  ; runtime은 버전 핀 고정(PortableGit·Python·uv·node)이라 스킵=동일 내용이 사실상 전부다.
  ; ★단, 이 침묵은 runtime 잔여물에만 허용된다 — 필수 실행물 3종의 침묵은 POSTINSTALL 이 깨뜨린다(R2).
  SetOverwrite try
  ClearErrors                                 ; 훅 종료 시 에러 플래그 잔류로 설치기 오판 방지
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; ★필수 실행물 검증 게이트 (R2 · 2026-08-01). 이 시점에는 모든 File 추출이 끝나 있다.
  ; 여기까지 도달하지 못하는 중단(CheckIfAppIsRunning Abort·사용자 취소 등)은 R1 이 이미
  ; '구버전 동작본이 정식 자리에 있는 상태'로 만들어 두었으므로 소실이 아니다.
  StrCpy $R6 ""     ; 복구 불가(존재하지 않거나 잘렸고 원복도 실패)
  StrCpy $R7 ""     ; 구본으로 원복됨(기계는 살았으나 업그레이드 미반영)
  StrCpy $R8 ""     ; 신본 미반영(파일은 멀쩡하나 이번 추출이 못 들어왔다 — R4)
  !insertmacro CYS_ENSURE_CRITICAL "cys" "cys"
  !insertmacro CYS_ENSURE_CRITICAL "cysd" "cysd"
  !insertmacro CYS_ENSURE_CRITICAL "cys-app" "cysapp"

  StrCmp $R6 "" 0 cys_post_fail
  StrCmp $R7 "" 0 cys_post_fail

  ; ★신선도 게이트 (R4) — 3종이 '존재·유효' 로 확인된 뒤에만 "그런데 그게 신본인가?" 를 묻는다.
  ; (원복이 일어났다면 지문이 당연히 바뀌므로, 그 경우는 위에서 이미 실패로 갈린다.)
  ; 면제 판정($R9)은 PREINSTALL 이 적어둔 결론을 그대로 읽는다 — 직전 설치 버전은 이 시점에
  ; 이미 신 버전으로 덮여 있어 여기서 다시 계산할 수 없다.
  StrCpy $R9 "diff"
  ClearErrors
  FileOpen $R5 "$PLUGINSDIR\cys-pre-version.txt" r
  IfErrors cys_post_verdone
  FileRead $R5 $R4
  FileClose $R5
  StrCmp $R4 "same" 0 cys_post_verdone
  StrCpy $R9 "same"
cys_post_verdone:
  ClearErrors
  !insertmacro CYS_ENSURE_FRESH "cys" "cys"
  !insertmacro CYS_ENSURE_FRESH "cysd" "cysd"
  !insertmacro CYS_ENSURE_FRESH "cys-app" "cysapp"
  StrCmp $R8 "" cys_post_ok cys_post_fail

cys_post_fail:
  ; 큰 소리로 중단한다 — 조용한 반쪽 설치(신 runtime + 구 CLI)는 이번 사고의 본질이었다.
  ; 흔적을 파일로도 남긴다(무인/silent 설치의 유일한 사후 판독원).
  ; 종료코드: 3 = 실행물이 없거나 구본으로 원복됨 / 4 = 파일은 멀쩡하나 신본 미반영(구본 잔존).
  StrCpy $R3 3
  StrCmp $R6 "" 0 cys_post_lvl
  StrCmp $R7 "" 0 cys_post_lvl
  StrCpy $R3 4
cys_post_lvl:
  FileOpen $R4 "$INSTDIR\cys-install-failure.txt" w
  FileWrite $R4 "cys installer: critical executable verification FAILED (exit $R3)$\r$\n"
  FileWrite $R4 "unrecoverable:$R6$\r$\n"
  FileWrite $R4 "rolled-back-to-previous:$R7$\r$\n"
  FileWrite $R4 "not-updated:$R8$\r$\n"
  FileWrite $R4 "Action: re-run the cys installer. Existing sessions were not killed.$\r$\n"
  FileWrite $R4 "Note: 'not-updated' means the old build is still in place and works;$\r$\n"
  FileWrite $R4 "      the new files could not be written (file in use / blocked).$\r$\n"
  FileClose $R4
  DetailPrint "cys: FATAL - executable verification failed.  unrecoverable:$R6  rolled-back:$R7  not-updated:$R8"
  ; 콘솔이 붙어 있으면(silent/CI) 표준출력으로도 외친다 — 템플릿 자체의 실패 통보와 동일 패턴.
  System::Call 'kernel32::AttachConsole(i -1)i.r0'
  StrCmp $0 0 cys_post_nocon 0
  System::Call 'kernel32::GetStdHandle(i -11)i.r0'
  FileWrite $0 "cys installer FAILED: executable verification.  unrecoverable:$R6  rolled-back:$R7  not-updated:$R8$\r$\n"
cys_post_nocon:
  IfSilent cys_post_abort 0
  MessageBox MB_ICONSTOP "cys installation did not complete correctly.$\r$\n$\r$\nMissing:$R6$\r$\nRolled back to previous:$R7$\r$\nNot updated (old build still in place):$R8$\r$\n$\r$\nYour existing cys still works and no sessions were killed.$\r$\nSee cys-install-failure.txt in the install folder, then run the installer again."
cys_post_abort:
  ; SetErrorLevel 은 리터럴로만 쓴다(변수 인자 지원 여부를 mac 개발기에서 컴파일로 확인할 수
  ; 없으므로, 검증 못 한 문법에 릴리스를 걸지 않는다 — 분기 4줄이 그 불확실성보다 싸다).
  StrCmp $R3 "4" 0 cys_post_lvl3
  SetErrorLevel 4
  Goto cys_post_end
cys_post_lvl3:
  SetErrorLevel 3
cys_post_end:
  Abort "cys installation failed: critical executable verification"

cys_post_ok:
  Delete "$INSTDIR\cys-install-failure.txt"   ; 지난 실패 흔적 청소(성공 시에만)
  ; ★버전 마커 — **모든 게이트를 통과한 뒤에만** 쓴다. 다음 설치가 '직전에 실제로 반영된 버전'
  ; 을 알아 같은 버전 재설치를 면제 판정할 수 있게 하는 근거이며(R4), 레지스트리와 달리
  ; 실패한 설치로는 갱신되지 않는다(중단 시 옛 값 그대로 = 진실).
  ClearErrors
  FileOpen $R4 "$INSTDIR\cys-installed-version.txt" w
  IfErrors cys_post_nomarker
  FileWrite $R4 "${VERSION}"
  FileClose $R4
cys_post_nomarker:
  ClearErrors

  ; 바탕화면 바로가기 자동 생성(2026-07-05 박사님 지시). Tauri NSIS 템플릿은 일반 GUI 설치에서
  ; 마침 페이지 체크박스(사용자 클릭)에만 의존해 자동 생성이 아니다 — 템플릿 내장 함수를 직접 호출해
  ; 설치 완료 시 항상 생성한다. 함수 내부 가드를 그대로 재사용: 업데이트(/UPDATE)·무바로가기(/NS)
  ; 모드는 스킵, 기존 .lnk는 타겟만 갱신(멱등 — silent/passive의 템플릿 자체 호출과 중복돼도 무해).
  ; 제거는 템플릿 uninstaller가 "$DESKTOP\<제품명>.lnk"를 지우므로 별도 처리 불요.
  Call CreateOrUpdateDesktopShortcut
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 제거(uninstall)는 의도적 전면 종료 — 세션 보존 대상이 아니다. lame-duck(.prev*)까지 정리.
  nsExec::Exec 'taskkill /F /T /IM cys-app.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cysd.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cys.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cysd.prev.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cysd.prev2.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cysd.prev3.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cys.prev.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cys.prev2.exe'
  Pop $R0
  nsExec::Exec 'taskkill /F /T /IM cys.prev3.exe'
  Pop $R0
  Sleep 1000
  Delete "$INSTDIR\cysd.prev.exe"
  Delete "$INSTDIR\cysd.prev2.exe"
  Delete "$INSTDIR\cysd.prev3.exe"
  Delete "$INSTDIR\cys.prev.exe"
  Delete "$INSTDIR\cys.prev2.exe"
  Delete "$INSTDIR\cys.prev3.exe"
  Delete "$INSTDIR\cys-install-failure.txt"
  Delete "$INSTDIR\cys-installed-version.txt"   ; R4 버전 마커(언인스톨러 미추적 파일)
  ; 잠금 스윕 잔해(*.prev<rand> — 언인스톨러 미추적 파일)까지 정리해 빈 폴더 잔존을 막는다.
  ; 프로세스는 위에서 전부 종료됐으므로 삭제 가능. runtime은 전량 우리 소유 트리다.
  RMDir /r "$INSTDIR\runtime"
  ClearErrors
!macroend
