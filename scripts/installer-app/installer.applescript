-- Install cys.app — cys 터미널 설치 도우미 (osacompile 로 .app 컴파일 · Developer ID 서명·공증·staple).
--
-- 존재 이유: Finder 드래그(=최종 경로로 직접 복사)는 트랜잭션이 아니라 복사 도중 최종 경로에
--   "반쪽 번들"을 노출한다. 그 순간 사용자가 앱을 열면 codesign/Gatekeeper 가 미완본을 보고
--   "손상되었기 때문에 열 수 없습니다"로 차단한다. 이 설치 도우미는 그 경합을 **원자 교체**(숨김
--   스테이징 → renamex_np/rename 단일 시스템 콜)로 제거한다 — 최종 경로엔 완본만, 한순간에 나타난다.
--
-- ★보안(LPE 방어): 원자 설치 코어는 **오직 자기 번들 내부** Contents/Resources/install-core.sh 로만
--   호출한다. DMG 형제·쓰기 가능 경로의 스크립트를 관리자 권한으로 실행하면 로컬 권한 상승 취약점이
--   되므로 절대 그 경로를 부르지 않는다. 소스 cys.app 은 **읽기 전용**으로만 탐색한다.
--
-- 동작: 자기 위치에서 소스 cys.app 탐색(숨김 .support/cys.app 등) → 진행 안내 → 번들 내부 코어로
--       무승격 원자 설치 시도 → 실패 시에만 관리자 승격 폴백 → 소스 미발견 시 stop 다이얼로그(명시 실패)
--       → 완료/실행 다이얼로그.
-- DEST 는 환경변수 CYS_INSTALL_DEST 로 재정의 가능(테스트용). 기본은 /Applications/cys.app.

on run
	-- ── 자기(.app) 경로 · 컨테이너 ──
	-- POSIX path of (path to me) 는 번들이라 끝에 슬래시가 붙는다 → 제거해 정규화.
	set mePosix to POSIX path of (path to me)
	if mePosix ends with "/" then set mePosix to text 1 thru -2 of mePosix
	set container to do shell script "dirname " & quoted form of mePosix

	-- ── 소스 cys.app 탐색 (읽기 전용) ──
	-- DMG 레이아웃: 설치 도우미는 가시 항목, cys.app 은 숨김 .support/ 아래. 형제/하위도 폴백 탐색.
	set src to ""
	set candidates to {container & "/.support/cys.app", container & "/cys.app", container & "/Resources/cys.app"}
	repeat with c in candidates
		set cPath to (c as text)
		if (do shell script "[ -d " & quoted form of cPath & "/Contents ] && echo yes || echo no") is "yes" then
			set src to cPath
			exit repeat
		end if
	end repeat
	if src is "" then
		try
			set src to do shell script "find " & quoted form of container & " -maxdepth 3 -name cys.app -type d 2>/dev/null | head -1"
		end try
	end if
	if src is "" then
		-- 무음 반쪽설치 금지 — 소스 미발견은 명시 실패로 멈춘다.
		display dialog "cys.app 을 찾지 못했습니다." & return & return & "이 설치 도우미와 같은 디스크 이미지(.dmg) 안에서 실행해 주세요. 디스크 이미지 밖으로 옮겨 실행하면 원본 앱을 찾을 수 없습니다." buttons {"확인"} default button 1 with icon stop
		return
	end if

	-- ── 목적지 ──
	set dest to "/Applications/cys.app"
	try
		set envDest to (do shell script "echo ${CYS_INSTALL_DEST:-}")
		if envDest is not "" then set dest to envDest
	end try

	-- ── 진행 안내 ──
	display dialog "cys 터미널을 설치합니다." & return & return & "설치 도우미가 앱을 " & dest & " 에 안전하게(원자적으로) 복사합니다. 복사가 끝나기 전에는 앱이 최종 위치에 나타나지 않으므로 '손상' 오류가 발생하지 않습니다." buttons {"취소", "설치"} default button "설치"
	if button returned of result is "취소" then return

	-- ── ★번들 내부 코어만 호출 (LPE 방어) ──
	set coreScript to mePosix & "/Contents/Resources/install-core.sh"
	set shcmd to "/bin/bash " & quoted form of coreScript & " " & quoted form of src & " " & quoted form of dest

	-- 무승격 우선 시도 → 권한 실패 시에만 관리자 승격 폴백.
	-- (관리자 계정 대다수는 /Applications 에 무승격으로 쓸 수 있어 사용자 소유가 유지된다.)
	try
		do shell script shcmd
	on error errMsg number errNum
		if errNum is 1 then
			try
				do shell script shcmd with administrator privileges
			on error errMsg2
				display dialog "설치 실패:" & return & errMsg2 buttons {"확인"} default button 1 with icon stop
				return
			end try
		else
			display dialog "설치 실패:" & return & errMsg buttons {"확인"} default button 1 with icon stop
			return
		end if
	end try

	-- ── 완료 · 실행 ──
	display dialog "설치가 완료되었습니다." & return & dest & return & return & "지금 실행하시겠습니까?" buttons {"닫기", "실행"} default button "실행"
	if button returned of result is "실행" then
		do shell script "open " & quoted form of dest
	end if
end run
