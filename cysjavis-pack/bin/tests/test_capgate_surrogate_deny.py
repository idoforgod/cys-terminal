#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_capgate_surrogate_deny.py — 0.14.44 A5: 거부 출력이 인코딩 예외로 깨지면 명령이 통과하는 틈.

짝 없는 대리 문자(\\ud800)가 든 명령에서 role-capability-gate.sh 는 거부 JSON(permissionDecision=deny)을 내고 exit 0 이어야 한다
(고치기 전: 표준출력 0바이트 + exit 1 = 비차단). 나머지 입력(한글·따옴표·역슬래시·줄바꿈·제어문자·이모지)의 출력은 종전과 같은 JSON.
실행: python3 test_capgate_surrogate_deny.py
"""
import json
import os
import subprocess
import tempfile
import unittest

HOOK = os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))), "hooks", "role-capability-gate.sh")


def run(raw, enc="utf-8"):
    home = tempfile.mkdtemp(prefix="a5-")
    env = {"PATH": "/usr/bin:/bin", "HOME": home, "TMPDIR": home, "CYS_ROLE": "cso", "CYS_NO_AUTOSTART": "1",
           "LANG": "en_US.UTF-8", "PYTHONIOENCODING": enc}
    return subprocess.run(["sh", HOOK], input=raw, capture_output=True, env=env, timeout=30, cwd=home)


class SurrogateDeny(unittest.TestCase):
    def test_lone_surrogate_still_denies_with_valid_json(self):
        r = run(b'{"tool_name":"Bash","session_id":"s","tool_input":{"command":"cys \\"a\\ud800b\\" x"}}')
        self.assertEqual(r.returncode, 0, r.stderr[-300:])
        j = json.loads(r.stdout)
        self.assertEqual(j["hookSpecificOutput"]["permissionDecision"], "deny")
        self.assertEqual(j["hookSpecificOutput"]["hookEventName"], "PreToolUse")
        self.assertEqual(r.stdout.count(b"\n"), 1)
        r.stdout.decode("ascii")      # 사유는 ASCII 로 바뀌어 나간다

    def test_fallback_output_escapes_each_char_once(self):
        # 리뷰 m1: 비 UTF-8 출력(latin-1·ascii)의 대체 출력에서 사유 글자가 `\\ub2a5` 로 겹쳐 나가면 좌석이 읽지 못한다.
        # 한 번만 이스케이프(`\ub2a5`)되어 json.loads 가 원래 글자를 돌려줘야 한다. 판정(deny)과 JSON 꼴은 그대로.
        for enc in ("latin-1", "ascii"):
            raw = json.dumps({"tool_name": "Bash", "session_id": "s", "tool_input": {"command": "cys close-surface surface:3"}}).encode()
            r = run(raw, enc)
            self.assertEqual(r.returncode, 0, (enc, r.stderr[-300:]))
            out = r.stdout.decode("ascii")
            self.assertNotIn("\\\\u", out, "역슬래시가 겹쳤다(" + enc + ")")
            j = json.loads(out)
            self.assertEqual(j["hookSpecificOutput"]["permissionDecision"], "deny")
            reason = j["hookSpecificOutput"]["permissionDecisionReason"]
            self.assertNotIn("\\u", reason, "복원한 사유에 글자 그대로의 \\u 가 남았다(" + enc + ")")
            self.assertTrue(any(ord(c) > 0x7F for c in reason), "한글 사유가 비ASCII 글자로 복원돼야 한다")
        # BMP 밖 글자는 서로게이트 쌍으로 — json.loads 가 원래 글자로
        raw = json.dumps({"tool_name": "Bash", "session_id": "s", "tool_input": {"command": "cys 😀verb x"}}).encode()
        r = run(raw, "ascii")
        j = json.loads(r.stdout.decode("ascii"))
        self.assertIn("😀", j["hookSpecificOutput"]["permissionDecisionReason"])

    def test_ordinary_inputs_keep_native_json(self):
        for cmd in ('cys 한글동사 인자', 'cys 😀verb x', 'cys close-surface surface:3'):
            raw = json.dumps({"tool_name": "Bash", "session_id": "s", "tool_input": {"command": cmd}}).encode()
            r = run(raw)
            self.assertEqual(r.returncode, 0)
            j = json.loads(r.stdout)
            self.assertEqual(j["hookSpecificOutput"]["permissionDecision"], "deny")
            if "한글" in cmd or "😀" in cmd:
                self.assertIn(cmd.split()[1], j["hookSpecificOutput"]["permissionDecisionReason"], "비ASCII 는 종전처럼 그대로")


if __name__ == "__main__":
    unittest.main(verbosity=2)
