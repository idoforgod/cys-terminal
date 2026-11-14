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


def wait_ready(cys_exe, limit_s):
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit_s:
        r = run([cys_exe, "list"], 15, env=clean_env({"CYS_NO_AUTOSTART": "1"}))
        if r["code"] == 0:
            return True, round(time.monotonic() - t0, 2), r
        time.sleep(1)
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
            if changed == want_changed and ok_listed:
                sel[key] = rel
                taken.add(rel)
                return
            why.setdefault(key, []).append("%s: changed=%s listed_exit=%s" % (rel, changed, lc))

    taken = set()
    for key in ("L1", "L2", "CL"):
        pick(key, sc["listed_changed"], True, True, taken)
    pick("L3", sc["listed_unchanged"], True, False, taken)
    for key in ("G1", "ENC1", "CG"):
        pick(key, sc["general_changed_py"], False, True, taken)
    pick("CRLF1", sc["general_changed_sh"], False, True, taken)
    pick("CRLF2", sc["general_changed_md"], False, True, taken)
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

    edit("L1", append_line("L1"))
    edit("L2", append_line("L2"))
    edit("L3", append_line("L3"))
    edit("G1", append_line("G1"))
    edit("ENC1", to_cp949)
    edit("CRLF1", to_crlf)
    edit("CRLF2", to_crlf)
    for key in ("CL", "CG"):
        rel = sel.get(key)
        if rel:
            b = read_bytes(os.path.join(pack, *rel.split("/"))) or b""
            edits[key] = {"rel": rel, "base_sha": sha256_bytes(b), "user_sha": sha256_bytes(b),
                          "vendor_sha": sha256_bytes(vendor(rel) or b""), "user_has_cr": b"\r" in b}
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
    for a in pids_under(pr.install_dir, "cys-app.exe"):
        terminate(a)
        pr.unregister_pid(a)
    if applied_by is None:
        ip = pr.ev_cmd("driver-init-pack.json", pr.cys(["init-pack", "--no-install-hook"], timeout=600))
        f["driver_init_pack_exit"] = ip["code"]
        f["driver_init_pack_secs"] = ip["secs"]
        applied_by = "driver" if ip["code"] == 0 else "driver-failed"
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
        ent = pending.get(rel) if isinstance(pending, dict) else None
        f[key + "_pending_kind"] = ent.get("kind") if isinstance(ent, dict) else None
        code, _, _ = _boot_critical_exit(pr.exe("cys.exe"), p)
        f[key + "_listed"] = code
    s1 = os.path.join(pr.work, "obs", "s1_base.json")
    try:
        f["ENC1_user_invalid_utf8"] = json.loads(read_text(s1))["facts"].get("ENC1_user_invalid_utf8")
    except (TypeError, ValueError, KeyError):
        f["ENC1_user_invalid_utf8"] = None
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
    rm1 = run(["cmd", "/c", "rmdir", ja], 30) if os.path.lexists(ja) else None
    rm2 = run(["cmd", "/c", "rmdir", jb], 30) if os.path.lexists(jb) else None
    f["junction_removed"] = not os.path.lexists(ja) and not os.path.lexists(jb) and os.path.isfile(lp) and os.path.isfile(gp)
    out["mklink"] = [mk1, mk2]
    out["rmdir"] = [rm1, rm2]
    pr.ev("queries.json", out)
    pr.restored = f["junction_removed"]


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
}
