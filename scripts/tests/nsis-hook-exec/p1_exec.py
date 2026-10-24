#!/usr/bin/env python3
"""WP12 P1 executor — runs the REAL P1 macro text of src-tauri/nsis-hooks.nsh against simulated
Windows outcomes (WP12 B1 r2: codex r1 B1-F1..F4).

    python3 scripts/tests/nsis-hook-exec/p1_exec.py [--hook PATH] [--case ID] [--json OUT]
      exit 0 = every case passed, 1 = a case failed, 2 = the executor could not run the hook
      (an instruction it does not implement, an undefined label/variable, a !error reached)

Why it exists. The compile harness (../nsis-hook-compile/run.sh) proves the hook compiles and its
compile-time guards fire; the model (../nsis-hook-model/model.py) mirrors the POSTINSTALL state
machine by hand and pins the P1 STRUCTURE. Neither executes the P1 macros, so a register that
overwrites its own input (B1-F3), an unbounded process query (B1-F2), an unproven termination
(B1-F1) or an unchecked record write (B1-F4) passes both. This file parses the hook's macros and
functions, expands !insertmacro / !if exactly as written (parameters substituted as text), and
interprets the resulting instructions one by one.

What is real and what is simulated.
  real      every P1 instruction of the hook as written (labels, jumps, registers, the error flag,
            StrCpy/StrCmp/StrCmpS/IntOp/IntCmp, File* line handling), and the two library
            functions it calls, ported from their NSIS 3.13 sources: ${StrLoc} (Include/StrFunc.nsh,
            case-insensitive, first match from the left) and ${GetOptions} (Include/FileFunc.nsh).
            Instruction semantics follow Source/exehead/exec.c of NSIS 3.13 (StrCpy length/offset
            rules, 32-bit IntOp, FileRead keeping CR/LF, FileWrite setting the error flag,
            FileClose reporting nothing, IfErrors clearing the flag).
  simulated the operating system: files and folders (read-only files, running images that cannot be
            deleted, writes that fail after N bytes); child processes (the probe-only worker, the three
            leaves, tasklist) from ONE model - ends after N ms or never, may keep printing, may refuse
            TerminateProcess, may fail WaitForSingleObject, may fail to start - reached through
            CreateProcessW/WaitForSingleObject/TerminateProcess/GetExitCodeProcess (current hook) or
            through nsExec as Contrib/nsExec/nsexec.c implements it (older bytes; NSIS 3.11 = 3.13:
            /TIMEOUT = time without output, the call returns only after the child ended); the clock
            (GetTickCount, Sleep), windows and processes for the hand-off, the console.
  so        a PASS here is evidence about the hook's LOGIC against those outcomes, not a Windows run.
            Windows behaviour (real SAC/App Control refusals, real nsExec timing, real dialogs) stays
            UNMEASURED until windows-build.yml win-update-probe runs.
"""
import argparse
import fnmatch
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_HOOK = os.path.normpath(os.path.join(HERE, "..", "..", "..", "src-tauri", "nsis-hooks.nsh"))
HEADER = "cys installer: new build refused by pre-swap run check (exit 6)\r\n"
NSIS_MAX_STRLEN = 1024
CONSTANTS = ("INSTDIR", "TEMP", "EXEPATH", "CMDLINE", "PLUGINSDIR")
CLOCK_LIMIT_MS = 30 * 60 * 1000          # simulated time after which a run counts as unbounded
STEP_LIMIT = 2000000


class ExecError(Exception):
    """the executor cannot run this text faithfully (exit 2 - never a silent pass)"""


class CompileRefused(Exception):
    """an !error was reached while expanding (the hook refuses to compile this call)"""


class Unbounded(Exception):
    """the run did not end within the simulated time/step budget"""


class Halt(Exception):
    def __init__(self, kind):
        Exception.__init__(self, kind)
        self.kind = kind


# ═════════════════════════════════════════════════════════════════════════════
# parsing and expansion
# ═════════════════════════════════════════════════════════════════════════════

def load(text):
    text = text.replace("\r\n", "\n")
    macros, funcs, cur = {}, {}, None
    for raw in text.split("\n"):
        s = raw.strip()
        if cur is None:
            m = re.match(r"^!macro\s+(\S+)(.*)$", s)
            if m:
                cur = ("macro", m.group(1), m.group(2).split(), [])
                continue
            m = re.match(r"^Function\s+(\S+)", s)
            if m:
                cur = ("func", m.group(1), [], [])
            continue
        if (cur[0] == "macro" and re.match(r"^!macroend\b", s)) or \
                (cur[0] == "func" and re.match(r"^FunctionEnd\b", s)):
            (macros if cur[0] == "macro" else funcs)[cur[1]] = (cur[2], cur[3])
            cur = None
            continue
        cur[3].append(raw)
    uservars = re.findall(r"(?m)^Var\s+(?:/GLOBAL\s+)?(\w+)", text)
    return macros, funcs, uservars


def tokenize(line):
    toks, i, n = [], 0, len(line)
    while i < n:
        c = line[i]
        if c in " \t":
            i += 1
            continue
        if c in ";#":
            break
        if c in "\"'`":
            q, i, buf = c, i + 1, []
            while True:
                if i >= n:
                    raise ExecError("unterminated quote: %r" % line)
                if line.startswith("$\\", i) and i + 2 < n and line[i + 2] in "\"'`":
                    buf.append(line[i:i + 3])
                    i += 3
                    continue
                if line[i] == q:
                    i += 1
                    break
                buf.append(line[i])
                i += 1
            toks.append("".join(buf))
        else:
            j = i
            while j < n and line[j] not in " \t":
                j += 1
            toks.append(line[i:j])
            i = j
    return toks


LIBRARY_CALLS = ("StrLoc", "GetOptions")


def subst(line, args, defines):
    def one(m):
        name = m.group(1)
        if name in args:
            return args[name]
        if name in defines:
            return defines[name]
        if name in LIBRARY_CALLS:
            return m.group(0)
        raise ExecError("undefined define ${%s} in %r" % (name, line))
    return re.sub(r"\$\{([A-Za-z_][\w]*)\}", one, line)


def eval_if(toks):
    if len(toks) == 3 and toks[1] in ("==", "!="):
        eq = toks[0].lower() == toks[2].lower()
        return eq if toks[1] == "==" else not eq
    raise ExecError("unsupported !if form %r" % toks)


def expand(lines, args, defines, macros, out, depth=0, where="?"):
    if depth > 20:
        raise ExecError("macro recursion too deep")
    stack = []   # [active, taken, parent_active]

    def active():
        return all(s[0] for s in stack)

    for raw in lines:
        s = raw.strip()
        if not s or s[0] in ";#":
            continue
        s = subst(s, args, defines)
        toks = tokenize(s)
        if not toks:
            continue
        head = toks[0]
        if head == "!if":
            parent = active()
            cond = parent and eval_if(toks[1:])
            stack.append([cond, cond, parent])
            continue
        if head == "!else":
            if not stack:
                raise ExecError("!else without !if")
            top = stack.pop()
            if len(toks) > 1 and toks[1] == "if":
                cond = top[2] and not top[1] and eval_if(toks[2:])
                stack.append([cond, top[1] or cond, top[2]])
            else:
                stack.append([top[2] and not top[1], True, top[2]])
            continue
        if head == "!endif":
            if not stack:
                raise ExecError("!endif without !if")
            stack.pop()
            continue
        if not active():
            continue
        if head == "!error":
            raise CompileRefused(" ".join(toks[1:]))
        if head in ("!warning", "!verbose", "!echo"):
            continue
        if head == "!insertmacro":
            name = toks[1]
            if name not in macros:
                raise ExecError("unknown macro %s" % name)
            params, body = macros[name]
            vals = toks[2:]
            if len(vals) != len(params):
                raise ExecError("macro %s takes %d args, got %d" % (name, len(params), len(vals)))
            expand(body, dict(zip(params, vals)), defines, macros, out, depth + 1, name)
            continue
        if head.startswith("!"):
            raise ExecError("unsupported directive %r" % s)
        if len(toks) == 1 and head.endswith(":") and not head.startswith("$"):
            out.append(("label", head[:-1], where))
            continue
        out.append(("stmt", toks, where, s))
    if stack:
        raise ExecError("unterminated !if in %s" % where)


# ═════════════════════════════════════════════════════════════════════════════
# simulated operating system
# ═════════════════════════════════════════════════════════════════════════════

def key(path):
    return path.replace("/", "\\").lower().rstrip("\\")


class FS:
    def __init__(self):
        self.files = {}   # key -> dict(path, data, ro, running, open_fail, write_limit)
        self.dirs = set()

    def mkdirs(self, path):
        k = key(path)
        while k and k not in self.dirs:
            self.dirs.add(k)
            if "\\" not in k:
                break
            k = k.rsplit("\\", 1)[0]

    def add(self, path, data="", **kw):
        self.mkdirs(path.rsplit("\\", 1)[0])
        f = dict(path=path, data=data, ro=False, running=False, open_fail=False, write_limit=None)
        f.update(kw)
        self.files[key(path)] = f
        return f

    def parent_exists(self, path):
        return key(path.rsplit("\\", 1)[0]) in self.dirs

    def children(self, d):
        k = key(d) + "\\"
        names = set()
        for c in list(self.files) + list(self.dirs):
            if c.startswith(k):
                rest = c[len(k):]
                if rest:
                    names.add(rest.split("\\", 1)[0])
        return sorted(names)


class Machine:
    def __init__(self, scen, macros, funcs, uservars):
        self.scen = scen
        self.macros, self.funcs = macros, funcs
        self.v = {}
        for r in range(10):
            self.v[str(r)] = ""
            self.v["R%d" % r] = ""
        for u in uservars:
            self.v[u] = ""
        self.v.update(INSTDIR=scen["instdir"], TEMP=scen["temp"], EXEPATH=scen["exepath"],
                      CMDLINE=scen["cmdline"], PLUGINSDIR=scen["temp"] + "\\nsp1.tmp")
        self.names = sorted(self.v, key=len, reverse=True)
        self.err = False
        self.stack = []
        self.clock = 0
        self.steps = 0
        self.fs = scen["fs"]
        self.handles = {}
        self.next_handle = 1000
        self.exitcode = None
        self.dialogs, self.console, self.details, self.execs, self.nsexec_log = [], [], [], [], []
        self.worker_end = None
        self.worker_start = None
        self.gui_since = None
        self.tasklist_i = 0
        self.procs = {}        # process handle -> simulated child
        self.mem = {}          # System struct address -> list of field values
        self.spawned = []      # (command line, clock) of every child started

    # ── strings ────────────────────────────────────────────────────────────
    def ex(self, s):
        out, i, n = [], 0, len(s)
        while i < n:
            c = s[i]
            if c != "$":
                out.append(c)
                i += 1
                continue
            if s.startswith("$\\", i):
                e = s[i + 2:i + 3]
                m = {"r": "\r", "n": "\n", "t": "\t", '"': '"', "'": "'", "`": "`"}
                if e not in m:
                    raise ExecError("unknown escape in %r" % s)
                out.append(m[e])
                i += 3
                continue
            if s.startswith("$$", i):
                out.append("$")
                i += 2
                continue
            for nm in self.names:
                if s.startswith(nm, i + 1):
                    out.append(self.v[nm])
                    i += 1 + len(nm)
                    break
            else:
                raise ExecError("unknown variable at %r" % s[i:i + 20])
        return "".join(out)

    def var(self, tok):
        if not tok.startswith("$") or tok[1:] not in self.v or tok[1:] in CONSTANTS:
            raise ExecError("not a writable variable: %r" % tok)
        return tok[1:]

    def setv(self, tok, val):
        self.v[self.var(tok)] = val

    @staticmethod
    def atoi(s):
        """exehead strtoiptr: optional '-', 0x hex, leading-0 octal, stops at a non-digit."""
        i, sign, base, top = 0, 1, 10, "9"
        if s.startswith("-"):
            sign, i = -1, 1
        if s[i:i + 1] == "0":
            i += 1
            if s[i:i + 1] and "0" <= s[i] <= "7":
                base, top = 8, "7"
            if s[i:i + 1] in ("x", "X"):
                base, i = 16, i + 1
        v = 0
        while i < len(s):
            c = s[i]
            if "0" <= c <= top:
                d = ord(c) - 48
            elif base == 16 and c.lower() in "abcdef":
                d = ord(c.lower()) - 87
            else:
                break
            v = v * base + d
            i += 1
        return Machine.i32(sign * v)

    @staticmethod
    def i32(v):
        v &= 0xFFFFFFFF
        return v - (1 << 32) if v & 0x80000000 else v

    @staticmethod
    def strcpy(src, maxlen, start):
        """exehead EW_ASSIGNVAR."""
        srclen = len(src)
        newlen = srclen if maxlen == "" else Machine.atoi(maxlen)
        st = Machine.atoi(start) if start else 0
        p = ""
        if newlen:
            if st < 0:
                st = srclen + st
            if st >= 0:
                if st > srclen:
                    st = srclen
                p = src[st:]
                if newlen < 0:
                    newlen = len(p) + newlen
                if newlen < 0:
                    newlen = 0
                if newlen < NSIS_MAX_STRLEN:
                    p = p[:newlen]
        return p

    # ── jumps ──────────────────────────────────────────────────────────────
    def target(self, tok, pc):
        if tok in ("", "0"):
            return pc + 1
        if re.match(r"^[+-]\d+$", tok):
            idx, k, want = pc, 0, int(tok)
            step = 1 if want > 0 else -1
            while k != want:
                idx += step
                if idx < 0 or idx >= len(self.prog):
                    raise ExecError("relative jump out of range")
                if self.prog[idx][0] == "stmt":
                    k += step
            return idx
        if tok not in self.labels:
            raise ExecError("unknown label %r" % tok)
        return self.labels[tok]

    # ── files ──────────────────────────────────────────────────────────────
    def fopen(self, path, mode):
        k, f = key(path), self.fs.files.get(key(path))
        if mode == "r":
            if f is None or f["open_fail"] == "all":
                return None
            pos = 0
        elif mode in ("w", "a"):
            if not self.fs.parent_exists(path):
                return None
            if f is not None and (f["ro"] or f["running"] or f["open_fail"]):
                return None
            if f is None:
                f = self.fs.add(path, "")
                lim = self.scen.get("write_limit", {}).get(k)
                if lim is not None:
                    f["write_limit"] = lim
            if mode == "w":
                f["data"] = ""
            pos = 0
        else:
            raise ExecError("FileOpen mode %r" % mode)
        h = str(self.next_handle)
        self.next_handle += 4
        self.handles[h] = dict(k=k, pos=pos, mode=mode)
        return h

    # ── simulated processes: ONE model behind nsExec (old bytes) and CreateProcessW (new) ──
    def behavior(self, cmd):
        """What the simulated child does: ms (None = never ends by itself), exit, out, chatty (keeps
        printing), term_fails (TerminateProcess fails), wait_fails (WaitForSingleObject fails),
        start_fails (CreateProcess fails - e.g. blocked by code integrity)."""
        if "/CYSPROBE=" in cmd:
            w = dict(self.scen["worker"])
            if w.get("exit") == "timeout":
                w["ms"] = None
            w.setdefault("ms", 9000)
            w["kind"] = "worker"
            return w
        if cmd.startswith("tasklist "):
            steps = self.scen["tasklist"]
            st = steps[min(self.tasklist_i, len(steps) - 1)]
            self.tasklist_i += 1
            if st not in TASKLIST:
                raise ExecError("tasklist step %r" % st)
            return dict(TASKLIST[st], kind="tasklist")
        m = re.match(r'^"(.+)\\(cysd|cys|cys-app)\.exe"(.*)$', cmd)
        if m:
            b = dict(DEFAULT_LEAF[m.group(2)])
            b.update(self.scen.get("leaves", {}).get(m.group(2), {}))
            b.update(kind="leaf", name=m.group(2), image=m.group(1) + "\\" + m.group(2) + ".exe", args=m.group(3))
            return b
        raise ExecError("no simulated process for %r" % cmd)

    def spawn(self, cmd, outk=None):
        b = self.behavior(cmd)
        self.spawned.append((cmd, self.clock))
        if b.get("start_fails"):
            return None
        p = dict(b, start=self.clock, done=False, killed=False, outk=outk)
        p["end"] = None if b.get("ms") is None else self.clock + b["ms"]
        if p["kind"] == "worker":
            self.worker_start = self.clock
            d = re.search(r"/D=(.+)$", cmd).group(1)
            self.fs.mkdirs(d)
            for leaf in b.get("leaves", ("cysd.exe", "cys.exe", "cys-app.exe")):
                self.fs.add(d + "\\" + leaf, "MZ", running=leaf in b.get("running", ()))
            if b.get("result") is not None:
                self.fs.add(d + "\\probe-result.txt", "".join(l + "\r\n" for l in b["result"]))
        if p.get("image") and key(p["image"]) in self.fs.files:
            self.fs.files[key(p["image"])]["running"] = True
        if p.get("chatty") and outk in self.fs.files:
            self.fs.files[outk]["data"] += "progress...\r\n" * 3
        return p

    def finish(self, p, killed):
        if p["done"]:
            return
        p["done"], p["killed"] = True, killed
        if p.get("image") and key(p["image"]) in self.fs.files:
            self.fs.files[key(p["image"])]["running"] = False
        if killed:
            p["exit"] = "1"
        else:
            if p["outk"] in self.fs.files:
                self.fs.files[p["outk"]]["data"] += p.get("out", "")
            if p["kind"] == "leaf" and p["name"] == "cys-app":
                m = re.search(r'--cys-probe "(.+)"', p["args"])
                if m and p.get("writes", True):
                    self.fs.add(m.group(1), "cys-app " + NEWV)
        if p["kind"] == "worker":
            self.worker_end = self.clock

    def sim_nsexec(self, cmd, timeout):
        """nsExec::ExecToStack as Contrib/nsExec/nsexec.c implements it (NSIS 3.11 = 3.13): the loop
        ends only when WaitForSingleObject(hProcess, 0) signals and the pipe is empty; /TIMEOUT counts
        time WITHOUT OUTPUT; on a timeout it calls TerminateProcess and keeps looping."""
        self.nsexec_log.append((cmd, timeout))
        p = self.spawn(cmd)
        if p is None:
            return "error", ""
        if p.get("wait_fails"):
            raise Unbounded("nsExec: WaitForSingleObject on the child keeps failing, its loop never ends")
        if p["end"] is not None and (p.get("chatty") or not timeout or p["end"] - p["start"] <= timeout):
            self.clock = p["end"]
            self.finish(p, False)
            return p.get("exit", "0"), p.get("out", "")
        if p.get("chatty"):
            raise Unbounded("nsExec: the child keeps printing, so the no-output timer never fires")
        if not timeout:
            raise Unbounded("nsExec without /TIMEOUT on a child that does not end")
        self.clock = p["start"] + timeout
        if p.get("term_fails"):
            raise Unbounded("nsExec: TerminateProcess fails, the child lives, nsExec keeps waiting")
        self.finish(p, True)
        return "timeout", ""

    def gui_windows(self):
        if self.gui_since is None:
            return []
        g = self.scen.get("gui")
        if not g or self.clock < self.gui_since + g["after_ms"]:
            return []
        return [dict(hwnd=7001, visible=g.get("visible", True), pid=5150,
                     image=g.get("image", self.v["INSTDIR"] + "\\cys-app.exe"))]

    def system(self, spec):
        def reg(name):
            return ("R" + name[1:]) if name.startswith("R") else name[1:]

        ms = re.match(r"^\*\((.*)\)\s*p\s*\.\s*([rR]\d)$", spec)
        if ms:                                    # new struct: *(fields) p .Rx
            vals = []
            for f in [x.strip() for x in ms.group(1).split(",")]:
                body = f.split(None, 1)[1].strip() if " " in f else ""
                vals.append(self.v[reg(body)] if re.match(r"^[rR]\d$", body) else body)
            addr = str(70000 + len(self.mem) * 64)
            self.mem[addr] = vals
            self.v[reg(ms.group(2))] = addr
            return
        ms = re.match(r"^\*(\d+)\((.*)\)$", spec)
        if ms:                                    # read struct fields: *addr(p .R5, ...)
            fields = self.mem[ms.group(1)]
            for i, f in enumerate([x.strip() for x in ms.group(2).split(",")]):
                mm = re.search(r"\.\s*([rR]\d)$", f)
                if mm:
                    self.v[reg(mm.group(1))] = fields[i]
            return
        m = re.match(r"^(\w+)::(\w+)\((.*)\)\s*(.*)$", spec)
        if not m:
            raise ExecError("System::Call spec %r" % spec)
        fn, params, ret = m.group(2), [p.strip() for p in m.group(3).split(",")], m.group(4)

        def val(p):
            body = p.split(None, 1)[1].strip() if " " in p else ""
            if re.match(r"^[rR]\d$", body):
                return self.v[reg(body)]
            return body.strip('"')

        outs = {}
        for idx, p in enumerate(params):
            mm = re.search(r"\.\s*([rR]\d)\s*$", p)
            if mm:
                outs[idx] = reg(mm.group(1))
        rm = re.search(r"\.\s*([rR]\d)", ret)
        rv, lasterr = "0", 0
        sc = self.scen.get("system", {})
        if fn == "GetTickCount":
            rv = str(self.i32(self.scen.get("tick_base", 1000000) + (0 if self.scen.get("tick_frozen") else self.clock)))
        elif fn == "CreateMutexW":
            rv, lasterr = sc.get("CreateMutexW", ("500", 0))
        elif fn == "OpenMutexW":
            rv = sc.get("OpenMutexW", "0")
        elif fn == "CreateFileW":
            path = val(params[0])
            if not self.fs.parent_exists(path):
                rv = "-1"
            else:
                f = self.fs.files.get(key(path)) or self.fs.add(path, "")
                f["data"] = ""
                rv = str(self.next_handle)
                self.next_handle += 4
                self.handles[rv] = dict(k=key(path), pos=0, mode="w")
        elif fn == "CreateProcessW":
            cmd, si, pi = val(params[1]), self.mem[val(params[8])], val(params[9])
            outk = None
            if self.atoi(si[11]) & 0x100 and si[16] in self.handles:
                outk = self.handles[si[16]]["k"]
            p = self.spawn(cmd, outk)
            if p is None:
                rv = "0"
            else:
                h = str(self.next_handle)
                self.next_handle += 4
                self.procs[h] = p
                self.mem[pi] = [h, str(self.next_handle), "4242", "4343"]
                self.next_handle += 4
                rv = "1"
        elif fn == "WaitForSingleObject":
            h = val(params[0])
            p = self.procs.get(h)
            if p is None:
                rv = sc.get("WaitForSingleObject", "128")       # the retired mutex of older bytes
            elif p.get("wait_fails"):
                rv = "-1"
            elif p["done"]:
                rv = "0"
            elif self.atoi(val(params[1])) == -1:          # INFINITE (0xFFFFFFFF)
                if p["end"] is None:
                    raise Unbounded("WaitForSingleObject(INFINITE) on a child that never ends")
                self.clock = max(self.clock, p["end"])
                self.finish(p, False)
                rv = "0"
            elif p["end"] is not None and p["end"] <= self.clock + self.atoi(val(params[1])):
                self.clock = max(self.clock, p["end"])
                self.finish(p, False)
                rv = "0"
            else:
                self.clock += self.atoi(val(params[1]))
                rv = "258"
        elif fn == "TerminateProcess":
            p = self.procs[val(params[0])]
            if p.get("term_fails"):
                rv = "0"
            else:
                self.finish(p, True)
                rv = "1"
        elif fn == "GetExitCodeProcess":
            p = self.procs[val(params[0])]
            self.v[outs[1]] = p.get("exit", "0") if p["done"] else "259"
            rv = "1"
        elif fn == "CloseHandle":
            self.handles.pop(val(params[0]), None)
            rv = "1"
        elif fn == "AttachConsole":
            rv = "1"
        elif fn == "GetStdHandle":
            rv = "CON"
        elif fn == "FindWindowExW":
            after = self.atoi(val(params[1]))
            wins = [w for w in self.gui_windows() if w["hwnd"] > after]
            rv = str(wins[0]["hwnd"]) if wins else "0"
        elif fn == "IsWindowVisible":
            h = self.atoi(val(params[0]))
            rv = "1" if any(w["hwnd"] == h and w["visible"] for w in self.gui_windows()) else "0"
        elif fn == "GetWindowThreadProcessId":
            h = self.atoi(val(params[0]))
            ws = [w for w in self.gui_windows() if w["hwnd"] == h]
            self.v[outs[1]] = str(ws[0]["pid"]) if ws else "0"
            rv = "77"
        elif fn == "OpenProcess":
            pid = self.atoi(val(params[2]))
            rv = "900" if any(w["pid"] == pid for w in self.gui_windows()) else "0"
        elif fn == "QueryFullProcessImageNameW":
            ws = self.gui_windows()
            self.v[outs[2]] = ws[0]["image"] if ws else ""
            rv = "1" if ws else "0"
        else:
            raise ExecError("System::Call to an unsimulated function %s" % fn)
        if rm:
            self.v[reg(rm.group(1))] = rv
        if "?e" in ret:
            self.stack.insert(0, str(lasterr))

    # ── library ports ──────────────────────────────────────────────────────
    def strloc(self, s, sub, direction):
        """Include/StrFunc.nsh StrLoc (NSIS 3.13): case-insensitive, first match."""
        l, n, i = len(sub), len(s), 0
        while True:
            part = self.strcpy(s, str(l), str(i))
            if part.lower() == sub.lower():
                return str(n - (l + i)) if direction == "<" else str(i)
            if i >= n:
                return ""
            i += 1

    def getoptions(self, s, opt):
        """Include/FileFunc.nsh GetOptions (NSIS 3.13): first unquoted occurrence of the option
        (case-insensitive), value up to the next unquoted delimiter, spaces and one pair of
        matching quotes trimmed; error flag when absent."""
        self.err = False
        d, rest = opt[0], opt[1:]
        q, i = "", 0
        while i < len(s):
            c = s[i]
            if c in "\"'`":
                q = "" if q == c else (c if not q else q)
            elif not q and c == d and s[i + 1:i + 1 + len(rest)].lower() == rest.lower():
                j, q2 = i + 1 + len(rest), ""
                k = j
                while k < len(s):
                    ck = s[k]
                    if ck in "\"'`":
                        q2 = "" if q2 == ck else (ck if not q2 else q2)
                    elif not q2 and ck == d:
                        break
                    k += 1
                v = s[j:k].strip(" ")
                if len(v) > 1 and v[0] == v[-1] and v[0] in "\"'`":
                    v = v[1:-1]
                return v
            i += 1
        self.err = True
        return ""

    # ── run ────────────────────────────────────────────────────────────────
    def run_function(self, name):
        if name not in self.funcs:
            raise ExecError("no Function %s in the hook" % name)
        prog = []
        expand(self.funcs[name][1], {}, self.defines(), self.macros, prog, 0, name)
        return self.run_prog(prog)

    def run_prog(self, prog):
        self.prog = prog
        self.labels = {}
        for i, it in enumerate(prog):
            if it[0] == "label":
                if it[1] in self.labels:
                    raise ExecError("duplicate label %s" % it[1])
                self.labels[it[1]] = i
        try:
            self.loop()
            return "end"
        except Halt as h:
            return h.kind

    def loop(self):
        pc = 0
        while pc < len(self.prog):
            self.steps += 1
            if self.steps > STEP_LIMIT or self.clock > CLOCK_LIMIT_MS:
                raise Unbounded("simulated run exceeded %d ms / %d steps" % (CLOCK_LIMIT_MS, STEP_LIMIT))
            it = self.prog[pc]
            if it[0] == "label":
                pc += 1
                continue
            pc = self.step(it[1], pc, it[3])

    def step(self, t, pc, src):
        op, a = t[0], t[1:]
        nxt = pc + 1
        E = self.ex
        if op == "StrCpy":
            self.setv(a[0], self.strcpy(E(a[1]), E(a[2]) if len(a) > 2 else "", E(a[3]) if len(a) > 3 else ""))
        elif op in ("StrCmp", "StrCmpS"):
            x, y = E(a[0]), E(a[1])
            same = (x == y) if op == "StrCmpS" else (x.lower() == y.lower())
            return self.target(a[2], pc) if same else self.target(a[3] if len(a) > 3 else "", pc)
        elif op == "StrLen":
            self.setv(a[0], str(len(E(a[1]))))
        elif op == "IntOp":
            x = self.atoi(E(a[1]))
            o = a[2]
            y = self.atoi(E(a[3])) if len(a) > 3 else 0
            r = {"+": lambda: x + y, "-": lambda: x - y, "*": lambda: x * y, "|": lambda: x | y,
                 "&": lambda: x & y, "^": lambda: x ^ y, "<<": lambda: x << (y & 31),
                 ">>": lambda: x >> (y & 31), "~": lambda: ~x, "!": lambda: int(not x)}.get(o)
            if r is None:
                raise ExecError("IntOp operator %r" % o)
            self.setv(a[0], str(self.i32(r())))
        elif op == "IntCmp":
            x, y = self.atoi(E(a[0])), self.atoi(E(a[1]))
            if x == y:
                tgt = a[2]
            elif x < y:
                tgt = a[3] if len(a) > 3 else ""
            else:
                tgt = a[4] if len(a) > 4 else ""
            return self.target(tgt, pc)
        elif op == "Goto":
            return self.target(a[0], pc)
        elif op == "ClearErrors":
            self.err = False
        elif op == "SetErrors":
            self.err = True
        elif op == "IfErrors":
            e, self.err = self.err, False
            return self.target(a[0] if e else (a[1] if len(a) > 1 else ""), pc)
        elif op == "IfSilent":
            return self.target(a[0] if self.scen["silent"] else (a[1] if len(a) > 1 else ""), pc)
        elif op == "IfFileExists":
            p = E(a[0])
            if p.endswith("\\*.*"):
                ok = key(p[:-4]) in self.fs.dirs
            elif "*" in p:
                d, pat = p.rsplit("\\", 1)
                ok = any(fnmatch.fnmatch(c, pat.lower()) for c in self.fs.children(d))
            else:
                ok = key(p) in self.fs.files or key(p) in self.fs.dirs
            return self.target(a[1] if ok else (a[2] if len(a) > 2 else ""), pc)
        elif op == "FileOpen":
            h = self.fopen(E(a[1]), E(a[2]))
            if h is None:
                self.setv(a[0], "")
                self.err = True
            else:
                self.setv(a[0], h)
        elif op == "FileRead":
            h = E(a[0])
            ent = self.handles.get(h)
            out = ""
            if ent is not None and ent["k"] in self.fs.files:
                data, pos = self.fs.files[ent["k"]]["data"], ent["pos"]
                while pos < len(data) and len(out) < NSIS_MAX_STRLEN - 1:
                    c = data[pos]
                    if out and out[-1] in "\r\n":
                        if out[-1] != c and c in "\r\n":
                            out += c
                            pos += 1
                        break
                    out += c
                    pos += 1
                ent["pos"] = pos
            self.setv(a[1], out)
            if not out:
                self.err = True
        elif op == "FileWrite":
            h, s = E(a[0]), E(a[1])
            if h == "CON":
                self.console.append(s)
            else:
                ent = self.handles.get(h)
                f = self.fs.files.get(ent["k"]) if ent else None
                if f is None or (f["write_limit"] is not None and ent["pos"] + len(s) > f["write_limit"]):
                    self.err = True
                else:
                    d = f["data"]
                    f["data"] = d[:ent["pos"]] + s + d[ent["pos"] + len(s):]
                    ent["pos"] += len(s)
        elif op == "FileSeek":
            ent = self.handles.get(E(a[0]))
            if ent is not None:
                mode = a[2] if len(a) > 2 else "SET"
                off = self.atoi(E(a[1]))
                ln = len(self.fs.files[ent["k"]]["data"])
                ent["pos"] = {"SET": off, "CUR": ent["pos"] + off, "END": ln + off}[mode]
        elif op == "FileClose":
            ent = self.handles.pop(E(a[0]), None)
            # injected: the file is gone after a successful close (AV quarantine, lost flush)
            if ent and ent.get("mode") in ("w", "a") and ent["k"] in self.scen.get("vanish_on_close", ()):
                self.fs.files.pop(ent["k"], None)
        elif op == "Delete":
            p = E(a[-1])
            f = self.fs.files.get(key(p))
            if f is not None:
                if f["running"] or f["ro"]:
                    self.err = True
                else:
                    del self.fs.files[key(p)]
        elif op == "RMDir":
            rec = a[0] == "/r"
            p = E(a[-1])
            k = key(p)
            if k in self.fs.dirs:
                inside = [fk for fk in self.fs.files if fk.startswith(k + "\\")]
                if rec:
                    for fk in inside:
                        if not (self.fs.files[fk]["running"] or self.fs.files[fk]["ro"]):
                            del self.fs.files[fk]
                left = [fk for fk in self.fs.files if fk.startswith(k + "\\")]
                subdirs = [dk for dk in self.fs.dirs if dk.startswith(k + "\\")]
                if not left and (rec or not subdirs):
                    for dk in subdirs:
                        self.fs.dirs.discard(dk)
                    self.fs.dirs.discard(k)
                else:
                    self.err = True
            else:
                self.err = True
        elif op == "CreateDirectory":
            self.fs.mkdirs(E(a[0]))
        elif op == "Sleep":
            self.clock += self.atoi(E(a[0]))
        elif op == "nsExec::ExecToStack":
            timeout, rest = 0, list(a)
            while rest and rest[0].startswith("/"):
                o = E(rest.pop(0))
                if o.upper().startswith("/TIMEOUT="):
                    timeout = self.atoi(o[9:])
                else:
                    raise ExecError("nsExec option %r" % o)
            rv, outtxt = self.nsexec_call(E(rest[0]), timeout)
            self.stack.insert(0, outtxt)
            self.stack.insert(0, rv)
        elif op == "Pop":
            if not self.stack:
                self.err = True
            else:
                self.setv(a[0], self.stack.pop(0))
        elif op == "Push":
            self.stack.insert(0, E(a[0]))
        elif op == "System::Call":
            self.system(E(a[0]))
        elif op == "System::Free":
            self.mem.pop(E(a[0]), None)
        elif op == "InitPluginsDir":
            self.fs.mkdirs(self.v["PLUGINSDIR"])
        elif op == "nsExec::Exec":
            if E(a[-1]).startswith("taskkill /F /IM cys-app.exe"):
                raise Halt("proceed")      # PREINSTALL step 1: the probe passed; the install path starts
            raise ExecError("nsExec::Exec outside the simulated path: %r" % E(a[-1]))
        elif op == "File":
            m = re.match(r"^/oname=(.+)$", E(a[0]))
            if not m:
                raise ExecError("File form %r" % a)
            if self.scen.get("extract_fails"):
                self.err = True
            else:
                self.fs.add(m.group(1), "MZ-new")
        elif op == "Exec":
            self.execs.append(E(a[0]))
            if self.gui_since is None:
                self.gui_since = self.clock
        elif op == "DetailPrint":
            self.details.append(E(a[0]))
        elif op == "MessageBox":
            self.dialogs.append(E(a[1]))
        elif op == "SetErrorLevel":
            self.exitcode = self.atoi(E(a[0]))
        elif op == "Quit":
            raise Halt("quit")
        elif op == "Abort":
            raise Halt("abort")
        elif op == "FindFirst":
            p = E(a[2])
            d, pat = p.rsplit("\\", 1)
            items = [c for c in self.fs.children(d) if fnmatch.fnmatch(c, pat.lower())]
            h = str(self.next_handle)
            self.next_handle += 4
            self.handles[h] = dict(find=items)
            self.setv(a[0], h)
            self.setv(a[1], items.pop(0) if items else "")
            if not self.v[self.var(a[1])]:
                self.err = True
        elif op == "FindNext":
            ent = self.handles.get(E(a[0]), {"find": []})
            items = ent.get("find", [])
            self.setv(a[1], items.pop(0) if items else "")
            if not self.v[self.var(a[1])]:
                self.err = True
        elif op == "FindClose":
            self.handles.pop(E(a[0]), None)
        elif op == "${StrLoc}":
            self.setv(a[0], self.strloc(E(a[1]), E(a[2]), E(a[3])))
        elif op == "${GetOptions}":
            self.setv(a[2], self.getoptions(E(a[0]), E(a[1])))
        else:
            raise ExecError("unsimulated instruction %r (%s)" % (op, src))
        return nxt

    def nsexec_call(self, cmd, timeout):
        return self.sim_nsexec(cmd, timeout)

    def run_macro(self, name):
        """run a parameterless macro (NSIS_HOOK_PREINSTALL) as if inserted in Section Install"""
        if name not in self.macros:
            raise ExecError("no macro %s in the hook" % name)
        prog = []
        expand(self.macros[name][1], {}, self.defines(), self.macros, prog, 0, name)
        return self.run_prog(prog)

    def defines(self):
        d = dict(NSIS_MAX_STRLEN=str(NSIS_MAX_STRLEN), NSIS_PTR_SIZE="4", VERSION=NEWV,
                 CYS_SRC_CYS="binaries\\cys.exe", CYS_SRC_CYSD="binaries\\cysd.exe",
                 MAINBINARYSRCPATH="binaries\\cys-app.exe")
        d.update(self.scen.get("defines", {}))
        return d


# ═════════════════════════════════════════════════════════════════════════════
# scenarios
# ═════════════════════════════════════════════════════════════════════════════

INST = r"D:\cysroot\Local\cys"
TEMP = r"D:\cysroot\Local\Temp"
NEWV = "0.14.53"
OLDV = "0.14.52"

# simulated children (see Machine.behavior)
TASKLIST = {
    "running": dict(ms=200, exit="0", out='"cys-app.exe","4321","Console","1","61,440 K"\r\n'),
    "gone": dict(ms=200, exit="0", out="INFO: No tasks are running which match the specified criteria.\r\n"),
    "fail": dict(ms=200, exit="1", out="ERROR: Invalid query\r\n"),
    "error": dict(start_fails=True),
    "stall": dict(ms=None),
    "chatty": dict(ms=None, chatty=True),
    "hang-term-fails": dict(ms=None, term_fails=True),
    "wait-fails": dict(ms=None, wait_fails=True),
}
DEFAULT_LEAF = {
    "cysd": dict(ms=300, exit="0", out="cysd %s (build a1b2c3d)\r\n" % NEWV),
    "cys": dict(ms=300, exit="0", out="cys %s\r\n" % NEWV),
    "cys-app": dict(ms=500, exit="0", out=""),
}


def base(marker=OLDV + "\r\n", update=False, prior=True, worker=None, tasklist=("gone",),
         gui=None, write_limit=None, extra=None, silent=False, tick_base=1000000, entry="page", leaves=None):
    fs = FS()
    fs.mkdirs(INST)
    fs.mkdirs(TEMP)
    if prior:
        for f in ("cysd.exe", "cys.exe", "cys-app.exe"):
            fs.add(INST + "\\" + f, "MZ-old")
        if marker is not None:
            fs.add(INST + "\\cys-installed-version.txt", marker)
    cmd = '"D:\\cysroot\\Downloads\\cys-setup.exe"' + (" /S" if silent else "") + (" /P /R /UPDATE" if update and not silent else "") + (" /UPDATE" if update and silent else "")
    s = dict(instdir=INST, temp=TEMP, exepath=r"D:\cysroot\Downloads\cys-setup.exe", cmdline=cmd,
             silent=silent, fs=fs, tasklist=list(tasklist), gui=gui, tick_base=tick_base,
             write_limit={key(k): v for k, v in (write_limit or {}).items()},
             worker=worker or refused_worker(), entry=entry, leaves=leaves or {})
    if extra:
        extra(fs)
    return s


def refused_worker(**kw):
    w = dict(exit="6", result=["probe:cysd=rc:-1073740284", "probe:cys=pass", "probe:cys-app=pass",
                               "new-version:" + NEWV, "result=fail"])
    w.update(kw)
    return w


def passing_worker():
    return dict(exit="0", result=["probe:cysd=pass", "probe:cys=pass", "probe:cys-app=pass",
                                  "new-version:" + NEWV, "result=pass"])


def probe_dirs(m):
    return [c for c in m.fs.children(INST) if c.startswith(".cys-probe-")]


def lines_of(m, path):
    f = m.fs.files.get(key(path))
    return None if f is None else f["data"].split("\r\n")


def named_records(m):
    """every record path the dialogs, the console and the details name as THE record"""
    out = []
    for t in m.dialogs:
        out += re.findall(r"Details: (\S+?\.txt)", t)
    for t in m.console + m.details:
        out += [p for p in re.findall(r"record:(\S+)", t) if p not in ("none", "end")]
    return out


def check_named_records(m, problems):
    """B1-F4: a named record must exist and be THIS attempt's record (header, kept, cleanup)."""
    for p in named_records(m):
        ls = lines_of(m, p)
        if ls is None:
            problems.append("named record %s does not exist" % p)
            continue
        if ls[0] + "\r\n" != HEADER or ("kept-version:" + m.v["CysKept"]) not in ls or \
                ("cleanup:" + m.v["CysCleanup"]) not in ls or ("new-version:" + m.v["CysNewVer"]) not in ls:
            problems.append("named record %s is not this attempt's record: %r" % (p, ls[:9]))


def expect_refused(m, halt, problems):
    if halt != "quit" or m.exitcode != 6:
        problems.append("expected exit 6 (halt=%s exit=%s)" % (halt, m.exitcode))


CASES = []


def case(cid, finding, what):
    def deco(fn):
        CASES.append((cid, finding, what, fn))
        return fn
    return deco


# ── controls (must pass on old and new bytes) ────────────────────────────────
@case("X-CTL-pass", "control", "healthy PC: worker exit 0 + result=pass -> page skipped, folder removed, no record")
def _(M):
    m, h = M(base(worker=passing_worker()))
    p = []
    if h != "abort" or m.v["CysProbeDone"] != "1":
        p.append("pass path not taken (halt=%s done=%r)" % (h, m.v["CysProbeDone"]))
    if probe_dirs(m) or m.dialogs or m.v["CysCleanup"] != "ok":
        p.append("leftover: dirs=%r dialogs=%d cleanup=%r" % (probe_dirs(m), len(m.dialogs), m.v["CysCleanup"]))
    return p


# ── B1-F3: the kept version survives the first-line cut ──────────────────────
for _cid, _marker in (("X-F3-crlf", OLDV + "\r\n"), ("X-F3-lf", OLDV + "\n"), ("X-F3-nonl", OLDV)):
    def _mk(marker):
        def fn(M):
            m, h = M(base(marker=marker))
            p = []
            expect_refused(m, h, p)
            if m.v["CysKept"] != OLDV:
                p.append("kept version %r, want %r" % (m.v["CysKept"], OLDV))
            ls = lines_of(m, INST + "\\cys-update-blocked.txt") or []
            if "kept-version:" + OLDV not in ls:
                p.append("record lacks kept-version:%s: %r" % (OLDV, ls[:9]))
            if not any(("(v%s)" % OLDV) in d for d in m.dialogs):
                p.append("dialog does not name v%s: %r" % (OLDV, m.dialogs))
            return p
        return fn
    case(_cid, "B1-F3", "prior marker %r -> kept-version:%s" % (_marker, OLDV))(_mk(_marker))


@case("X-F3-no-marker", "control", "prior install without a marker file -> kept-version:unknown (control)")
def _(M):
    m, h = M(base(marker=None))
    p = []
    expect_refused(m, h, p)
    if m.v["CysKept"] != "unknown":
        p.append("kept %r, want unknown" % m.v["CysKept"])
    return p


# ── B1-F4: a record is named only when it was persisted ──────────────────────
def _stale(fs):
    fs.add(INST + "\\cys-update-blocked.txt",
           HEADER + "blocked: cysd\r\nprobe:cysd=rc:1\r\nprobe:cys=pass\r\nprobe:cys-app=pass\r\n"
           "new-version:0.14.51\r\nkept-version:0.14.50\r\ncleanup:ok\r\nhandoff:not-needed\r\n", ro=True)


@case("X-F4-primary-readonly", "B1-F4", "an earlier read-only record blocks the primary path")
def _(M):
    m, h = M(base(extra=_stale))
    p = []
    expect_refused(m, h, p)
    check_named_records(m, p)
    st = m.fs.files[key(INST + "\\cys-update-blocked.txt")]["data"]
    if "new-version:0.14.51" not in st:
        p.append("the read-only earlier record was changed")
    if not named_records(m):
        p.append("no record of this attempt was named (the fallback was not used)")
    return p


@case("X-F4-disk-full", "B1-F4", "every record write fails after 40 bytes (primary and fallback)")
def _(M):
    s = base()
    s["write_limit_all"] = 40
    m, h = M(s)
    p = []
    expect_refused(m, h, p)
    check_named_records(m, p)
    texts = m.dialogs + m.console
    if not any("record-write-failed" in t for t in texts):
        p.append("no record-write-failed status shown: %r" % texts)
    for need in ("probe:cysd=rc:-1073740284", "kept-version:" + OLDV, "cleanup:ok"):
        if not all(need in t for t in texts):
            p.append("the complete diagnostic (%s) is not in every channel" % need)
    return p


@case("X-F4-not-persisted", "B1-F4", "the primary record is written but gone after close (quarantine, lost flush)")
def _(M):
    s = base()
    s["vanish_on_close"] = (key(INST + "\\cys-update-blocked.txt"),)
    m, h = M(s)
    p = []
    expect_refused(m, h, p)
    check_named_records(m, p)
    if not named_records(m):
        p.append("no record of this attempt was named (the fallback was not used)")
    return p


@case("X-F4-handoff-append-fails", "B1-F4", "record written, the later hand-off append cannot open it")
def _(M):
    s = base(update=True, tasklist=("gone",), gui=dict(after_ms=2000))
    s["fail_append"] = True
    m, h = M(s)
    p = []
    expect_refused(m, h, p)
    named = named_records(m)
    for path in named:
        ls = lines_of(m, path) or []
        if not any(l.startswith("handoff:") for l in ls) and \
                not any("handoff-line-not-written" in t for t in m.console + m.dialogs):
            p.append("record %s lacks its handoff line and no channel says so" % path)
    if not named:
        p.append("no record named")
    return p


# ── B1-F2: the hand-off is bounded and a failed query is not "gone" ──────────
def _handoff_ms(m):
    return m.clock - m.worker_end


@case("X-F2-stalled-query", "B1-F2", "tasklist never prints and never exits")
def _(M):
    m, h = M(base(update=True, tasklist=("stall",), gui=dict(after_ms=2000)))
    p = []
    expect_refused(m, h, p)
    if _handoff_ms(m) > 110000:
        p.append("hand-off took %d ms (bound 110000)" % _handoff_ms(m))
    if m.execs:
        p.append("relaunched while the initiator state is unknown: %r" % m.execs)
    if m.v["CysHandoff"] != "initiator-unknown":
        p.append("handoff %r, want initiator-unknown" % m.v["CysHandoff"])
    return p


@case("X-F2-failing-query", "B1-F2", "tasklist exits 1 with no matching text, every time")
def _(M):
    m, h = M(base(update=True, tasklist=("fail",), gui=dict(after_ms=2000)))
    p = []
    expect_refused(m, h, p)
    if m.execs:
        p.append("a failed query was read as 'initiator gone' and relaunched: %r" % m.execs)
    if m.v["CysHandoff"] != "initiator-unknown":
        p.append("handoff %r, want initiator-unknown" % m.v["CysHandoff"])
    if _handoff_ms(m) > 110000:
        p.append("hand-off took %d ms" % _handoff_ms(m))
    return p


@case("X-F2-nsexec-error", "B1-F2", "nsExec cannot start tasklist ('error')")
def _(M):
    m, h = M(base(update=True, tasklist=("error",), gui=dict(after_ms=2000)))
    p = []
    expect_refused(m, h, p)
    if m.execs or m.v["CysHandoff"] != "initiator-unknown":
        p.append("execs=%r handoff=%r" % (m.execs, m.v["CysHandoff"]))
    return p


@case("X-F2-ok", "control", "initiator exits after 3 queries, GUI up after 2 s -> relaunched-ok")
def _(M):
    m, h = M(base(update=True, tasklist=("running", "running", "running", "gone"), gui=dict(after_ms=2000)))
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "relaunched-ok" or len(m.execs) != 1:
        p.append("handoff %r execs %r" % (m.v["CysHandoff"], m.execs))
    return p


@case("X-F2-never-exits", "control", "initiator never exits -> initiator-did-not-exit within the bound")
def _(M):
    m, h = M(base(update=True, tasklist=("running",), gui=dict(after_ms=2000)))
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "initiator-did-not-exit" or m.execs:
        p.append("handoff %r execs %r" % (m.v["CysHandoff"], m.execs))
    if _handoff_ms(m) > 110000:
        p.append("hand-off took %d ms" % _handoff_ms(m))
    return p


@case("X-F2-tick-wrap", "control", "GetTickCount wraps 2^32 during the wait -> same bound")
def _(M):
    m, h = M(base(update=True, tasklist=("running",), gui=dict(after_ms=2000), tick_base=0xFFFFFFFF - 70000))
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "initiator-did-not-exit" or _handoff_ms(m) > 110000:
        p.append("handoff %r after %d ms" % (m.v["CysHandoff"], _handoff_ms(m)))
    return p


@case("X-F2-late-exit-gui-never", "control", "initiator exits after ~55 s, relaunched app shows no window -> whole hand-off still bounded")
def _(M):
    m, h = M(base(update=True, tasklist=("running",) * 78 + ("gone",), gui=None))
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "relaunch-not-running" or _handoff_ms(m) > 110000:
        p.append("handoff %r after %d ms (bound 110000)" % (m.v["CysHandoff"], _handoff_ms(m)))
    return p


@case("X-F2-stuck-clock", "control", "GetTickCount never moves, initiator never exits -> the iteration belt ends the wait")
def _(M):
    s = base(update=True, tasklist=("running",), gui=dict(after_ms=2000))
    s["tick_frozen"] = True
    m, h = M(s)
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "initiator-did-not-exit" or _handoff_ms(m) > 110000:
        p.append("handoff %r after %d ms" % (m.v["CysHandoff"], _handoff_ms(m)))
    return p


@case("X-F2-gui-never", "control", "relaunched app shows no window -> relaunch-not-running within the bound")
def _(M):
    m, h = M(base(update=True, tasklist=("gone",), gui=None))
    p = []
    expect_refused(m, h, p)
    if m.v["CysHandoff"] != "relaunch-not-running" or _handoff_ms(m) > 110000:
        p.append("handoff %r after %d ms" % (m.v["CysHandoff"], _handoff_ms(m)))
    return p


# ── B1-F1: only a classified or deletable leaf counts as ended ───────────────
def _unconfirmed(m, p):
    if m.v["CysCleanup"] != "cleanup-unconfirmed":
        p.append("cleanup %r, want cleanup-unconfirmed (a leaf may still run)" % m.v["CysCleanup"])
    if not probe_dirs(m):
        p.append("the probe folder was removed while a leaf may run")
    ls = lines_of(m, INST + "\\cys-update-blocked.txt") or []
    if "cleanup:cleanup-unconfirmed" not in ls:
        p.append("record does not say cleanup-unconfirmed: %r" % ls[:9])


@case("X-F1-crash-live-leaf", "B1-F1", "worker crashes (0xC0000005), no result file, cysd.exe leaf still running")
def _(M):
    m, h = M(base(worker=dict(exit="-1073741819", result=None, running=("cysd.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-exit6-incomplete-live-leaf", "B1-F1", "worker exit 6 with a result file cut short, cys.exe running")
def _(M):
    m, h = M(base(worker=dict(exit="6", result=["probe:cysd=rc:1"], running=("cys.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-exit6-status-missing-live-leaf", "B1-F1", "worker exit 6 + result=fail, but two status lines missing, cys.exe running")
def _(M):
    m, h = M(base(worker=dict(exit="6", result=["probe:cysd=rc:1", "new-version:" + NEWV, "result=fail"],
                              running=("cys.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-exit0-no-result-live-leaf", "B1-F1", "worker exit 0 but no result file, cysd.exe running")
def _(M):
    m, h = M(base(worker=dict(exit="0", result=None, running=("cysd.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-odd-exit-live-leaf", "B1-F1", "worker exit 1 with a complete-looking file, cys-app.exe running")
def _(M):
    m, h = M(base(worker=refused_worker(exit="1", running=("cys-app.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-timeout-live-leaf", "control", "worker timed out (nsExec terminated it), a leaf still running")
def _(M):
    m, h = M(base(worker=dict(exit="timeout", result=None, running=("cysd.exe",))))
    p = []
    expect_refused(m, h, p)
    _unconfirmed(m, p)
    return p


@case("X-F1-crash-no-leaf", "control", "worker crashes, no leaf running -> images deleted, folder removed")
def _(M):
    m, h = M(base(worker=dict(exit="-1073741819", result=None)))
    p = []
    expect_refused(m, h, p)
    if m.v["CysCleanup"] != "ok" or probe_dirs(m):
        p.append("cleanup %r dirs %r" % (m.v["CysCleanup"], probe_dirs(m)))
    return p


@case("X-F1-classified-refusal", "control", "worker exit 6 with a complete result -> folder removed, cleanup ok")
def _(M):
    m, h = M(base())
    p = []
    expect_refused(m, h, p)
    if m.v["CysCleanup"] != "ok" or probe_dirs(m):
        p.append("cleanup %r dirs %r" % (m.v["CysCleanup"], probe_dirs(m)))
    return p


# ── B1r2-F1: every P1 child has a HARD wall-clock bound (round 3) ─────────────
PAGE_BOUND_MS = 130000      # worker: 120 s + 5 s confirmed termination, then the refusal
SILENT_BOUND_MS = 130000    # 3 leaves x (30 s + 5 s) + cleanup retries
HANDOFF_BOUND_MS = 110000   # 60 s + one 10 s query, window search to 95 s, one 5 s re-check


def old_untouched(m, p):
    for f in ("cysd.exe", "cys.exe", "cys-app.exe"):
        e = m.fs.files.get(key(INST + "\\" + f))
        if e is None or e["data"] != "MZ-old":
            p.append("the installed %s changed" % f)
    mk = m.fs.files.get(key(INST + "\\cys-installed-version.txt"))
    if mk is None or mk["data"] != OLDV + "\r\n":
        p.append("the installed version marker changed")


def _worker_case(cid, what, **w):
    def fn(M):
        m, h = M(base(worker=dict(dict(result=None), **w)))
        p = []
        expect_refused(m, h, p)
        old_untouched(m, p)
        took = m.clock - (m.worker_start or 0)
        if took > PAGE_BOUND_MS:
            p.append("refusal took %d ms after the worker started (bound %d)" % (took, PAGE_BOUND_MS))
        if m.v["CysCleanup"] != w.get("want_cleanup", "cleanup-unconfirmed"):
            p.append("cleanup %r, want %r" % (m.v["CysCleanup"], w.get("want_cleanup", "cleanup-unconfirmed")))
        if w.get("want_cleanup", "cleanup-unconfirmed") == "cleanup-unconfirmed" and not probe_dirs(m):
            p.append("the folder of a worker that may still run was removed")
        return p
    case(cid, "B1r2-F1", what)(fn)


_worker_case("X-R3-worker-hang-terminate-fails", "worker never ends and TerminateProcess fails",
             ms=None, term_fails=True)
_worker_case("X-R3-worker-wait-fails", "WaitForSingleObject on the worker fails", ms=None, wait_fails=True)
_worker_case("X-R3-worker-keeps-printing", "worker never ends and keeps printing (terminate works)",
             ms=None, chatty=True, want_cleanup="ok")


def _leaf_case(cid, what, finding, want_status, **leaf):
    def fn(M):
        m, h = M(base(entry="preinstall", silent=True, update=True, leaves=dict(cysd=leaf)))
        p = []
        expect_refused(m, h, p)
        old_untouched(m, p)
        if m.clock > SILENT_BOUND_MS:
            p.append("silent refusal took %d ms (bound %d)" % (m.clock, SILENT_BOUND_MS))
        if m.v["CysPrCysd"] != want_status:
            p.append("probe:cysd=%r, want %r" % (m.v["CysPrCysd"], want_status))
        if want_status == "cleanup-unconfirmed" and m.v["CysCleanup"] != "cleanup-unconfirmed":
            p.append("cleanup %r, want cleanup-unconfirmed" % m.v["CysCleanup"])
        return p
    case(cid, finding, what)(fn)


_leaf_case("X-R3-leaf-hang-terminate-fails", "cysd.exe never ends and TerminateProcess fails",
           "B1r2-F1", "cleanup-unconfirmed", ms=None, term_fails=True)
_leaf_case("X-R3-leaf-wait-fails", "WaitForSingleObject on cysd.exe fails", "B1r2-F1",
           "cleanup-unconfirmed", ms=None, wait_fails=True)
_leaf_case("X-R3-leaf-keeps-printing", "cysd.exe never ends and keeps printing", "B1r2-F1",
           "timeout", ms=None, chatty=True)
_leaf_case("X-R3-leaf-prints-past-the-bound", "cysd.exe prints for 200 s, then exits with the right line",
           "B1r2-F1", "timeout", ms=200000, chatty=True)
_leaf_case("X-R3-leaf-blocked", "cysd.exe cannot start (code integrity) -> error", "control", "error",
           start_fails=True)


@case("X-R3-inline-pass", "control", "silent install, all three leaves pass -> the install path starts")
def _(M):
    m, h = M(base(entry="preinstall", silent=True, update=True))
    p = []
    if h != "proceed":
        p.append("halt %r, want proceed (exit %r, blocked %r)" % (h, m.exitcode, m.v["CysBlocked"]))
    if probe_dirs(m) or m.v["CysCleanup"] != "ok":
        p.append("probe folder %r cleanup %r" % (probe_dirs(m), m.v["CysCleanup"]))
    return p


def _query_case(cid, what, step):
    def fn(M):
        m, h = M(base(update=True, tasklist=(step,), gui=dict(after_ms=2000)))
        p = []
        expect_refused(m, h, p)
        if _handoff_ms(m) > HANDOFF_BOUND_MS:
            p.append("hand-off took %d ms (bound %d)" % (_handoff_ms(m), HANDOFF_BOUND_MS))
        if m.execs or m.v["CysHandoff"] != "initiator-unknown":
            p.append("execs %r handoff %r, want no relaunch + initiator-unknown" % (m.execs, m.v["CysHandoff"]))
        return p
    case(cid, "B1r2-F1", what)(fn)


_query_case("X-R3-query-hang-terminate-fails", "tasklist never ends and TerminateProcess fails", "hang-term-fails")
_query_case("X-R3-query-wait-fails", "WaitForSingleObject on tasklist fails", "wait-fails")
_query_case("X-R3-query-keeps-printing", "tasklist never ends and keeps printing", "chatty")


# ═════════════════════════════════════════════════════════════════════════════

def make_runner(macros, funcs, uservars):
    def M(scen):
        lim_all = scen.get("write_limit_all")
        m = Machine(scen, macros, funcs, uservars)
        orig_fopen = m.fopen

        def fopen(path, mode):
            # injected failures: every append-open fails; every write-open gets a byte limit
            if mode == "a" and scen.get("fail_append"):
                return None
            h = orig_fopen(path, mode)
            if h is not None and lim_all is not None and mode in ("w", "a"):
                m.fs.files[key(path)]["write_limit"] = lim_all
            return h
        m.fopen = fopen
        if scen["entry"] == "page":
            halt = m.run_function("cys_probe_page")
        else:
            halt = m.run_macro("NSIS_HOOK_PREINSTALL")
        return m, halt
    return M


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--hook", default=DEFAULT_HOOK)
    ap.add_argument("--case", action="append")
    ap.add_argument("--json")
    o = ap.parse_args()
    try:
        text = open(o.hook, "rb").read().decode("utf-8")
    except OSError as e:
        print("p1-exec: cannot read the hook: %s" % e, file=sys.stderr)
        return 2
    macros, funcs, uservars = load(text)
    M = make_runner(macros, funcs, uservars)
    results, worst = [], 0
    for cid, finding, what, fn in CASES:
        if o.case and cid not in o.case:
            continue
        try:
            probs = fn(M)
            state = "PASS" if not probs else "FAIL"
        except Unbounded as e:
            probs, state = ["UNBOUNDED: %s" % e], "FAIL"
        except (ExecError, CompileRefused) as e:
            probs, state = ["EXECUTOR: %s: %s" % (type(e).__name__, e)], "ERROR"
        results.append(dict(id=cid, finding=finding, what=what, result=state, problems=probs))
        worst = max(worst, {"PASS": 0, "FAIL": 1, "ERROR": 2}[state])
        print("p1-exec %-34s %-7s %-5s %s" % (cid, finding, state, "; ".join(probs)))
    if o.case and not results:
        print("p1-exec: no such case %r" % o.case, file=sys.stderr)
        return 2
    n_fail = sum(r["result"] != "PASS" for r in results)
    print("p1-exec: %d cases, %d passed, %d not passed (hook %s)" % (len(results), len(results) - n_fail, n_fail, o.hook))
    if o.json:
        with open(o.json, "w") as f:
            json.dump(dict(hook=o.hook, results=results), f, indent=1)
    return worst


if __name__ == "__main__":
    sys.exit(main())
