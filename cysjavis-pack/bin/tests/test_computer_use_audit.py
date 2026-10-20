#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_computer_use_audit.py — javis_computer_use.py(computer-use 탑재 감사) exit 계약 회귀 (B-328 #6 · 2026-10-20).

## 무엇을 못박는가
도구의 사실은 exit code 다(도구 머리말 "결정론 판정"). 실제 CLI 를 서브프로세스로 돌려 각 갈래를 한 번씩 낸다.
  A0 OK          발견된 Claude 좌석 cwd 가 전부 탑재 = 0 · 살아 있는 Claude 좌석 0 = 0(좌석없음)
                 · Claude 가 아닌 좌석(codex·gemini)은 판정 대상이 아니다 = 0
  A1 GAP         미탑재 cwd 가 하나라도 있으면 1 · ★기본값은 감사뿐 — --fix 없이는 .claude.json 바이트가 그대로다
                 · 부서 레인은 그 부서 데몬 소켓(HOME/.local/state/cys-dept-dept-<N>/cys.sock)으로 묻는다
  F  --fix       미탑재 cwd 에 enabledMcpServers 를 메우고 재독 검증 뒤 0 · 기존 항목·다른 키·파일 권한 보존
                 · 임시 파일을 남기지 않는다 · 이미 탑재된 레인은 쓰지 않는다 · Claude 가 아닌 좌석 cwd 는 쓰지 않는다
  U2 판정불가    .claude.json 판독 실패 = 2(--fix 여도 쓰지 않는다) · `cys list` 실패 = 2 · PATH 에 cys 없음 = 2
  C  호출 범위   도구가 띄우는 외부 명령은 `cys list` 하나뿐이다(스텁 호출 기록으로 잰다 — 프로세스 제어·
                 mcp__computer-use 호출 없음)
  D1 base lane   Run from a dept-like env (the caller's socket variable names the dept daemon - CYS_SOCKET, or only a
                 legacy JAVIS_SOCKET / AITERM_SOCKET, or an empty CYS_SOCKET beside a legacy one), the base lane still
                 asks the DEFAULT socket and --fix writes no dept cwd into the base .claude.json (PROMOTE-6-D1FIX).
  P0 producer    The stub is not a hand copy of `cys list`: its rows, the seat title and its socket-resolution order are
                 DERIVED at test time from the producer source (src/bin/cys.rs Command::List println and workflow_title,
                 src/lib.rs surface_ref and env_compat). When the producer moves, the derived rows move with it and the
                 tool is exercised against the moved format; an unreadable producer is a failure, never a skip. The
                 tool's socket-key tuple must equal javis_role.SOCKET_ENV_KEYS and the env_compat order.
                 Rendering is derived too, in both stages: the daemon's surface.list value for each printed key
                 (src/bin/cysd/handlers.rs, typed by the Surface struct in src/bin/cysd/state.rs) and the CLI's println
                 argument (`s["k"].as_str().unwrap_or("d")` = the raw string, `s["k"]` = serde_json display). Any other
                 expression in either stage is rejected as a moved producer (PROMOTE-6 R1).
  Registered coupling (not derivable from source, not faked): a seat's title can be changed at run time by the
                 surface.rename RPC (src/bin/cysd/handlers.rs "surface.rename"), and the tool reads the agent from the
                 title's "<role>-<agent>" prefix. Blast radius: a renamed Claude seat is judged non-Claude (its cwd is
                 never audited or fixed - a silent miss); a non-Claude seat renamed "<x>-claude · y" is judged Claude
                 (--fix would enable computer-use for its cwd). The rows here assume the launch title (workflow_title).

## 밀폐
- HOME·cwd 는 케이스마다 새 임시 디렉터리다. 라이브 설정(~/.cys/claude*)·라이브 팩·데몬은 읽지도 쓰지도 않는다.
- 자식 PATH 는 스텁 cys(임시 폴더의 sh 스크립트 · 정해 둔 `cys list` 행만 출력) + /usr/bin:/bin 이다.
  부모의 CLAUDE_CONFIG_DIR·CYS_*·타사 프로바이더 env 는 자식에 넘기지 않는다. The D1 cases set a socket variable
  on purpose, and it names a path inside the temp dir; only the stub cys ever sees it.
- 체크아웃에는 쓰지 않는다.

## 한계
- 스텁 cys 가 sh 스크립트라 Windows 에서는 전부 건너뛴다(사유가 찍힌다). 이 테스트는 비-Windows 레인에만 등록된다.
- The rows follow the producer's SOURCE, not a running daemon. Assumed, not measured: serde_json's compact display of a
  value equals json.dumps(ensure_ascii=False, separators=(",", ":")) for the plain values used here.
- 살아 있는 노드 프로세스의 실제 적재 여부는 도구도 이 테스트도 재지 않는다(도구 머리말 "계측의 유효 범위").

실행: CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_computer_use_audit.py
"""
import ast
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(TESTS_DIR)
ROOT = os.path.dirname(os.path.dirname(BIN))
TOOL = os.path.join(BIN, "javis_computer_use.py")
ROLE = os.path.join(BIN, "javis_role.py")
CYS_RS = os.path.join(ROOT, "src", "bin", "cys.rs")
LIB_RS = os.path.join(ROOT, "src", "lib.rs")
HANDLERS_RS = os.path.join(ROOT, "src", "bin", "cysd", "handlers.rs")
STATE_RS = os.path.join(ROOT, "src", "bin", "cysd", "state.rs")
SRV = "computer-use"
DEPT = "7"


class ProducerMoved(AssertionError):
    pass


def _read(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return fh.read()
    except OSError as e:
        raise ProducerMoved("producer source unreadable: %s (%s) - the stub cannot be derived" % (path, e))


def _rust_str(lit):
    """Decode the few escapes a Rust format literal here may hold; anything else means the producer moved."""
    out, i = [], 0
    while i < len(lit):
        ch = lit[i]
        if ch == "\\":
            nxt = lit[i + 1:i + 2]
            if nxt not in ("t", "n", "\\", '"'):
                raise ProducerMoved("unexpected escape \\%s in a producer format literal" % nxt)
            out.append({"t": "\t", "n": "\n", "\\": "\\", '"': '"'}[nxt])
            i += 2
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def _split_args(text):
    """Top-level comma split of a Rust argument list (quotes and brackets respected); a trailing comma is allowed."""
    out, depth, cur, i, q = [], 0, [], 0, False
    while i < len(text):
        ch = text[i]
        if q:
            cur.append(ch)
            if ch == "\\":
                cur.append(text[i + 1]); i += 1
            elif ch == '"':
                q = False
        elif ch == '"':
            q = True; cur.append(ch)
        elif ch in "([{":
            depth += 1; cur.append(ch)
        elif ch in ")]}":
            depth -= 1; cur.append(ch)
        elif ch == "," and depth == 0:
            out.append("".join(cur).strip()); cur = []
        else:
            cur.append(ch)
        i += 1
    if "".join(cur).strip():
        out.append("".join(cur).strip())
    return out


# How the CLI renders a printed argument: the raw string, or serde_json's display of the value. Nothing else is accepted.
_ARG_STR = re.compile(r's\["(\w+)"\]\.as_str\(\)\.unwrap_or\("((?:[^"\\]|\\.)*)"\)')
_ARG_JSON = re.compile(r's\["(\w+)"\]')
# How the daemon makes each printed value in surface.list: a recognised expression over one Surface field (typed by the
# struct) or the surface_ref() call. Any other expression is a moved producer.
_DAEMON_EXPR = re.compile(r'(?:cys::)?surface_ref\(s\.id\)|s\.(\w+)(?:\.lock\(\)\.unwrap\(\)\.clone\(\)|\.load\(Ordering::Relaxed\))?')
_RUST_KIND = {"String": "str", "Mutex<String>": "str", "Option<String>": "str|null", "Mutex<Option<String>>": "str|null",
              "u32": "int", "u64": "int", "AtomicBool": "bool"}


def _daemon_kinds(keys):
    """JSON kind of each printed key as the daemon builds it in the surface.list arm (handlers.rs + Surface in state.rs)."""
    h, st = _read(HANDLERS_RS), _read(STATE_RS)
    i = h.find('"surface.list" => {')
    j = h.find("json!({", i) if i >= 0 else -1
    if j < 0:
        raise ProducerMoved("src/bin/cysd/handlers.rs: surface.list payload not found")
    body = h[j:h.find("})", j)]
    m = re.search(r"pub struct Surface \{(.*?)\n\}", st, re.S)
    if not m:
        raise ProducerMoved("src/bin/cysd/state.rs: struct Surface not found")
    ftype = dict(re.findall(r"pub (\w+): ([^,\n]+),", m.group(1)))
    kinds = {}
    for k in keys:
        e = re.search(r'"%s": ([^,\n]+),' % re.escape(k), body)
        if not e:
            raise ProducerMoved("src/bin/cysd/handlers.rs: surface.list has no %r value" % k)
        expr = e.group(1).strip()
        d = _DAEMON_EXPR.fullmatch(expr)
        if not d:
            raise ProducerMoved("src/bin/cysd/handlers.rs: surface.list %r is made by an unrecognised expression %r" % (k, expr))
        if d.group(1) is None:
            if k != "surface_ref":
                raise ProducerMoved("src/bin/cysd/handlers.rs: surface.list %r is made by surface_ref()" % k)
            kinds[k] = "str"                                  # surface_ref(id) -> String (src/lib.rs)
        else:
            if d.group(1) != k:                               # the printed key must be that Surface field, not another
                raise ProducerMoved("src/bin/cysd/handlers.rs: surface.list %r is read from Surface.%s" % (k, d.group(1)))
            t = ftype.get(d.group(1), "").strip()
            if t not in _RUST_KIND:
                raise ProducerMoved("src/bin/cysd/state.rs: Surface.%s has type %r - its JSON form is not derived" % (d.group(1), t))
            kinds[k] = _RUST_KIND[t]
    return kinds


_PRODUCER = None


def producer():
    """What the real producer prints, read from its source: the `cys list` row template and argument keys, the seat
    title format, the surface ref format, and the socket names cys reads (in order, and whether empty is skipped)."""
    global _PRODUCER
    if _PRODUCER is not None:
        return _PRODUCER
    cys, lib = _read(CYS_RS), _read(LIB_RS)
    m = re.search(r'Command::List\s*=>\s*request\("surface\.list".*?println!\(\s*"((?:[^"\\]|\\.)*)",(.*?)\);',
                  cys, re.S)
    if not m:
        raise ProducerMoved("src/bin/cys.rs: Command::List println not found")
    row_fmt, keys, render = _rust_str(m.group(1)), [], []
    for arg in _split_args(m.group(2)):
        a, b = _ARG_STR.fullmatch(arg), _ARG_JSON.fullmatch(arg)
        if a:
            keys.append(a.group(1)); render.append(("str", _rust_str(a.group(2))))
        elif b:
            keys.append(b.group(1)); render.append(("json", None))
        else:
            raise ProducerMoved("src/bin/cys.rs: Command::List renders an argument by an unrecognised expression %r" % arg)
    if row_fmt.count("{}") != len(keys) or re.search(r"\{[^}]", row_fmt.replace("{}", "")):
        raise ProducerMoved("src/bin/cys.rs: Command::List format %r does not take exactly %d plain {} fields" % (row_fmt, len(keys)))
    m = re.search(r'fn workflow_title\(.*?\.map\(\|folder\| format!\("([^"]*)"\)\)\s*'
                  r'\.unwrap_or_else\(\|\| format!\("([^"]*)"\)\)', cys, re.S)
    if not m:
        raise ProducerMoved("src/bin/cys.rs: workflow_title formats (with folder / without) not found")
    title_fmt, title_bare_fmt = _rust_str(m.group(1)), _rust_str(m.group(2))
    m = re.search(r'pub fn surface_ref\(id: u64\) -> String \{\s*format!\("([^"]*)"\)', lib)
    if not m:
        raise ProducerMoved("src/lib.rs: surface_ref format not found")
    ref_fmt = _rust_str(m.group(1))
    m = re.search(r'pub fn env_compat\(primary: &str\) -> Option<String> \{(.*?)\n\}', lib, re.S)
    p = re.search(r'pub const ENV_SOCKET: &str = "(\w+)";', lib)
    if not (m and p):
        raise ProducerMoved("src/lib.rs: env_compat or ENV_SOCKET not found")
    body, primary = m.group(1), p.group(1)
    alias = {v: primary.replace(a, b, 1)
             for v, a, b in re.findall(r'let (\w+) = primary\.replacen\("(\w+)", "(\w+)", 1\);', body)}
    a = re.search(r'\[primary((?:, \w+\.as_str\(\))*)\]', body)
    if not a:
        raise ProducerMoved("src/lib.rs: env_compat lookup order not found")
    order = [primary] + [alias[v] for v in re.findall(r'(\w+)\.as_str\(\)', a.group(1))]
    if "!v.is_empty()" not in body:
        raise ProducerMoved("src/lib.rs: env_compat no longer skips empty values - the stub's resolution would be wrong")
    _PRODUCER = {"row_fmt": row_fmt, "keys": keys, "render": render, "kinds": _daemon_kinds(keys),
                 "title_fmt": title_fmt, "title_bare_fmt": title_bare_fmt, "ref_fmt": ref_fmt, "socket_keys": order}
    return _PRODUCER


def stub_script(socket_keys):
    """Stub cys: logs `<resolved socket or base>|<argv>` and prints the rows the rig wrote for that socket. It resolves
    the socket the way src/lib.rs env_compat does - the first NON-EMPTY of the derived names."""
    names = " ".join(socket_keys)
    return r"""#!/bin/sh
key=""
for k in %s; do
  eval "v=\${$k:-}"
  if [ -n "$v" ]; then key="$v"; break; fi
done
printf '%%s|%%s\n' "${key:-base}" "$*" >> "$RIG_LOG"
[ -n "$key" ] || key="$RIG_DIR/base"
[ "$1" = "list" ] || exit 64
[ -e "$key.fail" ] && exit 1
[ -f "$key.list" ] && cat "$key.list"
exit 0
""" % names


def sha(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def tuple_in(path, name):
    """A module-level tuple of string constants, read with ast (no import)."""
    with open(path, encoding="utf-8") as fh:
        tree = ast.parse(fh.read())
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == name for t in node.targets):
            return tuple(ast.literal_eval(node.value))
    return None


@unittest.skipIf(os.name == "nt", "Windows: 스텁 cys 가 sh 스크립트라 건너뛴다(비-Windows 레인 전용)")
class Rig(unittest.TestCase):
    """임시 HOME(base + dept-7 설정) + 스텁 cys — 케이스마다 새로 만든다."""

    def setUp(self):
        self.assertTrue(os.path.isfile(TOOL), "도구 부재: %s" % TOOL)
        self.prod = producer()
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="cu-audit-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.rigdir = os.path.join(self.tmp, "rig")
        self.stubbin = os.path.join(self.tmp, "stubbin")
        self.log = os.path.join(self.tmp, "cys-calls.log")
        for d in (self.home, self.rigdir, self.stubbin):
            os.makedirs(d)
        self.base_cfg = os.path.join(self.home, ".cys", "claude", ".claude.json")
        self.dept_cfg = os.path.join(self.home, ".cys", "claude-default-dept-%s" % DEPT, ".claude.json")
        self.dept_sock = os.path.join(self.home, ".local", "state", "cys-dept-dept-%s" % DEPT, "cys.sock")
        os.makedirs(os.path.dirname(self.dept_sock))
        self.stub = os.path.join(self.stubbin, "cys")
        with open(self.stub, "w", encoding="utf-8") as fh:
            fh.write(stub_script(self.prod["socket_keys"]))
        os.chmod(self.stub, 0o755)
        # 기본: 두 레인 모두 좌석 없음, 설정은 탑재 항목 없이 판독 가능.
        self.write_cfg(self.base_cfg, {})
        self.write_cfg(self.dept_cfg, {})

    # ── 리그 구성 ──
    def write_cfg(self, path, projects, extra=None):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        cfg = {"oauthAccount": {"organizationType": "claude_max"}, "numStartups": 3,
               "projects": projects}
        cfg.update(extra or {})
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(cfg, fh, ensure_ascii=False, indent=2)
        os.chmod(path, 0o644)

    def row(self, sid, role, agent, cwd):
        """One `cys list` row built from the producer's own templates and rendering (see producer())."""
        # workflow_title: the last path segment, or the bare "<role>-<agent>" form when there is none (a root cwd)
        folder = cwd.rstrip("/\\").replace("\\", "/").rsplit("/", 1)[-1]
        if folder and not (len(folder) == 2 and folder[1] == ":" and folder[0].isalpha()):
            title = self.prod["title_fmt"].format(role=role, agent=agent, folder=folder)
        else:
            title = self.prod["title_bare_fmt"].format(role=role, agent=agent)
        vals = {"surface_ref": self.prod["ref_fmt"].format(id=sid), "role": role, "pid": 4000 + sid, "exited": False,
                "title": title, "cwd": cwd}
        kind_of = {str: "str", int: "int", bool: "bool"}
        out = []
        for k, (how, default) in zip(self.prod["keys"], self.prod["render"]):
            if k not in vals:
                raise ProducerMoved("src/bin/cys.rs: Command::List prints field %r the stub cannot fill" % k)
            v = vals[k]
            if kind_of[type(v)] not in self.prod["kinds"][k].split("|"):
                raise ProducerMoved("daemon makes %r as %s, the stub has %s" % (k, self.prod["kinds"][k], kind_of[type(v)]))
            if how == "str":
                out.append(v if isinstance(v, str) else default)
            else:                                   # serde_json display of a Value: compact JSON
                out.append(json.dumps(v, ensure_ascii=False, separators=(",", ":")))
        parts = self.prod["row_fmt"].split("{}")
        return "".join(p + x for p, x in zip(parts, out)) + parts[-1]

    def seats(self, lane, rows):
        """rows = [(sid, role, agent, cwd)] — written in the producer's row format."""
        key = os.path.join(self.rigdir, "base") if lane == "base" else self.dept_sock
        with open(key + ".list", "w", encoding="utf-8") as fh:
            for sid, role, agent, cwd in rows:
                fh.write(self.row(sid, role, agent, cwd) + "\n")

    def fail_list(self, lane):
        key = os.path.join(self.rigdir, "base") if lane == "base" else self.dept_sock
        open(key + ".fail", "w").close()

    def env(self, path=None, extra=None):
        e = {k: v for k, v in os.environ.items()
             if not k.startswith(("CYS_", "CLAUDE_", "ANTHROPIC_", "JAVIS_", "AITERM_"))}
        e.update({"HOME": self.home, "PATH": path or (self.stubbin + os.pathsep + "/usr/bin:/bin"),
                  "PYTHONUTF8": "1", "RIG_LOG": self.log, "RIG_DIR": self.rigdir})
        e.update(extra or {})
        return e

    def run_tool(self, *argv, path=None, extra=None):
        return subprocess.run([sys.executable, "-I", "-B", TOOL] + list(argv), cwd=self.tmp,
                              env=self.env(path, extra), capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=60)

    def rc(self, want, r, what):
        self.assertEqual(r.returncode, want, "%s: rc=%d (기대 %d)\n--stdout--\n%s\n--stderr--\n%s"
                         % (what, r.returncode, want, r.stdout, r.stderr))

    def enabled(self, path, cwd):
        with open(path, encoding="utf-8") as fh:
            cfg = json.load(fh)
        return ((cfg.get("projects") or {}).get(cwd) or {}).get("enabledMcpServers")

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log, encoding="utf-8") as fh:
            return [ln.rstrip("\n") for ln in fh if ln.strip()]


class A0Ok(Rig):
    def test_all_mounted_is_0(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
        self.write_cfg(self.dept_cfg, {"/w/d7": {"enabledMcpServers": ["other", SRV]}})
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        self.seats("dept", [(2, "master", "claude", "/w/d7"), (3, "worker", "claude", "/w/d7")])
        r = self.run_tool()
        self.rc(0, r, "전부 탑재")
        self.assertIn("cwd 2개 전부 탑재", r.stdout)

    def test_no_live_claude_seat_is_0(self):
        r = self.run_tool()
        self.rc(0, r, "좌석 0")
        self.assertEqual(r.stdout.count("좌석없음"), 2, r.stdout)

    def test_non_claude_seats_are_not_judged(self):
        self.seats("base", [(4, "worker", "codex", "/w/codex"), (5, "reviewer", "gemini", "/w/gem")])
        r = self.run_tool()
        self.rc(0, r, "Claude 아닌 좌석만")
        self.assertNotIn("/w/codex", r.stdout)


class A1Gap(Rig):
    def test_gap_is_1_and_audit_writes_nothing(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": ["other"]}})
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        before = (sha(self.base_cfg), sha(self.dept_cfg))
        r = self.run_tool()
        self.rc(1, r, "미탑재 1")
        self.assertIn("미탑재", r.stdout)
        self.assertIn("/w/base", r.stdout)
        self.assertEqual((sha(self.base_cfg), sha(self.dept_cfg)), before, "감사(무 --fix)가 설정을 썼다")

    def test_dept_lane_asks_its_own_socket(self):
        self.seats("dept", [(9, "worker", "claude", "/w/d7")])
        r = self.run_tool()
        self.rc(1, r, "부서 레인 미탑재")
        self.assertIn("dept-%s" % DEPT, r.stdout)
        self.assertIn("%s|list" % self.dept_sock, self.calls())

    def test_seat_whose_title_has_no_folder_is_still_claude(self):
        # a root cwd gives workflow_title's bare "<role>-<agent>" form - the agent must still be read from it
        self.seats("base", [(1, "worker", "claude", "/")])
        r = self.run_tool()
        self.rc(1, r, "bare-title Claude seat")
        self.assertTrue(any(ln.split()[:3] == ["미탑재", "base", "/"] for ln in r.stdout.splitlines()), r.stdout)

    def test_missing_projects_entry_is_a_gap(self):
        self.write_cfg(self.base_cfg, {"/elsewhere": {"enabledMcpServers": [SRV]}})
        self.seats("base", [(1, "worker", "claude", "/w/new")])
        self.rc(1, self.run_tool(), "새 작업 폴더")


class FFix(Rig):
    def test_fix_fills_gap_and_preserves(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": ["other"], "allowedTools": ["x"]}},
                       extra={"keep": {"a": 1}})
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        self.seats("dept", [(2, "worker", "claude", "/w/d7")])
        r = self.run_tool("--fix")
        self.rc(0, r, "--fix 뒤")
        self.assertEqual(self.enabled(self.base_cfg, "/w/base"), ["other", SRV])
        self.assertEqual(self.enabled(self.dept_cfg, "/w/d7"), [SRV])
        with open(self.base_cfg, encoding="utf-8") as fh:
            cfg = json.load(fh)
        self.assertEqual(cfg["keep"], {"a": 1})
        self.assertEqual(cfg["projects"]["/w/base"]["allowedTools"], ["x"])
        self.assertEqual(cfg["oauthAccount"], {"organizationType": "claude_max"})
        for p in (self.base_cfg, self.dept_cfg):
            self.assertEqual(stat.S_IMODE(os.stat(p).st_mode), 0o644, "권한 보존 실패: %s" % p)
            left = [n for n in os.listdir(os.path.dirname(p)) if n.startswith(".claude.json.cu-")]
            self.assertEqual(left, [], "임시 파일 잔존")
        self.rc(0, self.run_tool(), "--fix 뒤 재감사")

    def test_fix_leaves_mounted_lane_and_non_claude_cwd_untouched(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        self.seats("dept", [(2, "worker", "claude", "/w/d7"), (3, "worker", "codex", "/w/codex")])
        base_before = sha(self.base_cfg)
        self.rc(0, self.run_tool("--fix"), "--fix")
        self.assertEqual(sha(self.base_cfg), base_before, "이미 탑재된 레인을 다시 썼다")
        self.assertIsNone(self.enabled(self.dept_cfg, "/w/codex"), "Claude 아닌 좌석 cwd 를 썼다")


class U2Unreadable(Rig):
    def test_unreadable_config_is_2_even_with_fix(self):
        with open(self.base_cfg, "w", encoding="utf-8") as fh:
            fh.write("{not json")
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        before = sha(self.base_cfg)
        r = self.run_tool("--fix")
        self.rc(2, r, "판독 불가 설정")
        self.assertIn("판정불가", r.stdout)
        self.assertEqual(sha(self.base_cfg), before)

    def test_cys_list_failure_is_2(self):
        self.fail_list("dept")
        self.rc(2, self.run_tool(), "cys list 실패")

    def test_no_cys_on_path_is_2(self):
        nobin = os.path.join(self.tmp, "nobin")
        os.makedirs(nobin)
        self.assertIsNone(shutil.which("cys", path=nobin))
        self.rc(2, self.run_tool(path=nobin), "cys 부재")


class CCalls(Rig):
    def test_only_cys_list_is_spawned(self):
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        self.seats("dept", [(2, "worker", "claude", "/w/d7")])
        self.run_tool("--fix")
        got = self.calls()
        self.assertEqual(sorted(got), sorted(["base|list", "%s|list" % self.dept_sock]), got)


class D1BaseLaneIgnoresTheCallersSocket(Rig):
    """D1: from a dept seat the caller's env names the dept daemon. The base lane must still ask the default socket,
    so the dept's Claude seat is judged (and fixed) only against the dept config."""

    def dept_like_envs(self):
        legacy = [k for k in self.prod["socket_keys"] if k != "CYS_SOCKET"]
        cases = [("CYS_SOCKET", {"CYS_SOCKET": self.dept_sock})]
        cases += [(k + " only", {k: self.dept_sock}) for k in legacy]
        cases += [("empty CYS_SOCKET + " + k, {"CYS_SOCKET": "", k: self.dept_sock}) for k in legacy]
        return cases

    def test_fix_from_a_dept_like_env_writes_no_dept_cwd_into_the_base_config(self):
        self.assertEqual(shutil.which("cys", path=self.stubbin + os.pathsep + "/usr/bin:/bin"), self.stub,
                         "the stub must be the cys the tool finds")
        for what, extra in self.dept_like_envs():
            with self.subTest(env=what):
                self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
                self.write_cfg(self.dept_cfg, {})
                self.seats("base", [(1, "worker", "claude", "/w/base")])
                self.seats("dept", [(2, "worker", "claude", "/w/d7")])
                if os.path.exists(self.log):
                    os.unlink(self.log)
                base_before = sha(self.base_cfg)
                r = self.run_tool("--fix", extra=extra)
                self.rc(0, r, "--fix from %s" % what)
                self.assertIsNone(self.enabled(self.base_cfg, "/w/d7"),
                                  "%s: the dept cwd was written into the BASE config\n%s" % (what, r.stdout))
                self.assertEqual(sha(self.base_cfg), base_before, "%s: the base config changed" % what)
                self.assertEqual(self.enabled(self.dept_cfg, "/w/d7"), [SRV], "%s: dept lane not fixed" % what)
                self.assertEqual(sorted(self.calls()), sorted(["base|list", "%s|list" % self.dept_sock]),
                                 "%s: the base lane did not ask the default socket" % what)

    def test_audit_from_a_dept_like_env_reports_no_dept_seat_under_base(self):
        self.write_cfg(self.base_cfg, {"/w/base": {"enabledMcpServers": [SRV]}})
        self.write_cfg(self.dept_cfg, {"/w/d7": {"enabledMcpServers": [SRV]}})
        self.seats("base", [(1, "worker", "claude", "/w/base")])
        self.seats("dept", [(2, "worker", "claude", "/w/d7")])
        for what, extra in self.dept_like_envs():
            with self.subTest(env=what):
                r = self.run_tool(extra=extra)
                self.rc(0, r, "audit from %s" % what)
                base_rows = [ln for ln in r.stdout.splitlines() if ln.split()[1:2] == ["base"]]
                self.assertTrue(base_rows and all("/w/d7" not in ln for ln in base_rows),
                                "%s: base lane rows %r" % (what, base_rows))


@unittest.skipIf(os.name == "nt", "Windows: 비-Windows 레인 전용(같은 테스트 파일)")
class P0ProducerCoupling(unittest.TestCase):
    """The tool's socket names, the pack's convention and the producer's lookup order are one list."""

    def test_socket_key_tuple_matches_the_pack_and_the_producer(self):
        prod = producer()
        tool = tuple_in(TOOL, "SOCKET_ENV_KEYS")
        role = tuple_in(ROLE, "SOCKET_ENV_KEYS")
        self.assertIsNotNone(tool, "javis_computer_use.SOCKET_ENV_KEYS missing")
        self.assertEqual(tool, role, "tool tuple != javis_role.SOCKET_ENV_KEYS")
        self.assertEqual(list(tool), prod["socket_keys"], "tool tuple != src/lib.rs env_compat order")

    def test_derived_rows_parse_as_the_tool_expects(self):
        prod = producer()
        self.assertIn("cwd", prod["keys"], "src/bin/cys.rs: Command::List no longer prints cwd")
        self.assertIn("title", prod["keys"], "src/bin/cys.rs: Command::List no longer prints the title")


if __name__ == "__main__":
    unittest.main(verbosity=2)
