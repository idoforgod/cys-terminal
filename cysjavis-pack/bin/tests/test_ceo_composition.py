#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_ceo_composition.py — CEO 지침 조립 승격·영수증·강등 회귀 핀 (0.15.0 「스스로 해결」 T8 · M14 승격 부분 · M15-나).

설계 v0.5 §4-14(D:958–961) · §6-14 · §6-15 나 · CONTRACTS C11 11.8–11.11 · O-C11-1(영수증) · O-C11-2(구분선) ·
시험 계획 E-6-01…12 · H-14-N1…N6 · H-14-A2-12 · H-RW-10.

  CEO 지침 = 생성된 머리말(CEO_TEMPLATE 의 첫 구분선 앞 바이트) + 구분선(scripts/gen_ceo_template.py SEPARATOR 그대로)
            + 사용자 자신의 본부 지침(조립 — 병합 아님). 강등 = 구분선 뒤 바이트. .pre-ceo = 백업일 뿐.
  영수증 = directives/.ceo-receipt.json {schema, header_sha256, master_sha256_at_compose, composed_sha256,
            separator, composed_at}. promote-ceo 의 승격 확인 = 조립된 지침 + 유효한 영수증('백업 있음'은 성공 아님).

격리: 가짜 HOME(임시 폴더) + $HOME/.local/bin 의 스텁 cys/cysd(cys-dept 의 PATH 선두) — 실 데몬·실 팩 무접촉.
각 시험은 실패해야 할 때 실패하는지(결함 주입·대조)를 함께 잰다 — "통과한다"만으로는 아무것도 증명하지 않는다.

    python3 cysjavis-pack/bin/tests/test_ceo_composition.py
"""
import ast
import contextlib
import errno
import glob
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
DEPT = os.path.join(BIN, "cys-dept")
sys.path.insert(0, BIN)
import javis_preflight as pf  # noqa: E402 — C03 핀·표지 핀 정본

SEP = "\n---\n\n# [본문 — 표준 MASTER 운영 계약 전문]\n\n"
HEADER = "# CEO-HEADER v1 (master of master · 단일소유 강제 · exit 7)\n"
HEADER_V2 = "# CEO-HEADER v2 (master of master · 단일소유 강제 · exit 7)\n새 릴리스의 머리말 줄\n"
PINS = [p for p, _ in pf.CONTENT_PINS["MASTER_DIRECTIVE.md"]]


def master_text(tag):
    return tag + "\n" + "\n".join(PINS) + "\n"


VENDOR = master_text("STANDARD-MASTER")
USER = VENDOR + "USERFIX-E6-1 사용자가 고친 본부 지침 줄\n"
TEMPLATE = HEADER + SEP + VENDOR
TEMPLATE_V2 = HEADER_V2 + SEP + master_text("STANDARD-MASTER-V2")


# 배포 태그 v0.14.0 … v0.14.56 — 저장소에 없는 v0.14.34·35·39 는 발행되지 않았다(54개 · cys-dept V014_* 표의 출처).
V014_TAGS = tuple("v0.14.%d" % n for n in range(57) if n not in (34, 35, 39))
_RELEASED = None


def released_templates():
    """{태그: CEO_TEMPLATE.md 바이트} — 로컬 저장소의 배포 태그에서 `git show` 로 읽는다(네트워크 없음). 태그가 없으면 {}."""
    global _RELEASED
    if _RELEASED is None:
        _RELEASED = {}
        if shutil.which("git") and os.path.exists(os.path.join(REPO, ".git")):
            for tag in V014_TAGS:
                r = subprocess.run(["git", "-C", REPO, "show", "%s:cysjavis-pack/directives/CEO_TEMPLATE.md" % tag],
                                   capture_output=True, timeout=60)
                if r.returncode == 0:
                    _RELEASED[tag] = r.stdout
    return _RELEASED


def need_released(test):
    rel = released_templates()
    if not rel:
        print("\n[notice] 이 체크아웃에 배포 태그 v0.14.x 가 없다(얕은 클론·배포 팩) — 0.14.x 표 재유도·실제 0.14.x 틀 "
              "이전 시험을 건너뛴다", file=sys.stderr)
        test.skipTest("배포 태그 v0.14.x 없음")
    return rel


def sha(text_or_bytes):
    b = text_or_bytes.encode("utf-8") if isinstance(text_or_bytes, str) else text_or_bytes
    return hashlib.sha256(b).hexdigest()


def _slurp(*args, **kw):
    with open(*args, **kw) as f:
        return f.read()


def _write(path, data, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data.encode("utf-8") if isinstance(data, str) else data)
    if mode is not None:
        os.chmod(path, mode)


def _descendant_names(pid):
    """pid 아래 모든 자손 프로세스의 실행 파일 이름(ps 한 번 — POSIX)."""
    out = subprocess.run(["ps", "-eo", "pid=,ppid=,comm="], capture_output=True, text=True).stdout
    kids = {}
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0].isdigit() and parts[1].isdigit():
            kids.setdefault(int(parts[1]), []).append((int(parts[0]), os.path.basename(parts[2].strip())))
    names, stack = [], [pid]
    while stack:
        for child, comm in kids.get(stack.pop(), []):
            names.append(comm)
            stack.append(child)
    return names


def extract_shell_function(src, name):
    """cys-dept 원문에서 함수 정의 한 개를 그대로 뽑는다(사본이 아니라 실제 코드로 단위 시험 — 변이가 그대로 보인다)."""
    m = re.search(r"^%s\(\)\{.*?^\}\n" % re.escape(name), src, re.S | re.M)
    if m is None:
        raise AssertionError("cys-dept 에 %s() 가 없다" % name)
    return m.group(0)


def ceo_py_module():
    """cys-dept 의 ceo_py 파이썬 원문(heredoc)을 그대로 모듈로 싣는다 — 마지막 `sys.exit(main(sys.argv))` 한 줄만 뺀다.
    시험이 그 안의 함수 하나를 감싸 '읽은 뒤·쓰기 전'에 사용자 저장을 끼워 넣을 수 있게(결정론 끼어들기 · 제품 시험 이음매 없음)."""
    src = extract_shell_function(_slurp(DEPT, encoding="utf-8"), "ceo_py")
    body = src.split("<<'PY'\n", 1)[1].rsplit("\nPY\n", 1)[0]
    tail = "sys.exit(main(sys.argv))"
    assert body.rstrip().endswith(tail), "ceo_py 원문의 끝이 바뀌었다"
    ns = {"__name__": "ceo_py_under_test"}
    exec(compile(body.rstrip()[: -len(tail)], DEPT + ":ceo_py", "exec"), ns)
    return ns


class Rig(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ceo-comp-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.bindir = os.path.join(self.home, ".local", "bin")
        self.dirs = os.path.join(self.home, ".cys", "pack", "directives")
        self.log = os.path.join(self.tmp, "calls.log")
        self.feedlist = os.path.join(self.tmp, "feed-list.txt")
        os.makedirs(self.bindir)
        os.makedirs(self.dirs)
        # 스텁 cys: 전 호출 기록 · ping=소켓 파일 실존 · status 실패(재주입 skip 경로) · feed list = 파일 내용.
        _write(os.path.join(self.bindir, "cys"),
               '#!/bin/sh\n'
               'echo "cys $*" >> "%(log)s"\n'
               'case "$1" in\n'
               '  ping) [ -e "$CYS_SOCKET" ] && exit 0 || exit 1 ;;\n'
               '  status|identify) exit 1 ;;\n'
               '  feed) if [ "$2" = "list" ]; then [ -f "%(fl)s" ] && cat "%(fl)s"; exit 0; fi; exit 0 ;;\n'
               'esac\nexit 0\n' % {"log": self.log, "fl": self.feedlist}, 0o755)
        _write(os.path.join(self.bindir, "cysd"),
               '#!/bin/sh\nmkdir -p "$(dirname "$CYS_SOCKET")"\ntouch "$CYS_SOCKET"\nexit 0\n', 0o755)
        self.env = dict(os.environ)
        self.env.update({"HOME": self.home,
                         "CYS_DEPTS_JSON": os.path.join(self.home, ".cys", "depts.json"),
                         "PATH": self.bindir + os.pathsep + self.env.get("PATH", "")})
        for k in list(self.env):
            if k.startswith(("CYS_", "JAVIS_", "AITERM_")) and k != "CYS_DEPTS_JSON":
                self.env.pop(k)
        self.env["CYS_NO_AUTOSTART"] = "1"

    # ── 픽스처 ──
    def p(self, name):
        return os.path.join(self.dirs, name)

    def seed(self, md=USER, tmpl=TEMPLATE, marker=True, ndepts=1):
        _write(self.p("MASTER_DIRECTIVE.md"), md)
        _write(self.p("CEO_TEMPLATE.md"), tmpl)
        if marker:
            _write(os.path.join(self.home, ".cys", ".master-bootstrapped"), "{}")
        self.register(ndepts)

    def register(self, ndepts=1):
        """부서 레지스트리(소켓 파일 = 살아 있는 스텁 데몬) — `down` 이 지운 뒤 다시 등록할 때도 쓴다."""
        depts = {}
        for i in range(ndepts):
            d = os.path.join(self.home, ".local", "state", "cys-dept-d%d" % i)
            os.makedirs(d, exist_ok=True)
            sock = os.path.join(d, "cys.sock")
            open(sock, "w").close()
            depts["d%d" % i] = {"socket": sock, "pack_dir": ""}
        _write(self.env["CYS_DEPTS_JSON"], json.dumps({"depts": depts}))

    def pending_path(self):
        return os.path.join(self.home, ".cys", "state", "ceo-pending")

    def set_pending(self):
        _write(self.pending_path(), "pending\n")

    def run_dept(self, *args, timeout=120):
        r = subprocess.run(["bash", DEPT] + list(args), capture_output=True, text=True,
                           encoding="utf-8", env=self.env, timeout=timeout)
        return r.returncode, r.stdout, r.stderr

    def read(self, name):
        try:
            with open(self.p(name), encoding="utf-8", newline="") as f:
                return f.read()
        except FileNotFoundError:
            return None

    def receipt(self):
        try:
            with open(self.p(".ceo-receipt.json"), encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError):
            return None

    def calls(self):
        return _slurp(self.log, encoding="utf-8") if os.path.exists(self.log) else ""

    def pushes(self, title):
        """그 제목으로 올린 feed push 수(스텁 cys 호출 기록)."""
        return sum(1 for line in self.calls().splitlines()
                   if line.startswith("cys feed push --title %s --body " % title))

    def pushes_prefix(self, prefix):
        """그 접두어로 시작하는 제목의 feed push 줄들(보관 사본 알림 제목은 사본 이름을 담는다 — CG2)."""
        return [line for line in self.calls().splitlines() if line.startswith("cys feed push --title " + prefix)]

    def run_ceo_py(self, *args):
        """cys-dept 의 실제 ceo_py 코드를 그대로 돌린다(동작·인자는 cys-dept 와 같다)."""
        funcs = extract_shell_function(_slurp(DEPT, encoding="utf-8"), "ceo_py")
        r = subprocess.run(["bash", "-c", funcs + 'ceo_py "$@"\n', DEPT] + list(args), capture_output=True, text=True,
                           encoding="utf-8", env=self.env, timeout=60)
        return r.returncode, r.stdout, r.stderr

    def kept(self, reason="*"):
        """보관 폴더(directives/.ceo-kept/)의 보관 사본들(곁 파일 *.kept 제외) — ★RULINGS EY1."""
        return sorted(f for f in glob.glob(os.path.join(self.dirs, ".ceo-kept", "*.%s" % reason))
                      if not f.endswith(".kept"))

    def captures(self, kind):
        """시각 붙은 보관 사본 — EY1 부터 보관 폴더 안(<UTC>-<pid>-<n>.<kind>)."""
        return self.kept(kind)

    def leftovers(self):
        return [f for f in os.listdir(self.dirs) if f.endswith(".tmp")]

    def assert_receipt_for(self, header, body, md_text):
        r = self.receipt()
        self.assertIsNotNone(r, "영수증(.ceo-receipt.json) 없음")
        self.assertEqual(set(r), {"schema", "header_sha256", "master_sha256_at_compose", "composed_sha256",
                                  "separator", "composed_at"}, "영수증 필드가 O-C11-1 과 다르다: %r" % r)
        self.assertEqual(r["schema"], 1)
        self.assertEqual(r["header_sha256"], sha(header))
        self.assertEqual(r["master_sha256_at_compose"], sha(body))
        self.assertEqual(r["composed_sha256"], sha(md_text))
        self.assertEqual(r["separator"], SEP)
        self.assertRegex(r["composed_at"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")


class Composition(Rig):
    def test_promotion_composes_and_keeps_user_edits(self):
        """E-6-01: 승격 = 머리말 + 구분선 + 사용자 본부 지침(수정 줄 포함) 바이트 그대로 + 영수증 + 백업."""
        self.seed()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertIn("USERFIX-E6-1", self.read("MASTER_DIRECTIVE.md"))
        self.assert_receipt_for(HEADER, USER, HEADER + SEP + USER)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER, "백업 = 승격 전 사용자 본부 지침")
        self.assertIsNone(self.read(".ceo-template-applied"), "0.14.x 영수증을 계속 쓴다")
        self.assertEqual(self.captures("before-compose"), [], "새 승격에서 사본을 남길 이유가 없다")
        self.assertEqual(self.leftovers(), [])
        # 대조: 종전 통째 복사였다면 지침 == 틀(사용자 줄 소실) — 지금은 틀과 다르다.
        self.assertNotEqual(self.read("MASTER_DIRECTIVE.md"), TEMPLATE)
        # 두 번째 지명은 이미 현행 — 무변경·성공.
        before = _slurp(self.p("MASTER_DIRECTIVE.md"), "rb")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(_slurp(self.p("MASTER_DIRECTIVE.md"), "rb"), before)

    def test_edits_during_ceo_survive_launch_and_ticks(self):
        """E-6-03: CEO 동안 본문을 고친 뒤 부서 켜기(launch)·지명·대기 틱이 그 수정을 지우지 않는다(재주입도 없음)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        edited = HEADER + SEP + USER + "USERFIX-E6-2 CEO 동안 고친 줄\n"
        _write(self.p("MASTER_DIRECTIVE.md"), edited)
        calls_before = self.calls()
        rc, out, err = self.run_dept("launch", "d0")
        self.assertEqual(rc, 0, out + err)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertIn("사용자가 고친 본부 지침", out + err, "승격 확인이 '고친 본문'을 인정하지 않았다")
        self.set_pending()
        rc, out, err = self.run_dept("promote-if-pending")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), edited, "본문 수정이 사라졌다(M15-나 재발)")
        self.assertFalse(os.path.exists(self.pending_path()), "이미 승격 — PENDING 은 해소된다")
        new_calls = self.calls()[len(calls_before):]
        self.assertNotIn("reinject", new_calls, "지침이 안 바뀌었는데 재주입했다")

    def test_demotion_returns_bytes_after_separator(self):
        """E-6-05: 강등 = 구분선 뒤 바이트(두 표지 모두) — .pre-ceo 는 원본이 아니라 시각 붙은 백업으로 남는다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        body = USER + "USERFIX-E6-2 CEO 동안 고친 줄\n"
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP + body)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), body, "강등이 구분선 뒤 바이트가 아니다")
        self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"), "강등 뒤 .pre-ceo 가 남아 '승격 중'으로 보인다")
        kept = sorted(self.kept("backup"))
        self.assertEqual(len(kept), 1, "다른 내용의 백업은 시각을 붙여 보관해야 한다: %r" % os.listdir(self.dirs))
        self.assertEqual(_slurp(kept[0], encoding="utf-8"), USER, "백업이 원본으로 쓰이지 않았는데 내용이 바뀌었다")
        self.assertIsNone(self.read(".ceo-receipt.json"))
        self.assertIn("CEO 강등", out + err)

    def test_demotion_without_edits_keeps_the_identical_backup(self):
        """★RULINGS EY1 로 다시 씀(종전: '같은 내용의 백업은 지운다'): 비교로 지우지 않는다 — 강등 뒤 .pre-ceo 는 같은 내용이어도
        보관 폴더로 옮겨 남고, 디렉티브 폴더에는 .pre-ceo 가 없다(승격 중에만 있게)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertEqual(glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo*")), [], "강등 뒤 .pre-ceo 가 디렉티브 폴더에 남았다")
        self.assertEqual([_slurp(k, encoding="utf-8") for k in self.kept("backup")], [USER], "같은 내용의 백업을 지웠다(EY1)")

    def test_user_header_edit_is_captured_before_recomposition(self):
        """E-6-07: 사용자가 머리말 자체를 고쳤다 → 다시 조립하기 전에 시각을 붙여 보관(틀 전진·틀 동일 두 경우)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        hand = HEADER + "USER-HEADER-EDIT 사용자가 머리말에 넣은 줄\n"
        live = hand + SEP + USER + "USERFIX-E6-2\n"
        _write(self.p("MASTER_DIRECTIVE.md"), live)
        _write(self.p("CEO_TEMPLATE.md"), TEMPLATE_V2)        # 틀 전진(새 릴리스)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER_V2 + SEP + USER + "USERFIX-E6-2\n")
        caps = self.captures("before-compose")
        self.assertEqual(len(caps), 1, "고친 머리말을 보관하지 않았다: %r" % os.listdir(self.dirs))
        self.assertRegex(os.path.basename(caps[0]), r"^\d{8}T\d{6}Z-\d+-\d+\.before-compose$")   # EY1: 보관 폴더의 이름
        self.assertEqual(_slurp(caps[0], encoding="utf-8"), live, "보관 사본이 조립 전 바이트가 아니다")
        self.assert_receipt_for(HEADER_V2, USER + "USERFIX-E6-2\n", self.read("MASTER_DIRECTIVE.md"))
        lines = self.pushes_prefix("CEO 지침 사본 보관(머리말) — ")
        self.assertEqual(len(lines), 1, "보관 사본을 알리지 않았다")
        self.assertIn("— %s --body" % os.path.basename(caps[0]), lines[0], "알림 제목이 보관 사본을 이름으로 가리키지 않는다")
        self.assertNotIn("다른 MASTER", lines[0], ".pre-ceo 가 본문과 같은데 다르다고 알렸다")

    def test_template_advance_recomposes_header_only_without_capture(self):
        """생성된 머리말(영수증이 증명)은 사본 없이 새 머리말로 바뀌고, 본문은 그대로 · 재주입 1회."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "USERFIX-E6-2\n")
        _write(self.p("CEO_TEMPLATE.md"), TEMPLATE_V2)
        n_before = self.calls().count("cys status --json")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER_V2 + SEP + USER + "USERFIX-E6-2\n")
        self.assertEqual(self.captures("before-compose"), [])
        self.assertIn("CEO 지침 다시 조립", out + err)
        self.assertEqual(self.calls().count("cys status --json") - n_before, 1, "재조립 뒤 재주입 시도 1회")

    def test_corrupt_or_missing_receipt_is_rewritten_not_held(self):
        """E-6-09 ★RULINGS CG1 이후: 영수증이 깨졌거나 없으면 승격 확인이 지금 지침을 그대로 두고 영수증을 다시 쓴다(종전 rc 4
        보류 · exit 5 없음). post-verify 자체는 여전히 유효한 영수증을 요구한다(대조: ceo_py verify 단위)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        md_before = self.read("MASTER_DIRECTIVE.md")
        for bad in ("{not json", json.dumps({"schema": 1, "header_sha256": "x"}),
                    json.dumps(dict(self.receipt(), separator="\n---\n")), None):
            with self.subTest(bad=(bad or "absent")[:30]):
                if bad is None:
                    os.remove(self.p(".ceo-receipt.json"))
                else:
                    _write(self.p(".ceo-receipt.json"), bad)
                    # 대조: 승격 확인(verify)은 깨진 영수증을 성공으로 보지 않는다 — 그래서 조립이 다시 써야 한다.
                    rc, out, _ = self.run_ceo_py("verify", self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"))
                    self.assertEqual(rc, 1, "깨진 영수증을 승격 확인이 받아들였다: " + out)
                    self.assertIn("믿을 수 없다", out)
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 0, "빠졌거나 손상된 영수증에 승격을 보류했다(CG1)\n" + out + err)
                self.assertIn("영수증 기록", out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), md_before, "영수증만 다시 쓸 일에 지침을 바꿨다")
                self.assert_receipt_for(HEADER, USER, md_before)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER, ".pre-ceo 를 건드렸다")
                self.assertEqual(self.captures("before-compose"), [])
        self.assertEqual(self.pushes_prefix("CEO "), [], "영수증 다시 쓰기에 알림을 띄웠다")

    def test_backup_alone_is_not_success(self):
        """C11 11.10: 승격이 안 됐는데 .pre-ceo 만 있으면 — 종전 post-verify 는 exit 0(성공)이었다. 이제 exit 5."""
        self.seed(tmpl="CEO-STUB\n")
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), USER)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)

    def test_superset_half_template_is_refused(self):
        """H-14-N1-01: 머리말은 있는데 표준 MASTER 가 잘린 '반쪽' 틀 → 보류(지침·백업·영수증 무변경) · 대조: 온전한 틀은 통과."""
        half = HEADER + SEP + "STANDARD-MASTER\n" + "\n".join(PINS[: len(PINS) // 2]) + "\n"
        bad_templates = {
            "half": half,
            "no-separator": HEADER + VENDOR,
            "header-without-marker-pins": "# 그냥 머리말\n" + SEP + VENDOR,
            "empty-header": SEP + VENDOR,
        }
        for name, tmpl in bad_templates.items():
            with self.subTest(template=name):
                shutil.rmtree(self.dirs)
                self.seed(tmpl=tmpl)
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 5, "%s 틀이 승격으로 받아들여졌다\n%s" % (name, out + err))
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
                self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"))
                self.assertIsNone(self.read(".ceo-receipt.json"))
                self.assertIn("상위집합", err)
        # 자동(대기형) 경로도 보류 — exit 0 · PENDING 유지 · 비대기 알림 1건.
        shutil.rmtree(self.dirs)
        self.seed(tmpl=half)
        self.set_pending()
        n0 = self.calls().count("feed push --title CEO 승격 보류(템플릿 상위집합 검사 실패)")
        rc, out, err = self.run_dept("promote-if-pending")
        self.assertEqual(rc, 0, out + err)
        self.assertTrue(os.path.exists(self.pending_path()))
        self.assertIn("반쪽", err, "보류 이유(반쪽 틀)가 보이지 않는다")
        self.assertEqual(self.calls().count("feed push --title CEO 승격 보류(템플릿 상위집합 검사 실패)") - n0, 1)
        # 대조: 온전한 틀이면 같은 경로가 승격한다.
        _write(self.p("CEO_TEMPLATE.md"), TEMPLATE)
        rc, out, err = self.run_dept("promote-if-pending")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)

    def test_superset_function_unit_on_the_real_code(self):
        """H-14-N1-01 단위: cys-dept 의 실제 ceo_template_superset_ok·ceo_py 코드를 그대로 돌린다(변이가 그대로 보인다).
        (i) 틀 = 머리말(표지 3핀) + 구분선 + 표준 MASTER 전문 · (ii) 후보 == 머리말 + 구분선 + 사용자 지침 전문."""
        src = _slurp(DEPT, encoding="utf-8")
        funcs = extract_shell_function(src, "ceo_py") + extract_shell_function(src, "ceo_template_superset_ok")
        self.assertTrue(funcs.count("PY\n}") == 1, "ceo_py 추출 경계가 heredoc 끝이 아니다")
        f = {}
        for name, data in {"good": TEMPLATE, "half": HEADER + SEP + "\n".join(PINS[:-1]) + "\n",
                           "stub": "CEO-STUB\n", "user": USER, "cand": HEADER + SEP + USER,
                           "cand-half": HEADER + SEP + USER[: len(USER) // 2]}.items():
            f[name] = os.path.join(self.tmp, name)
            _write(f[name], data)

        def call(*args):
            script = funcs + 'PACK_DEFAULT="%s"\nceo_template_superset_ok "$@"\n' % PACK
            r = subprocess.run(["bash", "-c", script, DEPT] + list(args), capture_output=True, text=True,
                               encoding="utf-8", env=self.env, timeout=60)
            return r.returncode, r.stdout.strip(), r.stderr

        rc, out, _ = call(f["good"])
        self.assertEqual((rc, out), (0, sha(TEMPLATE)), "온전한 틀은 통과하고 틀 해시를 낸다")
        self.assertEqual(call(f["half"])[0], 1, "반쪽 틀(핀 1개 소실)을 통과시켰다")
        self.assertEqual(call(f["stub"])[0], 1, "스텁 틀을 통과시켰다")
        self.assertEqual(call(f["good"], f["cand"], f["user"])[0], 0, "온전한 조립 후보를 거부했다")
        rc, _, err = call(f["good"], f["cand-half"], f["user"])
        self.assertEqual(rc, 1, "반쪽 조립 후보(사용자 지침 절반)를 통과시켰다")
        self.assertIn("반쪽 조립", err)

    def test_failures_exit_zero_and_leave_no_partial_file(self):
        """H-14-N3-01: 조립이 실패해도 늘 0 으로 끝나고(부서 흐름 불파괴) 실패가 보이며 반쪽·임시 파일이 남지 않는다."""
        # (a) 읽을 수 없는 본부 지침 — 대기 틱·부서 켜기 모두 exit 0, PENDING 유지.
        self.seed()
        self.set_pending()
        os.chmod(self.p("MASTER_DIRECTIVE.md"), 0)
        unreadable = True
        try:
            open(self.p("MASTER_DIRECTIVE.md"), "rb").close()   # root 는 0 권한도 읽는다 — 그 환경에선 (a) 를 잴 수 없다
            unreadable = False
        except PermissionError:
            pass
        try:
            if not unreadable:
                raise unittest.SkipTest("권한 0 파일을 읽을 수 있는 실행 계정(root) — (a) 측정 불가")
            for args in (("promote-if-pending",), ("launch", "d0")):
                rc, out, err = self.run_dept(*args)
                self.assertEqual(rc, 0, "%r 가 생명주기를 깼다\n%s" % (args, out + err))
                self.assertTrue(err.strip(), "실패가 보이지 않는다(무음)")
            self.assertTrue(os.path.exists(self.pending_path()))
        finally:
            os.chmod(self.p("MASTER_DIRECTIVE.md"), 0o644)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertEqual(self.leftovers(), [])
        # (b) 강등할 본문이 비었다(머리말 + 구분선뿐) — down 은 exit 0, 지침 그대로, 경고.
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP)
        self.assertIn("강등 보류", err)
        self.assertEqual(self.pushes("CEO 강등 보류(본부 지침 확인 필요)"), 1, "강등 보류가 stderr 에만 남았다(CB5)")
        self.assertEqual(self.leftovers(), [])
        # (c) 같은 빈 본문의 승격 확인 — 영수증을 쓰지 않고 보류(rc 6 → promote-ceo exit 5) · 알림 1건(CG1: 영수증을 다시
        #     쓰는 갈래는 본문이 온전할 때만이다). `down` 이 부서를 지웠으니 다시 등록한다.
        self.register()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertIsNone(self.read(".ceo-receipt.json"), "빈 본문에 영수증을 썼다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP)
        self.assertEqual(self.pushes("CEO 승격 보류(본부 지침 확인 필요)"), 1)
        self.assertEqual(self.leftovers(), [])

    def test_concurrent_promotion_lock_is_taken(self):
        """H-14-N2-01 ★RULINGS DC1 로 다시 씀: 동시 승격 락(D-5)은 모든 플랫폼에서 진짜 잠금이다 — 잡힌 잠금 뒤에서 상한까지
        기다리고, 그 안에 풀리면 이어서 조립한다. (종전 시험의 mkdir 갈래 — '남이 쥔 락이면 경고 뒤 무락 진행'을 성공으로
        기대 — 은 DC1 이 금지한 동작이라 지웠다: 잡혀 있으면 이제 보류다 → LockIsReal.) 실제 ceo_py 코드를 싣고, 잠금 대기의
        첫 쉼(time.sleep)을 사건으로 받아 '기다리는 중'을 판정한다 — 시간이 아니라."""
        import threading
        try:
            import fcntl
        except ImportError:
            self.skipTest("fcntl 없는 플랫폼")
        self.seed()
        md = self.p("MASTER_DIRECTIVE.md")
        fd = os.open(md + ".promote.lock", os.O_RDWR | os.O_CREAT, 0o644)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        ns = ceo_py_module()
        waiting = threading.Event()
        real_time = ns["time"]

        class Clock:
            def __getattr__(self, name):
                return getattr(real_time, name)

            @staticmethod
            def sleep(s):
                waiting.set()
                real_time.sleep(s)
        ns["time"] = Clock()
        os.environ["CYS_CEO_LOCK_WAIT_S"] = "60"
        self.addCleanup(os.environ.pop, "CYS_CEO_LOCK_WAIT_S", None)
        out = {}
        t = threading.Thread(target=lambda: out.setdefault("rc", ns["do_compose"](
            [self.p("CEO_TEMPLATE.md"), md, sha(TEMPLATE), ""])))
        t.start()
        try:
            self.assertTrue(waiting.wait(60), "조립이 잠금에 닿지 않았다")  # wallclock: guard 60 s only ends a hang; the verdict is the event
            self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "잠금이 잡혀 있는데 지침을 바꿨다(잠금 없음)")
            self.assertTrue(t.is_alive(), "잠금이 잡혀 있는데 조립이 끝났다")
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            t.join(120)
        self.assertFalse(t.is_alive())
        self.assertEqual(out.get("rc"), 0, "잠금이 풀린 뒤 조립하지 않았다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)

    def test_concurrent_promotions_give_one_valid_composition(self):
        """E-6-11: 동시에 여러 번 승격해도 결과는 한 가지 조립이고 백업은 사용자 지침(CEO 사본 아님 — D-5) · 전부 exit 0."""
        self.seed()
        procs = [subprocess.Popen(["bash", DEPT, "promote-ceo"], env=self.env,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(8)]
        rcs = [p.wait(timeout=180) for p in procs]
        for p in procs:
            p.stdout.close()
            p.stderr.close()
        self.assertEqual(rcs, [0] * 8)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER, "백업에 조립된 지침이 들어갔다(D-5)")
        self.assert_receipt_for(HEADER, USER, HEADER + SEP + USER)
        self.assertEqual(self.leftovers(), [])
        self.assertEqual(self.captures("before-compose"), [])
        self.assertFalse(os.path.exists(self.p("MASTER_DIRECTIVE.md.promote.lock.d")))

    def test_boot_marker_gate_still_holds(self):
        """H-14-N4-01: 부트 표지 전 승격 = PENDING · 지침 무변경 · 백업 없음 → 표지 뒤 대기형이 조립으로 승격·PENDING 해소."""
        self.seed(marker=False)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertTrue(os.path.exists(self.pending_path()))
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"))
        self.assertIsNone(self.read(".ceo-receipt.json"))
        _write(os.path.join(self.home, ".cys", ".master-bootstrapped"), "{}")
        rc, out, err = self.run_dept("promote-if-pending")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertFalse(os.path.exists(self.pending_path()))

    def test_old_receipt_hold_notice_is_closed_and_never_pushed(self):
        """H-14-N6-01 ★CG1 이후: 손상된 영수증은 보류 사유가 아니다 — '영수증 손상' 보류 알림은 다시 올리지 않고, 개발판이
        남긴 것이 데몬에 있으면 승격 확인이 비허가 결정(ceo-promoted)으로 닫는다. 대기 틱을 거듭해도 알림 0."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        _write(self.p(".ceo-receipt.json"), "{broken")
        title = "CEO 승격 보류(영수증 손상)"
        _write(self.feedlist, "rid-r\t[pending]\tceo-notice\t%s\tdecision=-\n" % title)   # 데몬이 보관 중(개발판)
        self.set_pending()
        for _ in range(3):
            self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertEqual(self.pushes(title), 0, "손상된 영수증으로 보류 알림을 올렸다(CG1)")
        self.assertIn("cys feed reply rid-r ceo-promoted", self.calls(), "남은 보류 알림을 닫지 않았다")
        self.assert_receipt_for(HEADER, USER, self.read("MASTER_DIRECTIVE.md"))
        self.assertFalse(os.path.exists(self.pending_path()), "승격된 기계의 PENDING 이 남았다")

    def test_title_dedupe_is_bytewise(self):
        """★T8b: 알림 제목 비교는 바이트 그대로다 — 맥의 /usr/bin/awk 는 en_US.UTF-8 에서 문자열 == 을 strcoll 로 비교해 한글만
        다른 두 제목을 같다고 봤다(보관 중인 'CEO 승격 보류(…)' 때문에 'CEO 강등 보류(…)' 를 올리지 않음). 그 로캘로 잰다."""
        self.seed()
        self.env["LC_ALL"] = "en_US.UTF-8"
        _write(self.feedlist, "rid-p\t[pending]\tceo-notice\tCEO 승격 보류(다른 승격·강등이 진행 중)\tdecision=-\n")
        funcs = extract_shell_function(_slurp(DEPT, encoding="utf-8"), "ceo_notice_once")
        r = subprocess.run(["bash", "-c", "CYS=cys\n" + funcs + 'ceo_notice_once "$1" "본문"\n', DEPT,
                            "CEO 강등 보류(다른 승격·강등이 진행 중)"], capture_output=True, text=True, env=self.env, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.pushes("CEO 강등 보류(다른 승격·강등이 진행 중)"), 1, "다른 제목을 같은 제목으로 걸렀다(strcoll)")
        r = subprocess.run(["bash", "-c", "CYS=cys\n" + funcs + 'ceo_notice_once "$1" "본문"\n', DEPT,
                            "CEO 승격 보류(다른 승격·강등이 진행 중)"], capture_output=True, text=True, env=self.env, timeout=60)
        self.assertEqual(self.pushes("CEO 승격 보류(다른 승격·강등이 진행 중)"), 0, "보관 중인 같은 제목을 또 올렸다")

    def test_successful_demotion_closes_demote_hold_notices(self):
        """★CG2: 강등이 끝나면 '강등 보류'·'강등 불능' 알림의 사유가 사라진다 — 비허가 결정(ceo-demoted)으로 닫는다.
        대조: 보류(rc 6)로 끝난 강등은 닫지 않는다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        _write(self.feedlist, "rid-d1\t[pending]\tceo-notice\tCEO 강등 보류(본부 지침 확인 필요)\tdecision=-\n"
                              "rid-d2\t[pending]\tceo-notice\tCEO 강등 불능(구분선·백업 없음)\tdecision=-\n"
                              "rid-x\t[pending]\tceo-notice\tCEO 승격 대기\tdecision=-\n")
        good = self.read("MASTER_DIRECTIVE.md")
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP)          # 되돌릴 본문이 비었다 → 보류
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertNotIn("ceo-demoted", self.calls(), "보류된 강등이 강등 알림을 닫았다")
        _write(self.p("MASTER_DIRECTIVE.md"), good)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        calls = self.calls()
        self.assertIn("cys feed reply rid-d1 ceo-demoted", calls)
        self.assertIn("cys feed reply rid-d2 ceo-demoted", calls)
        self.assertNotIn("rid-x ceo-demoted", calls, "무관한 알림을 닫았다")

    def test_each_kept_copy_is_announced_by_name(self):
        """★CG2: 보관 사본 알림의 제목은 사본 이름을 담는다 — 같은 종류의 앞 알림이 데몬에 남아 있어도 새 사본의 경로가
        제목 dedupe 에 가려지지 않는다(같은 사본을 두 번 알리지는 않는다)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        titles = []
        for n in (1, 2):
            _write(self.p("MASTER_DIRECTIVE.md"), HEADER + "USER-HEADER-EDIT %d\n" % n + SEP + USER)
            self.assertEqual(self.run_dept("promote-ceo")[0], 0)
            lines = self.pushes_prefix("CEO 지침 사본 보관(머리말) — ")
            self.assertEqual(len(lines), n, "보관 사본 %d 의 알림이 앞 알림에 가려졌다" % n)
            titles.append(lines[-1].split(" --body ", 1)[0][len("cys feed push --title "):])
            _write(self.feedlist, "".join("rid-%d\t[pending]\tceo-notice\t%s\tdecision=-\n" % (i, t)
                                          for i, t in enumerate(titles)))
        caps = self.captures("before-compose")
        self.assertEqual(len(caps), 2)
        self.assertEqual(sorted(t.rsplit(" — ", 1)[1] for t in titles), sorted(os.path.basename(c) for c in caps))
        # 같은 사본의 알림을 다시 올리지는 않는다(제목 완전일치 dedupe 는 그대로).
        nf = os.path.join(self.tmp, "nf.txt")
        _write(nf, "NOTICE\t%s\t본문\n" % titles[0])
        funcs = "".join(extract_shell_function(_slurp(DEPT, encoding="utf-8"), f)
                        for f in ("ceo_notice_once", "ceo_notices_from"))
        r = subprocess.run(["bash", "-c", 'CYS=cys\n' + funcs + 'ceo_notices_from "$1"\n', DEPT, nf],
                           capture_output=True, text=True, encoding="utf-8", env=self.env, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(self.pushes_prefix("CEO 지침 사본 보관(머리말) — ")), 2, "같은 사본 알림을 또 올렸다")

    def test_failed_notice_write_keeps_the_replacement_result(self):
        """★CG2: 교체가 끝난 뒤 알림 기록이 실패해도(알림 파일 = 폴더) 결과(rc)는 교체의 결과다 — 다시 조립 12 · 강등 0."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        live = HEADER + "USER-HEADER-EDIT\n" + SEP + USER          # 영수증이 증명 못 하는 머리말 → 사본 + 알림
        _write(self.p("MASTER_DIRECTIVE.md"), live)
        bad_nf = os.path.join(self.tmp, "notice-dir")
        os.mkdir(bad_nf)
        rc, out, err = self.run_ceo_py("compose", self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"),
                                       sha(TEMPLATE), bad_nf)
        self.assertEqual(rc, 12, "알림 실패가 다시 조립의 결과를 바꿨다\n" + err)
        self.assertIn("알림 기록 실패", err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assert_receipt_for(HEADER, USER, HEADER + SEP + USER)
        _write(self.p("MASTER_DIRECTIVE.md"), live)
        rc, out, err = self.run_ceo_py("demote", self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), bad_nf)
        self.assertEqual(rc, 0, "알림 실패가 강등의 결과를 바꿨다\n" + err)
        self.assertIn("알림 기록 실패", err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertEqual(len(self.captures("before-demote")), 1)

    def test_backup_invariant_for_the_overwrite_guard(self):
        """조립 승격은 늘 백업(.pre-ceo)을 남기고, 지워지면 다음 승격 확인이 본문으로 다시 만든다. ★RULINGS CB1 이후 A12·
        가드 4 는 백업에 기대지 않는다(구분선 ∨ .pre-ceo — `ceo_vendor_overwrite_rejection_truth_table` ⑥′ 이 고정) —
        이 시험은 백업 자체(강등·이전 때의 사본)를 지킨다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertTrue(os.path.exists(self.p("MASTER_DIRECTIVE.md.pre-ceo")))
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "USERFIX-E6-2\n")
        os.remove(self.p("MASTER_DIRECTIVE.md.pre-ceo"))
        rc, out, err = self.run_dept("launch", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER + "USERFIX-E6-2\n", "백업이 다시 생기지 않았다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "USERFIX-E6-2\n")


class ReceiptlessComposition(Rig):
    """★RULINGS CG1 — 영수증을 잃은 0.15.0 조립은 조립이다: 본문 = 구분선 뒤(CEO 동안 고친 것 포함) · 영수증은 다시 쓴다 ·
    .pre-ceo 는 그대로(낡았어도 아무것도 바꾸지 않는다) · 강등은 구분선 뒤 바이트를 돌려준다."""
    EDIT = "USERFIX-CG1 CEO 동안 고친 본문 줄\n"

    def test_body_edit_survives_a_lost_receipt_through_promote_and_demote(self):
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        edited = HEADER + SEP + USER + self.EDIT
        _write(self.p("MASTER_DIRECTIVE.md"), edited)
        os.remove(self.p(".ceo-receipt.json"))
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), edited, "CEO 동안 고친 본문이 사라졌다")
        self.assert_receipt_for(HEADER, USER + self.EDIT, edited)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER, ".pre-ceo 를 바꿨다")
        self.assertEqual(self.captures("before-compose"), [])
        self.assertEqual(self.pushes_prefix("CEO "), [], "영수증을 다시 쓰는 데 알림을 띄웠다")
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER + self.EDIT, "강등이 고친 본문을 돌려주지 않았다")
        kept = self.kept("backup")
        self.assertEqual([_slurp(k, encoding="utf-8") for k in kept], [USER], "다른 내용의 백업은 시각을 붙여 보관한다")
        self.assertIsNone(self.read(".ceo-receipt.json"))

    def test_stale_backup_beside_it_changes_nothing(self):
        stale = master_text("STALE-PRE-CEO 다른 MASTER")
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        edited = HEADER + SEP + USER + self.EDIT
        _write(self.p("MASTER_DIRECTIVE.md"), edited)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), stale)
        r0 = self.receipt()
        for receipt in ("valid", "absent"):
            with self.subTest(receipt=receipt):
                if receipt == "absent":
                    os.remove(self.p(".ceo-receipt.json"))
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), edited)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), stale)
                if receipt == "absent":
                    self.assert_receipt_for(HEADER, USER + self.EDIT, edited)     # 지금 지침 그대로 다시 기록
                else:
                    self.assertEqual(self.receipt(), r0, "유효한 영수증을 바꿨다")
                self.assertEqual(self.captures("before-compose"), [])
                self.assertEqual(self.pushes_prefix("CEO "), [])
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER + self.EDIT, "낡은 .pre-ceo 로 강등했다")
        self.assertEqual([_slurp(k, encoding="utf-8") for k in self.kept("backup")],
                         [stale])


class KeptFolder(Rig):
    """★RULINGS EY1·FA1 — 살아 있던 지침 파일은 지우지도 덮지도 않는다: 교체로 밀려난 파일·강등 뒤의 .pre-ceo 는 보관 폴더
    (directives/.ceo-kept/)로 옮겨져 남고, 각자 보관 시각의 해시를 담은 곁 파일(.kept, 단독 생성)을 가진다. 교체 전에 열린
    편집기 핸들이 교체 뒤에 저장하면 그 바이트는 보관 사본에 남고 preflight C03 가 파일마다 한 번 알린다(codex 순서 ①)."""
    EDIT = "USERSAVE-EY1 교체 뒤 옛 핸들로 저장한 줄\n"

    def c03(self):
        env0 = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            for k in pf.PACK_DIR_ENV_KEYS:
                os.environ.pop(k, None)
            p = pf.Preflight(fix=False, skips=[])
            p.c03_content_pins()
            return {r["id"]: r for r in p.results}
        finally:
            os.environ.clear()
            os.environ.update(env0)

    def late_save_through_an_old_handle(self, op):
        md = self.p("MASTER_DIRECTIVE.md")
        live = self.read("MASTER_DIRECTIVE.md")
        fh = open(md, "r+b")                      # 편집기가 교체 전에 연 핸들
        try:
            ns = ceo_py_module()
            if op == "demote":
                rc = ns["do_demote"]([self.p("CEO_TEMPLATE.md"), md, ""])
            else:
                rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), md, sha(_slurp(self.p("CEO_TEMPLATE.md"), "rb")), ""])
            self.assertIn(rc, (0, 12), "교체가 일어나지 않았다(시험 무효)")
            fh.seek(0)
            fh.write((live + self.EDIT).encode("utf-8"))
            fh.truncate()
        finally:
            fh.close()
        holders = [k for k in self.kept() if self.EDIT in _slurp(k, encoding="utf-8")]
        self.assertEqual(len(holders), 1, "교체 뒤 옛 핸들의 저장이 어디에도 없다(지워진 파일에 들어갔다) — %r" % self.kept())
        return holders[0]

    def test_late_save_into_the_displaced_file_survives_demotion_and_is_reported_until_handled(self):
        """★FY2a(FL6 의 '파일마다 한 번' 을 대신한다) — 경고는 조건이 서 있는 동안 매번 나오고, 사람이 다 본 사본을 handled/ 로
        옮기면 끝난다(이 행은 그 폴더를 보지 않고, 행의 문장이 그 폴더를 말한다)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        holder = self.late_save_through_an_old_handle("demote")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        rows = self.c03()
        self.assertEqual(rows["C03.ceo-kept"]["status"], pf.WARN, rows.get("C03.ceo-kept"))
        self.assertIn(holder, rows["C03.ceo-kept"]["detail"])
        self.assertIn("살아 있는 지침에는 없다", rows["C03.ceo-kept"]["detail"])
        handled = os.path.join(self.dirs, ".ceo-kept", "handled")
        self.assertIn(handled, rows["C03.ceo-kept"]["detail"], "행이 경고를 끝내는 폴더를 말하지 않는다")
        again = self.c03()["C03.ceo-kept"]
        self.assertEqual(again["status"], pf.WARN, "조건이 그대로인데 두 번째 확인이 경고하지 않았다(기억으로 끝냈다)")
        os.makedirs(handled)
        for f in (holder, holder + ".kept"):
            os.rename(f, os.path.join(handled, os.path.basename(f)))
        done = self.c03()["C03.ceo-kept"]
        self.assertEqual(done["status"], pf.PASS, "handled/ 로 옮긴 뒤에도 경고했다 — %r" % done)

    def test_late_save_into_the_displaced_file_survives_recomposition(self):
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        _write(self.p("CEO_TEMPLATE.md"), TEMPLATE_V2)             # 틀 전진 → 다시 조립(머리말이 영수증으로 증명됨)
        self.late_save_through_an_old_handle("compose")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER_V2 + SEP + USER)

    def test_nothing_that_was_live_is_deleted(self):
        """승격 → 강등 → 승격: 밀려난 지침 파일과 강등 뒤의 .pre-ceo 가 모두 보관 폴더에 남는다(지운 것 0) · 곁 파일마다 보관
        시각의 해시 · 승격·강등은 보관 사본을 지우지 않는다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertEqual([_slurp(k, encoding="utf-8") for k in self.kept()], [USER], "밀려난 평문 MASTER 를 보관하지 않았다")
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertIn(os.path.join(".ceo-kept", ""), out + err, "강등 출력이 보관 사본을 말하지 않는다")
        self.register()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertIn(os.path.join(".ceo-kept", ""), out + err, "승격 출력이 보관 사본을 말하지 않는다")
        got = sorted(_slurp(k, encoding="utf-8") for k in self.kept())
        self.assertEqual(got, sorted([USER, HEADER + SEP + USER, USER, USER]),
                         "보관 사본이 빠졌다(밀려난 평문·조립본·강등 뒤 .pre-ceo·두 번째 평문): %r" % self.kept())
        for k in self.kept():
            side = json.loads(_slurp(k + ".kept", encoding="utf-8"))
            self.assertEqual(side["sha256"], sha(_slurp(k, "rb")), "곁 파일의 해시가 보관 시각의 바이트가 아니다")

    def test_sidecar_is_created_exclusively(self):
        """곁 파일은 단독 생성이다 — 같은 이름이 이미 있으면 다른 보관 이름을 고른다(공유 원장을 다시 쓰지 않는다)."""
        self.seed()
        ns = ceo_py_module()
        ns["stamp"] = lambda: "20260101T000000Z"
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.makedirs(kd)
        taken = os.path.join(kd, "20260101T000000Z-%d-1.test" % os.getpid())
        _write(taken + ".kept", "{}")
        src = self.p("MASTER_DIRECTIVE.md.scratch")
        _write(src, "보관할 바이트\n")
        kept, digest = ns["keep_file"](src, self.p("MASTER_DIRECTIVE.md"), "test")
        self.assertNotEqual(kept, taken)
        self.assertEqual(_slurp(taken + ".kept", encoding="utf-8"), "{}", "남의 곁 파일을 덮었다")
        self.assertEqual(json.loads(_slurp(kept + ".kept", encoding="utf-8"))["sha256"], digest)
        self.assertFalse(os.path.exists(src))

    # ── ★RULINGS FL6 ──
    def test_a_crash_while_a_sidecar_is_published_leaves_no_empty_sidecar(self):
        """곁 파일은 통째로 게시된다(codex B6: 단독 생성 뒤 쓰기 전에 멈추면 빈 곁 파일이 남았다). 곁 파일 이름을 만드는 바로 그
        호출 뒤에서 멈춘다 — 남은 곁 파일은 온전하다(점으로 시작하는 임시 파일은 사본도 곁 파일도 아니다)."""
        self.seed()
        ns = ceo_py_module()
        stop = lambda target: str(target).endswith(".kept") and not os.path.basename(str(target)).startswith(".")

        class _Os:
            def __getattr__(self, name):
                return getattr(os, name)

            def open(self, path, flags, *a, **kw):
                fd = os.open(path, flags, *a, **kw)
                if stop(path) and flags & os.O_CREAT:
                    os.close(fd)
                    raise SystemExit("crash right after the sidecar name was created")
                return fd

            def link(self, a, b):
                os.link(a, b)
                if stop(b):
                    raise SystemExit("crash right after the sidecar was placed")
        ns["os"] = _Os()
        nr0 = ns["noreplace_fn"]()
        if nr0 is not None:
            def nr(a, b):
                nr0(a, b)
                if stop(b):
                    raise SystemExit("crash right after the sidecar was placed")
            ns["noreplace_fn"] = lambda: nr
        src = self.p("MASTER_DIRECTIVE.md.scratch")
        _write(src, "보관할 바이트\n")
        with self.assertRaises(SystemExit):
            ns["keep_file"](src, self.p("MASTER_DIRECTIVE.md"), "test")
        kd = os.path.join(self.dirs, ".ceo-kept")
        sides = [f for f in os.listdir(kd) if stop(f)]
        self.assertTrue(sides, "곁 파일 이름을 만드는 호출에 이르지 않았다(시험 무효)")
        for f in sides:
            raw = _slurp(os.path.join(kd, f), encoding="utf-8")
            self.assertTrue(raw.strip(), "멈춘 뒤 빈 곁 파일이 남았다(codex B6)")
            self.assertEqual(json.loads(raw)["sha256"], sha("보관할 바이트\n"))

    def test_a_kept_file_without_a_readable_sidecar_is_unresolved_not_skipped(self):
        """곁 파일이 없거나 형식이 틀린 사본은 C03 이 건너뛰지 않는다 — '미확인'으로 보이고, 그대로인 동안 매번 WARN(FY2)."""
        self.seed()
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.makedirs(kd)
        bare = os.path.join(kd, "20260101T000000Z-1-1.compose")
        bad = os.path.join(kd, "20260101T000000Z-1-2.compose")
        _write(bare, USER)
        _write(bad, USER)
        _write(bad + ".kept", "")
        _write(os.path.join(kd, ".20260101T000000Z-1-3.compose.kept.x.tmp"), "")    # 멈춘 게시의 임시 파일 — 사본이 아니다
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.WARN, row)
        for k in (bare, bad):
            self.assertIn(k, row["detail"], "곁 파일이 없거나 틀린 사본을 건너뛰었다")
        self.assertIn("미확인", row["detail"])
        self.assertIn("보관 사본 2개", row["detail"], "임시 파일을 사본으로 셌다")
        again = self.c03()["C03.ceo-kept"]
        self.assertEqual(again["status"], pf.WARN, "미확인 사본이 그대로인데 경고가 멈췄다(★FY2a — 기억 없음)")
        for k in (bare, bad):
            self.assertIn(k, again["detail"], "미확인 사본이 다음 확인에서 사라졌다")

    def test_nothing_is_acknowledged_so_a_lost_output_loses_no_warning(self):
        """codex R2 — 종전: WARN 행을 메모리에 남긴 뒤 '알렸다' 표지를 만들어, 출력 전에 멈추면 그 경고는 영영 나오지 않았다. 이제
        이 행은 아무것도 기억하지 않는다(FY2a): 출력되지 못한 실행 뒤의 다음 실행도 WARN 이고, 표지 폴더는 생기지 않는다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        holder = self.late_save_through_an_old_handle("demote")
        orig = pf.Preflight.add

        def failing_add(p, cid, status, *a, **kw):
            if cid == "C03.ceo-kept" and status == pf.WARN:
                raise RuntimeError("the WARN row was never printed (crash)")
            return orig(p, cid, status, *a, **kw)
        pf.Preflight.add = failing_add
        try:
            with self.assertRaises(RuntimeError):
                self.c03()
        finally:
            pf.Preflight.add = orig
        self.c03()                                  # 결과를 내지 못하고 끝난 또 한 번의 실행(출력 전에 멈춤)
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.WARN, "출력되지 못한 경고를 '이미 알림'으로 넘겼다(codex R2)")
        self.assertIn(holder, row["detail"])
        self.assertFalse(os.path.exists(os.path.join(self.home, ".cys", "state", "preflight-c03-kept-warned")),
                         "'알렸다' 표지 폴더가 생겼다")

    def test_a_header_only_change_is_not_reported_as_in_the_live_directive(self):
        """★FY5(codex R5) — 머리말만 바뀐 조립 사본: 본문은 살아 있는 지침에 있지만 머리말은 이 사본에만 있다. 종전에는 본문만 비교해
        '살아 있는 지침에도 있다'고 말했다. 이제 머리말과 본문을 따로 재고, 머리말이 다르면 그대로 말하며 WARN 이다. 머리말까지
        같으면(내용 전체가 살아 있다) PASS."""
        self.seed(md=HEADER + SEP + USER)
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.makedirs(kd)
        k = os.path.join(kd, "20260101T000000Z-1-1.before-compose")
        _write(k, "# 사용자가 고친 머리말 줄\n" + HEADER + SEP + USER)          # 보관 뒤 머리말만 바뀌었다
        _write(k + ".kept", json.dumps({"schema": 2, "kind": "regular", "sha256": sha(HEADER + SEP + USER)}))
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.WARN, "머리말만 바뀐 사본을 '살아 있는 지침에 있다'로 넘겼다(codex R5): %r" % row)
        self.assertIn("머리말이 살아 있는 지침의 머리말과 다르다", row["detail"])
        self.assertNotIn("그 내용은 살아 있는 지침에도 있다", row["detail"])
        _write(k, HEADER + SEP + USER + "옮겨 적은 줄\n")                      # 머리말 같음 · 본문의 새 줄도 살아 있게 옮긴다
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "옮겨 적은 줄\n")
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.PASS, row)
        self.assertIn("머리말과 본문", row["detail"])

    def test_every_unusable_sidecar_is_unresolved(self):
        """★FY2b·c — 기준을 세울 수 없는 곁 파일은 모두 '미확인'(WARN): {} · {"sha256": null} · 종류 없음 · 일반 파일인데 해시 없음 ·
        읽지 못했다(unread) · 객체가 아님 · 해시 형식이 틀림 · 일반 파일이 아니라는데 지금은 일반 파일. 폴더로 보관된 것은
        '비교하지 않음'으로 이름이 나온다(조용히 건너뛰지 않는다)."""
        good = sha("보관 때의 바이트\n")
        cases = [("empty object", {}, "file"), ("null hash only", {"sha256": None}, "file"),
                 ("regular without hash", {"kind": "regular", "sha256": None}, "file"),
                 ("unread", {"kind": "regular", "sha256": None, "unread": True}, "file"),
                 ("not an object", [good], "file"), ("bad hash", {"kind": "regular", "sha256": "xyz"}, "file"),
                 ("link but now a file", {"kind": "link", "sha256": None}, "file"),
                 ("no kind with a hash", {"sha256": good}, "file")]
        for label, side, what in cases:
            with self.subTest(sidecar=label):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                kd = os.path.join(self.dirs, ".ceo-kept")
                os.makedirs(kd)
                k = os.path.join(kd, "20260101T000000Z-1-1.compose")
                _write(k, "보관 때의 바이트\n")
                _write(k + ".kept", json.dumps(side))
                row = self.c03()["C03.ceo-kept"]
                self.assertEqual(row["status"], pf.WARN, "%s 곁 파일을 미확인으로 보지 않았다: %r" % (label, row))
                self.assertIn("미확인", row["detail"])
                self.assertIn(k, row["detail"])
        shutil.rmtree(self.dirs, ignore_errors=True)
        self.seed()
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.makedirs(os.path.join(kd, "20260101T000000Z-1-1.backup"))
        _write(os.path.join(kd, "20260101T000000Z-1-1.backup.kept"), json.dumps({"kind": "dir", "sha256": None}))
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.PASS, row)
        self.assertIn("비교하지 않음", row["detail"])
        self.assertIn("20260101T000000Z-1-1.backup", row["detail"])

    def test_the_warning_claims_only_what_it_measured(self):
        """경고는 잰 것을 말한다 — 사본의 바이트가 보관 뒤에 바뀌었다. '살아 있는 지침에는 없다'는 살아 있는 지침과 비교했을 때만:
        바뀐 내용이 살아 있는 지침에도 있으면 그렇게, 살아 있는 지침을 읽지 못하면 '비교하지 못했다'고 말한다(codex minor)."""
        self.seed()
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.makedirs(kd)
        k1 = os.path.join(kd, "20260101T000000Z-1-1.compose")
        _write(k1, USER)                                                  # 바뀐 사본 — 그 내용은 살아 있는 지침(USER)에 있다
        _write(k1 + ".kept", json.dumps({"schema": 2, "kind": "regular", "sha256": sha("보관 때의 바이트\n")}))
        row = self.c03()["C03.ceo-kept"]
        self.assertEqual(row["status"], pf.PASS, "바뀐 내용이 살아 있는 지침에 있는데 경고했다(★FY2a): %r" % row)
        self.assertIn("살아 있는 지침에도 있다", row["detail"])
        self.assertNotIn("살아 있는 지침에는 없다", row["detail"], "비교해 보면 있는데 없다고 말했다")
        k2 = os.path.join(kd, "20260101T000000Z-1-2.compose")
        _write(k2, USER + "다른 줄\n")
        _write(k2 + ".kept", json.dumps({"schema": 2, "kind": "regular", "sha256": sha("보관 때의 바이트\n")}))
        aside = self.p("MASTER_DIRECTIVE.md.aside")
        os.rename(self.p("MASTER_DIRECTIVE.md"), aside)                   # 살아 있는 지침을 읽을 수 없다
        try:
            row = self.c03()["C03.ceo-kept"]
        finally:
            os.rename(aside, self.p("MASTER_DIRECTIVE.md"))
        self.assertEqual(row["status"], pf.WARN, row)
        self.assertIn(k2, row["detail"])
        self.assertIn("비교하지 못했다", row["detail"])
        self.assertNotIn("살아 있는 지침에는 없다", row["detail"], "비교하지 않고 '없다'고 말했다")


class IntentRecovery(Rig):
    """★RULINGS EY2·FA2·FA3 — 교체는 복구할 수 있는 한 단위다: 살아 있는 이름이 바뀌기 전에 의도 기록(directives/.ceo-intent.json)
    이 남고, 조립 경로의 모든 입구가 끝나지 않은 의도를 먼저 마무리한다. 어느 지점에서 멈춰도 다음 실행은 옛 지침이나 (의도와
    함께) 새 지침을 보고, 빠졌거나 반쪽이거나 낡은 지침을 현행으로 받지 않는다. 멈춤은 실제 ceo_py 의 함수를 감싸 SystemExit 로
    끼워 넣는다(시간이 아니라)."""
    SAVE = "USERSAVE-EY2 읽은 뒤에 저장한 줄\n"

    def setUp(self):
        super().setUp()
        self.env["CYS_CEO_LOCK_WAIT_S"] = "5"

    def ns_mode(self, mode):
        ns = ceo_py_module()
        if mode == "deposit":
            ns["exchange_fn"] = lambda: None
        elif ns["exchange_fn"]() is None:
            self.skipTest("이 플랫폼에는 원자 맞바꾸기가 없다")
        return ns

    def crash_after(self, ns, name, before=None):
        orig = ns[name]

        def wrapped(*a, **kw):
            if before:
                before()
            orig(*a, **kw)
            raise SystemExit("injected crash after %s" % name)
        ns[name] = wrapped

    def compose(self, ns):
        return ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), ""])

    def candidates(self):
        return [f for f in os.listdir(self.dirs) if f.endswith(".candidate")]

    def assert_recovered_composed(self, body):
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + body, "다음 실행이 바른 조립에 이르지 않았다")
        self.assertIsNone(self.read(".ceo-intent.json"), "의도 기록이 남았다")
        self.assertEqual(self.candidates(), [], "후보 파일이 남았다")

    # ── codex 순서 ② ──
    def test_crash_after_exchange_before_validation_recovers_the_save(self):
        """B 를 읽은 뒤 B+E 저장 → 낡은 후보 H+S+B 가 맞바꿔 들어감 → 검사 전에 멈춤. 다음 실행은 낡은 후보를 현행으로 받지
        않고, 밀려난 파일(B+E)을 새 진실로 보고 그것으로 조립한다(종전: current-fix 가 H+S+B 를 받아 E 를 잃었다)."""
        ns = ceo_py_module()
        real = ns["exchange_fn"]()
        if real is None:
            self.skipTest("이 플랫폼에는 원자 맞바꾸기가 없다")
        self.seed()

        def ex_then_crash(a, b):
            if not a.endswith(".candidate"):
                return real(a, b)               # 방법 시험(swap_mode)의 맞바꾸기는 그대로
            _write(self.p("MASTER_DIRECTIVE.md"), USER + self.SAVE)    # 읽은 뒤 · 맞바꾸기 전의 저장
            real(a, b)
            raise SystemExit("crash right after the exchange")
        ns["exchange_fn"] = lambda: ex_then_crash
        with self.assertRaises(SystemExit):
            self.compose(ns)
        self.assert_recovered_composed(USER + self.SAVE)
        self.assertIn(HEADER + SEP + USER, [_slurp(k, encoding="utf-8") for k in self.kept()],
                      "잠깐 살아 있던 낡은 후보를 보관하지 않았다(EY1)")

    def test_second_save_before_the_rollback_is_never_moved_out(self):
        """맞바꾸기 직후(검사 전) 살아 있는 파일에 두 번째 저장. 되돌리기는 지금 살아 있는 것을 다시 읽는다 — 우리 후보가 아니면
        그것은 새 편집이라 그대로 두고(보류 · 알림), 앞의 저장은 보관 폴더에 남는다(종전: 무조건 맞바꿔 두 번째 저장을 밀어냈다)."""
        ns = ceo_py_module()
        real = ns["exchange_fn"]()
        if real is None:
            self.skipTest("이 플랫폼에는 원자 맞바꾸기가 없다")
        self.seed()
        second = USER + "SECOND-SAVE 맞바꾸기 직후 두 번째 저장\n"
        fired = []

        def ex_with_two_saves(a, b):
            if not a.endswith(".candidate"):
                return real(a, b)               # 방법 시험(swap_mode)의 맞바꾸기는 그대로
            if not fired:
                fired.append(1)
                _write(self.p("MASTER_DIRECTIVE.md"), USER + self.SAVE)
                real(a, b)
                with open(self.p("MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
                    f.write(second)
                return
            real(a, b)
        ns["exchange_fn"] = lambda: ex_with_two_saves
        nf = os.path.join(self.tmp, "nf.txt")
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), second, "되돌리기가 두 번째 저장을 살아 있는 이름에서 밀어냈다")
        self.assertIn(USER + self.SAVE, [_slurp(k, encoding="utf-8") for k in self.kept()], "앞의 저장이 보관되지 않았다")
        self.assertEqual(rc, 6, "두 저장이 겹쳤는데 보류가 아니다")
        self.assertIn("두 저장이 겹침", _slurp(nf, encoding="utf-8"))

    # ── 멈춤 지점 표(보고서 Round 2 (2)) ──
    def test_crash_points_of_a_promotion(self):
        steps = [("exchange", "write_candidate"), ("exchange", "write_intent"), ("exchange", "prim_exchange"),
                 ("exchange", "keep_file"), ("deposit", "write_intent"), ("deposit", "keep_file"),
                 ("deposit", "place_candidate")]
        for mode, name in steps:
            with self.subTest(mode=mode, crash_after=name):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                ns = self.ns_mode(mode)
                self.assertIn(name, ns, "교체 단계 함수 %s 가 없다" % name)
                self.crash_after(ns, name)
                with self.assertRaises(SystemExit):
                    self.compose(ns)
                live = self.read("MASTER_DIRECTIVE.md")
                self.assertIn(live, (None, USER, HEADER + SEP + USER), "멈춘 뒤 반쪽 지침: %r" % live)
                if live is None:
                    self.assertIsNotNone(self.read(".ceo-intent.json"), "살아 있는 이름이 빠졌는데 의도 기록이 없다")
                self.assert_recovered_composed(USER)
                self.assertIn(USER, [_slurp(k, encoding="utf-8") for k in self.kept()] + [self.read("MASTER_DIRECTIVE.md.pre-ceo")])

    def test_crash_points_of_a_demotion(self):
        for mode in ("exchange", "deposit"):
            with self.subTest(mode=mode):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                self.register()
                self.assertEqual(self.run_dept("promote-ceo")[0], 0)
                ns = self.ns_mode(mode)
                self.crash_after(ns, "prim_exchange" if mode == "exchange" else "keep_file")
                with self.assertRaises(SystemExit):
                    ns["do_demote"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), ""])
                rc, out, err = self.run_dept("down", "d0")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "다음 강등이 중단된 교체를 마무리하지 않았다")
                self.assertIsNone(self.read(".ceo-intent.json"))

    # ── codex B1 · B7(Round 3 — RULINGS FL4: 기록은 마지막에 지워지고, 복구는 기록된 동작을 끝낸다) ──
    def hook_save_before_swap(self, ns, text):
        """조립이 지침을 읽은 뒤 · 교체 전(첫 swap_unit 직전)에 사용자 저장 1회(제자리 쓰기)."""
        orig = ns["swap_unit"]
        fired = []

        def wrapped(md, *a, **kw):
            if not fired:
                fired.append(1)
                with open(self.p("MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
                    f.write(text)
            return orig(md, *a, **kw)
        ns["swap_unit"] = wrapped
        return fired

    def test_a_crash_after_the_record_is_removed_never_accepts_a_stale_composition(self):
        """codex B1 — 기록이 검증보다 먼저 지워져, 그 사이에서 멈추면 다음 실행이 낡은 조립을 현행으로 받아 읽은 뒤의 저장을
        잃었다. 기록은 마지막에 지워진다: 첫 기록 제거 바로 뒤에서 멈춰도 그 저장은 조립돼 살아 있다."""
        for mode in ("exchange", "deposit"):
            with self.subTest(mode=mode):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                ns = self.ns_mode(mode)
                fired = self.hook_save_before_swap(ns, USER + self.SAVE)
                self.crash_after(ns, "remove_intent")
                with self.assertRaises(SystemExit):
                    self.compose(ns)
                self.assertEqual(fired, [1], "끼어들기가 일어나지 않았다(시험 무효)")
                self.assert_recovered_composed(USER + self.SAVE)

    def test_the_tick_finishes_the_operation_after_restoring_a_newer_save(self):
        """codex B7 — 틱이 밀려난 새 진실을 되돌린 뒤 기록을 지우고 끝나 조립이 끝나지 않은 채 남았다(PENDING 이 없으면 다음 부서
        이벤트까지 · 기록도 없이). 틱은 기록된 동작을 끝낸다: 되돌린 저장으로 다시 조립하고 영수증까지 쓴 뒤 기록을 지운다."""
        for mode in ("exchange", "deposit"):
            with self.subTest(mode=mode):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                ns = self.ns_mode(mode)
                self.hook_save_before_swap(ns, USER + self.SAVE)
                self.crash_after(ns, "keep_file")
                with self.assertRaises(SystemExit):
                    self.compose(ns)
                self.assertFalse(os.path.exists(self.pending_path()))
                rc, out, err = self.run_dept("promote-if-pending")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + self.SAVE,
                                 "틱이 되돌리기만 하고 기록된 조립을 끝내지 않았다(codex B7)")
                self.assertIsNone(self.read(".ceo-intent.json"))
                self.assert_receipt_for(HEADER, USER + self.SAVE, HEADER + SEP + USER + self.SAVE)
                self.assertIn(USER + self.SAVE, [_slurp(k, encoding="utf-8") for k in self.kept()])

    def test_the_tick_closes_a_gap_without_a_launch(self):
        """FA3 — 10분 승격 대기 틱(promote-if-pending)이 PENDING 없이도 끝나지 않은 교체를 마무리한다(빠진 살아 있는 이름 포함) ·
        틱은 알림을 보내지 않는다."""
        self.seed()
        ns = self.ns_mode("deposit")
        self.crash_after(ns, "keep_file")
        with self.assertRaises(SystemExit):
            self.compose(ns)
        self.assertIsNone(self.read("MASTER_DIRECTIVE.md"), "시험 전제: 살아 있는 이름이 빠진 상태")
        self.assertFalse(os.path.exists(self.pending_path()))
        n0 = len(self.pushes_prefix("CEO "))
        rc, out, err = self.run_dept("promote-if-pending")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER, "틱이 빠진 이름을 메우지 않았다")
        self.assertIsNone(self.read(".ceo-intent.json"))
        self.assertEqual(len(self.pushes_prefix("CEO ")), n0, "틱이 알림을 보냈다")

    def test_read_only_entries_see_an_open_intent(self):
        self.seed()
        ns = self.ns_mode("exchange")
        self.crash_after(ns, "prim_exchange")
        with self.assertRaises(SystemExit):
            self.compose(ns)
        self.assertEqual(self.run_ceo_py("state", self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"))[1].strip(),
                         "intent")
        rc, out, _ = self.run_ceo_py("verify", self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"))
        self.assertEqual(rc, 1)
        self.assertIn("끝나지 않았다", out)
        env0 = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            for k in pf.PACK_DIR_ENV_KEYS:
                os.environ.pop(k, None)
            p = pf.Preflight(fix=False, skips=[])
            p.c03_content_pins()
            rows = {r["id"]: r for r in p.results}
        finally:
            os.environ.clear()
            os.environ.update(env0)
        self.assertEqual(rows["C03.ceo-intent"]["status"], pf.WARN)
        self.assertIn("끝나지 않은 CEO 지침 교체", rows["C03.ceo-intent"]["detail"])

    # ── ★RULINGS FL2(FA2 철회) — 이 코드는 아무것도 지우지 않는다 ──
    def test_an_orphan_candidate_goes_to_the_kept_folder(self):
        """기록이 이름 대지 않는 후보도 지우지 않고 보관 폴더로 옮긴다(이유 unused-candidate). 종전 FA2 는 '살아 있던 적이 없다'며
        지웠다 — 읽을 수 없는 기록을 치운 뒤에는 그것이 밀려난 판일 수 있었다(codex B3). 이 시험이 FA2 의 시험을 대신한다."""
        self.seed()
        orphan = self.p(".MASTER_DIRECTIVE.md.20260101T000000Z-1-1.candidate")
        _write(orphan, "never live\n")
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertFalse(os.path.exists(orphan), "이름 없는 후보가 지침 폴더에 남았다")
        self.assertEqual([_slurp(k, encoding="utf-8") for k in self.kept("unused-candidate")], ["never live\n"],
                         "이름 없는 후보를 지웠다(보관 폴더에 없다)")

    def test_a_cancelled_swap_keeps_its_candidate(self):
        """바꾸기 전에 멈춘 교체의 후보도 지우지 않는다 — 다음 입구가 그것을 보관 폴더로 옮기고 동작을 끝낸다."""
        for mode in ("exchange", "deposit"):
            with self.subTest(mode=mode):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                ns = self.ns_mode(mode)
                self.crash_after(ns, "write_intent")
                with self.assertRaises(SystemExit):
                    self.compose(ns)
                self.assert_recovered_composed(USER)
                self.assertEqual([_slurp(k, encoding="utf-8") for k in self.kept("unused-candidate")], [HEADER + SEP + USER],
                                 "바꾸기 전에 멈춘 교체의 후보를 지웠다")

    def test_an_unreadable_record_is_a_barrier(self):
        """codex B3 — 맞바꾼 뒤 멈춰 밀려난 판이 후보 이름에 있는데 기록을 읽을 수 없다. 종전: 첫 입구가 기록을 보관 폴더로 치우고
        다음 입구가 '이름 없는 후보'로 그 판을 지웠다. 이제 장벽이다: 아무것도 옮기거나 지우지 않고 조립·강등을 보류하며(알림
        한 번), 몇 번을 다시 와도(지명·틱) 그대로다."""
        ns = self.ns_mode("exchange")
        self.seed()
        self.crash_after(ns, "prim_exchange")
        with self.assertRaises(SystemExit):
            self.compose(ns)
        _write(self.p(".ceo-intent.json"), "{ 반쪽 기록")
        snap = lambda: {f: _slurp(self.p(f), "rb") for f in os.listdir(self.dirs) if os.path.isfile(self.p(f))}
        before = snap()
        self.assertIn(USER.encode(), before.values(), "시험 전제: 밀려난 판(평문 MASTER)이 후보 이름에 있다")
        for args in (("promote-ceo",), ("promote-ceo",), ("promote-if-pending",)):
            rc, out, err = self.run_dept(*args)
            self.assertIn(rc, (0, 5), out + err)
        after = snap()
        for f, b in before.items():
            self.assertEqual(after.get(f), b, "장벽인데 %s 가 바뀌거나 사라졌다" % f)
        self.assertIn(USER.encode(), after.values(), "밀려난 판이 지워졌다(codex B3)")
        self.assertEqual(self.kept(), [], "장벽인데 무언가를 옮겼다")
        self.assertEqual(self.pushes("CEO 지침 교체 기록 확인 필요"), 1, "장벽 알림이 한 번이 아니다(틱마다 보내지 않는다 — FA3)")


class _Crash(BaseException):
    """끼워 넣은 멈춤 — BaseException 이라 제품의 어떤 except 도 삼키지 않는다(죽은 프로세스)."""


class _Runaway(BaseException):
    """한 입구가 정한 연산 수를 넘었다 — 끝나지 않는 되풀이(S3 위반)."""


class _FS:
    """ceo_py 의 파일 시스템 기본 연산(상태를 바꾸는 것 — 만들기·fsync·이름 바꾸기·맞바꾸기·링크·지우기·폴더 만들기)에 번호를
    매긴다. crash_at 번째 연산 **뒤에서** 멈추고, 그 뒤의 연산은 하지 않고 다시 멈춘다(죽은 프로세스는 정리 코드도 돌지 않는다).
    save_at 번째 연산 **앞에서** 사용자 저장 1회. 연산마다 그 앞에서 살아 있는 지침을 관찰한다. 방법 시험(.ceo-probe.*)은 세지
    않는다. fsync 는 세되 실제로 하지 않는다(프로세스 안의 멈춤은 디스크 확정과 무관하다 — flush 실패는 FlushFailure 가 잰다)."""
    BUDGET = 400

    def __init__(self, crash_at=None, save_at=None, save=None, observe=None):
        self.crash_at, self.save_at, self.save, self.observe = crash_at, save_at, save, observe
        self.n, self.dead = 0, False

    def call(self, fn, paths, *a, **kw):
        if any(".ceo-probe." in str(p) for p in paths):
            return fn(*a, **kw)
        if self.dead:
            raise _Crash()
        self.n += 1
        if self.n > self.BUDGET:
            raise _Runaway()
        if self.observe:
            self.observe()
        if self.save_at == self.n:
            self.save()
        r = fn(*a, **kw)
        if self.crash_at == self.n:
            self.dead = True
            raise _Crash()
        return r


class _OsProxy:
    def __init__(self, fs):
        self._fs = fs

    def __getattr__(self, name):
        return getattr(os, name)

    def open(self, path, flags, *a, **kw):
        if flags & os.O_CREAT:
            return self._fs.call(os.open, (path,), path, flags, *a, **kw)
        return os.open(path, flags, *a, **kw)

    def fsync(self, fd):
        if stat.S_ISDIR(os.fstat(fd).st_mode):
            return None
        return self._fs.call(lambda: None, ())

    def rename(self, a, b):
        return self._fs.call(os.rename, (a, b), a, b)

    def replace(self, a, b):
        return self._fs.call(os.replace, (a, b), a, b)

    def link(self, a, b):
        return self._fs.call(os.link, (a, b), a, b)

    def unlink(self, p):
        return self._fs.call(os.unlink, (p,), p)

    def remove(self, p):
        return self._fs.call(os.remove, (p,), p)

    def mkdir(self, p, *a, **kw):
        return self._fs.call(os.mkdir, (p,), p, *a, **kw)

    def makedirs(self, p, mode=0o777, exist_ok=False):
        if os.path.isdir(p):
            return os.makedirs(p, mode, exist_ok)
        return self._fs.call(os.makedirs, (p,), p, mode, exist_ok)


class CrashProperty(Rig):
    """★RULINGS FL7 — 한 속성 시험이 S1–S3 을 진다(T6 의 멈춤 속성 시험 형식). 승격·강등·복구의 파일 시스템 기본 연산을 감싸
    **모든 연산 하나하나의 뒤에서** 멈춤을 끼워 넣고, 멈춘 지점마다 다음 입구(같은 동작의 입구 · 10분 틱)를 돌리되 그 입구도 **그
    연산마다** 멈춘다(두 번째 고장). 그리고 깨끗한 입구 하나가 마무리한 뒤 불변식을 단언한다:
      I1 살아 있던 적이 있는 모든 판(연산마다 관찰)과 어느 걸음에서든 끼워 넣은 사용자 저장은 살아 있는 지침이나 보관 폴더에
         있다(사용자 자신의 저장이 덮은 판만 예외 — 그것은 사용자가 바꾼 것이다).
      I2 살아 있는 이름이 있고, 더 새 저장이 있을 때 낡은 조립·강등 결과가 아니다(끝난 입구마다 · 마지막에).
      I3 기록된 동작은 끝났거나(조립 = 현행 머리말 + 마지막 본문 · 유효한 영수증 / 강등 = 마지막 본문 · 영수증 없음) 알림과 함께
         보류됐고, 남은 기록·후보가 없다(멈추지 않은 틱은 남은 기록을 끝낸다).
      I4 모든 입구가 끝났다(입구마다 연산 상한 — 넘으면 끝나지 않는 되풀이).
    맞바꾸기 경로(맥·리눅스)와 맡기고 먼저 경로(윈도우 등)를 모두 돈다. 시간은 쓰지 않는다(EZ1 — sleep 없음 · 끼워 넣기는 연산
    번호로). 저장은 제자리 쓰기와 원자 이름 바꾸기를 번갈아 쓴다."""
    SAVE_LINE = "USERSAVE-FL7 어느 걸음에서든 끼워 넣은 저장\n"
    _code = None

    @classmethod
    def code(cls):
        if cls._code is None:
            src = extract_shell_function(_slurp(DEPT, encoding="utf-8"), "ceo_py")
            body = src.split("<<'PY'\n", 1)[1].rsplit("\nPY\n", 1)[0].rstrip()
            tail = "sys.exit(main(sys.argv))"
            assert body.endswith(tail)
            cls._code = compile(body[: -len(tail)], DEPT + ":ceo_py", "exec")
        return cls._code

    def module(self, mode, fs=None):
        ns = {"__name__": "ceo_py_property"}
        exec(self.code(), ns)
        real_ex, real_nr = self.real_ex, self.real_nr
        if fs is not None:
            ns["os"] = _OsProxy(fs)
            ns["noreplace_fn"] = (lambda: None) if real_nr is None else (lambda: (lambda a, b: fs.call(real_nr, (a, b), a, b)))
            ex = None if real_ex is None else (lambda a, b: fs.call(real_ex, (a, b), a, b))
        else:
            ex = real_ex
        ns["exchange_fn"] = (lambda: None) if mode == "deposit" else (lambda: ex)
        ns["_MODE"][os.path.dirname(os.path.abspath(self.md))] = mode   # 방법 시험은 세지 않는다 — 시나리오의 경로로 고정
        return ns

    # ── 한 시나리오 ──
    def md_bytes(self):
        try:
            with open(self.md, "rb") as f:
                return f.read()
        except FileNotFoundError:
            return None

    def observe(self):
        try:
            st = os.stat(self.md)
            key = (st.st_ino, st.st_size, st.st_mtime_ns)
        except FileNotFoundError:
            return
        if key != self._seen:                # 바뀌었을 때만 읽는다(같은 inode·크기·시각 = 같은 판)
            self._seen = key
            b = self.md_bytes()
            if b is not None:
                self.observed.add(b)

    def make_save(self, op, how):
        text = ((HEADER + SEP + USER + self.SAVE_LINE) if op == "demote" else (USER + self.SAVE_LINE)).encode("utf-8")

        def save():
            cur = self.md_bytes()
            if cur is not None:
                self.excused.add(cur)       # 사용자 자신의 저장이 덮는 판
            if how == "in-place":
                with open(self.md, "wb") as f:
                    f.write(text)
            else:
                tmp = self.md + ".editor-tmp"
                with open(tmp, "wb") as f:
                    f.write(text)
                os.replace(tmp, self.md)
            self.saved, self._seen = text, None
        return save

    def expected(self, op):
        body = USER + (self.SAVE_LINE if self.saved else "")
        return ((HEADER + SEP + body) if op == "compose" else body).encode("utf-8")

    def fresh(self, op, mode):
        shutil.rmtree(self.dirs, ignore_errors=True)
        os.makedirs(self.dirs)
        _write(self.md, USER)
        _write(self.tmpl, TEMPLATE)
        if op == "demote":                  # 강등의 출발점 = 깨끗한 승격(세지 않는다)
            ns = self.module(mode)
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(ns["do_compose"]([self.tmpl, self.md, sha(TEMPLATE), ""]), 0)
        self.observed, self.excused, self.saved, self._seen = set(), set(), None, None
        self.observe()

    def entry(self, op, mode, kind, fs, nf):
        ns = self.module(mode, fs)
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            try:
                if kind == "tick":
                    return ns["do_recover"]([self.md, nf])
                if op == "compose":
                    return ns["do_compose"]([self.tmpl, self.md, sha(TEMPLATE), nf])
                return ns["do_demote"]([self.tmpl, self.md, nf])
            except _Crash:
                return "crash"

    def scenario(self, op, mode, plan):
        """plan = [(kind, crash_at, save_at, how), ...] — 그 뒤에 깨끗한 같은 동작 입구 하나. 돌려주는 값 = 입구마다 연산 수."""
        self.fresh(op, mode)
        where = "%s/%s %r" % (op, mode, plan)
        counts = []
        intent = self.p(".ceo-intent.json")
        for i, (kind, crash_at, save_at, how) in enumerate(list(plan) + [("op", None, None, None)]):
            had = os.path.lexists(intent)
            fs = _FS(crash_at, save_at, self.make_save(op, how) if save_at else None, self.observe)
            nf = os.path.join(self.tmp, "nf-prop.txt")
            if os.path.exists(nf):
                os.unlink(nf)
            try:
                res = self.entry(op, mode, kind, fs, nf)
            except _Runaway:
                self.fail("I4: 입구 %d 가 연산 %d 개를 넘었다(끝나지 않는 되풀이) — %s" % (i, _FS.BUDGET, where))
            counts.append(fs.n)
            self.observe()
            if res == "crash":
                continue
            live = self.md_bytes()
            notes = _slurp(nf, encoding="utf-8") if os.path.exists(nf) else ""
            if res in (6, 9):
                self.assertIn("NOTICE\t", notes, "I3: 보류(rc %s)에 알림이 없다 — %s" % (res, where))
                self.assertIsNotNone(live, "I2: 보류 뒤 살아 있는 이름이 없다 — %s" % where)
                continue
            if kind == "op" or had:
                self.assertIn(res, (0, 10, 11, 12), "입구 %d 의 rc — %s" % (i, where))
                self.assertFalse(os.path.lexists(intent), "I3: 끝난 입구 %d 뒤에 기록이 남았다 — %s" % (i, where))
                # 동작이 끝난 지침이거나, 교체가 검증된 **뒤**에 들어온 사용자 저장 그 자체(낡은 결과가 아니다 — 다음 확인이
                #   그것으로 다시 한다: 마지막 깨끗한 입구 뒤에는 동작이 끝난 지침이어야 한다).
                self.assertIn(live, (self.expected(op),) + ((self.saved,) if self.saved else ()),
                              "I2/I3: 끝난 입구 %d 뒤의 지침이 낡았거나 동작이 끝나지 않았다 — %s" % (i, where))
        self.final(op, where)
        return counts

    def final(self, op, where):
        live = self.md_bytes()
        self.assertEqual(live, self.expected(op), "I2/I3: 마지막 지침 — %s" % where)
        self.assertFalse(os.path.lexists(self.p(".ceo-intent.json")), "I3: 기록이 남았다 — %s" % where)
        self.assertEqual([f for f in os.listdir(self.dirs) if f.endswith(".candidate")], [], "후보가 남았다 — %s" % where)
        pool = {live} | {_slurp(k, "rb") for k in self.kept()}
        if self.saved:
            self.assertIn(self.saved, pool, "I1: 끼워 넣은 저장이 살아 있는 지침에도 보관 폴더에도 없다 — %s" % where)
        lost = [v[:60] for v in self.observed - self.excused if v not in pool]
        self.assertEqual(lost, [], "I1: 살아 있던 판이 사라졌다 — %s" % where)
        r = self.receipt()
        if op == "compose":
            self.assertIsNotNone(r, "I3: 조립을 끝냈는데 영수증이 없다 — %s" % where)
            self.assertEqual(r["composed_sha256"], hashlib.sha256(live).hexdigest(), "I3: 영수증 — %s" % where)
        else:
            self.assertIsNone(r, "I3: 강등을 끝냈는데 영수증이 남았다 — %s" % where)

    def test_every_crash_point_second_fault_and_save_keeps_s1_to_s3(self):
        base = ceo_py_module()
        self.real_ex, self.real_nr = base["exchange_fn"](), base["noreplace_fn"]()
        self.md, self.tmpl = self.p("MASTER_DIRECTIVE.md"), self.p("CEO_TEMPLATE.md")
        modes = (["exchange"] if self.real_ex is not None else []) + ["deposit"]
        if self.real_nr is None and os.name == "posix":
            modes.remove("deposit")
        hows = ("in-place", "rename")
        stats = {"crash points": 0, "second faults": 0, "saves": 0, "scenarios": 0}
        for op in ("compose", "demote"):
            for mode in modes:
                n1 = self.scenario(op, mode, [("op", None, None, None)])[0]
                self.assertGreater(n1, 5, "시험 무효: 감싼 연산이 없다")
                stats["scenarios"] += 1
                n2s = {}
                for k1 in range(1, n1 + 1):
                    stats["crash points"] += 1
                    for kind2 in ("op", "tick"):
                        n2 = n2s[k1, kind2] = self.scenario(op, mode, [("op", k1, None, None), (kind2, None, None, None)])[1]
                        stats["scenarios"] += 1
                        for k2 in range(1, n2 + 1):
                            self.scenario(op, mode, [("op", k1, None, None), (kind2, k2, None, None)])
                            stats["second faults"] += 1
                            stats["scenarios"] += 1
                # 첫 입구의 모든 연산 앞에서 저장 — 혼자, 그리고 그 뒤 모든 연산에서의 멈춤과 함께
                for s in range(1, n1 + 1):
                    how = hows[s % 2]
                    n1s = self.scenario(op, mode, [("op", None, s, how)])[0]
                    stats["saves"] += 1
                    stats["scenarios"] += 1
                    for k1 in range(s, n1s + 1):
                        self.scenario(op, mode, [("op", k1, s, how), ("tick" if k1 % 2 else "op", None, None, None)])
                        stats["saves"] += 1
                        stats["scenarios"] += 1
                # 복구 입구의 모든 연산 앞에서 저장(멈춘 지점마다)
                for k1 in range(1, n1 + 1):
                    kind2 = "tick" if k1 % 2 else "op"
                    for s2 in range(1, n2s[k1, kind2] + 1):
                        self.scenario(op, mode, [("op", k1, None, None), (kind2, None, s2, hows[(k1 + s2) % 2])])
                        stats["saves"] += 1
                        stats["scenarios"] += 1
        sys.stderr.write("\n[FL7] paths %s · crash points %d · second faults %d · saves injected %d · scenarios %d\n"
                         % ("+".join(modes), stats["crash points"], stats["second faults"], stats["saves"],
                            stats["scenarios"]))
        self.assertGreater(stats["second faults"], stats["crash points"])


class ReadUnknown(Rig):
    """★RULINGS FY1 — 이 경로의 모든 읽기는 한 읽기 함수를 지나고 답은 셋이다: 바이트 · 없음 · 모른다(오류). 어느 호출부도 '모른다'를
    '없음'·기본값·갈래 선택으로 바꾸지 않는다 — 기록은 그대로, 아무것도 옮기지 않고, 그 걸음은 알림과 함께 보류되며, 다음 입구가
    다시 읽는다(지나가는 실패는 저절로 풀리고 오래가는 실패는 보류로 남는다). 실패는 윈도우의 공유 위반처럼 open 이 PermissionError
    로 답하게 끼워 넣는다(시간이 아니라)."""
    SAVE = "USERSAVE-FY1 읽은 뒤에 저장한 줄\n"
    TITLE = "CEO 지침 보류(파일을 읽지 못함)"

    def failing_open(self, ns, hit, times=None):
        """ns 안의 open 이 hit(경로) 가 참인 경로를 읽을 때 PermissionError — times 번(None = 계속). 쓰기는 그대로."""
        real, left = open, [times]

        def fake(path, mode="r", *a, **kw):
            if ("r" in mode and "+" not in mode and hit(os.path.abspath(str(path)))
                    and (left[0] is None or left[0] > 0)):
                if left[0] is not None:
                    left[0] -= 1
                raise PermissionError(13, "sharing violation (injected)", str(path))
            return real(path, mode, *a, **kw)
        ns["open"] = fake

    def snapshot(self):
        out = {}
        for root, _dirs, files in os.walk(self.dirs):
            for f in files:
                if not f.endswith(".promote.lock"):
                    q = os.path.join(root, f)
                    out[os.path.relpath(q, self.dirs)] = _slurp(q, "rb")
        return out

    def compose(self, ns, nf):
        return ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])

    def crash_after_exchange_with_a_save(self):
        """B 를 읽은 뒤 B+E 저장 → 낡은 후보 H+S+B 가 맞바꿔 들어감 → 밀려난 B+E 를 보관하기 전에 멈춤. 돌려주는 값 = 후보 이름."""
        ns = ceo_py_module()
        real = ns["exchange_fn"]()

        def ex(a, b):
            if not a.endswith(".candidate"):
                return real(a, b)
            _write(self.p("MASTER_DIRECTIVE.md"), USER + self.SAVE)
            real(a, b)
            raise SystemExit("crash right after the exchange")
        ns["exchange_fn"] = lambda: ex
        with self.assertRaises(SystemExit):
            self.compose(ns, "")
        cands = [f for f in os.listdir(self.dirs) if f.endswith(".candidate")]
        self.assertEqual(len(cands), 1)
        self.assertEqual(_slurp(self.p(cands[0]), encoding="utf-8"), USER + self.SAVE, "시험 전제: 후보 이름에 밀려난 저장")
        return self.p(cands[0])

    def test_an_unreadable_candidate_after_the_exchange_holds_and_moves_nothing(self):
        """codex R1 — 맞바꾼 뒤 멈춘 기록의 후보(= 밀려난 새 저장)를 읽지 못하면, 종전에는 '맞바꾸지 않았다'로 읽혀 새 저장이 쓰이지
        않은 후보로 치워지고 낡은 조립이 현행으로 받아들여졌다. 이제 보류다 — 한 번 실패도, 계속 실패도. 풀리면 저장으로 조립한다."""
        if ceo_py_module()["exchange_fn"]() is None:
            self.skipTest("이 플랫폼에는 원자 맞바꾸기가 없다")
        for times in (1, None):
            with self.subTest(failure="once" if times else "lasting"):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                cand = self.crash_after_exchange_with_a_save()
                before = self.snapshot()
                for attempt in range(1 if times else 2):
                    ns = ceo_py_module()
                    self.failing_open(ns, lambda q: q == os.path.abspath(cand), times)
                    nf = os.path.join(self.tmp, "nf-r1-%d.txt" % attempt)
                    rc = self.compose(ns, nf)
                    self.assertEqual(rc, 6, "후보를 읽지 못했는데 보류하지 않았다(codex R1)")
                    self.assertEqual(self.snapshot(), before, "읽지 못했는데 무언가를 옮기거나 바꿨다")
                    self.assertIn(self.TITLE, _slurp(nf, encoding="utf-8"))
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + self.SAVE,
                                 "풀린 뒤에도 낡은 조립이 남았다 — 밀려난 저장으로 조립해야 한다")
                self.assertIsNone(self.read(".ceo-intent.json"))

    def test_every_read_of_the_path_holds_when_it_fails(self):
        """살아 있는 지침 · 틀 · 영수증 · 0.14 영수증 · .pre-ceo · 끝나지 않은 교체 기록 — 어느 것을 읽지 못해도 보류(rc 6 · 알림 ·
        파일 무변경)하고, 풀리면 다음 입구가 바르게 끝낸다(종전: 영수증은 '손상'으로 다시 쓰였고, 0.14 영수증·.pre-ceo 는 '없음'처럼
        지나갔고, 기록은 '형식이 틀린 기록'의 장벽 문구로 갔고, 지침·틀은 예외로 죽었다)."""
        def promoted():
            self.seed()
            self.assertEqual(self.run_dept("promote-ceo")[0], 0)

        def crashed_record():
            self.seed()
            ns = ceo_py_module()
            orig = ns["write_intent"]

            def stop(*a, **kw):
                orig(*a, **kw)
                raise SystemExit("crash after the record")
            ns["write_intent"] = stop
            with self.assertRaises(SystemExit):
                self.compose(ns, "")

        def legacy():
            self.seed()
            _write(self.p(".ceo-template-applied"), sha("다른 옛 지침\n") + "\n")    # 남은(낡은) 0.14 영수증
        cases = [("live", self.seed, "MASTER_DIRECTIVE.md"), ("template", self.seed, "CEO_TEMPLATE.md"),
                 ("receipt", promoted, ".ceo-receipt.json"), ("legacy receipt", legacy, ".ceo-template-applied"),
                 (".pre-ceo", promoted, "MASTER_DIRECTIVE.md.pre-ceo"), ("record", crashed_record, ".ceo-intent.json")]
        for label, setup, name in cases:
            with self.subTest(unreadable=label):
                shutil.rmtree(self.dirs, ignore_errors=True)
                setup()
                target = os.path.abspath(self.p(name))
                before = self.snapshot()
                ns = ceo_py_module()
                self.failing_open(ns, lambda q: q == target)
                nf = os.path.join(self.tmp, "nf-%s.txt" % name)
                rc = self.compose(ns, nf)
                self.assertEqual(rc, 6, "%s 를 읽지 못했는데 보류하지 않았다" % label)
                self.assertEqual(self.snapshot(), before, "%s 를 읽지 못했는데 무언가를 바꿨다" % label)
                self.assertIn(self.TITLE, _slurp(nf, encoding="utf-8"))
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
                self.assertIsNone(self.read(".ceo-intent.json"))

    def test_a_file_unreadable_when_kept_is_recorded_as_unread(self):
        """보관한 순간 읽지 못한 일반 파일은 '일반 파일이 아니었다'(sha256 null 만)가 아니라 **읽지 못했다**로 적힌다(FY1 · FY2c)."""
        self.seed()
        ns = ceo_py_module()
        kd = os.path.abspath(os.path.join(self.dirs, ".ceo-kept"))
        self.failing_open(ns, lambda q: os.path.dirname(q) == kd and not q.endswith(".kept"))
        src = self.p("MASTER_DIRECTIVE.md.scratch")
        _write(src, "보관할 바이트\n")
        kept, _side = ns["keep_file"](src, self.p("MASTER_DIRECTIVE.md"), "test")
        doc = json.loads(_slurp(kept + ".kept", encoding="utf-8"))
        self.assertEqual(doc.get("kind"), "regular", "읽지 못한 일반 파일을 다른 종류로 적었다: %r" % doc)
        self.assertIs(doc.get("unread"), True, "읽지 못했다는 사실을 적지 않았다: %r" % doc)
        self.assertIsNone(doc.get("sha256"))
        self.assertEqual(_slurp(kept, encoding="utf-8"), "보관할 바이트\n")


class SafePrimitives(Rig):
    """★RULINGS EY3 — 살아 있는 이름은 반쪽 지침을 드러낼 수 없는 방법으로만 바뀐다: 맞바꾸기(맥·리눅스), 그것이 없으면 맡기고
    먼저(살아 있는 파일을 보관 폴더로 옮긴 뒤 **다 쓴** 후보를 덮지 않는 이동으로 놓기 — 의도 기록 아래). 살아 있는 이름에 직접
    쓰는 일은 없고, 둘 다 안 되는 파일 시스템에서는 보류한다(지침 무변경) — codex 순서 ④."""

    class _Os:
        """ceo_py 모듈 안의 `os` 만 바꾼다 — 하드 링크가 없는 파일 시스템(EPERM)과, 살아 있는 이름을 쓰기로 연 기록."""
        def __init__(self, real, md):
            self._real, self._md, self.opened_live = real, md, []

        def __getattr__(self, name):
            return getattr(self._real, name)

        def link(self, a, b):
            raise PermissionError(1, "hard links unsupported here", a)

        def open(self, path, flags, *a, **kw):
            if os.path.abspath(path) == os.path.abspath(self._md) and flags & (os.O_WRONLY | os.O_RDWR):
                self.opened_live.append(flags)
            return self._real.open(path, flags, *a, **kw)

    def ns_without_links(self, noreplace=True):
        ns = ceo_py_module()
        ns["exchange_fn"] = lambda: None
        if not noreplace:
            ns["noreplace_fn"] = lambda: None
        proxy = self._Os(os, self.p("MASTER_DIRECTIVE.md"))
        ns["os"] = proxy
        return ns, proxy

    def test_no_safe_primitive_holds_and_leaves_live_untouched(self):
        """맞바꾸기도 하드 링크도 덮지 않는 이름 바꾸기도 없다 → 보류(rc 6 · 알림 1건) · 살아 있는 지침 바이트 그대로 · 의도·후보
        없음(종전: 하드 링크가 안 되면 살아 있는 경로에 직접 써서 반쪽 지침이 남을 수 있었다)."""
        self.seed()
        ns, proxy = self.ns_without_links(noreplace=False)
        nf = os.path.join(self.tmp, "nf.txt")
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])
        self.assertEqual(rc, 6, "안전한 바꾸기 방법이 없는데 보류하지 않았다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "살아 있는 지침이 바뀌었다")
        self.assertIsNone(self.read(".ceo-intent.json"))
        self.assertEqual([f for f in os.listdir(self.dirs) if f.endswith(".candidate")], [])
        self.assertIn("CEO 지침 교체 보류(이 디스크에서는 안전하게 바꿀 수 없음)", _slurp(nf, encoding="utf-8"))
        self.assertEqual(proxy.opened_live, [], "살아 있는 이름을 쓰기로 열었다")

    def test_the_live_name_is_never_written_in_place(self):
        """하드 링크가 없는 파일 시스템(덮지 않는 이름 바꾸기는 있음): 승격도, 빈 살아 있는 이름을 채우는 복구도 살아 있는 경로를
        쓰기로 열지 않는다 — 다 쓴 후보를 덮지 않게 옮겨 놓는다(종전: 복구의 채우기가 O_EXCL 로 살아 있는 경로에 직접 썼다)."""
        ns0 = ceo_py_module()
        if ns0.get("noreplace_fn") is not None and ns0["noreplace_fn"]() is None:
            self.skipTest("이 플랫폼에는 덮지 않는 이름 바꾸기가 없다")
        self.seed()
        ns, proxy = self.ns_without_links()
        self.assertEqual(ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), ""]), 0)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertEqual(proxy.opened_live, [], "승격이 살아 있는 이름을 쓰기로 열었다")
        # 빈 살아 있는 이름 + 의도 기록(후보 없음 — 보관된 판만 남은 경우) → 다음 입구가 채운다(쓰기로 열지 않고).
        #   기록 = ★FL4 스키마 2(동작 하나에 하나 · 단계 'swap' — 진행 중인 교체의 이름은 swap 안에).
        rec = {"schema": 2, "id": "X", "op": "compose", "args": {"tmpl": self.p("CEO_TEMPLATE.md"), "tsha": sha(TEMPLATE)},
               "step": "swap", "notices": [], "kept_report": [], "done": {},
               "swap": {"purpose": "op", "mode": "deposit", "read_sha256": sha(HEADER + SEP + USER),
                        "candidate": ".MASTER_DIRECTIVE.md.X.candidate", "candidate_sha256": "0" * 64,
                        "kept": "X.compose", "reason": "compose"},
               "created_at": "2026-11-14T00:00:00Z"}
        kd = os.path.join(self.dirs, ".ceo-kept")
        os.replace(self.p("MASTER_DIRECTIVE.md"), os.path.join(kd, "X.compose"))
        _write(self.p(".ceo-intent.json"), json.dumps(rec))
        ns2, proxy2 = self.ns_without_links()
        ns2["close_intent"](self.p("MASTER_DIRECTIVE.md"), "")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER, "빈 살아 있는 이름을 채우지 않았다")
        self.assertEqual(proxy2.opened_live, [], "복구의 채우기가 살아 있는 이름을 쓰기로 열었다(제자리 쓰기)")
        self.assertTrue(os.path.exists(os.path.join(kd, "X.compose")), "보관된 판을 지웠다")


class OneRename(Rig):
    """★RULINGS FL3 — 살아 있는 이름은 한 번의 원자 이름 바꾸기로 유일한 보관 이름에 옮겨진다(codex B2: 링크 뒤 지우기는, 두 호출
    사이에 편집기의 원자 저장이 이름을 바꿔 놓으면 둘째 호출이 그 저장을 지웠다). 저장은 옮기는 바로 그 호출 뒤에 끼워 넣는다."""
    SAVE = "USERSAVE-FL3 살아 있는 이름이 옮겨지는 순간의 원자 저장\n"

    def test_an_atomic_save_while_the_live_name_is_moved_away_survives(self):
        self.seed()
        ns = ceo_py_module()
        ns["exchange_fn"] = lambda: None              # 맡기고 먼저(윈도우 등) — 살아 있는 이름이 옮겨지는 형식
        md = os.path.abspath(self.p("MASTER_DIRECTIVE.md"))
        fired, linked = [], []

        def save():
            if not fired:
                fired.append(1)
                tmp = md + ".editor-tmp"
                _write(tmp, USER + self.SAVE)
                os.replace(tmp, md)

        class _Os:
            def __getattr__(self, name):
                return getattr(os, name)

            def link(self, a, b):
                os.link(a, b)
                if os.path.abspath(a) == md:
                    linked.append(b)
                    save()

            def rename(self, a, b):
                os.rename(a, b)
                if os.path.abspath(a) == md:
                    save()
        ns["os"] = _Os()
        nr0 = ns["noreplace_fn"]()
        if nr0 is not None:
            def nr(a, b):
                nr0(a, b)
                if os.path.abspath(a) == md:
                    save()
            ns["noreplace_fn"] = lambda: nr
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), md, sha(TEMPLATE), ""])
        self.assertEqual(fired, [1], "끼어들기가 일어나지 않았다(시험 무효)")
        everywhere = [self.read("MASTER_DIRECTIVE.md") or ""] + [_slurp(k, encoding="utf-8") for k in self.kept()]
        self.assertTrue(any(self.SAVE in v for v in everywhere),
                        "살아 있는 이름이 옮겨지는 순간의 원자 저장이 지워졌다(링크 뒤 지우기 — codex B2)")
        self.assertEqual(linked, [], "살아 있는 이름을 링크로 옮겼다(한 번의 이름 바꾸기가 아니다)")
        self.assertEqual(rc, 0)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + self.SAVE, "이긴 저장으로 조립하지 않았다")
        self.assertIn(USER, [_slurp(k, encoding="utf-8") for k in self.kept()], "옮겨진 판이 보관 폴더에 없다")


class FlushFailure(Rig):
    """★RULINGS FL5 — 실패한 flush 는 실패다(codex B4: fsync_dir 가 폴더 flush 실패를 삼켜, 디스크에 확정되지 않은 의도 기록 아래서
    살아 있는 이름을 바꿨다). 후보나 교체 기록의 폴더 flush 가 실패하면 살아 있는 이름을 건드리기 전에 보류하고(rc 6 · 알림 1건),
    만든 것은 지우지 않는다. 용인은 '지원하지 않음'(EINVAL·ENOTSUP) 뿐 — 그때는 교체가 그대로 끝난다."""

    class _Os:
        """ceo_py 모듈 안의 `os` 만 바꾼다 — 폴더 fd 의 fsync 가 주어진 errno 로 실패한다(when(폴더) 가 참일 때만)."""
        def __init__(self, real, code, when):
            self._real, self._code, self._when, self.failed = real, code, when, 0

        def __getattr__(self, name):
            return getattr(self._real, name)

        def fsync(self, fd):
            if stat.S_ISDIR(self._real.fstat(fd).st_mode) and self._when():
                self.failed += 1
                raise OSError(self._code, os.strerror(self._code))
            return self._real.fsync(fd)

    def run_with(self, mode, code, when):
        shutil.rmtree(self.dirs, ignore_errors=True)
        self.seed()
        ns = ceo_py_module()
        if mode == "deposit":
            ns["exchange_fn"] = lambda: None
        proxy = self._Os(os, code, when)
        ns["os"] = proxy
        nf = os.path.join(self.tmp, "nf-%s.txt" % mode)
        if os.path.exists(nf):
            os.unlink(nf)
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])
        return rc, proxy, (_slurp(nf, encoding="utf-8") if os.path.exists(nf) else "")

    def modes(self):
        return (["exchange"] if ceo_py_module()["exchange_fn"]() is not None else []) + ["deposit"]

    def test_a_failed_flush_holds_before_the_live_name_is_touched(self):
        intent = lambda: os.path.lexists(self.p(".ceo-intent.json"))
        for mode in self.modes():
            for label, when in (("candidate", lambda: True), ("intent", intent)):
                with self.subTest(mode=mode, failing=label):
                    rc, proxy, notes = self.run_with(mode, errno.EIO, when)
                    self.assertGreater(proxy.failed, 0, "폴더 flush 실패를 끼워 넣지 못했다(시험 무효)")
                    self.assertEqual(rc, 6, "flush 가 실패했는데 보류하지 않았다(codex B4)")
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "확정되지 않은 기록 아래서 살아 있는 이름이 바뀌었다")
                    self.assertIn("CEO 지침 교체 기록 확인 필요", notes, "보류 알림이 없다")
                    self.assertIn("디스크에 확정하지", notes)
                    self.assertTrue([f for f in os.listdir(self.dirs) if f.endswith(".candidate")],
                                    "확실하지 않은데 후보를 치웠다(모든 산출물을 남긴다)")
                    if label == "intent":
                        self.assertTrue(intent(), "교체 기록을 치웠다")
                    rc, out, err = self.run_dept("promote-ceo")
                    self.assertEqual(rc, 0, out + err)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER, "다음 입구가 이어 가지 않았다")
                    self.assertIsNone(self.read(".ceo-intent.json"))

    def test_an_unsupported_directory_flush_is_tolerated(self):
        for mode in self.modes():
            for code in (errno.EINVAL, errno.ENOTSUP):
                with self.subTest(mode=mode, errno=errno.errorcode[code]):
                    rc, proxy, notes = self.run_with(mode, code, lambda: True)
                    self.assertGreater(proxy.failed, 0, "폴더 flush 를 부르지 않았다(시험 무효)")
                    self.assertEqual(rc, 0, notes)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)


class SaveDuringSwap(Rig):
    """★RULINGS DC1 — 승격·강등은 이제 살아 있는 지침이 아닌 본문을 쓰지 않는다: 원본을 읽은 뒤·교체 전에 사용자 저장이
    끼어들면 그 새 바이트는 살아 있는 파일이나 백업 어딘가에 있다(어느 쪽에도 없는 경우는 없다). 실제 ceo_py 코드를 모듈로
    싣고 교체 직전의 함수를 감싸 저장을 결정론으로 끼워 넣는다."""
    EDIT = "USERSAVE-DC1 교체 직전에 저장한 줄\n"

    def everywhere(self):
        """살아 있는 파일 + 디렉티브 폴더의 모든 백업·사본(이름이 MASTER_DIRECTIVE.md 로 시작하는 파일) 바이트."""
        out = {}
        for f in os.listdir(self.dirs):
            if f.startswith("MASTER_DIRECTIVE.md") and not f.endswith(".lock") and os.path.isfile(self.p(f)):
                out[f] = _slurp(self.p(f), encoding="utf-8")
        for k in self.kept():
            out[os.path.relpath(k, self.dirs)] = _slurp(k, encoding="utf-8")
        return out

    def hook_first_live_write(self, ns, save):
        """살아 있는 MASTER 를 처음 건드리는 함수(지금 코드: swap_unit · DC1: swap_live · 종전: write_atomic)에 들어가기 직전 save() 1회."""
        md = self.p("MASTER_DIRECTIVE.md")
        name = next(n for n in ("swap_unit", "swap_live", "write_atomic") if n in ns)
        orig = ns[name]
        fired = []

        def wrapped(path, *a, **kw):
            if os.path.abspath(path) == os.path.abspath(md) and not fired:
                fired.append(1)
                save()
            return orig(path, *a, **kw)
        ns[name] = wrapped
        return fired

    @staticmethod
    def modes(ns):
        """교체의 두 방식 — 맞바꾸기(맥·리눅스, 이 플랫폼에 있을 때)와 이름 옮기기(윈도우 등의 대안 · 여기서 강제로 잰다)."""
        out = []
        if ns["exchange_fn"]() is not None:
            out.append("exchange")
        out.append("rename")
        return out

    @staticmethod
    def force(ns, mode):
        if mode == "rename":
            ns["exchange_fn"] = lambda: None

    def save_in_place(self, text):
        with open(self.p("MASTER_DIRECTIVE.md"), "w", encoding="utf-8") as f:
            f.write(text)

    def save_by_rename(self, text):
        tmp = self.p("MASTER_DIRECTIVE.md.editor-tmp")
        _write(tmp, text)
        os.replace(tmp, self.p("MASTER_DIRECTIVE.md"))

    def test_save_between_read_and_promotion_write_is_never_lost(self):
        for mode in self.modes(ceo_py_module()):
          for how in ("in-place", "rename"):
            with self.subTest(swap=mode, save=how):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                ns = ceo_py_module()
                self.force(ns, mode)
                saver = self.save_in_place if how == "in-place" else self.save_by_rename
                fired = self.hook_first_live_write(ns, lambda: saver(USER + self.EDIT))
                rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), ""])
                self.assertEqual(fired, [1], "끼어들기가 일어나지 않았다(시험 무효)")
                held = [k for k, v in self.everywhere().items() if self.EDIT in v]
                self.assertTrue(held, "교체 직전 저장(%s)이 살아 있는 파일에도 백업에도 없다: rc=%r %r"
                                % (how, rc, self.everywhere()))
                live = self.read("MASTER_DIRECTIVE.md")
                if rc in (0, 12):
                    self.assertIn(self.EDIT, live, "승격이 끝났는데 살아 있는 지침이 저장 전 본문이다(DC1)")
                    self.assertEqual(live, HEADER + SEP + USER + self.EDIT)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER + self.EDIT,
                                     "백업이 저장 전 본문이다 — 승격 전 본부 지침은 저장된 것이다")
                else:
                    self.assertEqual(live, USER + self.EDIT, "보류했는데 저장한 파일이 살아 있지 않다")

    def test_save_while_the_live_name_is_empty_wins(self):
        """이름 옮기기 방식(맞바꾸기가 없는 플랫폼)의 두 단계 사이 — 살아 있는 이름이 비어 있는 순간 — 에 편집기가 새 파일을
        만들면 그 파일이 이긴다(덮지 않는다)."""
        self.seed()
        ns = ceo_py_module()
        self.force(ns, "rename")
        self.assertIn("place_candidate", ns, "교체가 '덮지 않는 자리 놓기'를 쓰지 않는다")
        orig = ns["place_candidate"]
        md = self.p("MASTER_DIRECTIVE.md")
        fired = []

        def wrapped(src, dst):
            if os.path.abspath(dst) == os.path.abspath(md) and not fired:
                fired.append(1)
                _write(md, USER + self.EDIT)
            return orig(src, dst)
        ns["place_candidate"] = wrapped
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), md, sha(TEMPLATE), ""])
        self.assertEqual(fired, [1])
        self.assertEqual(rc, 0, "다시 읽어 저장된 본문으로 승격해야 한다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + self.EDIT)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER + self.EDIT)
        self.assertEqual([f for f in os.listdir(self.dirs) if ".displaced" in f], [], "옮겨 둔 옛 파일이 남았다")

    def test_save_between_read_and_demotion_write_is_never_lost(self):
        for mode in self.modes(ceo_py_module()):
            with self.subTest(swap=mode):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed()
                self.register()
                self.assertEqual(self.run_dept("promote-ceo")[0], 0)
                composed = self.read("MASTER_DIRECTIVE.md")
                ns = ceo_py_module()
                self.force(ns, mode)
                fired = self.hook_first_live_write(ns, lambda: self.save_in_place(composed + self.EDIT))
                rc = ns["do_demote"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), ""])
                self.assertEqual(fired, [1])
                held = [k for k, v in self.everywhere().items() if self.EDIT in v]
                self.assertTrue(held, "강등 직전 저장이 살아 있는 파일에도 백업에도 없다: rc=%r %r" % (rc, self.everywhere()))
                if rc == 0:
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER + self.EDIT,
                                     "강등이 저장 전 본문을 돌려줬다(DC1)")

    def test_live_name_never_absent_during_an_exchange_swap(self):
        """맞바꾸기 방식에서는 교체 중에도 살아 있는 이름이 한순간도 비지 않는다 — 읽는 쪽(훅·배달)이 '파일 없음'을 볼 수 없다.
        읽는 스레드가 200번의 교체 내내 여는 동안 FileNotFoundError 0."""
        import threading
        ns = ceo_py_module()
        if ns["exchange_fn"]() is None:
            self.skipTest("이 플랫폼에는 원자 맞바꾸기가 없다(이름 옮기기 방식 — 잠깐 빈다)")
        self.seed()
        md = self.p("MASTER_DIRECTIVE.md")
        stop, misses, reads = threading.Event(), [], [0]

        def reader():
            while not stop.is_set():
                try:
                    with open(md, "rb") as f:
                        f.read(16)
                    reads[0] += 1
                except FileNotFoundError:
                    misses.append(1)
        t = threading.Thread(target=reader)
        t.start()
        try:
            cur = USER.encode()
            for i in range(200):
                nxt = (USER + "v%d\n" % i).encode()
                ns["swap_unit"](md, nxt, sha(cur), "compose", "test")   # 밀려난 판은 보관 폴더로(EY1)
                cur = nxt
        finally:
            stop.set()
            t.join(60)
        self.assertGreater(reads[0], 0, "읽는 스레드가 돌지 않았다(시험 무효)")
        self.assertEqual(misses, [], "교체 중 살아 있는 이름이 비었다")

    def test_retries_are_bounded_and_end_in_a_hold(self):
        """원본이 매번 바뀌면 정해진 횟수만 다시 하고 보류한다(아무것도 쓰지 않음 · 알림 1건) — 끝없는 재시도 없음."""
        self.seed()
        ns = ceo_py_module()
        orig = ns["swap_unit"]
        n = []

        def always_changing(path, data, expect_sha, op, *a, **kw):
            if op == "compose":                 # 되돌리기(restore)는 끼어들지 않는다 — 조립마다 한 번 저장
                n.append(1)
                self.save_in_place(USER + "CHANGE %d\n" % len(n))
            return orig(path, data, expect_sha, op, *a, **kw)
        ns["swap_unit"] = always_changing
        nf = os.path.join(self.tmp, "nf.txt")
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])
        self.assertEqual(rc, 9, "계속 바뀌는 원본에서 보류(rc 9)가 아니다")
        self.assertLessEqual(len(n), 3, "재시도 상한이 없다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER + "CHANGE %d\n" % len(n), "마지막 저장이 살아 있지 않다")
        self.assertIsNone(self.read(".ceo-receipt.json"))
        self.assertIn("CEO 승격 보류(본부 지침이 조립 중에 계속 바뀜)", _slurp(nf, encoding="utf-8"))


class StaleLegacyReceipt(Rig):
    """★RULINGS DC2 — 남은 0.14 영수증(.ceo-template-applied)은 조립된 지침을 0.14.x 승격으로 다시 분류하지 못한다: 0.14 영수증은
    그 해시가 살아 있는 파일과 같을 때만 증거다. 이전이 끝나는 쓰기들 사이에서 멈춰도 다음 승격 확인이 바르게 이어 가고,
    빠른 경로도 남은 0.14 영수증을 치운다."""
    OLD_HEADER = "# CEO 옛 머리말 (master of master · 단일소유 강제 · exit 7)\n"
    OLD_VENDOR = master_text("OLD-STANDARD-MASTER")
    USER_OLD = OLD_VENDOR + "USERFIX-OLD 승격 전에 고친 줄\n"
    EDIT = "CEO-EDIT-DC2 CEO 동안 고친 본문 줄\n"

    def migrated(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.seed(md=old_copy)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)
        return old_copy

    def test_crash_residue_then_edit_then_lost_receipt_keeps_the_edit(self):
        old_copy = self.migrated()
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")      # 이전의 마지막 쓰기 앞에서 멈춘 흔적
        edited = HEADER + SEP + self.USER_OLD + self.EDIT
        _write(self.p("MASTER_DIRECTIVE.md"), edited)                       # CEO 동안 고친 본문
        os.remove(self.p(".ceo-receipt.json"))                              # 새 영수증 유실
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), edited, "남은 0.14 영수증이 조립된 지침을 다시 분류했다(DC2)")
        self.assertIsNone(self.read(".ceo-template-applied"), "남은 0.14 영수증을 치우지 않았다")
        self.assert_receipt_for(HEADER, self.USER_OLD + self.EDIT, edited)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD + self.EDIT, "강등이 CEO 동안 고친 본문을 잃었다")

    def test_demotion_with_residue_and_no_receipt_keeps_the_edit(self):
        old_copy = self.migrated()
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        edited = HEADER + SEP + self.USER_OLD + self.EDIT
        _write(self.p("MASTER_DIRECTIVE.md"), edited)
        os.remove(self.p(".ceo-receipt.json"))
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD + self.EDIT, "강등이 낡은 .pre-ceo 를 돌려줬다(DC2)")

    def test_fast_path_cleans_legacy_residue(self):
        old_copy = self.migrated()
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        rc, out, err = self.run_dept("launch", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertIsNone(self.read(".ceo-template-applied"), "빠른 경로가 남은 0.14 영수증을 치우지 않았다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)

    def test_matching_legacy_receipt_is_still_evidence(self):
        """대조: 해시가 살아 있는 파일과 같은 0.14 영수증은 여전히 0.14.x 승격의 증거다(배포 표 밖 머리말이어도)."""
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.seed(md=old_copy)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)

    def test_interrupted_migration_at_every_write_recovers(self):
        """이전의 쓰기마다(지침 교체 · 0.14 영수증 정리 · 새 영수증) 그 뒤에서 멈춘 상태 → CEO 동안 고친 본문 → 다음 승격 확인과
        강등이 그 본문을 지킨다. 실제 ceo_py 를 모듈로 싣고 n 번째 쓰기에서 멈춘다(SystemExit)."""
        for stop_after in ("swap", "legacy", "receipt"):
            with self.subTest(stop_after=stop_after):
                shutil.rmtree(self.dirs, ignore_errors=True)
                old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
                self.seed(md=old_copy)
                _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
                _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
                ns = ceo_py_module()
                names = {"swap": "swap_unit", "legacy": "drop_legacy", "receipt": "write_receipt"}
                self.assertIn(names[stop_after], ns)
                orig = ns[names[stop_after]]

                def stop(*a, _orig=orig, **kw):
                    _orig(*a, **kw)
                    raise SystemExit("crash after %s" % stop_after)
                ns[names[stop_after]] = stop
                with self.assertRaises(SystemExit):
                    ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), ""])
                live = self.read("MASTER_DIRECTIVE.md")
                self.assertEqual(live, HEADER + SEP + self.USER_OLD, "멈춘 뒤 지침이 이전 결과가 아니다")
                _write(self.p("MASTER_DIRECTIVE.md"), live + self.EDIT)
                rec = self.p(".ceo-receipt.json")
                if os.path.exists(rec):
                    os.remove(rec)
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), live + self.EDIT)
                self.assertIsNone(self.read(".ceo-template-applied"))
                rc, out, err = self.run_dept("down", "d0")
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD + self.EDIT)
                self.register()


class ReportingFailureKeepsTheLifeCycle(Rig):
    """★RULINGS DC3 — 보고 자체가 실패해도 생명주기 규칙은 선다: PENDING 쓰기 실패는 치명이 아니다(stderr 1줄), 조립과 그 상태
    파일 쓰기가 둘 다 실패해도 `launch` 는 0 으로 끝나고 티켓 발급·편성 단계에 닿는다(합성 레인 — 실제 설치 무접촉)."""

    def unwritable(self, path):
        os.makedirs(path, exist_ok=True)
        os.chmod(path, 0o555)
        self.addCleanup(os.chmod, path, 0o755)
        try:
            open(os.path.join(path, ".probe"), "w").close()
        except PermissionError:
            return
        os.remove(os.path.join(path, ".probe"))
        self.skipTest("읽기 전용 폴더에도 쓸 수 있는 실행 계정(root) — 측정 불가")

    def assert_life_cycle_reached(self, rc, out, err):
        self.assertEqual(rc, 0, "launch 가 생명주기를 깼다\n" + out + err)
        self.assertIn("CEO 티켓 자동 발급 생략", err, "티켓 발급 단계에 닿지 않았다\n" + err)
        self.assertIn("편성 ensure 생략", err, "편성 단계에 닿지 않았다\n" + err)
        self.assertIn("가동 완료", out, out + err)

    def test_composition_and_pending_write_both_fail(self):
        self.seed()
        self.unwritable(os.path.join(self.home, ".cys", "state"))     # PENDING(ceo-pending) 쓰기 실패
        self.unwritable(self.dirs)                                     # 조립(잠금·지침·영수증) 실패
        rc, out, err = self.run_dept("launch", "d0")
        self.assert_life_cycle_reached(rc, out, err)
        self.assertIn("PENDING 기록 실패", err, "PENDING 쓰기 실패가 보이지 않는다(무음)")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)

    def test_boot_gate_pending_write_fails(self):
        self.seed(marker=False)
        self.unwritable(os.path.join(self.home, ".cys", "state"))
        rc, out, err = self.run_dept("launch", "d0")
        self.assert_life_cycle_reached(rc, out, err)
        self.assertIn("PENDING 기록 실패", err)


class LockIsReal(Rig):
    """★RULINGS DC1 — 승격·강등은 진짜 잠금(POSIX: 잠금 파일의 flock(2) · 윈도우: msvcrt.locking)을 상한 있는 대기로 잡는다.
    시간 안에 못 잡으면 '잠금 없이 진행'하지 않고 보류한다(알림 1건 · 지침 무변경) — 스크립트는 그래도 0 으로 끝난다."""

    def setUp(self):
        super().setUp()
        try:
            import fcntl  # noqa: F401
        except ImportError:
            self.skipTest("fcntl 없는 플랫폼 — 윈도우의 msvcrt 잠금은 이 맥에서 잴 수 없다")
        self.env["CYS_CEO_LOCK_WAIT_S"] = "1"

    def hold_lock(self):
        import fcntl
        fd = os.open(self.p("MASTER_DIRECTIVE.md.promote.lock"), os.O_RDWR | os.O_CREAT, 0o644)
        fcntl.flock(fd, fcntl.LOCK_EX)
        self.addCleanup(os.close, fd)
        return fd

    def test_a_held_lock_holds_promotion_and_demotion(self):
        import fcntl
        self.seed()
        fd = self.hold_lock()
        title = "CEO 승격 보류(다른 승격·강등이 진행 중)"
        for args in (("promote-ceo",), ("launch", "d0")):
            rc, out, err = self.run_dept(*args)
            self.assertEqual(rc, 5 if args == ("promote-ceo",) else 0, out + err)
            self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "잠금을 못 잡았는데 지침을 바꿨다(무잠금 진행)")
            self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"))
            self.register()
        self.assertEqual(self.pushes(title), 2, "보류 알림이 없다(데몬이 보관 중이 아니면 매번 1건)")
        _write(self.feedlist, "rid-l\t[pending]\tceo-notice\t%s\tdecision=-\n" % title)
        self.assertEqual(self.run_dept("launch", "d0")[0], 0)
        self.assertEqual(self.pushes(title), 2, "보관 중인 보류 알림을 또 올렸다(폭주)")
        fcntl.flock(fd, fcntl.LOCK_UN)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertIn("cys feed reply rid-l ceo-promoted", self.calls(), "잠금 보류 알림이 승격 뒤에 남았다")
        # 강등도 같은 잠금 — 잡혀 있으면 보류(지침 무변경 · 알림), 풀리면 강등.
        fcntl.flock(fd, fcntl.LOCK_EX)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER, "잠금을 못 잡았는데 강등했다")
        self.assertEqual(self.pushes("CEO 강등 보류(다른 승격·강등이 진행 중)"), 1)
        self.assertNotIn("무락 진행", out + err, "'잠금 없이 진행' 갈래가 남았다")

    def test_bad_wait_values_fall_back_to_the_bounded_default(self):
        """★RULINGS EY4 — 환경의 대기 값이 유한하지 않거나(nan·inf) 음수거나 숫자가 아니면 기본값(10 초)이다. 풀리지 않는 잠금
        앞에서 `launch` 가 그 상한 안에 돌아와 티켓·편성 단계에 닿는다(종전: nan 이면 마감이 영원히 오지 않아 launch 가 멈췄다).
        subprocess 의 120 초 상한은 멈춤을 끝낼 뿐이고, 판정은 rc·지침·단계 문구다."""
        self.seed()
        self.hold_lock()
        for bad in ("nan", "inf", "-1", "abc"):
            with self.subTest(wait=bad):
                self.env["CYS_CEO_LOCK_WAIT_S"] = bad
                try:
                    rc, out, err = self.run_dept("launch", "d0", timeout=120)
                except subprocess.TimeoutExpired:
                    self.fail("CYS_CEO_LOCK_WAIT_S=%s 에서 launch 가 돌아오지 않았다(끝없는 잠금 대기)" % bad)
                self.assertEqual(rc, 0, out + err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "잠금 없이 진행했다")
                self.assertIn("가동 완료", out, out + err)
        ns = ceo_py_module()
        for bad, want in (("nan", 10.0), ("inf", 10.0), ("-1", 10.0), ("abc", 10.0), ("", 10.0),
                          ("1e9", 120.0), ("0.5", 0.5), ("0", 0.0)):
            os.environ["CYS_CEO_LOCK_WAIT_S"] = bad
            try:
                self.assertEqual(ns["lock_wait_s"](), want, "대기 값 %r" % bad)
            finally:
                os.environ.pop("CYS_CEO_LOCK_WAIT_S", None)

    def test_the_lock_is_taken_by_the_real_code(self):
        """잠금을 지우는 변이는 이 시험이 잡는다: 잠금이 잡혀 있으면 do_compose 는 상한 안에 rc 8(보류)로 끝난다."""
        self.seed()
        self.hold_lock()
        ns = ceo_py_module()
        nf = os.path.join(self.tmp, "nf.txt")
        os.environ["CYS_CEO_LOCK_WAIT_S"] = "0.3"
        self.addCleanup(os.environ.pop, "CYS_CEO_LOCK_WAIT_S", None)
        rc = ns["do_compose"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), sha(TEMPLATE), nf])
        self.assertEqual(rc, 8)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertIn("CEO 승격 보류(다른 승격·강등이 진행 중)", _slurp(nf, encoding="utf-8"))
        rc = ns["do_demote"]([self.p("CEO_TEMPLATE.md"), self.p("MASTER_DIRECTIVE.md"), nf])
        self.assertEqual(rc, 8)


class LegacyShapes(Rig):
    """★RULINGS CB3·CG1 — 0.14.x 에서 통째 복사로 승격된 기계의 첫 승격 확인. '0.14.x 승격'은 양성 증거로만 안다: 0.14
    영수증(.ceo-template-applied) 또는 구분선 앞이 배포된 v0.14.x 머리말과 바이트까지 같다(구분선 없는 옛 사본은 CEO 머리말
    표지 3핀 또는 배포된 틀 전문과의 바이트 등가 — v0.14.0–6 의 틀에는 표지 3핀이 없다). 그 밖의 구분선 있는 지침은 조립이다
    (본문 = 구분선 뒤 · 영수증은 다시 쓴다 · .pre-ceo 는 그대로). 강등은 0.14.x 승격이면 승격 전 본부 지침을 돌려준다.

    | 시작 상태 | 본문 | 남는 것 | 알림 |
    | 배포된 v0.14.x 틀 그대로(영수증 없음 — 표가 증거) + .pre-ceo | .pre-ceo | — | 없음 |
    | 배포된 머리말 + CEO 동안 고친 본문 + .pre-ceo | .pre-ceo | 옛 지침 통째 사본 | 1건 |
    | 0.14 영수증 + 그대로인 복사본 + .pre-ceo | .pre-ceo | — | 없음 |
    | 0.14 영수증 + CEO 동안 고친 복사본 + .pre-ceo | .pre-ceo | 옛 지침 통째 사본 | 1건 |
    | 배포된 머리말, .pre-ceo 없음 | 옛 지침의 구분선 뒤 | 옛 지침 통째 사본 | 1건 |
    | 고친 0.14.x 머리말(표 밖), 0.14 영수증 없음 + .pre-ceo | 구분선 뒤(조립) | 지금 지침 통째 사본 | 1건(.pre-ceo 경로 포함) |
    | 지금 설치된 0.15 틀 그대로, 영수증 없음 + .pre-ceo | 구분선 뒤(조립 — 지침 무변경) | — | 없음 |
    | 구분선 없는 옛 사본 + .pre-ceo | .pre-ceo | 배포 틀과 다르면 사본 | 다르면 1건 |
    """
    OLD_HEADER = "# CEO 옛 머리말 (master of master · 단일소유 강제 · exit 7)\n"     # 배포 표 밖 — '고친 머리말'과 같다
    OLD_VENDOR = master_text("OLD-STANDARD-MASTER")
    USER_OLD = OLD_VENDOR + "USERFIX-OLD 승격 전에 고친 줄\n"
    MIGRATION = "CEO 지침 사본 보관(0.14.x 이전) — "
    HEADER_COPY = "CEO 지침 사본 보관(머리말) — "
    DEMOTE_COPY = "CEO 지침 사본 보관(강등) — "
    EDIT = "CEO-EDIT CEO 동안 고친 줄\n".encode("utf-8")

    def migrate(self, md, pre=True, legacy_hash=None):
        shutil.rmtree(self.dirs, ignore_errors=True)
        self.seed(md=md)
        if pre:
            _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        if legacy_hash:
            _write(self.p(".ceo-template-applied"), legacy_hash + "\n")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        return out + err

    def demote_to(self, want):
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), want, "강등 결과가 다르다\n" + out + err)

    @staticmethod
    def representatives(rel, key):
        """같은 키(전문·머리말 해시)마다 가장 늦은 태그 하나 — {키: (태그, 바이트)}."""
        reps = {}
        for tag in V014_TAGS:
            if tag in rel:
                reps[key(rel[tag])] = (tag, rel[tag])
        return reps

    # ── 배포된 v0.14.x 틀(로컬 태그) — 0.14 영수증 없이도 표가 증거다 ──
    def test_every_released_template_migrates_to_the_backup(self):
        rel = need_released(self)
        reps = self.representatives(rel, sha)
        self.assertGreaterEqual(len(reps), 1)
        for tag, tb in sorted(reps.values()):
            with self.subTest(tag=tag):
                n0 = len(self.pushes_prefix("CEO 지침 사본 보관"))
                self.migrate(tb)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD,
                                 "%s 승격본의 본문이 승격 전 본부 지침(.pre-ceo)이 아니다" % tag)
                self.assertEqual(self.captures("before-compose"), [], "배포본 그대로인 옛 지침을 사본으로 남겼다")
                self.assertEqual(len(self.pushes_prefix("CEO 지침 사본 보관")) - n0, 0)
                self.assert_receipt_for(HEADER, self.USER_OLD, HEADER + SEP + self.USER_OLD)
                self.demote_to(self.USER_OLD)
                self.assertEqual(glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo*")), [])

    def test_every_released_template_demotes_to_the_backup_before_migration(self):
        rel = need_released(self)
        for tag, tb in sorted(self.representatives(rel, sha).values()):
            with self.subTest(tag=tag):
                shutil.rmtree(self.dirs, ignore_errors=True)
                self.seed(md=tb)
                _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
                self.demote_to(self.USER_OLD)
                self.assertEqual(self.captures("before-demote"), [])

    def test_released_header_with_ceo_edits_keeps_the_old_directive(self):
        rel = need_released(self)
        with_sep = {t: b for t, b in rel.items() if SEP.encode() in b}
        reps = self.representatives(with_sep, lambda b: sha(b[: b.index(SEP.encode())]))
        self.assertGreaterEqual(len(reps), 1)        # 5종 전수는 V014Tables 가 잰다(태그 일부만 있는 체크아웃 허용)
        for tag, tb in sorted(reps.values()):
            with self.subTest(tag=tag):
                edited = tb + self.EDIT
                n0 = len(self.pushes_prefix(self.MIGRATION))
                self.migrate(edited)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)
                caps = self.captures("before-compose")
                self.assertEqual([_slurp(c, "rb") for c in caps], [edited], "옛 CEO 지침을 통째로 보관하지 않았다")
                lines = self.pushes_prefix(self.MIGRATION)[n0:]
                self.assertEqual(len(lines), 1)
                self.assertIn("— %s --body" % os.path.basename(caps[0]), lines[0])
                self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD, "백업을 덮었다")
                self.demote_to(self.USER_OLD)

    def test_released_header_without_backup_takes_the_tail(self):
        rel = need_released(self)
        tag = max((t for t in rel if SEP.encode() in rel[t]), key=V014_TAGS.index)
        edited = rel[tag] + self.EDIT
        tail = edited[edited.index(SEP.encode()) + len(SEP.encode()):].decode("utf-8")
        # 강등(이전 전) — 구분선 뒤 본문 + 옛 지침 통째 사본 + 알림 1건.
        shutil.rmtree(self.dirs, ignore_errors=True)
        self.seed(md=edited)
        self.demote_to(tail)
        self.assertEqual([_slurp(c, "rb") for c in self.captures("before-demote")], [edited])
        self.assertEqual(len(self.pushes_prefix(self.DEMOTE_COPY)), 1)
        # 이전 — 같은 결과 + 백업은 그 본문.
        self.migrate(edited, pre=False)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + tail)
        self.assertEqual([_slurp(c, "rb") for c in self.captures("before-compose")], [edited])
        self.assertEqual(len(self.pushes_prefix(self.MIGRATION)), 1)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), tail)
        self.demote_to(tail)

    # ── 0.14 영수증(.ceo-template-applied) — 배포 표 밖 머리말이어도 0.14.x 승격이다 ──
    def test_unedited_whole_copy_takes_the_backup_as_body(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.migrate(old_copy, legacy_hash=sha(old_copy))
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD,
                         "0.14.x 가 덮어쓴 사용자 본부 지침(백업)이 돌아오지 않았다")
        self.assertEqual(self.captures("before-compose"), [], "배포본 그대로인 사본은 보관할 사용자 바이트가 없다")
        self.assertEqual(self.pushes_prefix("CEO "), [])
        self.assertIsNone(self.read(".ceo-template-applied"))
        self.assert_receipt_for(HEADER, self.USER_OLD, HEADER + SEP + self.USER_OLD)
        self.demote_to(self.USER_OLD)

    def test_unedited_whole_copy_demotes_to_the_backup(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.seed(md=old_copy)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        self.demote_to(self.USER_OLD)
        self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"))
        self.assertEqual(self.captures("before-demote"), [])

    def test_edited_copy_with_a_stale_legacy_receipt_is_a_composition(self):
        """★RULINGS DC2 로 다시 씀(종전: 'CEO 동안 고친 복사본 + 0.14 영수증 → 본문 = .pre-ceo'): 0.14 영수증은 그 해시가 살아
        있는 파일과 같을 때만 증거다. 고친 복사본은 해시가 다르고 머리말도 배포 표 밖이라 0.14.x 증거가 없다 → 조립(본문 =
        구분선 뒤 · 고친 줄 그대로) + 지금 지침 통째 사본 + 알림 1건(.pre-ceo 가 다른 MASTER 임과 그 경로). 배포된 머리말의 고친
        복사본은 여전히 .pre-ceo 를 본문으로 쓴다(test_released_header_with_ceo_edits_keeps_the_old_directive)."""
        original = self.OLD_HEADER + SEP + self.OLD_VENDOR
        edited = original + self.EDIT.decode()
        tail = self.OLD_VENDOR + self.EDIT.decode()
        self.migrate(edited, legacy_hash=sha(original))
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + tail, "해시가 안 맞는 0.14 영수증이 증거로 쓰였다(DC2)")
        caps = self.captures("before-compose")
        self.assertEqual([_slurp(c, encoding="utf-8") for c in caps], [edited], "지금 지침을 통째로 보관하지 않았다")
        lines = self.pushes_prefix("CEO ")
        self.assertEqual(len(lines), 1, lines)
        self.assertTrue(lines[0].startswith("cys feed push --title " + self.HEADER_COPY + os.path.basename(caps[0])))
        self.assertIn(self.p("MASTER_DIRECTIVE.md.pre-ceo"), lines[0])
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD, "백업을 덮었다")
        self.assertIsNone(self.read(".ceo-template-applied"), "0.14 영수증을 치우지 않았다")
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)             # 조립 뒤에는 현행 — 알림 없음
        self.assertEqual(len(self.pushes_prefix("CEO ")), 1)
        self.demote_to(tail)

    # ── ★CG1: 양성 증거 없는 구분선 지침 = 조립 ──
    def test_edited_v014_header_without_legacy_receipt_is_a_composition(self):
        """고친 0.14.x 머리말 + 0.14 영수증 없음 → 조립(본문 = 구분선 뒤 · .pre-ceo 무접촉) + 알림 1건(.pre-ceo 가 다른 MASTER
        를 담고 있다는 것과 그 경로 — 보관 사본 알림 하나에 싣는다). 실제 v0.14.x 틀이 있으면 그 머리말에 한 줄을 더해 잰다."""
        shapes = {"fake": (self.OLD_HEADER + SEP + self.OLD_VENDOR).encode("utf-8")}
        rel = released_templates()
        if rel:
            tag = max((t for t in rel if SEP.encode() in rel[t]), key=V014_TAGS.index)
            tb = rel[tag]
            i = tb.index(SEP.encode())
            shapes[tag] = tb[:i] + "\n사용자가 CEO 동안 머리말에 넣은 줄".encode("utf-8") + tb[i:]
        for name, live in shapes.items():
            with self.subTest(shape=name):
                tail = live[live.index(SEP.encode()) + len(SEP.encode()):]
                n0 = len(self.pushes_prefix("CEO "))
                self.migrate(live)
                self.assertEqual(_slurp(self.p("MASTER_DIRECTIVE.md"), "rb"), HEADER.encode() + SEP.encode() + tail,
                                 "양성 증거 없는 지침의 본문이 구분선 뒤가 아니다(CG1)")
                self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD, ".pre-ceo 를 건드렸다")
                caps = self.captures("before-compose")
                self.assertEqual([_slurp(c, "rb") for c in caps], [live], "지금 지침(고친 머리말)을 보관하지 않았다")
                lines = self.pushes_prefix("CEO ")[n0:]
                self.assertEqual(len(lines), 1, "알림은 1건이어야 한다: %r" % lines)
                self.assertTrue(lines[0].startswith("cys feed push --title " + self.HEADER_COPY + os.path.basename(caps[0])))
                self.assertIn(self.p("MASTER_DIRECTIVE.md.pre-ceo"), lines[0], "알림이 .pre-ceo 의 경로를 담지 않았다")
                self.assertIn("다른 MASTER", lines[0])
                self.assert_receipt_for(HEADER, tail, HEADER.encode() + SEP.encode() + tail)
                self.demote_to(tail.decode("utf-8"))
                kept = self.kept("backup")
                self.assertEqual([_slurp(k, encoding="utf-8") for k in kept], [self.USER_OLD], "강등이 .pre-ceo 를 썼거나 지웠다")

    def test_copy_of_the_current_template_is_a_composition(self):
        """지금 설치된 0.15 틀과 바이트까지 같고 영수증이 없다 = 고치지 않은 MASTER 의 0.15 조립(영수증 유실) — 지침은 그대로,
        영수증만 다시 쓰고 .pre-ceo 는 그대로(다른 내용이어도 알림 없음 — CG1 '낡은 .pre-ceo 는 아무것도 바꾸지 않는다')."""
        self.migrate(TEMPLATE)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), TEMPLATE)
        self.assert_receipt_for(HEADER, VENDOR, TEMPLATE)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        self.assertEqual(self.captures("before-compose"), [])
        self.assertEqual(self.pushes_prefix("CEO "), [])
        self.demote_to(VENDOR)

    def test_unreleased_header_without_backup_is_a_composition(self):
        edited = self.OLD_HEADER + SEP + self.OLD_VENDOR + "CEO-EDIT\n"
        self.migrate(edited, pre=False)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.OLD_VENDOR + "CEO-EDIT\n")
        self.assertEqual([_slurp(c, encoding="utf-8") for c in self.captures("before-compose")], [edited])
        lines = self.pushes_prefix("CEO ")
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].startswith("cys feed push --title " + self.HEADER_COPY))
        self.assertNotIn("다른 MASTER", lines[0], ".pre-ceo 가 없는데 다르다고 알렸다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.OLD_VENDOR + "CEO-EDIT\n")
        self.demote_to(self.OLD_VENDOR + "CEO-EDIT\n")

    def test_unreleased_header_demotion_keeps_the_header(self):
        edited = self.OLD_HEADER + SEP + self.OLD_VENDOR + "CEO-EDIT\n"
        self.seed(md=edited)
        self.demote_to(self.OLD_VENDOR + "CEO-EDIT\n")
        caps = self.captures("before-demote")
        self.assertEqual([_slurp(c, encoding="utf-8") for c in caps], [edited])
        lines = self.pushes_prefix(self.DEMOTE_COPY)
        self.assertEqual(len(lines), 1)
        self.assertIn("— %s --body" % os.path.basename(caps[0]), lines[0])

    # ── 구분선 없는 옛 사본 ──
    def test_copy_without_separator_uses_backup_and_captures(self):
        six_kb = "# CEO (master of master) — 행동 규약\n단일소유 강제 · exit 7\n구분선 없는 옛 머리글 사본\n"
        self.migrate(six_kb)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)
        self.assertEqual([_slurp(c, encoding="utf-8") for c in self.captures("before-compose")], [six_kb])
        self.assertEqual(len(self.pushes_prefix(self.MIGRATION)), 1)
        # 같은 형상의 강등(조립 전) — 백업으로 되돌리고 지금 파일은 보관.
        shutil.rmtree(self.dirs)
        self.seed(md=six_kb)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        self.demote_to(self.USER_OLD)
        self.assertEqual([_slurp(c, encoding="utf-8") for c in self.captures("before-demote")], [six_kb])
        self.assertEqual(len(self.pushes_prefix(self.DEMOTE_COPY)), 1)

    # ── ★CG2: CEO 문면을 담은 .pre-ceo 는 본문이 아니다 ──
    def test_ceo_backup_is_never_used_as_body(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        bad_backups = {"marker-pins": "# CEO (master of master)\n단일소유 강제 · exit 7\n" + self.OLD_VENDOR,
                       "separator": HEADER + SEP + self.USER_OLD}
        for name, pre in bad_backups.items():
            for md in (old_copy, "# CEO (master of master) — 행동 규약\n단일소유 강제 · exit 7\n옛 사본\n"):
                with self.subTest(backup=name, md=md[:12]):
                    shutil.rmtree(self.dirs, ignore_errors=True)
                    self.seed(md=md)
                    _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), pre)
                    _write(self.p(".ceo-template-applied"), sha(md) + "\n")
                    n_hold = self.pushes("CEO 승격 보류(본부 지침 확인 필요)")
                    rc, out, err = self.run_dept("promote-ceo")
                    self.assertEqual(rc, 5, "CEO 문면 백업을 본문으로 썼다\n" + out + err)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md"), md)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), pre)
                    self.assertIsNone(self.read(".ceo-receipt.json"))
                    self.assertEqual(self.pushes("CEO 승격 보류(본부 지침 확인 필요)") - n_hold, 1)
                    n_hold = self.pushes("CEO 강등 보류(본부 지침 확인 필요)")
                    self.demote_to(md)
                    self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), pre)
                    self.assertEqual(self.pushes("CEO 강등 보류(본부 지침 확인 필요)") - n_hold, 1)


class SeparatorWithoutEvidence(Rig):
    """★RULINGS CB2 — 구분선 줄은 있는데 승격된 적이 있다는 증거(영수증·0.14 영수증·.pre-ceo·CEO 머리말 표지)가 없는
    본부 지침은 보류된다: 바이트 무이동, rc 6, 알림 1건. 부트 표지 게이트도 건너뛰지 않는다. 강등도 잘라 내지 않는다."""
    NEVER = VENDOR + "사용자가 붙여 넣은 문서 조각" + SEP + "그 아래 이어진 사용자 줄\n"
    TITLE = "CEO 승격 보류(본부 지침에 조립 구분선 — 승격 기록 없음)"

    def test_held_without_moving_a_byte(self):
        self.seed(md=self.NEVER)
        listing = lambda: sorted(f for f in os.listdir(self.dirs) if ".promote.lock" not in f)   # flock 락 파일은 종전대로
        before = listing()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.NEVER, "증거 없는 구분선 파일을 바꿨다")
        self.assertEqual(listing(), before, "파일이 생겼다(.pre-ceo·영수증·사본)")
        self.assertEqual(self.pushes(self.TITLE), 1)
        advice = self.pushes_prefix(self.TITLE)[0]
        self.assertIn("① 이 파일이 원래 CEO 지침이었다면 구분선과 그 앞(머리말)을 지우고 구분선 뒤 본문만", advice)
        self.assertIn("② 본부 지침에 그 줄이 우연히 들어간 것이라면", advice)
        # 자동 경로(대기 틱)도 같은 보류 — 알림은 1건(dedupe).
        _write(self.feedlist, "rid-u\t[pending]\tceo-notice\t%s\tdecision=-\n" % self.TITLE)
        self.set_pending()
        self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertEqual(self.pushes(self.TITLE), 1)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.NEVER)
        # 사용자가 그 줄을 지우면 정상 승격 — 보류 알림은 닫힌다.
        _write(self.p("MASTER_DIRECTIVE.md"), USER)
        self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER)
        self.assertIn("cys feed reply rid-u ceo-promoted", self.calls())

    def test_boot_marker_gate_is_not_skipped(self):
        self.seed(md=self.NEVER, marker=False)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertTrue(os.path.exists(self.pending_path()), "증거 없는 구분선 파일에 부트 게이트를 건너뛰었다")
        self.assertIn("부트", out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.NEVER)

    def test_demotion_does_not_strip_it(self):
        self.seed(md=self.NEVER)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.NEVER)
        self.assertEqual(self.pushes("CEO 강등 보류(본부 지침 확인 필요)"), 1)


class SymlinkedDirective(Rig):
    """★RULINGS CB5 — 심볼릭 링크인 본부 지침(또는 백업)은 보류된다(rc 6 + 알림) — 일반 파일로 바꿔 쓰지 않는다."""

    def setUp(self):
        super().setUp()
        if not hasattr(os, "symlink"):
            self.skipTest("심볼릭 링크 없는 플랫폼")

    def link_master(self):
        target = os.path.join(self.tmp, "real-master.md")
        _write(target, USER)
        os.remove(self.p("MASTER_DIRECTIVE.md"))
        os.symlink(target, self.p("MASTER_DIRECTIVE.md"))
        return target

    def test_promotion_holds_a_symlinked_master(self):
        self.seed()
        target = self.link_master()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 5, out + err)
        self.assertTrue(os.path.islink(self.p("MASTER_DIRECTIVE.md")), "링크를 일반 파일로 바꿨다")
        self.assertEqual(_slurp(target, encoding="utf-8"), USER)
        self.assertEqual(self.pushes("CEO 승격 보류(본부 지침 확인 필요)"), 1)

    def test_demotion_holds_a_symlinked_master(self):
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        composed = self.read("MASTER_DIRECTIVE.md")
        target = os.path.join(self.tmp, "real-master.md")
        _write(target, composed)
        os.remove(self.p("MASTER_DIRECTIVE.md"))
        os.symlink(target, self.p("MASTER_DIRECTIVE.md"))
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertTrue(os.path.islink(self.p("MASTER_DIRECTIVE.md")))
        self.assertEqual(_slurp(target, encoding="utf-8"), composed)
        self.assertEqual(self.pushes("CEO 강등 보류(본부 지침 확인 필요)"), 1)


class OnePredicate(Rig):
    """★RULINGS CB1·CG2 — '승격'의 한 뜻(.pre-ceo 실재 ∨ 구분선)을 파이썬 판독자 셋(정본 javis_preflight, javis_org, cys-dept 의
    ceo_py 와 bash 게이트)이 같은 픽스처 한 벌에 같은 답으로 낸다. .pre-ceo 는 실제 항목(파일·끊긴 링크·폴더·읽을 수 없는
    파일)으로 만들고 각 판독자의 사전적 실재 판정으로 읽는다. Rust — 팩 코어 `cys::pack::ceo_promoted_in`(가드 4 · A12 가
    그것 하나만 부른다)과 앱 — 은 같은 파일을 cargo 시험이 읽는다."""
    FIXTURE = os.path.join(SELF, "fixtures", "ceo-promoted-cases.json")

    def make_pre(self, md_p, kind):
        pre = md_p + ".pre-ceo"
        if kind == "file":
            _write(pre, "백업\n")
        elif kind == "dangling-link":
            if not hasattr(os, "symlink"):
                return False
            os.symlink(os.path.join(os.path.dirname(md_p), "no-such-target"), pre)
        elif kind == "directory":
            os.mkdir(pre)
        elif kind == "unreadable":
            if os.name != "posix":
                return False
            _write(pre, "백업\n", 0)
            self.addCleanup(os.chmod, pre, 0o644)
        else:
            self.assertIsNone(kind, "픽스처의 pre_ceo 종류를 모른다: %r" % kind)
        return True

    def test_every_python_reader_agrees_with_the_fixture(self):
        with open(self.FIXTURE, encoding="utf-8") as f:
            fx = json.load(f)
        self.assertEqual(fx["separator"], SEP)
        spec = importlib.util.spec_from_file_location("javis_org_cb1", os.path.join(BIN, "javis_org.py"))
        org = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(org)
        src = _slurp(DEPT, encoding="utf-8")
        funcs = extract_shell_function(src, "ceo_py")
        gate = re.search(r"^ceo_pre_present\(\)\{.*\}$", src, re.M)
        self.assertIsNotNone(gate, "cys-dept 의 bash 게이트 판정 ceo_pre_present 가 없다")
        self.assertGreaterEqual(len(fx["cases"]), 12)
        kinds, ran = set(), 0
        for i, c in enumerate(fx["cases"]):
            with self.subTest(case=c["name"]):
                md_p = os.path.join(self.tmp, "fx%d" % i, "MASTER_DIRECTIVE.md")
                os.makedirs(os.path.dirname(md_p))
                md_b = None if c["md"] is None else c["md"].encode("utf-8")
                if md_b is not None:
                    _write(md_p, md_b)
                if not self.make_pre(md_p, c["pre_ceo"]):
                    continue
                kinds.add(c["pre_ceo"])
                want = c["promoted"]
                pre_pf = pf.Preflight._c03_pre_ceo_present(md_p)
                self.assertEqual(pre_pf, c["pre_ceo"] is not None, "preflight 의 .pre-ceo 실재 판정")
                self.assertEqual(org._pre_ceo_present(md_p), pre_pf, "javis_org 의 .pre-ceo 실재 판정")
                self.assertEqual(pf.Preflight._c03_ceo_promoted(md_b, pre_pf), want, "preflight")
                self.assertEqual(org._ceo_promoted(md_b, org._pre_ceo_present(md_p)), want, "javis_org")
                r = subprocess.run(["bash", "-c", funcs + 'ceo_py promoted "$1"\n', DEPT, md_p],
                                   capture_output=True, text=True, encoding="utf-8", env=self.env, timeout=60)
                self.assertEqual(r.stdout.strip(), "yes" if want else "no", "cys-dept: %s" % r.stderr)
                r = subprocess.run(["bash", "-c", gate.group(0) + '\nceo_pre_present "$1.pre-ceo" && echo yes || echo no\n',
                                    DEPT, md_p], capture_output=True, text=True, env=self.env, timeout=60)
                self.assertEqual(r.stdout.strip(), "yes" if pre_pf else "no", "cys-dept bash 게이트")
                ran += 1
        self.assertGreaterEqual(ran, 12, "실제 항목으로 잰 행이 너무 적다")
        if os.name == "posix":
            self.assertEqual(kinds, {None, "file", "dangling-link", "directory", "unreadable"}, "항목 종류가 빠졌다")


class V014Tables(unittest.TestCase):
    """★RULINGS CG1 — cys-dept 에 컴파일된 0.14.x 표(배포된 머리말 5종 · 틀 전문 15종)를 로컬 배포 태그에서 다시 만들어 대조한다
    (태그가 없는 체크아웃은 알림을 남기고 건너뛴다). javis_preflight 의 사본과 src/pack_prev_vendor.rs 의 CEO_TEMPLATE 행도 같다."""

    @staticmethod
    def dept_table(name):
        m = re.search(r"^%s = frozenset\(\((.*?)\)\)" % name, _slurp(DEPT, encoding="utf-8"), re.S | re.M)
        if m is None:
            raise AssertionError("cys-dept 에 %s 가 없다" % name)
        return frozenset(re.findall(r'"([0-9a-f]{64})"', m.group(1)))

    def test_copies_agree(self):
        heads, tmpls = self.dept_table("V014_HEADER_SHA256"), self.dept_table("V014_TEMPLATE_SHA256")
        self.assertEqual((len(heads), len(tmpls)), (5, 15))
        self.assertEqual(heads, pf.Preflight._C03_V014_HEADER_SHA256, "preflight 사본 ≠ cys-dept")
        self.assertEqual(tmpls, pf.Preflight._C03_V014_TEMPLATE_SHA256, "preflight 사본 ≠ cys-dept")
        real = _slurp(os.path.join(PACK, "directives", "CEO_TEMPLATE.md"), "rb")
        cut = real.index(SEP.encode())
        self.assertNotIn(sha(real[:cut]), heads, "현행 0.15 머리말이 0.14.x 표에 있다 — 모든 조립이 0.14.x 로 읽힌다")
        self.assertNotIn(sha(real), tmpls)
        prev = os.path.join(REPO, "src", "pack_prev_vendor.rs")
        if os.path.isfile(prev):
            m = re.search(r'\("directives/CEO_TEMPLATE\.md", &\[(.*?)\]\)', _slurp(prev, encoding="utf-8"))
            self.assertIsNotNone(m)
            self.assertEqual(frozenset(re.findall(r'"([0-9a-f]{64})"', m.group(1))), tmpls,
                             "src/pack_prev_vendor.rs 의 CEO_TEMPLATE 행 ≠ cys-dept 틀 표")

    def test_tables_are_rederived_from_the_release_tags(self):
        rel = need_released(self)
        heads, tmpls = {}, {}
        for tag in V014_TAGS:
            if tag not in rel:
                continue
            tb = rel[tag]
            tmpls.setdefault(sha(tb), []).append(tag)
            i = tb.find(SEP.encode())
            if i < 0:
                self.assertLessEqual(V014_TAGS.index(tag), V014_TAGS.index("v0.14.15"), "%s 에 구분선이 없다" % tag)
                continue
            heads.setdefault(sha(tb[:i]), []).append(tag)
            r = subprocess.run(["git", "-C", REPO, "show", "%s:scripts/ceo_template_header.md" % tag],
                               capture_output=True, timeout=60)
            if r.returncode == 0:
                self.assertEqual(r.stdout, tb[:i], "%s: 첫 구분선 앞 ≠ scripts/ceo_template_header.md" % tag)
        missing = [t for t in V014_TAGS if t not in rel]
        d_heads, d_tmpls = self.dept_table("V014_HEADER_SHA256"), self.dept_table("V014_TEMPLATE_SHA256")
        if missing:
            print("\n[notice] 배포 태그 %d개가 이 체크아웃에 없다(%s …) — 있는 태그만 표의 부분집합으로 잰다"
                  % (len(missing), ", ".join(missing[:3])), file=sys.stderr)
            self.assertLessEqual(set(heads), d_heads)
            self.assertLessEqual(set(tmpls), d_tmpls)
        else:
            self.assertEqual(set(heads), d_heads, "cys-dept 머리말 표 ≠ 배포 태그")
            self.assertEqual(set(tmpls), d_tmpls, "cys-dept 틀 표 ≠ 배포 태그")
        # v0.14.0–6 의 틀에는 CEO 머리말 표지 3핀이 없다 — 구분선 없는 사본은 전문 표가 유일한 증거다(v014_whole_copy).
        early = [t for t in ("v0.14.0", "v0.14.6") if t in rel]
        for t in early:
            self.assertFalse(all(p in rel[t].decode("utf-8") for p in pf.MARKER_PINS), "%s 에 표지 3핀이 있다" % t)


class HqDirectiveChain(Rig):
    """C11 11.11: MASTER → 생성기 → (윈도우 해시 = CI) → 상위집합 검사 → preflight 승격 판정 → 앱 — 실제 출하 파일로."""
    REAL_TMPL = os.path.join(PACK, "directives", "CEO_TEMPLATE.md")
    REAL_MASTER = os.path.join(PACK, "directives", "MASTER_DIRECTIVE.md")

    def _real(self, path):
        return _slurp(path, encoding="utf-8", newline="")

    def test_unedited_master_composes_to_the_shipped_template(self):
        """조립은 줄을 더하지 않는다 — 고치지 않은 MASTER 의 조립 == 출하 CEO_TEMPLATE 바이트(CEO 줄 예산 그대로)."""
        tmpl, master = self._real(self.REAL_TMPL), self._real(self.REAL_MASTER)
        self.seed(md=master, tmpl=tmpl)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(_slurp(self.p("MASTER_DIRECTIVE.md"), "rb"), _slurp(self.REAL_TMPL, "rb"))

    def test_composed_real_directive_passes_c03_and_org_axis(self):
        tmpl, master = self._real(self.REAL_TMPL), self._real(self.REAL_MASTER)
        user = master + "\nUSERFIX-E6-1 사용자가 본부 지침 끝에 더한 줄\n"
        self.seed(md=user, tmpl=tmpl)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        header = tmpl[: tmpl.index(SEP)]
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), header + SEP + user)
        # preflight C03(같은 가짜 HOME · 무-fix) — 조립된 지침은 핀 축 PASS · 강등 경로 PASS · '재적용' 권유 없음.
        env0 = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            for k in pf.PACK_DIR_ENV_KEYS:
                os.environ.pop(k, None)
            p = pf.Preflight(fix=False, skips=[])
            p.c03_content_pins()
            rows = {r["id"]: r for r in p.results}
        finally:
            os.environ.clear()
            os.environ.update(env0)
        self.assertEqual(rows["C03.pin.master"]["status"], pf.PASS, rows["C03.pin.master"])
        self.assertEqual(rows["C03.demote-guard"]["status"], pf.PASS, rows["C03.demote-guard"])
        self.assertIn("구분선 뒤 본문", rows["C03.demote-guard"]["detail"])
        # javis_org CEO 축 — 조립된 지침은 '승격'(지침 ≠ 틀이어도).
        spec = importlib.util.spec_from_file_location("javis_org_t8", os.path.join(BIN, "javis_org.py"))
        org = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(org)
        org.HOME = self.home
        org._audit_daemon_status = lambda socket=None: (None, "test: no daemon")   # 실 cys 호출 금지(격리)
        os.remove(self.p("MASTER_DIRECTIVE.md.pre-ceo"))      # 백업 없이도 조립 자체로 승격을 안다
        axis = org._audit_ceo_axis(1)
        self.assertTrue(axis["promoted"], axis)
        self.assertTrue(axis["receipt"], axis)

    def test_separator_and_marker_copies_match_the_generator(self):
        """O-C11-2: 구분선은 생성기의 것 하나 — cys-dept·preflight·javis_org·팩 코어(src/pack.rs)·앱 사본이 모두 같은 바이트여야 한다."""
        gen_p = os.path.join(REPO, "scripts", "gen_ceo_template.py")
        if not os.path.isfile(gen_p):
            self.skipTest("scripts/gen_ceo_template.py 없음(배포 팩 트리 — repo 전용 축)")
        spec = importlib.util.spec_from_file_location("gen_ceo_template_t8", gen_p)
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        canon = gen.SEPARATOR
        self.assertEqual(SEP.encode("utf-8"), canon)
        dept_src = _slurp(DEPT, encoding="utf-8")
        lit = re.search(r'^SEP = "(.*?)"\.encode\("utf-8"\)$', dept_src, re.M)
        self.assertIsNotNone(lit, "cys-dept 의 구분선 정의를 찾지 못했다")
        self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"),
                         canon, "cys-dept 구분선 ≠ 생성기")
        self.assertEqual(pf.Preflight._C03_CEO_SEPARATOR, canon, "preflight 구분선 ≠ 생성기")
        org_src = _slurp(os.path.join(BIN, "javis_org.py"), encoding="utf-8")
        lit = re.search(r'^_CEO_SEPARATOR = "(.*?)"\.encode\("utf-8"\)$', org_src, re.M)
        self.assertIsNotNone(lit)
        self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"), canon,
                         "javis_org 구분선 ≠ 생성기")
        core = os.path.join(REPO, "src", "pack.rs")
        if os.path.isfile(core):
            lit = re.search(r'^pub const CEO_SEPARATOR: &str = "(.*?)";$', _slurp(core, encoding="utf-8"), re.M)
            self.assertIsNotNone(lit, "src/pack.rs 의 구분선 정의(가드 4 · A12)를 찾지 못했다")
            self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"), canon,
                             "팩 코어(ceo_promoted_in) 구분선 ≠ 생성기")
        app = os.path.join(REPO, "src-tauri", "src", "main.rs")
        if os.path.isfile(app):
            lit = re.search(r'^const CEO_SEPARATOR: &str = "(.*?)";$', _slurp(app, encoding="utf-8"), re.M)
            self.assertIsNotNone(lit)
            self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"), canon,
                             "앱 구분선 ≠ 생성기")
        m = re.search(r'^MARKER_PINS = (\(.*?\))$', dept_src, re.M)
        self.assertIsNotNone(m)
        self.assertEqual(tuple(ast.literal_eval(m.group(1))), tuple(pf.MARKER_PINS),
                         "cys-dept 표지 핀 ≠ preflight 정본")


if __name__ == "__main__":
    unittest.main(verbosity=2)
