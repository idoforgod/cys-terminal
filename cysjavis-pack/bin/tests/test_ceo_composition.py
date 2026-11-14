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
import glob
import hashlib
import importlib.util
import json
import os
import re
import shutil
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

    def captures(self, kind):
        return sorted(glob.glob(self.p("MASTER_DIRECTIVE.md.%s.*" % kind)))

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
        kept = sorted(glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo.*")))
        self.assertEqual(len(kept), 1, "다른 내용의 백업은 시각을 붙여 보관해야 한다: %r" % os.listdir(self.dirs))
        self.assertEqual(_slurp(kept[0], encoding="utf-8"), USER, "백업이 원본으로 쓰이지 않았는데 내용이 바뀌었다")
        self.assertIsNone(self.read(".ceo-receipt.json"))
        self.assertIn("CEO 강등", out + err)

    def test_demotion_without_edits_removes_identical_backup(self):
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER)
        self.assertEqual(glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo*")), [], "같은 내용의 백업은 지운다")

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
        self.assertRegex(os.path.basename(caps[0]), r"\.before-compose\.\d{8}T\d{6}Z$")
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
        kept = glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo.*"))
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
        self.assertEqual([_slurp(k, encoding="utf-8") for k in glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo.*"))],
                         [stale])


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
        return out

    def hook_first_live_write(self, ns, save):
        """살아 있는 MASTER 를 처음 건드리는 함수(지금 코드: swap_live · 종전 코드: write_atomic)에 들어가기 직전 save() 1회."""
        md = self.p("MASTER_DIRECTIVE.md")
        name = "swap_live" if "swap_live" in ns else "write_atomic"
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
        self.assertIn("place_new", ns, "교체가 '덮지 않는 자리 놓기'를 쓰지 않는다")
        orig = ns["place_new"]
        md = self.p("MASTER_DIRECTIVE.md")
        fired = []

        def wrapped(src, dst):
            if os.path.abspath(dst) == os.path.abspath(md) and not fired:
                fired.append(1)
                _write(md, USER + self.EDIT)
            return orig(src, dst)
        ns["place_new"] = wrapped
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
                disp = ns["swap_live"](md, nxt, cur)
                os.unlink(disp)
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
        orig = ns["swap_live"]
        n = []

        def always_changing(path, *a, **kw):
            n.append(1)
            self.save_in_place(USER + "CHANGE %d\n" % len(n))
            return orig(path, *a, **kw)
        ns["swap_live"] = always_changing
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
                names = {"swap": "swap_live", "legacy": "drop_legacy", "receipt": "write_receipt"}
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
                kept = glob.glob(self.p("MASTER_DIRECTIVE.md.pre-ceo.*"))
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
