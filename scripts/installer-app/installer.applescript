-- Install cys.app — cys 터미널 설치 도우미 (osacompile 로 .app 컴파일 · Developer ID 서명·공증·staple).
--
-- 존재 이유: Finder 드래그(=최종 경로로 직접 복사)는 트랜잭션이 아니라 복사 도중 최종 경로에
--   "반쪽 번들"을 노출한다. 그 순간 사용자가 앱을 열면 codesign/Gatekeeper 가 미완본을 보고
--   "손상되었기 때문에 열 수 없습니다"로 차단한다. 이 설치 도우미는 그 경합을 **원자 교체**(숨김
--   스테이징 → renamex_np/rename 단일 시스템 콜)로 제거한다 — 최종 경로엔 완본만, 한순간에 나타난다.
--
-- ★보안(LPE 방어) 2겹:
--   (1) 승격·번들 코어 실행 **전에** 자기 자신을 codesign --verify --strict 로 검증한다. 봉인된
--       Contents/Resources/{install-core.sh,atomic_bundle.py} 가 변조되면 봉인이 깨져 검증 실패로
--       실행을 멈춘다 — 쓰기 가능한 위치로 옮겨진 사본에 공격자 코드를 심어 root 로 돌리는 경로를 닫는다.
--   (2) 원자 설치 코어는 **오직 자기 번들 내부** Contents/Resources/install-core.sh 로만 호출한다.
--       DMG 형제·임의 경로 스크립트를 관리자 권한으로 실행하지 않는다. 소스 cys.app 진위(서명·Team
--       ID·공증)는 install-core.sh 가 동봉 python 실행 전에 검증한다.
--
-- ★App Translocation 대응: 브라우저로 받은 DMG(quarantine 부착)에서 'Install cys.app' 을 더블클릭하면
--   macOS 가 이 앱을 무작위 읽기전용 경로(AppTranslocation)로 복제 실행한다. 그 순간 `path to me`
--   형제(.support/cys.app)는 함께 복제되지 않아 사라진다. 그래서 형제 탐색이 실패하면 **마운트된 cys
--   디스크 이미지(/Volumes/cys*)** 에서 소스를 찾는다 — DMG 는 앱 실행 중 계속 마운트돼 있으므로
--   번역이동돼도 원본을 확실히 찾는다.
--
-- 동작: 자기 무결성 검증 → 소스 cys.app 탐색(형제 → /Volumes/cys* → 하위 find) → 진행 안내 →
--       번들 내부 코어로 무승격 원자 설치 시도 → 권한 실패 시에만 관리자 승격 폴백 → 소스 미발견 시
--       stop 다이얼로그(명시 실패) → 완료/실행 다이얼로그. DEST 는 /Applications/cys.app 로 고정한다.

on run
	-- ── 자기(.app) 경로 · 컨테이너 ──
	-- POSIX path of (path to me) 는 번들이라 끝에 슬래시가 붙는다 → 제거해 정규화.
	set mePosix to POSIX path of (path to me)
	if mePosix ends with "/" then set mePosix to text 1 thru -2 of mePosix
	set container to do shell script "dirname " & quoted form of mePosix

	-- ── ★(1) 자기 무결성 검증 (승격·번들 코어 실행 전) ──
	-- 봉인된 리소스(install-core.sh·atomic_bundle.py)가 변조되면 봉인이 깨져 codesign 검증이 멈춘다.
	-- 게다가 Team ID 를 Q43YA2NMF9 로 고정한다 — 공격자가 변조 후 **ad-hoc 재서명으로 봉인을 다시
	-- 맞춰도**(codesign --verify 만으로는 통과) cys 개인키가 없어 이 Team ID 로는 서명할 수 없으므로
	-- 걸러진다(위조 불능). 번역이동된 정상 사본은 서명·Team ID 가 그대로라 통과한다(변조 시에만 실패).
	try
		do shell script "me=" & quoted form of mePosix & "; /usr/bin/codesign --verify --strict \"$me\" || exit 1; tid=$(/usr/bin/codesign -dv --verbose=4 \"$me\" 2>&1 | /usr/bin/sed -n 's/^TeamIdentifier=//p' | head -1); [ \"$tid\" = \"Q43YA2NMF9\" ] || exit 1"
	on error
		display dialog "설치 도우미의 무결성 검증에 실패했습니다." & return & return & "이 앱이 변조되었거나 서명이 손상되었습니다. 원본 디스크 이미지(.dmg)에서 다시 실행해 주세요." buttons {"확인"} default button 1 with icon stop
		return
	end try

	-- ── 소스 cys.app 탐색 (읽기 전용) ──
	-- ① 형제/하위 후보(번역이동 아닐 때 성립). ② 마운트된 cys DMG(번역이동돼도 성립). ③ 하위 find.
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
		-- ★App Translocation 방어: 마운트된 cys 디스크 이미지에서 소스를 찾는다(volname='cys').
		--   glob 무매치·미발견이면 빈 문자열을 돌려주고 exit 0(다음 단계로 넘어감).
		try
			set src to do shell script "for v in /Volumes/cys*; do for c in \"$v/.support/cys.app\" \"$v/cys.app\"; do [ -d \"$c/Contents\" ] && { printf %s \"$c\"; exit 0; }; done; done; exit 0"
		end try
	end if
	if src is "" then
		try
			set src to do shell script "find " & quoted form of container & " -maxdepth 3 -name cys.app -type d 2>/dev/null | head -1"
		end try
	end if
	if src is "" then
		-- 무음 반쪽설치 금지 — 소스 미발견은 명시 실패로 멈춘다.
		display dialog "cys.app 을 찾지 못했습니다." & return & return & "cys 디스크 이미지(.dmg)가 마운트된 상태에서, 그 디스크 이미지 안의 'Install cys.app'을 실행해 주세요." buttons {"확인"} default button 1 with icon stop
		return
	end if

	-- ── 목적지 (고정 — 사용자 환경변수로 root 파일연산 목적지를 바꿀 수 없다) ──
	set dest to "/Applications/cys.app"

	-- ── 진행 안내 ──
	display dialog "cys 터미널을 설치합니다." & return & return & "설치 도우미가 앱을 " & dest & " 에 안전하게(원자적으로) 복사합니다. 복사가 끝나기 전에는 앱이 최종 위치에 나타나지 않으므로 '손상' 오류가 발생하지 않습니다." buttons {"취소", "설치"} default button "설치"
	if button returned of result is "취소" then return

	-- ── ★(2) 번들 내부 코어만 호출 (LPE 방어) ──
	set coreScript to mePosix & "/Contents/Resources/install-core.sh"
	set shcmd to "/bin/bash " & quoted form of coreScript & " " & quoted form of src & " " & quoted form of dest

	-- 무승격 우선 시도 → **권한 실패(exit 1)** 일 때만 관리자 승격 폴백.
	-- (관리자 계정 대다수는 /Applications 에 무승격으로 쓸 수 있어 사용자 소유가 유지된다.)
	-- exit 2/3/4(소스 아님·도구 결손·소스 진위 실패)는 승격하지 않고 그대로 실패시킨다.
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
