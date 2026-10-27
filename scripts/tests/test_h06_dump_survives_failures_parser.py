"""T-H06-B (0.14.55-P8) — `DeadmanTickTrace` 의 덤프가 `failures.py` 의 패닉-메시지 수집을 끊지 않는다.

왜 존재하는가: `win-cysd-failures.json` 은 Windows 레인에서 **패닉 메시지**만 모은다. 0.14.55 이전의
덤프는 `println!` 로만 나갔으므로 그 JSON 채널에서는 보이지 않았고, 그래서 `deadman_*` 가족이 Windows
에서 붉을 때 판별 근거(틱 표)가 **원문 로그를 열어야만** 보였다. 덤프를 패닉 메시지 안으로 옮기면
보이게 되지만, 파서가 메시지를 **종결자 줄**에서 끊으므로 덤프 한 줄이 종결자처럼 생기면 그 뒤가
사라진다 — 즉 고치려던 채널을 조용히 반만 고치게 된다.

세 가지를 **실제 파서**로 잰다(설계 T-H06-B 항목 3, R3 §3-4 의 교정판):
  ⑴ 원시 음성: 종결자로 시작하는 줄은 **잘려야 한다** — 그것이 파서의 계약이고 결함이 아니다.
     (이 단언이 없으면 ⑶의 통과가 「파서를 약화시켜 얻은 통과」와 구별되지 않는다.)
  ⑵ 생산자의 인코딩이 실재한다: `governance.rs` 가 접두와 두 토큰 무해화를 실제로 들고 있다.
  ⑶ 인코딩된 덤프는 **온전히** 도착한다.

PyYAML 없이 돈다(CI 의 파이썬에 없다) — `failures.py` 본문을 들여쓰기로 떼어 그대로 실행한다.
돌연변이 검증용 노브: `H06_GOVERNANCE_RS_UNDER_TEST`, `H06_WINDOWS_HEALTH_YML_UNDER_TEST`.

    python3 scripts/tests/test_h06_dump_survives_failures_parser.py
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
WORKFLOW = os.environ.get("H06_WINDOWS_HEALTH_YML_UNDER_TEST") or os.path.join(
    ROOT, ".github", "workflows", "windows-health.yml")
GOVERNANCE = os.environ.get("H06_GOVERNANCE_RS_UNDER_TEST") or os.path.join(
    ROOT, "src", "bin", "cysd", "governance.rs")
HEREDOC_START = 'cat > "$CYSD_DIR/failures.py" <<\'PY\''

# 파서가 패닉 메시지를 끝내는 줄의 형태 — `failures.py` 의 `ends` 식과 1:1.
TERMINATORS = [
    "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
    "stack backtrace:",
    "test result: FAILED. 0 passed; 1 failed",
    "failures:",
    "FAILED",
    "ok",
    "---- governance::tests::x stdout ----",
    "test governance::tests::y ... ok",
    "thread 'other' (7) panicked at src/bin/cysd/x.rs:1:1:",
]


def extract_failures_py(path):
    """워크플로의 `failures.py` 히어독 본문을 **들여쓰기 의미 그대로** 떼어 낸다."""
    with open(path, encoding="utf-8") as fh:
        lines = fh.read().split("\n")
    start = None
    for i, ln in enumerate(lines):
        if ln.strip() == HEREDOC_START:
            start = i
            indent = len(ln) - len(ln.lstrip())
            break
    if start is None:
        raise AssertionError("failures.py 히어독 시작줄을 찾지 못했다: %s" % HEREDOC_START)
    body = []
    for ln in lines[start + 1:]:
        if ln.strip() == "PY":
            return "\n".join(body) + "\n"
        body.append(ln[indent:] if ln.startswith(" " * indent) else ln.lstrip())
    raise AssertionError("히어독의 끝(PY)을 찾지 못했다")


def run_parser(py_src, log_text):
    """떼어 낸 파서를 그대로 돌려 win-cysd-failures.json 을 받는다."""
    with tempfile.TemporaryDirectory() as d:
        pyf = os.path.join(d, "failures.py")
        logf = os.path.join(d, "cargo.log")
        outf = os.path.join(d, "win-cysd-failures.json")
        with open(pyf, "w", encoding="utf-8") as fh:
            fh.write(py_src)
        with open(logf, "w", encoding="utf-8") as fh:
            fh.write(log_text)
        r = subprocess.run([sys.executable, "-I", "-B", pyf, logf, "101", outf],
                           capture_output=True, text=True, cwd=d)
        if r.returncode != 0:
            raise AssertionError("파서가 rc=%d 로 끝났다: %s" % (r.returncode, r.stderr[:400]))
        with open(outf, encoding="utf-8") as fh:
            return json.load(fh)


def log_with_message(message_lines):
    """패닉 하나를 담은 최소 cargo 로그 — 헤더 뒤에 주어진 줄들이 메시지 자리에 온다."""
    head = [
        "running 1 test",
        "thread 'governance::tests::deadman_idle_debounce_knob_separate' (123) "
        "panicked at src/bin/cysd/governance.rs:26108:9:",
    ]
    tail = [
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        "test governance::tests::deadman_idle_debounce_knob_separate ... FAILED",
        "",
        "failures:",
        "    governance::tests::deadman_idle_debounce_knob_separate",
        "",
        "test result: FAILED. 0 passed; 1 failed; 0 ignored",
        "",
    ]
    return "\n".join(head + message_lines + tail)


def governance_src():
    with open(GOVERNANCE, encoding="utf-8") as fh:
        return fh.read()


class H06DumpSurvivesParser(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.parser = extract_failures_py(WORKFLOW)

    # ─ ⑴ 원시 음성 — 파서의 계약이다. 종결자로 시작하면 **잘린다**. ─────────────
    def test_raw_rows_that_begin_with_a_terminator_are_truncated(self):
        for t in TERMINATORS:
            with self.subTest(terminator=t):
                got = run_parser(self.parser, log_with_message([t, "틱 2 이 줄은 사라져야 한다"]))
                self.assertGreaterEqual(len(got["panics"]), 1, got)
                # 패닉 **헤더** 종결자는 끊는 데 그치지 않고 새 패닉을 연다(그래서 2건이 된다).
                # 어느 쪽이든 우리 패닉의 메시지에서 그 뒤 줄이 사라지는 것이 계약이다.
                ours = got["panics"][0]
                self.assertEqual(ours["thread"],
                                 "governance::tests::deadman_idle_debounce_knob_separate", got)
                self.assertNotIn(
                    "틱 2 이 줄은 사라져야 한다", ours["message"],
                    "종결자 %r 뒤의 줄이 살아남았다 — 파서가 약해졌거나 검체가 틀렸다" % t)

    # ─ ⑵ 생산자가 실제로 인코딩한다 ────────────────────────────────────────────
    def test_the_emitter_encodes_every_dump_line(self):
        src = governance_src()
        # ⛔assertIn 에 소스 전문을 넘기지 않는다 — 붉을 때 메가바이트가 쏟아져 사유가 묻힌다.
        required = [
            ('const DUMP_LINE_PREFIX: &\'static str = "  ";', "덤프 접두 상수가 없다 — 인코딩이 사라졌다"),
            ("fn dump_line(body: &str) -> String {", "dump_line 이 없다"),
            ('.replace(\'\\\'\', "ʼ")', "작은따옴표 무해화가 없다"),
            ('.replace("panicked at ", "panickedʼat ")',
             "`panicked at ` 무해화가 없다 — 파서의 헤더 종결자는 줄 머리에 고정돼 있지 않다"),
        ]
        missing = [why for token, why in required if token not in src]
        # ⛔`"#[track_caller]" in src` 로 세면 **이 저장소의 주석 본문**에도 그 글자가 있어서 늘
        #   통과한다(변이 M2 가 그렇게 살아남았다). 속성이 `fn expect(` **바로 앞 줄**에 붙어
        #   있는지를 본다 — 이것이 실제로 효력을 내는 유일한 자리다.
        attached = False
        lines = src.split("\n")
        for n, ln in enumerate(lines):
            if ln.strip().startswith("fn expect(&self, got: usize"):
                for prev in reversed(lines[max(0, n - 4):n]):
                    if prev.strip().startswith("///") or prev.strip() == "":
                        continue
                    attached = prev.strip() == "#[track_caller]"
                    break
                break
        if not attached:
            missing.append("#[track_caller] 가 `fn expect` 에 붙어 있지 않다 — 패닉 위치가"
                           " 호출 지점(=시험)을 가리키지 못한다")
        self.assertEqual(missing, [], "생산자의 인코딩이 빠졌다(%s): %s" % (GOVERNANCE, "; ".join(missing)))

    # ─ ⑶ 인코딩된 덤프는 온전히 도착한다 ───────────────────────────────────────
    def test_encoded_dump_arrives_whole(self):
        encoded = ["  " + t.replace("'", "ʼ").replace("panicked at ", "panickedʼat ")
                   for t in TERMINATORS]
        rows = ["idles.len() 이 기대와 다르다",
                "  ★D-R2-3 덤프(role=master · 틱 3개) — 판별: …"] + encoded + [
                "  틱 3 verdict=Idle now=1.0 last=0.0 now-last=+1.0 shell_alive=true exited=false"]
        got = run_parser(self.parser, log_with_message(rows))
        self.assertEqual(len(got["panics"]), 1, got)
        msg = got["panics"][0]["message"]
        for r in rows:
            self.assertIn(r, msg, "덤프 줄이 잘렸다: %r" % r)
        self.assertEqual(got["panics"][0]["location"],
                         "src/bin/cysd/governance.rs:26108:9")

    # ─ 메타: 떼어 낸 파서가 진짜 그 파서다 ─────────────────────────────────────
    def test_extracted_parser_is_the_workflow_one(self):
        self.assertIn("PANIC = re.compile", self.parser)
        self.assertIn('SEEN = re.compile', self.parser)
        self.assertIn('"stack backtrace:"', self.parser)


if __name__ == "__main__":
    unittest.main(verbosity=2)
