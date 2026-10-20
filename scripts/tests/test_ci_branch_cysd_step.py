"""ci-branch.yml 의 맥 데몬 검체 스텝(`cargo test --bin cysd (… macOS)`)의 밀폐 검체 — 그 스텝의 `run:` 본문을 워크플로 파일에서 **그대로 떼어** 가짜 `cargo`(PATH 맨 앞)로 돌린다(0.14.43 성찰 2회차 R2F-PK 2차).

왜 존재하는가: 그 스텝은 브랜치 push 마다 데몬 단위 검체 전량을 돌리는 **차단** 스텝이다. 실패한 검체 이름을 annotation 으로 내게 하면서(종전에는 "exit code 101" 한 줄뿐이라 러너에서만 흔들리는 검체의
이름을 알 길이 없었다) 본문에 파이프(`| tee`)와 rc 직독·판독 코드가 들어갔다. 그 배선이 한 줄만 틀어져도 — `-e` 를 끄는 줄이 빠지거나 `PIPESTATUS` 색인이 바뀌거나 마지막 `exit` 가 사라지면 —
**검체가 붉은데 스텝은 초록**이 되거나 판독 코드가 도달 불가가 된다. 레인 대조 게이트는 실행 줄의 존재와 `if:` 만 핀하므로 그 회귀를 보지 못한다(이 저장소의 교리: 통과만 보이는 게이트는 늘 초록인 검사와
구별되지 않는다). 이 검체가 성공·실패·하네스 이상 갈래와 종료 코드 전달, 그리고 `continue-on-error`·`if:` 부재를 잰다.

재는 방식: 워크플로를 YAML 라이브러리 없이 들여쓰기로 읽어(표준 라이브러리만 — CI 의 파이썬에 PyYAML 이 없다) 그 스텝의 `run: |` 블록을 떼고, GitHub 가 `shell: bash` 를 돌리는 방식 그대로
`bash --noprofile --norc -e -o pipefail <파일>` 로 실행한다. PATH 는 임시 폴더의 가짜 cargo + /usr/bin + /bin 뿐이다(진짜 cargo·네트워크·저장소 빌드 무접촉). 가짜 cargo 는 조각 파일을 순서대로
stdout/stderr 에 **바이트 그대로** 내고 정해진 종료 코드로 끝난다 — 깨진 UTF-8·NUL·줄바꿈 없이 끝난 출력도 재현한다.

★이 검체가 화면에 찍는 annotation 원문에는 줄 머리에 `|` 를 붙인다 — CI 러너는 줄 머리의 `::error` 를 **진짜 명령**으로 읽는다(앞 공백만으로는 막히지 않는 것으로 안다 — 그래서 글자를 붙인다). 검체의 가짜 실패가 런의 annotation 으로 올라가면 안 된다.

    python3 scripts/tests/test_ci_branch_cysd_step.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
# 돌연변이 검증용: CI_BRANCH_YML_UNDER_TEST=<변이본 경로> — 저장소의 워크플로 대신 그 파일의 같은 스텝을 대상으로 같은 핀을 돌린다(수정 전 판·변이본에서 이 검체가 붉은지 재는 데 쓴다).
WORKFLOW = os.environ.get("CI_BRANCH_YML_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "..", ".github", "workflows", "ci-branch.yml"))
STEP_NAME = "cargo test --bin cysd (데몬 단위 검체 · 브랜치 레인 편입 · macOS)"
BASH = os.environ.get("CYSD_STEP_BASH") or shutil.which("bash") or "/bin/bash"
SHOW = os.environ.get("CYSD_STEP_SHOW", "1") != "0"   # 갈래마다 덧붙은 줄을 화면에 보인다(줄 머리 `|` — 위 ★). 끄려면 CYSD_STEP_SHOW=0
LOG_NAME = "cysd-mac.log"
CARGO_ARGV = ["test", "--bin", "cysd", "--", "--test-threads=1", "--skip", "hwmon::tests::snapshot_has_all_sections"]
T_FAIL, T_HARNESS, T_TAIL = "cysd-mac 테스트 실패", "cysd-mac 측정 실패", "cysd-mac 로그"
# 스텝 하나에 남는 error annotation 수. 실측(익명 조회 · windows-health 런 37185687205): error 를 22줄 내는 본문의 스텝에서 조회에 남은 것은 앞 10줄이었고 맨 뒤의 이름 줄은 없었다 — 그래서 판정(이름) 줄이 맨 먼저여야 한다.
ERROR_BUDGET = 10

FAKE_CARGO = r'''#!/bin/sh
# 가짜 cargo — $FAKE_CARGO_DIR 의 조각(seg-NNN.out|err)을 이름순으로 stdout/stderr 에 그대로 내고 rc 파일의 코드로 끝난다.
d="$FAKE_CARGO_DIR"
echo call >> "$d/calls"
: > "$d/argv"
for a in "$@"; do printf '%s\n' "$a" >> "$d/argv"; done
printf '%s\n' "${CYS_PACK_DIR-}" > "$d/packdir"
if [ -n "${CYS_PACK_DIR-}" ] && [ -d "$CYS_PACK_DIR" ]; then echo yes > "$d/packdir-is-dir"; else echo no > "$d/packdir-is-dir"; fi
for f in "$d"/seg-*; do
  case "$f" in
    *.out) cat "$f" ;;
    *.err) cat "$f" >&2 ;;
  esac
done
exit "$(cat "$d/rc")"
'''


class StepShapeError(AssertionError):
    """워크플로의 스텝 꼴을 읽지 못했다 — 조용히 틀린 본문을 재느니 중단한다."""


def extract_step(path, name):
    """이름이 `name` 인 스텝의 키와 `run: |` 본문 → {키: 값, "run": 본문}. 들여쓰기로만 읽는다(모르는 꼴이면 StepShapeError)."""
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    heads = [i for i, l in enumerate(lines) if l.strip() == "- name: " + name]
    if len(heads) != 1:
        raise StepShapeError("%s 에 스텝 `%s` 가 %d 개다(1 이어야 한다) — 스텝 이름을 바꿨다면 이 검체의 STEP_NAME 도 함께 고쳐라" % (path, name, len(heads)))
    indent = lambda s: len(s) - len(s.lstrip(" "))
    dash = indent(lines[heads[0]])
    key_ind = dash + 2
    keys = {"name": name}
    j = heads[0] + 1
    while j < len(lines):
        raw = lines[j]
        s = raw.strip()
        if not s or (s.startswith("#") and indent(raw) > dash):
            j += 1
            continue
        if indent(raw) <= dash:
            break                                   # 다음 스텝 · 스텝 수준 주석 · steps 블록의 끝
        m = re.match(r"^([A-Za-z0-9_-]+):(?:\s+(.*))?$", s)
        if indent(raw) != key_ind or not m:
            raise StepShapeError("%s:%d 스텝의 키로 읽히지 않는 줄이다: %r" % (path, j + 1, raw))
        k, v = m.group(1), (m.group(2) or "")
        if k in keys:
            raise StepShapeError("%s:%d 키 `%s` 가 두 번 나온다" % (path, j + 1, k))
        j += 1
        if k != "run":
            if v == "" and k in ("env", "with"):
                # B-344: one nested mapping level (`env:` / `with:`) -> {key: value}; anything deeper is a shape error.
                sub = {}
                while j < len(lines) and (not lines[j].strip() or indent(lines[j]) > key_ind):
                    s2 = lines[j].strip()
                    if s2 and not s2.startswith("#"):
                        m2 = re.match(r"^([A-Za-z0-9_-]+):\s+(.*)$", s2)
                        if indent(lines[j]) != key_ind + 2 or not m2:
                            raise StepShapeError("%s:%d `%s:` 아래 한 단계 키로 읽히지 않는 줄이다: %r" % (path, j + 1, k, lines[j]))
                        sub[m2.group(1)] = m2.group(2)
                    j += 1
                keys[k] = sub
                continue
            keys[k] = v
            continue
        if v != "|":
            raise StepShapeError("%s:%d `run:` 이 `|` 블록이 아니다(%r) — 이 검체는 그 꼴만 읽는다" % (path, j, v))
        block = []
        while j < len(lines) and not (lines[j].strip() and indent(lines[j]) <= key_ind):
            block.append(lines[j])
            j += 1
        while block and not block[-1].strip():
            block.pop()
        if not block:
            raise StepShapeError("%s: 스텝 `%s` 의 run 본문이 비어 있다" % (path, name))
        first = next(b for b in block if b.strip())
        if any(b.strip() and indent(b) < indent(first) for b in block):
            raise StepShapeError("%s: 스텝 `%s` 의 run 본문에 첫 줄보다 얕게 들여쓴 줄이 있다" % (path, name))
        keys["run"] = "\n".join(b[indent(first):] for b in block) + "\n"
    if "run" not in keys:
        raise StepShapeError("%s: 스텝 `%s` 에 `run:` 이 없다" % (path, name))
    return keys


def read_text(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def u(text):
    """문자열 → 바이트. 깨진 바이트는 surrogateescape 꼴(U+DCxx)로 적으면 그 바이트 그대로 나간다."""
    return text.encode("utf-8", "surrogateescape")


def quoted(data, limit=2500):
    """화면·실패 메시지용 — 줄마다 머리에 `|` 를 붙인다(러너가 `::error` 를 명령으로 읽지 못하게 · 위 ★)."""
    text = data.decode("utf-8", "replace") if isinstance(data, bytes) else data
    if len(text) > limit:
        text = "…(앞 %d자 생략)\n" % (len(text) - limit) + text[-limit:]
    return "\n".join("    | " + line for line in text.split("\n"))


def expected_tail(cargo_out, n=20):
    """스텝이 내야 하는 '로그' 한 줄의 메시지 — 마지막 n 줄을 `%`→`%25` · CR 제거 뒤 `%0A` 로 잇는다."""
    lines = cargo_out.split(b"\n")
    if lines and lines[-1] == b"":
        lines.pop()
    return b"%0A".join(l.replace(b"%", b"%25").replace(b"\r", b"") for l in lines[-n:])


BUILD_ERR = u("   Compiling cys-terminal v0.14.43 (/work/cys-terminal)\n"
              "    Finished `test` profile [unoptimized + debuginfo] target(s) in 35.17s\n"
              "     Running unittests src/bin/cysd/main.rs (target/debug/deps/cysd-0000000000000000)\n")
RERUN_ERR = u("error: test failed, to rerun pass `--bin cysd`\n")
PASSING = ["mmm_fake::tests::passes_%02d" % i for i in range(1, 13)]
TWO_FAILED = ["aaa_fake::tests::first_fake_failure", "zzz_fake::tests::second_fake_failure"]


def libtest_out(passed, failed, preamble=(), extra_captured=(), ignored=6, filtered=1):
    """libtest 출력의 모양(`--test-threads=1` · 출력 캡처 켜짐) — 로컬 맥의 실제 실패 로그와 같은 구조다:
    검체 줄들 → `failures:` + 실패 검체마다 캡처된 출력 → `failures:` + 네 칸 들여쓴 이름 목록 → `test result:` 요약."""
    out = list(preamble) + ["", "running %d tests" % (len(passed) + len(failed))]
    out += ["test %s ... %s" % (n, "FAILED" if n in failed else "ok") for n in sorted(passed + failed)]
    if failed:
        out += ["", "failures:", ""]
        for n in sorted(failed):
            out += ["---- %s stdout ----" % n, "",
                    "thread '%s' (4242) panicked at src/bin/cysd/fake.rs:12:9:" % n,
                    "assertion `left == right` failed: 가짜 검체의 가짜 실패",
                    "  left: 1", " right: 2",
                    "    indented_line_inside_captured_output 은 검체 이름이 아니다",
                    "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace", ""]
        out += list(extra_captured)
        out += ["", "failures:"] + ["    " + n for n in sorted(failed)] + [""]
        out.append("test result: FAILED. %d passed; %d failed; %d ignored; 0 measured; %d filtered out; finished in 1.23s" % (len(passed), len(failed), ignored, filtered))
    else:
        out += ["", "test result: ok. %d passed; 0 failed; %d ignored; 0 measured; %d filtered out; finished in 1.23s" % (len(passed), ignored, filtered)]
    out.append("")
    return u("\n".join(out) + "\n")


class Ran(object):
    def __init__(self, rc, out, err, cargo_rc, cargo_out):
        self.rc, self.out, self.err, self.cargo_rc, self.cargo_out = rc, out, err, cargo_rc, cargo_out


class CysdMacStep(unittest.TestCase):
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls.step = extract_step(WORKFLOW, STEP_NAME)

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="cysd-step-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.bin, self.fake, self.rt, self.tmpd = (os.path.join(self.tmp, d) for d in ("bin", "fake", "runner-temp", "t"))
        for d in (self.bin, self.fake, self.rt, self.tmpd):
            os.mkdir(d)
        cargo = os.path.join(self.bin, "cargo")
        with open(cargo, "w", encoding="utf-8", newline="\n") as f:
            f.write(FAKE_CARGO)
        os.chmod(cargo, 0o755)
        self.script = os.path.join(self.tmp, "step.sh")
        with open(self.script, "w", encoding="utf-8", newline="\n") as f:
            f.write(self.step["run"])

    # ── 실행 ──
    def run_step(self, segs, rc, runner_temp=None, locale=None):
        """segs = [("out"|"err", 바이트), …] 를 가짜 cargo 가 순서대로 내고 rc 로 끝나게 한 뒤, 스텝 본문을 GitHub 의 `shell: bash` 방식으로 돌린다."""
        for n in os.listdir(self.fake):
            os.remove(os.path.join(self.fake, n))
        for i, (stream, data) in enumerate(segs):
            with open(os.path.join(self.fake, "seg-%03d.%s" % (i, stream)), "wb") as f:
                f.write(data)
        with open(os.path.join(self.fake, "rc"), "w") as f:
            f.write("%d\n" % rc)
        env = {"PATH": os.pathsep.join((self.bin, "/usr/bin", "/bin")), "RUNNER_TEMP": runner_temp or self.rt,
               "TMPDIR": self.tmpd, "HOME": self.tmp, "FAKE_CARGO_DIR": self.fake}
        if locale:
            env["LC_ALL"] = env["LANG"] = locale
        r = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", self.script], capture_output=True, env=env, cwd=self.tmp, timeout=120)
        ran = Ran(r.returncode, r.stdout, r.stderr, rc, b"".join(d for _s, d in segs))
        # 스텝이 `mktemp -d` 로 만든 CYS_PACK_DIR 은 이 검체의 임시 폴더 밖(OS 임시 폴더)에 생길 수 있다 — 맥의 mktemp 는 TMPDIR 을 따르지 않는 경우가 있다(로컬 실측).
        #   가짜 cargo 가 받아 적은 그 경로를 치운다. rmdir 은 **빈 폴더일 때만** 지운다(가짜 cargo 는 그 안에 아무것도 쓰지 않는다).
        made = os.path.join(self.fake, "packdir")
        pack = read_text(made).strip() if os.path.exists(made) else ""
        if pack and os.path.isdir(pack) and not os.path.islink(pack):
            try:
                os.rmdir(pack)
            except OSError:
                pass
        calls = os.path.join(self.fake, "calls")
        n_calls = len(read_text(calls).split()) if os.path.exists(calls) else 0
        self.assertEqual(n_calls, 1, "cargo 가 정확히 한 번 불려야 한다(%d회):\n%s\n-- stderr:\n%s" % (n_calls, quoted(ran.out), quoted(ran.err)))
        return ran

    def show(self, label, ran):
        if not SHOW:
            return
        same = ran.out.startswith(ran.cargo_out)
        lines = [l for l in ran.out[len(ran.cargo_out):].split(b"\n") if l] if same else []
        note = "" if same else " · ★cargo 출력(stdout+stderr)이 스텝 stdout 에 그대로 실리지 않았다(stdout %d · stderr %d바이트)" % (len(ran.out), len(ran.err))
        print("\n[%s] cargo rc=%d → 스텝 rc=%d · cargo 출력 %d바이트 · 덧붙은 줄 %d%s" % (label, ran.cargo_rc, ran.rc, len(ran.cargo_out), len(lines), note), flush=True)
        for l in lines:
            text = l.decode("utf-8", "replace")
            print("    | " + (text if len(text) <= 900 else text[:900] + " …(전체 %d자)" % len(text)), flush=True)

    # ── 판독 ──
    def added(self, ran):
        """cargo 출력 뒤에 스텝이 덧붙인 바이트. cargo 출력은 stdout 에 그대로(먼저) 나와야 한다 — stderr 도 같은 흐름에 실린다(`2>&1`)."""
        self.assertTrue(ran.out.startswith(ran.cargo_out),
                        "cargo 출력(stdout+stderr)이 스텝 stdout 에 그대로 나오지 않았다:\n%s\n-- stderr:\n%s" % (quoted(ran.out), quoted(ran.err)))
        rest = ran.out[len(ran.cargo_out):]
        if rest and ran.cargo_out and not ran.cargo_out.endswith(b"\n"):
            self.assertTrue(rest.startswith(b"\n"), "cargo 출력이 줄 중간에서 끝났는데 판독 줄이 그 줄에 이어 붙었다 — 러너는 줄 머리의 `::` 만 명령으로 읽는다:\n" + quoted(ran.out[-700:]))
        return rest

    def annotations(self, ran):
        """덧붙은 구간 → [(수준, 제목, 메시지 바이트)]. 빈 줄 말고 annotation 꼴이 아닌 줄이 있으면 실패다."""
        found = []
        for line in self.added(ran).split(b"\n"):
            if not line:
                continue
            m = re.match(rb"^::(error|warning|notice) title=([^:]*)::(.*)$", line, re.S)
            self.assertIsNotNone(m, "덧붙은 줄이 annotation 꼴이 아니다:\n" + quoted(line[:600]))
            found.append((m.group(1).decode(), m.group(2).decode("utf-8", "replace"), m.group(3)))
        return found

    def failure_report(self, ran, rc):
        """실패 갈래의 공통 핀 → (판정 제목, 판정 메시지, 꼬리 메시지 또는 None)."""
        self.assertEqual(ran.rc, rc, "스텝 종료 코드가 cargo 의 종료 코드와 다르다 — 차단 의미가 바뀌었다(0 이면 실패를 삼킨 것이다):\n%s\n-- stderr:\n%s" % (quoted(ran.out), quoted(ran.err)))
        anns = self.annotations(ran)
        self.assertTrue(anns, "실패했는데 annotation 이 한 줄도 없다 — 익명으로 읽히는 실패 사유가 'exit code %d' 한 줄뿐이다:\n%s\n-- stderr:\n%s" % (rc, quoted(ran.out), quoted(ran.err)))
        self.assertEqual([lvl for lvl, _t, _m in anns], ["error"] * len(anns), "실패 판독은 전부 error 수준이어야 한다")
        self.assertLessEqual(len(anns), ERROR_BUDGET, "error annotation 이 %d줄이다 — 스텝당 앞 %d줄만 남는다(실측) · 뒤의 줄은 사라진다" % (len(anns), ERROR_BUDGET))
        self.assertIn(anns[0][1], (T_FAIL, T_HARNESS), "판정(이름) 줄이 맨 먼저가 아니다 — 앞 %d줄 밖으로 밀리면 사라진다: 첫 줄 제목 %r" % (ERROR_BUDGET, anns[0][1]))
        verdicts = [(t, m) for _l, t, m in anns if t in (T_FAIL, T_HARNESS)]
        tails = [m for _l, t, m in anns if t == T_TAIL]
        self.assertEqual(len(verdicts), 1, "판정 줄은 정확히 하나여야 한다: %r" % [t for _l, t, _m in anns])
        self.assertLessEqual(len(tails), 1, "로그 꼬리는 줄마다가 아니라 한 줄이어야 한다(%d줄)" % len(tails))
        self.assertEqual(len(anns), 1 + len(tails), "모르는 제목의 annotation 이 있다: %r" % [t for _l, t, _m in anns])
        return verdicts[0][0], verdicts[0][1].decode("utf-8", "replace"), (tails[0] if tails else None)

    def failed_names(self, msg, count):
        """`failed=N — 이름 이름 …` 에서 이름 토큰들을 뗀다(뒤의 안내 문구는 버린다)."""
        head = "failed=%d — " % count
        self.assertTrue(msg.startswith(head), "판정 메시지가 `%s` 로 시작하지 않는다: %r" % (head, msg))
        toks = msg[len(head):].split()
        return [t for t in toks if "::" in t], [t for t in toks if "::" not in t]

    # ── 네 갈래(티켓) ──
    def test_01_success_exits_0_passes_the_output_through_and_adds_nothing(self):
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, []))], 0)
        self.show("① 성공", ran)
        self.assertEqual(ran.rc, 0, quoted(ran.out) + "\n-- stderr:\n" + quoted(ran.err))
        self.assertEqual(ran.out, ran.cargo_out, "성공이면 cargo 출력 그대로여야 한다(덧붙는 줄 없음)")
        self.assertEqual(ran.err, b"", "성공 경로에서 stderr 에 무언가 나왔다:\n" + quoted(ran.err))

    def test_02_two_failed_tests_keep_the_cargo_exit_code_and_name_both_tests_first(self):
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, TWO_FAILED)), ("err", RERUN_ERR)], 101)
        self.show("② 테스트 2건 실패", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_FAIL, msg)
        names, rest = self.failed_names(msg, 2)
        self.assertEqual(names, sorted(TWO_FAILED), "실패 검체 이름 두 개가 판정 줄에 있어야 한다: %r" % msg)
        self.assertEqual(rest, [], "이름 말고 다른 토큰이 있다(20개 이하인데 생략 안내가 붙었다?): %r" % msg)
        self.assertEqual(tail, expected_tail(ran.cargo_out), "로그 꼬리 20줄이 한 줄(%0A)로 그대로 실려야 한다")
        self.assertIn(u("test result: FAILED. 12 passed; 2 failed"), tail)
        self.assertIn(RERUN_ERR.rstrip(b"\n"), tail, "cargo 가 stderr 에 낸 줄도 로그에 있어야 한다(2>&1)")

    def test_03_nonzero_exit_without_a_summary_is_a_harness_anomaly_not_zero_failures(self):
        compile_err = u("   Compiling cys-terminal v0.14.43 (/work/cys-terminal)\n"
                        "error[E0425]: cannot find value `nope` in this scope\n"
                        "    --> src/bin/cysd/fake.rs:1:1\n\n"
                        "error: could not compile `cys-terminal` (bin \"cysd\" test) due to 1 previous error\n")
        ran = self.run_step([("err", compile_err)], 101)
        self.show("③ 요약 없이 rc 101(컴파일 실패 모양 · 전부 stderr)", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_HARNESS, msg)
        for want in ("rc=101", "'test result' 요약이 없다", "컴파일 실패이거나 하네스가 요약 전에 죽었다", "'실패한 테스트 없음' 이 아니다"):
            self.assertIn(want, msg)
        self.assertEqual(tail, expected_tail(ran.cargo_out))
        self.assertIn(b"could not compile", tail)

    def test_04_exit_0_is_trusted_even_when_the_log_shows_failed_lines(self):
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, TWO_FAILED)), ("err", RERUN_ERR)], 0)
        self.show("④ rc 0 인데 로그에 FAILED·failures 블록", ran)
        self.assertEqual(ran.rc, 0, "판정은 cargo 의 종료 코드 하나다 — 로그 문구로 붉히면 차단 의미가 바뀐다:\n" + quoted(ran.out[-900:]))
        self.assertEqual(ran.out, ran.cargo_out, "rc 0 이면 덧붙는 줄이 없어야 한다")
        self.assertEqual(ran.err, b"")

    # ── 그 밖의 갈래 ──
    def test_05_summary_with_zero_failed_but_nonzero_exit_is_a_harness_anomaly(self):
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, [])), ("err", u("error: test failed, to rerun pass `--bin cysd`\n\nCaused by:\n  process didn't exit successfully (signal: 6, SIGABRT: process abort signal)\n"))], 101)
        self.show("⑤ 요약은 failed=0 인데 rc 101", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_HARNESS, msg)
        for want in ("rc=101", "failed=0", "하네스 이상"):
            self.assertIn(want, msg)
        self.assertEqual(tail, expected_tail(ran.cargo_out))

    def test_06_more_than_20_failures_names_the_first_20_and_says_so(self):
        many = ["fake_many::tests::case_%02d" % i for i in range(25)]
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, many)), ("err", RERUN_ERR)], 101)
        self.show("⑥ 25건 실패", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_FAIL, msg)
        names, rest = self.failed_names(msg, 25)
        self.assertEqual(names, sorted(many)[:20], "이름은 이름순 앞 20개여야 한다")
        self.assertIn("20개", " ".join(rest), "20개를 넘겨 잘랐다는 안내가 없다 — 판정 줄이 전부인 것처럼 읽힌다: %r" % msg)
        self.assertEqual(tail, expected_tail(ran.cargo_out))

    def test_07_the_cargo_exit_code_is_propagated_unchanged(self):
        for code in (1, 3, 101, 137, 255):
            with self.subTest(code=code):
                ran = self.run_step([("out", u("가짜 cargo 가 종료 코드 %d 로 끝난다\n" % code))], code)
                self.show("⑦ 종료 코드 %d" % code, ran)
                title, msg, _tail = self.failure_report(ran, code)
                self.assertEqual(title, T_HARNESS, msg)
                self.assertIn("rc=%d" % code, msg)

    def test_08_an_unwritable_log_changes_neither_the_verdict_nor_the_exit_code(self):
        nowhere = os.path.join(self.tmp, "no-such-dir")
        ok = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, []))], 0, runner_temp=nowhere)
        self.show("⑧-가 로그를 쓸 수 없다 · cargo rc 0", ok)
        self.assertEqual(ok.rc, 0, "로그를 못 썼다는 이유로 통과가 실패로 바뀌면 안 된다(판정은 cargo 의 종료 코드):\n" + quoted(ok.err))
        self.assertEqual(ok.out, ok.cargo_out)
        bad = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, TWO_FAILED)), ("err", RERUN_ERR)], 101, runner_temp=nowhere)
        self.show("⑧-나 로그를 쓸 수 없다 · cargo rc 101", bad)
        title, msg, tail = self.failure_report(bad, 101)
        self.assertEqual(title, T_HARNESS, msg)
        self.assertIn("로그가 없거나 비어 있다", msg)
        self.assertIn("'실패한 테스트 없음' 이 아니다", msg)
        self.assertIsNone(tail, "로그가 없는데 꼬리 줄이 나왔다")

    def test_09_bytes_that_stop_bsd_text_tools_do_not_lose_the_names_or_the_tail(self):
        # 맥의 sed·tr·sort·awk 는 UTF-8 로케일에서 깨진 바이트 하나에 멈추고, grep 은 NUL 하나에 "Binary file … matches" 만 낸다 — 실패한 검체가 터미널 바이트를 찍었다고 이름·꼬리가 사라지면 안 된다.
        early = ["NUL 두 개 \x00\x00 가 든 줄(로그 앞쪽 — 꼬리 20줄 밖)"]
        captured = ["---- 캡처된 출력의 끝(특수 바이트 — 꼬리 20줄 안) ----", "퍼센트 100% 와 %0A 는 글자 그대로", "윈도우식 줄 끝\r", "백슬래시 \\n \\t \\\\ 는 글자 그대로",
                    "깨진 바이트 \udcff\udcfe 가 든 줄", "::error title=가짜::검체가 찍은 줄은 명령이 되면 안 된다", ""]
        out = libtest_out(PASSING, TWO_FAILED, preamble=early, extra_captured=captured)
        self.assertIn(b"\xff\xfe", out)
        self.assertIn(b"\x00\x00", out)
        seen = {}
        for locale in (None, "C", "en_US.UTF-8", "ko_KR.UTF-8"):
            with self.subTest(locale=locale):
                ran = self.run_step([("err", BUILD_ERR), ("out", out), ("err", RERUN_ERR)], 101, locale=locale)
                self.show("⑨ 깨진 바이트·NUL·CR·%% · 로케일 %s" % (locale or "(없음)"), ran)
                title, msg, tail = self.failure_report(ran, 101)
                self.assertEqual(title, T_FAIL, msg)
                self.assertEqual(self.failed_names(msg, 2)[0], sorted(TWO_FAILED))
                self.assertEqual(tail, expected_tail(ran.cargo_out), "꼬리가 바이트 그대로(%% → %%25 · CR 제거 · 한 줄) 실리지 않았다")
                self.assertIn(b"\xff\xfe", tail)
                self.assertIn(u("::error title=가짜::"), tail, "검체가 찍은 `::error` 줄은 꼬리 **안**에 글자로만 있어야 한다")
                seen[locale] = self.added(ran)
        self.assertEqual(len(set(seen.values())), 1, "로케일에 따라 판독 출력이 달라진다: %r" % sorted(str(k) for k in seen))

    def test_10_output_cut_in_the_middle_of_a_line_still_yields_annotations_at_line_start(self):
        cut = u("\nrunning 3 tests\ntest fake_hang::tests::first ... ok\ntest fake_hang::tests::stuck_forever ... ")
        ran = self.run_step([("out", cut)], 137)
        self.show("⑩ 줄 중간에서 끊긴 출력 · rc 137", ran)
        title, msg, tail = self.failure_report(ran, 137)
        self.assertEqual(title, T_HARNESS, msg)
        self.assertIn("'test result' 요약이 없다", msg)
        self.assertEqual(tail, expected_tail(ran.cargo_out))
        self.assertTrue(tail.endswith(u("test fake_hang::tests::stuck_forever ... ")), "마지막으로 시작한 검체가 꼬리의 끝에 보여야 한다")

    def test_11_a_stale_log_from_an_earlier_run_is_never_read(self):
        stale = os.path.join(self.rt, LOG_NAME)
        with open(stale, "wb") as f:
            f.write(libtest_out([], ["stale_fake::tests::from_an_earlier_run"]))
        os.chmod(stale, 0o444)   # 쓰기 금지 — 지우지 않고 덮어쓰려 하면 tee 가 열지 못해 낡은 내용이 남는다
        ran = self.run_step([("err", BUILD_ERR), ("out", libtest_out(PASSING, TWO_FAILED)), ("err", RERUN_ERR)], 101)
        self.show("⑪ 낡은 로그(쓰기 금지)가 남아 있다", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_FAIL, msg)
        self.assertEqual(self.failed_names(msg, 2)[0], sorted(TWO_FAILED), "낡은 로그의 이름을 읽었다: %r" % msg)
        self.assertNotIn(b"stale_fake", self.added(ran))
        with open(stale, "rb") as f:
            self.assertEqual(f.read(), ran.cargo_out, "로그 파일이 이번 실행의 cargo 출력 전체여야 한다")

    def test_12_the_command_line_and_the_pack_dir_isolation_are_unchanged(self):
        ran = self.run_step([("out", libtest_out(PASSING, []))], 0)
        self.assertEqual(ran.rc, 0)
        read = lambda n: read_text(os.path.join(self.fake, n))
        self.assertEqual(read("argv").split("\n")[:-1], CARGO_ARGV, "cargo 인자가 바뀌었다 — 실행 명령은 그대로여야 한다")
        self.assertEqual(read("packdir-is-dir").strip(), "yes", "CYS_PACK_DIR 가 실재하는 임시 폴더가 아니다: %r" % read("packdir"))
        if SHOW:
            print("\n[⑫ 실행 인자] cargo %s · CYS_PACK_DIR=임시 폴더(실재)" % " ".join(CARGO_ARGV), flush=True)

    def test_13_a_failed_count_without_a_readable_names_block_says_so_instead_of_an_empty_list(self):
        body = libtest_out(PASSING, TWO_FAILED).decode("utf-8")
        cut = "\nfailures:\n" + "".join("    %s\n" % n for n in sorted(TWO_FAILED))
        self.assertEqual(body.count(cut), 1)
        ran = self.run_step([("err", BUILD_ERR), ("out", u(body.replace(cut, "\n", 1))), ("err", RERUN_ERR)], 101)
        self.show("⑬ 요약은 2 failed 인데 이름 목록 블록이 없다", ran)
        title, msg, tail = self.failure_report(ran, 101)
        self.assertEqual(title, T_FAIL, msg)
        names, rest = self.failed_names(msg, 2)
        self.assertEqual(names, [], "이름 블록이 없는데 이름이 나왔다: %r" % msg)
        self.assertIn("이름을 읽지 못했다", msg, "이름 자리가 비면 '실패한 검체가 없다'로 읽힌다 — 못 읽었다고 적어야 한다")
        self.assertEqual(tail, expected_tail(ran.cargo_out))

    def test_14_the_step_stays_blocking_and_unconditional(self):
        step = self.step
        self.assertNotIn("continue-on-error", step, "이 스텝은 차단이다 — continue-on-error 를 달면 붉어도 잡이 초록이다(레인 대조 게이트는 이 키를 보지 못한다)")
        self.assertNotIn("if", step, "이 스텝은 무조건 돈다 — if: 를 달지 않는다(돌지 않는 초록 금지)")
        self.assertEqual(step.get("shell"), "bash", "판독 본문은 bash 를 전제한다(PIPESTATUS)")
        if SHOW:
            print("\n[⑭ 스텝 키] %s" % ", ".join(k for k in step if k != "run"), flush=True)


class S2WindowsCell(unittest.TestCase):
    """Run the actual S2 shell cell with fake Cargo; no Windows/native claim."""
    NAME = "A-ID S2 root registration (Windows blocking)"
    REQUIRED = [
        "root_registration::tests::windows_production_wait_result_preserves_unavailable",
        "root_registration::tests::observes_once_with_original_child_on_both_sides_and_binds_pid",
        "root_registration::tests::elapsed_limit_is_post_hoc_and_inclusive",
    ]

    def setUp(self):
        self.repo = os.path.normpath(os.path.join(HERE, "..", ".."))
        self.windows = os.path.join(self.repo, ".github", "workflows", "windows-health.yml")
        self.step = extract_step(self.windows, self.NAME)
        self.tmp = tempfile.mkdtemp(prefix="aid-s2-cell-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def run_fake(self, names=None, list_rc=0, test_rc=0):
        folder = tempfile.mkdtemp(dir=self.tmp)
        binary = os.path.join(folder, "cargo")
        with open(binary, "w") as stream:
            stream.write("#!/bin/sh\ncase \" $* \" in\n*' --list '*) cat \"$S2_FAKE_LIST\"; exit \"$S2_LIST_RC\";;\n*) echo executed > \"$S2_RAN\"; exit \"$S2_TEST_RC\";;\nesac\n")
        os.chmod(binary, 0o755)
        listing = os.path.join(folder, "names")
        with open(listing, "w") as stream:
            stream.write("".join(name + ": test\r\n" for name in (self.REQUIRED if names is None else names)))
        script = os.path.join(folder, "step.sh")
        with open(script, "w") as stream:
            stream.write(self.step["run"])
        marker = os.path.join(folder, "executed")
        env = dict(os.environ, PATH=folder + os.pathsep + "/usr/bin:/bin", S2_FAKE_LIST=listing,
                   S2_LIST_RC=str(list_rc), S2_TEST_RC=str(test_rc), S2_RAN=marker)
        run = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", script],
                             cwd=folder, env=env, capture_output=True, timeout=20)
        return run.returncode, os.path.exists(marker)

    def test_named_positive_reaches_actual_test_command(self):
        self.assertEqual(self.run_fake(), (0, True))

    def test_zero_or_missing_windows_name_blocks_execution(self):
        for names in [[], self.REQUIRED[1:]]:
            with self.subTest(names=names):
                rc, ran = self.run_fake(names)
                self.assertNotEqual(rc, 0)
                self.assertFalse(ran)

    def test_listing_and_test_failures_are_not_masked(self):
        self.assertEqual(self.run_fake(list_rc=101), (101, False))
        self.assertEqual(self.run_fake(test_rc=101), (101, True))

    def test_cell_is_blocking_and_registered(self):
        import ast
        self.assertNotIn("if", self.step)
        self.assertNotIn("continue-on-error", self.step)
        self.assertEqual(self.step["shell"], "bash")
        code = self.gate_code()
        assignments = {node.targets[0].id: node.value for node in ast.parse(code).body
                       if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)}
        required = ast.literal_eval(assignments["FILTERED_CARGO_REQUIRED"])
        self.assertIn("--bin cysd root_registration::", required["windows-health"])
        self.assertEqual(list(ast.literal_eval(assignments["S2_REQUIRED_NAMES"])), self.REQUIRED)

    def gate_code(self):
        lines = read_text(WORKFLOW).splitlines()
        begin = next(i for i, line in enumerate(lines) if line.strip() == "python3 - <<'PY'") + 1
        end = next(i for i in range(begin, len(lines)) if lines[i].strip() == "PY")
        return "\n".join(line[10:] for line in lines[begin:end])

    def test_actual_lane_gate_rejects_missing_conditional_nonblocking_and_unguarded_cell(self):
        destination = os.path.join(self.tmp, "gate")
        shutil.copytree(os.path.join(self.repo, ".github"), os.path.join(destination, ".github"))
        windows = os.path.join(destination, ".github", "workflows", "windows-health.yml")
        original = read_text(windows)
        start = original.index("      - name: " + self.NAME)
        end = original.index("      # Q9-b:", start)
        cell = original[start:end]
        def run_gate(content):
            with open(windows, "w") as stream:
                stream.write(content)
            return subprocess.run([sys.executable, "-c", self.gate_code()], cwd=self.repo,
                                  env=dict(os.environ, LANE_GATE_ROOT=destination), capture_output=True, timeout=20)
        baseline = run_gate(original)
        self.assertEqual(baseline.returncode, 0, quoted(baseline.stdout + baseline.stderr))
        guard = "          grep -Fqx '" + self.REQUIRED[0] + ": test' \"$s2_list\"\n"
        variants = ["", cell.replace("        timeout-minutes:", "        if: false\n        timeout-minutes:", 1),
                    cell.replace("        timeout-minutes:", "        continue-on-error: true\n        timeout-minutes:", 1),
                    cell.replace(guard, ""),
                    cell.replace("root_registration:: -- --test-threads=1", "root_registration:: -- --test-threads=1 --skip missing")]
        for changed in variants:
            with self.subTest(change=variants.index(changed)):
                result = run_gate(original[:start] + changed + original[end:])
                self.assertNotEqual(result.returncode, 0, "actual lane gate accepted a broken required S2 cell")


# ── B-344 (DESIGN-v4 s6, s8 T2-T5): windows-health bootstrap-health step 5, its reader and its verdict step ─────────
# The actual step bodies are taken out of windows-health.yml and run under bash with a fake `python` (step 5) or with
# marker files written as exact bytes (verdict step); the reader heredoc runs on synthetic runner documents. Mutation and
# RED-first runs point WINDOWS_HEALTH_YML_UNDER_TEST at another copy of the workflow.
WINDOWS_HEALTH = os.environ.get("WINDOWS_HEALTH_YML_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "..", ".github", "workflows", "windows-health.yml"))
BH_STEP = "부트스트랩 건강성 — H-WIN 검체 실기 (windows-latest)"
BH_VERDICT = "부트스트랩 건강성 판정 결박 (H-WIN · 차단 · fail-closed)"
BH_STATUSES = ("pass", "fail", "skip", "disabled", "pending")
BH_CAP = 16 * 1024
BH_DIR_ENTRY = object()          # a verdict cell whose rc marker path is a directory


def bh_read_bytes(path):
    """The file's exact bytes, or None when there is no regular file."""
    if not os.path.isfile(path):
        return None
    with open(path, "rb") as stream:
        return stream.read()


def bh_reader_code(step_run):
    """The step-5 reader heredoc out of the step's run text (extract_step already removed the run block's indentation)."""
    lines = step_run.split("\n")
    starts = [i for i, line in enumerate(lines) if line.startswith('python - "$rc"') and "<<'PY'" in line]
    if len(starts) != 1:
        raise StepShapeError("step 5 has %d reader heredocs (expected 1)" % len(starts))
    end = next((i for i in range(starts[0] + 1, len(lines)) if lines[i] == "PY"), None)
    if end is None:
        raise StepShapeError("the step-5 reader heredoc has no PY terminator")
    return "\n".join(lines[starts[0] + 1:end]) + "\n"


def bh_reader_funcs(code, names):
    """Exec only the imports, the upper-case constants and the named functions of the reader (it runs its main code at
    import), so the cut functions can be probed directly."""
    import ast
    tree = ast.parse(code)
    keep = [n for n in tree.body if isinstance(n, (ast.Import, ast.ImportFrom))
            or (isinstance(n, ast.Assign) and all(isinstance(t, ast.Name) and t.id.isupper() for t in n.targets))
            or (isinstance(n, ast.FunctionDef) and n.name in names)]
    found = {n.name for n in keep if isinstance(n, ast.FunctionDef)}
    if found != set(names):
        raise AssertionError("reader functions missing: %s" % sorted(set(names) - found))
    ns = {}
    exec(compile(ast.Module(body=keep, type_ignores=[]), "<step-5 reader>", "exec"), ns)
    return ns


def bh_doc(rows, verdict=None, **override):
    """The full summary the runner writes, counts derived from the rows unless overridden (a stale count)."""
    s = dict((k, sum(1 for r in rows if r.get("status") == k)) for k in BH_STATUSES)
    s["verdict"] = verdict or ("RED" if s["fail"] else "GREEN")
    s["total"] = len(rows)
    s.update(override)
    return {"summary": s, "specimens": rows}


def bh_rows(n, status="pass"):
    return [{"id": "H-WIN-%d" % i, "status": status, "detail" if status == "pass" else "reason": "ok"} for i in range(1, n + 1)]


def bh_hw(cats, msgs=None, status=None, child_extra=None):
    msgs = msgs or {}
    causes = [{"category": c, "code": "X", "message": msgs.get(c, "%s cause" % c)} for c in cats]
    ch = {"categories": list(cats), "causes": causes, "failing": [], "stdout": {"bytes": 0}, "stderr": {"bytes": 0}}
    ch.update(child_extra or {})
    return {"id": "H-WIN-11", "status": status or ("fail" if cats else "pass"), "reason": "r", "evidence": {"child": ch}}


def bh_blocks(raw):
    """Bytes printed per stream block: from a '--- <stream>: ' header to the next one or the end of the stop region."""
    out, cur = {}, None
    for line in raw.split(b"\n"):
        if line.startswith(b"--- stdout: ") or line.startswith(b"--- stderr: "):
            cur = line[4:10].decode()
            out[cur] = 0
        elif cur is not None and line.startswith(b"::") and line.endswith(b"::") and len(line) == 36:
            cur = None
        if cur is not None:
            out[cur] += len(line) + 1
    return out


class BootstrapHealthStructure(unittest.TestCase):
    """T3: step 5 is severable (id, continue-on-error, own timeout); the verdict step is unconditional, blocking and after
    every blocking step; the cells the lane gate pins stay unconditional and blocking."""

    def test_step5_is_severable_and_bounded(self):
        step = extract_step(WINDOWS_HEALTH, BH_STEP)
        self.assertEqual(step.get("id"), "bootstrap_health")
        self.assertEqual(step.get("continue-on-error"), "true")
        self.assertEqual(step.get("timeout-minutes"), "20")
        self.assertNotIn("if", step)
        self.assertEqual(step.get("shell"), "bash")
        self.assertIn('echo "$vrc" > "$BH_DIR/rc"', step["run"], "the rc marker is written from the reader's exit code")
        self.assertIn('exit "$vrc"', step["run"], "the step still fails exactly as before (outcome failure)")

    def test_verdict_step_is_unconditional_blocking_and_after_every_blocking_step(self):
        step = extract_step(WINDOWS_HEALTH, BH_VERDICT)
        self.assertEqual(step.get("if"), "always()")
        self.assertNotIn("continue-on-error", step)
        self.assertEqual(step.get("timeout-minutes"), "2")
        self.assertEqual(step.get("shell"), "bash")
        self.assertEqual(step.get("env"), {"BH_OUTCOME": "${{ steps.bootstrap_health.outcome }}"})
        self.assertIn('readm "$d/rc"', step["run"])
        self.assertIn('readm "$d/category"', step["run"])
        names = [line.strip()[len("- name: "):] for line in read_text(WINDOWS_HEALTH).split("\n")
                 if line.startswith("      - name: ")]
        at = names.index(BH_VERDICT)
        self.assertLess(names.index("결과 업로드 (기계 판독 JSON)"), at, "the verdict step comes after the artifact upload")
        for later in names[at + 1:]:
            self.assertEqual(extract_step(WINDOWS_HEALTH, later).get("continue-on-error"), "true",
                             "a blocking step after the verdict step: %s" % later)

    def test_lane_gate_cells_stay_unconditional_and_blocking(self):
        for name in ("A-ID S2 root registration (Windows blocking)", "A-ID Q9-b vendor clone error controls (Windows blocking)",
                     "A-ID S6 Tauri close operation (Windows blocking)"):
            with self.subTest(cell=name):
                step = extract_step(WINDOWS_HEALTH, name)
                self.assertNotIn("if", step)
                self.assertNotIn("continue-on-error", step)


class BootstrapHealthStepBodies(unittest.TestCase):
    """T2: the step-5 body with a fake `python` (14 scenarios, byte-exact marker oracle) and the verdict body on 49 marker
    cells written as exact bytes (codex's 30 00 0a first among the R3 cells), plus the byte sweep of both markers."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls.step5 = extract_step(WINDOWS_HEALTH, BH_STEP)["run"]
        cls.verdict = extract_step(WINDOWS_HEALTH, BH_VERDICT)["run"]
        cls.tmp = tempfile.mkdtemp(prefix="bh-steps-")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def run_step5(self, text, runner_rc):
        w = tempfile.mkdtemp(dir=self.tmp)
        os.makedirs(os.path.join(w, "bin"))
        os.makedirs(os.path.join(w, "cysjavis-pack", "bin", "tests"))
        with open(os.path.join(w, "cysjavis-pack", "bin", "tests", "run_bootstrap_health.py"), "w") as stream:
            stream.write("# placeholder\n")
        fake = os.path.join(w, "bin", "python")
        with open(fake, "w") as stream:
            stream.write('#!/bin/bash\nif [ "$1" = "-" ]; then exec "%s" "$@"; fi\ncat "$FAKE_JSON"; exit "$FAKE_RC"\n' % sys.executable)
        os.chmod(fake, 0o755)
        canned = os.path.join(w, "canned.json")
        with open(canned, "wb") as stream:
            stream.write(text if isinstance(text, bytes) else text.encode("utf-8"))
        script = os.path.join(w, "step5.sh")
        with open(script, "w", encoding="utf-8") as stream:
            stream.write(self.step5)
        rt = os.path.join(w, "rt")
        os.makedirs(rt)
        env = {"PATH": os.path.join(w, "bin") + ":/usr/bin:/bin", "HOME": w, "RUNNER_TEMP": rt, "FAKE_JSON": canned,
               "FAKE_RC": str(runner_rc), "PYTHONUTF8": "1", "LANG": "en_US.UTF-8"}
        r = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", script], cwd=w, env=env,
                           capture_output=True, timeout=60)
        markers = {}
        for k in ("rc", "category"):
            p = os.path.join(rt, "bootstrap-health", k)
            markers[k] = bh_read_bytes(p)      # byte-exact oracle, never stripped
        return r, markers

    def test_step5_scenarios(self):
        def dup_row_status():
            rows = [json.dumps(r, ensure_ascii=False) for r in bh_rows(24)]
            rows.append('{"id": "H-WIN-25", "status": "fail", "reason": "x", "status": "pass"}')
            s = json.dumps(bh_doc(bh_rows(25))["summary"], ensure_ascii=False)
            return '{"summary": %s, "specimens": [%s]}' % (s, ", ".join(rows))

        def dup_summary_verdict():
            s = json.dumps(bh_doc(bh_rows(25))["summary"], ensure_ascii=False)
            return '{"summary": {"verdict": "UNMEASURED", %s, "specimens": %s}' % (s[1:], json.dumps(bh_rows(25), ensure_ascii=False))

        scen = [   # (label, runner document (dict or raw text) or None = not JSON, runner rc, step rc, category marker)
            ("green", bh_doc(bh_rows(25)), 0, 0, "NONE"),
            ("red PRODUCT", bh_doc(bh_rows(24) + [bh_hw(["PRODUCT"])]), 1, 1, "PRODUCT"),
            ("red HARNESS", bh_doc(bh_rows(24) + [bh_hw(["HARNESS"])]), 1, 1, "HARNESS"),
            ("red both", bh_doc(bh_rows(24) + [bh_hw(["PRODUCT", "HARNESS"])]), 1, 1, "HARNESS,PRODUCT"),
            ("runner rc2 no fail", bh_doc(bh_rows(25), verdict="UNMEASURED"), 2, 1, "HARNESS"),
            ("unparsable json", None, 1, 1, "HARNESS"),
            ("other specimen fail", bh_doc(bh_rows(24) + [{"id": "H-WIN-25", "status": "fail", "reason": "x", "fail_kind": "fail"}]),
             1, 1, "PRODUCT"),
            ("other specimen crash", bh_doc(bh_rows(24) + [{"id": "H-WIN-25", "status": "fail", "reason": "검체 크래시 E: x",
                                                            "fail_kind": "crash"}]), 1, 1, "HARNESS"),
            ("pass 0", bh_doc(bh_rows(1, "skip")), 0, 1, "HARNESS"),
            ("R2 stale parent pass", bh_doc(bh_rows(25, "skip"), verdict="GREEN", **{"pass": 1, "skip": 24}), 0, 1, "HARNESS"),
            ("R2 rc0 verdict UNMEASURED", bh_doc(bh_rows(25), verdict="UNMEASURED"), 0, 1, "HARNESS"),
            ("R2 POSITIVE green with skips", bh_doc(bh_rows(20) + [{"id": "S-%d" % i, "status": "skip", "reason": "s"}
                                                                  for i in range(5)]), 0, 0, "NONE"),
            ("R3 duplicate row status (fail, then pass)", dup_row_status(), 0, 1, "HARNESS"),
            ("R3 duplicate summary verdict (UNMEASURED, then GREEN)", dup_summary_verdict(), 0, 1, "HARNESS"),
        ]
        for label, d, runner_rc, want_rc, want_cat in scen:
            with self.subTest(scenario=label):
                text = d if isinstance(d, str) else (json.dumps(d, ensure_ascii=False) if d else "Traceback: boom")
                r, m = self.run_step5(text, runner_rc)
                got = (r.returncode, m["rc"], m["category"])
                self.assertEqual(got, (want_rc, b"%d\n" % want_rc, want_cat.encode()), quoted(r.stdout + r.stderr))

    def run_verdict(self, outcome, rc_marker, category_marker):
        w = tempfile.mkdtemp(dir=self.tmp)
        d = os.path.join(w, "bootstrap-health")
        os.makedirs(d)
        if rc_marker is BH_DIR_ENTRY:
            os.makedirs(os.path.join(d, "rc"))
        elif rc_marker is not None:
            with open(os.path.join(d, "rc"), "wb") as stream:
                stream.write(rc_marker)
        if category_marker is not None:
            with open(os.path.join(d, "category"), "wb") as stream:
                stream.write(category_marker)
        script = os.path.join(w, "verdict.sh")
        with open(script, "w", encoding="utf-8") as stream:
            stream.write(self.verdict)
        env = {"PATH": "/usr/bin:/bin", "RUNNER_TEMP": w, "BH_OUTCOME": outcome, "LANG": "en_US.UTF-8"}
        return subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", script], cwd=w, env=env,
                              capture_output=True, timeout=30)

    def test_verdict_cells(self):
        cells = [  # (outcome, rc marker bytes / None / BH_DIR_ENTRY, category marker bytes / None, exit, titles, label)
            ("success", b"0\n", b"NONE", 0, set(), "green"),
            ("failure", b"1\n", b"PRODUCT", 1, {"PRODUCT"}, "red PRODUCT"),
            ("failure", b"1\n", b"HARNESS", 1, {"HARNESS"}, "red HARNESS"),
            ("failure", b"1\n", b"HARNESS,PRODUCT", 1, {"PRODUCT", "HARNESS"}, "red both"),
            ("success", None, None, 1, {"HARNESS"}, "no markers"),
            ("success", b"1\n", b"PRODUCT", 1, {"PRODUCT", "HARNESS"}, "success with rc 1"),
            ("", b"0\n", b"NONE", 1, {"HARNESS"}, "empty outcome"),
            ("skipped", None, None, 1, {"HARNESS"}, "skipped"),
            ("cancelled", None, None, 1, {"HARNESS"}, "cancelled"),
            ("failure", b"0\n", b"NONE", 1, {"HARNESS"}, "failure with rc 0"),
            ("failure", b"1\n", None, 1, {"HARNESS"}, "no category"),
            ("failure", b"1\n", b"PRODUCTX\n::warning::Y%", 1, {"HARNESS"}, "injected category"),
            ("success", b"0\n", None, 1, {"HARNESS"}, "R2 codex missing category"),
            ("success", b"0\n", b"PRODUCT", 1, {"PRODUCT", "HARNESS"}, "R2 codex PRODUCT category with rc 0"),
            ("success", b"error0garbage\n", b"NONE", 1, {"HARNESS"}, "R2 codex malformed rc"),
            ("success", b" 0\n", b"NONE", 1, {"HARNESS"}, "R2 rc with a leading space"),
            ("success", b"0\n\n", b"NONE", 1, {"HARNESS"}, "R2 rc with two LF"),
            ("success", b"00\n", b"NONE", 1, {"HARNESS"}, "R2 rc 00"),
            ("success", b"0\r\n", b"NONE", 1, {"HARNESS"}, "R2 rc with CR LF"),
            ("success", u"٠\n".encode("utf-8"), b"NONE", 1, {"HARNESS"}, "R2 rc U+0660"),
            ("success", b"0" + b" " * 40 + b"\n", b"NONE", 1, {"HARNESS"}, "R2 rc marker over 32 B"),
            ("success", BH_DIR_ENTRY, b"NONE", 1, {"HARNESS"}, "R2 rc marker is a directory"),
            ("success", b"0\n", b"NONEX", 1, {"HARNESS"}, "R2 category NONEX"),
            ("success", b"0\n", b"", 1, {"HARNESS"}, "R2 category empty file"),
            ("success", b"0\n", b"NONE\r\n", 1, {"HARNESS"}, "R2 category with CR LF"),
            ("success", b"0\n", b"HARNESS,PRODUCT", 1, {"PRODUCT", "HARNESS"}, "R2 both categories with rc 0"),
            ("failure", b"1\n", b"PRODUCT,HARNESS", 1, {"HARNESS"}, "R2 categories in the wrong order"),
            ("failure", b"1\n", b"NONE", 1, {"HARNESS"}, "R2 rc 1 with category NONE"),
            ("success", b"0", b"NONE", 0, set(), "R2 POSITIVE rc marker without LF"),
            ("success", b"0\n", b"NONE\n", 0, set(), "R2 POSITIVE category with one LF"),
            ("success", b"0\x00\n", b"NONE", 1, {"HARNESS"}, "R3 codex rc 30 00 0a"),
            ("success", b"\x000\n", b"NONE", 1, {"HARNESS"}, "R3 rc NUL before the digit"),
            ("success", b"0\n\x00", b"NONE", 1, {"HARNESS"}, "R3 rc NUL after the LF"),
            ("success", b"0" + b"\x00" * 31, b"NONE", 1, {"HARNESS"}, "R3 rc 0 + 31 NUL (32 B)"),
            ("success", b"\x00" * 33, b"NONE", 1, {"HARNESS"}, "R3 rc 33 NUL (over 32 B)"),
            ("success", b"\x00", b"NONE", 1, {"HARNESS"}, "R3 rc a single NUL"),
            ("success", b"0\n", b"NONE\x00", 1, {"HARNESS"}, "R3 category NONE + NUL"),
            ("success", b"0\n", b"NO\x00NE", 1, {"HARNESS"}, "R3 category NUL inside"),
            ("success", b"0\n", b"\x00NONE\n", 1, {"HARNESS"}, "R3 category NUL first"),
            ("success", b"\xef\xbb\xbf0\n", b"NONE", 1, {"HARNESS"}, "R3 rc with a UTF-8 BOM"),
            ("success", b"0\n", b"\xef\xbb\xbfNONE", 1, {"HARNESS"}, "R3 category with a UTF-8 BOM"),
            ("success", b"0 \n", b"NONE", 1, {"HARNESS"}, "R3 rc with a trailing space"),
            ("success", b"0\n", b"NONE \n", 1, {"HARNESS"}, "R3 category with a trailing space"),
            ("success", b"0\n0\n", b"NONE", 1, {"HARNESS"}, "R3 rc on two lines"),
            ("success", b"0\n", b"NONE\nNONE", 1, {"HARNESS"}, "R3 category on two lines"),
            ("success", b"0\r", b"NONE", 1, {"HARNESS"}, "R3 rc with CR alone"),
            ("success", b"0\n", b"none", 1, {"HARNESS"}, "R3 category lower case"),
            ("failure", b"255\n", b"HARNESS", 1, {"HARNESS"}, "R3 POSITIVE rc 255 parses (HARNESS red)"),
            ("failure", b"137\n", b"PRODUCT", 1, {"PRODUCT"}, "R3 POSITIVE rc 137 with PRODUCT"),
        ]
        self.assertEqual(len(cells), 49)
        for outcome, rc_marker, category_marker, want, want_titles, label in cells:
            with self.subTest(cell=label):
                r = self.run_verdict(outcome, rc_marker, category_marker)
                out = r.stdout.decode("utf-8", "backslashreplace")
                errs = [line for line in out.split("\n") if line.startswith("::error")]
                titles = [line.split("::", 2)[1].rsplit(" ", 1)[-1] for line in errs]
                rogue = [line for line in out.split("\n") if line.startswith("::") and not line.startswith("::error title=")]
                self.assertEqual(r.returncode, want, quoted(r.stdout + r.stderr))
                self.assertEqual(set(titles), want_titles, quoted(r.stdout))
                self.assertEqual(len(titles), len(set(titles)), "at most one annotation per category\n" + quoted(r.stdout))
                self.assertTrue(all((("[PRODUCT" in line) + ("[HARNESS" in line)) == 1 for line in errs),
                                "one category per annotation\n" + quoted(r.stdout))
                self.assertTrue(all(len(line.split("::", 2)[2]) <= 600 for line in errs), quoted(r.stdout))
                self.assertEqual(rogue, [], "no other workflow command line\n" + quoted(r.stdout))
                if label == "R3 codex rc 30 00 0a":
                    self.assertIn("hex 30000a", out, "an unreadable marker is shown as its hex")

    def test_verdict_byte_sweep(self):
        """Every byte 0x00-0xff inserted at every position of a green marker and every byte replaced by every other value,
        the other marker held green, outcome success: exit 0 only for an allowed green form (rc 0 / 0 LF, NONE / NONE LF)."""
        green = {"rc": {b"0", b"0\n"}, "category": {b"NONE", b"NONE\n"}}
        base = {"rc": b"0\n", "category": b"NONE"}
        w = tempfile.mkdtemp(dir=self.tmp)
        d = os.path.join(w, "bootstrap-health")
        os.makedirs(d)
        script = os.path.join(w, "verdict.sh")
        with open(script, "w", encoding="utf-8") as stream:
            stream.write(self.verdict)
        env = {"PATH": "/usr/bin:/bin", "RUNNER_TEMP": w, "BH_OUTCOME": "success", "LANG": "en_US.UTF-8"}
        runs, greens, bad_green, bad_red = 0, 0, [], []
        for which in ("rc", "category"):
            other = "category" if which == "rc" else "rc"
            b0 = base[which]
            muts = [b0[:pos] + bytes([b]) + b0[pos:] for pos in range(len(b0) + 1) for b in range(256)]
            muts += [b0[:pos] + bytes([b]) + b0[pos + 1:] for pos in range(len(b0)) for b in range(256) if b != b0[pos]]
            for val in muts:
                with open(os.path.join(d, which), "wb") as stream:
                    stream.write(val)
                with open(os.path.join(d, other), "wb") as stream:
                    stream.write(base[other])
                r = subprocess.run([BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", script], cwd=w, env=env,
                                   capture_output=True, timeout=30)
                runs += 1
                greens += r.returncode == 0
                if r.returncode == 0 and val not in green[which]:
                    bad_green.append((which, val))
                if val in green[which] and r.returncode != 0:
                    bad_red.append((which, val))
        self.assertEqual(runs, 3578)
        self.assertEqual(bad_green, [], "a non-allowed marker form went green")
        self.assertEqual(bad_red, [], "an allowed green form went red")
        self.assertEqual(greens, 1, "exactly one mutation is an allowed green form (NONE followed by one LF)")


class BootstrapHealthReader(unittest.TestCase):
    """T4: the step-5 reader on 90 synthetic runner documents (v2 F1-F7, R2 G1-G15, R3 H1-H6, IMPL G16) with a byte-exact category
    marker oracle, and T5 probes P2-P4 on its cut functions."""
    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls.code = bh_reader_code(extract_step(WINDOWS_HEALTH, BH_STEP)["run"])
        cls.tmp = tempfile.mkdtemp(prefix="bh-reader-")
        cls.reader = os.path.join(cls.tmp, "reader.py")
        with open(cls.reader, "w", encoding="utf-8") as stream:
            stream.write(cls.code)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    @staticmethod
    def stream(text, utf8=True, alts=None, truncated=False):
        rec = {"bytes": len(text.encode("utf-8")), "sha256": "0" * 64, "hash_complete": True, "stable": True,
               "truncated": truncated, "head": {"utf8_valid": utf8, "text": text}}
        if alts:
            rec["head"]["alt"] = dict(alts)
        if truncated:
            rec["marker"] = "[… 123456 bytes dropped: stderr bytes 8192..131648 (kept 0..8192 and 131648..189000) …]"
            rec["tail"] = {"utf8_valid": utf8, "text": text, "alt": dict(alts or {})}
        return rec

    def fixtures(self):
        fix = []   # (name, document (dict / str / bytes) or None = not JSON, rc argument, reader rc, category marker or None)
        for o in range(560, 621):
            fix.append(("F1-pct-%d" % o, bh_doc(bh_rows(24) + [bh_hw(["PRODUCT"], {"PRODUCT": "a" * o + "%" * 40})]), "1", 1, "PRODUCT"))
        fix.append(("F2-flood", bh_doc(bh_rows(5) + [{"id": "F-%02d" % i, "status": "fail", "reason": "x" * 50, "fail_kind": "fail"}
                                                     for i in range(30)]), "1", 1, "PRODUCT"))
        fix.append(("F3-both", bh_doc(bh_rows(24) + [bh_hw(["PRODUCT", "HARNESS"])]), "1", 1, "HARNESS,PRODUCT"))
        fix.append(("F4-crash-row", bh_doc(bh_rows(23) + [bh_hw([]), {"id": "H-WIN-5", "status": "fail",
                                                                     "reason": "검체 크래시 KeyError: 'k'", "fail_kind": "crash"}]),
                    "1", 1, "HARNESS"))
        fix.append(("F5-pass0", bh_doc(bh_rows(1, "skip")), "1", 1, "HARNESS"))
        fix.append(("F6-unreadable", None, "1", 1, "HARNESS"))
        fix.append(("F7-multiline", bh_doc(bh_rows(24) + [{"id": "H-WIN-9", "status": "fail", "fail_kind": "fail",
                                                           "reason": "first\r\n::error::INJECTED-ROW\nthird 50%"}]), "1", 1, "PRODUCT"))
        fix.append(("G1-stale-parent-pass", bh_doc(bh_rows(25, "skip"), verdict="GREEN", **{"pass": 1, "skip": 24}), "0", 1, "HARNESS"))
        fix.append(("G2-consistent-skips", bh_doc(bh_rows(20) + [dict(x, id="S-%d" % n) for n, x in enumerate(bh_rows(5, "skip"))]),
                    "0", 0, "NONE"))
        fix.append(("G3-disabled-hidden", bh_doc(bh_rows(24) + [{"id": "H-WIN-24", "status": "disabled", "reason": "off"}],
                                                 **{"disabled": 0}), "0", 1, "HARNESS"))
        fix.append(("G4-rc-garbage", bh_doc(bh_rows(25)), "error0garbage", 1, "HARNESS"))
        for tag, arg in (("space", " 0"), ("plus", "+0"), ("zero-zero", "00"), ("arabic-indic", u"٠")):
            fix.append(("G5-rc-%s" % tag, bh_doc(bh_rows(25)), arg, 1, "HARNESS"))
        fix.append(("G6-rc0-unmeasured", bh_doc(bh_rows(25), verdict="UNMEASURED"), "0", 1, "HARNESS"))
        fix.append(("G7-fail-kind-on-pass", bh_doc(bh_rows(24) + [{"id": "H-WIN-25", "status": "pass", "detail": "ok",
                                                                   "fail_kind": "fail"}]), "0", 1, "HARNESS"))
        fix.append(("G8-pass-row-child-category", bh_doc(bh_rows(24) + [bh_hw(["PRODUCT"], status="pass")]), "0", 1, "HARNESS"))
        fix.append(("G9-category-unwritable", bh_doc(bh_rows(25)), "0", 1, None))
        fix.append(("G11-log-20000", bh_doc(bh_rows(24) + [bh_hw(["HARNESS"], child_extra={"stdout": self.stream("A" * 20000)})]),
                    "1", 1, "HARNESS"))
        fix.append(("G12-log-hangul", bh_doc(bh_rows(24) + [bh_hw(["HARNESS"], child_extra={"stderr": self.stream(u"한" * 10000)})]),
                    "1", 1, "HARNESS"))
        fix.append(("G13-log-two-views", bh_doc(bh_rows(24) + [bh_hw(["HARNESS"], child_extra={"stderr": self.stream(
            "\\x80" * 10000, utf8=False, alts={"cp949": u"가" * 13653, "cp1252": u"€" * 13653}, truncated=True)})]), "1", 1, "HARNESS"))
        fix.append(("G15-ann-7-digit-N", bh_doc(bh_rows(24) + [{"id": "H-WIN-9", "status": "fail", "fail_kind": "fail",
                                                                "reason": "a" * 1100000}]), "1", 1, "PRODUCT"))
        # G16 (B-344 IMPL, mutation RD2 survived the 89 design fixtures): every other green condition holds - 25 pass rows,
        # verdict GREEN, rc 0 - and only a stated count disagrees with the rows, so only the count reconciliation can make it red.
        fix.append(("G16-stale-parent-skip-count", bh_doc(bh_rows(25), **{"skip": 3}), "0", 1, "HARNESS"))
        p24 = ", ".join(json.dumps(r, ensure_ascii=False) for r in bh_rows(24))
        s25 = json.dumps(bh_doc(bh_rows(25))["summary"], ensure_ascii=False)
        fix.append(("H1-dup-row-status", '{"summary": %s, "specimens": [%s, {"id": "H-WIN-25", "status": "fail", "reason": "x", '
                    '"status": "pass"}]}' % (s25, p24), "0", 1, "HARNESS"))
        fix.append(("H2-dup-summary-verdict", '{"summary": {"verdict": "UNMEASURED", %s, "specimens": %s}'
                    % (s25[1:], json.dumps(bh_rows(25))), "0", 1, "HARNESS"))
        fix.append(("H3-dup-summary-pass", '{"summary": {"pass": 0, %s, "specimens": %s}' % (s25[1:], json.dumps(bh_rows(25))),
                    "0", 1, "HARNESS"))
        fix.append(("H4-nul-in-string", json.dumps(bh_doc(bh_rows(25))).replace('"detail": "ok"', '"detail": "o\\u0000k"', 1)
                    .encode().replace(b"\\u0000", b"\x00", 1), "0", 1, "HARNESS"))
        fix.append(("H5-utf8-bom", b"\xef\xbb\xbf" + json.dumps(bh_doc(bh_rows(25))).encode(), "0", 1, "HARNESS"))
        fix.append(("H6-invalid-utf8", json.dumps(bh_doc(bh_rows(25))).encode().replace(b'"detail": "ok"', b'"detail": "o\xffk"', 1),
                    "0", 1, "HARNESS"))
        return fix

    def test_fixtures(self):
        fix = self.fixtures()
        self.assertEqual(len(fix), 90)
        for name, d, rc_arg, want_rc, want_cat in fix:
            with self.subTest(fixture=name):
                w = tempfile.mkdtemp(dir=self.tmp)
                cwd, marker = os.path.join(w, "cwd"), os.path.join(w, "marker")
                os.makedirs(cwd)
                if name == "G9-category-unwritable":
                    with open(marker, "w") as stream:
                        stream.write("a regular file where the marker directory should be\n")
                else:
                    os.makedirs(marker)
                data = d if isinstance(d, bytes) else (d if isinstance(d, str) else (json.dumps(d, ensure_ascii=False)
                                                                                     if d is not None else "{not json")).encode("utf-8")
                with open(os.path.join(cwd, "win-health.json"), "wb") as stream:
                    stream.write(data)
                r = subprocess.run([sys.executable, self.reader, rc_arg, marker], cwd=cwd, capture_output=True, timeout=60)
                raw = r.stdout + r.stderr
                out = raw.decode("utf-8", "backslashreplace")
                lines = out.split("\n")
                errs = [line for line in lines if line.startswith("::error")]
                msgs = [line.split("::", 2)[2] if line.count("::") >= 2 else "" for line in errs]
                mk = os.path.join(marker, "category")
                cat = bh_read_bytes(mk)
                blocks = bh_blocks(r.stdout)
                self.assertEqual(r.returncode, want_rc, quoted(raw))
                self.assertLessEqual(len(errs), 10)
                self.assertTrue(all(len(m) <= 600 for m in msgs), "an annotation over 600 characters")
                self.assertFalse(any(re.search(r"%(?![0-9A-F]{2})", m) for m in msgs), "a split %XX escape")
                self.assertEqual(cat, None if want_cat is None else want_cat.encode("utf-8"), "category marker (bytes)")
                self.assertFalse(any("[PRODUCT" in m and "[HARNESS" in m for m in msgs), "an annotation with both categories")
                self.assertTrue(all(v <= BH_CAP for v in blocks.values()), "a stream block over 16 KiB: %s" % blocks)
                if name.startswith("F1"):
                    self.assertTrue(all("…(+" in m for m in msgs if "aaaa" in m), "the cut says how much was cut")
                if name == "F2-flood":
                    self.assertIn("annotation cap: 21 more", out)
                if name == "F3-both":
                    self.assertEqual(sum(line.startswith("::error title=H-WIN-11 PRODUCT::") for line in errs), 1)
                    self.assertEqual(sum(line.startswith("::error title=H-WIN-11 HARNESS::") for line in errs), 1)
                if name == "F7-multiline":
                    self.assertFalse(any(line.startswith("::error::INJECTED") for line in lines))
                if name.startswith(("G1-", "G3-", "G4-", "G5-", "G6-", "G7-", "G8-", "G9-", "G16-", "H")):
                    self.assertTrue(any(m.startswith("[HARNESS") for m in msgs), "a [HARNESS ...] fail-closed error from the reader itself")
                if name in ("G11-log-20000", "G12-log-hangul", "G13-log-two-views"):
                    self.assertIn("bytes of the log copy dropped here", out)
                    self.assertNotIn(u"�", raw.decode("utf-8", "replace"), "the cut introduced U+FFFD")

    def test_probe_p2_log_copy_cut_holds_its_cap_with_the_notice(self):
        part = bh_reader_funcs(self.code, ["_part"])["_part"]
        out = part("A" * 20000, "stdout", 16384)
        self.assertLessEqual(len(out.encode("utf-8")), 16384)
        self.assertIn("bytes of the log copy dropped here", out)
        over, n = [], 0
        for cap in (16384, 8192, 4096, 4000, 1024):
            for unit in ("A", u"한", u"a한€"):
                for size in list(range(cap - 8, cap + 300)) + [cap * 2, cap * 10, 1 << 20]:
                    t = (unit * (size // len(unit.encode("utf-8")) + 1)).encode("utf-8")[:size].decode("utf-8", "ignore")
                    b = len(part(t, "stderr as cp949", cap).encode("utf-8"))
                    n += 1
                    if b > cap:
                        over.append((cap, unit, size, b))
        self.assertEqual(n, 4665)
        self.assertEqual(over, [])

    def test_probe_p3_whole_stream_block_holds_16_kib(self):
        ns = bh_reader_funcs(self.code, ["_part", "stream_block", "nb", "clip"])
        worst, n = 0, 0
        for size in list(range(15500, 17500, 7)) + [65536, 1 << 20]:
            for alts in (None, ("cp949",), ("cp949", "cp1252"), ("a", "b", "c")):
                rec = {"bytes": size, "sha256": "f" * 64, "hash_complete": True, "stable": True, "truncated": True,
                       "marker": "[… %d bytes dropped: stderr bytes 8192..%d (kept 0..8192 and %d..%d) …]" % (size, size, size, size),
                       "head": {"text": u"한" * (size // 3)}, "tail": {"text": "x" * size}}
                if alts:
                    rec["head"]["alt"] = dict((k, u"가" * (size // 3)) for k in alts)
                    rec["tail"]["alt"] = dict((k, u"€" * (size // 3)) for k in alts)
                worst = max(worst, sum(ns["nb"](x) for x in ns["stream_block"]("stderr", rec)))
                n += 1
        self.assertEqual(n, 1152)
        self.assertLessEqual(worst, ns["LOG_STREAM_MAX"])
        self.assertEqual(ns["LOG_STREAM_MAX"], BH_CAP)

    def test_probe_p4_annotation_cut_never_exceeds_600(self):
        cut = bh_reader_funcs(self.code, ["cut_ann"])["cut_ann"]
        for n in (10 ** 5, 999990, 10 ** 6 - 1, 10 ** 6, 10 ** 6 + 1, 3 * 10 ** 6):
            with self.subTest(n=n):
                self.assertLessEqual(len(cut("a" * n)), 600)


if __name__ == "__main__":
    # 한글·깨진 바이트(U+FFFD)를 찍다가 러너의 출력 인코딩 때문에 검체가 죽지 않게 한다(찍는 것은 표시일 뿐 판정이 아니다).
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(errors="backslashreplace")
        except Exception:
            pass
    if SHOW:
        try:
            ver = subprocess.run([BASH, "--version"], capture_output=True, text=True, timeout=30).stdout.split("\n")[0]
        except Exception as e:   # 표시용일 뿐이다 — 판정과 무관
            ver = "버전 조회 실패: %s" % e
        print("[대상] %s · 스텝 `%s`\n[셸] %s — %s" % (WORKFLOW, STEP_NAME, BASH, ver), flush=True)
    # This already-wired CI entry also executes the Unix PTY result gate's
    # negative controls; adding a scripts/tests file alone is not discovery.
    from test_unix_pty_results import OutputControls, WorkflowControls
    unittest.main()
