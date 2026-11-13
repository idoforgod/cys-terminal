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


def sha(text_or_bytes):
    b = text_or_bytes.encode("utf-8") if isinstance(text_or_bytes, str) else text_or_bytes
    return hashlib.sha256(b).hexdigest()


def _write(path, data, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data.encode("utf-8") if isinstance(data, str) else data)
    if mode is not None:
        os.chmod(path, mode)


def extract_shell_function(src, name):
    """cys-dept 원문에서 함수 정의 한 개를 그대로 뽑는다(사본이 아니라 실제 코드로 단위 시험 — 변이가 그대로 보인다)."""
    m = re.search(r"^%s\(\)\{.*?^\}\n" % re.escape(name), src, re.S | re.M)
    if m is None:
        raise AssertionError("cys-dept 에 %s() 가 없다" % name)
    return m.group(0)


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
            return json.load(open(self.p(".ceo-receipt.json"), encoding="utf-8"))
        except (OSError, ValueError):
            return None

    def calls(self):
        return open(self.log, encoding="utf-8").read() if os.path.exists(self.log) else ""

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
        before = open(self.p("MASTER_DIRECTIVE.md"), "rb").read()
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(open(self.p("MASTER_DIRECTIVE.md"), "rb").read(), before)

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
        self.assertEqual(open(kept[0], encoding="utf-8").read(), USER, "백업이 원본으로 쓰이지 않았는데 내용이 바뀌었다")
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
        self.assertEqual(open(caps[0], encoding="utf-8").read(), live, "보관 사본이 조립 전 바이트가 아니다")
        self.assert_receipt_for(HEADER_V2, USER + "USERFIX-E6-2\n", self.read("MASTER_DIRECTIVE.md"))

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
        self.assertIn("머리말 다시 조립", out + err)
        self.assertEqual(self.calls().count("cys status --json") - n_before, 1, "재조립 뒤 재주입 시도 1회")

    def test_post_verify_requires_valid_receipt(self):
        """E-6-09: 영수증이 깨졌으면 .pre-ceo 가 있어도 성공으로 보고하지 않는다(exit 5) · 대조: 영수증이 없으면 다시 기록."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertTrue(os.path.exists(self.p("MASTER_DIRECTIVE.md.pre-ceo")))
        md_before = self.read("MASTER_DIRECTIVE.md")
        for bad in ("{not json", json.dumps({"schema": 1, "header_sha256": "x"}),
                    json.dumps(dict(self.receipt(), separator="\n---\n"))):
            with self.subTest(bad=bad[:30]):
                _write(self.p(".ceo-receipt.json"), bad)
                rc, out, err = self.run_dept("promote-ceo")
                self.assertEqual(rc, 5, "깨진 영수증인데 승격 성공으로 보고했다\n" + out + err)
                self.assertIn("영수증", err)
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), md_before, "보류인데 지침이 바뀌었다")
                self.assertEqual(self.read(".ceo-receipt.json"), bad, "깨진 영수증을 조용히 덮어썼다")
        os.remove(self.p(".ceo-receipt.json"))
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), md_before)
        self.assert_receipt_for(HEADER, USER, md_before)

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
        src = open(DEPT, encoding="utf-8").read()
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
        self.assertEqual(self.leftovers(), [])

    def test_concurrent_promotion_lock_is_taken(self):
        """H-14-N2-01: 동시 승격 락(D-5) — flock 이 있으면 잡힌 락 뒤에서 기다리고, 없으면(mac·PortableGit) mkdir 락을
        잡으며, 남이 쥔 락은 경고 후 무락 진행(종전 계약)한다. 락을 지우는 변이는 이 시험이 잡는다."""
        self.seed()
        md = self.p("MASTER_DIRECTIVE.md")
        path_dirs = [self.bindir, "/opt/homebrew/bin", "/usr/local/bin"] + self.env["PATH"].split(os.pathsep)
        if shutil.which("flock", path=os.pathsep.join(path_dirs)):
            import fcntl
            fd = os.open(md + ".promote.lock", os.O_RDWR | os.O_CREAT, 0o644)
            fcntl.flock(fd, fcntl.LOCK_EX)
            proc = subprocess.Popen(["bash", DEPT, "promote-ceo"], env=self.env,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                time.sleep(2.0)
                self.assertIsNone(proc.poll(), "락이 잡혀 있는데 승격이 끝났다(락 없음)")
                self.assertEqual(self.read("MASTER_DIRECTIVE.md"), USER, "락 밖에서 지침이 바뀌었다")
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
                os.close(fd)
            self.assertEqual(proc.wait(timeout=120), 0)
        else:
            os.mkdir(md + ".promote.lock.d")       # 남이 쥔 락
            rc, out, err = self.run_dept("promote-ceo")
            self.assertEqual(rc, 0, out + err)
            self.assertIn("승격 락(", err, "mkdir 락을 시도하지 않았다(락 제거 변이)")
            self.assertTrue(os.path.isdir(md + ".promote.lock.d"), "남의 락을 지웠다")
            os.rmdir(md + ".promote.lock.d")
            # 강등도 같은 락을 잡는다.
            os.mkdir(md + ".promote.lock.d")
            rc, out, err = self.run_dept("down", "d0")
            self.assertEqual(rc, 0, out + err)
            self.assertIn("승격 락(", err)
            os.rmdir(md + ".promote.lock.d")
        self.assertFalse(os.path.exists(md + ".promote.lock.d"), "제 락을 풀지 않았다")

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

    def test_receipt_hold_notice_is_deduped_and_closed(self):
        """H-14-N6-01: 영수증 손상 보류 알림 — 제목 완전일치로 1건만, 해소되면 비허가 결정으로 닫힌다(부서별 알림 없음)."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        _write(self.p(".ceo-receipt.json"), "{broken")
        self.set_pending()
        title = "CEO 승격 보류(영수증 손상)"
        self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertEqual(self.calls().count("feed push --title %s" % title), 1)
        _write(self.feedlist, "rid-r\t[pending]\tceo-notice\t%s\tdecision=-\n" % title)   # 데몬이 보관 중
        for _ in range(3):
            self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertEqual(self.calls().count("feed push --title %s" % title), 1, "보류 알림이 쌓인다(F1)")
        os.remove(self.p(".ceo-receipt.json"))
        self.assertEqual(self.run_dept("promote-if-pending")[0], 0)
        self.assertIn("cys feed reply rid-r ceo-promoted", self.calls(), "해소 뒤 보류 알림이 남았다")

    def test_backup_invariant_for_the_overwrite_guard(self):
        """A12·팩 갱신 가드 4 는 .pre-ceo 실재로 '승격 중'을 안다 — 조립 승격은 늘 백업을 남기고, 지워져도 다시 만든다."""
        self.seed()
        self.assertEqual(self.run_dept("promote-ceo")[0], 0)
        self.assertTrue(os.path.exists(self.p("MASTER_DIRECTIVE.md.pre-ceo")))
        _write(self.p("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "USERFIX-E6-2\n")
        os.remove(self.p("MASTER_DIRECTIVE.md.pre-ceo"))
        rc, out, err = self.run_dept("launch", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), USER + "USERFIX-E6-2\n", "백업이 다시 생기지 않았다")
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + USER + "USERFIX-E6-2\n")


class LegacyShapes(Rig):
    """0.14.x 에서 통째 복사로 승격된 기계 — 0.15.0 의 첫 승격 확인이 조립으로 옮긴다(잃는 바이트 없음)."""
    OLD_HEADER = "# CEO 옛 머리말 (master of master · 단일소유 강제 · exit 7)\n"
    OLD_VENDOR = master_text("OLD-STANDARD-MASTER")
    USER_OLD = OLD_VENDOR + "USERFIX-OLD 승격 전에 고친 줄\n"

    def test_unedited_whole_copy_takes_the_backup_as_body(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.seed(md=old_copy)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD,
                         "0.14.x 가 덮어쓴 사용자 본부 지침(백업)이 돌아오지 않았다")
        self.assertEqual(self.captures("before-compose"), [], "그대로인 vendor 사본은 보관할 사용자 바이트가 없다")
        self.assertIsNone(self.read(".ceo-template-applied"))
        self.assert_receipt_for(HEADER, self.USER_OLD, HEADER + SEP + self.USER_OLD)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD)

    def test_unedited_whole_copy_demotes_to_the_backup(self):
        old_copy = self.OLD_HEADER + SEP + self.OLD_VENDOR
        self.seed(md=old_copy)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(old_copy) + "\n")
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD, "0.14.x 강등 결과와 달라졌다")
        self.assertIsNone(self.read("MASTER_DIRECTIVE.md.pre-ceo"))

    def test_edited_whole_copy_keeps_the_live_body_and_captures(self):
        original = self.OLD_HEADER + SEP + self.OLD_VENDOR
        edited = original + "CEO-EDIT CEO 동안 고친 줄\n"
        self.seed(md=edited)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        _write(self.p(".ceo-template-applied"), sha(original) + "\n")
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"),
                         HEADER + SEP + self.OLD_VENDOR + "CEO-EDIT CEO 동안 고친 줄\n", "살아 있는 본문이 바뀌었다")
        caps = self.captures("before-compose")
        self.assertEqual(len(caps), 1)
        self.assertEqual(open(caps[0], encoding="utf-8").read(), edited)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD, "백업을 덮었다")

    def test_copy_without_separator_uses_backup_and_captures(self):
        six_kb = "# CEO (master of master) — 행동 규약\n단일소유 강제 · exit 7\n구분선 없는 옛 머리글 사본\n"
        self.seed(md=six_kb)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), HEADER + SEP + self.USER_OLD)
        caps = self.captures("before-compose")
        self.assertEqual([open(c, encoding="utf-8").read() for c in caps], [six_kb])
        # 같은 형상의 강등(조립 전) — 백업으로 되돌리고 지금 파일은 보관.
        shutil.rmtree(self.dirs)
        self.seed(md=six_kb)
        _write(self.p("MASTER_DIRECTIVE.md.pre-ceo"), self.USER_OLD)
        rc, out, err = self.run_dept("down", "d0")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.read("MASTER_DIRECTIVE.md"), self.USER_OLD)
        self.assertEqual([open(c, encoding="utf-8").read() for c in self.captures("before-demote")], [six_kb])


class HqDirectiveChain(Rig):
    """C11 11.11: MASTER → 생성기 → (윈도우 해시 = CI) → 상위집합 검사 → preflight 승격 판정 → 앱 — 실제 출하 파일로."""
    REAL_TMPL = os.path.join(PACK, "directives", "CEO_TEMPLATE.md")
    REAL_MASTER = os.path.join(PACK, "directives", "MASTER_DIRECTIVE.md")

    def _real(self, path):
        return open(path, encoding="utf-8", newline="").read()

    def test_unedited_master_composes_to_the_shipped_template(self):
        """조립은 줄을 더하지 않는다 — 고치지 않은 MASTER 의 조립 == 출하 CEO_TEMPLATE 바이트(CEO 줄 예산 그대로)."""
        tmpl, master = self._real(self.REAL_TMPL), self._real(self.REAL_MASTER)
        self.seed(md=master, tmpl=tmpl)
        rc, out, err = self.run_dept("promote-ceo")
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(open(self.p("MASTER_DIRECTIVE.md"), "rb").read(), open(self.REAL_TMPL, "rb").read())

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
        """O-C11-2: 구분선은 생성기의 것 하나 — cys-dept·preflight·javis_org·앱 사본이 모두 같은 바이트여야 한다."""
        gen_p = os.path.join(REPO, "scripts", "gen_ceo_template.py")
        if not os.path.isfile(gen_p):
            self.skipTest("scripts/gen_ceo_template.py 없음(배포 팩 트리 — repo 전용 축)")
        spec = importlib.util.spec_from_file_location("gen_ceo_template_t8", gen_p)
        gen = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gen)
        canon = gen.SEPARATOR
        self.assertEqual(SEP.encode("utf-8"), canon)
        dept_src = open(DEPT, encoding="utf-8").read()
        lit = re.search(r'^SEP = "(.*?)"\.encode\("utf-8"\)$', dept_src, re.M)
        self.assertIsNotNone(lit, "cys-dept 의 구분선 정의를 찾지 못했다")
        self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"),
                         canon, "cys-dept 구분선 ≠ 생성기")
        self.assertEqual(pf.Preflight._C03_CEO_SEPARATOR, canon, "preflight 구분선 ≠ 생성기")
        org_src = open(os.path.join(BIN, "javis_org.py"), encoding="utf-8").read()
        lit = re.search(r'^_CEO_SEPARATOR = "(.*?)"\.encode\("utf-8"\)$', org_src, re.M)
        self.assertIsNotNone(lit)
        self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"), canon,
                         "javis_org 구분선 ≠ 생성기")
        app = os.path.join(REPO, "src-tauri", "src", "main.rs")
        if os.path.isfile(app):
            lit = re.search(r'^const CEO_SEPARATOR: &str = "(.*?)";$', open(app, encoding="utf-8").read(), re.M)
            self.assertIsNotNone(lit)
            self.assertEqual(lit.group(1).encode("utf-8").decode("unicode_escape").encode("latin-1"), canon,
                             "앱 구분선 ≠ 생성기")
        m = re.search(r'^MARKER_PINS = (\(.*?\))$', dept_src, re.M)
        self.assertIsNotNone(m)
        self.assertEqual(tuple(ast.literal_eval(m.group(1))), tuple(pf.MARKER_PINS),
                         "cys-dept 표지 핀 ≠ preflight 정본")


if __name__ == "__main__":
    unittest.main(verbosity=2)
