#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_hook_launcher_split.py — UserPromptSubmit 훅 런처/본체 분할 계약 (0.14.30 W-A A2).

명세 §2-1(자기완결 런처) · 시뮬 §4 T1-1(무발화 고지)·T1-2(BOM)·T2-1(경로 2벌)·T2-2(비동기 계속)의
검체다. 검체 ID: **H-HOOK-IN-1/2/3 · H-HOOK-SELF-1 · H-HOOK-WIN-PATH**(명세 §6).

무엇을 막는가:
 ①★**단일 소유** 파손 — `cys hook` 이 처리했는데 본체까지 도는(또는 그 반대로 **둘 다 안 도는**)
   경로. 후자가 특히 치명적이다: 현행 CLI 에서 `rc 0` 은 '처리완료'가 아니라 `HOOK_EXIT_PROCEED`
   (=종전 게이트로 **계속 진행**)이고 `--input` 인자 자체가 없다(실측 rc 2 + unexpected argument).
   명세 초안대로 rc 0 을 '처리완료'로 읽으면 W-B 착지 전까지 **모든 마스터 선언이 무음 사망**한다.
   그래서 런처는 rc 를 해석하기 전에 **능력 프로브**(`--help` 에 `--input` 이 있는가)를 돌린다.
 ②자기완결 파손 — 런처가 공용 프리루드에 의존하면 그 파일이 없는 배선에서 통째로 강등된다.
 ③'반드시 exit 0' 파손 — 본체 부재·상태 디렉터리 불가에서 비0 이 나가 사람의 프롬프트를 깬다.
 ④무발화가 **무음**이 되는 것 — 판정 불가는 고지 1줄과 함께여야 한다(T1-1).
 ⑤경로 1벌 회귀 — Windows 에서 sh 명령용 경로와 네이티브 인자용 경로를 섞으면 파일은 만들어지고
   인자는 못 읽는 상태가 된다(T2-1).
 ⑥비-cys 터미널 부작용 부활 — 좌석 게이트가 런처 최선두에 없으면 상태 디렉터리 생성·데몬 왕복이
   임의 세션에서 되살아난다.

밀폐: 임시 디렉터리에 런처를 복사하고 **스텁 본체**(실행 사실과 `$1` 내용을 파일로 남긴다)를
나란히 둔다. `cys`·`cygpath` 도 전부 스텁이라 라이브 데몬·실 본체 무접촉이고 스폰 0이다.
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 HOOK-LAUNCHER-SPLIT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_hook_launcher_split.py
"""
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
HOOKS = os.path.join(os.path.dirname(BIN), "hooks")
LAUNCHER = os.path.join(HOOKS, "role-bootstrap.sh")
BODY = os.path.join(HOOKS, "role-bootstrap-legacy.sh")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def w(path, body, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def rd(path):
    # ★newline="" 필수: 기본(universal newlines)으로 읽으면 파이썬이 `\r\n` 을 `\n` 으로
    #   **번역**해 CRLF 보존 여부를 재는 검체가 스스로 증거를 지운다(계측기가 결과를 만든다).
    try:
        with io.open(path, encoding="utf-8", newline="") as f:
            return f.read()
    except OSError:
        return ""


STUB_BODY = """#!/bin/sh
# 스텁 본체 — 실행 사실과 인자 파일 내용을 남긴다(실 본체 무접촉).
echo "BODY-RAN" >> "$MARK"
printf '%s' "${PYTHONDONTWRITEBYTECODE:-}" > "$MARK.seal"
if [ -n "${1:-}" ] && [ -f "$1" ]; then
  printf '%s' "$(cat "$1")" > "$MARK.arg"
  printf '%s' "$1" > "$MARK.argpath"
  rm -f "$1"
else
  printf '%s' "$(cat)" > "$MARK.stdin"
fi
exit 0
"""


def stub_cys(rc_input=None, supports_input=True, log=None):
    """가짜 cys. supports_input=False 면 `--help` 에 --input 이 없다(구 CLI 모사)."""
    helpout = ("Usage: cys hook user-prompt-submit [OPTIONS]\\n\\nOptions:\\n"
               + ("      --input <FILE>  hook payload\\n" if supports_input else "")
               + "  -h, --help  Print help\\n")
    return (
        "#!/bin/sh\n"
        'printf "%s\\n" "cys $*" >> "$CYSLOG"\n'
        'if [ "$1" = hook ] && [ "$3" = --help ]; then printf "' + helpout + '"; exit 0; fi\n'
        'if [ "$1" = hook ] && [ "$3" = --input ]; then exit ' + str(rc_input if rc_input is not None else 5) + '; fi\n'
        "exit 0\n")


def lab(root, name):
    """런처+스텁본체+스텁 cys 를 갖춘 격리 실험실. 반환 (hooks_dir, env, mark)."""
    d = os.path.join(root, name)
    hooks = os.path.join(d, "hooks")
    binp = os.path.join(d, "bin")
    state = os.path.join(d, "state")
    os.makedirs(hooks); os.makedirs(binp)
    shutil.copy(LAUNCHER, os.path.join(hooks, "role-bootstrap.sh"))
    mark = os.path.join(d, "mark")
    env = dict(os.environ)
    for k in ("AITERM_SURFACE_ID", "CYS_PACK_DIR", "CYS_MISSION", "CYS_SOCKET"):
        env.pop(k, None)
    # ★CYS_PACK_DIR 를 실험실로 고정하는 이유(밀폐): 런처의 프리루드 2단 폴백은 키가 없으면
    #   `$HOME/.cys/pack/hooks/_lib.sh`(사용자 실팩)로 간다 — 그러면 이 검체가 기계에 따라
    #   다른 것을 재게 된다. 실험실을 레인으로 지정하면 레인 가드도 같은 팩으로 통과한다.
    env.update({"CYS_SURFACE_ID": "7", "CYS_STATE_DIR": state, "MARK": mark,
                "CYS_PACK_DIR": d, "CYSLOG": os.path.join(d, "cys.log"),
                "PATH": binp + os.pathsep + env.get("PATH", "")})
    return hooks, binp, state, env, mark


def run(hooks, env, payload='{"prompt":"너는 마스터다"}', shell="sh"):
    return subprocess.run([shell, os.path.join(hooks, "role-bootstrap.sh")],
                          input=payload, capture_output=True, text=True, timeout=90, env=env)


root = tempfile.mkdtemp()
try:
    # ───────── H-HOOK-IN-1: 런처가 stdin 을 파일로 받아 본체에 넘긴다 ─────────
    hooks, binp, state, env, mark = lab(root, "in1")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    r = run(hooks, env)
    check("IN-1a 런처 exit 0", r.returncode == 0, repr((r.returncode, r.stderr[-200:])))
    check("IN-1b 본체가 정확히 1회 실행", rd(mark).count("BODY-RAN") == 1, repr(rd(mark)))
    check("IN-1c 본체가 stdin 이 아니라 **$1 파일**로 받았다",
          rd(mark + ".arg") == '{"prompt":"너는 마스터다"}' and not os.path.exists(mark + ".stdin"),
          repr(rd(mark + ".arg")))
    check("IN-1d 입력 파일은 상태 디렉터리 아래에 만들어진다",
          rd(mark + ".argpath").startswith(state), repr(rd(mark + ".argpath")))
    check("IN-1e 소비 후 잔재 0(본체가 지운다)",
          not [n for n in os.listdir(state) if n.startswith("hook-input-")], str(os.listdir(state)))

    # ───────── H-HOOK-IN-2: 단일 소유 — 0/3 이면 본체 미실행 · 그 밖은 본체 1회 ─────────
    # ★rc 계약(master 판정 2026-09-04 ①): **6=처리완료 · 3=억제** 만 본체를 건너뛴다.
    #   rc 0 은 `HOOK_EXIT_PROCEED`(종전 의미 불변)라 **반드시 본체가 돌아야 한다** — 0 을
    #   처리완료로 읽는 순간 모든 마스터 선언이 무음 사망한다(이 행이 그 회귀의 유일한 관측점).
    matrix = [(6, False), (3, False), (0, True), (1, True), (4, True), (5, True),
              (127, True), (2, True)]
    for rc, expect_body in matrix:
        hooks, binp, state, env, mark = lab(root, "in2-%d" % rc)
        w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
        w(os.path.join(binp, "cys"), stub_cys(rc_input=rc, supports_input=True), 0o755)
        r = run(hooks, env)
        ran = rd(mark).count("BODY-RAN")
        check("IN-2 rc=%d → 본체 %s" % (rc, "미실행" if not expect_body else "정확히 1회"),
              r.returncode == 0 and ran == (1 if expect_body else 0),
              repr((r.returncode, ran, r.stdout[:120])))
    # ★둘 다 도는 경로가 없다: 위 매트릭스 전체에서 ran ∈ {0,1} 이고 rc 0/3 에서만 0 이다.
    hooks, binp, state, env, mark = lab(root, "in2-dup")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(rc_input=6, supports_input=True), 0o755)
    run(hooks, env)
    check("IN-2z 처리완료(rc 6)에서 입력 파일도 회수된다(런처가 지운다)",
          not [n for n in os.listdir(state) if n.startswith("hook-input-")], str(os.listdir(state)))

    # ───────── ★능력 프로브: 구 CLI(--input 미지원)는 위임 자체를 하지 않는다 ─────────
    hooks, binp, state, env, mark = lab(root, "probe-old")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(rc_input=0, supports_input=False), 0o755)
    r = run(hooks, env)
    log = rd(os.path.join(os.path.dirname(hooks), "cys.log"))
    check("PROBE-1 구 CLI 는 rc 0 을 내도 본체가 돈다(무음 사망 봉인)",
          rd(mark).count("BODY-RAN") == 1, repr((rd(mark), log)))
    check("PROBE-2 구 CLI 에는 --input 위임을 **시도조차** 하지 않는다",
          "--input" not in log and "--help" in log, repr(log))

    # ───────── H-HOOK-IN-3: BOM·CRLF 무해(T1-2) ─────────
    for tag, payload in (("BOM", "﻿" + '{"prompt":"너는 마스터다"}'),
                         ("CRLF", '{\r\n"prompt":"너는 마스터다"\r\n}')):
        hooks, binp, state, env, mark = lab(root, "in3-" + tag)
        w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
        w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
        r = run(hooks, env, payload=payload)
        check("IN-3 %s 입력이 바이트 그대로 본체에 도달" % tag,
              r.returncode == 0 and rd(mark).count("BODY-RAN") == 1
              and rd(mark + ".arg") == payload,
              repr((r.returncode, rd(mark + ".arg")[:60])))

    # ───────── H-HOOK-SELF-1: 프리루드 없는 디렉터리에서도 런처는 돈다 ─────────
    hooks, binp, state, env, mark = lab(root, "self1")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    check("SELF-1a 실험실에 공용 프리루드 파일이 없다(전제)",
          not os.path.exists(os.path.join(hooks, "_lib.sh")))
    r = run(hooks, env)
    check("SELF-1b 자기완결 실행 — exit 0 · 본체 위임 성립",
          r.returncode == 0 and rd(mark).count("BODY-RAN") == 1, repr((r.returncode, r.stderr[-200:])))
    src = rd(LAUNCHER)
    check("SELF-1c 런처는 프리루드를 **하드 의존하지 않는다**(부재 시 조용한 강등 · loud-skip 0)",
          "_lib.sh 소실" not in src and "|| :" in src, repr([l for l in src.splitlines() if "_lib.sh" in l]))
    check("SELF-1d 런처가 프리루드 규약 심볼을 쓰지 않는다(소비 훅 판정에서 제외되어야 한다)",
          not any(m in src for m in ("CYS_PY", "PYBIN", "python3", "cys_norm_", "cys_is_abs",
                                     "cys_native_path", "cys_require_surface", "cys_have_surface",
                                     "cys_path_has_prefix", "cys_shquote")),
          repr([m for m in ("CYS_PY", "PYBIN", "python3", "cys_native_path") if m in src]))
    for shbin in ("sh", "dash", "bash"):
        if shutil.which(shbin) is None:
            print("SKIP SELF-1e %s 부재" % shbin)
            continue
        rr = subprocess.run([shbin, "-n", LAUNCHER], capture_output=True, text=True)
        check("SELF-1e %s -n 문법 통과(런처는 진짜 POSIX sh)" % shbin, rr.returncode == 0,
              repr(rr.stderr[-200:]))

    # ───────── H-HOOK-WIN-PATH: 경로 2벌(T2-1) ─────────
    hooks, binp, state, env, mark = lab(root, "win")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(rc_input=6, supports_input=True), 0o755)
    # cygpath 스텁: -u 는 항등(POSIX 복원) · -w 는 네이티브 표기 모사(접두 부착)
    w(os.path.join(binp, "cygpath"),
      '#!/bin/sh\ncase "$1" in\n  -u) printf "%s" "$2" ;;\n'
      '  -w) printf "WINPFX::%s" "$2" ;;\n  *) printf "%s" "$2" ;;\nesac\n', 0o755)
    r = run(hooks, env)
    log = rd(os.path.join(os.path.dirname(hooks), "cys.log"))
    check("WIN-1a 파일 조작은 POSIX 표기로 한다(상태 디렉터리가 실제로 생성됨)",
          os.path.isdir(state), state)
    check("WIN-1b `cys --input` 인자는 **네이티브 표기**로 넘어간다(경로 2벌 분리)",
          "--input WINPFX::" in log, repr(log))
    check("WIN-1c 네이티브 표기를 파일 조작에 쓰지 않았다(WINPFX 디렉터리 미생성)",
          not os.path.exists(os.path.join(os.path.dirname(hooks), "WINPFX::")),
          str(os.listdir(os.path.dirname(hooks))))

    # ───────── ★H-HOOK-CWD-1: 본체 경로가 **CWD 에 의존하지 않는다**(A2 회귀 · W-C 적발) ─────
    #   결함: `_LEGACY="${0%/*}/…"` 는 `$0` 에 슬래시가 없으면 `${0%/*}` 가 `$0` 를 그대로
    #   돌려주고, 그 폴백이 **CWD 상대** `./role-bootstrap-legacy.sh` 였다. PATH 경유 호출
    #   (`bash -c 'role-bootstrap.sh'`)처럼 `$0` 가 이름뿐이면, 본체가 **실재하는데도**
    #   '부재 — 부트 미발화'로 판정된다(windows-health H-WIN-11·H-MISSION-1·H-DETECT-10 적색).
    #   무음이 아니라 **거짓 고지**라 더 나쁘다 — 팩 재설치를 처방하지만 팩은 멀쩡하다.
    hooks, binp, state, env, mark = lab(root, "cwd")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    os.chmod(os.path.join(hooks, "role-bootstrap.sh"), 0o755)
    elsewhere = os.path.join(root, "cwd-elsewhere")
    os.makedirs(elsewhere, exist_ok=True)
    env_path = dict(env, PATH=hooks + os.pathsep + env["PATH"])   # 런처를 PATH 로 찾게 한다
    rc = subprocess.run(["bash", "role-bootstrap.sh"], input='{"prompt":"너는 마스터다"}',
                        capture_output=True, text=True, timeout=90, env=env_path, cwd=elsewhere)
    #   ※ 호출 형태 주의: `bash -c 'name'` 은 **전체 경로로 exec** 되어 argv0 에 슬래시가 생기므로
    #     결함을 재현하지 못한다(실측). `bash <name>` 이라야 PATH 해소 + argv0 이 이름 그대로다.
    check("CWD-1a $0 에 슬래시가 없어도 본체를 찾는다(PATH 경유·다른 CWD)",
          rd(mark).count("BODY-RAN") == 1,
          "mark=%r stderr=%r" % (rd(mark), rc.stderr[-220:]))
    check("CWD-1b 본체가 실재하는데 '부재' 고지를 내지 않는다",
          "부트 본체" not in (rc.stdout + rc.stderr),
          repr((rc.stdout + rc.stderr)[-220:]))
    check("CWD-1c 반드시 exit 0(사람 프롬프트 불가침)", rc.returncode == 0, str(rc.returncode))

    # ★③ Git Bash 경로형에서도 같은 단언(H-HOOK-WIN-PATH 와 같은 실험실 조건 = cygpath 보유)
    hooks, binp, state, env, mark = lab(root, "cwd-win")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(rc_input=6, supports_input=True), 0o755)
    w(os.path.join(binp, "cygpath"),
      '#!/bin/sh\ncase "$1" in\n  -u) printf "%s" "$2" ;;\n'
      '  -w) printf "WINPFX::%s" "$2" ;;\n  *) printf "%s" "$2" ;;\nesac\n', 0o755)
    os.chmod(os.path.join(hooks, "role-bootstrap.sh"), 0o755)
    env_pathw = dict(env, PATH=hooks + os.pathsep + env["PATH"])
    rcw = subprocess.run(["bash", "role-bootstrap.sh"], input='{"prompt":"너는 마스터다"}',
                         capture_output=True, text=True, timeout=90, env=env_pathw, cwd=elsewhere)
    #   rc6 이라 본체는 건너뛰는 것이 정상 — 여기서 재는 것은 **'부재' 오판이 없는가** 다.
    check("CWD-1d Git Bash 경로형에서도 '본체 부재' 오판 0",
          "부트 본체" not in (rcw.stdout + rcw.stderr),
          repr((rcw.stdout + rcw.stderr)[-220:]))
    check("CWD-1e Git Bash 경로형도 exit 0", rcw.returncode == 0, str(rcw.returncode))

    # ★계측 타당성(음성 대조): 본체를 **정말로** 치우면 그때는 '부재' 고지가 나야 한다 —
    #   위 CWD-1b/1d 가 "고지 없음"을 재므로, 고지가 영영 안 나오는 런처였다면 무의미하다.
    hooks, binp, state, env, mark = lab(root, "cwd-truly-absent")
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    os.chmod(os.path.join(hooks, "role-bootstrap.sh"), 0o755)
    env_pathn = dict(env, PATH=hooks + os.pathsep + env["PATH"])
    rcn = subprocess.run(["bash", "role-bootstrap.sh"], input='{"prompt":"너는 마스터다"}',
                         capture_output=True, text=True, timeout=90, env=env_pathn, cwd=elsewhere)
    check("CWD-1f 본체가 진짜 없으면 정직하게 '부재' 고지(무음 통과 금지)",
          "부트 본체" in (rcn.stdout + rcn.stderr) and rcn.returncode == 0,
          repr((rcn.stdout + rcn.stderr)[-220:]))

    # ───────── 프리루드: **있으면 소비 · 없으면 자기완결 강등**(master 판정 ②) ─────────
    hooks, binp, state, env, mark = lab(root, "prelude")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    shutil.copy(os.path.join(HOOKS, "_lib.sh"), os.path.join(hooks, "_lib.sh"))
    r = run(hooks, env)
    check("PRELUDE-1a 프리루드가 있으면 정상 위임(강등 아님)",
          r.returncode == 0 and rd(mark).count("BODY-RAN") == 1, repr((r.returncode, r.stderr[-200:])))
    check("PRELUDE-1b 프리루드의 바이트코드 봉인(SEAL-1)이 본체까지 상속된다",
          rd(mark + ".seal") == "1", repr(rd(mark + ".seal")))
    hooks2, binp2, state2, env2, mark2 = lab(root, "prelude-none")
    w(os.path.join(hooks2, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp2, "cys"), stub_cys(supports_input=False), 0o755)
    r = run(hooks2, env2)
    check("PRELUDE-2 프리루드가 없어도 죽지 않는다(조용한 강등 · loud-skip 아님)",
          r.returncode == 0 and rd(mark2).count("BODY-RAN") == 1 and "소실" not in r.stderr,
          repr((r.returncode, r.stderr[-200:])))
    src_l = rd(LAUNCHER)
    check("PRELUDE-3 2단 폴백 문자열 보유(G-PRELUDE `no2` 계약)",
          "CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh" in src_l)

    # ───────── 반드시 exit 0 · 고지 계약 ─────────
    hooks, binp, state, env, mark = lab(root, "nobody")     # 본체 없음
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    r = run(hooks, env)
    check("EXIT0-1 본체 부재에서도 exit 0(127 유출 없음)", r.returncode == 0, repr(r.returncode))
    check("EXIT0-2 본체 부재는 고지된다(무음 아님)",
          r.stdout.startswith('{"hookSpecificOutput"') and "본체" in r.stdout, repr(r.stdout[:200]))
    check("EXIT0-3 고지 JSON 이 파싱된다",
          bool(json.loads(r.stdout.strip().splitlines()[0])["hookSpecificOutput"]["additionalContext"]))

    hooks, binp, state, env, mark = lab(root, "nostate")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    env2 = dict(env); env2["CYS_STATE_DIR"] = "/dev/null/nope"
    r = run(hooks, env2)
    check("T1-1a 상태 디렉터리 쓰기 불가 → exit 0", r.returncode == 0, repr(r.returncode))
    check("T1-1b 무발화가 **무음이 아니다**(고지 1줄)",
          r.stdout.startswith('{"hookSpecificOutput"') and "쓰기 불가" in r.stdout, repr(r.stdout[:200]))
    check("T1-1c 그 경로에서 본체는 돌지 않는다", rd(mark).count("BODY-RAN") == 0, repr(rd(mark)))

    # ───────── 좌석 게이트 · 레인 가드(부작용 부활 봉인) ─────────
    hooks, binp, state, env, mark = lab(root, "noseat")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    env3 = dict(env); env3.pop("CYS_SURFACE_ID", None); env3.pop("AITERM_SURFACE_ID", None)
    r = run(hooks, env3)
    check("SEAT-1a 좌석 없으면 exit 0 · stdout 무오염", r.returncode == 0 and r.stdout == "",
          repr((r.returncode, r.stdout[:150])))
    check("SEAT-1b 좌석 없으면 상태 디렉터리 무생성(부작용 0)", not os.path.isdir(state), state)
    check("SEAT-1c 좌석 없으면 cys 왕복 0(데몬 autostart 표면 0)",
          not os.path.exists(os.path.join(os.path.dirname(hooks), "cys.log")))
    check("SEAT-1d 본체도 돌지 않는다", rd(mark).count("BODY-RAN") == 0)

    hooks, binp, state, env, mark = lab(root, "lane")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    otherlane = os.path.join(root, "otherlane")
    w(os.path.join(otherlane, "hooks", "role-bootstrap.sh"), "#!/bin/sh\nexit 0\n", 0o755)
    env4 = dict(env); env4["CYS_PACK_DIR"] = otherlane
    r = run(hooks, env4)
    check("LANE-1a 타 레인 팩 훅은 조기 종료(exit 0 · 본체 미실행)",
          r.returncode == 0 and rd(mark).count("BODY-RAN") == 0, repr((r.returncode, rd(mark))))
    check("LANE-1b 조기 종료는 stderr 로 고지", "타 레인" in r.stderr, repr(r.stderr[-200:]))
    env5 = dict(env); env5["CYS_PACK_DIR"] = otherlane; env5["CYS_HOOK_LANE_GUARD"] = "0"
    hooks2, binp2, state2, env6, mark2 = lab(root, "lane-off")
    w(os.path.join(hooks2, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp2, "cys"), stub_cys(supports_input=False), 0o755)
    env6["CYS_PACK_DIR"] = otherlane; env6["CYS_HOOK_LANE_GUARD"] = "0"
    r = run(hooks2, env6)
    check("LANE-1c 노브로 가드를 끌 수 있다(되돌림 경로 실재)",
          rd(mark2).count("BODY-RAN") == 1, repr(rd(mark2)))

    # ───────── 입력 파일 유계 GC ─────────
    hooks, binp, state, env, mark = lab(root, "gc")
    w(os.path.join(hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    w(os.path.join(binp, "cys"), stub_cys(supports_input=False), 0o755)
    os.makedirs(state, exist_ok=True)
    for i in range(40):
        w(os.path.join(state, "hook-input-9%02d.json" % i), "{}")
    run(hooks, env)
    left = [n for n in os.listdir(state) if n.startswith("hook-input-")]
    check("GC-1 잔재 입력 파일이 유계로 정리된다(40 → ≤20)", len(left) <= 20, str(len(left)))

    # ───────── ★음성 대조 1: 능력 프로브를 떼면 구 CLI 에서 부트가 무음 사망한다 ─────────
    mut_hooks = os.path.join(root, "mut1", "hooks")
    os.makedirs(mut_hooks)
    mut = src.replace("      6|3) rm -f \"$IN\" 2>/dev/null; exit 0 ;;",
                      "      0|3) rm -f \"$IN\" 2>/dev/null; exit 0 ;;")
    check("MUT-1a 변조 앵커 실재(계측 타당성)", mut != src)
    w(os.path.join(mut_hooks, "role-bootstrap.sh"), mut, 0o755)
    w(os.path.join(mut_hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    d = os.path.dirname(mut_hooks)
    binm = os.path.join(d, "bin"); os.makedirs(binm, exist_ok=True)
    w(os.path.join(binm, "cys"), stub_cys(rc_input=0, supports_input=True), 0o755)
    envm = dict(env)
    envm.update({"MARK": os.path.join(d, "mark"), "CYSLOG": os.path.join(d, "cys.log"),
                 "CYS_STATE_DIR": os.path.join(d, "state"),
                 "PATH": binm + os.pathsep + os.environ.get("PATH", "")})
    r = run(mut_hooks, envm)
    check("MUT-1b rc 0 을 처리완료로 읽는 변조본은 **본체가 안 돈다**(무음 사망 재현) — "
          "이 검체가 실제로 그 갈래를 재고 있다",
          rd(os.path.join(d, "mark")).count("BODY-RAN") == 0,
          repr(rd(os.path.join(d, "mark"))))

    # ───────── ★음성 대조 2: 좌석 게이트를 떼면 부작용이 되살아난다 ─────────
    mut2_hooks = os.path.join(root, "mut2", "hooks")
    os.makedirs(mut2_hooks)
    seat = '[ -n "${CYS_SURFACE_ID:-}" ] || [ -n "${AITERM_SURFACE_ID:-}" ] || exit 0'
    check("MUT-2a 좌석 게이트 앵커 실재", seat in src)
    w(os.path.join(mut2_hooks, "role-bootstrap.sh"), src.replace(seat, ":"), 0o755)
    w(os.path.join(mut2_hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
    d2 = os.path.dirname(mut2_hooks)
    binm2 = os.path.join(d2, "bin"); os.makedirs(binm2, exist_ok=True)
    w(os.path.join(binm2, "cys"), stub_cys(supports_input=False), 0o755)
    st2 = os.path.join(d2, "state")
    envm2 = dict(os.environ)
    for k in ("CYS_SURFACE_ID", "AITERM_SURFACE_ID", "CYS_PACK_DIR", "CYS_MISSION"):
        envm2.pop(k, None)
    envm2.update({"MARK": os.path.join(d2, "mark"), "CYSLOG": os.path.join(d2, "cys.log"),
                  "CYS_STATE_DIR": st2, "PATH": binm2 + os.pathsep + envm2.get("PATH", "")})
    run(mut2_hooks, envm2)
    check("MUT-2b 게이트를 떼면 좌석 없이도 상태 디렉터리가 생긴다(부작용 재현)",
          os.path.isdir(st2), st2)
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("HOOK-LAUNCHER-SPLIT-OK")
sys.exit(0)
