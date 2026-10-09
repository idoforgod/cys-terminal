#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""phoenix 기본 인코딩 내성 시험(0.14.48 · 리포 커밋) — UTF-8 이 아닌 기본 인코딩에서도 죽지 않고 멀쩡한 파일을 치우지 않는다.

왜: 데몬은 자동 복원용 phoenix 를 PYTHONUTF8 없이 띄우고 stdout 을 파일로 돌린다. 그러면 파이썬은 시스템 기본
코드페이지(한국어 윈도우 cp949 · 영문 윈도우 cp1252)로 파일을 읽고 stdout 에 쓴다. 0.14.47 까지는 그 조건에서
①UTF-8 로 저장된 저널·로스터를 못 읽어 「손상」으로 오판하고 `.corrupt-<시각>` 으로 치웠고 ②줄표(—)·★·한글이
든 로그·결과 JSON 을 쓰다 UnicodeEncodeError 로 종료코드 1 이 됐다(실측: 윈도우 11 러너 cp1252 · 맥 cp949 모의).
CI 는 잡 수준에서 PYTHONUTF8=1 을 두르고 돌아 이 조건을 본 적이 없다 — 이 시험은 자식 프로세스에서 그 변수를 벗긴다.

조건(전부 자식 프로세스 · PYTHONUTF8·PYTHONIOENCODING 제거 · stdout 은 파일로):
  · locale:cp949 — LC_ALL=ko_KR.CP949 (운영체제 로케일이 있을 때만 · 파이썬이 스스로 cp949 를 기본으로 잡는다)
  · shim:cp949 · shim:cp1252 — 인코딩 없는 텍스트 open 의 기본값과 stdout 을 그 코덱으로 갈아 끼운다(어느 OS 에서나 돈다)
  · native — 벗기기만 한다(윈도우 러너에서는 이것이 진짜 ANSI 코드페이지 조건이다 · 맥/리눅스는 UTF-8 이라 대조군 구실)
  · shim:utf-8 — 대조군(모의 장치 자체의 오류를 가른다)
추가 시나리오: 상태 응답 수신(가짜 cys 가 한글 든 UTF-8 JSON) · 외부 도구 수신(cp949 바이트를 내는 가짜 schtasks) · 스냅샷 manifest · 묘비+치워진 파일 알림.
선택(E): PHOENIX_HARNESS_CYSD·PHOENIX_CYS 가 둘 다 있고 locale:cp949 가 성립하면, 격리 데몬에 `restore --auto` 를
  데몬이 주는 것과 같은 환경 키로 한 번 불러 종료코드 0 · `.corrupt-*` 0 · Traceback 0 을 본다. 없으면 건너뛰고 그 사실을 찍는다.

실행: python3 cysjavis-pack/bin/tests/test_phoenix_encoding_default.py  (0=전건 PASS)
"""
import json, os, shutil, signal, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
# PHOENIX_UNDER_TEST — 시험 대상 파일을 바꾼다(수정 전 판의 사본에 같은 시험을 돌려 빨강을 먼저 찍는 용도 · 형제 파일은 그 옆에서 찾는다).
PH = os.path.abspath(os.environ.get("PHOENIX_UNDER_TEST") or os.path.normpath(os.path.join(HERE, "..", "javis_phoenix.py")))

_results = []
def check(name, cond, detail=""):
    _results.append(bool(cond))
    print(("PASS " if cond else "FAIL ") + name + (" | " + detail if detail else ""))
    sys.stdout.flush()


# 자식 스크립트 — 조건을 세운 뒤 배포 파일을 불러 한 가지 시나리오를 돌리고, 결과를 **파일**에 ASCII JSON 으로 남긴다
# (stdout 은 시험 대상이라 결과 통로로 쓰지 않는다).
CHILD = r'''
import builtins, importlib.util, io, json, os, runpy, sys, traceback
ph, shim, scenario, work, resfile = sys.argv[1:6]
res = {"scenario": scenario, "exc": None}
if shim != "-":
    _oo = builtins.open
    def _open(file, mode="r", buffering=-1, encoding=None, errors=None, newline=None, closefd=True, opener=None):
        if "b" not in mode and encoding is None:
            encoding = shim
        return _oo(file, mode, buffering, encoding, errors, newline, closefd, opener)
    builtins.open = _open
    io.open = _open
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding=shim, errors="strict")
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding=shim, errors="backslashreplace")
_probe = builtins.open(os.path.join(work, "probe.tmp"), "w"); res["open_default"] = _probe.encoding; _probe.close()
res["stdout_enc"] = sys.stdout.encoding; res["stdout_errors"] = sys.stdout.errors; res["utf8_mode"] = sys.flags.utf8_mode
def done(code=0):
    with io.open(resfile, "w", encoding="ascii") as f:
        json.dump(res, f, ensure_ascii=True)
    sys.stdout.flush()
    os._exit(code)
if scenario == "probe":
    done()
if scenario == "entry_plan":
    sys.argv = [ph, "deploy", "--plan"]
    try:
        runpy.run_path(ph, run_name="__main__")
    except SystemExit as e:
        res["exit"] = e.code
        done(e.code or 0)
    except Exception as e:
        res["exc"] = type(e).__name__ + ": " + str(e)[:160]
        done(1)
    done()
spec = importlib.util.spec_from_file_location("javis_phoenix", ph); m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
m._emit_evt = lambda *a, **k: False            # 시험 자리 밖으로 이벤트를 내지 않는다
m.CYS = os.path.join(work, "nonexistent-cys")
sd = os.path.join(work, "state"); home = os.path.join(sd, "phoenix"); os.makedirs(home, exist_ok=True)
sock = os.path.join(sd, "cys.sock")
if os.name == "nt":                            # 윈도우는 파이프 이름에서 상태 폴더를 찾는다 — 시험은 경로를 직접 물린다
    m.state_dir_for = lambda socket: sd
KO = "한글 — 줄표 ★"
def seed(path, obj):
    with builtins.open(path, "wb") as f:       # 제품의 쓰기(_atomic_write_json)와 같은 직렬화 — UTF-8 · ensure_ascii=False
        f.write(json.dumps(obj, ensure_ascii=False, indent=1).encode("utf-8"))
try:
    if scenario == "files":
        seed(os.path.join(sd, "topology.json"), {"entries": [{"role": "worker", "agent": "claude", "cwd": "/Users/홍길동/작업 — 폴더"}], "updated_at": 1})
        roster = {"roster": {"worker": {"role": "worker", "agent": "claude", "cwd": "/Users/홍길동/작업 — 폴더"}}, "tombstones": ["reviewer-x"], "updated_at": 1}
        dp = m.desired_roster_path(sock); seed(dp, roster); seed(dp + ".bak", roster)
        seed(m.dept_roster_path(sock), {"roster": {"부서 — 1": {"name": "부서 — 1"}}, "tombstones": []})
        seed(m.journal_path(sock, "default"), {"ticket_id": "default", "roles": {}, "events": [{"ts": 1, "role": "*", "stage": "s0", "status": "ok", "msg": KO}], "created": 1})
        seed(m.deploy_journal_path(sock, "default"), {"ticket": "default", "stages": {}, "events": [{"msg": KO}], "created": 1})
        res["status_desired"] = m._roster_file_status(dp)
        res["status_bak"] = m._roster_file_status(dp + ".bak")
        res["recover"] = m._recover_retention_file(sock, dp, "desired_roster").get("status")
        r, t = m.load_desired_roster(sock); res["desired_n"] = len(r); res["tombstones"] = sorted(t)
        r, t = m.load_dept_roster(sock); res["dept_n"] = len(r)
        topo = m.read_topology(sock); res["topo_n"] = len(topo.get("entries", [])); res["topo_error"] = topo.get("_error")
        res["journal_events"] = len(m.load_journal(sock, "default").get("events", []))
        res["deploy_events"] = len(m.load_deploy_journal(sock, "default").get("events", []))
    elif scenario == "stdout":
        force = getattr(m, "_force_utf8_stdio", None)
        res["has_force"] = force is not None
        if force: force()
        m.log(KO)
        print(json.dumps({"note": "다른 restore가 진행 중 — skip ★"}, ensure_ascii=False, indent=2))
        sys.stdout.flush()
    elif scenario == "gen_manual":
        force = getattr(m, "_force_utf8_stdio", None)
        if force: force()
        seed(os.path.join(sd, "topology.json"), {"entries": [], "updated_at": 1})
        class A: pass
        a = A(); a.socket = sock; a.dest = os.path.join(work, "manual")
        m.cmd_gen_manual(a)
        raw = builtins.open(os.path.join(work, "manual", "manual_restore.sh"), "rb").read()
        res["manual_utf8_ok"] = raw.decode("utf-8") == m.MANUAL_RESTORE_TEMPLATE
        # 만든 스크립트를 실제로 돌린다 — 그 안의 python3 는 별도 프로세스라 부모의 재설정을 물려받지 않는다.
        #   터미널이 한글을 못 쓰는 코드페이지(cp1252 · strict)여도 복원 명령 줄이 끝까지 나와야 한다.
        import shutil, subprocess
        if os.name != "nt" and shutil.which("bash") and shutil.which("python3"):
            seed(os.path.join(work, "manual", "topology.json"), {"entries": [{"role": "worker", "agent": "claude", "session_id": "s1"}]})
            env = dict(os.environ, PYTHONIOENCODING="cp1252:strict")
            pr = subprocess.run(["bash", os.path.join(work, "manual", "manual_restore.sh")], capture_output=True, env=env, timeout=60)
            res["manual_run"] = [b"cys launch-agent --role worker --agent claude" in pr.stdout, b"UnicodeEncodeError" not in pr.stderr]
    elif scenario == "status_recv":
        # 가짜 cys — 좌석 제목·작업 폴더에 한글·줄표가 든 UTF-8 JSON 을 낸다(Rust CLI 의 출력 계약과 같은 바이트).
        payload = os.path.join(work, "status.json")
        with builtins.open(payload, "wb") as f:
            f.write(json.dumps({"daemon": {"build_id": "abc123", "proto": 1, "version": "t"},
                                "surfaces": [{"title": "한글 — 좌석 제목", "cwd": "/Users/홍길동/작업 — 폴더", "task": KO}]},
                               ensure_ascii=False, indent=2).encode("utf-8"))
        if os.name == "nt":
            fake = os.path.join(work, "fakecys.cmd")
            with builtins.open(fake, "wb") as f: f.write(('@type "%s"\r\n' % payload).encode("ascii"))
        else:
            fake = os.path.join(work, "fakecys")
            with builtins.open(fake, "wb") as f: f.write(('#!/bin/sh\nexec cat "%s"\n' % payload).encode("utf-8"))
            os.chmod(fake, 0o755)
        d = m._daemon_identity(fake, None)
        res["daemon_build"] = (d or {}).get("build_id")
        m.CYS = fake
        r = m.cys("status", "--json", socket=None, timeout=20)
        res["cys_rc"] = getattr(r, "returncode", None)
        res["cys_title_ok"] = "한글 — 좌석 제목" in (getattr(r, "stdout", "") or "")
    elif scenario == "native_recv":
        # 가짜 schtasks — 한국어 윈도우 도구처럼 cp949 바이트를 낸다(유닉스에서만 · 윈도우는 진짜 schtasks 가 돈다).
        if os.name != "nt":
            bindir = os.path.join(work, "bin"); os.makedirs(bindir)
            fake = os.path.join(bindir, "schtasks")
            with builtins.open(fake, "wb") as f: f.write(b"#!/bin/sh\nprintf '\\274\\272\\260\\370: \\301\\266\\310\\270\\n'\nexit 0\n")
            os.chmod(fake, 0o755)
            os.environ["PATH"] = bindir + os.pathsep + os.environ.get("PATH", "")
            r = m._schtasks("/Query")
            res["schtasks_rc"] = r.returncode
        else:
            r = m._schtasks("/Query", "/TN", "cys-encoding-test-nonexistent")
            res["schtasks_rc"] = 0 if r.returncode != 127 else 127   # 없는 작업이라 실패가 정상 — 127(수신 예외)만 아니면 된다
        dec = getattr(m, "_dec_any", None)
        res["dec_any"] = None if dec is None else [dec(b"\xbc\xba\xb0\xf8") != "", dec("한글 —".encode("utf-8")) == "한글 —", dec(b"") == ""]
    elif scenario == "manifest":
        import contextlib
        sys.path.insert(0, os.path.dirname(ph))
        import javis_state_snapshot as snap
        srcdir = os.path.join(work, "원본 — 폴더"); os.makedirs(srcdir)
        src = os.path.join(srcdir, "topology.json"); seed(src, {"entries": [], "updated_at": 1})
        gen_root = os.path.join(work, "gens")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            name = snap.do_snapshot(sources=[src], gen_root=gen_root)
        mpath = os.path.join(gen_root, name, "manifest.json")
        raw = builtins.open(mpath, "rb").read()
        try:
            res["manifest_utf8"] = "원본 — 폴더" in raw.decode("utf-8")
        except UnicodeDecodeError:
            res["manifest_utf8"] = False
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            res["verify_new"] = snap.do_verify(gen_root=gen_root)
        # 과거 cp949 프로세스가 쓴 manifest(줄표 없는 한글 경로) — 버리지 않고 읽혀야 한다
        obj = json.loads(raw.decode("utf-8", "replace")) if res["manifest_utf8"] else None
        if obj is not None:
            for e in obj.get("files", []): e["source"] = "/Users/홍길동/작업/topology.json"
            with builtins.open(mpath, "wb") as f: f.write(json.dumps(obj, indent=2, ensure_ascii=False).encode("cp949"))
            import locale
            res["locale_pref"] = locale.getpreferredencoding(False)
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                res["verify_old_cp949"] = snap.do_verify(gen_root=gen_root)
            # 「징」의 cp949 바이트(c2 a1)는 UTF-8 로도 읽힌다(¡) — 디코딩 성공만 믿으면 저장 파일 이름이 조용히 바뀐다.
            try:
                amb = os.path.join(srcdir, "징.md")
                with builtins.open(amb, "wb") as f: f.write(b"x")
            except (OSError, UnicodeError):
                amb = None                 # 이 파일 시스템이 그 이름을 못 쓴다 — 이 칸은 재지 않는다
            if amb:
                gen2 = os.path.join(work, "gens2")
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    name2 = snap.do_snapshot(sources=[amb], gen_root=gen2)
                mp2 = os.path.join(gen2, name2, "manifest.json")
                o2 = json.loads(builtins.open(mp2, "rb").read().decode("utf-8"))
                for e in o2.get("files", []): e["source"] = "/Users/x/징.md"
                with builtins.open(mp2, "wb") as f: f.write(json.dumps(o2, indent=2, ensure_ascii=False).encode("cp949"))
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    res["verify_ambiguous_cp949"] = snap.do_verify(gen_root=gen2)
    elif scenario == "emit_child":
        # 진짜 _emit_evt — 형제 javis_event.py 자리에 「어느 인코딩으로도 풀리지 않는 바이트」를 내는 가짜를 둔다(버스에 닿지 않는다).
        spec2 = importlib.util.spec_from_file_location("javis_phoenix_evt", ph); m2 = importlib.util.module_from_spec(spec2); spec2.loader.exec_module(m2)
        fdir = os.path.join(work, "evt"); os.makedirs(fdir)
        with builtins.open(os.path.join(fdir, "javis_event.py"), "wb") as f:
            f.write(b"import sys\nsys.stdout.buffer.write(b'\\xff\\x81\\xff\\x81 \\xed\\x95\\x9c\\n'); sys.stdout.buffer.flush()\nsys.exit(0)\n")
        m2.__file__ = os.path.join(fdir, "javis_phoenix.py")
        res["emit_ok"] = m2._emit_evt("test.event", note=KO)
    elif scenario == "log_flush":
        class S:
            encoding = "utf-8"
            def __init__(self): self.w = []
            def write(self, t): self.w.append(t)
            def flush(self): raise OSError("disk full")
        class U(S):
            def flush(self): raise UnicodeEncodeError("x", "y", 0, 1, "flush")
        out = {}
        for nm, cls in (("oserror", S), ("unicode", U)):
            st = cls(); keep = sys.stdout; sys.stdout = st
            try:
                m.log(KO); out[nm] = len(st.w)
            except Exception as e:
                out[nm] = type(e).__name__
            finally:
                sys.stdout = keep
        res["log_flush"] = out
    elif scenario == "notice_cap":
        notice = getattr(m, "_notice_requalified_corrupt", None)
        for i in range(35):
            seed(m.journal_path(sock, "t%02d" % i) + ".corrupt-20260101T0000%02d-000001" % i, {"ticket_id": "t", "events": []})
        big = m.desired_roster_path(sock) + ".corrupt-20260101T000059-000001"
        with builtins.open(big, "wb") as f: f.write(b'{"roster": {}, "pad": "' + b"a" * (1 << 20) + b'"}')
        got = None if notice is None else [os.path.basename(x) for x in notice(sock)]
        res["notice_n"] = None if got is None else len(got)
        res["notice_has_big"] = None if got is None else (os.path.basename(big) in got)
        res["notice_left"] = len([f for f in os.listdir(home) if ".corrupt-" in f])
        with io.open(resfile, "w", encoding="ascii") as f: json.dump(res, f, ensure_ascii=True)
        sys.stdout.flush(); os._exit(0)
    elif scenario == "tombstone":
        force = getattr(m, "_force_utf8_stdio", None)
        if force: force()
        seed(os.path.join(sd, "topology.json"), {"schema_version": 1, "tombstones_rev": 3, "tombstones": ["worker"], "updated_at": 1,
             "entries": [{"role": "worker", "agent": "claude", "cwd": "/Users/홍길동/작업 — 폴더"}, {"role": "cso", "agent": "claude", "cwd": "/Users/홍길동"}]})
        parked = m.desired_roster_path(sock) + ".corrupt-20260101T000000-000001"
        seed(parked, {"roster": {"worker": {"role": "worker", "cwd": "/Users/홍길동/작업 — 폴더"}}, "tombstones": ["worker"]})
        entries, tombs = m.observe_and_persist_roster(sock)
        res["tombs"] = sorted(tombs); res["roster_roles"] = sorted(entries.keys()) if isinstance(entries, dict) else sorted(e.get("role") for e in entries)
        notice = getattr(m, "_notice_requalified_corrupt", None)
        res["notice"] = None if notice is None else [os.path.basename(x) for x in notice(sock)]
        res["parked_still_there"] = os.path.exists(parked)
        res["corrupt_files"] = []          # 이 시나리오는 격리본을 일부러 심는다 — 아래 공통 수집을 덮어쓴다
        with io.open(resfile, "w", encoding="ascii") as f: json.dump(res, f, ensure_ascii=True)
        sys.stdout.flush(); os._exit(0)
    else:
        raise SystemExit("unknown scenario")
except SystemExit:
    raise
except Exception as e:
    res["exc"] = type(e).__name__ + ": " + str(e)[:160]
    res["tb"] = traceback.format_exc().strip().splitlines()[-4:]
res["corrupt_files"] = sorted(f for f in os.listdir(home) if ".corrupt-" in f)
done(1 if res["exc"] else 0)
'''


def _bare_env(extra=None):
    """자식 환경 — UTF-8 강제 변수를 벗긴다(데몬이 자동 복원에 주는 환경에는 이 둘이 없다)."""
    env = dict(os.environ)
    for k in ("PYTHONUTF8", "PYTHONIOENCODING", "LC_ALL", "LC_CTYPE", "LANG"):
        env.pop(k, None)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    if os.name != "nt":
        env["LC_ALL"] = "en_US.UTF-8"
    env.update(extra or {})
    return env


def run_child(td, child, cond, scenario):
    """한 조건 × 한 시나리오. 반환: (rc, 결과 dict|None, stdout 바이트)."""
    kind, codec = cond
    work = tempfile.mkdtemp(prefix="w-", dir=td)
    resfile = os.path.join(work, "result.json"); outfile = os.path.join(work, "stdout.bin")
    env = _bare_env({"LC_ALL": "ko_KR.CP949"} if kind == "locale" else None)
    shim = codec if kind == "shim" else "-"
    with open(outfile, "wb") as of, open(os.path.join(work, "stderr.txt"), "wb") as ef:
        rc = subprocess.run([sys.executable, child, PH, shim, scenario, work, resfile], env=env, stdin=subprocess.DEVNULL,
                            stdout=of, stderr=ef, timeout=120).returncode
    res = None
    if os.path.exists(resfile):
        with open(resfile, encoding="ascii") as f:
            res = json.load(f)
    with open(outfile, "rb") as f:
        out = f.read()
    return rc, res, out


def _norm(enc):
    return (enc or "").lower().replace("-", "").replace("_", "")


def conditions(td, child):
    conds = [("shim", "cp949"), ("shim", "cp1252"), ("native", None), ("shim", "utf-8")]
    if os.name != "nt":
        rc, res, _ = run_child(td, child, ("locale", "cp949"), "probe")
        ok = bool(res) and _norm(res.get("stdout_enc")) == "cp949" and _norm(res.get("open_default")) == "cp949"
        print("조건 탐침 locale:cp949 →", "성립" if ok else "미성립(이 기계에 ko_KR.CP949 로케일이 없다 — shim:cp949 로만 잰다)", res)
        if ok:
            conds.insert(0, ("locale", "cp949"))
    return conds


def label(cond):
    return cond[0] + (":" + cond[1] if cond[1] else "")


def t_files(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "files")
    L = label(cond)
    check("%s files: 자식 종료코드 0" % L, rc == 0, "rc=%s exc=%s" % (rc, res and res.get("exc")))
    if not res:
        check("%s files: 결과 파일" % L, False, "없음"); return
    check("%s files: 멀쩡한 desired_roster·.bak 을 valid 로 본다" % L, res.get("status_desired") == "valid" and res.get("status_bak") == "valid",
          "desired=%s bak=%s (open 기본=%s)" % (res.get("status_desired"), res.get("status_bak"), res.get("open_default")))
    check("%s files: 손상 복구 경로를 타지 않는다" % L, res.get("recover") == "valid", "recover=%s" % res.get("recover"))
    check("%s files: 로스터·묘비·부서·좌석표를 읽는다" % L,
          res.get("desired_n") == 1 and res.get("tombstones") == ["reviewer-x"] and res.get("dept_n") == 1 and res.get("topo_n") == 1 and not res.get("topo_error"),
          "desired_n=%s tombstones=%s dept_n=%s topo_n=%s topo_error=%s" % (res.get("desired_n"), res.get("tombstones"), res.get("dept_n"), res.get("topo_n"), res.get("topo_error")))
    check("%s files: 저널·배포 저널을 읽는다" % L, res.get("journal_events") == 1 and res.get("deploy_events") == 1,
          "journal=%s deploy=%s" % (res.get("journal_events"), res.get("deploy_events")))
    check("%s files: .corrupt-* 로 치운 파일 0" % L, res.get("corrupt_files") == [], "corrupt=%s" % res.get("corrupt_files"))


def t_stdout(td, child, cond):
    rc, res, out = run_child(td, child, cond, "stdout")
    L = label(cond)
    check("%s stdout: 줄표·★·한글 로그와 결과 JSON 을 쓰고 종료코드 0" % L, rc == 0, "rc=%s exc=%s" % (rc, res and res.get("exc")))
    try:
        text = out.decode("utf-8"); ok = "한글 — 줄표 ★" in text and "진행 중 — skip ★" in text
    except UnicodeDecodeError as e:
        text = ""; ok = False
    check("%s stdout: 로그 파일이 UTF-8 이고 글자가 그대로 남는다" % L, ok, "bytes=%d" % len(out))


def t_entry(td, child, cond):
    rc, res, out = run_child(td, child, cond, "entry_plan")
    L = label(cond)
    plan = None
    try:
        plan = json.loads(out.decode("utf-8"))
    except Exception:
        pass
    check("%s 진입점 deploy --plan: 종료코드 0 + UTF-8 JSON" % L, rc == 0 and isinstance(plan, dict) and plan.get("deploy") == "PLAN",
          "rc=%s exc=%s bytes=%d" % (rc, res and res.get("exc"), len(out)))


def t_gen_manual(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "gen_manual")
    L = label(cond)
    check("%s gen-manual: 수동 복구 스크립트를 UTF-8 로 쓴다" % L, rc == 0 and bool(res) and res.get("manual_utf8_ok") is True,
          "rc=%s exc=%s" % (rc, res and res.get("exc")))
    if res and res.get("manual_run") is not None:
        check("%s gen-manual: 만든 스크립트가 cp1252 터미널에서도 복원 명령 줄을 끝까지 낸다" % L, res.get("manual_run") == [True, True], str(res.get("manual_run")))


def t_status_recv(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "status_recv")
    L = label(cond)
    check("%s 상태 응답 수신: 한글·줄표 제목이 든 cys status --json 을 받아 신원을 읽는다" % L,
          rc == 0 and bool(res) and res.get("daemon_build") == "abc123", "rc=%s build=%s exc=%s" % (rc, res and res.get("daemon_build"), res and res.get("exc")))
    check("%s 상태 응답 수신: cys() 가 글자를 그대로 돌려준다" % L, bool(res) and res.get("cys_rc") == 0 and res.get("cys_title_ok") is True,
          "cys_rc=%s title_ok=%s" % (res and res.get("cys_rc"), res and res.get("cys_title_ok")))


def t_native_recv(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "native_recv")
    L = label(cond)
    check("%s 외부 도구 수신: 다른 코드페이지 바이트를 받아도 종료코드를 지킨다(수신 예외로 127 이 되지 않는다)" % L,
          rc == 0 and bool(res) and res.get("schtasks_rc") == 0, "rc=%s schtasks_rc=%s exc=%s" % (rc, res and res.get("schtasks_rc"), res and res.get("exc")))
    check("%s 외부 도구 수신: _dec_any 가 cp949·UTF-8·빈 바이트를 예외 없이 푼다" % L, bool(res) and res.get("dec_any") == [True, True, True], str(res and res.get("dec_any")))


def t_manifest(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "manifest")
    L = label(cond)
    check("%s 스냅샷 manifest: 한글·줄표 경로를 UTF-8 로 쓰고 다시 검증한다" % L,
          rc == 0 and bool(res) and res.get("manifest_utf8") is True and res.get("verify_new") == 0,
          "rc=%s utf8=%s verify=%s exc=%s" % (rc, res and res.get("manifest_utf8"), res and res.get("verify_new"), res and res.get("exc")))
    if res and _norm(res.get("locale_pref")) == "cp949":
        check("%s 스냅샷 manifest: 과거 cp949 로 쓰인 것도 읽는다" % L, res.get("verify_old_cp949") == 0, "verify_old=%s" % res.get("verify_old_cp949"))
    if res and _norm(res.get("open_default")) == "cp949" and res.get("verify_ambiguous_cp949") is not None:
        check("%s 스냅샷 manifest: UTF-8 로도 읽히는 cp949 이름(징.md)을 저장 파일로 가려 읽는다" % L, res.get("verify_ambiguous_cp949") == 0,
              "verify=%s" % res.get("verify_ambiguous_cp949"))


def t_tombstone(td, child, cond):
    rc, res, _ = run_child(td, child, cond, "tombstone")
    L = label(cond)
    check("%s 묘비: desired_roster 가 없어도 topology 의 묘비가 지켜진다" % L, rc == 0 and bool(res) and "worker" in (res.get("tombs") or []),
          "rc=%s tombs=%s exc=%s" % (rc, res and res.get("tombs"), res and res.get("exc")))
    check("%s 치워진 파일: 멀쩡히 읽히는 격리본을 알리되 건드리지 않는다" % L,
          bool(res) and res.get("notice") == ["desired_roster.json.corrupt-20260101T000000-000001"] and res.get("parked_still_there") is True,
          "notice=%s still=%s" % (res and res.get("notice"), res and res.get("parked_still_there")))


def t_extra(td, child, cond):
    L = label(cond)
    rc, res, _ = run_child(td, child, cond, "emit_child")
    check("%s 이벤트 자식: 어느 인코딩으로도 안 풀리는 출력을 내도 _emit_evt 가 예외 없이 종료코드만 본다" % L,
          rc == 0 and bool(res) and res.get("emit_ok") is True, "rc=%s emit=%s exc=%s" % (rc, res and res.get("emit_ok"), res and res.get("exc")))
    rc, res, _ = run_child(td, child, cond, "log_flush")
    check("%s log(): flush 가 실패해도 죽지 않고 같은 줄을 두 번 쓰지 않는다" % L,
          rc == 0 and bool(res) and res.get("log_flush") == {"oserror": 1, "unicode": 1}, "rc=%s %s exc=%s" % (rc, res and res.get("log_flush"), res and res.get("exc")))
    rc, res, _ = run_child(td, child, cond, "notice_cap")
    check("%s 치워진 파일: 알림이 여는 격리본을 30개·1 MiB 로 묶고 아무것도 지우지 않는다" % L,
          bool(res) and res.get("notice_n") == 29 and res.get("notice_has_big") is False and res.get("notice_left") == 36,  # 36개 중 최근 30개를 살피고 그 안의 큰 파일 1개는 건너뛴다
          "n=%s big=%s left=%s exc=%s" % (res and res.get("notice_n"), res and res.get("notice_has_big"), res and res.get("notice_left"), res and res.get("exc")))


def t_source_pins():
    """자리 고정 — 재설정이 main() 의 selftest 판정보다 앞에 있고, 사람용 복구 틀 안의 읽기 2곳도 utf-8 을 적는다."""
    with open(PH, encoding="utf-8") as f:
        src = f.read()
    i = src.find("\ndef main():"); body = src[i:]
    a = body.find("_force_utf8_stdio()"); b = body.find('"--selftest" in sys.argv')
    check("자리: main() 이 selftest 판정보다 먼저 stdio 를 UTF-8 로 재설정한다", 0 < a < b, "force@%d selftest@%d" % (a, b))
    import ast
    n_text = 0
    for node in ast.walk(ast.parse(src)):
        if isinstance(node, ast.Call):
            for kw in node.keywords:
                if kw.arg in ("text", "universal_newlines") and not (isinstance(kw.value, ast.Constant) and kw.value.value in (False, None)):
                    n_text += 1
    check("자리: 하위 프로세스 출력을 기본 인코딩의 글자로 받는 호출(text=True)이 0곳", n_text == 0, "text=True %d곳" % n_text)
    check("자리: 수동 복구 틀 안의 json.load(open(...)) 2곳이 encoding 을 적는다",
          "json.load(open('$TOPO'))" not in src and "json.load(open(sys.argv[1]))" not in src
          and "json.load(open('$TOPO', encoding='utf-8'))" in src and "json.load(open(sys.argv[1], encoding='utf-8'))" in src)


def t_e2e_daemon(td, conds):
    """선택 — 격리 데몬 + 실제 진입점 `restore --auto`(데몬이 주는 환경 키 그대로 · PYTHONUTF8 없음 · LC_ALL=ko_KR.CP949)."""
    cysd = os.environ.get("PHOENIX_HARNESS_CYSD"); cys = os.environ.get("PHOENIX_CYS")
    if os.name == "nt" or not cysd or not cys or not os.path.exists(cysd) or not os.path.exists(cys) or ("locale", "cp949") not in conds:
        print("건너뜀 E(격리 데몬 끝에서 끝): 조건 미성립 — PHOENIX_HARNESS_CYSD=%s PHOENIX_CYS=%s locale:cp949=%s"
              % (bool(cysd), bool(cys), ("locale", "cp949") in conds))
        return
    root = tempfile.mkdtemp(prefix="cysE", dir="/tmp")     # 유닉스 소켓 경로 길이 한도 때문에 짧은 자리
    home = os.path.join(root, "h"); sd = os.path.join(root, "s"); ph_home = os.path.join(sd, "phoenix")
    for p in (home, ph_home, os.path.join(root, "t")):
        os.makedirs(p)
    sock = os.path.join(sd, "cys.sock")
    def seed(path, obj):
        with open(path, "wb") as f:
            f.write(json.dumps(obj, ensure_ascii=False, indent=1).encode("utf-8"))
    seed(os.path.join(ph_home, "journal-default.json"), {"ticket_id": "default", "roles": {}, "events": [{"ts": 1, "role": "*", "stage": "s0", "status": "ok", "msg": "복원 시작 — 1단계"}], "created": 1})
    seed(os.path.join(ph_home, "desired_roster.json"), {"roster": {"worker": {"role": "worker", "agent": "claude", "cwd": "/Users/홍길동/작업 — 폴더"}}, "tombstones": ["worker"],
                                                       "tombstones_rev": None, "daemon_epoch": None, "state_dir_tag": os.path.realpath(sd), "updated_at": 1})
    denv = {"HOME": home, "PATH": "/usr/bin:/bin", "LC_ALL": "ko_KR.CP949", "CYS_SOCKET": sock, "CYS_NO_OFFICE_BRIDGE": "1", "CYS_NO_AUTORESTORE": "1",
            "TMPDIR": os.path.join(root, "t"), "CYS_PACK_DIR": os.path.join(root, "pack")}
    errf = open(os.path.join(root, "cysd.stderr.txt"), "wb")
    p = subprocess.Popen([cysd], env=denv, stdin=subprocess.DEVNULL, stdout=errf, stderr=errf, start_new_session=True, cwd=root)
    try:
        for _ in range(60):
            if os.path.exists(sock):
                break
            time.sleep(0.5)
        time.sleep(2)
        cenv = dict(denv); cenv.pop("CYS_SOCKET")
        cenv.update({"PHOENIX_CYS": cys, "CYS_NO_AUTOSTART": "1", "PYTHONDONTWRITEBYTECODE": "1"})
        # 좌석 제목에 한글·줄표 — 상태 응답(cys status --json)에 비ASCII 가 실리게 한다(수신 디코딩 경로를 실제로 탄다).
        uenv = dict(cenv); uenv["LC_ALL"] = "en_US.UTF-8"
        ns = subprocess.run([cys, "--socket", sock, "new-surface", "--cwd", root, "--title", "한글 — 좌석 제목"], env=uenv, capture_output=True, timeout=30)
        check("E: 한글 제목 좌석 생성(시험 전제)", ns.returncode == 0, "rc=%s" % ns.returncode)
        logp = os.path.join(root, "phoenix-restore.log")
        with open(logp, "wb") as lf:
            rc = subprocess.run([sys.executable, PH, "--socket", sock, "restore", "--auto"], env=cenv, stdin=subprocess.DEVNULL, stdout=lf, stderr=lf, timeout=180).returncode
        with open(logp, "rb") as f:
            raw = f.read()
        text = raw.decode("utf-8", "replace")
        after = sorted(os.listdir(ph_home))
        check("E locale:cp949 격리 데몬 restore --auto: 종료코드 0", rc == 0, "rc=%s" % rc)
        check("E: 로그에 Traceback·Unicode 예외 0줄", "Traceback" not in text and "UnicodeEncodeError" not in text and "UnicodeDecodeError" not in text,
              "Traceback=%d" % text.count("Traceback"))
        check("E: 멀쩡한 저널·desired_roster 가 제자리에 있고 .corrupt-* 0", "journal-default.json" in after and "desired_roster.json" in after and not [f for f in after if ".corrupt-" in f], str(after))
        check("E: 묘비가 읽혀 지켜진다(로그)", "worker" in text and "묘비" in text, "")
    finally:
        try:
            os.killpg(p.pid, signal.SIGTERM)
        except Exception:
            pass
        time.sleep(1.0)
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except Exception:
            pass
        errf.close()
        shutil.rmtree(root, ignore_errors=True)


def main():
    try:  # 이 시험 자신의 출력은 UTF-8 로(윈도우 러너 cp1252 에서 한글 PASS/FAIL 줄이 죽지 않게 — 자식 조건과는 무관하다)
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
    td = tempfile.mkdtemp(prefix="phoenix-enc-")
    try:
        child = os.path.join(td, "child.py")
        with open(child, "w", encoding="utf-8") as f:
            f.write(CHILD)
        conds = conditions(td, child)
        for cond in conds:
            rc, res, _ = run_child(td, child, cond, "probe")
            print("조건 %s: stdout=%s errors=%s open 기본=%s utf8_mode=%s" % (label(cond), res and res.get("stdout_enc"), res and res.get("stdout_errors"),
                                                                       res and res.get("open_default"), res and res.get("utf8_mode")))
            if cond[0] == "shim":
                check("%s 장치 검증: 의도한 코덱이 잡혔다" % label(cond), bool(res) and _norm(res.get("stdout_enc")) == _norm(cond[1]) and _norm(res.get("open_default")) == _norm(cond[1]))
            t_files(td, child, cond)
            t_stdout(td, child, cond)
            t_entry(td, child, cond)
            t_gen_manual(td, child, cond)
            t_status_recv(td, child, cond)
            t_native_recv(td, child, cond)
            t_manifest(td, child, cond)
            t_tombstone(td, child, cond)
            t_extra(td, child, cond)
        t_source_pins()
        t_e2e_daemon(td, conds)
    finally:
        shutil.rmtree(td, ignore_errors=True)
    ok = all(_results)
    print("\n%d/%d PASS" % (sum(_results), len(_results)))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
