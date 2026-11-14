"""TW1 probes (stages S0-S3) - the Windows side of the measurement driver.

Every probe returns OBSERVATIONS (facts) only; tw1_record computes every verdict from them (plan impl/plans/TW1.md 3.7).
A probe fills its facts dict as it goes, so that an exception still leaves what was observed in the evidence.

Interpreters (plan 2.2): the runner's Python runs this module. The INSTALLED product's bundled Python runs only the
holders and the probes about that runtime (always with -B: no __pycache__ in the installed tree). The installed bundled
Git Bash runs the hook holder.

Nothing here repairs what it finds; product files are changed only as the scenario's user edits (stage S1) and the
product's own actions the scenario reproduces (the app's init-pack, the daemon start, the narrow hook registrar).
"""

import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import zipfile

IS_WIN = os.name == "nt"
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import tw1_record as R  # noqa: E402

CREATE_NO_WINDOW = 0x08000000
CREATE_NEW_PROCESS_GROUP = 0x00000200
LIFELINE = ("session-start.sh", "role-bootstrap.sh", "inject-context.sh", "save-state.sh", "cys-statusline.sh",
            "role-capability-gate.sh")

if IS_WIN:
    import ctypes
    import ctypes.wintypes as wt
    import winreg

    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _u32 = ctypes.WinDLL("user32", use_last_error=True)
    _ver = ctypes.WinDLL("version", use_last_error=True)

    class _PE32(ctypes.Structure):
        _fields_ = [("dwSize", wt.DWORD), ("cntUsage", wt.DWORD), ("th32ProcessID", wt.DWORD),
                    ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", wt.DWORD), ("cntThreads", wt.DWORD),
                    ("th32ParentProcessID", wt.DWORD), ("pcPriClassBase", ctypes.c_long), ("dwFlags", wt.DWORD),
                    ("szExeFile", wt.WCHAR * 260)]

    _k32.CreateToolhelp32Snapshot.restype = wt.HANDLE
    _k32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
    _k32.Process32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(_PE32)]
    _k32.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(_PE32)]
    _k32.OpenProcess.restype = wt.HANDLE
    _k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    _k32.CloseHandle.argtypes = [wt.HANDLE]
    _k32.TerminateProcess.argtypes = [wt.HANDLE, wt.UINT]
    _k32.GetExitCodeProcess.argtypes = [wt.HANDLE, ctypes.POINTER(wt.DWORD)]
    _k32.QueryFullProcessImageNameW.argtypes = [wt.HANDLE, wt.DWORD, wt.LPWSTR, ctypes.POINTER(wt.DWORD)]
    _k32.GetShortPathNameW.argtypes = [wt.LPCWSTR, wt.LPWSTR, wt.DWORD]
    _k32.ProcessIdToSessionId.argtypes = [wt.DWORD, ctypes.POINTER(wt.DWORD)]
    _WNDENUMPROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    _u32.EnumWindows.argtypes = [_WNDENUMPROC, wt.LPARAM]
    _u32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
    _u32.IsWindowVisible.argtypes = [wt.HWND]
    _u32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
    _ver.GetFileVersionInfoSizeW.argtypes = [wt.LPCWSTR, ctypes.POINTER(wt.DWORD)]
    _ver.GetFileVersionInfoW.argtypes = [wt.LPCWSTR, wt.DWORD, wt.DWORD, ctypes.c_void_p]
    _ver.VerQueryValueW.argtypes = [ctypes.c_void_p, wt.LPCWSTR, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wt.UINT)]
    _k32.CreateFileW.restype = wt.HANDLE
    _k32.CreateFileW.argtypes = [wt.LPCWSTR, wt.DWORD, wt.DWORD, ctypes.c_void_p, wt.DWORD, wt.DWORD, wt.HANDLE]
    _k32.MoveFileExW.argtypes = [wt.LPCWSTR, wt.LPCWSTR, wt.DWORD]
    _k32.CreateHardLinkW.argtypes = [wt.LPCWSTR, wt.LPCWSTR, ctypes.c_void_p]
    _k32.FlushFileBuffers.argtypes = [wt.HANDLE]
    _k32.GetFinalPathNameByHandleW.argtypes = [wt.HANDLE, wt.LPWSTR, wt.DWORD, wt.DWORD]

    class _BHFI(ctypes.Structure):
        _fields_ = [("dwFileAttributes", wt.DWORD), ("ftCreationTime", wt.FILETIME), ("ftLastAccessTime", wt.FILETIME),
                    ("ftLastWriteTime", wt.FILETIME), ("dwVolumeSerialNumber", wt.DWORD), ("nFileSizeHigh", wt.DWORD),
                    ("nFileSizeLow", wt.DWORD), ("nNumberOfLinks", wt.DWORD), ("nFileIndexHigh", wt.DWORD),
                    ("nFileIndexLow", wt.DWORD)]

    _k32.GetFileInformationByHandle.argtypes = [wt.HANDLE, ctypes.POINTER(_BHFI)]


# ───────────────────────────── small helpers ─────────────────────────────

def sha256_file(p):
    try:
        h = hashlib.sha256()
        with open(p, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def sha256_bytes(b):
    return hashlib.sha256(b).hexdigest()


def read_bytes(p):
    try:
        with open(p, "rb") as f:
            return f.read()
    except OSError:
        return None


def read_text(p):
    b = read_bytes(p)
    return None if b is None else b.decode("utf-8", "replace")


def write_bytes(p, b):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as f:
        f.write(b)


def pack_manifest(pack_dir):
    """{rel: sha256} of every file under the pack (dot files included). None when the pack folder is absent."""
    if not os.path.isdir(pack_dir):
        return None
    out = {}
    for dp, _dns, fns in os.walk(pack_dir):
        for fn in fns:
            p = os.path.join(dp, fn)
            out[os.path.relpath(p, pack_dir).replace("\\", "/")] = sha256_file(p)
    return out


def vendor_rels():
    """The embedded pack's file list the way build.rs makes PACK_ALL: git-tracked files under cysjavis-pack/ without
    a component that starts with '.', 'tests' or '__pycache__' - sorted (the install's item order)."""
    r = subprocess.run(["git", "-C", REPO, "ls-files", "-z", "cysjavis-pack"], capture_output=True, timeout=120)
    rels = set()
    for raw in r.stdout.split(b"\0"):
        line = raw.decode("utf-8", "replace").strip()
        if not line.startswith("cysjavis-pack/"):
            continue
        rel = line[len("cysjavis-pack/"):]
        if any(c.startswith(".") or c in ("tests", "__pycache__") for c in rel.split("/")):
            continue
        rels.add(rel)
    return sorted(rels)


def vendor_shas(rels):
    """{rel: sha256 of the candidate's vendor bytes} (the checkout is the commit the candidate was built from)."""
    return {rel: sha256_file(os.path.join(REPO, "cysjavis-pack", *rel.split("/"))) for rel in rels}


def dir_identity(path):
    """Volume serial + file index of a folder through an open handle (backup semantics); None when it cannot be
    opened. A folder that was renamed away and replaced has a different identity at the same path."""
    if not IS_WIN:
        return None
    h, _err = _open_h(path, 0, _SR | _SW | _SD, _OPEN_EXISTING, _BACKUP)
    if h is None:
        return None
    try:
        info = _BHFI()
        if not _k32.GetFileInformationByHandle(h, ctypes.byref(info)):
            return None
        return "%08x-%08x%08x" % (info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow)
    finally:
        _k32.CloseHandle(h)


def staging_leftovers(cys_dir):
    """Names beside the pack that a staged publish or an older swap leaves behind."""
    try:
        names = os.listdir(cys_dir)
    except OSError:
        return None
    return sorted(n for n in names if n.startswith(".pack-staging") or "superseded" in n)


def vendor_changed(pack, rel):
    """Does the candidate change this pack file's bytes? None when either side is missing."""
    v = sha256_file(os.path.join(REPO, "cysjavis-pack", *rel.split("/")))
    b = sha256_file(os.path.join(pack, *rel.split("/")))
    return None if v is None or b is None else v != b


def surface_roles(cys_exe):
    """`cys list` -> ([role per surface], result); None roles when the list did not answer."""
    r = run([cys_exe, "list"], 30, env=clean_env({"CYS_NO_AUTOSTART": "1"}))
    if r["code"] != 0:
        return None, r
    roles = []
    for line in r["out"].splitlines():
        m = re.search(r"\brole=(\S*)", line)
        if m:
            roles.append(m.group(1))
    return roles, r


def _dump(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=0, sort_keys=True)


def _load(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return None


def norm_path(p):
    """Case-folded, backslash form of a Windows path for comparisons (None stays None)."""
    if p is None:
        return None
    s = str(p)
    if s.startswith("\\\\?\\"):
        s = s[4:]
    return s.replace("/", "\\").rstrip("\\").lower()


def run(cmd, timeout, env=None, cwd=None, stdin_path=None):
    """Run a command to completion under a time limit. Never raises for a failing command."""
    t0 = time.monotonic()
    res = {"cmd": [str(c) for c in cmd], "code": None, "out": "", "err": "", "secs": None, "timed_out": False}
    stdin = open(stdin_path, "rb") if stdin_path else subprocess.DEVNULL
    try:
        kw = {"creationflags": CREATE_NO_WINDOW} if IS_WIN else {}
        p = subprocess.run([str(c) for c in cmd], stdin=stdin, capture_output=True, timeout=timeout, env=env,
                           cwd=cwd, **kw)
        res["code"] = p.returncode
        res["out"] = p.stdout.decode("utf-8", "replace")
        res["err"] = p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired as e:
        res["timed_out"] = True
        res["out"] = (e.stdout or b"").decode("utf-8", "replace")
        res["err"] = (e.stderr or b"").decode("utf-8", "replace")
    except OSError as e:
        res["err"] = "launch error: %s" % e
    finally:
        if stdin_path:
            stdin.close()
        res["secs"] = round(time.monotonic() - t0, 3)
    return res


def powershell(script, timeout=120):
    return run(["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script],
               timeout)


def ps_json(script, timeout=120):
    r = powershell(script + " | ConvertTo-Json -Depth 4 -Compress", timeout)
    try:
        return json.loads(r["out"]) if r["code"] == 0 and r["out"].strip() else None
    except ValueError:
        return None


def clean_env(extra=None, drop_prefixes=("CYS_", "JAVIS_", "CLAUDE", "PYTHON")):
    """The process environment without product / Python knobs (the measured product must not inherit them)."""
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith(tuple(p.upper() for p in drop_prefixes))}
    env.pop("GH_TOKEN", None)
    env.pop("GITHUB_TOKEN", None)
    if extra:
        env.update(extra)
    return env


# ───────────────────────────── processes ─────────────────────────────

def processes():
    """[{pid, ppid, name, path}] - toolhelp snapshot; PowerShell CIM as the fallback."""
    if not IS_WIN:
        return []
    out = []
    try:
        snap = _k32.CreateToolhelp32Snapshot(0x2, 0)
        if snap in (None, wt.HANDLE(-1).value):
            raise OSError("snapshot failed")
        try:
            e = _PE32()
            e.dwSize = ctypes.sizeof(_PE32)
            ok = _k32.Process32FirstW(snap, ctypes.byref(e))
            while ok:
                out.append({"pid": int(e.th32ProcessID), "ppid": int(e.th32ParentProcessID), "name": e.szExeFile})
                ok = _k32.Process32NextW(snap, ctypes.byref(e))
        finally:
            _k32.CloseHandle(snap)
        for p in out:
            p["path"] = image_path(p["pid"])
        return out
    except Exception:  # noqa: BLE001 - the fallback is the point
        data = ps_json("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath")
        if isinstance(data, dict):
            data = [data]
        return [{"pid": d.get("ProcessId"), "ppid": d.get("ParentProcessId"), "name": d.get("Name"),
                 "path": d.get("ExecutablePath")} for d in (data or [])]


def image_path(pid):
    h = _k32.OpenProcess(0x1000, False, pid)
    if not h:
        return None
    try:
        buf = ctypes.create_unicode_buffer(32768)
        n = wt.DWORD(32768)
        if _k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(n)):
            return buf.value
        return None
    finally:
        _k32.CloseHandle(h)


def alive(pid):
    if not IS_WIN or not pid:
        return False
    h = _k32.OpenProcess(0x1000, False, int(pid))
    if not h:
        return False
    try:
        code = wt.DWORD()
        if not _k32.GetExitCodeProcess(h, ctypes.byref(code)):
            return False
        return code.value == 259  # STILL_ACTIVE
    finally:
        _k32.CloseHandle(h)


def terminate(pid):
    if not IS_WIN or not pid:
        return False
    h = _k32.OpenProcess(0x0001, False, int(pid))
    if not h:
        return False
    try:
        return bool(_k32.TerminateProcess(h, 1))
    finally:
        _k32.CloseHandle(h)


def descendants(root_pid, procs=None):
    procs = processes() if procs is None else procs
    kids, frontier = set(), {root_pid}
    while frontier:
        nxt = {p["pid"] for p in procs if p["ppid"] in frontier and p["pid"] not in kids and p["pid"] != root_pid}
        kids |= nxt
        frontier = nxt
    return kids


def kill_tree(root_pid, keep=()):
    """Terminate root_pid and its descendants except the pids in keep. -> list of pids terminated."""
    keep = set(int(k) for k in keep)
    done = []
    for pid in sorted(descendants(root_pid)) + [root_pid]:
        if pid in keep:
            continue
        if terminate(pid):
            done.append(pid)
    return done


def count_named(name):
    return sum(1 for p in processes() if (p.get("name") or "").lower() == name.lower())


def pids_under(folder, name=None):
    f = norm_path(folder)
    res = []
    for p in processes():
        path = norm_path(p.get("path"))
        if path and f and path.startswith(f + "\\"):
            if name is None or (p.get("name") or "").lower() == name.lower():
                res.append(p["pid"])
    return res


def start_detached(cmd, env=None, cwd=None, stdout_path=None):
    """Start a long-lived process that outlives the probe (registered for the always() cleanup)."""
    out = open(stdout_path, "wb") if stdout_path else subprocess.DEVNULL
    kw = {"creationflags": CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP} if IS_WIN else {}
    p = subprocess.Popen([str(c) for c in cmd], stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT,
                         env=env, cwd=cwd, **kw)
    return p.pid


def visible_windows():
    """[(pid, title)] of visible top-level windows (empty when no desktop is reachable)."""
    if not IS_WIN:
        return []
    res = []

    def cb(hwnd, _lp):
        try:
            if _u32.IsWindowVisible(hwnd):
                pid = wt.DWORD()
                _u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                buf = ctypes.create_unicode_buffer(512)
                _u32.GetWindowTextW(hwnd, buf, 512)
                res.append((int(pid.value), buf.value))
        except Exception:  # noqa: BLE001
            pass
        return True

    _u32.EnumWindows(_WNDENUMPROC(cb), 0)
    return res


def file_version(path):
    """VS_FIXEDFILEINFO file version 'a.b.c.d' (None when absent / unreadable)."""
    if not IS_WIN or not os.path.isfile(path):
        return None
    try:
        size = _ver.GetFileVersionInfoSizeW(path, None)
        if not size:
            return None
        buf = ctypes.create_string_buffer(size)
        if not _ver.GetFileVersionInfoW(path, 0, size, buf):
            return None
        ptr = ctypes.c_void_p()
        ln = wt.UINT()
        if not _ver.VerQueryValueW(buf, "\\", ctypes.byref(ptr), ctypes.byref(ln)) or not ptr.value:
            return None
        words = (wt.DWORD * 13).from_address(ptr.value)
        ms, ls = words[2], words[3]
        return "%d.%d.%d.%d" % (ms >> 16, ms & 0xFFFF, ls >> 16, ls & 0xFFFF)
    except Exception:  # noqa: BLE001
        r = ps_json("$v=(Get-Item -LiteralPath '%s').VersionInfo; [pscustomobject]@{a=$v.FileMajorPart;b=$v.FileMinorPart;c=$v.FileBuildPart;d=$v.FilePrivatePart}" % path.replace("'", "''"))
        return None if not r else "%s.%s.%s.%s" % (r["a"], r["b"], r["c"], r["d"])


def short_path(p):
    if not IS_WIN:
        return None
    buf = ctypes.create_unicode_buffer(32768)
    n = _k32.GetShortPathNameW(p, buf, 32768)
    return buf.value if n else None


def reg_value(root, key, name):
    if not IS_WIN:
        return None
    try:
        with winreg.OpenKey(root, key) as k:
            return winreg.QueryValueEx(k, name)[0]
    except OSError:
        return None


# ───────────────────────────── context and evidence ─────────────────────────────

class Probe:
    """What a probe gets: the shared context, its evidence folder, a facts dict filled as it goes."""

    def __init__(self, pid, work, out, ctx):
        self.pid = pid
        self.work = work
        self.out = out
        self.ctx = ctx
        self.facts = {}
        self.evidence = []
        self.ctx_updates = {}
        self.restored = None
        self.ev_dir = os.path.join(out, "evidence", pid)
        os.makedirs(self.ev_dir, exist_ok=True)
        self.mapping = [tuple(x) for x in ctx.get("redaction", [])]

    def ev(self, name, content):
        """Write an evidence file (always redacted - RULINGS EQ6) and list it."""
        p = os.path.join(self.ev_dir, name)
        if not isinstance(content, str):
            content = json.dumps(content, ensure_ascii=False, indent=1, sort_keys=True, default=str)
        content = R.redact(content, self.mapping)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(content)
        rel = os.path.relpath(p, self.out).replace("\\", "/")
        if rel not in self.evidence:
            self.evidence.append(rel)
        return p

    def ev_cmd(self, name, res):
        self.ev(name, res)
        return res

    def timeline_point(self, label, edits=None):
        """Full pack manifest {rel: sha256} right after a writer (run-1 lesson: a partial delete that a later writer
        refills is invisible at the end). Saved in the work folder (for later probes) and as evidence; the edited
        files' hashes at this point go into facts["timeline"][label]."""
        m = pack_manifest(self.pack_dir)
        _dump(os.path.join(self.work, "timeline", label + ".json"), m)
        self.ev("timeline/%s.json" % label, m if m is not None else {"absent": True})
        edits = edits if edits is not None else (self.ctx.get("edits") or {})
        tl = self.facts.setdefault("timeline", {})
        tl[label] = {"at": round(time.time(), 3), "files": None if m is None else len(m),
                     "edited": {k: (None if m is None else m.get(e["rel"])) for k, e in edits.items()}}
        return m

    def register_pid(self, pid, purpose):
        reg = os.path.join(self.work, "pids.jsonl")
        with open(reg, "a", encoding="utf-8") as f:
            f.write(json.dumps({"pid": pid, "purpose": purpose, "probe": self.pid, "at": time.time()}) + "\n")

    def unregister_pid(self, pid):
        reg = os.path.join(self.work, "pids.jsonl")
        with open(reg, "a", encoding="utf-8") as f:
            f.write(json.dumps({"pid": pid, "stopped": True, "probe": self.pid, "at": time.time()}) + "\n")

    # convenience paths
    @property
    def install_dir(self):
        return self.ctx["paths"]["install_dir"]

    @property
    def pack_dir(self):
        return self.ctx["paths"]["pack_dir"]

    def exe(self, name):
        return os.path.join(self.install_dir, name)

    def cys(self, args, timeout=60, env=None, extra=None):
        e = env if env is not None else clean_env(extra)
        return run([self.exe("cys.exe")] + list(args), timeout, env=e)

    def bundled_python(self):
        return os.path.join(self.install_dir, "runtime", "python", "python.exe")

    def bundled_hook_bash(self):
        return os.path.join(self.install_dir, "runtime", "git", "bin", "bash.exe")


def matrix():
    return R.load_matrix(os.path.join(HERE, "matrix.json"))


# ───────────────────────────── REST (stdlib) ─────────────────────────────

class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def api(path, token, timeout=60):
    base = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    req = urllib.request.Request(base + path, headers={
        "Accept": "application/vnd.github+json", "Authorization": "Bearer " + token,
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "tw1-windows-measure"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def download(url, dest, timeout=900, token=None):
    """Download to dest. With a token the first request is authenticated and a redirect is followed WITHOUT the token
    (artifact archives redirect to storage that must not receive it)."""
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    headers = {"User-Agent": "tw1-windows-measure"}
    if token:
        headers["Authorization"] = "Bearer " + token
        headers["Accept"] = "application/vnd.github+json"
        opener = urllib.request.build_opener(_NoRedirect)
        try:
            r = opener.open(urllib.request.Request(url, headers=headers), timeout=timeout)
        except urllib.error.HTTPError as e:
            if e.code in (301, 302, 303, 307, 308) and e.headers.get("Location"):
                url = e.headers["Location"]
                headers = {"User-Agent": "tw1-windows-measure"}
                r = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=timeout)
            else:
                raise
    else:
        r = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=timeout)
    with r, open(dest, "wb") as f:
        shutil.copyfileobj(r, f, 1 << 20)
    return dest


def repo_slug():
    return os.environ.get("GITHUB_REPOSITORY", "")


def fetch_artifact(run_id, name, dest, token, work):
    """Download and unpack artifact `name` of workflow run `run_id` into dest. -> (ok, note)."""
    try:
        arts = api("/repos/%s/actions/runs/%s/artifacts?per_page=100" % (repo_slug(), run_id), token)
        art = next((a for a in arts.get("artifacts", []) if a.get("name") == name and not a.get("expired")), None)
        if not art:
            return False, "artifact %s not listed in run %s" % (name, run_id)
        z = download(art["archive_download_url"], os.path.join(work, "dl", name + ".zip"), token=token)
        with zipfile.ZipFile(z) as zf:
            zf.extractall(dest)
        return True, "downloaded %s (%s bytes) from run %s" % (name, art.get("size_in_bytes"), run_id)
    except Exception as e:  # noqa: BLE001 - recorded, never raised
        return False, "artifact %s from run %s failed: %s" % (name, run_id, e)


# ───────────────────────────── S0 — environment and installers ─────────────────────────────

def _git(args, timeout=120):
    return run(["git", "-C", REPO] + list(args), timeout)


def merge_log_units(units):
    r = _git(["log", "--merges", "--format=%s", "HEAD"])
    if r["code"] != 0:
        return {u: None for u in units}, r
    subjects = r["out"].splitlines()
    res = {}
    for u in units:
        rx = re.compile(r"^merge\([^)]*\b%s\b[^)]*\):" % re.escape(u))
        res[u] = any(rx.match(s) for s in subjects)
    return res, {"code": r["code"], "merges": len(subjects)}


def find_files(root, pattern):
    return sorted(glob.glob(os.path.join(root, "**", pattern), recursive=True))


def seven_zip():
    for c in (shutil.which("7z"), r"C:\Program Files\7-Zip\7z.exe"):
        if c and os.path.isfile(c):
            return c
    return None


def extract_sidecars(installer, dest, pr):
    """The cys.exe / cysd.exe the installer carries (top-level entries), via 7-Zip. -> {name: path} or {}."""
    z = seven_zip()
    if not z:
        pr.facts["cand_extract_note"] = "7-Zip not found"
        return {}
    lst = run([z, "l", "-slt", installer], 300)
    pr.ev("candidate-7z-list.txt", "\n".join(lst["out"].splitlines()[:4000]))
    paths = [ln[7:].strip() for ln in lst["out"].splitlines() if ln.startswith("Path = ")]
    names = {"cys.exe": ("cys.exe", "cys.new.exe"), "cysd.exe": ("cysd.exe", "cysd.new.exe")}
    want = {}
    for name, accepted in names.items():
        want[name] = [p for p in paths if p.replace("\\", "/").split("/")[-1].lower() in accepted
                      and "runtime" not in p.lower()]
    got = {}
    os.makedirs(dest, exist_ok=True)
    for name, inners in want.items():
        for i, inner in enumerate(inners):
            sub = os.path.join(dest, "%s.%d.d" % (name, i))
            r = run([z, "e", installer, "-o" + sub, inner, "-y"], 300)
            cand = os.path.join(sub, os.path.basename(inner.replace("\\", "/")))
            if r["code"] == 0 and os.path.isfile(cand):
                got.setdefault(name, []).append(cand)
    pr.facts["cand_extract_entries"] = want
    return got


def probe_s0_env(pr):
    f = pr.facts
    m = matrix()
    ctx = pr.ctx
    base = m["base"]
    f["base_version"] = base["version"]
    f["base_version4"] = base["version"] + ".0"
    # ── Windows facts ──
    os_info = ps_json("Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,ProductType,OSArchitecture") or {}
    f["windows_caption"] = os_info.get("Caption")
    f["windows_version"] = os_info.get("Version")
    f["windows_build"] = os_info.get("BuildNumber")
    f["product_type"] = os_info.get("ProductType")  # 1 workstation, 2 DC, 3 server
    f["is_server"] = os_info.get("ProductType") in (2, 3) if os_info else None
    f["ubr"] = reg_value(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "UBR") if IS_WIN else None
    f["acp"] = reg_value(winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Nls\CodePage", "ACP") if IS_WIN else None
    f["oemcp"] = reg_value(winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Nls\CodePage", "OEMCP") if IS_WIN else None
    f["long_paths_enabled"] = reg_value(winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\FileSystem", "LongPathsEnabled") if IS_WIN else None
    f["console_cp"] = ctypes.windll.kernel32.GetConsoleCP() if IS_WIN else None
    f["console_output_cp"] = ctypes.windll.kernel32.GetConsoleOutputCP() if IS_WIN else None
    r = run(["fsutil", "8dot3name", "query", os.environ.get("SystemDrive", "C:")], 60)
    f["eight_dot_three"] = (r["out"] + r["err"]).strip()[:400]
    f["webview2_version"] = reg_value(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}", "pv") if IS_WIN else None
    if IS_WIN:
        sid = wt.DWORD()
        f["session_id"] = int(sid.value) if _k32.ProcessIdToSessionId(os.getpid(), ctypes.byref(sid)) else None
    f["user_interactive"] = (powershell("[Environment]::UserInteractive")["out"].strip() == "True")
    wins = visible_windows()
    f["visible_windows"] = len(wins)
    # ── Defender (RULINGS EQ4 D8: switched on for the antivirus legs; whether it took effect is recorded) ──
    before = ps_json("Get-MpComputerStatus | Select-Object AMServiceEnabled,AntivirusEnabled,RealTimeProtectionEnabled,IsTamperProtected")
    f["defender_available"] = before is not None
    f["rtp_before"] = (before or {}).get("RealTimeProtectionEnabled")
    pr.ctx_updates["defender_original_rtp"] = f["rtp_before"]
    if before is not None:
        sw = powershell("Set-MpPreference -DisableRealtimeMonitoring $false")
        pr.ev_cmd("defender-switch.json", sw)
        time.sleep(3)
        after = ps_json("Get-MpComputerStatus | Select-Object AMServiceEnabled,AntivirusEnabled,RealTimeProtectionEnabled,IsTamperProtected")
        f["rtp_after"] = (after or {}).get("RealTimeProtectionEnabled")
        pr.ev("defender-status.json", {"before": before, "after": after})
    else:
        f["rtp_after"] = None
    # ── the commit, the units, the sibling runs ──
    f["commit"] = os.environ.get("GITHUB_SHA") or _git(["rev-parse", "HEAD"])["out"].strip()
    units, info = merge_log_units(m["units"])
    f["merge_log"] = units
    pr.ctx_updates["merge_log"] = units
    pr.ev("merge-log.json", {"units": units, "info": info})
    # ── versions at this commit ──
    try:
        with open(os.path.join(REPO, "src-tauri", "tauri.conf.json"), encoding="utf-8") as fh:
            f["candidate_version"] = json.load(fh).get("version")
    except (OSError, ValueError):
        f["candidate_version"] = None
    cargo = read_text(os.path.join(REPO, "Cargo.toml")) or ""
    mv = re.search(r'(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"', cargo)
    f["cargo_version"] = mv.group(1) if mv else None
    f["candidate_version4"] = (f["candidate_version"] + ".0") if f["candidate_version"] else None
    # ── the candidate installer: an artifact of the windows-build run at this commit (REST) ──
    cand_dir = os.path.join(pr.work, "candidate")
    token = os.environ.get("GH_TOKEN", "")
    run_id = os.environ.get("TW1_BUILD_RUN_ID", "")
    f["build_run_id"] = run_id or None
    f["build_conclusion"] = os.environ.get("TW1_BUILD_CONCLUSION") or None
    f["build_lookup_reason"] = os.environ.get("TW1_BUILD_REASON") or None
    found, via, note = [], None, None
    if run_id and token and repo_slug():
        ok, note = fetch_artifact(run_id, "cys-windows-x64-nsis", cand_dir, token, pr.work)
        found = find_files(cand_dir, "*-setup.exe") if ok else []
        via = "rest" if found else None
    else:
        note = "no windows-build run id / token / repository in the environment"
    f["candidate_note"] = note
    f["candidate_obtained"] = bool(found)
    f["candidate_via"] = via
    cand = found[0] if found else None
    f["candidate_files"] = [os.path.basename(p) for p in found]
    f["candidate_sha256"] = sha256_file(cand) if cand else None
    mm = re.search(r"cys_([0-9][0-9.]*)_x64-setup\.exe$", os.path.basename(cand or ""))
    f["candidate_version_filename"] = mm.group(1) if mm else None
    pr.ctx_updates.setdefault("installers", {})["candidate"] = cand
    # ── the released base, two independent hashes ──
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
    rel = "%s/%s/releases/download/v%s/" % (server, repo_slug(), base["version"])
    base_path = os.path.join(pr.work, "base", base["installer"])
    try:
        download(rel + base["installer"], base_path)
        f["base_installer_obtained"] = os.path.isfile(base_path)
    except Exception as e:  # noqa: BLE001
        f["base_installer_obtained"] = False
        f["base_download_note"] = str(e)
    f["base_sha256"] = sha256_file(base_path) if f["base_installer_obtained"] else None
    f["base_sha_pinned_match"] = f["base_sha256"] == base["sha256"] if f["base_sha256"] else False
    try:
        sums = os.path.join(pr.work, "base", "SHA256SUMS.txt")
        download(rel + "SHA256SUMS.txt", sums)
        line = next((ln for ln in (read_text(sums) or "").splitlines() if ln.strip().endswith(" " + base["installer"])), None)
        f["base_sha_sums_value"] = line.split()[0] if line else None
        f["base_sha_sums_match"] = bool(line) and f["base_sha256"] == line.split()[0]
    except Exception as e:  # noqa: BLE001
        f["base_sha_sums_match"] = False
        f["base_sums_note"] = str(e)
    pr.ctx_updates.setdefault("installers", {})["base"] = base_path
    # ── the candidate's sidecars from inside its installer (identity of the installation later) ──
    got = extract_sidecars(cand, os.path.join(pr.work, "candidate-bins"), pr) if cand else {}
    f["cand_extract_ok"] = bool(got.get("cys.exe") and got.get("cysd.exe"))
    f["cand_cys_shas"] = sorted({sha256_file(x) for x in got.get("cys.exe", [])})
    f["cand_cysd_shas"] = sorted({sha256_file(x) for x in got.get("cysd.exe", [])})
    pr.ctx_updates["candidate_bins"] = got
    pr.ctx_updates["candidate_shas"] = {"cys.exe": f["cand_cys_shas"], "cysd.exe": f["cand_cysd_shas"]}
    # ── the following-version pair (RULINGS EQ2 D4): an artifact of THIS run (REST) ──
    tnv_dir = os.path.join(pr.work, "tnv")
    own = os.environ.get("GITHUB_RUN_ID", "")
    if own and token and repo_slug():
        ok, f["tnv_note"] = fetch_artifact(own, "tw1-tnv-binaries", tnv_dir, token, pr.work)
    else:
        f["tnv_note"] = "no run id / token / repository in the environment"
    tc = find_files(tnv_dir, "cys.exe")
    td = find_files(tnv_dir, "cysd.exe")
    tj = find_files(tnv_dir, "tnv-prepare.json")
    f["tnv_obtained"] = bool(tc and td)
    f["tnv_cys_sha"] = sha256_file(tc[0]) if tc else None
    f["tnv_cysd_sha"] = sha256_file(td[0]) if td else None
    try:
        f["tnv_version"] = json.loads(read_text(tj[0]))["tnv_version"] if tj else None
    except (ValueError, KeyError, TypeError):
        f["tnv_version"] = None
    pr.ctx_updates["tnv"] = {"cys": tc[0] if tc else None, "cysd": td[0] if td else None}
    pr.ctx_updates["versions"] = {"base": f["base_version"], "candidate": f["candidate_version"]}
    pr.ev("environment-variables.json", R.env_snapshot(dict(os.environ)))
    pr.restored = True


# ───────────────────────────── S1 — the used base ─────────────────────────────

def _boot_critical_exit(cys_exe, path, env=None):
    r = run([cys_exe, "repair", "boot-critical", "--path", path, "--json"], 60, env=env or clean_env())
    lane = None
    try:
        lane = json.loads(r["out"]).get("lane_pack") if r["out"].strip().startswith("{") else None
    except ValueError:
        pass
    return r["code"], lane, r


def lifeline_registrations(settings_files):
    """{token: [settings file]} for the six lifeline hooks found in hook / statusLine commands."""
    found = {}
    for sf in settings_files:
        txt = read_text(sf)
        if not txt:
            continue
        try:
            doc = json.loads(txt)
        except ValueError:
            continue
        cmds = []
        for groups in (doc.get("hooks") or {}).values():
            for g in groups or []:
                for h in (g or {}).get("hooks", []) or []:
                    cmds.append(str((h or {}).get("command", "")))
        sl = doc.get("statusLine")
        if isinstance(sl, dict):
            cmds.append(str(sl.get("command", "")))
        for tok in LIFELINE:
            if any(tok in c for c in cmds):
                found.setdefault(tok, []).append(sf)
    return found


def settings_candidates(profile):
    found = [os.path.join(profile, ".claude", "settings.json")]
    for depth in ("*", "*/*", "*/*/*"):
        found += glob.glob(os.path.join(profile, ".cys", *depth.split("/"), "settings.json"))
    return sorted({p for p in found if os.path.isfile(p)})


def builtin_job_ids(pack_dir):
    txt = read_text(os.path.join(pack_dir, "schedule.json"))
    if txt is None:
        return None
    try:
        doc = json.loads(txt)
    except ValueError:
        return None
    return sorted(j.get("id") for j in (doc.get("jobs") or []) if isinstance(j, dict) and j.get("_builtin"))


def remove_hook_registration(settings_file, token):
    doc = json.loads(read_text(settings_file))
    changed = False
    for ev, groups in list((doc.get("hooks") or {}).items()):
        new_groups = []
        for g in groups or []:
            hooks = [h for h in (g or {}).get("hooks", []) or [] if token not in str((h or {}).get("command", ""))]
            if len(hooks) != len((g or {}).get("hooks", []) or []):
                changed = True
            if hooks:
                g = dict(g)
                g["hooks"] = hooks
                new_groups.append(g)
        doc["hooks"][ev] = new_groups
    if changed:
        write_bytes(settings_file, (json.dumps(doc, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    return changed


def wait_ready(cys_exe, limit_s, t0=None):
    """Poll `cys list` (never autostarting) until it answers. -> (ready, seconds since t0, last result)."""
    t0 = time.monotonic() if t0 is None else t0
    while time.monotonic() - t0 < limit_s:
        r = run([cys_exe, "list"], 15, env=clean_env({"CYS_NO_AUTOSTART": "1"}))
        if r["code"] == 0:
            return True, round(time.monotonic() - t0, 2), r
        time.sleep(0.25)
    return False, round(time.monotonic() - t0, 2), None


def probe_s1_base(pr):
    f = pr.facts
    m = matrix()
    sc = m["scenario"]
    f["base_version"] = m["base"]["version"]
    f["base_version4"] = m["base"]["version"] + ".0"
    inst = pr.ctx["installers"]["base"]
    stray = [p for p in processes() if (p.get("name") or "").lower() in ("cys.exe", "cysd.exe", "cys-app.exe")]
    f["stray_before_install"] = len(stray)
    r = run([inst, "/S"], 900)
    pr.ev_cmd("base-install.json", r)
    f["base_install_exit"] = r["code"]
    f["base_install_secs"] = r["secs"]
    time.sleep(3)
    after_install = [p for p in processes() if (p.get("name") or "").lower() in ("cys.exe", "cysd.exe", "cys-app.exe")]
    f["processes_after_base_install"] = [p["name"] for p in after_install]
    for p in after_install:  # windows-build's self-contained step does the same; recorded
        terminate(p["pid"])
    time.sleep(1)
    f["cysd_count_before_base_start"] = count_named("cysd.exe")
    f["base_marker"] = (read_text(pr.exe("cys-installed-version.txt")) or "").strip() or None
    f["base_cys_version"] = file_version(pr.exe("cys.exe"))
    f["base_cysd_version"] = file_version(pr.exe("cysd.exe"))
    ip = pr.ev_cmd("base-init-pack.json", pr.cys(["init-pack"], timeout=600))
    f["base_init_pack_exit"] = ip["code"]
    dpid = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, "base-cysd.out"))
    pr.register_pid(dpid, "base daemon (launcher)")
    ok, secs, _ = wait_ready(pr.exe("cys.exe"), 90)
    f["base_daemon_ready"] = ok
    f["base_daemon_ready_s"] = secs
    daemons = [p for p in processes() if (p.get("name") or "").lower() == "cysd.exe"]
    f["cysd_count_after_base_start"] = len(daemons)
    for p in daemons:
        pr.register_pid(p["pid"], "base daemon")
    pr.ctx_updates["base_daemon_pids"] = [p["pid"] for p in daemons]
    ns = pr.ev_cmd("base-new-surface.json", pr.cys(["new-surface", "--role", "master"], timeout=60))
    f["master_surface_created"] = ns["code"] == 0
    pack = pr.pack_dir
    t0 = time.monotonic()
    while builtin_job_ids(pack) is None and time.monotonic() - t0 < 60:
        time.sleep(2)
    # ── the base's sets (before any edit) ──
    jobs = builtin_job_ids(pack)
    f["base_jobs_builtin"] = jobs
    settings = settings_candidates(os.environ.get("USERPROFILE", ""))
    regs = lifeline_registrations(settings)
    f["base_hooks_lifeline"] = sorted(regs)
    pr.ev("base-lifeline-registrations.json", regs)
    pr.timeline_point("s1_before_edits", {})
    # ── selection: listed / general by the candidate's own query; vendor change by bytes ──
    cand_cys = ((pr.ctx.get("candidate_bins") or {}).get("cys.exe") or [None])[0]
    sel, why = {}, {}

    def vendor(rel):
        return read_bytes(os.path.join(REPO, "cysjavis-pack", *rel.split("/")))

    def base_b(rel):
        return read_bytes(os.path.join(pack, *rel.split("/")))

    def listed(rel):
        if not cand_cys:
            return None
        code, _, _ = _boot_critical_exit(cand_cys, os.path.join(pack, *rel.split("/")))
        return code

    def pick(key, pool, want_listed, want_changed, taken):
        for rel in pool:
            if rel in taken:
                continue
            v, b = vendor(rel), base_b(rel)
            if v is None or b is None:
                why.setdefault(key, []).append("%s: missing (vendor=%s base=%s)" % (rel, v is not None, b is not None))
                continue
            changed = sha256_bytes(v) != sha256_bytes(b)
            lc = listed(rel)
            ok_listed = (lc == 0) if want_listed else (lc == 1)
            if lc is None:
                ok_listed = True  # verified after the update with the installed binary (rows' 'complete' checks)
            if (want_changed is None or changed == want_changed) and ok_listed:
                sel[key] = rel
                taken.add(rel)
                return
            why.setdefault(key, []).append("%s: changed=%s listed_exit=%s" % (rel, changed, lc))

    taken = set()
    for key in ("L1", "L2", "CL", "PE", "PD"):
        pick(key, sc["listed_changed"], True, True, taken)
    pick("L3", sc["listed_unchanged"], True, False, taken)
    for key in ("G1", "ENC1", "CG"):
        pick(key, sc["general_changed_py"], False, True, taken)
    pick("CRLF1", sc["general_changed_sh"], False, True, taken)
    pick("CRLF2", sc["general_changed_md"], False, True, taken)
    pick("CRLF3", sc["general_cmd"], False, None, taken)        # a Windows batch file (FT1); vendor change recorded
    pick("CRLF4", sc["general_changed_py"], False, True, taken)  # content AND line endings changed together (FT)
    f["selection"] = sel
    f["selection_listed_verified"] = cand_cys is not None
    f["selection_ok"] = all(k in sel for k in ("L1", "L2", "CL", "L3", "G1", "ENC1", "CG", "CRLF1", "CRLF2"))
    pr.ev("selection.json", {"selected": sel, "rejected": why, "listed_query_by": "candidate cys.exe" if cand_cys else None})
    # ── the user's edits (each with before / after hash) ──
    edits, errors, aux = {}, [], []

    def edit(key, transform):
        rel = sel.get(key)
        if not rel:
            errors.append("%s: not selected" % key)
            return
        p = os.path.join(pack, *rel.split("/"))
        b = read_bytes(p)
        if b is None:
            errors.append("%s: unreadable" % key)
            return
        nb = transform(b)
        write_bytes(p, nb)
        write_bytes(os.path.join(pr.work, "user-bytes", key), nb)
        edits[key] = {"rel": rel, "base_sha": sha256_bytes(b), "user_sha": sha256_bytes(nb),
                      "vendor_sha": sha256_bytes(vendor(rel) or b""), "user_has_cr": b"\r" in nb}

    def append_line(tag):
        def t(b):
            nl = b"" if b.endswith(b"\n") else b"\n"
            return b + nl + ("# tw1 user edit %s\n" % tag).encode("ascii")
        return t

    def to_cp949(b):
        text = b.decode("utf-8", "replace") + "\n# 사용자 수정 (tw1 CP949 edit)\n"
        return text.encode("cp949", "replace")

    def to_crlf(b):
        return b.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")

    def content_and_crlf(comment):
        def t(b):
            nl = b"" if b.endswith(b"\n") else b"\n"
            return to_crlf(b + nl + (comment + "\n").encode("ascii"))
        return t

    edit("L1", append_line("L1"))
    edit("L2", append_line("L2"))
    edit("L3", append_line("L3"))
    edit("G1", append_line("G1"))
    edit("ENC1", to_cp949)
    edit("CRLF1", to_crlf)
    edit("CRLF2", to_crlf)
    if sel.get("CRLF3"):
        edit("CRLF3", content_and_crlf("REM tw1 user edit CRLF3"))
    if sel.get("CRLF4"):
        edit("CRLF4", content_and_crlf("# tw1 user edit CRLF4"))
    # T2g (report R5.5 item 3): a copy already at <rel>.user before the update - PE equal to the user's bytes, PD different
    for key in ("PE", "PD"):
        if sel.get(key):
            edit(key, append_line(key))
            e = edits.get(key)
            if not e:
                continue
            lp_ = os.path.join(pack, *e["rel"].split("/"))
            planted = read_bytes(lp_) if key == "PE" else ("# tw1 planted earlier copy %s (differs from the user's bytes)\n"
                                                           % key).encode("ascii")
            e["planted_preexisting"] = os.path.lexists(lp_ + ".user")
            if planted is not None:
                write_bytes(lp_ + ".user", planted)
                e["planted_sha"] = sha256_bytes(planted)
                e["planted_read_back_ok"] = sha256_file(lp_ + ".user") == e["planted_sha"]
    for key in ("CL", "CG"):
        rel = sel.get(key)
        if rel:
            b = read_bytes(os.path.join(pack, *rel.split("/"))) or b""
            edits[key] = {"rel": rel, "base_sha": sha256_bytes(b), "user_sha": sha256_bytes(b),
                          "vendor_sha": sha256_bytes(vendor(rel) or b""), "user_has_cr": b"\r" in b}
    # read-back: every edited file is re-hashed from the disk (run 1 recorded the bytes it wrote, not a read-back)
    rb_bad = []
    for key, e in edits.items():
        e["read_back_sha"] = sha256_file(os.path.join(pack, *e["rel"].split("/")))
        if e["read_back_sha"] != e["user_sha"]:
            rb_bad.append(key)
    f["edits_read_back_ok"] = not rb_bad
    f["edits_read_back_bad"] = rb_bad
    # the base's own install record for the edited files (design 4-11 da: without it every file is "no base")
    man_txt = read_text(os.path.join(pack, ".install-manifest.json"))
    try:
        man = json.loads(man_txt) if man_txt else None
    except ValueError:
        man = None
    man = man.get("files") if isinstance(man, dict) and isinstance(man.get("files"), dict) else man
    f["base_manifest_present"] = isinstance(man, dict)
    f["base_manifest_entries"] = len(man) if isinstance(man, dict) else None
    for key, e in edits.items():
        e["base_manifest_hash"] = man.get(e["rel"]) if isinstance(man, dict) else None
    enc = sel.get("ENC1")
    if enc:
        try:
            read_bytes(os.path.join(pack, *enc.split("/"))).decode("utf-8")
            f["ENC1_user_invalid_utf8"] = False
        except UnicodeDecodeError:
            f["ENC1_user_invalid_utf8"] = True
    # E-JOB: one built-in job removed by the user
    ejob = None
    if jobs:
        ejob = "fleet-digest" if "fleet-digest" in jobs else jobs[0]
        sp = os.path.join(pack, "schedule.json")
        doc = json.loads(read_text(sp))
        doc["jobs"] = [j for j in doc.get("jobs", []) if not (isinstance(j, dict) and j.get("id") == ejob)]
        write_bytes(sp, (json.dumps(doc, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
        after = builtin_job_ids(pack) or []
        f["ejob_detected_missing"] = ejob not in after
    else:
        aux.append("E-JOB: no built-in job found in schedule.json")
    f["ejob_id"] = ejob
    # E-HOOK: one lifeline hook registration removed by the user
    ehook = "save-state.sh" if "save-state.sh" in regs else (sorted(regs)[0] if regs else None)
    if ehook:
        for sf in regs[ehook]:
            remove_hook_registration(sf, ehook)
        f["ehook_detected_missing"] = ehook not in lifeline_registrations(settings)
    else:
        aux.append("E-HOOK: no lifeline registration found in %s" % settings)
    f["ehook_token"] = ehook
    # E-RT: a bundled-runtime file in the install folder edited (C-sp1-01)
    rt = os.path.join(pr.install_dir, "runtime", "LICENSES", "BUNDLED-RUNTIMES.txt")
    rb = read_bytes(rt)
    if rb is not None:
        nb = rb + b"\n# tw1 runtime edit (first-update case)\n"
        write_bytes(rt, nb)
        f["rt_edit_sha"] = sha256_bytes(nb)
    else:
        f["rt_edit_sha"] = None
        aux.append("E-RT: %s not found" % "runtime/LICENSES/BUNDLED-RUNTIMES.txt")
    # PACK-PREV (RULINGS GH2 / GJ3): a pack.prev an older version left behind - here the only copy of the L1 edit
    prev_dir = os.path.join(os.path.dirname(pack), "pack.prev")
    f["pack_prev_preexisting"] = os.path.lexists(prev_dir)
    f["pack_prev_planted_files"] = None
    if not f["pack_prev_preexisting"]:
        write_bytes(os.path.join(prev_dir, "tw1-planted-marker.txt"),
                    b"planted by TW1 S1: a pack.prev left by an older version (must stay byte-identical)\n")
        write_bytes(os.path.join(prev_dir, "nested", "deep", "note.md"), b"# nested planted note\n")
        if edits.get("L1"):
            lb = read_bytes(os.path.join(pack, *edits["L1"]["rel"].split("/")))
            if lb is not None:
                write_bytes(os.path.join(prev_dir, *edits["L1"]["rel"].split("/")), lb)
        pm = pack_manifest(prev_dir)
        _dump(os.path.join(pr.work, "pack-prev-planted.json"), pm)
        pr.ev("pack-prev-planted.json", pm)
        f["pack_prev_planted_files"] = None if pm is None else len(pm)
    pr.timeline_point("s1_after_edits", edits)
    f["edits"] = edits
    f["edit_errors"] = errors
    f["aux_edit_errors"] = aux  # these only make their own rows NOT-MEASURED (the rows' controls)
    f["edits_applied"] = not errors and f.get("selection_ok", False)
    pr.ev("edits.json", {"edits": edits, "errors": errors, "aux_errors": aux, "ejob": ejob, "ehook": ehook})
    pr.ctx_updates.update({"edits": edits, "base_jobs_builtin": jobs, "base_hooks_lifeline": sorted(regs),
                           "ejob": ejob, "ehook": ehook, "settings_files": settings, "rt_file": rt,
                           "rt_edit_sha": f["rt_edit_sha"], "selection": sel,
                           "ejob_detected_missing": f.get("ejob_detected_missing"),
                           "ehook_detected_missing": f.get("ehook_detected_missing")})
    pr.restored = True


# ───────────────────────────── S2 — the update ─────────────────────────────

class Watcher(threading.Thread):
    """200-ms presence samples of the three executables (checklist P1) and the visible windows that appear."""

    def __init__(self, install_dir, period=0.2):
        super().__init__(daemon=True)
        self.d = install_dir
        self.period = period
        self.stop_ev = threading.Event()
        self.samples = []
        self.windows = set()
        self.window_enum_ok = True
        self.base_windows = set()

    def run(self):
        try:
            self.base_windows = {t for _, t in visible_windows()}
        except Exception:  # noqa: BLE001
            self.window_enum_ok = False
        n = 0
        while not self.stop_ev.is_set():
            self.samples.append((round(time.time(), 3), os.path.isfile(os.path.join(self.d, "cys.exe")),
                                 os.path.isfile(os.path.join(self.d, "cysd.exe")),
                                 os.path.isfile(os.path.join(self.d, "cys-app.exe"))))
            n += 1
            if n % 5 == 0 and self.window_enum_ok:
                try:
                    for _, t in visible_windows():
                        if t and t not in self.base_windows:
                            self.windows.add(t)
                except Exception:  # noqa: BLE001
                    self.window_enum_ok = False
            time.sleep(self.period)


def manifest_missing(pack_dir):
    txt = read_text(os.path.join(pack_dir, ".install-manifest.json"))
    if txt is None:
        return None
    try:
        doc = json.loads(txt)
    except ValueError:
        return None
    entries = doc.get("files") if isinstance(doc, dict) and isinstance(doc.get("files"), dict) else doc
    if not isinstance(entries, dict):
        return None
    return sorted(rel for rel in entries if not os.path.exists(os.path.join(pack_dir, *rel.split("/"))))


def snapshot_dirs(local_dir):
    return sorted(glob.glob(os.path.join(local_dir, "update-snapshots", "*", "*")))


def probe_s2_update(pr):
    f = pr.facts
    ctx = pr.ctx
    m = matrix()
    f["base_version"] = m["base"]["version"]
    f["candidate_version"] = ctx["versions"]["candidate"]
    f["candidate_version4"] = (f["candidate_version"] + ".0") if f["candidate_version"] else None
    f["marker_before"] = (read_text(pr.exe("cys-installed-version.txt")) or "").strip() or None
    pack = pr.pack_dir
    local = ctx["paths"]["local_dir"]
    snaps_before = snapshot_dirs(local)
    f["snapshot_count_before"] = len(snaps_before)
    f["pack_dir_id_pre"] = dir_identity(pack)
    f["staging_leftovers_before"] = staging_leftovers(os.path.dirname(pack))
    roles, lr = surface_roles(pr.exe("cys.exe"))
    pr.ev("surfaces-before-update.json", lr)
    f["seats_before_update"] = roles
    # G01 (RULINGS GH1 / GS2): the held files need no replacement - the update must then succeed while they are held
    held_rels = {"bash": "hooks/" + m["scenario"]["holder_hook"], "python": "bin/" + m["scenario"]["holder_module"] + ".py"}
    f["held_vendor_changed"] = {k: vendor_changed(pack, rel) for k, rel in held_rels.items()}
    f["held_files_need_no_replace"] = all(v is False for v in f["held_vendor_changed"].values())
    # ── holders (CW7-HELD, G01) ──
    holder_file = os.path.join(pack, "hooks", m["scenario"]["holder_hook"])
    bash = pr.bundled_hook_bash()
    bpid = None
    if os.path.isfile(bash) and os.path.isfile(holder_file):
        bpid = start_detached([bash, "-c", 'exec 3< "%s"; sleep 900' % holder_file.replace("\\", "/")],
                              env=clean_env(), stdout_path=os.path.join(pr.ev_dir, "holder-bash.out"))
        pr.register_pid(bpid, "bash holder")
    py = pr.bundled_python()
    mod = m["scenario"]["holder_module"]
    held_py = os.path.join(pack, "bin", mod + ".py")
    code = ("import sys,time\nsys.path.insert(0, r'%s')\nok='import-failed'\ntry:\n import %s\n ok='imported'\n"
            "except Exception as e:\n ok='import-failed: %%s' %% e\nf=open(r'%s','rb')\nprint('held', ok, flush=True)\n"
            "time.sleep(900)\n") % (os.path.join(pack, "bin"), mod, held_py)
    ppid = None
    if os.path.isfile(py):
        ppid = start_detached([py, "-B", "-c", code], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, "holder-py.out"))
        pr.register_pid(ppid, "python holder")
    dpid = None
    if ctx.get("defender_original_rtp") is not None:
        dpid = start_detached(["powershell.exe", "-NoProfile", "-Command",
                               "Start-MpScan -ScanType CustomScan -ScanPath '%s'" % pack.replace("'", "''")],
                              stdout_path=os.path.join(pr.ev_dir, "holder-defender.out"))
        pr.register_pid(dpid, "defender scan")
    time.sleep(4)
    f["bash_holder_started"] = bool(bpid and alive(bpid))
    pyout = read_text(os.path.join(pr.ev_dir, "holder-py.out")) or ""
    f["py_holder_started"] = bool(ppid and alive(ppid) and pyout.startswith("held"))
    f["py_holder_import"] = pyout.strip()[:200]
    f["defender_scan_started"] = bool(dpid and (alive(dpid)))
    # ── the state immediately before the update (run-1 lesson; RULINGS FS): every edited file re-hashed ──
    pre = pr.timeline_point("s2_pre_update")
    for key, e in (ctx.get("edits") or {}).items():
        f["pre_update_%s_sha" % key] = None if pre is None else pre.get(e["rel"])
    # ── the update with the app's exact arguments (RULINGS EQ4 D9) ──
    cand = ctx["installers"]["candidate"]
    w = Watcher(pr.install_dir)
    w.start()
    um_limit = m["scenario"]["update_mode_limit_s"]
    t0 = time.monotonic()
    kw = {"creationflags": CREATE_NO_WINDOW} if IS_WIN else {}
    proc = subprocess.Popen([cand, "/P", "/R", "/UPDATE", "/ARGS"], stdin=subprocess.DEVNULL, **kw)
    pr.register_pid(proc.pid, "installer (update mode)")
    f["um_attempted"] = True
    try:
        proc.wait(timeout=um_limit)
        f["update_mode_finished"] = True
        f["um_exit"] = proc.returncode
    except subprocess.TimeoutExpired:
        f["update_mode_finished"] = False
        f["um_exit"] = None
        f["um_killed"] = kill_tree(proc.pid)
    f["um_secs"] = round(time.monotonic() - t0, 2)
    pr.unregister_pid(proc.pid)
    # relaunch (/R) — observed for up to 60 s
    app = None
    t1 = time.monotonic()
    while time.monotonic() - t1 < 60:
        ids = pids_under(pr.install_dir, "cys-app.exe")
        if ids:
            app = ids[0]
            break
        time.sleep(1)
    f["app_relaunched"] = app is not None
    if app:
        pr.register_pid(app, "relaunched app")
    final_exit = f["um_exit"]
    f["fallback_s_used"] = False
    if not f["update_mode_finished"]:
        s = run([cand, "/S"], 900)
        pr.ev_cmd("installer-fallback-S.json", s)
        f["fallback_s_used"] = True
        f["s_exit"] = s["code"]
        final_exit = s["code"]
    f["installer_final_exit"] = final_exit
    time.sleep(2)
    after_inst = pr.timeline_point("s2_after_installer")
    ff = pr.exe("cys-install-failure.txt")
    f["failure_file_present"] = os.path.isfile(ff)
    if f["failure_file_present"]:
        pr.ev("cys-install-failure.txt", read_text(ff) or "")
    # ── pack application: the app's pass, else the app's exact command (driver-reproduced) ──
    want = f["candidate_version"]
    pv = os.path.join(pack, ".pack-version")
    applied_by = None
    if app:
        t2 = time.monotonic()
        while time.monotonic() - t2 < m["scenario"]["app_pack_wait_s"]:
            if (read_text(pv) or "").strip() == want:
                applied_by = "app"
                break
            time.sleep(2)
        f["app_wait_s"] = round(time.monotonic() - t2, 1)
    # GS2: the app's pending-update pass restores the seats after an in-place update (observed up to 30 s)
    f["seats_after_app"] = None
    if applied_by == "app":
        t3 = time.monotonic()
        while time.monotonic() - t3 < 30:
            roles, lr = surface_roles(pr.exe("cys.exe"))
            f["seats_after_app"] = roles
            if roles and "master" in roles:
                break
            time.sleep(2)
        pr.ev("surfaces-after-app.json", lr)
    f["master_seat_after_app"] = None if f["seats_after_app"] is None else ("master" in f["seats_after_app"])
    for a in pids_under(pr.install_dir, "cys-app.exe"):
        terminate(a)
        pr.unregister_pid(a)
    after_app = pr.timeline_point("s2_after_app")
    # the app's exact command, run by the driver ALWAYS with the holders in place (push 2: its exit code is recorded
    # also when the app applied the pack - then it is one further same-version install)
    f["driver_init_ran"] = True
    f["driver_init_after_app"] = applied_by == "app"
    ip = pr.ev_cmd("driver-init-pack.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
    f["driver_init_pack_exit"] = ip["code"]
    f["driver_init_pack_secs"] = ip["secs"]
    f["driver_init_rename_failed"] = "rename 실패" in ip["err"]
    f["driver_init_swap_refused"] = "교체 안 함" in ip["err"]
    f["driver_init_stderr_tail"] = ip["err"][-600:]
    if applied_by is None:
        applied_by = "driver" if ip["code"] == 0 else "driver-failed"
    after_drv = pr.timeline_point("s2_after_driver_init")
    f["pack_dir_id_after_driver"] = dir_identity(pack)
    f["pack_applied_by"] = applied_by
    w.stop_ev.set()
    w.join(5)
    samples = w.samples
    f["watcher_samples"] = len(samples)
    f["cli_absent_max_consecutive"] = R.longest_false_run([s[1] for s in samples]) if samples else None
    f["cysd_absent_max_consecutive"] = R.longest_false_run([s[2] for s in samples]) if samples else None
    f["um_windows"] = sorted(w.windows)
    f["um_window_enum_ok"] = w.window_enum_ok
    pr.ev("watcher.json", {"samples": samples[-3000:], "windows": sorted(w.windows)})
    # ── the daemon rotation the way rotate_daemon does it ──
    dr = pr.ev_cmd("drain.json", pr.cys(["drain"], timeout=45))
    f["drain_exit"] = dr["code"]
    old = [p for p in ctx.get("base_daemon_pids", []) if alive(p)]
    old += [p["pid"] for p in processes() if (p.get("name") or "").lower() == "cysd.exe" and p["pid"] not in old]
    for p in old:
        terminate(p)
        pr.unregister_pid(p)
    time.sleep(2)
    f["old_daemons_stopped"] = not any(alive(p) for p in old)
    before_daemon = pr.timeline_point("s2_before_daemon")
    f["manifest_present_before_install"] = (before_daemon is not None and ".install-manifest.json" in before_daemon)
    start = time.time()
    lp = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, "new-cysd.out"))
    pr.register_pid(lp, "new daemon (launcher)")
    ok, secs, _ = wait_ready(pr.exe("cys.exe"), 90)
    f["new_daemon_ready"] = ok
    f["new_daemon_ready_s"] = secs
    daemons = [p["pid"] for p in processes() if (p.get("name") or "").lower() == "cysd.exe"]
    f["cysd_count_after_rotation"] = len(daemons)
    for d in daemons:
        pr.register_pid(d, "new daemon")
    pr.ctx_updates["new_daemon"] = {"pids": daemons, "start_epoch": start, "ready": ok}
    ns = pr.ev_cmd("new-surface.json", pr.cys(["new-surface", "--role", "master"], timeout=60))
    f["master_surface_after"] = ns["code"] == 0
    after_daemon = pr.timeline_point("s2_after_daemon")
    f["pack_dir_id_after_daemon"] = dir_identity(pack)
    ids = (f["pack_dir_id_pre"], f["pack_dir_id_after_driver"], f["pack_dir_id_after_daemon"])
    f["pack_dir_same_after_update"] = None if None in ids else len(set(ids)) == 1
    # ── what disappeared from the live pack, writer by writer (G01; RULINGS FS5) ──
    for label, a, b in (("installer", pre, after_inst), ("app", after_inst, after_app),
                        ("driver_init", after_app, after_drv), ("daemon", before_daemon, after_daemon)):
        d = R.manifest_delta(a, b)
        f["pack_missing_after_" + label] = None if d is None else d["missing"]
        f["pack_changed_by_" + label] = None if d is None else len(d["changed"]) + len(d["added"])
    d_all = R.manifest_delta(pre, before_daemon)
    f["pack_missing_before_daemon"] = None if d_all is None else d_all["missing"]
    # ── holders' fate, then stop them ──
    f["bash_holder_alive_after"] = bool(bpid and alive(bpid))
    f["py_holder_alive_after"] = bool(ppid and alive(ppid))
    for hp in (bpid, ppid, dpid):
        if hp:
            kill_tree(hp)
            pr.unregister_pid(hp)
    # ── identity of the installation under test ──
    f["cand_marker"] = (read_text(pr.exe("cys-installed-version.txt")) or "").strip() or None
    f["cand_cys_version"] = file_version(pr.exe("cys.exe"))
    f["cand_cysd_version"] = file_version(pr.exe("cysd.exe"))
    f["cand_cys_sha_installed"] = sha256_file(pr.exe("cys.exe"))
    f["cand_cysd_sha_installed"] = sha256_file(pr.exe("cysd.exe"))
    shas = ctx.get("candidate_shas") or {}
    f["cand_hash_match_cys"] = (f["cand_cys_sha_installed"] in shas["cys.exe"]) if shas.get("cys.exe") else None
    f["cand_hash_match_cysd"] = (f["cand_cysd_sha_installed"] in shas["cysd.exe"]) if shas.get("cysd.exe") else None
    f["pack_version_after"] = (read_text(pv) or "").strip() or None
    f["manifest_missing"] = manifest_missing(pack)
    profile = os.path.dirname(os.path.dirname(pack))
    f["pack_leftovers"] = sorted(n for n in os.listdir(os.path.dirname(pack))
                                 if n.lower().startswith("pack") and n != "pack" and not n.startswith("pack-dept-"))
    rt = ctx.get("rt_file")
    f["rt_edit_sha"] = ctx.get("rt_edit_sha")
    f["rt_live_sha_after"] = sha256_file(rt) if rt else None
    f["rt_survived"] = (f["rt_live_sha_after"] == f["rt_edit_sha"]) if f["rt_edit_sha"] else None
    snaps_after = snapshot_dirs(local)
    new = [d for d in snaps_after if d not in snaps_before]
    f["snapshot_new_count"] = len(new)
    meta_ok = False
    if new:
        try:
            json.loads(read_text(os.path.join(new[-1], "meta.json")))
            meta_ok = True
        except (TypeError, ValueError):
            meta_ok = False
    f["snapshot_new_meta_ok"] = meta_ok
    for name in (".install-manifest.json", ".merge-pending.json", ".merge-audit.jsonl", ".pack-version"):
        t = read_text(os.path.join(pack, name))
        if t is not None:
            pr.ev("pack-ledgers/" + name.lstrip("."), t[:2_000_000])
    pr.ev("snapshots.json", {"before": snaps_before, "after": snaps_after})
    # ── where the user's bytes are now: pack.prev, pack-captures, the update snapshot (RULINGS FS; update-rules hint) ──
    edits = ctx.get("edits") or {}
    prev_dir = os.path.join(os.path.dirname(pack), "pack.prev")
    f["pack_prev_exists"] = os.path.isdir(prev_dir)
    # PACK-PREV: the pack.prev planted in S1 is byte-identical; nothing new beside the pack
    planted = _load(os.path.join(pr.work, "pack-prev-planted.json"))
    pm = pack_manifest(prev_dir)
    pr.ev("pack-prev-after.json", pm)
    f["pack_prev_planted"] = planted is not None
    f["pack_prev_unchanged"] = None if planted is None else (pm == planted)
    d = R.manifest_delta(planted, pm) if planted is not None and pm is not None else None
    f["pack_prev_delta"] = None if d is None else {k: len(v) for k, v in d.items()}
    f["staging_leftovers_after"] = staging_leftovers(os.path.dirname(pack))
    cap_root = os.path.join(os.path.dirname(pack), "pack-captures")
    cap_files = []
    if os.path.isdir(cap_root):
        for dp, _dns, fns in os.walk(cap_root):
            for fn in fns:
                cap_files.append(os.path.join(dp, fn))
    f["pack_captures_files"] = len(cap_files)
    snap_listing = {}
    for d in new:
        files_root = os.path.join(d, "files")
        lst = []
        for dp, _dns, fns in os.walk(files_root):
            for fn in fns:
                lst.append(os.path.relpath(os.path.join(dp, fn), files_root).replace("\\", "/"))
        snap_listing[os.path.basename(d)] = {"files": sorted(lst), "meta": read_text(os.path.join(d, "meta.json"))}
    pr.ev("update-snapshot-listing.json", snap_listing)
    pr.ev("pack-captures-listing.json", sorted(os.path.relpath(c, cap_root).replace("\\", "/") for c in cap_files)[:2000])
    for key, e in edits.items():
        rel = e["rel"]
        f["%s_in_pack_prev_sha" % key] = sha256_file(os.path.join(prev_dir, *rel.split("/"))) if f["pack_prev_exists"] else None
        f["%s_in_captures_shas" % key] = sorted({sha256_file(c) for c in cap_files
                                                  if c.replace("\\", "/").endswith("/" + rel)})
        snap = [os.path.join(d, "files", *rel.split("/")) for d in new]
        f["%s_in_snapshot_sha" % key] = next((sha256_file(x) for x in snap if os.path.isfile(x)), None)
    pr.ev("profile-top.json", sorted(os.listdir(os.path.join(profile, ".cys"))) if os.path.isdir(os.path.join(profile, ".cys")) else None)
    pr.restored = True


# ───────────────────────────── S3 — reading the updated state ─────────────────────────────

def _source_const(rel, name):
    txt = read_text(os.path.join(REPO, *rel.split("/"))) or ""
    mm = re.search(r'const %s: &str = "((?:[^"\\]|\\.)*)";' % re.escape(name), txt)
    return mm.group(1) if mm else None


def probe_s3_cli(pr):
    f = pr.facts
    m = matrix()
    for key, args in (("help_exit_root", ["--help"]), ("help_exit_queue", ["queue", "--help"]),
                      ("help_exit_repair", ["repair", "--help"])):
        f[key] = pr.ev_cmd(key + ".json", pr.cys(args, timeout=30))["code"]
    sentence = _source_const("src/bin/cysd/repair.rs", "WINDOWS_SEATLESS_REFUSAL_KO")
    f["seatless_sentence_expected"] = sentence or ""
    c = pr.ev_cmd("cleanup-seatless.json", pr.cys(["repair", "cleanup", "R-0000"], timeout=60))
    f["cleanup_exit"] = c["code"]
    f["cleanup_has_sentence"] = bool(sentence) and sentence in (c["out"] + c["err"])
    s = pr.ev_cmd("status-seatless.json", pr.cys(["repair", "status"], timeout=60))
    f["status_exit"] = s["code"]
    f["status_has_sentence"] = bool(sentence) and sentence in (s["out"] + s["err"])
    ni = _source_const("src/repair/mod.rs", "NOT_IMPLEMENTED_KO")
    beh = {}
    for unit, argv in m["unit_markers"].items():
        r = pr.ev_cmd("marker-%s.json" % unit, pr.cys(argv, timeout=60))
        absent = r["code"] == 99 and bool(ni) and ni in (r["out"] + r["err"])
        beh[unit] = not absent
        f["behaviour_present_" + unit] = not absent
    pr.ctx_updates["units_behaviour"] = beh
    pr.restored = True


def probe_s3_files(pr):
    f = pr.facts
    pack = pr.pack_dir
    edits = pr.ctx.get("edits") or {}
    pending = {}
    txt = read_text(os.path.join(pack, ".merge-pending.json"))
    if txt:
        try:
            doc = json.loads(txt)
            pending = doc.get("entries") if isinstance(doc, dict) and isinstance(doc.get("entries"), dict) else doc
        except ValueError:
            pending = {}
    for key, e in edits.items():
        rel = e["rel"]
        p = os.path.join(pack, *rel.split("/"))
        live = read_bytes(p)
        f[key + "_rel"] = rel
        f[key + "_vendor_sha"] = e["vendor_sha"]
        f[key + "_user_sha"] = e["user_sha"]
        f[key + "_base_sha"] = e["base_sha"]
        f[key + "_user_has_cr"] = e.get("user_has_cr")
        f[key + "_live_sha"] = sha256_bytes(live) if live is not None else None
        f[key + "_live_has_cr"] = (b"\r" in live) if live is not None else None
        f[key + "_park_sha"] = sha256_file(p + ".user") if os.path.isfile(p + ".user") else None
        f[key + "_new_sha"] = sha256_file(p + ".new") if os.path.isfile(p + ".new") else None
        prev = sorted(glob.glob(glob.escape(p) + ".user.prev-*"))
        f[key + "_prev_parks"] = len(prev)
        f[key + "_prev_shas"] = [sha256_file(x) for x in prev]
        if "planted_sha" in e:
            f[key + "_planted_sha"] = e["planted_sha"]
            f[key + "_planted_preexisting"] = e.get("planted_preexisting")
            f[key + "_planted_read_back_ok"] = e.get("planted_read_back_ok")
            f[key + "_planted_in_prev"] = e["planted_sha"] in f[key + "_prev_shas"]
        ent = pending.get(rel) if isinstance(pending, dict) else None
        f[key + "_pending_kind"] = ent.get("kind") if isinstance(ent, dict) else None
        f[key + "_pending_eol"] = ent.get("eol") if isinstance(ent, dict) else None
        f[key + "_pending_reason"] = ent.get("reason") if isinstance(ent, dict) else None
        user_bytes_path = os.path.join(pr.work, "user-bytes", key)
        ub = read_bytes(user_bytes_path)
        f[key + "_user_norm_sha"] = sha256_bytes(R.eol_normalized(ub)) if ub is not None else None
        f[key + "_live_norm_sha"] = sha256_bytes(R.eol_normalized(live)) if live is not None else None
        code, _, _ = _boot_critical_exit(pr.exe("cys.exe"), p)
        f[key + "_listed"] = code
    s1 = os.path.join(pr.work, "obs", "s1_base.json")
    try:
        f["ENC1_user_invalid_utf8"] = json.loads(read_text(s1))["facts"].get("ENC1_user_invalid_utf8")
    except (TypeError, ValueError, KeyError):
        f["ENC1_user_invalid_utf8"] = None
    # facts of S2 the CW7 rows compare with (the state right before the update; where the user's bytes went)
    s2 = _load(os.path.join(pr.work, "obs", "s2_update.json")) or {}
    s2f = s2.get("facts") or {}
    for key in edits:
        for suffix in ("_in_snapshot_sha", "_in_pack_prev_sha", "_in_captures_shas"):
            f[key + suffix] = s2f.get(key + suffix)
        f["pre_update_%s_sha" % key] = s2f.get("pre_update_%s_sha" % key)
    f["manifest_present_before_install"] = s2f.get("manifest_present_before_install")
    pr.ev("pending.json", pending)
    pr.restored = True


def probe_s3_jobs_hooks(pr):
    f = pr.facts
    ctx = pr.ctx
    pack = pr.pack_dir
    base_jobs = ctx.get("base_jobs_builtin") or []
    after = builtin_job_ids(pack)
    f["base_jobs_builtin"] = base_jobs
    f["jobs_after_ids"] = after
    f["jobs_missing_after"] = sorted(set(base_jobs) - set(after or [])) if after is not None else None
    f["ejob_back"] = (ctx.get("ejob") in (after or [])) if ctx.get("ejob") else None
    f["ejob_detected_missing"] = ctx.get("ejob_detected_missing")
    f["ehook_detected_missing"] = ctx.get("ehook_detected_missing")
    settings = ctx.get("settings_files") or settings_candidates(os.environ.get("USERPROFILE", ""))
    base_hooks = ctx.get("base_hooks_lifeline") or []
    r1 = lifeline_registrations(settings)
    f["hooks_after_update"] = sorted(r1)
    f["hooks_missing_after_update"] = sorted(set(base_hooks) - set(r1))
    f["ehook_back_after_update"] = (ctx.get("ehook") in r1) if ctx.get("ehook") else None
    seat_path = os.pathsep.join([pr.install_dir, os.path.join(pr.install_dir, "runtime", "git", "usr", "bin"),
                                 os.path.join(pr.install_dir, "runtime", "git", "bin"),
                                 os.path.join(pr.install_dir, "runtime", "python"), os.environ.get("PATH", "")])
    reg = pr.ev_cmd("reconcile-hooks.json", run(
        [pr.bundled_python(), "-B", os.path.join(pack, "bin", "javis_preflight.py"), "--reconcile-hooks", "--json"],
        180, env=clean_env({"PATH": seat_path})))
    f["registrar_exit"] = reg["code"]
    r2 = lifeline_registrations(settings)
    f["hooks_after_registrar"] = sorted(r2)
    f["hooks_missing_after_registrar"] = sorted(set(base_hooks) - set(r2))
    f["ehook_back_after_registrar"] = (ctx.get("ehook") in r2) if ctx.get("ehook") else None
    pr.ev("registrations.json", {"after_update": r1, "after_registrar": r2, "base": base_hooks})
    pr.restored = True


def probe_s3_query(pr):
    f = pr.facts
    pack = pr.pack_dir
    cys = pr.exe("cys.exe")
    listed_rel = matrix()["scenario"]["query_listed_rel"]
    general_rel = (pr.ctx.get("selection") or {}).get("CG") or "bin/javis_report.py"
    lp = os.path.join(pack, *listed_rel.split("/"))
    gp = os.path.join(pack, *general_rel.split("/"))
    f["listed_rel"] = listed_rel
    f["general_rel"] = general_rel
    drive = lp[0].lower()
    fwd = lp.replace("\\", "/")
    spell = {"backslash": lp, "slash": fwd, "msys": "/" + drive + fwd[2:], "cygdrive": "/cygdrive/" + drive + fwd[2:],
             "upper": lp.upper()}
    sp = short_path(lp)
    f["short_name_available"] = bool(sp and "~" in sp and sp.lower() != lp.lower())
    if f["short_name_available"]:
        spell["short"] = sp
    out = {}
    for k, path in spell.items():
        code, lane, r = _boot_critical_exit(cys, path)
        f["q_listed_" + k] = code
        out["listed_" + k] = r
    code, lane, r = _boot_critical_exit(cys, gp)
    f["q_general_backslash"] = code
    out["general_backslash"] = r
    code, lane, r = _boot_critical_exit(cys, "/" + gp[0].lower() + gp.replace("\\", "/")[2:])
    f["q_general_msys"] = code
    # junctions inside the pack (removed after)
    ja = os.path.join(pack, "tw1-alias-hooks")
    jb = os.path.join(pack, "tw1-alias-bin")
    mk1 = run(["cmd", "/c", "mklink", "/J", ja, os.path.dirname(lp)], 30)
    mk2 = run(["cmd", "/c", "mklink", "/J", jb, os.path.dirname(gp)], 30)
    f["junction_created"] = mk1["code"] == 0 and os.path.isdir(ja) and mk2["code"] == 0 and os.path.isdir(jb)
    if f["junction_created"]:
        code, lane, r = _boot_critical_exit(cys, os.path.join(ja, os.path.basename(lp)))
        f["q_listed_junction"] = code
        out["listed_junction"] = r
        code, lane, r = _boot_critical_exit(cys, os.path.join(jb, os.path.basename(gp)))
        f["q_general_junction"] = code
        jm = os.path.join(ja, os.path.basename(lp)).replace("\\", "/")
        code, lane, r = _boot_critical_exit(cys, "/" + jm[0].lower() + jm[2:])
        f["q_listed_junction_msys"] = code
        out["listed_junction_msys"] = r
    # DY3 / T5c report 4: '..' after a junction - <pack>/tw1-jx -> <pack>/hooks/tw1-sub; the spelling
    # <pack>/tw1-jx/../<name> is <pack>/<name> read textually and <pack>/hooks/<name> read physically
    sub = os.path.join(os.path.dirname(lp), "tw1-sub")
    jx = os.path.join(pack, "tw1-jx")
    os.makedirs(sub, exist_ok=True)
    mk3 = run(["cmd", "/c", "mklink", "/J", jx, sub], 30)
    f["dotdot_junction_created"] = mk3["code"] == 0 and os.path.isdir(jx)
    if f["dotdot_junction_created"]:
        nat = os.path.join(jx, "..", os.path.basename(lp))
        f["q_dotdot_native"] = _boot_critical_exit(cys, nat)[0]
        f["q_dotdot_msys"] = _boot_critical_exit(cys, _msys(jx) + "/../" + os.path.basename(lp))[0]
        lands = {}
        for who in ("bash", "python"):
            name = "tw1-dotdot-%s.txt" % who
            lex, phys = os.path.join(pack, name), os.path.join(os.path.dirname(lp), name)
            if who == "bash":
                wr = run([pr.bundled_hook_bash(), "-c", "printf x > '%s/../%s'" % (_msys(jx), name)], 30, env=clean_env())
            else:
                wr = run([pr.bundled_python(), "-I", "-c", "open(r'%s','w').write('x')" % os.path.join(jx, "..", name)],
                         30, env=clean_env())
            out["dotdot_write_" + who] = wr
            lands[who] = ("physical" if os.path.isfile(phys) else "lexical" if os.path.isfile(lex) else "none")
            for x in (lex, phys):
                if os.path.isfile(x):
                    os.remove(x)
        f["dotdot_bash_lands"] = lands["bash"]
        f["dotdot_python_lands"] = lands["python"]
        # a spelling that reaches the listed file when WRITTEN must be classified listed (0 / 2); one that does not
        # reach it needs no protection (None = nothing to judge)
        f["dotdot_msys_protects"] = (f["q_dotdot_msys"] in (0, 2)) if lands["bash"] == "physical" else None
        f["dotdot_native_protects"] = (f["q_dotdot_native"] in (0, 2)) if lands["python"] == "physical" else None
    rm3 = run(["cmd", "/c", "rmdir", jx], 30) if os.path.lexists(jx) else None
    shutil.rmtree(sub, ignore_errors=True)
    f["dotdot_removed"] = not os.path.lexists(jx) and not os.path.exists(sub)
    out["dotdot"] = [mk3, rm3]
    rm1 = run(["cmd", "/c", "rmdir", ja], 30) if os.path.lexists(ja) else None
    rm2 = run(["cmd", "/c", "rmdir", jb], 30) if os.path.lexists(jb) else None
    f["junction_removed"] = not os.path.lexists(ja) and not os.path.lexists(jb) and os.path.isfile(lp) and os.path.isfile(gp)
    out["mklink"] = [mk1, mk2]
    out["rmdir"] = [rm1, rm2]
    pr.ev("queries.json", out)
    pr.restored = f["junction_removed"] and f["dotdot_removed"]


def probe_s3_home(pr):
    f = pr.facts
    pack = pr.pack_dir
    cys = pr.exe("cys.exe")
    home = os.path.join(pr.work, "home")
    fake = os.path.join(home, ".cys", "pack", "hooks", "session-start.sh")
    write_bytes(fake, b"")
    lp = os.path.join(pack, "hooks", "session-start.sh")
    f["real_pack_dir"] = norm_path(pack)

    def lane(env):
        code, ln, r = _boot_critical_exit(cys, lp, env=env)
        return (norm_path(ln) if ln else "<none: exit %s>" % code), r

    f["lp_real_default"], r0 = lane(clean_env())
    f["lp_real_under_override"], r1 = lane(clean_env({"USERPROFILE": home}))
    f["lp_real_under_home"], r2 = lane(clean_env({"HOME": home}))
    code, ln, r3 = _boot_critical_exit(cys, fake, env=clean_env({"USERPROFILE": home}))
    f["lp_fake_under_override"] = norm_path(ln) if ln else "<none: exit %s>" % code
    pyc = "import os,pathlib,json;print(json.dumps([os.path.expanduser('~'), str(pathlib.Path.home())]))"
    py = pr.bundled_python()

    def pyhome(env):
        r = run([py, "-B", "-c", pyc], 30, env=env)
        try:
            return [norm_path(x) for x in json.loads(r["out"])], r
        except ValueError:
            return None, r

    f["py_home_default"], p0 = pyhome(clean_env())
    f["py_home_under_override"], p1 = pyhome(clean_env({"USERPROFILE": home}))
    f["home_override_dir"] = norm_path(home)
    pr.ev("home.json", {"default": r0, "userprofile": r1, "home": r2, "fake": r3, "py_default": p0, "py_override": p1})
    pr.restored = True


WATCH_PATTERNS = (r"ReadDirectoryChangesW", r"FindFirstChangeNotification", r"\bnotify::", r'^\+\s*notify\s*=\s*"',
                  r"\buse notify\b", r"FileSystemWatcher", r"^\+\s*(?:import|from) watchdog\b", r"inotify_add_watch",
                  r"FSEventStream", r"\bkqueue\(")


def probe_s3_diff(pr):
    f = pr.facts
    base = "v" + matrix()["base"]["version"]
    r = _git(["rev-parse", "--verify", base + "^{commit}"])
    f["base_ref_found"] = r["code"] == 0
    if not f["base_ref_found"]:
        pr.ev_cmd("rev-parse.json", r)
        return
    d = run(["git", "-C", REPO, "diff", "-U0", base, "HEAD", "--", "src", "src-tauri/src", "cysjavis-pack",
             "Cargo.toml", "src-tauri/Cargo.toml"], 300)
    hits, ctl, cur, files = [], 0, None, set()
    rx = [re.compile(p) for p in WATCH_PATTERNS]
    for ln in d["out"].splitlines():
        if ln.startswith("+++ "):
            cur = ln[6:] if ln.startswith("+++ b/") else ln[4:]
            files.add(cur)
            continue
        if not ln.startswith("+") or ln.startswith("+++"):
            continue
        if "boot_critical" in ln:
            ctl += 1
        for p in rx:
            if p.search(ln):
                hits.append("%s: %s" % (cur, ln[:200]))
    f["diff_files"] = len(files)
    f["watcher_hits"] = hits
    f["control_hits"] = ctl
    pr.ev("watcher-search.json", {"patterns": WATCH_PATTERNS, "hits": hits, "control_token": "boot_critical",
                                  "control_hits": ctl, "files": len(files), "diff_exit": d["code"]})
    pr.restored = True


def probe_s3_dy1(pr):
    f = pr.facts
    bpy = pr.bundled_python()
    f["bundled_python_found"] = os.path.isfile(bpy)
    if not f["bundled_python_found"]:
        return
    rpy = sys.executable
    root = os.path.join(pr.work, "dy1")
    marks = os.path.join(root, "markers")
    os.makedirs(marks, exist_ok=True)

    def plant(path, name):
        write_bytes(path, ("import os\nopen(os.path.join(r'%s', '%s'), 'w').close()\n" % (marks, name)).encode("utf-8"))

    plant(os.path.join(root, "pp_abs", "json.py"), "pp_abs")
    plant(os.path.join(root, "pp_rel", "json.py"), "pp_rel")
    plant(os.path.join(root, "cwd", "json.py"), "cwd_c")
    plant(os.path.join(root, "sd", "json.py"), "script_dir")
    write_bytes(os.path.join(root, "sd", "target.py"), b"import json\n")
    plant(os.path.join(root, "startup.py"), "startup")
    plant(os.path.join(root, "ub", "Python312", "site-packages", "usercustomize.py"), "userbase")
    plant(os.path.join(root, "sc", "sitecustomize.py"), "sitecustomize")
    os.makedirs(os.path.join(root, "fakehome"), exist_ok=True)
    empty = os.path.join(root, "empty-stdin.txt")
    write_bytes(empty, b"")
    FLAGS = ["-I", "-S", "-B", "-X", "utf8"]
    cases = {
        "pp_abs": ({"PYTHONPATH": os.path.join(root, "pp_abs")}, root, ["-c", "import json"], None),
        "pp_rel": ({"PYTHONPATH": "pp_rel"}, root, ["-c", "import json"], None),
        "cwd_c": ({}, os.path.join(root, "cwd"), ["-c", "import json"], None),
        "script_dir": ({}, root, [os.path.join(root, "sd", "target.py")], None),
        "startup": ({"PYTHONSTARTUP": os.path.join(root, "startup.py")}, root, ["-i"], empty),
        "userbase": ({"PYTHONUSERBASE": os.path.join(root, "ub")}, root, ["-c", "pass"], None),
        "sitecustomize": ({"PYTHONPATH": os.path.join(root, "sc")}, root, ["-c", "pass"], None),
    }
    log = {}

    def one(exe, flags, case):
        env_extra, cwd, args, stdin = cases[case]
        for fn in os.listdir(marks):
            os.remove(os.path.join(marks, fn))
        r = run([exe] + flags + args, 60, env=clean_env(env_extra), cwd=cwd, stdin_path=stdin)
        return os.path.isfile(os.path.join(marks, case)), r

    for case in cases:
        f["bundled_flags_" + case], log["bundled_flags_" + case] = one(bpy, FLAGS, case)
        f["runner_plain_" + case], log["runner_plain_" + case] = one(rpy, [], case)
        f["bundled_plain_" + case], log["bundled_plain_" + case] = one(bpy, [], case)
    h1 = run([bpy] + FLAGS + ["-c", "import sys; print('ok', sys.flags.isolated)"], 60,
             env=clean_env({"PYTHONHOME": os.path.join(root, "fakehome")}))
    f["bundled_flags_home_ok"] = h1["code"] == 0 and h1["out"].startswith("ok")
    h2 = run([rpy, "-c", "pass"], 60, env=clean_env({"PYTHONHOME": os.path.join(root, "fakehome")}))
    f["runner_plain_home_effect"] = h2["code"] != 0
    log["home"] = [h1, h2]
    fl = run([bpy] + FLAGS + ["-c", "import sys; print(sys.flags.isolated, sys.flags.no_site, sys.flags.dont_write_bytecode, sys.flags.utf8_mode)"], 60, env=clean_env())
    f["flags_accepted_exit"] = fl["code"]
    parts = fl["out"].split()
    f["utf8_flag"] = int(parts[3]) if len(parts) >= 4 and parts[3].isdigit() else None
    al = run([bpy] + FLAGS + ["-c", "import json; print(json.dumps({'ok': 1}))"], 60, env=clean_env())
    f["allowed_ok"] = al["code"] == 0 and '{"ok": 1}' in al["out"]
    log["flags"] = fl
    log["allowed"] = al
    pr.ev("dy1.json", log)
    pr.restored = True


def _master_queue(pr):
    """Every queued entry of the lane (active and expired). The only surface on the runner is the master's."""
    lst = pr.cys(["list"], timeout=30)
    if "role=master" not in lst["out"]:
        return None, {"list": lst}
    q = pr.cys(["queue", "list", "--json", "--expired"], timeout=30)
    try:
        entries = json.loads(q["out"])
        return (len(entries) if isinstance(entries, list) else None), {"list": lst, "queue": q}
    except ValueError:
        return None, {"list": lst, "queue": q}


def probe_s3_intake(pr):
    f = pr.facts
    rel = (pr.ctx.get("selection") or {}).get("CG") or "bin/javis_report.py"
    repairs = os.path.join(pr.ctx["paths"]["local_dir"], "repairs")
    before_files = len(glob.glob(os.path.join(repairs, "**", "*"), recursive=True))
    f["master_queue_before"], q0 = _master_queue(pr)
    r1 = pr.ev_cmd("report-1.json", pr.cys(["repair", "report", "--file", rel, "--error", "tw1 intake probe: the same failure"], timeout=60))
    r2 = pr.ev_cmd("report-2.json", pr.cys(["repair", "report", "--file", rel, "--error", "tw1 intake probe: the same failure"], timeout=60))
    time.sleep(5)
    f["master_queue_after"], q1 = _master_queue(pr)
    f["report1_exit"] = r1["code"]
    f["report2_exit"] = r2["code"]
    if f["master_queue_before"] is not None and f["master_queue_after"] is not None:
        f["queue_growth"] = f["master_queue_after"] - f["master_queue_before"]
    f["repairs_files_new"] = len(glob.glob(os.path.join(repairs, "**", "*"), recursive=True)) - before_files
    pr.ev("queues.json", {"before": q0, "after": q1})
    pr.restored = True


def probe_s3_listen(pr):
    f = pr.facts
    nd = pr.ctx.get("new_daemon") or {}
    f["new_daemon_ready"] = nd.get("ready")
    pids = set(nd.get("pids") or [])
    start = nd.get("start_epoch")
    if not pids or not start:
        return
    seen = set(pids)
    list_ok = list_fail = 0
    target = start + 610
    last_alive = None
    n = 0
    while time.time() < target:
        now_pids = {p["pid"] for p in processes() if (p.get("name") or "").lower() == "cysd.exe"}
        seen |= now_pids
        if any(alive(p) for p in pids):
            last_alive = time.time()
        if n % 6 == 0:
            if pr.cys(["list"], timeout=15)["code"] == 0:
                list_ok += 1
            else:
                list_fail += 1
        n += 1
        time.sleep(5)
    f["alive_at_end"] = any(alive(p) for p in pids)
    if f["alive_at_end"]:
        last_alive = time.time()
    f["stable_s"] = round((last_alive or start) - start, 1)
    f["pid_changes"] = len(seen - pids)
    f["list_ok_count"] = list_ok
    f["list_fail_count"] = list_fail
    f["cysd_count_end"] = count_named("cysd.exe")
    pr.restored = True


def probe_s3_siblings(pr):
    f = pr.facts
    token = os.environ.get("GH_TOKEN", "")
    sha = os.environ.get("GITHUB_SHA", "")
    if not (token and sha and repo_slug()):
        return
    data = api("/repos/%s/actions/runs?head_sha=%s&per_page=100" % (repo_slug(), sha), token)
    own = os.environ.get("GITHUB_RUN_ID")
    runs = []
    for r in data.get("workflow_runs", []):
        path = (r.get("path") or "").split("@")[0]
        if str(r.get("id")) == own or not path.endswith(("windows-build.yml", "windows-health.yml")):
            continue
        runs.append({"workflow": os.path.basename(path), "status": r.get("status"), "conclusion": r.get("conclusion"),
                     "id": r.get("id"), "event": r.get("event")})
    f["siblings"] = runs
    f["siblings_found"] = len(runs)
    f["siblings_completed"] = all(r["status"] == "completed" for r in runs) if runs else False
    f["siblings_red"] = sorted("%s:%s" % (r["workflow"], r["conclusion"]) for r in runs
                               if r["status"] == "completed" and r["conclusion"] not in ("success", "skipped", "neutral"))
    pr.ev("siblings.json", runs)
    pr.restored = True


# ───────────────────────────── S7 — OS primitives (scratch folder; the installed bash / Python as holders) ─────────────────────────────

_GR, _GW = 0x80000000, 0x40000000
_LIST_DIR, _SYNC = 0x0001, 0x00100000
_SR, _SW, _SD = 1, 2, 4
_OPEN_EXISTING, _CREATE_ALWAYS = 3, 2
_BACKUP, _REPARSE = 0x02000000, 0x00200000
_REPLACE = 0x1


def _open_h(path, access, share, disp=_OPEN_EXISTING, flags=0):
    h = _k32.CreateFileW(path, access, share, None, disp, flags, None)
    err = ctypes.get_last_error()
    return (None if h in (None, wt.HANDLE(-1).value) else h), err


def _move(src, dst, flags=0):
    ok = bool(_k32.MoveFileExW(src, dst, flags))
    return {"ok": ok, "error": 0 if ok else ctypes.get_last_error()}


def _wait_dead(pid, limit):
    t0 = time.time()
    while alive(pid) and time.time() - t0 < limit:
        time.sleep(0.5)
    if alive(pid):
        kill_tree(pid)


def probe_s7_primitives(pr):
    """G06, G16.prim (+ the 50 two-move gaps), EC2.prim, EC4.prim, T7C.prim, G09.flush, G09.kill - observations only."""
    f = pr.facts
    root = os.path.join(pr.work, "prim")
    os.makedirs(root, exist_ok=True)
    bash, py = pr.bundled_hook_bash(), pr.bundled_python()
    f["prim_runtime_found"] = os.path.isfile(bash) and os.path.isfile(py)
    if not f["prim_runtime_found"]:
        return
    log = {}
    # ── G06: a running Git Bash script / an imported Python module, renamed, rewritten in place, replaced ──
    d = os.path.join(root, "g06")
    os.makedirs(d, exist_ok=True)
    body = b"echo L1\nsleep 4\necho L2\nsleep 4\necho L3\n"
    alt = b"echo X1\nsleep 4\necho X2\nsleep 4\necho X3\n"
    for case in ("a_rename", "b_rewrite", "c_move_in"):
        h = os.path.join(d, case + ".sh")
        write_bytes(h, body)
        out = os.path.join(d, case + ".out")
        pid = start_detached([bash, h.replace("\\", "/")], env=clean_env(), stdout_path=out)
        pr.register_pid(pid, "g06 " + case)
        time.sleep(1.5)
        if case == "a_rename":
            r = _move(h, h + ".aside")
        elif case == "b_rewrite":
            try:
                with open(h, "r+b") as fh:
                    fh.write(alt)
                r = {"ok": True, "error": 0}
            except OSError as e:
                r = {"ok": False, "error": getattr(e, "winerror", None) or e.errno}
        else:
            write_bytes(h + ".new", alt)
            r = _move(h + ".new", h, _REPLACE)
        _wait_dead(pid, 20)
        pr.unregister_pid(pid)
        printed = (read_text(out) or "").split()
        f["g06_%s_ok" % case] = r["ok"]
        f["g06_%s_error" % case] = r["error"]
        f["g06_%s_printed" % case] = printed
        log[case] = {"action": r, "printed": printed}
    mod = os.path.join(d, "tw1mod.py")
    for case in ("d_rename", "e_move_in", "f_rewrite"):
        write_bytes(mod, b"def v():\n    return 'old'\n")
        out = os.path.join(d, case + ".pyout")
        code = ("import sys,time\nsys.path.insert(0, r'%s')\nimport tw1mod\n"
                "for i in range(10):\n print(tw1mod.v(), flush=True)\n time.sleep(1)\n") % d
        pid = start_detached([py, "-B", "-c", code], env=clean_env(), stdout_path=out)
        pr.register_pid(pid, "g06 " + case)
        time.sleep(2)
        if case == "d_rename":
            r = _move(mod, mod + ".aside")
        elif case == "e_move_in":
            write_bytes(mod + ".new", b"def v():\n    return 'new'\n")
            r = _move(mod + ".new", mod, _REPLACE)
        else:
            try:
                write_bytes(mod, b"def v():\n    return 'rew'\n")
                r = {"ok": True, "error": 0}
            except OSError as e:
                r = {"ok": False, "error": getattr(e, "winerror", None) or e.errno}
        _wait_dead(pid, 20)
        pr.unregister_pid(pid)
        printed = (read_text(out) or "").split()
        f["g06_%s_ok" % case] = r["ok"]
        f["g06_%s_kept_old_code" % case] = bool(printed) and printed[-1] == "old"
        log[case] = {"action": r, "printed": printed}
        if os.path.exists(mod + ".aside"):
            os.replace(mod + ".aside", mod)
    c = os.path.join(d, "ctl.sh")
    write_bytes(c, body)
    f["g06_ctl_rename_ok"] = _move(c, c + ".aside")["ok"]
    write_bytes(c + ".new", alt)
    f["g06_ctl_move_in_ok"] = _move(c + ".new", c + ".aside", _REPLACE)["ok"]
    # ── G16.prim: the deposit-first form on held files; the empty gap; EC2.prim: no-replace onto an existing name ──
    d = os.path.join(root, "g16")
    vault = os.path.join(d, "vault")
    os.makedirs(vault, exist_ok=True)

    def deposit_first(live, new, tag):
        slot = os.path.join(vault, "%s-%d" % (tag, time.perf_counter_ns()))
        m1 = _move(live, slot, 0)
        m2 = _move(new, live, 0) if m1["ok"] else {"ok": False, "error": None, "skipped": True}
        return {"move_live_to_vault": m1, "place_new": m2, "live_exists": os.path.exists(live),
                "vault_has_old": os.path.exists(slot)}

    live_b = os.path.join(d, "held-bash.sh")
    write_bytes(live_b, b"echo live\n")
    hb = start_detached([bash, "-c", 'exec 3< "%s"; sleep 60' % live_b.replace("\\", "/")], env=clean_env())
    live_p = os.path.join(d, "held-py.py")
    write_bytes(live_p, b"x = 1\n")
    hp = start_detached([py, "-B", "-c", "import time\nf=open(r'%s','rb')\ntime.sleep(60)\n" % live_p], env=clean_env())
    for x in (hb, hp):
        pr.register_pid(x, "g16 holder")
    live_s = os.path.join(d, "held-shareread.txt")
    write_bytes(live_s, b"live\n")
    h3, _ = _open_h(live_s, _GR, _SR)
    time.sleep(2)
    for tag, live in (("bash", live_b), ("python", live_p), ("shareread", live_s)):
        write_bytes(live + ".incoming", b"new content\n")
        r = deposit_first(live, live + ".incoming", tag)
        f["g16_%s_deposit_ok" % tag] = r["move_live_to_vault"]["ok"]
        f["g16_%s_error" % tag] = r["move_live_to_vault"]["error"]
        f["g16_%s_nothing_lost" % tag] = r["live_exists"] or r["vault_has_old"]
        log["g16_" + tag] = r
    if h3:
        _k32.CloseHandle(h3)
    for x in (hb, hp):
        kill_tree(x)
        pr.unregister_pid(x)
    lc = os.path.join(d, "unheld.txt")
    write_bytes(lc, b"live\n")
    write_bytes(lc + ".incoming", b"new\n")
    rc = deposit_first(lc, lc + ".incoming", "ctl")
    f["g16_ctl_deposit_ok"] = rc["move_live_to_vault"]["ok"] and rc["place_new"]["ok"]
    gaps = []
    gl = os.path.join(d, "gap.txt")
    write_bytes(gl, b"v0\n")
    for i in range(50):
        write_bytes(gl + ".n", b"v%d\n" % (i + 1))
        a = _move(gl, os.path.join(vault, "gap-%d" % i), 0)
        t2 = time.perf_counter_ns()
        b = _move(gl + ".n", gl, 0)
        t3 = time.perf_counter_ns()
        if a["ok"] and b["ok"]:
            gaps.append(round((t3 - t2) / 1000.0, 1))
    f["g16_gap_runs"] = len(gaps)
    f["g16_gap_us"] = ({"min": min(gaps), "median": sorted(gaps)[len(gaps) // 2], "max": max(gaps)} if gaps else None)
    tgt, src = os.path.join(d, "occupied.txt"), os.path.join(d, "placing.txt")
    write_bytes(tgt, b"occupant\n")
    write_bytes(src, b"placing\n")
    r = _move(src, tgt, 0)
    f["ec2_noreplace_ok"] = r["ok"]
    f["ec2_noreplace_error"] = r["error"]
    f["ec2_target_unchanged"] = read_bytes(tgt) == b"occupant\n"
    f["ec2_source_kept"] = read_bytes(src) == b"placing\n"
    f["ec2_ctl_free_ok"] = _move(src, os.path.join(d, "free-name.txt"), 0)["ok"]
    # ── EC4.prim: facts from the open handle ──
    d = os.path.join(root, "ec4")
    os.makedirs(os.path.join(d, "real"), exist_ok=True)
    a = os.path.join(d, "ledger.jsonl")
    write_bytes(a, b"{}\n")
    f["ec4_hardlink_created"] = bool(_k32.CreateHardLinkW(os.path.join(d, "alias.jsonl"), a, None))

    def links(p):
        h, _e = _open_h(p, _GR, _SR | _SW | _SD)
        if not h:
            return None
        try:
            info = _BHFI()
            return int(info.nNumberOfLinks) if _k32.GetFileInformationByHandle(h, ctypes.byref(info)) else None
        finally:
            _k32.CloseHandle(h)

    f["ec4_links_hardlinked"] = links(a)
    single = os.path.join(d, "single.jsonl")
    write_bytes(single, b"{}\n")
    f["ec4_links_single"] = links(single)
    write_bytes(os.path.join(d, "real", "x.txt"), b"x\n")
    j = os.path.join(d, "junction")
    mk = run(["cmd", "/c", "mklink", "/J", j, os.path.join(d, "real")], 30)
    f["ec4_junction_created"] = mk["code"] == 0 and os.path.isdir(j)
    final = None
    h, _e = _open_h(os.path.join(j, "x.txt"), _GR, _SR | _SW | _SD)
    if h:
        buf = ctypes.create_unicode_buffer(32768)
        n = _k32.GetFinalPathNameByHandleW(h, buf, 32768, 0)
        final = buf.value if n else None
        _k32.CloseHandle(h)
    f["ec4_final_path_resolves_junction"] = bool(final) and "\\junction\\" not in final.lower() and final.lower().endswith("\\real\\x.txt")
    run(["cmd", "/c", "rmdir", j], 30)
    # ── T7C.prim: a folder held without FILE_SHARE_DELETE (RULINGS DB3's share mode) ──
    held = os.path.join(root, "t7c", "store")
    os.makedirs(held, exist_ok=True)
    h, err = _open_h(held, _LIST_DIR | _SYNC, _SR | _SW, _OPEN_EXISTING, _BACKUP | _REPARSE)
    f["t7c_held"] = h is not None
    if h:
        try:
            p = os.path.join(held, "copy.tmp")
            try:
                write_bytes(p, b"c\n")
                f["t7c_create_inside_ok"] = True
            except OSError:
                f["t7c_create_inside_ok"] = False
            f["t7c_move_inside_ok"] = _move(p, os.path.join(held, "copy.bin"), 0)["ok"]
            r = _move(held, held + "-renamed", 0)
            f["t7c_rename_held_folder_ok"] = r["ok"]
            f["t7c_rename_held_folder_error"] = r["error"]
        finally:
            _k32.CloseHandle(h)
    if os.path.isdir(held + "-renamed"):
        os.replace(held + "-renamed", held)
    f["t7c_ctl_rename_unheld_ok"] = _move(held, held + "-renamed", 0)["ok"]
    # ── G09.flush: FlushFileBuffers on a file and on a directory (RULINGS EC7) ──
    d = os.path.join(root, "g09")
    os.makedirs(d, exist_ok=True)
    h, _e = _open_h(os.path.join(d, "file.bin"), _GW, _SR, _CREATE_ALWAYS, 0)
    f["g09_file_flush_ok"] = bool(h and _k32.FlushFileBuffers(h))
    f["g09_file_flush_error"] = 0 if f["g09_file_flush_ok"] else ctypes.get_last_error()
    if h:
        _k32.CloseHandle(h)
    hd, err = _open_h(d, _GW, _SR | _SW | _SD, _OPEN_EXISTING, _BACKUP)
    f["g09_dir_open_for_flush"] = hd is not None
    f["g09_dir_open_error"] = 0 if hd else err
    f["g09_dir_flush_ok"] = None
    if hd:
        f["g09_dir_flush_ok"] = bool(_k32.FlushFileBuffers(hd))
        f["g09_dir_flush_error"] = 0 if f["g09_dir_flush_ok"] else ctypes.get_last_error()
        _k32.CloseHandle(hd)
    # ── G09.kill: a writer killed mid-record (process death only - NOT a power loss) ──
    torn = {"flush": 0, "noflush": 0}
    for mode in ("flush", "noflush"):
        for i in range(10):
            led = os.path.join(d, "%s-%d.jsonl" % (mode, i))
            code = ("import json,os\nf=open(r'%s','ab')\nn=0\nwhile True:\n"
                    " f.write((json.dumps({'n':n,'pad':'x'*200})+'\\n').encode())\n%s n+=1\n") % (
                led, " f.flush(); os.fsync(f.fileno())\n" if mode == "flush" else "")
            wp = subprocess.Popen([py, "-B", "-c", code], env=clean_env(),
                                  creationflags=CREATE_NO_WINDOW if IS_WIN else 0)
            time.sleep(0.3 + 0.07 * i)
            wp.kill()
            wp.wait(10)
            lines = (read_bytes(led) or b"").split(b"\n")
            for ln in lines[:-1]:
                try:
                    json.loads(ln)
                except ValueError:
                    torn[mode] += 1
    f["g09_kill_torn_flush"] = torn["flush"]
    f["g09_kill_torn_noflush"] = torn["noflush"]
    pr.ev("primitives.json", log)
    pr.restored = True


# ───────────────────────────── S6 — G01 variants (each on a fresh base) ─────────────────────────────

def _kill_product(install_dir):
    killed = []
    for p in processes():
        name = (p.get("name") or "").lower()
        path = norm_path(p.get("path"))
        if name in ("cys.exe", "cysd.exe", "cys-app.exe") or (path and path.startswith(norm_path(install_dir) + "\\")):
            if terminate(p["pid"]):
                killed.append(p["pid"])
    time.sleep(1)
    return killed


def _reset_installation(pr):
    """Uninstall and remove the installation and the user's .cys (the runner is ephemeral). -> facts dict."""
    res = {"killed": _kill_product(pr.install_dir)}
    un = os.path.join(pr.install_dir, "uninstall.exe")
    if os.path.isfile(un):
        r = run([un, "/S"], 300)
        res["uninstall_exit"] = r["code"]
        time.sleep(3)
    errs = []
    for d in (pr.install_dir, os.path.dirname(pr.pack_dir)):
        if os.path.exists(d):
            shutil.rmtree(d, onerror=lambda fn, path, exc: errs.append("%s: %s" % (path, exc[1])))
    res["rmtree_errors"] = errs[:20]
    res["clean"] = not os.path.exists(pr.install_dir) and not os.path.exists(os.path.dirname(pr.pack_dir))
    return res


_HELD_REL = {"bash": lambda sc: "hooks/" + sc["holder_hook"],
             "python": lambda sc: "bin/" + sc["holder_module"] + ".py",
             "early": lambda sc: sc["early_holder_file"]}


def _manifest_entries(pack):
    """The install record {rel: sha256} (flat or under "files"); None when absent or unreadable."""
    txt = read_text(os.path.join(pack, ".install-manifest.json"))
    try:
        doc = json.loads(txt) if txt else None
    except ValueError:
        return None
    doc = doc.get("files") if isinstance(doc, dict) and isinstance(doc.get("files"), dict) else doc
    return doc if isinstance(doc, dict) else None


def _start_holder(pr, kind, pack, tag):
    """One holder of run 1's kinds: 'bash' (Git Bash fd on a hook), 'python' (a pack module imported + its file open),
    'early' (Python holding a file that sorts early in the pack root, without delete sharing)."""
    m = matrix()["scenario"]
    out = os.path.join(pr.ev_dir, "%s-holder-%s.out" % (tag, kind))
    if kind == "bash":
        hf = os.path.join(pack, "hooks", m["holder_hook"])
        return start_detached([pr.bundled_hook_bash(), "-c", 'exec 3< "%s"; sleep 900' % hf.replace("\\", "/")],
                              env=clean_env(), stdout_path=out)
    if kind == "python":
        mod = m["holder_module"]
        held = os.path.join(pack, "bin", mod + ".py")
        code = ("import sys,time\nsys.path.insert(0, r'%s')\nimport %s\nf=open(r'%s','rb')\nprint('held', flush=True)\n"
                "time.sleep(900)\n") % (os.path.join(pack, "bin"), mod, held)
        return start_detached([pr.bundled_python(), "-B", "-c", code], env=clean_env(), stdout_path=out)
    if kind == "early":
        held = os.path.join(pack, m["early_holder_file"])
        code = "import time\nf=open(r'%s','rb')\nprint('held', flush=True)\ntime.sleep(900)\n" % held
        return start_detached([pr.bundled_python(), "-B", "-c", code], env=clean_env(), stdout_path=out)
    raise ValueError(kind)


def _first_start(pr, installer, tag):
    """RULINGS FW3: from a clean state (no installation, no ~/.cys), install with /S and launch cysd.exe with NO pack
    folder - seconds from the launch to its first answer on the pipe (it installs the whole pack before it listens)."""
    out = {"reset_clean": _reset_installation(pr)["clean"]}
    r = run([installer, "/S"], 900)
    time.sleep(2)
    _kill_product(pr.install_dir)
    out["install_exit"] = r["code"]
    out["pack_absent_before"] = not os.path.exists(pr.pack_dir)
    t0 = time.monotonic()
    pid = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, tag + "-cysd.out"))
    pr.register_pid(pid, tag + " daemon")
    ok, secs, _ = wait_ready(pr.exe("cys.exe"), 180, t0=t0)
    out["first_answer_s"] = secs if ok else None
    m = pack_manifest(pr.pack_dir)
    out["pack_files_after"] = None if m is None else len(m)
    _kill_product(pr.install_dir)
    pr.unregister_pid(pid)
    return out


def probe_s6_variants(pr):
    """G01 variants (RULINGS FS5): for each holder set, a fresh v0.14.56 base with the L1 edit, the candidate installed
    with /S, then the app's exact `cys init-pack --no-install-hook` WITH the holders, then the daemon's install - the
    pack manifest after every writer. The control has no holder."""
    f = pr.facts
    ctx = pr.ctx
    m = matrix()
    sc = m["scenario"]
    base_inst = ctx["installers"]["base"]
    cand = ctx["installers"]["candidate"]
    want = ctx["versions"]["candidate"]
    rel = (ctx.get("selection") or {}).get("L1")
    f["variants_L1_rel"] = rel
    if not (base_inst and cand and rel):
        return
    vendor = read_bytes(os.path.join(REPO, "cysjavis-pack", *rel.split("/")))
    # ── RULINGS FW3: the daemon's first start with an empty pack, base and candidate, each from a clean state ──
    fs_base = _first_start(pr, base_inst, "firststart-base")
    fs_cand = _first_start(pr, cand, "firststart-candidate")
    pr.ev("first-start.json", {"base": fs_base, "candidate": fs_cand})
    f["first_start_base_s"] = fs_base["first_answer_s"]
    f["first_start_candidate_s"] = fs_cand["first_answer_s"]
    f["first_start_base_clean"] = fs_base["reset_clean"] and fs_base["pack_absent_before"] and fs_base["install_exit"] == 0
    f["first_start_candidate_clean"] = fs_cand["reset_clean"] and fs_cand["pack_absent_before"] and fs_cand["install_exit"] == 0
    f["first_start_base_pack_files"] = fs_base["pack_files_after"]
    f["first_start_candidate_pack_files"] = fs_cand["pack_files_after"]
    f["first_start_ratio"] = (round(f["first_start_candidate_s"] / f["first_start_base_s"], 2)
                              if f["first_start_base_s"] and f["first_start_candidate_s"] else None)
    s2f = (_load(os.path.join(pr.work, "obs", "s2_update.json")) or {}).get("facts") or {}
    f["start_after_update_s"] = s2f.get("new_daemon_ready_s")
    for name, kinds in sc["g01_variants"]:
        tag = "v_" + name
        rs = _reset_installation(pr)
        f[tag + "_reset_clean"] = rs["clean"]
        pr.ev(tag + "/reset.json", rs)
        r = run([base_inst, "/S"], 900)
        time.sleep(2)
        _kill_product(pr.install_dir)
        f[tag + "_base_ok"] = (r["code"] == 0 and (read_text(pr.exe("cys-installed-version.txt")) or "").strip()
                               == m["base"]["version"])
        ip0 = pr.cys(["init-pack"], timeout=600)
        dpid = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, tag + "-base-cysd.out"))
        pr.register_pid(dpid, tag + " base daemon")
        wait_ready(pr.exe("cys.exe"), 90)
        lp = os.path.join(pr.pack_dir, *rel.split("/"))
        b = read_bytes(lp)
        if b is None:
            f[tag + "_L1_edit_ok"] = False
            continue
        nb = b + (b"" if b.endswith(b"\n") else b"\n") + b"# tw1 user edit L1 (variant)\n"
        write_bytes(lp, nb)
        f[tag + "_L1_user_sha"] = sha256_bytes(nb)
        f[tag + "_L1_edit_ok"] = sha256_file(lp) == f[tag + "_L1_user_sha"]
        f[tag + "_L1_vendor_sha"] = sha256_bytes(vendor) if vendor else None
        holders = {}
        for k in kinds:
            hp = _start_holder(pr, k, pr.pack_dir, tag)
            pr.register_pid(hp, "%s holder %s" % (tag, k))
            holders[k] = hp
        time.sleep(4)
        f[tag + "_holders_started"] = {k: alive(hp) for k, hp in holders.items()}
        f[tag + "_holders_all_started"] = all(f[tag + "_holders_started"].values()) if holders else True
        root_names = sorted(os.listdir(pr.pack_dir), key=R.ntfs_key)
        if "early" in kinds:
            f[tag + "_predicted_if_partial_delete"] = R.entries_before(root_names, sc["early_holder_file"])
        held_rels = [_HELD_REL[k](sc) for k in kinds]
        f[tag + "_held_vendor_changed"] = [r for r in held_rels if vendor_changed(pr.pack_dir, r) is not False]
        f[tag + "_pack_dir_id_pre"] = dir_identity(pr.pack_dir)
        pre = pr.timeline_point(tag + "_pre", {})
        ri = run([cand, "/S"], 900)
        time.sleep(2)
        f[tag + "_cand_ok"] = (ri["code"] == 0 and (read_text(pr.exe("cys-installed-version.txt")) or "").strip() == want)
        after_inst = pr.timeline_point(tag + "_after_installer", {})
        ip = pr.cys(["init-pack", "--no-install-hook"], timeout=600)
        pr.ev(tag + "/init-pack.json", ip)
        f[tag + "_init_exit"] = ip["code"]
        f[tag + "_rename_failed"] = "rename 실패" in ip["err"]
        f[tag + "_swap_refused"] = "교체 안 함" in ip["err"]
        after_drv = pr.timeline_point(tag + "_after_driver_init", {})
        f[tag + "_pack_dir_id_after_driver"] = dir_identity(pr.pack_dir)
        if name == "control":
            man = _manifest_entries(pr.pack_dir)
            _dump(os.path.join(pr.work, "s6-control.json"), {"after_driver": after_drv, "manifest": man, "L1": rel,
                                                             "init_exit": ip["code"]})
        d = R.manifest_delta(pre, after_drv)
        f[tag + "_missing_before_daemon"] = None if d is None else d["missing"]
        d1 = R.manifest_delta(pre, after_inst)
        f[tag + "_missing_after_installer"] = None if d1 is None else d1["missing"]
        f[tag + "_manifest_present_before_install"] = after_drv is not None and ".install-manifest.json" in after_drv
        f[tag + "_L1_sha_before_daemon"] = None if after_drv is None else after_drv.get(rel)
        dr = pr.cys(["drain"], timeout=45)
        _kill_product(pr.install_dir)
        nd = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, tag + "-new-cysd.out"))
        pr.register_pid(nd, tag + " new daemon")
        ok, _secs, _ = wait_ready(pr.exe("cys.exe"), 90)
        f[tag + "_daemon_ready"] = ok
        after_dmn = pr.timeline_point(tag + "_after_daemon", {})
        f[tag + "_L1_live_sha"] = sha256_file(lp)
        f[tag + "_L1_park_sha"] = sha256_file(lp + ".user") if os.path.isfile(lp + ".user") else None
        pend = _load(os.path.join(pr.pack_dir, ".merge-pending.json")) or {}
        ent = pend.get(rel) if isinstance(pend, dict) else None
        f[tag + "_L1_pending_kind"] = ent.get("kind") if isinstance(ent, dict) else None
        ids = (f[tag + "_pack_dir_id_pre"], f[tag + "_pack_dir_id_after_driver"], dir_identity(pr.pack_dir))
        f[tag + "_pack_dir_same"] = None if None in ids else len(set(ids)) == 1
        f[tag + "_pack_prev_after"] = os.path.lexists(os.path.join(os.path.dirname(pr.pack_dir), "pack.prev"))
        f[tag + "_staging_leftovers"] = staging_leftovers(os.path.dirname(pr.pack_dir))
        for k, hp in holders.items():
            kill_tree(hp)
            pr.unregister_pid(hp)
        _kill_product(pr.install_dir)
    pr.restored = False  # the installation is a variant's now: nothing after this stage may assume the S2 state


# ───────────────────────────── S6 — the in-place update with a held file / a held install record (T9a) ─────────────────────────────

def _fresh_base(pr, tag, rel):
    """A fresh v0.14.56 base: reset, /S, its own init-pack, its daemon, the L1 user edit, then the product stopped."""
    m = matrix()
    out = {"reset_clean": _reset_installation(pr)["clean"]}
    r = run([pr.ctx["installers"]["base"], "/S"], 900)
    time.sleep(2)
    _kill_product(pr.install_dir)
    out["base_ok"] = (r["code"] == 0 and (read_text(pr.exe("cys-installed-version.txt")) or "").strip()
                      == m["base"]["version"])
    pr.cys(["init-pack"], timeout=600)
    d = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, tag + "-base-cysd.out"))
    pr.register_pid(d, tag + " base daemon")
    out["base_daemon_ready"] = wait_ready(pr.exe("cys.exe"), 90)[0]
    lp = os.path.join(pr.pack_dir, *rel.split("/"))
    b = read_bytes(lp)
    out["L1_user_sha"] = None
    if b is not None:
        nb = b + (b"" if b.endswith(b"\n") else b"\n") + ("# tw1 user edit L1 (%s)\n" % tag).encode("ascii")
        write_bytes(lp, nb)
        if sha256_file(lp) == sha256_bytes(nb):
            out["L1_user_sha"] = sha256_bytes(nb)
    _kill_product(pr.install_dir)
    pr.unregister_pid(d)
    return out


def _cand_install(pr):
    r = run([pr.ctx["installers"]["candidate"], "/S"], 900)
    time.sleep(2)
    _kill_product(pr.install_dir)
    return r["code"] == 0 and (read_text(pr.exe("cys-installed-version.txt")) or "").strip() == pr.ctx["versions"]["candidate"]


def _hold_file(pr, path, tag):
    """A Python tool holding `path` open with a plain open() (the C runtime shares read and write, NOT delete)."""
    out = os.path.join(pr.ev_dir, tag + "-holder.out")
    code = "import time\nf=open(r'%s','rb')\nprint('held', flush=True)\ntime.sleep(900)\n" % path
    hp = start_detached([pr.bundled_python(), "-B", "-c", code], env=clean_env(), stdout_path=out)
    pr.register_pid(hp, tag + " holder")
    t0 = time.monotonic()
    while time.monotonic() - t0 < 20 and not (read_text(out) or "").startswith("held"):
        time.sleep(0.5)
    return hp, bool(alive(hp) and (read_text(out) or "").startswith("held"))


def _stop_holder(pr, hp):
    if hp:
        kill_tree(hp)
        pr.unregister_pid(hp)
        time.sleep(1)


def probe_s6_inplace(pr):
    """IN-PLACE-HELD (RULINGS GJ3 / GS2) and MANIFEST-HELD (DEFERRED #122 M3), each on a fresh base with the L1 edit:
    the candidate /S, then the app's `cys init-pack --no-install-hook` while one file is held without delete sharing,
    then the holder released and one more run - the pack manifest after every writer."""
    f = pr.facts
    ctx = pr.ctx
    m = matrix()
    want = ctx["versions"]["candidate"]
    rel = (ctx.get("selection") or {}).get("L1")
    f["inplace_L1_rel"] = rel
    f["candidate_version"] = want
    f["base_version"] = m["base"]["version"]
    ctl = _load(os.path.join(pr.work, "s6-control.json")) or {}
    ctl_after = ctl.get("after_driver") or {}
    ctl_man = ctl.get("manifest")
    f["control_available"] = bool(ctl_after) and ctl.get("init_exit") == 0
    if not (ctx["installers"].get("base") and ctx["installers"].get("candidate") and rel):
        return
    vr = vendor_rels()
    vs = vendor_shas(vr)
    f["vendor_rels"] = len(vr)
    # files the uninterrupted control run (G01 variant 'control') installed with exactly the vendor bytes
    verbatim = [r for r in vr if vs.get(r) and ctl_after.get(r) == vs[r]]
    f["verbatim_rels"] = len(verbatim)
    pv = os.path.join(pr.pack_dir, ".pack-version")
    pend = os.path.join(pr.pack_dir, ".merge-pending.json")

    # ── IN-PLACE-HELD: a file the update must REPLACE is held ──
    tag = "iph"
    fb = _fresh_base(pr, tag, rel)
    f["iph_base_ok"] = bool(fb["reset_clean"] and fb["base_ok"] and fb["L1_user_sha"])
    base = pr.timeline_point(tag + "_base", {})
    f["iph_cand_ok"] = _cand_install(pr)
    changed = [r for r in verbatim if r != rel and base and base.get(r) is not None and base.get(r) != vs[r]]
    pool = [r for r in changed if r.startswith("bin/") and r.endswith(".py")]
    held = pool[len(pool) // 2] if pool else None
    f["iph_held_rel"] = held
    f["iph_changed_verbatim"] = len(changed)
    f["iph_held_vendor_changed"] = None if held is None else True
    hp, f["iph_holder_held"] = (_hold_file(pr, os.path.join(pr.pack_dir, *held.split("/")), tag) if held else (None, False))
    before = pr.timeline_point(tag + "_pre_init", {})
    ip = pr.ev_cmd(tag + "/init-pack-1.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
    after1 = pr.timeline_point(tag + "_after_init_1", {})
    f["iph_exit_1"] = ip["code"]
    f["iph_err_tail_1"] = ip["err"][-600:]
    f["iph_err_names_held"] = bool(held) and "cannot write" in ip["err"] and os.path.basename(held) in ip["err"]
    d1 = R.manifest_delta(before, after1)
    f["iph_missing_1"] = None if d1 is None else d1["missing"]
    if held and after1 is not None and before is not None:
        upd = [r for r in changed if after1.get(r) == vs[r]]
        f["iph_updated_count_1"] = len(upd)
        f["iph_updated_before_held_count"] = len([r for r in upd if r < held])
        f["iph_updated_after_held"] = [r for r in upd if r > held]
        f["iph_not_updated_before_held"] = [r for r in changed if r < held and after1.get(r) != vs[r]]
        f["iph_held_unchanged_1"] = after1.get(held) == before.get(held)
        pr.ev(tag + "/updated-1.json", {"held": held, "updated": upd, "changed": changed})
    f["iph_pack_version_1"] = (read_text(pv) or "").strip() or None
    f["iph_pack_version_1_is_base"] = f["iph_pack_version_1"] == m["base"]["version"]
    pr.ev(tag + "/merge-pending-1.json", read_text(pend) or "")
    _stop_holder(pr, hp)
    ip2 = pr.ev_cmd(tag + "/init-pack-2.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
    after2 = pr.timeline_point(tag + "_after_init_2", {})
    f["iph_exit_2"] = ip2["code"]
    d2 = R.manifest_delta(before, after2)
    f["iph_missing_2"] = None if d2 is None else d2["missing"]
    f["iph_pack_version_2"] = (read_text(pv) or "").strip() or None
    f["iph_mismatch_2"] = (None if after2 is None or not verbatim else [r for r in verbatim if after2.get(r) != vs[r]])
    f["iph_L1_user_kept"] = fb["L1_user_sha"] is not None and fb["L1_user_sha"] in (
        (after2 or {}).get(rel), (after2 or {}).get(rel + ".user"))
    f["iph_pack_prev_after"] = os.path.lexists(os.path.join(os.path.dirname(pr.pack_dir), "pack.prev"))

    # ── MANIFEST-HELD: the install record is held during the update (#122 M3) ──
    tag = "mh"
    fb = _fresh_base(pr, tag, rel)
    f["mh_base_ok"] = bool(fb["reset_clean"] and fb["base_ok"] and fb["L1_user_sha"])
    base = pr.timeline_point(tag + "_base", {})
    f["mh_cand_ok"] = _cand_install(pr)
    mp = os.path.join(pr.pack_dir, ".install-manifest.json")
    man0 = sha256_file(mp)
    f["mh_manifest_before_present"] = man0 is not None
    hp, f["mh_holder_held"] = _hold_file(pr, mp, tag) if man0 else (None, False)
    before = pr.timeline_point(tag + "_pre_init", {})
    ip = pr.ev_cmd(tag + "/init-pack-1.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
    after1 = pr.timeline_point(tag + "_after_init_1", {})
    f["mh_exit_1"] = ip["code"]
    f["mh_err_tail_1"] = ip["err"][-600:]
    f["mh_manifest_refused"] = man0 is not None and sha256_file(mp) == man0
    f["mh_pack_version_1"] = (read_text(pv) or "").strip() or None
    d1 = R.manifest_delta(before, after1)
    f["mh_missing_1"] = None if d1 is None else d1["missing"]
    rewritten = [r for r in verbatim if base and base.get(r) is not None and base.get(r) != vs[r]]
    man1 = _manifest_entries(pr.pack_dir)
    f["mh_files_updated_1"] = None if after1 is None else len([r for r in rewritten if after1.get(r) == vs[r]])
    f["mh_manifest_stale_1"] = None if man1 is None else len([r for r in rewritten if man1.get(r) != vs[r]])
    f["mh_L1_user_kept_1"] = fb["L1_user_sha"] is not None and fb["L1_user_sha"] in (
        (after1 or {}).get(rel), (after1 or {}).get(rel + ".user"))
    _stop_holder(pr, hp)
    # a daemon start in between (expected: nothing - the version gate is closed when .pack-version is the candidate's)
    dpid = start_detached([pr.exe("cysd.exe")], env=clean_env(), stdout_path=os.path.join(pr.ev_dir, tag + "-cysd.out"))
    pr.register_pid(dpid, tag + " daemon")
    f["mh_daemon_ready"] = wait_ready(pr.exe("cys.exe"), 90)[0]
    time.sleep(5)
    after_d = pr.timeline_point(tag + "_after_daemon", {})
    _kill_product(pr.install_dir)
    pr.unregister_pid(dpid)
    # the files the update rewrote must keep their vendor bytes; whether the start touched the install record is recorded
    f["mh_daemon_changed_vendor"] = (None if after1 is None or after_d is None
                                     else [r for r in rewritten if after1.get(r) != after_d.get(r)])
    f["mh_daemon_manifest_changed"] = (None if after1 is None or after_d is None
                                       else after1.get(".install-manifest.json") != after_d.get(".install-manifest.json"))
    f["mh_daemon_pack_version"] = (read_text(pv) or "").strip() or None
    ip2 = pr.ev_cmd(tag + "/init-pack-2.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
    after2 = pr.timeline_point(tag + "_after_init_2", {})
    f["mh_exit_2"] = ip2["code"]
    f["mh_pack_version_2"] = (read_text(pv) or "").strip() or None
    d2 = R.manifest_delta(before, after2)
    f["mh_missing_2"] = None if d2 is None else d2["missing"]
    man2 = _manifest_entries(pr.pack_dir)
    pr.ev(tag + "/manifests.json", {"after_refused": man1, "after_rerun": man2})
    f["mh_manifest_mismatch_2"] = (None if man2 is None or not isinstance(ctl_man, dict) or not verbatim
                                   else [r for r in verbatim if man2.get(r) != ctl_man.get(r)])
    f["mh_L1_user_kept_2"] = fb["L1_user_sha"] is not None and fb["L1_user_sha"] in (
        (after2 or {}).get(rel), (after2 or {}).get(rel + ".user"))
    _kill_product(pr.install_dir)
    pr.restored = False


# ───────────────────────────── S3 — the three judges under the bundled runtime (T5c, RULINGS GL4) ─────────────────────────────

def _fwd(p):
    return p.replace("\\", "/")


def _msys(p):
    q = _fwd(p)
    return ("/" + q[0].lower() + q[2:]) if len(q) > 2 and q[1] == ":" else q


def seat_env(pr, extra=None):
    """What a seat's hooks see: the bundled Python, the install folder and the bundled Git first on PATH; no product /
    Python variable from the runner; CYS_PY = the bundled interpreter."""
    inst = pr.install_dir
    git = os.path.join(inst, "runtime", "git")
    sysroot = os.environ.get("SystemRoot", r"C:\Windows")
    path = os.pathsep.join([os.path.join(inst, "runtime", "python"), inst, os.path.join(git, "usr", "bin"),
                            os.path.join(git, "mingw64", "bin"), os.path.join(git, "bin"),
                            os.path.join(sysroot, "System32"), sysroot])
    e = clean_env({"PATH": path, "CYS_NO_AUTOSTART": "1", "CYS_PY": _fwd(pr.bundled_python())})
    e.pop("LC_CTYPE", None)
    if extra:
        e.update(extra)
    return e


def _payload(tool, tool_input, cwd):
    return {"session_id": "tw1-judges", "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": tool_input,
            "cwd": cwd}


def _hook(pr, hook, payload, env, tag, wrap=None, timeout=90):
    pf = os.path.join(pr.work, "judges", tag + ".json")
    write_bytes(pf, json.dumps(payload, ensure_ascii=False).encode("utf-8"))
    cmd = [pr.bundled_hook_bash(), _fwd(os.path.join(pr.pack_dir, "hooks", hook))]
    if wrap:
        cmd = wrap(cmd)
    r = run(cmd, timeout, env=env, stdin_path=pf)
    pr.ev("judges/%s.json" % tag, r)
    return r


def _deny_of(out):
    """(is a deny, reason) from a hook's stdout: its last JSON line."""
    for line in reversed((out or "").strip().splitlines()):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            hso = json.loads(line).get("hookSpecificOutput") or {}
        except ValueError:
            return False, None
        return hso.get("permissionDecision") == "deny", hso.get("permissionDecisionReason")
    return False, None


def _bash(pr, script, env, tag, timeout=60):
    r = run([pr.bundled_hook_bash(), "-c", script], timeout, env=env)
    pr.ev("judges/%s.json" % tag, r)
    return r


def probe_s3_judges(pr):
    f = pr.facts
    pack = pr.pack_dir
    cys = pr.exe("cys.exe")
    lib = _fwd(os.path.join(pack, "hooks", "_lib.sh"))
    gate = os.path.join(pack, "hooks", "role-capability-gate.sh")
    guard = os.path.join(pack, "hooks", "guard.sh")
    f["bundled_bash_found"] = os.path.isfile(pr.bundled_hook_bash())
    f["bundled_python_found"] = os.path.isfile(pr.bundled_python())
    gate_txt = read_text(gate) or ""
    guard_txt = read_text(guard) or ""
    lib_txt = read_text(os.path.join(pack, "hooks", "_lib.sh")) or ""
    f["capgate_exec_flags_in_hook"] = 'exec "$CYS_PY" -I -S -B -X utf8 - <<' in gate_txt
    f["guard_judge_flags_in_hook"] = "-I -S -B -X utf8" in guard_txt
    f["lib_wrapper_flags_in_hook"] = lib_txt.count('"$CYS_PY" -I -S -B -X utf8 -c') >= 2
    tmp = os.environ.get("TEMP") or os.environ.get("TMP") or pr.work
    forbidden = os.path.join(pack, "bin", "tw1_capgate_probe.py")
    allowed = os.path.join(tmp, "tw1-capgate-allowed.txt")
    listed = os.path.join(pack, *matrix()["scenario"]["query_listed_rel"].split("/"))
    # 1. the capability gate's judge - reviewer by the environment fallback: a forbidden write is DENIED
    env_rev = seat_env(pr, {"CYS_ROLE": "reviewer-codex"})
    r = _hook(pr, "role-capability-gate.sh", _payload("Write", {"file_path": forbidden, "content": "x"}, pack),
              env_rev, "capgate-reviewer-forbidden")
    f["cap_rev_forbidden_exit"] = r["code"]
    f["cap_rev_forbidden_deny"], reason = _deny_of(r["out"])
    f["cap_rev_forbidden_reason_decoded"] = bool(reason) and "산출물" in reason
    m_src = re.search(r"src=([^\]\s]+)\]", r["err"])
    f["cap_rev_forbidden_src"] = m_src.group(1) if m_src else None
    f["cap_rev_forbidden_file_absent"] = not os.path.exists(forbidden)
    # ... and an allowed write passes (the judge decided, not crashed)
    r = _hook(pr, "role-capability-gate.sh", _payload("Write", {"file_path": allowed, "content": "x"}, pack),
              env_rev, "capgate-reviewer-allowed")
    f["cap_rev_allowed_exit"] = r["code"]
    f["cap_rev_allowed_deny"] = _deny_of(r["out"])[0]
    f["cap_rev_allowed_traceback"] = "Traceback" in r["err"]
    # 2. a worker writing a LISTED file: the judge asks `cys repair boot-critical` from inside the isolated interpreter
    r = _hook(pr, "role-capability-gate.sh", _payload("Write", {"file_path": listed, "content": "x"}, pack),
              seat_env(pr, {"CYS_ROLE": "worker"}), "capgate-worker-listed")
    f["cap_worker_listed_exit"] = r["code"]
    f["cap_worker_listed_deny"], reason = _deny_of(r["out"])
    f["cap_worker_listed_reason"] = (reason or "")[:300]
    # 3. the role from the DAEMON through _lib.sh (surface-role via cys_timeout_run)
    ns = pr.ev_cmd("judges/new-surface-reviewer.json", pr.cys(["new-surface", "--role", "reviewer-codex"], timeout=60))
    sid = ns["out"].strip().splitlines()[-1].strip() if ns["code"] == 0 and ns["out"].strip() else None
    f["reviewer_surface"] = sid
    tk = _bash(pr, 'command -v timeout; timeout --version 2>&1 | head -n 1; echo "rc=$?"', seat_env(pr), "timeout-kind")
    f["lib_timeout_path"] = (tk["out"].splitlines() or [""])[0].strip() or None
    f["lib_timeout_is_gnu"] = "GNU coreutils" in tk["out"] or "coreutils" in tk["out"]
    if sid:
        env_sid = seat_env(pr, {"CYS_SURFACE_ID": sid})
        r = _hook(pr, "role-capability-gate.sh", _payload("Write", {"file_path": forbidden, "content": "x"}, pack),
                  env_sid, "capgate-reviewer-daemon")
        f["cap_daemon_exit"] = r["code"]
        f["cap_daemon_deny"] = _deny_of(r["out"])[0]
        m_src = re.search(r"src=([^\]\s]+)\]", r["err"])
        f["cap_daemon_src"] = m_src.group(1) if m_src else None
        q = '. "%s"; cys_resolve_role --no-cache; printf "%%s|%%s\\n" "$CYS_RESOLVED_ROLE" "$CYS_RESOLVED_ROLE_SOURCE"' % lib
        r = _bash(pr, q, env_sid, "lib-role-native")
        f["lib_role_native"] = (r["out"].strip().splitlines() or [None])[-1]
        # the wrapper's own interpreter (branch 3): GNU timeout / gtimeout made to answer "not GNU"
        q = ('. "%s"; timeout() { return 1; }; gtimeout() { return 1; }; set -x; cys_resolve_role --no-cache; set +x; '
             'printf "%%s|%%s\\n" "$CYS_RESOLVED_ROLE" "$CYS_RESOLVED_ROLE_SOURCE"') % lib
        r = _bash(pr, q, env_sid, "lib-role-wrapper")
        f["lib_role_wrapper"] = (r["out"].strip().splitlines() or [None])[-1]
        f["lib_role_wrapper_traced_flags"] = "-I -S -B -X utf8 -c" in r["err"]
    # 4. guard.sh's judge: an outgoing POST is refused by the Python judge (not by the shell backstop)
    r = _hook(pr, "guard.sh", _payload("Bash", {"command": "curl -X POST https://example.invalid/tw1 -d x"}, pack),
              seat_env(pr), "guard-curl-post")
    f["guard_post_exit"] = r["code"]
    f["guard_post_named"] = "curl" in r["err"] and "POST" in r["err"]
    f["guard_post_backstop"] = "백스톱" in r["err"] or "백스톱" in r["out"]
    f["guard_post_traceback"] = "Traceback" in r["err"]
    r = _hook(pr, "guard.sh", _payload("Bash", {"command": "ls"}, pack), seat_env(pr), "guard-ls")
    f["guard_ls_exit"] = r["code"]
    f["round_autopilot_active"] = os.path.exists(os.path.join(pack, "AUTOPILOT_ACTIVE"))
    # 5. -X utf8 on a cp949 console: the reviewer deny (Korean reason) through `chcp 949`
    if IS_WIN:
        f["acp"] = int(_k32.GetACP())
        f["oemcp"] = int(_k32.GetOEMCP())
    wrap = lambda cmd: ["cmd", "/d", "/c", "chcp 949 >nul & chcp & " + " ".join(cmd)]
    r = _hook(pr, "role-capability-gate.sh", _payload("Write", {"file_path": forbidden, "content": "x"}, pack),
              env_rev, "capgate-cp949", wrap=wrap)
    first = (r["out"].strip().splitlines() or [""])[0]
    f["cp949_chcp_line"] = first[:80]
    f["cp949_active"] = "949" in first
    f["cp949_exit"] = r["code"]
    f["cp949_deny"], reason = _deny_of(r["out"])
    f["cp949_reason_decoded"] = bool(reason) and "산출물" in reason
    enc = os.path.join(pr.work, "judges", "enc.py")
    write_bytes(enc, b"import sys\nprint(sys.stdout.encoding, sys.flags.utf8_mode)\n")
    for k, flags in (("utf8flag", "-I -S -B -X utf8"), ("plain", "-I -S -B")):
        r = run(["cmd", "/d", "/c", "chcp 949 >nul & %s %s %s" % (pr.bundled_python(), flags, enc)], 60,
                env=seat_env(pr))
        pr.ev("judges/cp949-encoding-%s.json" % k, r)
        f["cp949_py_encoding_" + k] = (r["out"].strip().split() or [None])[0]
    # 6. the embeddable runtime's ._pth `import site` against -S (T5c 2.4): a copy of the runtime with a sitecustomize
    pc = os.path.join(pr.work, "judges", "pthcopy")
    shutil.rmtree(pc, ignore_errors=True)
    try:
        shutil.copytree(os.path.join(pr.install_dir, "runtime", "python"), pc)
        f["pth_copy_ok"] = True
    except (OSError, shutil.Error) as e:
        f["pth_copy_ok"] = False
        pr.ev("judges/pth-copy-error.txt", str(e))
    if f["pth_copy_ok"]:
        pth = sorted(glob.glob(os.path.join(pc, "*._pth")))
        f["pth_file"] = os.path.basename(pth[0]) if pth else None
        pr.ev("judges/pth.txt", read_text(pth[0]) if pth else "")
        marker = os.path.join(pc, "tw1-sitecustomize-ran.txt")
        write_bytes(os.path.join(pc, "sitecustomize.py"),
                    ("open(r'%s','a').write('ran\\n')\n" % marker).encode("utf-8"))
        show = "import sys; print(sys.flags.no_site, 'site' in sys.modules, sys.flags.utf8_mode)"
        for k, flags in (("S", ["-I", "-S", "-B", "-X", "utf8"]), ("plain", ["-B"])):
            if os.path.exists(marker):
                os.remove(marker)
            r = run([os.path.join(pc, "python.exe")] + flags + ["-c", show], 60, env=seat_env(pr), cwd=pr.work)
            pr.ev("judges/pth-%s.json" % k, r)
            f["pth_%s_flags" % k] = r["out"].strip()[:120]
            f["pth_%s_sitecustomize_ran" % k] = os.path.exists(marker)
        shutil.rmtree(pc, ignore_errors=True)
    # 7. compgen in Git Bash: _cys_lc_handback for the four LC_CTYPE states, and what the wrapper's child sees
    f["compgen_type"] = _bash(pr, "type compgen", seat_env(pr), "compgen-type")["out"].strip()[:80]
    child = ("import os; print(repr(os.environ.get('LC_CTYPE')))")
    cases = (("exported", {"LC_CTYPE": "C.UTF-8"}, ""), ("unset", {}, ""), ("shellvar", {}, "LC_CTYPE=xx_TW1; "),
             ("empty", {"LC_CTYPE": ""}, ""))
    for name, extra, pre in cases:
        q = ('. "%s"; %s_cys_lc_handback; printf "[%%s]\\n" "$_cys_lc"; timeout() { return 1; }; '
             'gtimeout() { return 1; }; cys_timeout_run 20 "$CYS_PY" -I -S -c "%s"') % (lib, pre, child)
        r = _bash(pr, q, seat_env(pr, extra), "compgen-" + name)
        lines = r["out"].strip().splitlines()
        f["compgen_handback_" + name] = lines[0] if lines else None
        f["compgen_child_" + name] = lines[-1] if len(lines) > 1 else None
    # 8. a dangling junction inside the pack (removed after)
    dj, djt = os.path.join(pack, "tw1-dj"), os.path.join(pack, "tw1-dj-target")
    os.makedirs(djt, exist_ok=True)
    mk = run(["cmd", "/c", "mklink", "/J", dj, djt], 30)
    shutil.rmtree(djt, ignore_errors=True)
    f["dj_dangling"] = mk["code"] == 0 and os.path.lexists(dj) and not os.path.exists(dj)
    if f["dj_dangling"]:
        t0 = time.monotonic()
        code, _l, rr = _boot_critical_exit(cys, os.path.join(dj, "session-start.sh"))
        f["dj_q_native"], f["dj_q_native_secs"] = code, round(time.monotonic() - t0, 2)
        f["dj_q_native_timed_out"] = rr["timed_out"]
        code, _l, rr = _boot_critical_exit(cys, _msys(os.path.join(dj, "session-start.sh")))
        f["dj_q_msys"] = code
        r = _hook(pr, "role-capability-gate.sh",
                  _payload("Write", {"file_path": os.path.join(dj, "x.sh"), "content": "x"}, pack),
                  seat_env(pr, {"CYS_ROLE": "worker"}), "capgate-dangling-junction")
        f["dj_gate_exit"] = r["code"]
        f["dj_gate_traceback"] = "Traceback" in r["err"]
        f["dj_gate_deny"] = _deny_of(r["out"])[0]
    f["dj_listed_still_0"] = _boot_critical_exit(cys, listed)[0] == 0
    f["dj_daemon_answers"] = wait_ready(cys, 15)[0]
    if os.path.lexists(dj):
        run(["cmd", "/c", "rmdir", dj], 30)
    f["dj_removed"] = not os.path.lexists(dj)
    pr.restored = f["dj_removed"]


# ───────────────────────────── S3 — T3d EG2: a folder's identity, driven from worker seats ─────────────────────────────

_EG2_SCRIPT = r'''import json, subprocess, sys
cys, out = sys.argv[1], sys.argv[2]
r = subprocess.run([cys, "repair", "begin"] + sys.argv[3:], capture_output=True, timeout=60)
with open(out + ".tmp", "w", encoding="utf-8") as fh:
    json.dump({"rc": r.returncode, "out": r.stdout.decode("utf-8", "replace"), "err": r.stderr.decode("utf-8", "replace")}, fh)
import os
os.replace(out + ".tmp", out)
'''


def _seat_begin(pr, sid, kind, args, tag):
    """Type one command into the worker seat's pane: the bundled Python runs `cys repair begin <args>` as the pane's
    descendant (the daemon finds the caller's seat by the process ancestry) and writes {rc, out, err} to a file."""
    d = os.path.join(pr.work, "eg2")
    script = os.path.join(d, "begin.py")
    write_bytes(script, _EG2_SCRIPT.encode("utf-8"))
    outp = os.path.join(d, tag + ".json")
    # bash: MSYS spellings; cmd / PowerShell: native spellings unquoted (both run a bare path without spaces)
    conv = _msys if kind == "bash" else (lambda x: x)
    paths = [conv(pr.bundled_python()), conv(script), conv(pr.exe("cys.exe")), conv(outp)]
    line = " ".join([paths[0], "-I", "-S", "-B"] + paths[1:] + list(args))
    s1 = pr.cys(["send", "--surface", sid, line], timeout=30)
    s2 = pr.cys(["send-key", "--surface", sid, "Return"], timeout=30)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 60 and not os.path.isfile(outp):
        time.sleep(1)
    res = _load(outp)
    pr.ev("eg2/%s.json" % tag, {"line": line, "send": s1, "send_key": s2, "result": res})
    return res


def probe_s3_eg2(pr):
    f = pr.facts
    pack = pr.pack_dir
    notes = os.path.join(pack, "notes")
    f["notes_preexisting"] = os.path.isdir(notes)
    os.makedirs(notes, exist_ok=True)
    f["notes_targets_absent"] = not os.path.exists(os.path.join(notes, "a.txt")) and not os.path.exists(os.path.join(notes, "A.txt"))
    seats = []
    for i in (1, 2):
        ns = pr.ev_cmd("eg2/new-surface-%d.json" % i, pr.cys(["new-surface", "--role", "worker"], timeout=60))
        seats.append(ns["out"].strip().splitlines()[-1].strip() if ns["code"] == 0 and ns["out"].strip() else None)
    f["worker_surfaces"] = seats
    time.sleep(5)
    lst = pr.ev_cmd("eg2/list.json", pr.cys(["list"], timeout=30, extra={"CYS_NO_AUTOSTART": "1"}))
    kinds = {}
    for sid in seats:
        if not sid:
            continue
        mm = re.search(r"^%s\t.*?\bpid=(\d+)\texited=(\w+)" % re.escape(sid), lst["out"], re.M)
        img = image_path(int(mm.group(1))) if mm else None
        kinds[sid] = {"pid_image": os.path.basename(img or "").lower() or None, "exited": mm.group(2) if mm else None}
    f["worker_panes"] = kinds
    f["eg2_seat_driven"] = False
    if not all(seats):
        return
    kind = lambda sid: "bash" if (kinds.get(sid) or {}).get("pid_image") in ("bash.exe", "sh.exe") else "native"
    pair = _seat_begin(pr, seats[0], kind(seats[0]),
                       ["--files", "notes/a.txt", "notes/A.txt", "--trigger", "owner-order"], "pair")
    one = _seat_begin(pr, seats[0], kind(seats[0]), ["--files", "notes/a.txt", "--trigger", "owner-order"], "first")
    two = _seat_begin(pr, seats[1], kind(seats[1]), ["--files", "notes/A.txt", "--trigger", "owner-order"], "second")
    texts = [((x or {}).get("out", "") + (x or {}).get("err", "")) for x in (pair, one, two)]
    f["eg2_seat_driven"] = all(x is not None for x in (pair, one, two)) and not any("좌석이 없습니다" in t for t in texts)
    f["eg2_pair_rc"] = (pair or {}).get("rc")
    f["eg2_pair_casefold"] = "볼륨에 따라 같은 파일일 수 있습니다" in texts[0]
    f["eg2_pair_unknown"] = "확인할 수 없습니다" in texts[0]
    f["eg2_first_rc"] = (one or {}).get("rc")
    f["eg2_second_rc"] = (two or {}).get("rc")
    f["eg2_second_casefold"] = "대소문자만 다른 이름은 볼륨에 따라 같은 파일이라" in texts[2]
    f["eg2_second_unknown"] = "확인할 수 없습니다" in texts[2]
    f["eg2_targets_still_absent"] = not os.path.exists(os.path.join(notes, "a.txt"))
    pr.restored = True


PROBES = {
    "s0_env": probe_s0_env,
    "s1_base": probe_s1_base,
    "s2_update": probe_s2_update,
    "s3_cli": probe_s3_cli,
    "s3_files": probe_s3_files,
    "s3_jobs_hooks": probe_s3_jobs_hooks,
    "s3_query": probe_s3_query,
    "s3_home": probe_s3_home,
    "s3_diff": probe_s3_diff,
    "s3_dy1": probe_s3_dy1,
    "s3_intake": probe_s3_intake,
    "s3_listen": probe_s3_listen,
    "s3_siblings": probe_s3_siblings,
    "s3_judges": probe_s3_judges,
    "s3_eg2": probe_s3_eg2,
    "s7_primitives": probe_s7_primitives,
    "s6_variants": probe_s6_variants,
    "s6_inplace": probe_s6_inplace,
}
