#!/usr/bin/env python3
"""H-SEAL-RT-1 — 동봉 런타임 봉인 매니페스트 회귀 핀 (부트 v2 티켓 C1 · 명세 §6).

집의 규율: "PASS 가 나온다"는 증거가 아니다. 각 축은 **결함을 주입해 판정이 실제로 뒤집히는지**로
증명한다(뮤테이션). 아래 음성 대조가 없으면 이 스위트는 게이트가 아니라 껍데기다.

봉인하는 것
  ① 무결 트리 → 일치(rc 0)
  ② 파일 추가 → added (실사고 계급: npm i -g 가 번들 안으로 · 2026-09-04 오너 머신 실측)
  ③ 파일 삭제 → missing
  ④ 내용 변경 → changed
  ⑤ ★심링크 → 실복사본 = changed. `find -type f` 기반 매니페스트가 **구조적으로 눈이 머는**
     실패 모드다. node/bin/{npm,npx,corepack} 이 이렇게 되면 번들 npm 호출 전체가
     MODULE_NOT_FOUND 로 깨진다(scripts/restore-runtime-symlinks.sh:11-13).
  ⑥ 실복사본 → 심링크 = changed (역방향도 같은 계급)
  ⑦ 실행 비트 상실 → changed (app_bundle.rs 의 ExecutableNotExecutable 결손 계급)
  ⑧ 심링크를 따라가지 않는다 — 트리 밖 대상이 해시 대상으로 끌려오지 않는다
  ⑨ 결정론 — 같은 트리는 같은 바이트(뮤테이션 대조가 이것에 의존한다)
  ⑩ 판정 불가는 통과가 아니다 — 트리 부재·매니페스트 부재·형식 오류는 전부 rc 2
  ⑪ 매니페스트가 트리 안에 놓여도 자기 자신을 봉인 대상으로 세지 않는다
  ⑫ 판독 거부(권한·잠금)는 불일치가 아니라 판정 불가다 — **트리 파일과 매니페스트 자신 둘 다**.
     Windows 에서도 실제로 주입한다(msvcrt 바이트 잠금) — 그 오류군을 skip 으로 비우면
     "안 재고 초록"이 된다(R1 codex #7 · R2 codex #5).
"""

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

BIN = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if BIN not in sys.path:
    sys.path.insert(0, BIN)
import javis_runtime_seal as rs  # noqa: E402

TOOL = os.path.join(BIN, "javis_runtime_seal.py")


def _write(path, data=b"x", mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    if mode is not None:
        os.chmod(path, mode)


class RuntimeSealTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rtseal-")
        self.root = os.path.join(self.tmp, "runtime")
        # 배송 트리를 축소 모사한다 — 일반 파일·실행 파일·심링크·중첩 디렉토리.
        _write(os.path.join(self.root, "python", "bin", "python3"), b"ELF-ish", 0o755)
        _write(os.path.join(self.root, "python", "lib", "mod.py"), b"print(1)\n", 0o644)
        _write(os.path.join(self.root, "git", "libexec", "git-core", "git"), b"GITBIN", 0o755)
        _write(os.path.join(self.root, "node", "lib", "node_modules", "npm", "bin",
                            "npm-cli.js"), b"#!/usr/bin/env node\n", 0o644)
        # node/bin/npm → ../lib/node_modules/npm/bin/npm-cli.js (배송본과 동형 심링크)
        os.makedirs(os.path.join(self.root, "node", "bin"), exist_ok=True)
        os.symlink("../lib/node_modules/npm/bin/npm-cli.js",
                   os.path.join(self.root, "node", "bin", "npm"))
        # git-core 빌트인 dedup 링크(같은 디렉토리 'git' 을 가리키는 형태)
        os.symlink("git", os.path.join(self.root, "git", "libexec", "git-core", "git-add"))
        self.man = os.path.join(self.tmp, "runtime-manifest.json")
        self.assertEqual(0, self._emit())

    def tearDown(self):
        # ★권한을 되돌린 뒤에 지운다. addCleanup 은 tearDown **뒤**에 돌아 이미 삭제된 경로를
        #   만지고, rmtree 는 0o000 디렉터리를 지우지 못해(ignore_errors 로 조용히) 임시폴더를
        #   남긴다 — 둘 다 여기서 막는다.
        for p in getattr(self, "_chmodded", []):
            try:
                os.chmod(p, 0o755)
            except OSError:
                pass
        # Windows 잠금 해제 — 열린 핸들이 남으면 임시폴더 삭제가 거부된다.
        for f, size in getattr(self, "_locked", []):
            try:
                import msvcrt
                f.seek(0)
                msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, size)
            except OSError:
                pass
            try:
                f.close()
            except OSError:
                pass
        shutil.rmtree(self.tmp, ignore_errors=True)

    # ── 하네스 ──────────────────────────────────────────────────────────
    def _emit(self, root=None, out=None):
        return subprocess.call([sys.executable, TOOL, "emit",
                                "--root", root or self.root,
                                "--out", out or self.man,
                                "--app-version", "0.0.0", "--source", "test"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def _verify(self, root=None, man=None):
        """(rc, 3분류 dict) 를 돌려준다."""
        p = subprocess.run([sys.executable, TOOL, "verify",
                            "--root", root or self.root,
                            "--manifest", man or self.man, "--json", "--max-list", "50"],
                           capture_output=True, text=True)
        out = {}
        if p.stdout.strip():
            try:
                out = json.loads(p.stdout)
            except json.JSONDecodeError:
                out = {}
        return p.returncode, out

    # ── ① 무결 ─────────────────────────────────────────────────────────
    def test_clean_tree_matches(self):
        rc, d = self._verify()
        self.assertEqual(0, rc, "무결 트리인데 불일치로 판정됐다: %r" % d)
        self.assertTrue(d.get("ok"))
        self.assertEqual({"added": 0, "changed": 0, "missing": 0}, d["counts"])

    # ── ② 추가(실사고 계급) ─────────────────────────────────────────────
    def test_added_file_is_detected(self):
        _write(os.path.join(self.root, "node", "lib", "node_modules",
                            "@openai", "codex", "package.json"), b"{}\n")
        rc, d = self._verify()
        self.assertEqual(1, rc)
        self.assertEqual(1, d["counts"]["added"])
        self.assertIn("node/lib/node_modules/@openai/codex/package.json", d["added"])
        self.assertEqual(0, d["counts"]["missing"] + d["counts"]["changed"])

    # ── ③ 삭제 ─────────────────────────────────────────────────────────
    def test_missing_file_is_detected(self):
        os.remove(os.path.join(self.root, "python", "lib", "mod.py"))
        rc, d = self._verify()
        self.assertEqual(1, rc)
        self.assertEqual(["python/lib/mod.py"], d["missing"])

    # ── ④ 내용 변경 ────────────────────────────────────────────────────
    def test_content_change_is_detected(self):
        _write(os.path.join(self.root, "python", "lib", "mod.py"), b"print(2)\n", 0o644)
        rc, d = self._verify()
        self.assertEqual(1, rc)
        self.assertEqual(["python/lib/mod.py"], d["changed"])

    # ── ⑤ ★심링크 → 실복사본 (find -type f 가 눈머는 자리) ──────────────
    def test_symlink_replaced_by_copy_is_detected(self):
        link = os.path.join(self.root, "node", "bin", "npm")
        target = os.path.join(self.root, "node", "lib", "node_modules", "npm",
                              "bin", "npm-cli.js")
        self.assertTrue(os.path.islink(link))
        os.remove(link)
        shutil.copyfile(target, link)          # 링크가 실복사본으로 뒤바뀐 상태
        self.assertFalse(os.path.islink(link))
        rc, d = self._verify()
        self.assertEqual(1, rc, "심링크가 실복사본으로 바뀌었는데 무결로 통과했다 — 봉인 무의미")
        self.assertEqual(["node/bin/npm"], d["changed"])

    # ── ⑥ 실복사본 → 심링크 (역방향) ────────────────────────────────────
    def test_file_replaced_by_symlink_is_detected(self):
        p = os.path.join(self.root, "python", "lib", "mod.py")
        os.remove(p)
        os.symlink("../bin/python3", p)
        rc, d = self._verify()
        self.assertEqual(1, rc)
        self.assertEqual(["python/lib/mod.py"], d["changed"])

    # ── ⑦ 실행 비트 상실 ───────────────────────────────────────────────
    def test_exec_bit_loss_is_detected(self):
        p = os.path.join(self.root, "python", "bin", "python3")
        os.chmod(p, 0o644)
        rc, d = self._verify()
        self.assertEqual(1, rc, "실행 비트가 사라졌는데 통과했다 — 실행 불가 번들이 초록")
        self.assertEqual(["python/bin/python3"], d["changed"])

    # ── ⑧ 심링크를 따라가지 않는다 ──────────────────────────────────────
    def test_symlinks_are_not_followed_outside_tree(self):
        outside = os.path.join(self.tmp, "outside")
        _write(os.path.join(outside, "secret.bin"), b"A" * 4096)
        os.symlink(outside, os.path.join(self.root, "escape"))
        m = rs.build_manifest(self.root)
        rels = set(m["entries"])
        self.assertIn("escape", rels)
        self.assertEqual("l", m["entries"]["escape"]["t"], "디렉토리 심링크가 링크로 안 잡혔다")
        self.assertFalse([r for r in rels if r.startswith("escape/")],
                         "심링크를 따라 트리 밖 파일이 봉인 대상으로 끌려왔다")

    # ── ⑨ 결정론 ───────────────────────────────────────────────────────
    def test_emit_is_deterministic(self):
        a = os.path.join(self.tmp, "a.json")
        b = os.path.join(self.tmp, "b.json")
        self.assertEqual(0, self._emit(out=a))
        self.assertEqual(0, self._emit(out=b))
        with open(a, "rb") as f1, open(b, "rb") as f2:
            self.assertEqual(f1.read(), f2.read(), "같은 트리에서 매니페스트 바이트가 달라졌다")

    # ── ⑩ 판정 불가는 통과가 아니다 ─────────────────────────────────────
    def test_undecidable_is_not_a_pass(self):
        rc, _ = self._verify(root=os.path.join(self.tmp, "nope"))
        self.assertEqual(2, rc, "대상 트리 부재가 통과(0)로 접혔다")
        rc, _ = self._verify(man=os.path.join(self.tmp, "nope.json"))
        self.assertEqual(2, rc, "매니페스트 부재가 통과로 접혔다")
        bad = os.path.join(self.tmp, "bad.json")
        with open(bad, "w", encoding="utf-8") as f:
            f.write("{not json")
        rc, _ = self._verify(man=bad)
        self.assertEqual(2, rc, "형식 오류 매니페스트가 통과로 접혔다")
        wrong = os.path.join(self.tmp, "wrong.json")
        with open(wrong, "w", encoding="utf-8") as f:
            json.dump({"schema": 99, "entries": {}}, f)
        rc, _ = self._verify(man=wrong)
        self.assertEqual(2, rc, "미지 schema 가 통과로 접혔다")

    # ── ⑫ 판독 실패(권한·공유위반)는 불일치가 아니라 판정 불가다 (R1 codex #7) ──
    #   Windows 의 sharing violation(WinError 32)과 POSIX 의 EACCES 는 같은 계급이다 —
    #   "봉인이 깨졌다"가 아니라 "재지 못했다". 둘을 같은 exit 로 내보내면 소비자가 구분할 수
    #   없고, 특히 rc 1(불일치)로 새면 **없는 파손을 보고**하게 된다.
    #   ★R2 codex #5: Windows 에서 이 계급을 통째로 skip 하면 "그 오류군을 안 재고 초록"이 된다.
    #   그래서 파일 판독 거부는 **양 플랫폼에서 실제로 주입**한다 — POSIX 는 chmod 000,
    #   Windows 는 stdlib `msvcrt.locking` 으로 바이트 영역을 잠근다(다른 프로세스가 읽으면
    #   WinError 33 → PermissionError). 판독기는 subprocess 라 같은 프로세스 면제를 받지 않는다.
    def _deny_file_read(self, path):
        """파일 판독을 실제로 거부시킨다(양 플랫폼). 주입 불가 환경만 명시 skip."""
        if os.name == "nt":
            import msvcrt
            size = max(1, os.path.getsize(path))
            f = open(path, "r+b")
            try:
                msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, size)
            except OSError as e:
                f.close()
                self.skipTest("Windows 잠금 주입 실패(%s) — 무측정을 명시 skip 한다" % e)
            self._locked = getattr(self, "_locked", [])
            self._locked.append((f, size))
            return
        if os.geteuid() == 0:
            self.skipTest("root 는 권한 거부를 주입할 수 없다 — 무측정을 명시 skip 한다")
        self._chmodded = getattr(self, "_chmodded", [])
        self._chmodded.append(path)
        os.chmod(path, 0o000)
        if os.access(path, os.R_OK):
            self.skipTest("이 파일계가 권한을 강제하지 않는다 — 주입 실패(명시 skip)")

    def _drop_read_permission(self, path):
        """디렉터리 판독 거부 — POSIX 형상 전용 주입이다(Windows 에는 대응 원시가 없다).
        ★파일 계급은 `_deny_file_read` 가 양 플랫폼에서 재므로, 이 skip 이 오류군 전체를
        가리지 않는다(R2 codex #5 의 요구는 그 구분이다)."""
        if os.name != "posix" or os.geteuid() == 0:
            self.skipTest("디렉터리 권한 거부를 주입할 수 없는 환경(root·비 POSIX) — "
                          "파일 계급은 _deny_file_read 가 이 플랫폼에서도 잰다")
        self._chmodded = getattr(self, "_chmodded", [])
        self._chmodded.append(path)
        os.chmod(path, 0o000)
        if os.access(path, os.R_OK):
            self.skipTest("이 파일계가 권한을 강제하지 않는다 — 주입 실패(명시 skip)")

    def test_unreadable_file_is_undecidable_not_mismatch(self):
        target = os.path.join(self.root, "python", "lib", "mod.py")
        self._deny_file_read(target)
        rc, d = self._verify()
        self.assertEqual(2, rc, "판독 실패가 rc %d 로 샜다(1이면 '없는 파손'을 보고한 것) — %r" % (rc, d))
        self.assertTrue(d.get("undecidable"), "JSON 에 판정 불가 표기가 없다: %r" % d)
        self.assertFalse(d.get("ok"))

    # ── ⑬ **매니페스트 자신**의 판독 거부도 같은 계급이다 (R2 codex #5) ──
    #   R1 #7 은 트리 대조(classify)만 OSError 로 닫았고 매니페스트 open 은 열려 있었다.
    #   PermissionError·Windows 공유위반이 uncaught 로 새면 파이썬 기본 종료코드 1 = 이 계약의
    #   '불일치'가 되어 **없는 파손을 보고**하고, `--json` 은 객체를 한 줄도 내지 않았다.
    def test_unreadable_manifest_is_undecidable_not_mismatch(self):
        self._deny_file_read(self.man)
        rc, d = self._verify()
        self.assertEqual(
            2, rc,
            "매니페스트 판독 실패가 rc %d 로 샜다(1이면 '없는 파손'을 보고한 것) — %r" % (rc, d))
        self.assertTrue(d.get("undecidable"),
                        "판정 불가인데 --json 이 객체를 내지 않았다(소비자가 읽을 것이 없다): %r" % d)
        self.assertFalse(d.get("ok"))

    def test_unreadable_directory_is_undecidable_not_missing(self):
        """★os.walk 기본값이 디렉터리 오류를 삼키면 그 아래가 전부 '누락' 으로 둔갑한다."""
        target = os.path.join(self.root, "git", "libexec", "git-core")
        self._drop_read_permission(target)
        rc, d = self._verify()
        self.assertEqual(2, rc,
                         "권한 없는 디렉터리가 '누락' 으로 보고됐다(rc %d · %r) — 판정 불가여야 한다"
                         % (rc, d))
        self.assertTrue(d.get("undecidable"))

    def test_emit_read_failure_is_undecidable(self):
        target = os.path.join(self.root, "python", "bin", "python3")
        self._drop_read_permission(target)
        out = os.path.join(self.tmp, "emit-fail.json")
        rc = self._emit(out=out)
        self.assertEqual(2, rc, "emit 판독 실패가 rc %d — 판정 불가(2)여야 한다" % rc)

    # ── ⑪ 매니페스트 자기 제외 ─────────────────────────────────────────
    def test_manifest_inside_root_is_self_excluded(self):
        inside = os.path.join(self.root, rs.MANIFEST_BASENAME)
        self.assertEqual(0, self._emit(out=inside))
        rc, d = self._verify(man=inside)
        self.assertEqual(0, rc, "트리 안 매니페스트가 자기 자신을 추가 항목으로 셌다: %r" % d)

    # ── digest 는 포매팅과 독립이다 ─────────────────────────────────────
    def test_digest_is_independent_of_json_formatting(self):
        m = rs.build_manifest(self.root)
        reparsed = json.loads(json.dumps(m, indent=4))   # 포매팅만 바꾼 사본
        self.assertEqual(m["digest"], rs.entries_digest(reparsed["entries"]))

    def test_counts_match_entries(self):
        m = rs.build_manifest(self.root)
        c = m["counts"]
        self.assertEqual(len(m["entries"]), c["files"] + c["symlinks"] + c["other"])
        self.assertEqual(0, c["other"], "모사 트리에 비정규 엔트리가 생겼다")


if __name__ == "__main__":
    unittest.main(verbosity=2)
