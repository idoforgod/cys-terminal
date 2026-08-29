; cys NSIS 설치 훅 — 업그레이드/재설치 시 잠긴 exe("Error opening file for writing") 문제를
; ★잠금 무관 배치(lock-tolerant placement · 2026-08-29 W4 재설계)로 푼다.
;
; 역사: 종전 kill(cysd)은 마스터·워커·부서 PTY가 전부 cysd 소유라 "업데이트 = 전 세션 사망"
; 이었고(2026-07-02 rename-swap 도입 사유), rename-swap + 지문(pre/post) 비교는 0.14.27 에서
; "같은 버전 재설치 → 영원한 exit 4 루프" 와 T3("CLI 소실") 사고 계열을 낳았다
; (실측 감사: _work/win-installer-verify-fail-20260828/AUDIT-2ND-PASS-2026-08-28.md).
;
; ══════════════════════════════════════════════════════════════════════════════
; 이 파일의 계약 (W4 · 2026-08-29 · IMPL-SPEC §W4)
; ══════════════════════════════════════════════════════════════════════════════
;  R1. **정식 이름(`$INSTDIR\cys.exe`·`cysd.exe`·`cys-app.exe`)이 비어 있는 종단을 만들지
;      않는다.** 데몬 부팅 스윕(cysd P1b)은 정식 부재를 T3 사고로 취급하므로, 어떤 분기에서
;      끊겨도 정식 자리에는 '동작하는 실행물'(구본 또는 신본)이 있어야 한다.
;      PREINSTALL 은 정식 3종을 아예 건드리지 않고, POSTINSTALL 배치는 rename 2회(비우기→
;      채우기) 사이의 모든 실패에서 되돌리며, 콜백(.onInstFailed/.onUserAbort)과 최종
;      바닥 점검(CYS_LASTDITCH)이 잔여 경로를 받친다.
;  R2. **신선도 판정은 절대 오라클로 한다** (P1-3B). "지문이 바뀌었나"가 아니라 "정식 자리
;      파일의 VERSIONINFO DWORD 2개가 이번 빌드 원본의 것과 일치하나"를 묻는다. 기대값은
;      컴파일 시 `!getdllversion /packed` 로 **같은 원본 파일**에서 뽑는다(형식 불일치 불가).
;      런타임 GetDLLVersion 실패(리소스 없음·열기 실패)는 **fail-closed**(신본 아님으로 간주).
;      같은 버전 재설치는 오라클이 "이미 신본"으로 단락하므로 면제 기계가 필요 없다.
;  R3. **배치는 3종에 걸쳐 트랜잭션이다.** 순서는 cys → cysd → cys-app 고정(cys 를 먼저:
;      "새 cys + 구 cysd" 는 지원되는 lame-duck 상태지만 "새 cysd + 구 cys" 는 다음 부팅의
;      phoenix 정체 exit 6 = 전 pane 복원 불가). 어느 하나가 거부되면 이미 배치한 앞선 것을
;      역순으로 되돌리고, 되돌리기가 실패하면 **거기서 멈춰** 신본 집합이 항상 (cys, cysd,
;      cys-app) 의 접두(prefix)로 남게 한다.
;      ★스코프(정직 · R1 라운드1 정정): 이 prefix 성질은 **훅이 배치한 것**에만 미친다.
;      비잠금 정식은 POSTINSTALL **이전에** 템플릿 Section 의 File 추출이 제자리에서 신본으로
;      덮는다(구 바이트 소멸 · 오라클은 '이미 신본' 단락 · 트랜잭션 밖 = undo 재료 없음).
;      따라서 "cys 만 vacate 거부(share-delete 차단 핸들) + cysd/cys-app 비잠금 추출"이면
;      exit 4 에서도 구 cys + 신 cysd/cys-app 세대 분할이 도달 가능하다 — 항상 유음
;      (placement-refused 토큰)이고 잠금 해제 후 재실행이 치유한다(NSIS-CONTRACT §9-5).
;  R4. **모든 rename 은 유한 재시도(5회 × Sleep 1000)** 이고 모든 실패는 유음(loud)이다:
;      DetailPrint + cys-install-failure.txt 토큰 + SetErrorLevel + Abort. 측정 불능은 통과가
;      아니다. 성공 위장 금지.
;
; ── 잠금 규약 (업그레이드 중 데몬·CLI 잠금 처리 · 이 절이 SOT) ────────────────
;   L1. 죽여도 되는 것은 GUI(cys-app.exe) 뿐이며 **/T(트리) 없이** 죽인다 — GUI 가 cysd 를
;       평범한 자식으로 스폰하므로 트리 kill 은 데몬·전 세션 사망이다(0.14.27 실측 결함).
;       cysd.exe·cys.exe 를 죽이는 코드는 설치 경로에 존재하지 않는다(제거(uninstall)만 예외).
;   L2. 잠긴 실행 이미지는 '덮어쓰기' 불가·'rename' 가능이다(NTFS 동일 볼륨). 교체는 항상
;       신본을 옆 이름(`<bin>.new.exe`)에 추출→검증한 뒤, 정식을 prev 슬롯으로 rename 하고
;       `.new` 를 정식으로 rename 하는 2단으로 한다. 정식 부재 창은 rename 2회 사이의 수 ms 뿐.
;   L3. prev 슬롯은 prev·prev2·prev3 + tick 슬롯(`<bin>.prev<GetTickCount>.exe` · 음수 방지
;       마스킹 · 숫자만)이다. 이름들은 cysd 부팅 스윕의 잔해 문법(`is_update_leftover`) 안에
;       있어 데몬이 자가청소한다. `<bin>.new.exe` 는 그 문법 **밖**이다(설치 중 데몬 재부팅이
;       신본을 격리하지 못하게 — cysd 쪽 가드가 정식 부재 시 복구 재료로 쓴다).
;   L4. 잠금 스윕(unlock-sweep)은 `runtime\` 재귀 + 설치 루트 최상위 1단만 본다(설치 루트는
;       기본 데몬 state dir 와 같은 폴더 — 재귀 스윕은 세션 DB·로그 수천 개를 훑는 사고 위험).
;   L5. 스윕은 정식 3종을 절대 만지지 않는다(배치가 전담). 스윕 필터는 실행 이미지 확장자
;       4종(.exe/.dll/.pyd/.node)이고 결과를 `cys-sweep: scanned=N renamed=M` 로 로그에 남긴다.
;   L6. 잔해(.prev*) 삭제 책임은 설치기가 아니라 새 cysd 기동 자가청소다. 설치 성공 시의
;       슬롯 정리는 best-effort 이며 실패를 오류로 취급하지 않는다.
;
; ── 컴파일 시점 계약 (실측: tauri-cli v2.11.4 installer.nsi) ──────────────────
;   훅은 템플릿의 `!define` 들(VERSION:42·MAINBINARYSRCPATH:53·UNINSTKEY:66)보다 **먼저**
;   include 된다(:34-35). 따라서 템플릿 심볼은 매크로 본문 안(= !insertmacro 시점 :642/:734/
;   :779 에 평가)에서만 쓰고, 훅 최상단과 콜백 함수(.onInstFailed/.onUserAbort — 템플릿은
;   이 둘을 정의하지 않음을 실측 확인)에서는 런타임 변수와 훅 자체 define 만 쓴다.
;   사이드카 원본은 `${__FILEDIR__}\binaries\` 로 훅이 직접 가리킨다(번들러가 훅 경로를
;   canonicalize 하므로 __FILEDIR__ = src-tauri). 원본 부재·VERSIONINFO 부재는 **빌드 실패**
;   다(/noerrors 금지 — 무음 실패 금지).
;   이 파일은 BOM 없는 UTF-8 + CRLF 이며(.gitattributes:70), 번들러가 makensis 를
;   `-INPUTCHARSET UTF8` 로 부른다. 사용자에게 보이는 문자열(MessageBox·DetailPrint·
;   FileWrite·콘솔)은 전부 ASCII 로 적는다 — 깨진 글자로 경고하면 경고가 아니다.
;
; ── 레지스터 지도 (POSTINSTALL 트랜잭션 스코프) ───────────────────────────────
;   $R2 오라클 결과("fresh"/"notfresh"/"absent") · $R3 배치/undo 상태("ok"/"fail"/"undofail")
;   $R4 placement-refused 목록 · $R6 unrecoverable 목록 · $R7 rolled-back 목록
;   $R8 not-updated 목록 · $R5/$R9 스크래치(크기·버전·핸들) · $R1 tick(★.R1 — .r1 은 $1 이다)
;   $5/$6/$7 = cys/cysd/cys-app 의 구본이 쉬고 있는 prev 슬롯 접미(빈 값 = 구본 미대피)
;   $8 cmd copy 종료코드 · $9 rename 재시도 카운터 · $3 실패 exit 레벨 · $4 파일 핸들
;   $0 콘솔 핸들(실패 보고). 콜백 함수는 레지스터를 쓰지 않는다(순수 파일 연산).
; ══════════════════════════════════════════════════════════════════════════════

; ── 컴파일 상수: 사이드카 원본 경로 + 기대 버전 DWORD (R2 오라클 기대값) ──────
;   ★훅 최상단이므로 템플릿 define 사용 금지(위 '컴파일 시점 계약'). cys-app 의 원본
;   (${MAINBINARYSRCPATH})은 매크로 본문(NSIS_HOOK_POSTINSTALL)에서 같은 방식으로 뽑는다.
!define CYS_HOOK_DIR "${__FILEDIR__}"
!define CYS_SRC_CYS  "${CYS_HOOK_DIR}\binaries\cys-x86_64-pc-windows-msvc.exe"
!define CYS_SRC_CYSD "${CYS_HOOK_DIR}\binaries\cysd-x86_64-pc-windows-msvc.exe"
!if ! /FileExists "${CYS_SRC_CYS}"
  !error "cys hook: sidecar source not found at compile time: ${CYS_SRC_CYS} (run scripts/bundle-prep.sh first)"
!endif
!if ! /FileExists "${CYS_SRC_CYSD}"
  !error "cys hook: sidecar source not found at compile time: ${CYS_SRC_CYSD} (run scripts/bundle-prep.sh first)"
!endif
; VERSIONINFO 없는 사이드카는 여기서 빌드가 깨진다(/noerrors 금지 — 전 기계 exit 4 회귀 차단).
!getdllversion /packed "${CYS_SRC_CYS}"  CYS_V_CYS_
!getdllversion /packed "${CYS_SRC_CYSD}" CYS_V_CYSD_
; ★실측(makensis v3.12 POSIX): 리소스가 없어도 !getdllversion 이 에러 대신 **빈 값**을
;   정의하고 지나갈 수 있다(하네스 negative control 로 확인). 빈/0 기대값은 오라클 실명
;   (blind oracle)이므로 여기서 빌드를 끊는다 — I6 계약의 기계적 집행.
!if "${CYS_V_CYS_High}" == ""
  !error "cys hook: no VERSIONINFO readable in ${CYS_SRC_CYS} - refusing to build a blind oracle"
!endif
!if "${CYS_V_CYSD_High}" == ""
  !error "cys hook: no VERSIONINFO readable in ${CYS_SRC_CYSD} - refusing to build a blind oracle"
!endif
!if "${CYS_V_CYS_High}" == "0"
!if "${CYS_V_CYS_Low}" == "0"
  !error "cys hook: ${CYS_SRC_CYS} is stamped 0.0.0.0 - version resource regression"
!endif
!endif
!if "${CYS_V_CYSD_High}" == "0"
!if "${CYS_V_CYSD_Low}" == "0"
  !error "cys hook: ${CYS_SRC_CYSD} is stamped 0.0.0.0 - version resource regression"
!endif
!endif

; ── 매크로: rename 유한 재시도 (R4) — 5회 × Sleep 1000 ────────────────────────
;   성공 = 에러 플래그 clear · 최종 실패 = 에러 플래그 set. $9 를 카운터로 쓴다.
;   AV·백업·인덱서가 FILE_SHARE_DELETE 없는 핸들을 잠깐 쥐는 경우를 흡수한다(유계).
!macro CYS_RENAME_RETRY SRC DST TAG
  StrCpy $9 0
cys_rr_try_${TAG}:
  ClearErrors
  Rename "${SRC}" "${DST}"
  IfErrors 0 cys_rr_done_${TAG}
  IntOp $9 $9 + 1
  IntCmp $9 5 cys_rr_fail_${TAG} 0 cys_rr_fail_${TAG}
  Sleep 1000
  Goto cys_rr_try_${TAG}
cys_rr_fail_${TAG}:
  SetErrors
cys_rr_done_${TAG}:
!macroend

; ── 매크로: 절대 신선도 오라클 (R2 · 런타임) ──────────────────────────────────
;   OUT := "fresh"  = 존재 + 크기 ≥ 64KiB + GetDLLVersion 두 DWORD == 기대값(이번 빌드)
;          "notfresh" = 존재하나 위 판정 미달(버전 불일치·리소스 없음·열기 실패 = fail-closed)
;          "absent" = 정식 이름 부재
;   ★GetDLLVersion 은 DWORD 2개를 준다 — 문자열 버전과 비교하지 않는다(형식 불일치 = 전
;     기계 오탐). 기대값은 컴파일 시 같은 원본에서 !getdllversion /packed 로 뽑은 상수다.
;   $R9(상위)·$R5(하위/크기) 를 덮어쓴다.
!macro CYS_ORACLE BIN TAG EXPHIGH EXPLOW OUT
  StrCpy ${OUT} "absent"
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_or_done_${TAG}
  StrCpy ${OUT} "notfresh"
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_or_done_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 0 cys_or_done_${TAG} 0
  ClearErrors
  GetDLLVersion "$INSTDIR\${BIN}.exe" $R9 $R5
  IfErrors cys_or_done_${TAG}
  IntCmp $R9 ${EXPHIGH} 0 cys_or_done_${TAG} cys_or_done_${TAG}
  IntCmp $R5 ${EXPLOW} 0 cys_or_done_${TAG} cys_or_done_${TAG}
  StrCpy ${OUT} "fresh"
cys_or_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: prev 슬롯의 구본을 정식 이름으로 복귀 (배치 실패의 undo 코어) ─────
;   전제: 정식 이름이 현재 비어 있다. 성공 = 에러 플래그 clear / 실패 = set.
;   ${SLOTVAR}(레지스터)는 성공·실패와 무관하게 비운다 — 이후 단계(트랜잭션 undo·슬롯
;   정리)가 이 슬롯을 다시 만지지 않게 하기 위해서다(최종 복구는 CYS_LASTDITCH 가 슬롯
;   변수 없이 prev 체인을 직접 훑는다).
;   rename 이 5회 전부 거부되면(공유 위반) copy 로 강등한다 — 실행 중 이미지도 읽기는
;   허용되므로 copy 는 성립한다. copy 부분 실패(디스크 부족)는 잘린 사본을 지우고 마지막
;   rename 을 한 번 더 시도한다(rename 은 메타데이터 연산이라 공간 부족으로 실패하지 않는다).
!macro CYS_RESTORE_SLOT BIN TAG SLOTVAR
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.${SLOTVAR}.exe" "$INSTDIR\${BIN}.exe" "${TAG}a"
  IfErrors 0 cys_rs_ok_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.${SLOTVAR}.exe" "$INSTDIR\${BIN}.exe"'
  Pop $8
  StrCmp $8 "0" cys_rs_ok_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.${SLOTVAR}.exe" "$INSTDIR\${BIN}.exe" "${TAG}b"
  IfErrors 0 cys_rs_ok_${TAG}
  DetailPrint "cys: FATAL - could not restore ${BIN}.exe from its prev slot"
  StrCpy ${SLOTVAR} ""
  SetErrors
  Goto cys_rs_done_${TAG}
cys_rs_ok_${TAG}:
  StrCpy ${SLOTVAR} ""
  ClearErrors
cys_rs_done_${TAG}:
!macroend

; ── 매크로: 바이너리 1종의 잠금 무관 배치 (R2·R3 · IMPL-SPEC §W4-B 8단) ───────
;   결과: $R3 = "ok"(신본 반영 완료 또는 이미 신본) / "fail"(거부 — $R4 에 사유 축적,
;   정식 자리는 구본 그대로). ${SLOTVAR} = 구본이 대피한 prev 슬롯 접미(성공 시에만 —
;   트랜잭션 undo 재료로 쓰이고, 전체 성공 후에야 best-effort 정리한다).
;   어떤 분기도 정식 이름을 비운 채 끝나지 않는다(R1 — 실패는 전부 '구본 유지' 종단).
!macro CYS_PLACE BIN TAG SRC EXPHIGH EXPLOW SLOTVAR
  StrCpy ${SLOTVAR} ""
  StrCpy $R3 "ok"
  ; ① 오라클 단락: 정식이 이미 이번 빌드면 아무것도 하지 않는다(같은 버전 재설치·복구
  ;    설치·추출이 이미 성공한 경우 전부 여기서 끝 — 면제 기계 불요의 근거).
  !insertmacro CYS_ORACLE "${BIN}" "pl${TAG}" "${EXPHIGH}" "${EXPLOW}" $R2
  StrCmp $R2 "fresh" 0 cys_pl_need_${TAG}
  Delete "$INSTDIR\${BIN}.new.exe"
  ClearErrors
  Goto cys_pl_done_${TAG}
cys_pl_need_${TAG}:
  ; ② 이전 실행의 stale `.new` 는 반드시 완전 제거 — 잠겨서 못 지우면 거부한다.
  ;    (SetOverwrite try 아래에서 잠긴 stale 위로 추출이 무음 스킵되면, 뒤 단계가 그
  ;     구버전 `.new` 를 정식으로 세워 **다운그레이드**가 제자리에 들어간다 — 실측 함정.)
  ClearErrors
  Delete "$INSTDIR\${BIN}.new.exe"
  IfErrors 0 cys_pl_extract_${TAG}
  DetailPrint "cys: placement refused - stale ${BIN}.new.exe is locked"
  StrCpy $R4 "$R4 ${BIN}.exe(stale-new-locked)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_extract_${TAG}:
  ; ③ 신본을 옆 이름으로 추출한다(정식은 아직 무손상).
  ClearErrors
  File "/oname=$INSTDIR\${BIN}.new.exe" "${SRC}"
  IfErrors cys_pl_exfail_${TAG}
  ; ④ 추출물 검증: 존재 + 크기 ≥ 64KiB + 버전 == 이번 빌드 (fail-closed).
  IfFileExists "$INSTDIR\${BIN}.new.exe" 0 cys_pl_exfail_${TAG}
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.new.exe" r
  IfErrors cys_pl_newbad_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 0 cys_pl_newbad_${TAG} 0
  ClearErrors
  GetDLLVersion "$INSTDIR\${BIN}.new.exe" $R9 $R5
  IfErrors cys_pl_newbad_${TAG}
  IntCmp $R9 ${EXPHIGH} 0 cys_pl_newbad_${TAG} cys_pl_newbad_${TAG}
  IntCmp $R5 ${EXPLOW} 0 cys_pl_newbad_${TAG} cys_pl_newbad_${TAG}
  Goto cys_pl_vacate_${TAG}
cys_pl_exfail_${TAG}:
  Delete "$INSTDIR\${BIN}.new.exe"
  ClearErrors
  DetailPrint "cys: placement refused - could not extract ${BIN}.new.exe"
  StrCpy $R4 "$R4 ${BIN}.exe(extract-failed)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_newbad_${TAG}:
  Delete "$INSTDIR\${BIN}.new.exe"
  ClearErrors
  DetailPrint "cys: placement refused - ${BIN}.new.exe failed verification (size/version)"
  StrCpy $R4 "$R4 ${BIN}.exe(new-bad)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_vacate_${TAG}:
  ; ⑤ 정식(구본·잠겨 있어도 rename 은 허용)을 prev 슬롯으로 대피시킨다.
  ;    슬롯 순서: prev → prev2 → prev3 → tick. Delete 실패 = 그 슬롯을 lame-duck 이 점유
  ;    중(실행 이미지 삭제 거부) → 다음 슬롯. tick 슬롯 이름은 항상 새 이름이라 충돌이 없다.
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_pl_fill_${TAG}
  ClearErrors
  Delete "$INSTDIR\${BIN}.prev.exe"
  IfErrors cys_pl_s2_${TAG}
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev.exe" "${TAG}s1"
  IfErrors cys_pl_s2_${TAG}
  StrCpy ${SLOTVAR} "prev"
  Goto cys_pl_fill_${TAG}
cys_pl_s2_${TAG}:
  ClearErrors
  Delete "$INSTDIR\${BIN}.prev2.exe"
  IfErrors cys_pl_s3_${TAG}
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev2.exe" "${TAG}s2"
  IfErrors cys_pl_s3_${TAG}
  StrCpy ${SLOTVAR} "prev2"
  Goto cys_pl_fill_${TAG}
cys_pl_s3_${TAG}:
  ClearErrors
  Delete "$INSTDIR\${BIN}.prev3.exe"
  IfErrors cys_pl_stick_${TAG}
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev3.exe" "${TAG}s3"
  IfErrors cys_pl_stick_${TAG}
  StrCpy ${SLOTVAR} "prev3"
  Goto cys_pl_fill_${TAG}
cys_pl_stick_${TAG}:
  ; tick 슬롯 — ★System::Call 출력은 반드시 `.R1`($R1)이다(`.r1` 은 $1). GetTickCount 는
  ;   24.8일 후 음수가 되므로 마스킹해 숫자만 남긴다(음수 `-` 는 잔해 문법을 벗어난다).
  System::Call 'kernel32::GetTickCount()i.R1'
  IntOp $R1 $R1 & 0x7FFFFFFF
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.prev$R1.exe" "${TAG}s4"
  IfErrors 0 cys_pl_stickok_${TAG}
  ; 슬롯 전부 거부 — 정식은 손대지 않았다(구본 무손상). 거부하고 크게 알린다.
  DetailPrint "cys: placement refused - could not move ${BIN}.exe aside (file busy)"
  StrCpy $R4 "$R4 ${BIN}.exe(vacate-locked)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_stickok_${TAG}:
  StrCpy ${SLOTVAR} "prev$R1"
cys_pl_fill_${TAG}:
  ; ⑥ 검증된 신본을 정식 이름으로 rename(채우기). 여기서부터 정식 부재 창 — 실패는 즉시
  ;    구본 복귀로 닫는다.
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.new.exe" "$INSTDIR\${BIN}.exe" "${TAG}f"
  IfErrors 0 cys_pl_rv_${TAG}
  StrCmp ${SLOTVAR} "" cys_pl_fillfail_${TAG}
  !insertmacro CYS_RESTORE_SLOT "${BIN}" "${TAG}rf" ${SLOTVAR}
cys_pl_fillfail_${TAG}:
  DetailPrint "cys: placement refused - could not move ${BIN}.new.exe into place"
  StrCpy $R4 "$R4 ${BIN}.exe(fill-failed)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_rv_${TAG}:
  ; ⑦ 재검증 — rename 은 내용을 바꾸지 않지만, AV 격리·파일시스템 이상이 그 사이에 끼어들
  ;    수 있다. 미달이면 신본을 `.new` 로 되물리고 구본을 복귀시킨 뒤 거부한다.
  !insertmacro CYS_ORACLE "${BIN}" "rv${TAG}" "${EXPHIGH}" "${EXPLOW}" $R2
  StrCmp $R2 "fresh" cys_pl_commit_${TAG} 0
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.new.exe" "${TAG}rvv"
  IfErrors cys_pl_rvstuck_${TAG}
  StrCmp ${SLOTVAR} "" cys_pl_rvfail_${TAG}
  !insertmacro CYS_RESTORE_SLOT "${BIN}" "${TAG}rv" ${SLOTVAR}
cys_pl_rvfail_${TAG}:
  DetailPrint "cys: placement refused - ${BIN}.exe failed re-verification after placement"
  StrCpy $R4 "$R4 ${BIN}.exe(reverify-failed)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_rvstuck_${TAG}:
  ; 재검증 미달본을 비울 수도 없다 — 정식 이름에 '무언가'는 있다(비우지 않는다 — R1).
  ; 최종 바닥 점검(CYS_LASTDITCH)이 다시 판정한다.
  DetailPrint "cys: placement refused - ${BIN}.exe failed re-verification (could not roll back)"
  StrCpy $R4 "$R4 ${BIN}.exe(reverify-failed)"
  StrCpy $R3 "fail"
  Goto cys_pl_done_${TAG}
cys_pl_commit_${TAG}:
  ; ⑧ 이 바이너리는 완료. `.new` 이름은 rename 으로 이미 비었다(Delete 는 벨트). prev 슬롯은
  ;    트랜잭션 전체가 끝날 때까지 남긴다(뒤 바이너리 실패 시 undo 재료 — 여기서 지우면
  ;    트랜잭션이 성립하지 않는다).
  Delete "$INSTDIR\${BIN}.new.exe"
  ClearErrors
cys_pl_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 배치 완료된 바이너리 1종의 트랜잭션 undo (R3) ─────────────────────
;   신본(정식)을 `.new` 로 되물리고 prev 슬롯의 구본을 복귀시킨다. 실패하면 $R3="undofail"
;   — 호출측(POSTINSTALL)은 거기서 undo 를 멈춰 신본 집합의 접두(prefix) 성질을 지킨다
;   (undo 를 계속 강행하면 "구 cys + 신 cysd" 세대 분할을 스스로 만들 수 있다).
!macro CYS_UNPLACE BIN TAG SLOTVAR
  StrCmp ${SLOTVAR} "" cys_up_done_${TAG}
  ClearErrors
  Delete "$INSTDIR\${BIN}.new.exe"
  ClearErrors
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.exe" "$INSTDIR\${BIN}.new.exe" "${TAG}v"
  IfErrors cys_up_stuck_${TAG}
  !insertmacro CYS_RESTORE_SLOT "${BIN}" "${TAG}r" ${SLOTVAR}
  IfErrors cys_up_norestore_${TAG}
  StrCpy $R8 "$R8 ${BIN}.exe(not-updated)"
  Goto cys_up_done_${TAG}
cys_up_norestore_${TAG}:
  ; 구본 복귀 실패 — 정식이 지금 비어 있다. 신본이라도 되세워 자리를 닫는다(R1 우선).
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.new.exe" "$INSTDIR\${BIN}.exe" "${TAG}n"
  IfErrors 0 cys_up_keepnew_${TAG}
  DetailPrint "cys: FATAL - ${BIN}.exe rollback lost both builds (final floor check will retry)"
  StrCpy $R3 "undofail"
  Goto cys_up_done_${TAG}
cys_up_keepnew_${TAG}:
  DetailPrint "cys: WARNING - could not roll back ${BIN}.exe to previous build; new build kept"
  StrCpy $R3 "undofail"
  Goto cys_up_done_${TAG}
cys_up_stuck_${TAG}:
  DetailPrint "cys: WARNING - could not roll back ${BIN}.exe (rename busy); new build kept"
  StrCpy $R3 "undofail"
cys_up_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 트랜잭션 커밋 후 슬롯 정리 (L6 · best-effort) ─────────────────────
;   lame-duck 이 슬롯 파일로 실행 중이면 Delete 가 거부된다 — 무시한다(새 cysd 가 청소).
!macro CYS_SLOT_CLEANUP BIN TAG SLOTVAR
  StrCmp ${SLOTVAR} "" cys_sc_done_${TAG}
  Delete "$INSTDIR\${BIN}.${SLOTVAR}.exe"
  ClearErrors
cys_sc_done_${TAG}:
!macroend

; ── 매크로: 최종 바닥 점검 (R1 의 기계적 집행 · 성공/실패 경로 공통) ──────────
;   무엇이 어떻게 실패했든, 이 매크로 이후 정식 이름에는 '동작하는 실행물'이 있어야 한다.
;   미달이면 `.new`(이번 빌드 — rename) → prev·prev2·prev3(구본 — copy, 실패 시 rename 승격)
;   순으로 복구하고, 그래도 안 되면 $R6(unrecoverable · exit 3)에 기록한다. 복구가 구본으로
;   이뤄지면 $R7(rolled-back)에 기록해 성공으로 위장하지 않는다.
;   ★크기 프로브의 한계 고지(R2 라운드2 — 64KiB 는 절단의 하한이지 상한이 아니다):
;     · 진입 판정(아래 첫 블록)과 prev 복귀 재검(cys_ld_mark)은 **크기만** 본다. 더 세게 갈 수
;       없다: 정식·prev 자리에는 **구본**이 정상 거주하고, 구본(0.14.27 이하)은 VERSIONINFO 가
;       없을 수 있어 GetDLLVersion 프로브는 '건강한 구본'과 '절단본'을 못 가른다 — 여기서
;       오라클을 걸면 배타 잠금 exit-4 정경로(P6: 구본 무손상)가 rolled-back/unrecoverable 로
;       오판돼 동결 계약(§4)이 깨진다. 이 창은 cysd 부팅 가드의 **PE 구조 프로브**(재료가 걸린
;       SweepAll 직전에만 절단 검사 · main.rs probe_pe_extents)가 다음 부팅에서 받친다.
;     · `.new` 승격 갈래만은 오라클 재검이 안전하다 — `.new` 는 '이번 빌드' 주장이고 이번 빌드는
;       VERSIONINFO 를 컴파일 게이트(§5 문자열 3종)로 보증하므로, 아래에서 크기+버전을 함께 재
;       크기 통과 절단본(전원차단 tear 의 >99% 지점)의 무검 승격을 끊는다.
;   ★D11(2026-08-29): 이 `.new` 승격이 성공하고 앞선 바이너리가 구본으로 끝난 조합은 구/신
;     세대 분할을 exit-4 레인에 남길 수 있다 — 모델 전수 실측에서 분할 종단은 전부 유음
;     (exit-0 분할 0건 · 상시 핀 lane:D11-ld-split-exit4)이고 설치기 재실행이 치유하며,
;     근본 봉합(PREINSTALL `.old` 대피)은 D9-b 로 Release B 이월. NSIS-CONTRACT §9-3.
;   기대값 ${EXPHIGH}/${EXPLOW} = 이 바이너리의 컴파일 상수(호출부가 CYS_V_* 를 넘긴다).
!macro CYS_LASTDITCH BIN TAG EXPHIGH EXPLOW
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ld_bad_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 cys_ld_done_${TAG} cys_ld_bad_${TAG} cys_ld_done_${TAG}
cys_ld_bad_${TAG}:
  DetailPrint "cys: EMERGENCY - ${BIN}.exe missing or truncated; restoring"
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.new.exe" 0 cys_ld_p1_${TAG}
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.new.exe" "$INSTDIR\${BIN}.exe" "${TAG}n"
  IfErrors cys_ld_p1_${TAG}
  ; 이번 빌드로 복구됨 — 재검(크기 + ★오라클)을 통과하면 보고 목록에 올리지 않는다.
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ld_p1_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 0 cys_ld_nbad_${TAG} 0
  ; ★R2 라운드2: `.new` 는 이번 빌드 주장 — 크기 통과 절단본(tear ≥64KiB)을 거르기 위해
  ;   버전 오라클을 재검한다(구본 갈래와 달리 여기서는 VERSIONINFO 부재 = 진짜 미달이다).
  ClearErrors
  GetDLLVersion "$INSTDIR\${BIN}.exe" $R9 $R5
  IfErrors cys_ld_nsus_${TAG}
  IntCmp $R9 ${EXPHIGH} 0 cys_ld_nsus_${TAG} cys_ld_nsus_${TAG}
  IntCmp $R5 ${EXPLOW} 0 cys_ld_nsus_${TAG} cys_ld_nsus_${TAG}
  Goto cys_ld_done_${TAG}
cys_ld_nsus_${TAG}:
  ; 오라클 미달 `.new` 승격본 — **구본 재료가 남아 있을 때만** 처분하고 prev 체인을 계속한다.
  ; 빈손(잔여 prev 전무)이면 크기 통과분을 그대로 유지한다(빈손 삭제 금지 — 지우면 다음 부팅
  ; 가드가 NothingToDo 무음으로 접혀 사고가 침묵한다. CYS_ABORT_RESCUE 와 같은 계약).
  DetailPrint "cys: WARNING - restored ${BIN}.exe failed version re-check (possible torn extract)"
  IfFileExists "$INSTDIR\${BIN}.prev.exe" cys_ld_nbad_${TAG}
  IfFileExists "$INSTDIR\${BIN}.prev2.exe" cys_ld_nbad_${TAG}
  IfFileExists "$INSTDIR\${BIN}.prev3.exe" cys_ld_nbad_${TAG} cys_ld_done_${TAG}
cys_ld_nbad_${TAG}:
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
cys_ld_p1_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev.exe" 0 cys_ld_p2_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"'
  Pop $8
  StrCmp $8 "0" cys_ld_mark_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe" "${TAG}r1"
  IfErrors cys_ld_p2_${TAG} cys_ld_mark_${TAG}
cys_ld_p2_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev2.exe" 0 cys_ld_p3_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"'
  Pop $8
  StrCmp $8 "0" cys_ld_mark_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe" "${TAG}r2"
  IfErrors cys_ld_p3_${TAG} cys_ld_mark_${TAG}
cys_ld_p3_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev3.exe" 0 cys_ld_fatal_${TAG}
  nsExec::Exec 'cmd /c copy /y /b "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"'
  Pop $8
  StrCmp $8 "0" cys_ld_mark_${TAG} 0
  Delete "$INSTDIR\${BIN}.exe"
  ClearErrors
  !insertmacro CYS_RENAME_RETRY "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe" "${TAG}r3"
  IfErrors cys_ld_fatal_${TAG} cys_ld_mark_${TAG}
cys_ld_mark_${TAG}:
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ld_fatal_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 cys_ld_rolled_${TAG} cys_ld_fatal_${TAG} cys_ld_rolled_${TAG}
cys_ld_rolled_${TAG}:
  StrCpy $R7 "$R7 ${BIN}.exe"
  Goto cys_ld_done_${TAG}
cys_ld_fatal_${TAG}:
  DetailPrint "cys: FATAL - ${BIN}.exe could not be restored from any material"
  StrCpy $R6 "$R6 ${BIN}.exe"
cys_ld_done_${TAG}:
  ClearErrors
!macroend

; ── 매크로: 중단 콜백 구조 (R1 · 콜백 전용) ───────────────────────────────────
;   설치가 어느 시점에 중단되든(사용자 취소·Abort — 배치 rename 2회 사이 포함) 정식 이름을
;   비워 두지 않는다: 동작본 부재 시 `.new`(이번 빌드) → prev 체인(구본) 순 rename 승격.
;   동작본 정식이 있으면 `.new` 스테이징 잔해만 지운다(그때만 지운다 — cysd 가드와 같은 규칙).
;   ★정식은 '존재'가 아니라 **크기 프로브(≥65536 = W1 가드·LASTDITCH 와 같은 문턱)** 로 판정
;     한다(R1 라운드1 수리): 취소가 템플릿 File 의 truncate-write 도중이면 **절단 정식**이
;     남는데, 그걸 존재로 읽으면 이 갈래가 `.new`(마지막 동작본 재료)를 지운다. 절단 정식은
;     Delete 로 자리를 비운 뒤 복구 체인으로 간다(Rename 은 실재하는 dest 에 실패하므로 자리
;     비우기 없는 승격은 성립하지 않는다) · Delete 거부 시 **무접촉 종료**(재료 보존 — cysd
;     부팅 가드가 2차 복구선) · 프로브 불가(열기 거부)도 판정 불가 = 무접촉 종료.
;   ★빈손 삭제 금지(R2 라운드2 수리): 절단 정식이라도 **재료(`.new`/`.prev*`)가 하나도 없으면
;     지우지 않는다** — 재료 없이 자리만 비우면 다음 부팅 가드가 (정식無·재료無)를
;     NothingToDo(무음)로 접어 사고가 침묵한다. 절단 정식을 남겨야 Hold(매 부팅 loud —
;     "설치기를 다시 실행하라")가 발화한다. 삭제는 언제나 재료를 손에 쥔 갈래에서만 한다
;     (cysd 가드 실행 순서 계약·main.rs:150-157 과 동일 규율). prev 프로브는 와일드카드
;     (`.prev*.exe`)라 tick 슬롯까지 재료로 센다(복구 체인은 고정 3칸이지만, tick 만 남은
;     경우에도 자리를 비워 두면 다음 부팅 가드의 PromotePrev 가 tick 을 승격한다).
;   ★콜백 제약: 런타임 변수·리터럴만 사용(템플릿 define 금지 — 훅이 먼저 include 된다).
!macro CYS_ABORT_RESCUE BIN TAG
  IfFileExists "$INSTDIR\${BIN}.exe" 0 cys_ar_try_${TAG}
  ClearErrors
  FileOpen $R9 "$INSTDIR\${BIN}.exe" r
  IfErrors cys_ar_done_${TAG}
  FileSeek $R9 0 END $R5
  FileClose $R9
  IntCmp $R5 65536 cys_ar_clean_${TAG} 0 cys_ar_clean_${TAG}
  ; 절단 정식 — 재료 프로브 선행(빈손이면 무접촉 종료 · 위 ★빈손 삭제 금지).
  IfFileExists "$INSTDIR\${BIN}.new.exe" cys_ar_mat_${TAG}
  IfFileExists "$INSTDIR\${BIN}.prev*.exe" cys_ar_mat_${TAG} cys_ar_done_${TAG}
cys_ar_mat_${TAG}:
  ClearErrors
  Delete "$INSTDIR\${BIN}.exe"
  IfFileExists "$INSTDIR\${BIN}.exe" cys_ar_done_${TAG} cys_ar_try_${TAG}
cys_ar_try_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.new.exe" 0 cys_ar_p1_${TAG}
  Rename "$INSTDIR\${BIN}.new.exe" "$INSTDIR\${BIN}.exe"
  IfErrors cys_ar_p1_${TAG} cys_ar_done_${TAG}
cys_ar_p1_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev.exe" 0 cys_ar_p2_${TAG}
  Rename "$INSTDIR\${BIN}.prev.exe" "$INSTDIR\${BIN}.exe"
  IfErrors cys_ar_p2_${TAG} cys_ar_done_${TAG}
cys_ar_p2_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev2.exe" 0 cys_ar_p3_${TAG}
  Rename "$INSTDIR\${BIN}.prev2.exe" "$INSTDIR\${BIN}.exe"
  IfErrors cys_ar_p3_${TAG} cys_ar_done_${TAG}
cys_ar_p3_${TAG}:
  ClearErrors
  IfFileExists "$INSTDIR\${BIN}.prev3.exe" 0 cys_ar_done_${TAG}
  Rename "$INSTDIR\${BIN}.prev3.exe" "$INSTDIR\${BIN}.exe"
  Goto cys_ar_done_${TAG}
cys_ar_clean_${TAG}:
  Delete "$INSTDIR\${BIN}.new.exe"
cys_ar_done_${TAG}:
  ClearErrors
!macroend

; 콜백 정의 — 실측(tauri-cli v2.11.4 installer.nsi): 템플릿은 .onInstFailed/.onUserAbort 를
; 정의하지 않는다(.onInit/.onInstSuccess/un.onInit 만). 충돌 없음.
Function .onInstFailed
  !insertmacro CYS_ABORT_RESCUE "cys" "if1"
  !insertmacro CYS_ABORT_RESCUE "cysd" "if2"
  !insertmacro CYS_ABORT_RESCUE "cys-app" "if3"
FunctionEnd

; ---------------------------------------------------------------------------
; MUI2 공존 규약 (2026-08-29 실사고 수리 · CI run 33246693830)
; 템플릿은 MUI2 를 쓰고, `!insertmacro MUI_LANGUAGE`(installer.nsi:470) 가
; MUI_FUNCTION_ABORTWARNING (Contrib/Modern UI 2/Interface.nsh:327) 을 통해
; **`.onUserAbort` 를 스스로 만든다**. 훅이 같은 이름을 정의하면
; "Function named \".onUserAbort\" already exists." 로 **빌드가 죽는다**(실제 발생).
; 그래서 MUI 가 공식으로 제공하는 확장점을 쓴다 — Interface.nsh 의
; `.onUserAbort` 내부가 `Call "${MUI_CUSTOMFUNCTION_ABORT}"` 을 하므로 취소 정리가
; 그대로 돌아간다. 훅은 installer.nsi:35 에서 include 되므로 이 define 은
; MUI_LANGUAGE(470) 보다 항상 먼저다.
; ★ 이미 누가 점유했으면 조용히 정리가 사라지는 것이 더 나쁘다 → 빌드 중단.
!ifdef MUI_CUSTOMFUNCTION_ABORT
  !error "cys: MUI_CUSTOMFUNCTION_ABORT already taken - user-cancel cleanup would be silently lost"
!endif
!define MUI_CUSTOMFUNCTION_ABORT "cys_on_user_abort"
Function cys_on_user_abort
  !insertmacro CYS_ABORT_RESCUE "cys" "ua1"
  !insertmacro CYS_ABORT_RESCUE "cysd" "ua2"
  !insertmacro CYS_ABORT_RESCUE "cys-app" "ua3"
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; ⓪ ★설치기 싱글톤 (R2 라운드2 신설): 동시 2인스턴스는 `.new`/prev/정식 이름 공간을
  ;    공유해 서로의 트랜잭션을 교차 오염시킨다(실증 트레이스: A 의 fill 5초 재시도 창에서
  ;    B 가 배치를 끝내면, A 의 copy-복귀가 B 의 검증 완료 정식을 구본으로 덮어 B 의 exit 0
  ;    주장이 거짓이 된다 — R2-break-windows.log S2). 템플릿 CheckIfAppIsRunning 은 앱만
  ;    본다(설치기끼리는 무방비). Global\ 이름 뮤텍스로 인스턴스를 1개로 강제한다.
  ;    · 핸들은 의도적으로 닫지 않는다 — 프로세스 종료가 해제이며, 그 수명이 곧 잠금이다.
  ;    · 진 쪽은 **Quit**(Abort 아님)으로 나간다: Abort 는 .onInstFailed 콜백(구조 매크로)을
  ;      발화시켜, 아무것도 안 한 인스턴스가 이긴 쪽의 `.new` 스테이징을 지울 수 있다.
  ;      Quit 은 콜백 없이 즉시 종료라 부작용이 0이다(이 시점까지 파일 접촉 0).
  ;    · exit 5 = "다른 설치기 인스턴스 실행 중 · 무접촉"(NSIS-CONTRACT §4 신설 행).
  ;    · ERROR_ALREADY_EXISTS(183) 외에, 핸들 0 + ERROR_ACCESS_DENIED(5)도 '이미 존재'다
  ;      (다른 무결성 수준의 인스턴스가 선점). 그 밖의 생성 실패는 fail-open(설치 계속) —
  ;      뮤텍스는 벨트이지 게이트가 아니고, 여기서 막히면 정상 단독 설치까지 죽는다.
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Global\cys-installer") p .r0 ?e'
  Pop $1
  StrCmp $1 "183" cys_pre_dup 0
  StrCmp $1 "5" 0 cys_pre_single
  StrCmp $0 "0" cys_pre_dup cys_pre_single
cys_pre_dup:
  DetailPrint "cys: another installer instance is running - quitting untouched"
  IfSilent cys_pre_dupquit 0
  MessageBox MB_ICONSTOP "Another cys installer is already running.$\r$\nFinish that installation first, then run this one again.$\r$\nNothing was changed by this instance."
cys_pre_dupquit:
  SetErrorLevel 5
  Quit
cys_pre_single:
  ClearErrors

  ; ① GUI만 종료 — ★/T 금지(L1): GUI 가 cysd 를 평범한 자식으로 스폰하므로 트리 kill 은
  ;    데몬과 전 PTY 세션을 함께 죽인다(0.14.27 실측 결함). updater 경로면 이미 종료 중이라
  ;    멱등. 세션은 데몬 소유 — 무손실.
  nsExec::Exec 'taskkill /F /IM cys-app.exe'
  Pop $R0

  ; ★잠금 스윕 일반화(2026-07-02 실장애: msys-2.0.dll Can't write → Installation Aborted).
  ; 라이브 세션 셸(claude의 bash 훅 등)이 로드한 runtime 이미지(.exe/.dll/.pyd/.node)는
  ; '덮어쓰기'가 잠기지만 'rename'은 허용된다(로드된 PE 이미지의 Windows 특성).
  ; 잠긴 이미지 파일만 <이름>.prev<rand>로 밀어 이름을 비운다 → 추출이 전부 성공.
  ; 잔해(*.prev*)는 새 cysd 기동이 재귀 청소(P1b·L6). 스크립트는 $PLUGINSDIR에 생성(따옴표 지옥 회피).
  ; ★사정거리 축소(L4)+핵심 3종 제외(L5): 설치 루트는 기본 데몬 state dir 와 같은 폴더라
  ;   재귀 스윕이 세션 DB·로그 수천 개를 훑는다. runtime\ 재귀 + 루트 최상위 1단만 본다.
  ;   그리고 cys/cysd/cys-app 은 POSTINSTALL 의 잠금 무관 배치가 전담하므로 스윕이 절대
  ;   만지지 않는다(만지면 정식 자리를 비워 R1 이 무너진다).
  ;   결과는 `cys-sweep: scanned=N renamed=M` 한 줄로 상세로그에 남는다(CI 가 인용하는 토큰).
  FileOpen $R0 "$PLUGINSDIR\unlock-sweep.ps1" w
  FileWrite $R0 'param([string]$$Root)$\r$\n'
  FileWrite $R0 '$$ErrorActionPreference = "SilentlyContinue"$\r$\n'
  FileWrite $R0 '$$skip = @("cys.exe","cysd.exe","cys-app.exe")$\r$\n'
  FileWrite $R0 '$$exts = @(".exe",".dll",".pyd",".node")$\r$\n'
  FileWrite $R0 '$$scanned = 0$\r$\n'
  FileWrite $R0 '$$renamed = 0$\r$\n'
  FileWrite $R0 '$$targets = @()$\r$\n'
  FileWrite $R0 '$$rt = Join-Path $$Root "runtime"$\r$\n'
  FileWrite $R0 'if (Test-Path -LiteralPath $$rt) { $$targets += Get-ChildItem -LiteralPath $$rt -Recurse -File }$\r$\n'
  FileWrite $R0 '$$targets += Get-ChildItem -LiteralPath $$Root -File$\r$\n'
  FileWrite $R0 'foreach ($$f in $$targets) {$\r$\n'
  FileWrite $R0 '  if ($$exts -notcontains $$f.Extension) { continue }$\r$\n'
  FileWrite $R0 '  if ($$f.Name -like "*.prev*") { continue }$\r$\n'
  FileWrite $R0 '  if ($$skip -contains $$f.Name) { continue }$\r$\n'
  FileWrite $R0 '  $$scanned++$\r$\n'
  FileWrite $R0 '  try { $$s = [IO.File]::Open($$f.FullName, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $$s.Close() }$\r$\n'
  FileWrite $R0 '  catch { try { [IO.File]::Move($$f.FullName, $$f.FullName + ".prev" + (Get-Random -Maximum 99999)); $$renamed++ } catch {} }$\r$\n'
  FileWrite $R0 '}$\r$\n'
  FileWrite $R0 'Write-Output ("cys-sweep: scanned=" + $$scanned + " renamed=" + $$renamed)$\r$\n'
  FileClose $R0
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\unlock-sweep.ps1" "$INSTDIR"'
  Pop $R0

  ; 벨트-앤-브레이스: 스윕이 못 민 잔여 잠금 파일은 스킵하고 설치를 계속한다(Abort 금지).
  ; runtime은 버전 핀 고정(PortableGit·Python·uv·node)이라 스킵=동일 내용이 사실상 전부다.
  ; ★단, 이 침묵은 runtime 잔여물에만 허용된다 — 필수 실행물 3종은 POSTINSTALL 의 배치와
  ;   오라클이 결정론으로 판정한다(추출 성공 여부와 무관).
  SetOverwrite try
  ClearErrors                                 ; 훅 종료 시 에러 플래그 잔류로 설치기 오판 방지
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; cys-app 의 오라클 기대값 — 원본은 템플릿 define ${MAINBINARYSRCPATH}. 훅 최상단에서는
  ; 쓸 수 없고(훅이 먼저 include), 매크로 본문은 !insertmacro 시점(템플릿 Section 내부)에
  ; 평가되므로 여기서는 유효하다. VERSIONINFO 부재는 빌드 실패(/noerrors 금지).
  !ifndef CYS_V_APP_High
    !if ! /FileExists "${MAINBINARYSRCPATH}"
      !error "cys hook: main binary source not found at compile time: ${MAINBINARYSRCPATH}"
    !endif
    !getdllversion /packed "${MAINBINARYSRCPATH}" CYS_V_APP_
    !if "${CYS_V_APP_High}" == ""
      !error "cys hook: no VERSIONINFO readable in ${MAINBINARYSRCPATH} - refusing to build a blind oracle"
    !endif
    !if "${CYS_V_APP_High}" == "0"
    !if "${CYS_V_APP_Low}" == "0"
      !error "cys hook: ${MAINBINARYSRCPATH} is stamped 0.0.0.0 - version resource regression"
    !endif
    !endif
  !endif

  ; ★잠금 무관 배치 (R2·R3 · IMPL-SPEC §W4-B). 이 시점에는 모든 File 추출이 끝나 있다.
  ;   - 추출이 이미 신본을 앉혔으면(비잠금 경로) 오라클이 단락한다.
  ;   - 잠겨서 추출이 스킵됐으면 `.new` 추출→검증→rename 2회로 배치한다.
  ;   순서 고정 cys → cysd → cys-app (R3: 실패 시에도 **훅 배치분**의 세대 집합은 항상 prefix
  ;   — 비잠금 정식은 템플릿 File 이 이미 덮었을 수 있어 전체 3종의 prefix 는 아니다. 스코프와
  ;   도달 가능한 exit-4 세대 분할은 파일 머리 R3 ★스코프 주석·NSIS-CONTRACT §9-3
  ;   (LASTDITCH `.new` 승격 경로 — ★D11)·§9-5(템플릿 경로)).
  StrCpy $R4 ""     ; placement-refused (구본 무손상 · 신본 미반영 · 예외 = rvstuck 절단 창, NSIS-CONTRACT §9-3 ★D11)
  StrCpy $R6 ""     ; unrecoverable (정식 부재·절단 + 복구 전멸 — 최악)
  StrCpy $R7 ""     ; rolled-back (비상 복구로 구본 복귀 — 기계는 살았으나 미반영)
  StrCpy $R8 ""     ; not-updated (트랜잭션 undo 로 구본 복귀)
  StrCpy $5 ""
  StrCpy $6 ""
  StrCpy $7 ""
  SetOverwrite try
  !insertmacro CYS_PLACE "cys" "cys" "${CYS_SRC_CYS}" "${CYS_V_CYS_High}" "${CYS_V_CYS_Low}" $5
  StrCmp $R3 "fail" cys_txn_undo 0
  !insertmacro CYS_PLACE "cysd" "cysd" "${CYS_SRC_CYSD}" "${CYS_V_CYSD_High}" "${CYS_V_CYSD_Low}" $6
  StrCmp $R3 "fail" cys_txn_undo 0
  !insertmacro CYS_PLACE "cys-app" "cysapp" "${MAINBINARYSRCPATH}" "${CYS_V_APP_High}" "${CYS_V_APP_Low}" $7
  StrCmp $R3 "fail" cys_txn_undo 0
  Goto cys_txn_commit

cys_txn_undo:
  ; 역순 undo. "undofail" 이 뜨면 그 자리에서 멈춘다(prefix 보존 — 위 CYS_UNPLACE 주석).
  !insertmacro CYS_UNPLACE "cys-app" "ucysapp" $7
  StrCmp $R3 "undofail" cys_txn_after 0
  !insertmacro CYS_UNPLACE "cysd" "ucysd" $6
  StrCmp $R3 "undofail" cys_txn_after 0
  !insertmacro CYS_UNPLACE "cys" "ucys" $5
  Goto cys_txn_after

cys_txn_commit:
  ; 3종 전부 반영 — 이제야 undo 재료(prev 슬롯)를 놓아준다(L6 · best-effort).
  !insertmacro CYS_SLOT_CLEANUP "cys" "ccys" $5
  !insertmacro CYS_SLOT_CLEANUP "cysd" "ccysd" $6
  !insertmacro CYS_SLOT_CLEANUP "cys-app" "ccysapp" $7

cys_txn_after:
  ; ★최종 바닥 점검 (R1) — 성공·실패 어느 경로든 정식 3종이 '동작본'인지 기계로 확인한다.
  ;   기대 DWORD 는 `.new` 승격 갈래의 오라클 재검에만 쓰인다(구본 갈래는 크기 프로브 유지).
  !insertmacro CYS_LASTDITCH "cys" "ldcys" "${CYS_V_CYS_High}" "${CYS_V_CYS_Low}"
  !insertmacro CYS_LASTDITCH "cysd" "ldcysd" "${CYS_V_CYSD_High}" "${CYS_V_CYSD_Low}"
  !insertmacro CYS_LASTDITCH "cys-app" "ldcysapp" "${CYS_V_APP_High}" "${CYS_V_APP_Low}"

  StrCmp $R6 "" 0 cys_post_fail
  StrCmp $R7 "" 0 cys_post_fail
  StrCmp $R8 "" 0 cys_post_fail
  StrCmp $R4 "" 0 cys_post_fail
  Goto cys_post_ok

cys_post_fail:
  ; 큰 소리로 중단한다 — 조용한 반쪽 설치(신 runtime + 구 CLI)는 T3 사고의 본질이었다.
  ; 흔적을 파일로도 남긴다(무인/silent 설치의 유일한 사후 판독원). 토큰 스키마는 동결
  ; (NSIS-CONTRACT.md) — 소비자: CI T4-13 · 체크리스트 P5 · GUI 리더(Release B).
  ; 종료코드: 3 = 정식이 없거나 구본으로 비상 복구됨 / 4 = 구본 무손상·신본 미반영(거부).
  ;   (★D11 스코프: exit-4 라도 rvstuck 절단 창(NSIS-CONTRACT §9-3)은 정식 자리에 크기
  ;    통과 절단본을 남길 수 있다 — 항상 유음·토큰 명명, 동작본으로 시작했다면 동작
  ;    사본이 가족 이름에 잔존 = cysd 부팅 가드 재료. Note 2행은 지배-레인 안내문이다.)
  StrCpy $3 3
  StrCmp $R6 "" 0 cys_post_lvl
  StrCmp $R7 "" 0 cys_post_lvl
  StrCpy $3 4
cys_post_lvl:
  ClearErrors
  FileOpen $4 "$INSTDIR\cys-install-failure.txt" w
  IfErrors cys_post_nofile
  FileWrite $4 "cys installer: critical executable verification FAILED (exit $3)$\r$\n"
  FileWrite $4 "unrecoverable:$R6$\r$\n"
  FileWrite $4 "rolled-back-to-previous:$R7$\r$\n"
  FileWrite $4 "not-updated:$R8$\r$\n"
  FileWrite $4 "placement-refused:$R4$\r$\n"
  FileWrite $4 "Action: Do NOT uninstall. Quit cys from the app (it saves sessions),$\r$\n"
  FileWrite $4 "        wait 10 s, run this installer again and choose 'Do not uninstall'.$\r$\n"
  FileWrite $4 "Note: tokens above name binaries whose old build is still in place;$\r$\n"
  FileWrite $4 "      the new files could not be written (file in use / blocked).$\r$\n"
  FileClose $4
cys_post_nofile:
  ClearErrors
  DetailPrint "cys: FATAL - executable verification failed.  unrecoverable:$R6  rolled-back:$R7  not-updated:$R8  placement-refused:$R4"
  ; 콘솔이 붙어 있으면(silent/CI) 표준출력으로도 외친다 — 템플릿 자체의 실패 통보와 동일 패턴.
  System::Call 'kernel32::AttachConsole(i -1)i.r0'
  StrCmp $0 0 cys_post_nocon 0
  System::Call 'kernel32::GetStdHandle(i -11)i.r0'
  FileWrite $0 "cys installer FAILED: executable verification.  unrecoverable:$R6  rolled-back:$R7  not-updated:$R8  placement-refused:$R4$\r$\n"
cys_post_nocon:
  IfSilent cys_post_abort 0
  MessageBox MB_ICONSTOP "cys installation did not complete correctly.$\r$\n$\r$\nMissing:$R6$\r$\nRolled back to previous:$R7$\r$\nNot updated (old build still in place):$R8$\r$\nPlacement refused (old build still in place):$R4$\r$\n$\r$\nYour existing cys still works and no sessions were killed.$\r$\nDo NOT uninstall. Quit cys from the app (it saves sessions), wait 10 s,$\r$\nthen run this installer again and choose 'Do not uninstall'.$\r$\nSee cys-install-failure.txt in the install folder."
cys_post_abort:
  ; SetErrorLevel 은 리터럴로만 쓴다(변수 인자 지원 여부를 mac 개발기에서 컴파일로 확인할 수
  ; 없으므로, 검증 못 한 문법에 릴리스를 걸지 않는다 — 분기 4줄이 그 불확실성보다 싸다).
  StrCmp $3 "4" 0 cys_post_lvl3
  SetErrorLevel 4
  Goto cys_post_end
cys_post_lvl3:
  SetErrorLevel 3
cys_post_end:
  Abort "cys installation failed: critical executable verification"

cys_post_ok:
  Delete "$INSTDIR\cys-install-failure.txt"   ; 지난 실패 흔적 청소(성공 시에만)
  ; ★버전 마커 — **모든 게이트를 통과한 뒤에만** 쓴다. 오라클 도입으로 판정 재료는 아니게
  ; 됐지만(정보성), 실패한 설치로는 갱신되지 않는 '직전 반영 버전' 기록으로 유지한다
  ; (제거 시 삭제 목록·CI·가이드가 아는 이름 — NSIS-CONTRACT.md).
  ClearErrors
  FileOpen $4 "$INSTDIR\cys-installed-version.txt" w
  IfErrors cys_post_nomarker
  FileWrite $4 "${VERSION}"
  FileClose $4
cys_post_nomarker:
  ClearErrors

  ; 바탕화면 바로가기 자동 생성(2026-07-05 오너 지시). Tauri NSIS 템플릿은 일반 GUI 설치에서
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
  ; 이름 스코프 와일드카드 — tick 슬롯(`<bin>.prev<숫자>.exe`)까지 잡는다. $INSTDIR 는
  ; 사용자 데이터가 있는 state dir 라 광범위 와일드카드는 금지(우리 이름 문법만).
  Delete "$INSTDIR\cys.new.exe"
  Delete "$INSTDIR\cysd.new.exe"
  Delete "$INSTDIR\cys-app.new.exe"
  Delete "$INSTDIR\cys.prev*.exe"
  Delete "$INSTDIR\cysd.prev*.exe"
  Delete "$INSTDIR\cys-app.prev*.exe"
  Delete "$INSTDIR\cys-install-failure.txt"
  Delete "$INSTDIR\cys-installed-version.txt"   ; 버전 마커(언인스톨러 미추적 파일)
  ; 잠금 스윕 잔해(*.prev<rand> — 언인스톨러 미추적 파일)까지 정리해 빈 폴더 잔존을 막는다.
  ; 프로세스는 위에서 전부 종료됐으므로 삭제 가능. runtime은 전량 우리 소유 트리다.
  RMDir /r "$INSTDIR\runtime"
  ClearErrors
!macroend
