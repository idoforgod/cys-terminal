#!/usr/bin/env python3
"""test_role_bootstrap_hook.py — 마스터 선언 발화 계층 회귀(감지기 함수 + 훅 통합).

제품 절대요구(출하 기본 계약): "너는 마스터다" 입력 → 부트스트랩 100% 발화. 2회 성찰(적대검증+아키텍트)이
지적한 트리거 미검증(arch#2)·감지 오류(adv#2)·role-blind(arch#1)·인용 오발화(adv#8)·부정 오억제(adv#7)를
회귀로 핀한다.

★T-0147-7 W1b 구조 변경(CS-8): 감지·억제 판정이 훅의 셸 grep 스택에서 **python 단일 함수**
  (`bin/javis_detect.py::detect`)로 이관됐다. 그래서 이 파일은 두 층을 나눠 검증한다:
    ① **함수 직접 검증**(빠르고 결정론적 · corpus 본진) — 절 경계 스코프(A4)·주어 어휘
       (P3-A-NEGA)·filler 15/16 경계(P3-A-FILLER)·감지창 200 **문자** 경계(G25).
    ② **훅 통합 검증**(프로세스 경계가 필요한 것만) — 기존 발화/무시 행렬 전량 보존 + role
       allowlist 반전(A3)·surface-role 판정불가 fail-closed(A5)·LC_ALL=C 파리티(G9)·
       A2 surface 게이트·★W-F2 cp949 note 생존(+가드 제거 음성 대조).
  ①의 케이스를 ②로 중복 실행하지 않는 이유는 비용이다(훅 1회 ≈ 프로세스 3개). 대표 케이스만
  훅으로 교차 확인해 '함수는 맞는데 배선이 틀린' 구멍을 막는다.

★corpus 원본(W-A1b 사본 수렴): ① 의 FIRE/SKIP 정의처는 `fixtures/detect-corpus.json`
  하나다 — 이 파일은 더 이상 리터럴 사본을 갖지 않고 원본을 읽으며, ② 훅 행렬은 원본에서의
  **대표 선정**(소속·극성을 원본과 대조)이다. 사본이 원본과 어긋난 채 주석까지 낡는 드리프트가
  실제로 있었다(아래 '역수리 경고' 참조).

관측 기법(②): 격리 HOME + 빈 팩(목 javis_bootstrap.py) + PATH 앞 목 cys(surface-role 반환값·rc 주입)로
훅을 실행하고, stdout에 "발화됨" NOTE가 있으면 발화, 없으면 무시로 판정.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))       # cysjavis-pack/bin
HOOK = os.path.join(os.path.dirname(BIN), "hooks", "role-bootstrap.sh")
# ★부트 v2 A2(2026-09-04): 훅이 **런처(HOOK) + 본체(HOOK_BODY)** 로 갈렸다. 실행 진입점은
#   여전히 HOOK 이고, note 블록·가드 변수 같은 **내용 핀은 본체**에 있다. 팩 밖 사본을 만들어
#   실행하는 하네스는 **두 파일을 함께** 복사해야 한다(런처 단독이면 '본체 부재' 고지로 끝난다).
HOOK_BODY = os.path.join(os.path.dirname(HOOK), "role-bootstrap-legacy.sh")
sys.path.insert(0, BIN)
import javis_detect  # noqa: E402  (형제 모듈 — 감지기 단일 소유)


# ══════════════════════════════════════════════════════════════════════════════
# ① 감지기 함수 직접 검증 — corpus 본진
# ══════════════════════════════════════════════════════════════════════════════
# ★corpus 단일 원본(W-A1b 사본 수렴): FIRE/SKIP 의 정의처는 fixtures/detect-corpus.json
#   하나다. 종전 이 파일의 리터럴 사본(LEGACY_*/NEW_*)은 원본과 이미 어긋나 있었고(문구 드리프트
#   1건 + 아래 구 스펙 주석 박제), 어긋난 사본·주석을 좇은 역수리가 곧 사고 경로였다 — 그래서
#   사본을 지우고 원본을 읽는다. 구 훅 시대 회귀 목록(W1a 결정론화분)은 원본의 부분집합으로
#   전량 보존된다(적재기의 "fixture ⊇ 내장 리터럴" 불변식이 그 보존을 기계로 지킨다).
#   적재는 javis_detect._load_corpus() 재사용 — 스키마 검증·상위집합 불변식 동승(로더 사본 금지).
#
# ※경계 명시(신 스펙 — W-A2 4축 재설계 이후): 절 경계 **없이** 이어붙인 후속 의문
#   ("너는 마스터다 뭐부터 할까?")은 이제 **발화**가 정답이다(실측 fire). 한국어 채팅체는
#   문장부호 생략이 기본값이라, 절 꼬리의 '?'·'무엇' 같은 원거리 의문 표지를 선언의 억제
#   마커로 읽던 구 규칙이 무발화 최빈 사고의 원인이었다. 억제는 이제 부정(neg)과 **구조
#   신호** 3축으로만 성립한다 — pre(절 안에서 선언보다 **앞**의 마커) / quote(인용부호가
#   선언을 **감쌈**) / quotative(선언 종료 **직후** 인접창의 인용 전달 조사·에코 '?').
# ★역수리 경고: 이 자리에 있던 구 주석("무구두점 후속 의문은 억제가 정답")을 좇아 감지기를
#   되돌리면 무발화 최빈 사고가 그대로 재발한다 — 정답의 정의처는 위 corpus 원본이다.
_CORPUS_SOURCE, _CORPUS_FIRE_ITEMS, _CORPUS_SKIP_ITEMS, _CORPUS_ERR = javis_detect._load_corpus()
if _CORPUS_ERR or "fixture" not in (_CORPUS_SOURCE or ""):
    # 이 테스트는 fixtures/ 와 같은 트리에 산다 — 원본 부재·불량이면 내장 리터럴 폴백으로
    # 조용히 좁혀 돌지 않고 검체 무효로 죽는다(측정 불능은 통과가 아니다).
    sys.exit("detect-corpus 원본 적재 실패(검체 무효): %s"
             % (_CORPUS_ERR or ("source=%r — fixture 미적재" % (_CORPUS_SOURCE,))))
CORPUS_FIRE = [it["text"] for it in _CORPUS_FIRE_ITEMS]
CORPUS_SKIP = [it["text"] for it in _CORPUS_SKIP_ITEMS]


def fn_matrix(fails):
    for p in CORPUS_FIRE:
        v = javis_detect.detect(p)
        if not v["fire"]:
            fails.append("fn FALSE-NEGATIVE %r → %s" % (p, v["reason"]))
    for p in CORPUS_SKIP:
        v = javis_detect.detect(p)
        if v["fire"]:
            fails.append("fn FALSE-POSITIVE %r → %s" % (p, v["reason"]))

    # ── filler 경계(P3-A-FILLER): 15자=발화 / 16자=미발화. 주석(구 '12자')과 코드의 불일치를
    #    수치 상수(FILLER_MAX)로 못박고 경계를 corpus로 박제한다.
    fmax = javis_detect.FILLER_MAX
    if fmax != 15:
        fails.append("FILLER_MAX 스펙 이탈(%r≠15)" % fmax)
    if not javis_detect.detect("너는" + "가" * fmax + "마스터다")["fire"]:
        fails.append("filler %d자 경계 미발화" % fmax)
    if javis_detect.detect("너는" + "가" * (fmax + 1) + "마스터다")["fire"]:
        fails.append("filler %d자 오발화(경계 누수)" % (fmax + 1))

    # ── 감지창 경계(G25): **문자** 200자. GNU cut -c(바이트)면 한글 약 66자에서 잘렸다.
    w = javis_detect.WINDOW_CHARS
    if w != 200:
        fails.append("WINDOW_CHARS 스펙 이탈(%r≠200)" % w)
    decl = "너는 마스터다"
    inside = "가" * (w - len(decl)) + decl          # 선언이 창 **끝**에 정확히 걸린다
    if not javis_detect.detect(inside)["fire"]:
        fails.append("감지창 %d자 경계(한글)에서 미발화 — 바이트 슬라이스 회귀" % w)
    if len(inside.encode("utf-8")) <= w:
        fails.append("경계 검체가 바이트 관점에서도 창 안이라 회귀를 못 잡는다(검체 무효)")
    if javis_detect.detect("가" * w + decl)["fire"]:
        fails.append("감지창 밖 선언이 발화(창 미적용)")

    # ── 억제/미검출 분리(CS-8⑤ 로그 의무의 전제): verdict 타입이 융합되면 훅이 분기 못 한다
    if javis_detect.detect("'너는 마스터다'가 무슨 뜻?")["verdict"] != "suppressed":
        fails.append("인용·의문 케이스 verdict≠suppressed")
    if javis_detect.detect("오늘 작업 지시해줘")["verdict"] != "no_declaration":
        fails.append("무선언 케이스 verdict≠no_declaration")

    # ── 감지기 CLI 계약(훅이 소비하는 exit 표) ──
    for prompt, want in (("너는 마스터다", 0), ("오늘 작업 지시해줘", 1),
                         ("'너는 마스터다'가 무슨 뜻?", 3)):
        r = subprocess.run([sys.executable, os.path.join(BIN, "javis_detect.py"), "hook-gate"],
                           input=json.dumps({"prompt": prompt}), capture_output=True,
                           text=True, timeout=30)
        if r.returncode != want:
            fails.append("CLI exit 계약 이탈 %r: %d≠%d" % (prompt, r.returncode, want))
        if want == 3 and "SKIP(억제)" not in r.stderr:
            fails.append("억제인데 로그 1줄이 없다(침묵 억제 회귀): %r" % r.stderr[:200])
        if want == 1 and r.stderr.strip():
            fails.append("무선언인데 로그를 냈다(정상 침묵 위반): %r" % r.stderr[:200])
    r = subprocess.run([sys.executable, os.path.join(BIN, "javis_detect.py"), "hook-gate"],
                       input="{not json", capture_output=True, text=True, timeout=30)
    if r.returncode != 2:
        fails.append("입력 파싱 불가가 cannot-judge(2)로 분리되지 않음: %d" % r.returncode)


# ══════════════════════════════════════════════════════════════════════════════
# ② 훅 통합 검증
# ══════════════════════════════════════════════════════════════════════════════
_MOCK_BOOT = "#!/usr/bin/env python3\nprint('MOCK')\n"   # 발화 성공 경로용 목 부트(존재+즉시 exit 0)


def _run_hook_proc(prompt, surface_role="", surface_env=True, role_rc=0, extra_env=None,
                   hook_path=None, boot_py=_MOCK_BOOT, cys_body=None):
    """훅을 격리 실행하고 CompletedProcess 를 그대로 돌려준다(stdout/stderr **분리** 관측).

    ★W-F2 가 요구한 분리다: note 생존 판정은 stdout 의 JSON 완주가 근거고, 소실 원인 판정은
    stderr 의 UnicodeEncodeError 가 근거다 — 합쳐 보면 두 사실이 융합된다.
    hook_path — 기본 None=실제 훅(HOOK). W-F2 음성 대조가 가드를 뗀 변형 사본 경로를 넘긴다.
    boot_py   — 목 javis_bootstrap.py 소스(기본 _MOCK_BOOT=발화 성공). None 이면 **만들지
                않아** BOOT 부재 경로가 된다(notify_pipe_release 와 같은 방향).
    cys_body  — 목 cys 전체 소스 오버라이드(기본 None=surface-role 주입 no-op). P0-4 정직 예보
                검체가 claim-role rc6 을 재현할 때 쓴다(기본 목은 claim rc0 으로 접는다).
    """
    home = tempfile.mkdtemp()
    pack = tempfile.mkdtemp()
    mockbin = tempfile.mkdtemp()
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    # 목 javis_bootstrap.py (부트 안 함 — 존재만. boot_py=None 은 BOOT 부재 경로)
    if boot_py is not None:
        with open(os.path.join(pack, "bin", "javis_bootstrap.py"), "w") as f:
            f.write(boot_py)
    # 목 cys — surface-role만 주입, 나머지 no-op.
    # ★P2(R3-P2-7 ⓐ): boot-intent 기본값 = **구 CLI 모사**(rc2+"unrecognized subcommand") —
    #   이 파일의 기존 행렬 전부가 종전 spawn **폴백 leg** 를 계속 재게 한다(조용한 스큐 폴백).
    #   frontdoor leg 는 cys_body 오버라이드(_FRONTDOOR_CYS)가 전담한다.
    cysp = os.path.join(mockbin, "cys")
    with open(cysp, "w") as f:
        f.write(cys_body if cys_body is not None else
                '#!/bin/bash\n[ "$1" = surface-role ] && { echo "%s"; exit %d; }\n'
                '[ "$1" = boot-intent ] && { echo "error: unrecognized subcommand '
                "'boot-intent'\" >&2; exit 2; }\nexit 0\n"
                % (surface_role, role_rc))
    os.chmod(cysp, 0o755)
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_PACK_DIR"] = pack
    env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
    env.pop("CYS_SOCKET", None)
    # ★기계유래 스폰 게이트(2026-08-10 P3B): 훅은 spawn 전에 javis_mission machine-origin 으로
    #   배달 원장을 **읽는다**(층1 해시 대조). 러너 환경의 CYS_STATE_DIR 이 새면 실사용 원장이
    #   판정에 끼어들어 FIRE 행렬이 실행 위치에 따라 흔들린다(결정론 파괴) — HOME 격리(tempdir)에
    #   맞춰 명시 제거한다(원장 경로가 격리 HOME 아래로 해소되게 · run_bootstrap_health._base_env
    #   와 같은 규율).
    env.pop("CYS_STATE_DIR", None)
    # ★A2 surface 이중 게이트(T-0147-7 W1a): 훅은 cys pane 안에서만 발화한다. 이 하네스는
    # 'cys pane 안'을 재현하므로 CYS_SURFACE_ID를 **명시** 주입한다 — os.environ 상속에 의존하면
    # 테스트 결과가 실행 위치(cys pane 안/밖)에 따라 흔들린다(결정론 파괴).
    env.pop("AITERM_SURFACE_ID", None)
    # ★임무 게이트(2026-08-10 D4-a′ 오너 재정): 훅의 **발화(spawn)** 는 이제 임무 유무와
    #   무관하다(선언=기동 명령) — 임무 게이트 exit 는 주입문의 **착수 규율 문안**(MISSION_SENT)
    #   만 가른다. 이 하네스가 재는 것은 **감지·게이트 배선**(A2·A4·A3=B7·G9·G25)이므로,
    #   문안 변인을 고정하기 위해 임무 지정 세션을 명시 주입한다
    #   (실재 수단: `javis_mission.gate()` ①번 신호 `CYS_MISSION`).
    #   ★정정(2026-08-01 R2 적발 (d)): 이 변수를 자동으로 채우는 launcher 는 **없다** — pane env 는
    #   데몬 프로세스 env 를 상속하므로 `cys launch-agent` 호출 시점의 값은 전달되지 않는다.
    #   여기서는 하네스가 직접 주입한다. 상속에 기대면 실행 위치에 따라 결과가 흔들린다(결정론 파괴).
    #   ※임무 미지정 경로(신규 사용자 · spawn 1 + 착수금지 문안)의 계약은
    #     run_bootstrap_health H-MISSION-1 소속.
    env["CYS_MISSION"] = "test_role_bootstrap_hook 검체 임무(오너 지정 가정)"
    if surface_env:
        env["CYS_SURFACE_ID"] = "7"
    else:
        env.pop("CYS_SURFACE_ID", None)
    if extra_env:
        env.update(extra_env)
    return subprocess.run(["bash", hook_path or HOOK], input=json.dumps({"prompt": prompt}),
                          capture_output=True, text=True, timeout=30, env=env)


def _run_hook(prompt, surface_role="", surface_env=True, role_rc=0, extra_env=None):
    """훅을 격리 실행. surface_role = 목 cys surface-role 반환값(빈=미claim). 반환: (발화 bool, stdout).

    surface_env=False 는 **cys 밖**(VS Code 등 임의 claude 세션)을 재현한다 — A2 게이트 핀.
    role_rc≠0 은 surface-role **판정 불가**(데몬 미응답·hang 등)를 재현한다 — A5 fail-closed 핀.
    (기존 소비면 유지 — 원시 관측이 필요하면 _run_hook_proc 를 직접 쓴다.)
    """
    try:
        r = _run_hook_proc(prompt, surface_role, surface_env, role_rc, extra_env)
    except Exception as e:
        return False, "exec 실패: %s" % e
    return ("발화됨" in r.stdout), r.stdout + r.stderr


def notify_pipe_release(fails):
    """★W-A0 알림 파이프 점유 해제 계측 — 훅 exit 후 stdout EOF 까지의 벽시계가 짧아야 한다.

    결함(수리 전 실측): `_notify_bg` 류 백그라운드 서브셸이 훅의 stdout/stderr 를 상속한 채 남아,
    데몬 wedge 로 `cys feed push` 가 hang 이면 하네스(UserPromptSubmit stdout 파이프 리더)가 EOF 를
    영영 못 받았다 — 사용자 프롬프트 제출 먹통. `subprocess.run(timeout=…)` 은 프로세스 종료가
    아니라 **파이프 EOF** 를 기다리므로 이 하네스 자체가 계측기다(POSIX 는 TimeoutExpired 시
    kill 후 직접 자식만 wait 하므로 손자 hang 에 재차 붙잡히지 않는다 — 20s 에 깨끗이 FAIL).

    검체 경로: BOOT 부재(pack/bin 에 javis_bootstrap.py 를 **만들지 않는다**) — 성공 spawn 없이
    알림이 결정론으로 발화하는 경로. 목 cys 는 판정 채널(surface-role)엔 즉답하고 알림 채널
    (feed·send)만 60s hang 해 wedge 데몬을 재현한다(전 채널 hang 이면 role 게이트 데드라인이
    먼저 fail-closed 로 꺼져 검체가 알림 경로에 못 간다).

    검증 3속성(전부 W-A0 계약 핀):
      ① EOF 빠름 — 서브셸 진입 즉시 exec fd 분리. 경계 8s 는 '수리 전=hang 자식 종료까지 60s
         (하네스 20s 초과)' 와 'exec 소실+데드라인 잔존 부분회귀=5s+5s≈10s+' 를 모두 잡되
         정상(≈2s)에 4배 여유를 준 값이다.
      ② 데드라인 실효 — feed hang 이 5s 에 잘려야 send 폴백이 로그에 나타난다(cys_timeout_run
         소실이면 feed 가 60s 를 다 자므로 창 안에 send 가 없다).
      ③ 알림 시도는 여전히 발생(feed push 로그) — 발화 조건을 줄이지도 늘리지도 않았다는 앵커 핀.
    """
    home = tempfile.mkdtemp()
    pack = tempfile.mkdtemp()
    mockbin = tempfile.mkdtemp()
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)   # ★javis_bootstrap.py 없음 = BOOT 부재 경로
    calls = os.path.join(mockbin, "cys-calls.log")
    cysp = os.path.join(mockbin, "cys")
    with open(cysp, "w") as f:
        f.write("#!/bin/bash\n"
                "printf '%%s\\n' \"$*\" >> '%s'\n"
                "case \"$1\" in\n"
                "  feed|send) sleep 60 ;;\n"
                "  surface-role) echo ''; exit 0 ;;\n"
                "esac\n"
                "exit 0\n" % calls)
    os.chmod(cysp, 0o755)
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_PACK_DIR"] = pack
    env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
    env.pop("CYS_SOCKET", None)
    env.pop("CYS_STATE_DIR", None)          # _run_hook 과 같은 격리 규율(실사용 원장 누수 차단)
    env.pop("AITERM_SURFACE_ID", None)
    env["CYS_MISSION"] = "W-A0 파이프 계측 검체 임무(오너 지정 가정)"
    env["CYS_SURFACE_ID"] = "7"
    t0 = time.monotonic()
    try:
        r = subprocess.run(["bash", HOOK], input=json.dumps({"prompt": "너는 마스터다"}),
                           capture_output=True, text=True, timeout=20, env=env)
    except subprocess.TimeoutExpired:
        fails.append("W-A0 회귀: 훅 stdout EOF 미도달(20s) — 알림 서브셸이 하네스 파이프를 "
                     "점유 중(서브셸 exec fd 분리 소실)")
        return
    elapsed = time.monotonic() - t0
    if elapsed >= 8.0:
        fails.append("W-A0 회귀: EOF 까지 %.1fs ≥ 8s — 서브셸이 파이프를 데드라인만큼 붙잡았다"
                     "(exec fd 분리 소실 · cys_timeout_run 만 잔존 형태)" % elapsed)
    if r.returncode != 0:
        fails.append("W-A0: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    if "부트스트랩 불가" not in r.stdout:
        fails.append("W-A0 검체 무효: BOOT 부재 경로 미도달(additionalContext 부재): %r"
                     % (r.stdout + r.stderr)[:300])
    # ②·③ 알림 채널 관측 — 목 cys 는 hang **전에** 로그부터 남기므로 폴링으로 충분하다.
    #    feed push = 알림 발화 자체(③) / send --queued = feed 가 데드라인(5s)에 잘린 뒤의
    #    폴백(②). 폴백 대기 상한 12s: notify fork(≈2s)+데드라인 5s+스폰 여유. 이 창 안에
    #    send 가 없으면 feed 가 60s 를 다 자고 있다는 뜻 = 데드라인 소실.
    seen = ""
    deadline = time.monotonic() + 12.0
    while time.monotonic() < deadline:
        try:
            with open(calls, encoding="utf-8") as f:
                seen = f.read()
        except OSError:
            seen = ""
        if "send --queued" in seen:
            break
        time.sleep(0.2)
    if "feed push" not in seen:
        fails.append("W-A0 회귀: BOOT 부재인데 cys feed push 시도가 없다(알림 발화 조건이 "
                     "변경됐다 — 줄이기 금지): %r" % seen[:300])
    elif "send --queued" not in seen:
        fails.append("W-A0 회귀: feed hang 후 12s 안에 send 폴백이 없다(cys_timeout_run "
                     "데드라인 소실 — 서브셸이 60s 잔존): %r" % seen[:300])


def _note_ctx(stdout):
    """stdout 의 hookSpecificOutput JSON 줄에서 additionalContext 를 회수한다(없으면 "").

    json.loads 가 줄 전체를 검증하므로 '부분 출력 후 사망'도 소실로 판정된다 — substring 검사보다
    강한 계측이다(주입 계약의 실소비자가 JSON 파서라는 사실과 정렬).
    """
    for line in stdout.splitlines():
        if line.startswith('{"hookSpecificOutput"'):
            try:
                return json.loads(line)["hookSpecificOutput"]["additionalContext"]
            except (ValueError, KeyError, TypeError):
                return ""
    return ""


_BOOT_FAIL_PY = "#!/usr/bin/env python3\nimport sys\nsys.exit(7)\n"   # 즉사 rc7 = '발화 실패' 경로
# ★P2 frontdoor 목 cys — boot-intent 가 enqueued 토큰+rc0 을 내 스폰 생략 경로(신설 6번째
#   note 블록)를 태운다. 판정 채널(surface-role·claim rc0)은 기본 목과 동일하게 즉답한다.
_FRONTDOOR_CYS = ('#!/bin/bash\n'
                  '[ "$1" = surface-role ] && { echo ""; exit 0; }\n'
                  '[ "$1" = boot-intent ] && {\n'
                  '  echo "[cys-hook] boot-intent: enqueued" >&2\n'
                  '  echo "[cys-hook] boot-intent detail: mock enqueue" >&2\n'
                  '  exit 0\n'
                  '}\n'
                  'exit 0\n')
# (경로명, boot_py, note 마커, cys_body 오버라이드) — ★W-F2 가 무가드로 남겼던 3블록 전부
# (발화 성공·발화 실패·BOOT 부재) + ★P2 frontdoor(6번째 note 블록 — cp949 생존 확장 · R3-P2-7 ⓒ).
_CP949_PATHS = (
    ("발화 성공", _MOCK_BOOT, "발화됨", None),
    ("발화 실패", _BOOT_FAIL_PY, "발화 실패", None),
    ("BOOT 부재", None, "부트스트랩 불가", None),
    ("frontdoor", _MOCK_BOOT, "데몬 감독자", _FRONTDOOR_CYS),
)


def note_cp949_survival(fails):
    """★W-F2 note 인코딩 가드 회귀 핀 — cp949 stdio 스큐에서도 통보(note)가 생존해야 한다.

    결함(수리 전 실측): 훅의 `python -c` note 블록 5개 중 3개(BOOT 부재·발화 실패·발화 성공)가
    stdio 재구성 가드 없이 남아, PYTHONUTF8 미주입 스큐(구 데몬)의 비UTF8 Windows(cp949)에서
    문안의 U+2014(—) 인코딩 실패로 **선언마다 모델에 가는 통보가 통째로 소실**됐다(훅은 exit 0
    = 완전 침묵). 부트는 실제로 발화됐는데 모델은 아무것도 못 봐 "선언했는데 무반응"이 됐고,
    수동 재실행 금지 경고문도 함께 사라졌다. 수리는 훅의 가드 단일 소스 변수(CYS_NOTE_IO_GUARD)다.

    3속성:
      ① 배선 정합(정적) — 6블록 전부(★P2 frontdoor 포함)가 단일 소스 변수를 쓰고 우회
         인라인이 0이어야 한다(인라인 사본이 반드시 낡는 것이 이번 결함의 형태 그 자체였다).
      ② cp949 생존(양성) — 성공·발화 실패·BOOT 부재·★P2 frontdoor 네 경로 모두 cp949 에서
         note JSON 이 완주 파싱되고 마커가 남는다(+UnicodeEncodeError 부재·훅 exit 0 계약).
      ③ 음성 대조(계측기 타당성) — 가드 값을 뗀 변형 훅에서는 같은 케이스가 실제로 note 를
         잃고(UnicodeEncodeError 동반) 그래야만 이 계측기가 무언가를 재고 있는 것이다.
    ※발화 조건은 관측만 한다(앵커 ①폭주 — 이 핀은 순수 '출력 생존' 계약이다).
    """
    with open(HOOK_BODY, encoding="utf-8") as f:      # note 블록·가드 변수는 본체에 산다
        hook_src = f.read()

    # ① 배선 정합(정적) — ★P2 개정: frontdoor note 블록 신설로 5→6(R3-P2-7 ⓒ · 약화 아님,
    #   블록 수 증가에 따른 계약 핀 갱신 — 훅 정의부 주석 '현재 6곳'과 동기).
    n_sites = hook_src.count('-c "$CYS_NOTE_IO_GUARD"' + "'\n")
    if n_sites != 6:
        fails.append("W-F2 배선: 가드 변수 call site %d≠6 — note 블록을 추가/제거했다면 이 핀과 "
                     "훅 정의부 주석을 함께 갱신하라" % n_sites)
    if "-c 'import json,sys" in hook_src:
        fails.append("W-F2 배선: 가드 변수를 우회하는 인라인 `-c 'import json,sys` 블록 잔존"
                     "(사본 드리프트 재발 경로)")
    m = re.search(r"CYS_NOTE_IO_GUARD='[^']*'", hook_src)
    guard_defined = bool(m and "_s.reconfigure" in m.group(0))
    if not guard_defined:
        # 정의 붕괴라도 ② 실측은 돌린다 — 소실을 정적 소견이 아니라 실행 출력으로 보인다
        # (측정 가능한 것을 정적 FAIL 뒤에 숨기지 않는다). ③ 뮤테이션만 정의 부재로 불능.
        fails.append("W-F2 배선: CYS_NOTE_IO_GUARD 정의에 stdio 재구성 가드가 없다")

    def _proc(name, hook_path=None, boot_py=_MOCK_BOOT, cys_body=None):
        try:
            return _run_hook_proc("너는 마스터다", extra_env={"PYTHONIOENCODING": "cp949"},
                                  hook_path=hook_path, boot_py=boot_py, cys_body=cys_body)
        except Exception as e:
            fails.append("W-F2(%s): 훅 실행 실패(계측 불능은 통과가 아니다): %s" % (name, e))
            return None

    # ② cp949 생존(양성) — 실제 훅(★P2: frontdoor 6번째 블록 포함 4경로)
    for name, boot_py, marker, cys_body in _CP949_PATHS:
        r = _proc(name, boot_py=boot_py, cys_body=cys_body)
        if r is None:
            continue
        ctx = _note_ctx(r.stdout)
        if marker not in ctx:
            fails.append("W-F2 회귀(%s): cp949 에서 note 소실 — additionalContext=%r stderr=%r"
                         % (name, ctx[:120], r.stderr[:200]))
        if "UnicodeEncodeError" in r.stderr:
            fails.append("W-F2 회귀(%s): cp949 에서 UnicodeEncodeError 잔존(무가드 블록 재발)" % name)
        if r.returncode != 0:
            fails.append("W-F2(%s): 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % (name, r.returncode))

    if not guard_defined:
        return   # ③ 은 정의 텍스트를 뮤테이션 대상으로 요구한다(위 ①이 이미 FAIL을 남겼다)

    # ③ 음성 대조 — 가드 값을 뗀 변형(수리 전 상태 재현). 변형 사본은 격리 디렉터리에 두고
    #    _lib.sh 사본 + bin 링크로 훅의 형제 해소(`$(dirname $0)/../bin`)만 재현한다 —
    #    실 저장소 파일은 건드리지 않는다(관측=개입 금지).
    mut_root = tempfile.mkdtemp()
    mut_hooks = os.path.join(mut_root, "hooks")
    os.makedirs(mut_hooks)
    mut_src = hook_src[:m.start()] + "CYS_NOTE_IO_GUARD='import json,sys'" + hook_src[m.end():]
    if "_s.reconfigure" in mut_src:
        fails.append("W-F2 음성 대조 무효: 변형본에 재구성 가드 잔존(뮤테이션 미적중)")
        return
    # 변조 대상은 **본체**이고, 실행 진입점으로는 무변조 런처를 함께 둔다(A2 분할).
    with open(os.path.join(mut_hooks, "role-bootstrap-legacy.sh"), "w", encoding="utf-8") as f:
        f.write(mut_src)
    mut_hook = os.path.join(mut_hooks, "role-bootstrap.sh")
    shutil.copy(HOOK, mut_hook)
    shutil.copy(os.path.join(os.path.dirname(HOOK), "_lib.sh"), os.path.join(mut_hooks, "_lib.sh"))
    try:
        os.symlink(BIN, os.path.join(mut_root, "bin"))
    except OSError:                     # Windows 비대칭(무권한 심링크) — 사본 폴백
        shutil.copytree(BIN, os.path.join(mut_root, "bin"))
    # 성공·실패 양쪽(티켓 요구 범위) + ★P2 frontdoor(신설 블록도 가드 없이는 실제로 죽는가)
    for name, boot_py, marker, cys_body in _CP949_PATHS[:2] + _CP949_PATHS[3:]:
        r = _proc("음성 " + name, hook_path=mut_hook, boot_py=boot_py, cys_body=cys_body)
        if r is None:
            continue
        ctx = _note_ctx(r.stdout)
        if marker in ctx:
            fails.append("W-F2 계측기 무효(%s): 가드를 뗐는데 note 생존 — 이 핀은 아무것도 재지 "
                         "않는다(양성 케이스의 증명력 0)" % name)
        if "UnicodeEncodeError" not in r.stderr:
            fails.append("W-F2 계측기 의심(%s): 변형본의 note 소실이 인코딩 경로가 아니다"
                         "(다른 사망 원인 — 검체 무효): %r" % (name, (r.stdout + r.stderr)[:200]))


def note_success_named_format(fails):
    """★P0-4/R3-P04-1 회귀 핀 — 성공 note 는 '파싱 가능한 JSON 1줄'로 발행된다(명명식 포맷).

    결함 클래스(수리 전 실측): 성공 note 는 %s 10개 × **비순차** argv 튜플([6],[6],[7],[8],[3],
    [5],[1],[4],[2],[6])이라, 문안에 자리표시자 하나를 더하거나 빼며 튜플·인자 목록을 함께 못
    고치면 python % 가 TypeError 로 죽고 stdout 에 JSON 이 전혀 안 나가 통보가 통째로 소실됐다
    (spawn 은 이미 끝난 뒤 + 훅은 exit 0 = 완전 침묵 — W-F2 가 문서화한 사고 클래스의 재발
    경로·비순차 매핑이라 눈 대조도 불가). 수리는 dict(zip(순차 인자)) + %(key)s 명명식 전환이고,
    이 핀은 그 전환 후의 유일한 안전망이다(새 키 추가 시 셸 인자 누락 = KeyError = 같은 소실).

    검증 2경로(+P0-4 정직 예보 분기):
      ① claim rc0(기본 목) — note JSON 완주 + 헤드라인 마커 + 종전 예보('팀 세션 기동') +
         재실행 금지 경고 생존 + 포맷 예외(stderr TypeError/KeyError) 부재.
      ② claim rc6(목 오버라이드) — 같은 생존 속성 + 정직 예보('결정론적으로 실패한다' ·
         retry_eligible/§0-A 포인터). 헤드라인 마커·재실행 금지 문구는 rc6 에서도 불변이어야
         한다(H-DOC-1 ②·'발화됨' substring 판정 훅 테스트의 전복 금지 — P0-4 계약).
    """
    cases = (
        ("rc0", None,
         ("팀 세션 기동", "생존확인")),
        ("rc6", ('#!/bin/bash\n'
                 '[ "$1" = surface-role ] && { echo ""; exit 0; }\n'
                 '[ "$1" = claim-role ] && exit 6\n'
                 'exit 0\n'),
         ("결정론적으로 실패한다", "retry_eligible", "session_error")),
    )
    for name, cys_body, frags in cases:
        try:
            r = _run_hook_proc("너는 마스터다", cys_body=cys_body)
        except Exception as e:
            fails.append("P0-4(%s): 훅 실행 실패(계측 불능은 통과가 아니다): %s" % (name, e))
            continue
        ctx = _note_ctx(r.stdout)   # json.loads 완주가 곧 'JSON 1줄' 판정(부분 출력 사망=소실)
        if "[결정론 부트스트랩 발화됨 — 하네스 강제]" not in ctx:
            fails.append("P0-4 회귀(%s): 성공 note JSON 소실 또는 헤드라인 마커 훼손 — "
                         "additionalContext=%r stderr=%r" % (name, ctx[:150], r.stderr[:200]))
            continue
        if "재실행하지 마라" not in ctx:
            fails.append("P0-4 회귀(%s): 재실행 금지 경고가 note 에서 사라졌다(불변 문구)" % name)
        for frag in frags:
            if frag not in ctx:
                fails.append("P0-4 회귀(%s): 예보 문안에 %r 이 없다(정직 예보/종전 예보 분기 "
                             "훼손): %r" % (name, frag, ctx[:300]))
        for sig in ("TypeError", "KeyError", "Traceback"):
            if sig in r.stderr:
                fails.append("P0-4 회귀(%s): note 포맷 예외 관측(%s) — 침묵 소실 경로 부활: %r"
                             % (name, sig, r.stderr[:300]))
        if r.returncode != 0:
            fails.append("P0-4(%s): 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % (name, r.returncode))


# ── ★P0-5 hook-triage 배치 왕복 — 증분 라인 프로토콜 소비면 계측 ─────────────────────────────
# 목 javis_mission.py 공통 머리: argv 를 HOME/mission-calls.log 에 남긴다(왕복 수 실측의 근거 —
# 배치 채택의 정량 주장 '선언 경로 mission 계열 3왕복→1왕복'을 계수로 검증한다).
_TRIAGE_CALL_LOG = (
    "#!/usr/bin/env python3\n"
    "import os, sys\n"
    "open(os.path.join(os.environ['HOME'], 'mission-calls.log'), 'a')"
    ".write(' '.join(sys.argv[1:]) + '\\n')\n"
    "cmd = sys.argv[1] if len(sys.argv) > 1 else ''\n")
# 전량 출력(정상 배치): record→machine-origin→path 3라인 완주 + exit 1(=MO human 보조 계약)
# ★라인 순서는 실 산출자(javis_mission.cmd_hook_triage)의 개정 순서와 같다(R2-2·R2-5 — 안전
#   임계 라인 MO 가 임의 내용 필드 path 보다 앞).
_TRIAGE_FULL = _TRIAGE_CALL_LOG + (
    "if cmd == 'hook-triage':\n"
    "    sys.stdin.read()\n"
    "    sys.stdout.write('record: rc=1\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('machine-origin: human\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('path: /TRIAGE-PATH-PIN\\n'); sys.stdout.flush()\n"
    "    raise SystemExit(1)\n"
    "raise SystemExit(64)\n")
# ★CRLF 쌍둥이(리뷰어 R1 must_fix 검체 — Windows 실출력 재현): Windows 파이썬(번들 embeddable
# 포함)의 sys.stdout 은 기본 newline 변환으로 \n 을 \r\n 으로 내보낸다(모듈의
# reconfigure(encoding=...)는 newline 을 바꾸지 않는다). write 로 \r\n 을 명시해 POSIX 러너에서도
# 이 라인 종결 성질을 핀한다 — 훅의 sed '$' 앵커가 \r 비관용이면 RECORD_RC 상시 ""(Windows 임무
# 게이트 영구 폐쇄)+MO substring 만 통과하는 조용한 열화가 되는데, LF 목만으로는 그 회귀가
# green 을 통과한다(이 검체가 없어서 실제로 통과했던 이력이 있다).
_TRIAGE_FULL_CRLF = _TRIAGE_CALL_LOG + (
    "if cmd == 'hook-triage':\n"
    "    sys.stdin.read()\n"
    "    sys.stdout.write('record: rc=1\\r\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('machine-origin: human\\r\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('path: /TRIAGE-PATH-PIN\\r\\n'); sys.stdout.flush()\n"
    "    raise SystemExit(1)\n"
    "raise SystemExit(64)\n")
# 중도 사망: record 라인만 flush 하고 즉사(os._exit — MO 라인 없음) = R3-RISK-3 중도 kill 재현.
# ★MO 단독 재왕복(R2-1)도 실패하도록 machine-origin 분기를 두지 않는다(rc 64·무토큰) —
#   '독립 예산을 한 번 더 줬는데도 판정이 없으면 fail-closed' 라는 최종 귀결을 잰다.
_TRIAGE_MIDDEATH = _TRIAGE_CALL_LOG + (
    "if cmd == 'hook-triage':\n"
    "    sys.stdout.write('record: rc=1\\n'); sys.stdout.flush()\n"
    "    os._exit(1)\n"
    "raise SystemExit(64)\n")
# ★record 지연(R2-1 must_fix 재현 검체 · 2026-08-26): record 단계가 느려(AV·백업이 원장을 잡는
#   Windows 공유위반 계급) 배치 단일 8s 예산을 먹어치우면 MO 라인이 창 밖으로 밀린다. 종전
#   배치 경로는 그대로 fail-closed 무스폰(부트 침묵)이었고, 같은 지연에서 구 3왕복 형상은
#   MO 가 자기 5s 를 따로 받아 **부팅했다** — 캠페인이 없애려던 증상의 재도입이자 실패 방향
#   역전(정직 강등 → 부트 0회). 수리 후 계약: record 판정 생존 + MO 단독 재왕복 1회로 spawn.
_TRIAGE_SLOW_RECORD = _TRIAGE_CALL_LOG + (
    "import time\n"
    "if cmd == 'hook-triage':\n"
    "    sys.stdin.read()\n"
    "    time.sleep(6.5)\n"                                     # record 가 예산 대부분을 먹는다
    "    sys.stdout.write('record: rc=1\\n'); sys.stdout.flush()\n"
    "    time.sleep(4.0)\n"                                     # MO 는 8s 창 밖 — 라인 미도달
    "    sys.stdout.write('machine-origin: human\\n'); sys.stdout.flush()\n"
    "    raise SystemExit(1)\n"
    "if cmd == 'machine-origin':\n"                             # 단독 왕복은 자기 예산 안에서 완주
    "    sys.stdin.read(); sys.stdout.write('machine-origin: human\\n'); raise SystemExit(1)\n"
    "raise SystemExit(64)\n")
# ★path 필드 개행 주입(R2-2 재현 검체 · 2026-08-26): 산출자가 무해화를 잃어 path 라인에 개행이
#   실리면, 그 조각이 위조 `machine-origin:` 줄이 된다. 훅 판독기는 **정확 일치 + 토큰 줄 개수
#   1** 이므로 진짜 판정(machine)과 위조(human)가 함께 세어져 개수 2 = 판정 불가 fail-closed 다
#   (종전 `sed|head -1` + substring case 는 위조를 집어 spawn 을 열었다 — 실측 boot_spawn=1).
#   ★검체는 **산출 측 방어 2겹이 모두 무너진** 최악(구 순서 path→MO + 무해화 없음)을 모사해
#     판독기 **단독**의 강도를 잰다 — 음성 대조(수리 전 훅)에서 이 입력은 spawn 을 열었다.
_TRIAGE_INJECT = _TRIAGE_CALL_LOG + (
    "if cmd == 'hook-triage':\n"
    "    sys.stdin.read()\n"
    "    sys.stdout.write('record: rc=1\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('path: /tmp/lane\\nmachine-origin: human\\n'); sys.stdout.flush()\n"
    "    sys.stdout.write('machine-origin: machine\\n'); sys.stdout.flush()\n"
    "    raise SystemExit(0)\n"
    "raise SystemExit(64)\n")
# 구팩 재현: hook-triage 는 stdin 무소비 EX_USAGE(64) 거부(v0.14.25 실측 거동), 구 3서브커맨드만 존재
_TRIAGE_OLDPACK = _TRIAGE_CALL_LOG + (
    "if cmd == 'hook-triage':\n"
    "    sys.stderr.write('usage: javis_mission.py ...\\n')\n"
    "    raise SystemExit(64)\n"
    "if cmd == 'record':\n"
    "    sys.stdin.read(); raise SystemExit(1)\n"
    "if cmd == 'path':\n"
    "    sys.stdout.write('/LEGACY-PATH-PIN\\n'); raise SystemExit(0)\n"
    "if cmd == 'machine-origin':\n"
    "    sys.stdin.read(); sys.stdout.write('machine-origin: human\\n'); raise SystemExit(1)\n"
    "raise SystemExit(64)\n")


def _run_hook_mission_mock(fails, name, mission_py, prompt="너는 마스터다"):
    """훅을 격리 실행하되 javis_mission.py 를 **목**으로 바꾼다(P0-5 소비면 계측 전용).

    실훅은 MISSION 을 형제(../bin) 우선으로 해소하므로 훅+_lib.sh 를 격리 디렉터리로 복사하고,
    형제 bin 에 실 감지기(javis_detect.py — DETECT 는 진짜 판정) + 목 javis_mission.py 를 심는다
    (관례: run_bootstrap_health H-MISSION-1 ⓕ · 위 note_cp949_survival 뮤테이션 하네스 —
    실 저장소 파일은 건드리지 않는다). 반환 (CompletedProcess|None, calls: [서브커맨드,...]).
    """
    root = tempfile.mkdtemp()
    hooks = os.path.join(root, "hooks")
    os.makedirs(hooks)
    shutil.copy(HOOK, os.path.join(hooks, "role-bootstrap.sh"))
    shutil.copy(HOOK_BODY, os.path.join(hooks, "role-bootstrap-legacy.sh"))
    shutil.copy(os.path.join(os.path.dirname(HOOK), "_lib.sh"), os.path.join(hooks, "_lib.sh"))
    binf = os.path.join(root, "bin")
    os.makedirs(binf)
    shutil.copy(os.path.join(BIN, "javis_detect.py"), os.path.join(binf, "javis_detect.py"))
    with open(os.path.join(binf, "javis_mission.py"), "w") as f:
        f.write(mission_py)
    home = tempfile.mkdtemp()
    pack = tempfile.mkdtemp()
    mockbin = tempfile.mkdtemp()
    os.makedirs(os.path.join(pack, "bin"), exist_ok=True)
    with open(os.path.join(pack, "bin", "javis_bootstrap.py"), "w") as f:
        f.write(_MOCK_BOOT)
    cysp = os.path.join(mockbin, "cys")
    with open(cysp, "w") as f:
        # boot-intent 기본값 = 구 CLI 모사(R3-P2-7 ⓐ — 이 하네스는 종전 spawn 폴백 leg 를 잰다)
        f.write('#!/bin/bash\n[ "$1" = surface-role ] && { echo ""; exit 0; }\n'
                '[ "$1" = boot-intent ] && { echo "error: unrecognized subcommand '
                "'boot-intent'\" >&2; exit 2; }\nexit 0\n")
    os.chmod(cysp, 0o755)
    env = dict(os.environ)
    env["HOME"] = home
    env["CYS_PACK_DIR"] = pack
    env["PATH"] = mockbin + os.pathsep + env.get("PATH", "")
    for k in ("CYS_SOCKET", "CYS_STATE_DIR", "AITERM_SURFACE_ID", "CYS_MISSION"):
        env.pop(k, None)
    env["CYS_SURFACE_ID"] = "7"
    try:
        r = subprocess.run(["bash", os.path.join(hooks, "role-bootstrap.sh")],
                           input=json.dumps({"prompt": prompt}),
                           capture_output=True, text=True, timeout=30, env=env)
    except Exception as e:
        fails.append("P0-5(%s): 훅 실행 실패(계측 불능은 통과가 아니다): %s" % (name, e))
        return None, []
    calls = []
    calls_p = os.path.join(home, "mission-calls.log")
    if os.path.isfile(calls_p):
        with open(calls_p, encoding="utf-8") as f:
            calls = [ln.split()[0] for ln in f.read().splitlines() if ln.strip()]
    return r, calls


def triage_batch_protocol(fails):
    """★P0-5/R3-P05-1/R3-RISK-3 회귀 핀 — hook-triage 배치 왕복의 훅 소비면 3계약.

    ⓐ 정상 배치: mission 계열 파이썬 왕복이 선언 프롬프트에서 **정확히 1회**(종전 3회 —
       record·path·machine-origin), record rc 라인·path 라인이 성공 note 문안까지 도달하고
       MO 토큰(human)으로 spawn 이 열린다.
    ⓑ 중도 사망(record 라인만 나온 채 즉사): record 판정은 생존(stderr record=1)하고 MO 는
       **단독 재왕복 1회**(자기 5s)까지 태운 뒤에도 토큰 부재 → 판정 불가 fail-closed
       **무스폰**. 크래시를 구팩 폴백으로 오독해 record·path 까지 3왕복 재시도하지는 않는다
       (폴백 신호는 rc=64 하나 — wedge 기계 최악 지연 방어는 MO 5s 만 얹어 13s 로 유지).
    ⓔ ★record 지연(R2-1): record 가 배치 예산을 먹어 MO 라인이 창 밖으로 밀려도 MO 단독
       재왕복이 판정을 살려 spawn 이 열린다 — '독립 데드라인' 성질의 회귀 핀.
    ⓕ ★path 개행 주입(R2-2): 위조 `machine-origin:` 줄은 토큰 줄 개수를 2로 만들어 판정
       불가(무스폰)로 접힌다 — 진짜 판정(machine)을 뒤집지 못한다.
    ⓒ 구팩 스큐(rc=64): stderr 1줄 고지 후 종전 3왕복 경로로 폴백(조용한 강등 금지) —
       레거시 path·record 소비가 그대로 살아 spawn 까지 완주한다(1릴리스 병존 계약).
    ⓓ CRLF 쌍둥이(Windows 실출력): 라인 종결이 \\r\\n 이어도 ⓐ 와 동일 소비 — record rc·path·
       MO 전부 생존하고 note 에 \\r 잔류가 없다(훅 캡처 직후 1회 정규화의 핀).
    """
    # ⓐ 정상 배치 — 왕복 1회 + 라인 소비가 note 문안까지 관통
    r, calls = _run_hook_mission_mock(fails, "정상 배치", _TRIAGE_FULL)
    if r is not None:
        ctx = _note_ctx(r.stdout)
        if "발화됨" not in r.stdout:
            fails.append("P0-5 ⓐ: MO 토큰 human 인데 spawn 이 열리지 않았다: %r"
                         % (r.stdout + r.stderr)[:300])
        if calls != ["hook-triage"]:
            fails.append("P0-5 ⓐ: mission 계열 왕복이 1회(hook-triage)가 아니다 — 배치 채택의 "
                         "정량 근거(3→1) 미달성: %s" % calls)
        if "exit 1(임무 없음)" not in ctx:
            fails.append("P0-5 ⓐ: record 라인(rc=1)이 note 의 관측 파생 문안으로 소비되지 "
                         "않았다: %r" % ctx[:300])
        if "/TRIAGE-PATH-PIN" not in ctx:
            fails.append("P0-5 ⓐ: path 라인이 note 의 대장 경로로 소비되지 않았다: %r" % ctx[:300])
        if "record=1" not in r.stderr:
            fails.append("P0-5 ⓐ: 배치 진단 1줄(record=1)이 stderr 에 없다: %r" % r.stderr[:300])
        if "구팩" in r.stderr:
            fails.append("P0-5 ⓐ: 정상 배치를 구팩으로 오판해 폴백했다: %r" % r.stderr[:300])
        if r.returncode != 0:
            fails.append("P0-5 ⓐ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    # ⓑ 중도 사망 — record 생존 + MO fail-closed 무스폰 + 재시도 없음
    r, calls = _run_hook_mission_mock(fails, "중도 사망", _TRIAGE_MIDDEATH)
    if r is not None:
        if "발화됨" in r.stdout:
            fails.append("P0-5 ⓑ: MO 라인 없는 중도 사망인데 spawn 이 열렸다(fail-open — "
                         "치명): %r" % r.stdout[:300])
        if "record=1" not in r.stderr:
            fails.append("P0-5 ⓑ: 중도 사망에서 record 판정이 생존하지 않았다(증분 라인 "
                         "프로토콜 소실): %r" % r.stderr[:300])
        if "기계유래 판정 불가" not in r.stderr:
            fails.append("P0-5 ⓑ: MO 토큰 부재가 판정 불가 fail-closed 로 접히지 않았다: %r"
                         % r.stderr[:300])
        if "선언 아님이 아니라 판정 불가다" not in r.stdout:
            fails.append("P0-5 ⓑ: 판정 불가 주입문(선언 아님/판정 불가 분리)이 없다: %r"
                         % r.stdout[:300])
        # ★핀 개정(R2-1 · 2026-08-26): 종전 단언은 `calls == ["hook-triage"]`(재왕복 0)이었다.
        #   그 단언의 근거는 '크래시까지 **구팩 3왕복** 폴백으로 접으면 wedge 최악 지연이
        #   8s+15s' 였는데, 여기서 도는 것은 3왕복 폴백이 아니라 **MO 단독 재왕복 1회**(5s)라
        #   최악 지연은 8s+5s=13s — 구 3왕복 15s 보다 짧다. 즉 wedge 방어는 유지되고, MO 의
        #   독립 예산만 복원된다. record 재실행이 없다는 것(대장 이중 기록 금지)이 핵심이다.
        if calls != ["hook-triage", "machine-origin"]:
            fails.append("P0-5 ⓑ: 중도 사망의 왕복 구성이 [hook-triage, machine-origin] 이 "
                         "아니다 — MO 독립 예산 복원(R2-1) 또는 record 무재실행 계약 위반: %s"
                         % calls)
        if r.returncode != 0:
            fails.append("P0-5 ⓑ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    # ⓒ 구팩 스큐 — rc64 → stderr 1줄 + 종전 3왕복 폴백 완주
    r, calls = _run_hook_mission_mock(fails, "구팩 폴백", _TRIAGE_OLDPACK)
    if r is not None:
        if "구팩" not in r.stderr or "폴백" not in r.stderr:
            fails.append("P0-5 ⓒ: 구팩 폴백이 침묵 강등이다(stderr 1줄 고지 계약): %r"
                         % r.stderr[:300])
        if "발화됨" not in r.stdout:
            fails.append("P0-5 ⓒ: 구팩 폴백 경로에서 spawn 이 죽었다(1릴리스 병존 파괴): %r"
                         % (r.stdout + r.stderr)[:300])
        if calls != ["hook-triage", "record", "path", "machine-origin"]:
            fails.append("P0-5 ⓒ: 폴백 왕복 구성이 종전 3왕복(+probe)이 아니다: %s" % calls)
        ctx = _note_ctx(r.stdout)
        if "/LEGACY-PATH-PIN" not in ctx:
            fails.append("P0-5 ⓒ: 레거시 path 소비가 note 에 도달하지 않았다: %r" % ctx[:300])
        if r.returncode != 0:
            fails.append("P0-5 ⓒ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    # ⓓ CRLF 쌍둥이(Windows 실출력) — \r\n 라인 종결에서도 ⓐ 와 동일 소비(정규화 회귀 핀)
    r, calls = _run_hook_mission_mock(fails, "CRLF 배치", _TRIAGE_FULL_CRLF)
    if r is not None:
        ctx = _note_ctx(r.stdout)
        if "record=1" not in r.stderr:
            fails.append("P0-5 ⓓ: CRLF 라인 종결에서 record rc 소비가 죽었다(sed '$' 앵커 \\r "
                         "비관용 — Windows 임무 게이트 영구 폐쇄 회귀): %r" % r.stderr[:300])
        if "발화됨" not in r.stdout:
            fails.append("P0-5 ⓓ: CRLF 배치에서 spawn 이 열리지 않았다: %r"
                         % (r.stdout + r.stderr)[:300])
        if calls != ["hook-triage"]:
            fails.append("P0-5 ⓓ: CRLF 배치를 오독해 재왕복했다(왕복 1회 계약): %s" % calls)
        if "exit 1(임무 없음)" not in ctx:
            fails.append("P0-5 ⓓ: CRLF 에서 record 라인이 note 의 관측 파생 문안으로 소비되지 "
                         "않았다(허위 '모듈 부재' 문안 의심): %r" % ctx[:300])
        if "/TRIAGE-PATH-PIN" not in ctx:
            fails.append("P0-5 ⓓ: CRLF 에서 path 라인이 note 로 소비되지 않았다: %r" % ctx[:300])
        if "\r" in ctx:
            fails.append("P0-5 ⓓ: note 에 \\r 잔류 — CRLF 정규화가 캡처 전체(path·MO 포함)에 "
                         "적용되지 않았다: %r" % ctx[:300])
        if r.returncode != 0:
            fails.append("P0-5 ⓓ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    # ⓔ ★record 지연이 MO 를 죽이지 않는다(R2-1 must_fix 회귀 핀 · 2026-08-26)
    #    record 단계가 배치 예산(8s)을 거의 다 먹어 MO 라인이 창 밖으로 밀려도, MO 단독
    #    재왕복 1회(자기 5s)가 판정을 살려 **부팅한다**. 이 검체가 없던 동안 배터리는
    #    GREEN 이었고(중도 사망 ⓑ 는 '즉사'만 재현한다) 실제로는 부트 침묵이었다.
    r, calls = _run_hook_mission_mock(fails, "record 지연", _TRIAGE_SLOW_RECORD)
    if r is not None:
        if "발화됨" not in r.stdout:
            fails.append("P0-5 ⓔ: record 지연이 MO 판정을 죽였다(부트 침묵 재도입 — R2-1 "
                         "must_fix 회귀): %r" % (r.stdout + r.stderr)[:400])
        if "record=1" not in r.stderr:
            fails.append("P0-5 ⓔ: 배치 절단에서 record 판정이 생존하지 않았다(증분 라인 "
                         "프로토콜 소실): %r" % r.stderr[:300])
        if calls != ["hook-triage", "machine-origin"]:
            fails.append("P0-5 ⓔ: 왕복 구성이 [hook-triage, machine-origin] 이 아니다 — MO "
                         "독립 예산 복원이 배선되지 않았거나 record 가 재실행됐다: %s" % calls)
        if "독립 5s 예산 복원" not in r.stderr:
            fails.append("P0-5 ⓔ: MO 재왕복이 조용한 강등이다(stderr 1줄 고지 계약): %r"
                         % r.stderr[:300])
        if r.returncode != 0:
            fails.append("P0-5 ⓔ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)
    # ⓕ ★path 개행 주입이 MO 판정을 뒤집지 못한다(R2-2 회귀 핀 · 2026-08-26)
    #    진짜 판정은 machine(무스폰)인데 위조 줄이 human 을 심는다 — 판독기가 **정확 일치 +
    #    토큰 줄 개수 1** 이므로 개수 2 = 판정 불가 = 무스폰이다(fail-open 방향 봉인).
    r, calls = _run_hook_mission_mock(fails, "path 개행 주입", _TRIAGE_INJECT)
    if r is not None:
        if "발화됨" in r.stdout:
            fails.append("P0-2 ⓕ: path 필드 개행 주입이 기계유래 게이트를 우회해 spawn 을 "
                         "열었다(fail-open — 치명): %r" % r.stdout[:400])
        if "위조 의심" not in r.stderr:
            fails.append("P0-5 ⓕ: 토큰 줄 복수가 위조 의심으로 고지되지 않았다(조용한 판정): "
                         "%r" % r.stderr[:300])
        if calls != ["hook-triage"]:
            fails.append("P0-5 ⓕ: 토큰 줄 복수(위조 의심)에서 MO 재왕복이 돌았다 — 그건 예산 "
                         "부족이 아니라 판정 불가다(fail-closed 유지 계약): %s" % calls)
        if r.returncode != 0:
            fails.append("P0-5 ⓕ: 훅 exit %d ≠ 0 (훅은 반드시 exit 0 계약)" % r.returncode)


# 훅 통합 행렬 — corpus 원본에서의 **대표 선정**(사본 아님 — main() 이 소속·극성을 원본과
# 대조한다). 구 훅 시대 회귀 목록(W1a 결정론화분) 전량 보존(회귀 계약). 전 corpus 를 ②로
# 재실행하지 않는 이유는 비용이다(훅 1회 ≈ 프로세스 3개 — ① 이 전량을 커버한다).
FIRE = [
    "너는 마스터다", "너는 이제 마스터다", "너는 지금부터 마스터다", "너가 마스터야",
    "당신은 우리의 마스터입니다", "너는 마스터로 각성하라", "you are the master",
    "지금부터 너는 마스터가 된다",
]
SKIP = [
    "너는 마스터가 아니다", "'너는 마스터다'가 무슨 뜻이야?", "너는 마스터다라고 말하지 마",
    "오늘 작업 지시해줘", "너는 워커다", "마스터 브랜치를 확인해줘", "너는 오늘 마스터 브랜치 봐",
]
# 훅으로 교차 확인하는 W1b/W-A2 대표 케이스(함수 검증과 이중화하지 않되 배선은 확인).
# ★"너는 마스터다 오늘 뭐부터 할까?"(무구두점 후속 의문 = **발화**)를 반드시 포함한다 — 신
#   스펙에서 극성이 뒤집힌 대표 케이스의 훅 배선 핀(구 주석을 좇은 역수리의 조기 검출 장치).
HOOK_NEW_FIRE = ["너는 마스터다. 오늘 뭐부터 할까?", "너는 마스터다 오늘 뭐부터 할까?",
                 "네가 마스터다", "당신이 마스터다"]
HOOK_NEW_SKIP = ["'네가 마스터다'가 무슨 뜻?"]
# A3 allowlist 반전 — 구 denylist가 통과시켰던 좌석들이 전부 차단돼야 한다.
NON_MASTER_ROLES = ["worker", "cso", "reviewer-gemini", "reviewer-codex",
                    "worker-2", "cso-1", "reviewer-claude-1", "reviewer-grok",
                    "verifier", "unknown-role"]


def main():
    fails = []
    fn_matrix(fails)

    # 0. 훅 대표 선정 ↔ corpus 원본 대조 — 선정이 사본으로 퇴화(원본과 극성·문구가 어긋난 채
    #    잔존)하는 드리프트를 기계로 잡는다(이 파일이 리터럴을 갖는 유일한 이유가 '선정'이다).
    for t in FIRE + HOOK_NEW_FIRE:
        if t not in CORPUS_FIRE:
            fails.append("훅 대표 FIRE 케이스가 corpus 원본에 없다(선정≠사본 계약 위반): %r" % t)
    for t in SKIP + HOOK_NEW_SKIP:
        if t not in CORPUS_SKIP:
            fails.append("훅 대표 SKIP 케이스가 corpus 원본에 없다(선정≠사본 계약 위반): %r" % t)

    # 1. 감지 행렬 — 발화해야 함
    for p in FIRE + HOOK_NEW_FIRE:
        fired, _ = _run_hook(p)
        if not fired:
            fails.append("FALSE-NEGATIVE(발화 안 됨): %r" % p)
    # 2. 감지 행렬 — 무시해야 함
    for p in SKIP + HOOK_NEW_SKIP:
        fired, _ = _run_hook(p)
        if fired:
            fails.append("FALSE-POSITIVE(오발화): %r" % p)
    # 3. ★A3 role 게이트 allowlist 반전 — 비-master 좌석 전부 차단(미지 role 포함)
    for role in NON_MASTER_ROLES:
        fired, out = _run_hook("너는 마스터다", surface_role=role)
        if fired:
            fails.append("A3 회귀(%s pane에서 마스터 부트 오발화 — denylist 잔존): %r" % (role, out[:200]))
    # 4. role-aware — master·미claim은 발화 허용
    for role in ("master", ""):
        fired, _ = _run_hook("너는 마스터다", surface_role=role)
        if not fired:
            fails.append("role='%s'에서 발화 안 됨(정상 마스터 선언 차단)" % (role or "미claim"))

    # 5. ★A2 surface 이중 게이트 — cys 밖(CYS_SURFACE_ID·AITERM_SURFACE_ID 둘 다 부재)은 무발화
    fired, _ = _run_hook("너는 마스터다", surface_env=False)
    if fired:
        fails.append("A2 회귀: surface env 부재(비-cys 터미널)에서 마스터 부트 오발화")

    # 6. ★A5 surface-role 판정 불가(rc≠0) — 무발화 + 로그(빈값=미claim 과 분리)
    fired, out = _run_hook("너는 마스터다", surface_role="", role_rc=3)
    if fired:
        fails.append("A5 회귀: surface-role 판정 불가인데 발화(fail-open)")
    if "판정 불가" not in out:
        fails.append("A5 회귀: 판정 불가가 침묵으로 접혔다(로그 없음): %r" % out[:200])

    # 7. ★G9 LC_ALL=C 파리티 — 로케일이 감지 결과를 바꾸면 안 된다(구 grep은 바이트를 셌다)
    for p in ("너는 이제 마스터다", "너는" + "가" * javis_detect.FILLER_MAX + "마스터다"):
        fired_c, _ = _run_hook(p, extra_env={"LC_ALL": "C", "LANG": "C"})
        if not fired_c:
            fails.append("G9 회귀: LC_ALL=C 에서 미발화 %r" % p)
    fired_c, _ = _run_hook("'너는 마스터다'가 무슨 뜻이야?", extra_env={"LC_ALL": "C", "LANG": "C"})
    if fired_c:
        fails.append("G9 회귀: LC_ALL=C 에서 억제 케이스가 오발화")

    # 8. ★G25 감지창 200 **문자** — 한글 경계에서 훅 배선까지 문자 단위인지(셸 슬라이스 제거 확인)
    decl = "너는 마스터다"
    fired, _ = _run_hook("가" * (javis_detect.WINDOW_CHARS - len(decl)) + decl)
    if not fired:
        fails.append("G25 회귀: 한글 200자 창 경계 선언이 훅에서 미발화(바이트 슬라이스 잔존)")
    fired, _ = _run_hook("가" * javis_detect.WINDOW_CHARS + decl)
    if fired:
        fails.append("G25 회귀: 감지창 밖 선언이 훅에서 발화")

    # 9. ★W-A0 알림 파이프 점유 해제 — wedge 데몬에서도 훅 exit ≈ stdout EOF(프롬프트 먹통 차단)
    notify_pipe_release(fails)

    # 10. ★W-F2 note 인코딩 가드 — cp949 스큐에서도 통보 생존(+가드 제거 음성 대조)
    note_cp949_survival(fails)

    # 11. ★P0-4/R3-P04-1 성공 note 명명식 포맷 — 'JSON 1줄' 생존 + rc0/rc6 예보 분기
    note_success_named_format(fails)

    # 12. ★P0-5 hook-triage 배치 왕복 — 왕복 1회·중도 사망 record 생존·구팩 폴백·CRLF 쌍둥이
    triage_batch_protocol(fails)

    if fails:
        print("FAIL (%d):" % len(fails))
        for f in fails:
            print("  -", f)
        sys.exit(1)
    print("PASS: 함수 corpus(원본 fixtures) %d발화/%d무시(+filler·창 경계·CLI exit 계약) + "
          "훅 %d발화/%d무시 + A3 allowlist %dskip/2fire + A2 1skip + A5 판정불가 1skip + "
          "G9 파리티 3 + G25 경계 2 + W-A0 파이프해제 1 + W-F2 cp949 4생존(P2 frontdoor 포함)/"
          "음성대조 3 + P0-4 note 명명식 rc0/rc6 2 + P0-5 triage 배치 6(왕복1·중도사망·구팩폴백·"
          "CRLF·★record지연 MO독립예산·★path개행주입 위조차단)"
          % (len(CORPUS_FIRE), len(CORPUS_SKIP),
             len(FIRE) + len(HOOK_NEW_FIRE), len(SKIP) + len(HOOK_NEW_SKIP),
             len(NON_MASTER_ROLES)))


if __name__ == "__main__":
    main()
