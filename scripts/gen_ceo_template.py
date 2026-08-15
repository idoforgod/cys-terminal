#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""gen_ceo_template.py — CEO_TEMPLATE.md 합성 생성기 + 드리프트 게이트 (스펙 v4 §D2+A7 · W6).

★MASTER 본문 편집 시 재합성 필수: `cysjavis-pack/directives/MASTER_DIRECTIVE.md` 가 1바이트라도
  바뀌면 이 스크립트를 재실행해 CEO_TEMPLATE.md 를 다시 합성해야 한다 — 안 하면 `--check`
  드리프트 게이트(재합성 결과 vs 커밋본 바이트 비교)가 적색(exit 1)이 된다.
★서문(fragment) 개정 시 H-DOC-3 주의: 차단 동사(launch|allocate|create|down|down-sock|rotate|
  reap|promote-ceo)를 `cys-dept <동사>` **호출형**(백틱 안이라도)으로 적으면 지시-집행 통일
  검체 H-DOC-3(run_bootstrap_health.py)와 본 --check 가 동시에 적색이 된다 — 산문으로 언급할
  때는 동사 단독 백틱(`launch`)까지만 쓰고 `cys-dept` 를 앞에 붙이지 마라.

합성(빌드타임 변환 금지 — repo 커밋 산출물):
    [scripts/ceo_template_header.md — CEO 머리글+합성 서문 fragment(비출하)]
  + [구분선 "\\n---\\n\\n# [본문 — 표준 MASTER 운영 계약 전문]\\n\\n"]
  + [cysjavis-pack/directives/MASTER_DIRECTIVE.md 바이트 무수정]
  → cysjavis-pack/directives/CEO_TEMPLATE.md  (utf-8 · 개행 무변형 — 바이너리 연접)

★fragment :22 좌표 계약: fragment 의 22행(라벨 규약 줄)은 합성본에서도 22행이다 —
  src/bin/cysd/delivery.rs:8 이 `CEO_TEMPLATE.md:22` 좌표를 참조하므로 fragment 선두부에
  줄을 끼워 넣지 마라(본 스크립트도 합성본 선두에 아무것도 덧붙이지 않는다).

사용:
    python3 scripts/gen_ceo_template.py            # 재합성 → CEO_TEMPLATE.md 기록
    python3 scripts/gen_ceo_template.py --check    # 드리프트+위생 게이트 (CI · exit 1=적색)

--check 단언(스펙 §D2 — 전부 통과해야 exit 0):
  ① 드리프트: 재합성 결과 == 커밋본 CEO_TEMPLATE.md 바이트 등가(다르면 diff 요지 출력).
  ② 표지 핀 불변식: MARKER_PINS 3핀({"master of master","단일소유 강제","exit 7"} —
     SOT=javis_preflight.MARKER_PINS)이 fragment(신판 머리글)에 전수 실존. 구판 축은
     git 가용 시 구판 템플릿(기본 ref 1a90128 · CYS_CEO_OLD_TEMPLATE_REF 로 재지정)에도
     전수 실존을 대조한다(배포 팩 등 비-repo 환경은 skip — 신판 축은 무조건 단언).
  ③ 위생(A3×A7 재발 차단): 합성본에 `sock -- ` 문자열 부재 — fan-out 루프는
     `s=$(cys-dept sock "$d")` 그대로여야 한다(sock 동사는 `--` 를 해석하지 못해
     `--` 를 넣으면 전부서 방송이 전 함대에서 항상 실패한다 · R3 critical).
  ④ H-DOC-3 동일 규칙: cys-dept 단일소유 가드 case 에서 차단 동사 집합을 코드로 뽑아,
     합성본이 호출형(`cys-dept <동사>`)으로 지시하는 동사 집합과 교집합 0 을 단언.
  ⑤ fan-out 스니펫 실행 스모크: fragment 의 방송 루프를 스텁 cys-dept/cys 로 격리 실행해
     전 부서(2개 스텁)에 send+send-key 가 실제 도달함을 단언(라이브 데몬 무접촉).
"""
import io
import os
import re
import stat
import subprocess
import sys
import tempfile

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_DIR = os.path.dirname(SCRIPTS_DIR)
PACK_DIR = os.path.join(REPO_DIR, "cysjavis-pack")
FRAGMENT = os.path.join(SCRIPTS_DIR, "ceo_template_header.md")
MASTER = os.path.join(PACK_DIR, "directives", "MASTER_DIRECTIVE.md")
TARGET = os.path.join(PACK_DIR, "directives", "CEO_TEMPLATE.md")
CYS_DEPT = os.path.join(PACK_DIR, "bin", "cys-dept")

# 구분선 — fragment 끝(개행 종단)과 사이에 빈 줄 1개를 만든다.
SEPARATOR = "\n---\n\n# [본문 — 표준 MASTER 운영 계약 전문]\n\n".encode("utf-8")

# 구판 템플릿 축의 기본 ref: Wave1 착지 커밋(구 6KB 머리글 템플릿의 마지막 트리).
OLD_TEMPLATE_REF = os.environ.get("CYS_CEO_OLD_TEMPLATE_REF", "1a90128")

# 표지 핀 기대 리터럴 — SOT(javis_preflight.MARKER_PINS)와 어긋나면 loud 적색.
EXPECTED_MARKER_PINS = {"master of master", "단일소유 강제", "exit 7"}


def _read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def compose():
    """fragment + 구분선 + MASTER 본문(바이트 무수정) — 개행 무변형 바이너리 연접."""
    frag = _read_bytes(FRAGMENT)
    if not frag.endswith(b"\n"):
        frag += b"\n"     # 종단 개행 보정(구분선 앞 빈 줄 성립) — fragment 본문은 무수정
    return frag + SEPARATOR + _read_bytes(MASTER)


def _marker_pins():
    """표지 핀 SOT 로드(javis_preflight.MARKER_PINS) + 기대 리터럴 등가 단언."""
    sys.path.insert(0, os.path.join(PACK_DIR, "bin"))
    import javis_preflight as pf   # noqa: E402 — 핀 SOT(사본 금지)
    pins = tuple(pf.MARKER_PINS)
    if set(pins) != EXPECTED_MARKER_PINS:
        raise AssertionError(
            "MARKER_PINS SOT 가 스펙 기대 3핀과 다르다(드리프트): %r != %r"
            % (sorted(pins), sorted(EXPECTED_MARKER_PINS)))
    return pins


def _git_show_old_template():
    """구판 CEO_TEMPLATE 바이트(계측 대조용) — 비-repo/실패면 None(skip)."""
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        return None
    r = subprocess.run(
        ["git", "-C", REPO_DIR, "show",
         "%s:cysjavis-pack/directives/CEO_TEMPLATE.md" % OLD_TEMPLATE_REF],
        capture_output=True, timeout=30)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def _blocked_verbs():
    """cys-dept 단일소유 가드가 막는 동사 집합 — 문서 목록이 아니라 **코드에서** 뽑는다
    (H-DOC-3 과 동일 추출식 — 가드 형태가 바뀌면 여기가 fail-closed 로 적색)."""
    src = _read_bytes(CYS_DEPT).decode("utf-8", "replace")
    m = re.search(r"^\s*(launch\|[a-z|\-]+)\)\s*$", src, re.M)
    if m is None:
        raise AssertionError("cys-dept 단일소유 가드의 동사 case 를 찾지 못했다(가드 형태 변경?)")
    blocked = set(m.group(1).split("|"))
    if not {"launch", "allocate", "create", "down", "down-sock",
            "rotate", "reap", "promote-ceo"} <= blocked:
        raise AssertionError("가드 차단 집합이 예상보다 좁다: %s" % sorted(blocked))
    return blocked


def _fanout_snippet(frag_text):
    """fragment 의 방송(fan-out) bash 펜스 — `cys-dept list` 를 포함하는 유일 블록."""
    blocks = re.findall(r"```bash\n(.*?)```", frag_text, re.S)
    hits = [b for b in blocks if "cys-dept list" in b]
    if len(hits) != 1:
        raise AssertionError("fan-out 스니펫 추출 실패(기대 1건, 실측 %d건)" % len(hits))
    return hits[0]


def _smoke_fanout(snippet):
    """스텁 cys-dept/cys 로 fan-out 루프 격리 실행 — 방송이 전 부서에 실제 도달하는지.
    스텁 sock 은 실물과 동형으로 '-' 선두 인자를 거부한다(`sock -- "$d"` 회귀가 여기서 죽는다)."""
    tmp = tempfile.mkdtemp(prefix="gen-ceo-smoke-")
    bindir = os.path.join(tmp, "bin")
    os.makedirs(bindir)
    log = os.path.join(tmp, "calls.log")

    def w(path, body):
        with open(path, "w", encoding="utf-8") as f:
            f.write(body)
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    w(os.path.join(bindir, "cys-dept"),
      '#!/bin/sh\n'
      'case "$1" in\n'
      '  list) printf "alpha\\nbeta\\n" ;;\n'
      '  sock) shift\n'
      '        case "$1" in -*) echo "invalid name: $1" >&2; exit 2 ;; esac\n'
      '        printf "%s/%s.sock\\n" "' + tmp + '" "$1" ;;\n'
      '  *) exit 2 ;;\n'
      'esac\n')
    w(os.path.join(bindir, "cys"),
      '#!/bin/sh\nprintf "cys %s\\n" "$*" >> "' + log + '"\n')

    env = dict(os.environ)
    env["PATH"] = bindir + os.pathsep + env.get("PATH", "")
    r = subprocess.run(["bash", "-c", snippet], capture_output=True, text=True,
                       encoding="utf-8", errors="replace", env=env, cwd=tmp, timeout=30)
    calls = ""
    if os.path.isfile(log):
        with open(log, encoding="utf-8") as f:
            calls = f.read()
    problems = []
    if r.returncode != 0:
        problems.append("스니펫 exit %d — stderr: %s" % (r.returncode, r.stderr.strip()[-300:]))
    for d in ("alpha", "beta"):
        sock = "%s/%s.sock" % (tmp, d)
        if ("--socket %s send --to master" % sock) not in calls:
            problems.append("부서 %s 에 send 미도달(방송 전멸 회귀?)" % d)
        if ("--socket %s send-key --to master Return" % sock) not in calls:
            problems.append("부서 %s 에 send-key 미도달" % d)
    if "--.sock" in calls or "invalid name" in (r.stderr or ""):
        problems.append("sock 인자에 '--' 혼입 흔적(R3 critical A3×A7 재발)")
    return problems


def check():
    problems = []
    composed = compose()
    frag_text = _read_bytes(FRAGMENT).decode("utf-8")
    composed_text = composed.decode("utf-8")

    # ① 드리프트: 재합성 == 커밋본
    current = _read_bytes(TARGET) if os.path.isfile(TARGET) else b""
    if composed != current:
        import difflib
        diff = list(difflib.unified_diff(
            current.decode("utf-8", "replace").splitlines(),
            composed_text.splitlines(),
            fromfile="CEO_TEMPLATE.md(커밋본)", tofile="CEO_TEMPLATE.md(재합성)",
            lineterm="", n=1))
        problems.append(
            "드리프트: 재합성 결과 != 커밋본(%d != %d bytes) — MASTER 본문/fragment 편집 후 "
            "재합성 누락. `python3 scripts/gen_ceo_template.py` 재실행하라. diff 요지:\n%s"
            % (len(composed), len(current), "\n".join(diff[:20])))

    # ② 표지 핀: 신판(fragment) 전수 실존 + 구판 축(git 가용 시)
    try:
        pins = _marker_pins()
        missing = [p for p in pins if p not in frag_text]
        if missing:
            problems.append("표지 핀이 fragment(신판 머리글)에 부재(불변식 붕괴): %r" % missing)
        old = _git_show_old_template()
        if old is None:
            print("· 표지 핀 구판 축: skip(no-git — 신판 축만 단언)")
        else:
            old_missing = [p for p in pins if p not in old]
            if old_missing:
                problems.append(
                    "표지 핀이 구판 템플릿(%s)에 부재 — 구·신 공통 실존 불변식 붕괴: %r"
                    % (OLD_TEMPLATE_REF, old_missing))
    except AssertionError as e:
        problems.append(str(e))

    # ③ 위생: 'sock -- ' 부재(A3×A7)
    if "sock -- " in composed_text:
        problems.append("합성본에 'sock -- ' 잔존 — sock 동사는 '--' 를 해석하지 못한다"
                        "(전부서 방송 전멸 · R3 critical). fan-out 은 `sock \"$d\"` 그대로.")

    # ④ H-DOC-3 동일 규칙: 호출형 지시 동사 ⊆ 가드 허용 집합
    try:
        blocked = _blocked_verbs()
        used = set(re.findall(r"cys-dept\s+([a-z][a-z\-]*)", composed_text))
        if not used:
            problems.append("합성본에 cys-dept 사용 예 0건 — 추출기 파손 의심(fail-closed)")
        illegal = sorted(used & blocked)
        if illegal:
            problems.append("합성본이 가드 차단 동사를 호출형으로 지시(H-DOC-3 위반): %s "
                            "— GUI 부서 버튼/CSO 위임 경유로 고쳐라" % illegal)
    except AssertionError as e:
        problems.append(str(e))

    # ⑤ fan-out 스니펫 실행 스모크
    try:
        problems += _smoke_fanout(_fanout_snippet(frag_text))
    except AssertionError as e:
        problems.append(str(e))

    if problems:
        for p in problems:
            print("✗ %s" % p, file=sys.stderr)
        return 1
    print("gen --check GREEN — 드리프트 0 · 표지 3핀(신판%s) · 'sock -- ' 부재 · "
          "H-DOC-3 교집합 0 · fan-out 스모크 PASS (%d bytes)"
          % ("" if _git_show_old_template() is None else "+구판 %s" % OLD_TEMPLATE_REF,
             len(composed)))
    return 0


def main(argv):
    if "--check" in argv:
        return check()
    composed = compose()
    with open(TARGET, "wb") as f:      # 바이너리 기록 — utf-8 원문·개행 무변형
        f.write(composed)
    print("CEO_TEMPLATE.md 재합성 완료: %d bytes" % len(composed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
