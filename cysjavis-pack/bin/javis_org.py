#!/usr/bin/env python3
"""javis_org.py — 부서 자동 편성 브리지 (org-manifest → 검증·적용·착수확인·삭제).
설계: multi-master-ceo/2026-06-26-org-provisioning-design.md (v2)
하우스스타일: javis_manifest.py (--self-test 밀폐 검증)
exit: 0=성공 1=위반/실패 2=입출력 3=권한(CSO아님) 4=대상없음
(org-audit 동사: 0=수렴 1=미수렴 2=입출력 — read-only 파생 집계·영속 0 · T10/P3-3)
"""
import argparse, json, os, sys, hashlib, subprocess, tempfile, tarfile, time, shutil

# RC-6: OS중립 파일락 — unix는 fcntl.flock(제로 회귀·파일 닫힐 때 자동 해제), Windows는 fcntl
# 부재라 msvcrt 바이트락으로 폴백(과거 top-level `import fcntl`이 Windows에서 즉시 ModuleNotFoundError로
# javis_org 전체 불능이던 P0 차단). 락 실패해도 최종 원자교체(os.replace)가 일관성 보장 → best-effort.
try:
    import fcntl as _fcntl
    def _flock(f):
        _fcntl.flock(f, _fcntl.LOCK_EX)
except ImportError:  # Windows
    import msvcrt as _msvcrt
    def _flock(f):
        try:
            _msvcrt.locking(f.fileno(), _msvcrt.LK_LOCK, 1)
        except OSError:
            pass

HOME = os.path.expanduser("~")
CATALOG = os.environ.get("CYS_DEPT_CATALOG", f"{HOME}/.cys/dept-catalog.json")
DEPTS = os.environ.get("CYS_DEPTS_JSON", f"{HOME}/.cys/depts.json")
MISSIONS = os.environ.get("CYS_DEPT_MISSIONS", f"{HOME}/.cys/dept-missions")
ALLOWED_ROLES = ("worker", "reviewer", "cso")  # tasks[].to enum
MIN_QUOTE = 20  # source_quote 최소 길이(F3)

def expand(p): return os.path.expandvars(os.path.expanduser(p)) if p else p

def load_json(path, default=None):
    if not os.path.exists(path):
        if default is not None: return default
        raise FileNotFoundError(path)
    with open(path, encoding="utf-8") as f:
        return json.load(f)

def sha256_text(s): return hashlib.sha256(s.encode("utf-8")).hexdigest()
def sha256_file(path):
    with open(path, "rb") as f: return hashlib.sha256(f.read()).hexdigest()

def require_cso():
    if os.environ.get("CYS_ROLE") != "cso":
        sys.stderr.write("[javis_org] ★CSO 전용: apply/destroy는 CYS_ROLE=cso에서만(부서 mutation 단일소유). CSO에 위임하라.\n")
        sys.exit(3)

def v_schema(m):
    errs = []
    if not isinstance(m, dict): return ["매니페스트가 객체 아님"]
    if m.get("kind") != "org-manifest": errs.append("kind != 'org-manifest'")
    if m.get("manifest_version") != 1: errs.append("manifest_version != 1")
    if m.get("reconcile_mode", "additive") != "additive":
        errs.append("MVP reconcile_mode는 additive만(exact는 후속)")
    src = m.get("source") or {}
    if not src.get("design_doc"): errs.append("source.design_doc 누락")
    if not src.get("design_doc_sha256"): errs.append("source.design_doc_sha256 누락")
    for key in ("departments", "tasks"):
        if not isinstance(m.get(key), list): errs.append(f"필수 키 누락/배열아님: {key}")
    for i, d in enumerate(m.get("departments") or []):
        for f in ("key", "display", "account", "cwd", "mission_md", "source_quote"):
            if not d.get(f): errs.append(f"departments[{i}].{f} 누락")
    for i, t in enumerate(m.get("tasks") or []):
        for f in ("dept", "task", "scope", "source_quote"):
            if not t.get(f): errs.append(f"tasks[{i}].{f} 누락")
        if t.get("to", "worker") not in ALLOWED_ROLES:
            errs.append(f"tasks[{i}].to enum 위반: {t.get('to')}")
    return errs

def _atomic_write(path, obj):
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(dir=d, suffix=".tmp")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)

def catalog_upsert(catalog_path, dept):
    """catalog 전용 .lock으로 직렬화 + 원자교체. mission_key=key 규약."""
    lock = catalog_path + ".lock"
    with open(lock, "w") as lf:
        _flock(lf)
        cat = load_json(catalog_path, {"version":1,"accounts":{},"departments":{}})
        cat.setdefault("departments", {})[dept["key"]] = {
            "display": dept["display"], "account": dept["account"],
            "mission_key": dept["key"], "cwd": dept["cwd"]}
        _atomic_write(catalog_path, cat)

def write_mission(key, mission_md):
    os.makedirs(MISSIONS, exist_ok=True)
    with open(os.path.join(MISSIONS, f"{key}.md"), "w", encoding="utf-8") as f:
        f.write(mission_md)

def ensure_dirs(dept):
    cwd = expand(dept["cwd"])
    os.makedirs(cwd, exist_ok=True)
    return cwd

def backfill_mission_key(depts_path, key, mission_key, display=None):
    """레거시 부서에 mission_key 소급(F6). cys-dept와 같은 .lock으로 직렬화, 다른 필드 무접촉.
    매처: cwd 마지막 세그먼트가 display(한글 규약·1차) 또는 key(영문 레거시·2차)와 정확일치(R3-1).
    정확일치라 suffix-bleed(R1 BLOCK-1) 회귀 없음. expand()로 $HOME 토큰 정규화."""
    lock = depts_path + ".lock"
    with open(lock, "w") as lf:
        _flock(lf)
        reg = load_json(depts_path, {"depts":{}})
        for name, e in reg.get("depts", {}).items():
            cwd_base = os.path.basename(expand(e.get("cwd","")).rstrip("/"))
            if cwd_base == display or cwd_base == key:  # 한글 display 1차·영문 key 레거시 2차
                if not e.get("mission_key"):
                    e["mission_key"] = mission_key
        _atomic_write(depts_path, reg)

def v_catalog_consistency(depts, catalog):
    """기존 catalog key의 recorded display/cwd ≠ manifest display/cwd면 거부 (R3-2 display drift 차단).
    신규 key(catalog 미등록)는 대상 아님 — F1 승인플래그가 담당."""
    errs = []
    cat_depts = catalog.get("departments") or {}
    for i, d in enumerate(depts):
        key = d.get("key", "")
        rec = cat_depts.get(key)
        if not rec: continue  # 신규 key는 drift 대상 아님
        tag = f"departments[{i}]({key})"
        if rec.get("display") and d.get("display") and rec["display"] != d.get("display"):
            errs.append(f"{tag}: catalog display drift — catalog '{rec['display']}' ≠ manifest '{d.get('display')}'")
        if rec.get("cwd") and d.get("cwd") and expand(rec["cwd"]) != expand(d.get("cwd")):
            errs.append(f"{tag}: catalog cwd drift — catalog '{rec['cwd']}' ≠ manifest '{d.get('cwd')}'")
    return errs

def v_refs(depts, tasks):
    keys = {d.get("key") for d in depts}
    return [f"tasks[{i}].dept '{t.get('dept')}' 미존재(참조 무결성)"
            for i, t in enumerate(tasks) if t.get("dept") not in keys]

def v_sha256(actual, expected):
    return [] if actual == expected else [f"design_doc sha256 불일치(SOT 드리프트): 실제 {actual[:12]}… ≠ 매니페스트 {expected[:12]}…"]

def validate_manifest(m, doc_text=None, catalog=None):
    errs = v_schema(m)
    if errs: return errs  # 스키마 깨지면 이후 검사 무의미
    catalog = catalog if catalog is not None else load_json(CATALOG, {})
    if doc_text is None:
        dd = expand(m["source"]["design_doc"])
        doc_text = open(dd, encoding="utf-8").read() if os.path.exists(dd) else ""
        errs += v_sha256(sha256_text(doc_text) if doc_text else "", m["source"]["design_doc_sha256"])
    errs += v_quote_binding(m.get("departments", []), doc_text, catalog)
    errs += v_quote_binding(  # tasks는 quote-수준(길이/존재/고유)만 — 정체결속·account·cwd는 dept만
        [{"key":t.get("dept"),"display":"","account":"","cwd":"","source_quote":t.get("source_quote","")}
         for t in m.get("tasks", [])], doc_text, {"accounts":{},"departments":{}}, dept_level=False)
    errs += v_catalog_consistency(m.get("departments", []), catalog)  # R3-2 display/cwd drift
    errs += v_refs(m.get("departments", []), m.get("tasks", []))
    return errs

def cmd_validate(path):
    try:
        m = load_json(path)
    except Exception as e:
        sys.stderr.write(f"[validate] 매니페스트 로드 실패: {e}\n"); return 2
    errs = validate_manifest(m)
    if errs:
        sys.stderr.write("[validate] FAIL:\n" + "\n".join(f"  - {e}" for e in errs) + "\n")
        return 1
    print(json.dumps({"validate": "ok", "departments": len(m["departments"]),
                      "tasks": len(m["tasks"])}, ensure_ascii=False))
    return 0

def _norm(s): return " ".join((s or "").split())

def v_quote_binding(depts, doc_text, catalog, dept_level=True):
    """F1: quote 존재+정체결속+key↔account+cwd패턴+span유일+길이/고유성.
    dept_level=False(tasks)면 dept-수준 검사(정체·account·cwd)를 건너뛰고 quote-수준(길이/존재/고유/span)만."""
    errs = []
    ndoc = _norm(doc_text)
    accounts = (catalog.get("accounts") or {}).keys()
    cat_depts = catalog.get("departments") or {}
    disp_to_key = {v.get("display"): k for k, v in cat_depts.items() if v.get("display")}  # 역인덱스(R1 BLOCK-2)
    seen_quotes = {}
    for i, d in enumerate(depts):
        key, disp, acct = d.get("key",""), d.get("display",""), d.get("account","")
        q = _norm(d.get("source_quote", ""))
        tag = f"departments[{i}]({key})"
        # 길이(F3) → 존재 → 고유성(F3) 순 (존재성을 고유성보다 먼저 — 부재 분기 도달 보장)
        if len(q) < MIN_QUOTE:
            errs.append(f"{tag}: source_quote 길이<{MIN_QUOTE}")
        elif q not in ndoc:
            errs.append(f"{tag}: source_quote 부재(doc에 없음)")
        elif ndoc.count(q) != 1:
            errs.append(f"{tag}: source_quote 고유성 위반(doc 내 {ndoc.count(q)}회)")
        else:
            # 정체 결속: quote가 display 또는 key 변별토큰 포함 (dept-수준만)
            if dept_level and disp not in q and key not in q:
                errs.append(f"{tag}: quote가 부서 정체(display/key) 미포함 — 결속 실패")
            # span 유일성 (quote-수준)
            if q in seen_quotes:
                errs.append(f"{tag}: source_quote가 {seen_quotes[q]}와 동일 span 재사용")
            seen_quotes[q] = tag
        if dept_level:
            # ★key 결속(R1 BLOCK-2): quote가 가리키는 catalog display의 정규 key 중 매니페스트 key가
            #   있어야 함. display 위장(실재 부서명을 허위 key에 부착)을 account 대조 없이 차단.
            matched_keys = {ck for cd, ck in disp_to_key.items() if cd and cd in q}
            if matched_keys and key not in matched_keys:
                errs.append(f"{tag}: quote가 가리키는 정규 key {sorted(matched_keys)} 중 매니페스트 key='{key}' 없음 — 오귀속")
            # key↔account 일관성
            if key in cat_depts:
                want = cat_depts[key].get("account")
                if want and acct != want:
                    errs.append(f"{tag}: account 오배정({acct}≠catalog {want})")
            else:
                # 신규 key(catalog 미등록): account 유효성 + 오너 승인 플래그 둘 다 필수 (R1 BLOCK-2)
                if acct not in accounts:
                    errs.append(f"{tag}: 신규 key의 account '{acct}'가 승인 accounts에 없음")
                if not d.get("new_dept_approved"):
                    errs.append(f"{tag}: 신규 key '{key}'는 오너 승인 플래그(new_dept_approved) 필요 — account 유효성만으로 통과 불가")
            # cwd 규약 경로
            cwd = d.get("cwd", "")
            if disp and not cwd.replace("\\", "/").endswith(f"Desktop/CYSjavis/{disp}"):
                errs.append(f"{tag}: cwd가 규약 경로($HOME/Desktop/CYSjavis/{disp}) 불일치")
    return errs

def intake_ok(surfaces, idle_max=600):
    """착수 PASS = role=worker 별개 surface가 alive + 데몬 관측 하드신호(idle/line). 부서장 working은 불충분."""
    for s in surfaces:
        if s.get("role") != "worker": continue
        if not s.get("agent_alive"): continue
        active = (s.get("idle_secs", 1e9) < idle_max) or (s.get("line_count", 0) > 0) or (s.get("queue_depth", 0) > 0)
        if active: return True
    return False

def dept_status(socket):
    """부서 소켓의 cys status --json 회수."""
    r = subprocess.run(["cys", "--socket", socket, "status", "--json"],
                       capture_output=True, text=True, env={**os.environ, "CYS_NO_AUTOSTART": "1"})
    if r.returncode != 0: return None
    try: return json.loads(r.stdout)
    except Exception: return None

def cmd_status(path=None):
    reg = load_json(DEPTS, {"depts":{}})
    keys = None
    if path:
        m = load_json(path); keys = {d["key"] for d in m["departments"]}
    rows = []
    for name, e in reg.get("depts", {}).items():
        if keys is not None and e.get("mission_key") not in keys: continue
        st = dept_status(e["socket"])
        surfaces = (st or {}).get("surfaces", [])
        rows.append({"dept": name, "display": e.get("display_name", name),
                     "alive": st is not None, "intake": intake_ok(surfaces)})
    ok = all(r["intake"] for r in rows) if rows else False
    print(json.dumps({"status": "ok" if ok else "incomplete", "depts": rows}, ensure_ascii=False))
    return 0 if ok else 1

# ── ★T10(P3-3 최소안) org-audit — pack-only **read-only** 파생 집계 ─────────────────────────
# 정본 이원화 금지(R3 corrected (b)안): 이미 9갈래 부분 진실(depts.json·topology·org.status·
# formation 상태파일·티켓·레인 마커·ceo-pending/.pre-ceo/영수증·묘비 쌍·fallback map)이 있고
# 복원 리바이버(spawn_org_restore)는 depts.json 에 결박돼 있다 — 신설 영속 정본(org-state.json)은
# 10번째 갈래+복원 이원화라 기각. 이 동사는 **매번 파생 계산**만 하고(영속 쓰기 0) 판정을
# exit code(0=수렴/1=미수렴/2=입출력)로 낸다. 기존 cmd_status 의 exit 계약(집계=intake 축)은
# 무접촉 보존 — org-audit 는 편제(편성·승격) 축의 별도 동사다.
# ★라이브 관측 우선 규약(P3-3 결합 4행): formation 상태파일은 '전이 감지용 캐시'다 — 여기서는
#   updated_epoch 를 붙인 참고 필드(formation_cache)로만 노출하고, 판정(team_state)은 데몬
#   org.status(cys status --json) 라이브 관측으로만 한다(캐시를 실시간 사실로 승격 인용 금지).

def _audit_formation_mod():
    """형제 모듈 javis_formation 로드(REQUIRED_ROLES SOT·캐시 판독기 재사용) — 실패=None(폴백)."""
    try:
        d = os.path.dirname(os.path.abspath(__file__))
        if d not in sys.path:
            sys.path.insert(0, d)
        import javis_formation
        return javis_formation
    except Exception:
        return None

def _audit_daemon_status(socket=None):
    """데몬 org.status 회수(`cys status --json` = org.status RPC) — read-only·autostart 금지.
    socket=None 은 base(부서 레인에서 실행돼도 CYS_SOCKET 스크럽으로 base 데몬만 본다)."""
    env = {**os.environ, "CYS_NO_AUTOSTART": "1"}
    if socket is None:
        env.pop("CYS_SOCKET", None)
        cmd = ["cys", "status", "--json"]
    else:
        cmd = ["cys", "--socket", socket, "status", "--json"]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=10)
    except Exception:
        return None
    if r.returncode != 0:
        return None
    try:
        d = json.loads(r.stdout)
        return d if isinstance(d, dict) else None
    except Exception:
        return None

def _audit_seat(surfaces, role):
    """역할 좌석 관측: agent(부착) / empty_shell(좌석만) / absent. agent '존재'를 쓴다 —
    부팅 중(미seen) 노드를 결원으로 오판하지 않는 보수 판정(dept_classify_idle 선례)."""
    for s in surfaces:
        if s.get("exited"):
            continue
        if (s.get("role") or "") != role:
            continue
        return "agent" if s.get("agent") is not None else "empty_shell"
    return "absent"

def _audit_ticket_state(name):
    """티켓 대장 관측(정보 전용 — 오너 결정 ⑩B: 상비편성이 admission 정본·티켓은 수동 선언
    레인 전용이라 수렴 판정에 **불참**)."""
    d = os.path.join(HOME, ".cys", "state", "dept-boot-tickets")
    if os.path.exists(os.path.join(d, name + ".ticket")):
        return "present"
    if os.path.exists(os.path.join(d, name + ".ticket.used")):
        return "used"
    return "none"

def _audit_ceo_axis(dept_count):
    """CEO 축 파생: 마커(부트·pending)·.pre-ceo·영수증·디렉티브 등가로 승격 상태를 계산.
    수렴 = (부서 0 ∧ 미승격) 또는 (부서 ≥1 ∧ 승격 ∧ PENDING 없음 ∧ base 데몬 생존)."""
    base_pack = os.path.join(HOME, ".cys", "pack")          # cys-dept PACK_DEFAULT 동일 규약
    md = os.path.join(base_pack, "directives", "MASTER_DIRECTIVE.md")
    ceo = os.path.join(base_pack, "directives", "CEO_TEMPLATE.md")
    pre = md + ".pre-ceo"
    receipt = os.path.join(base_pack, "directives", ".ceo-template-applied")
    pending = os.path.join(HOME, ".cys", "state", "ceo-pending")
    marker = os.path.join(HOME, ".cys", ".master-bootstrapped")
    same = False
    try:
        with open(md, "rb") as a, open(ceo, "rb") as b:
            same = a.read() == b.read()
    except OSError:
        same = False
    promoted = os.path.exists(pre) or same   # 영수증 단독은 승격 증거 아님(강등이 증거 보존)
    is_pending = os.path.exists(pending)
    base_st = _audit_daemon_status(None)
    base_alive = base_st is not None
    if dept_count >= 1:
        state = "ceo" if (promoted and not is_pending) else ("pending" if is_pending else "unpromoted")
        conv = promoted and not is_pending and base_alive
    else:
        state = "demote-due" if promoted else "master"
        conv = not promoted   # 부서 0 이면 base 데몬 생존은 편제 수렴 판정 밖(정보 필드로만)
    return {"state": state, "promoted": promoted, "pending": is_pending,
            "boot_marker": os.path.exists(marker), "receipt": os.path.exists(receipt),
            "daemon_alive": base_alive, "dept_count": dept_count, "converged": conv}

def cmd_org_audit():
    """org-audit — {ceo, depts:{name:{daemon_alive, master_seat, team_state}}} 파생 출력.
    exit 0=수렴 / 1=미수렴 / 2=입출력. 영속 쓰기 0 · 스폰 0 · read-only(역할 가드 불요)."""
    try:
        reg = load_json(DEPTS, {"depts": {}})
    except Exception as e:
        sys.stderr.write("[org-audit] depts.json 판독 실패: %s\n" % e)
        return 2
    fm = _audit_formation_mod()
    required = list(getattr(fm, "REQUIRED_ROLES", None) or
                    ("master", "cso", "worker", "reviewer-gemini", "reviewer-codex"))
    external = set()
    if fm is not None:
        try:
            external = set(fm._external_roles())
        except Exception:
            external = set()
    need = [r for r in required if r not in external]
    depts_out, all_ok = {}, True
    for name, e in sorted((reg.get("depts") or {}).items()):
        sock = (e or {}).get("socket") or ""
        st = _audit_daemon_status(sock) if sock else None
        alive = st is not None
        surfaces = (st or {}).get("surfaces", [])
        master_seat = _audit_seat(surfaces, "master") if alive else "unknown"
        missing = [r for r in need if _audit_seat(surfaces, r) != "agent"] if alive else list(need)
        if not alive:
            team_state = "unknown"
        elif not missing:
            team_state = "complete"
        elif len(missing) == len(need):
            team_state = "empty"
        else:
            team_state = "partial"
        cache = {}
        if fm is not None and sock:
            try:
                obj = fm._read_state_obj(sock)
                if obj:
                    cache = {"state": obj.get("state"),
                             "updated_epoch": obj.get("updated_epoch")}
            except Exception:
                cache = {}
        conv = alive and master_seat == "agent" and team_state == "complete"
        all_ok = all_ok and conv
        depts_out[name] = {"daemon_alive": alive, "master_seat": master_seat,
                           "team_state": team_state, "missing": missing,
                           "ticket": _audit_ticket_state(name),
                           "formation_cache": cache,   # 참고 전용 — 판정은 라이브 관측(위)뿐
                           "converged": conv}
    ceo = _audit_ceo_axis(len(depts_out))
    all_ok = all_ok and ceo["converged"]
    print(json.dumps({"audit": "converged" if all_ok else "unconverged",
                      "ceo": ceo, "depts": depts_out}, ensure_ascii=False, indent=1))
    return 0 if all_ok else 1

def tar_snapshot(key, workdir, dest_dir=None):
    """--purge-workdir 의무 선행. 성공 시 경로 반환, 실패(소스 없음·예외) 시 None → 호출자가 rm 중단."""
    if not workdir or not os.path.isdir(workdir): return None
    dest_dir = dest_dir or f"{HOME}/.cys/dept-snapshots"
    os.makedirs(dest_dir, exist_ok=True)
    stamp = os.environ.get("CYS_SNAP_STAMP", str(int(os.path.getmtime(workdir))))
    out = os.path.join(dest_dir, f"{key}-{stamp}.tar.gz")
    # ★D1b(purge-safety 2026-07-16): 읽기불가 항목(TCC 거부 등)의 예외가 raw traceback으로 전파되고
    # 부분 tar가 잔존하던 결함 봉인 — 예외=부분 tar 정리 후 None(_snapshot_gate의 fail-closed 경로).
    try:
        with tarfile.open(out, "w:gz") as tar:
            tar.add(workdir, arcname=os.path.basename(workdir.rstrip("/")))
    except (OSError, tarfile.TarError) as ex:
        sys.stderr.write("[snapshot] %s 실패: %s: %s\n" % (workdir, type(ex).__name__, ex))
        try:
            if os.path.exists(out): os.remove(out)
        except OSError:
            pass
        return None
    return out if os.path.exists(out) else None

def _snapshot_gate(name, workdir):
    """--purge-workdir 선행 결정 (R1 REVISE-2): (proceed, action).
    workdir 부재=백업·삭제 둘 다 no-op → skip 후 진행(영구 락인 해소) /
    workdir 존재+스냅샷 실패=진짜 위험 → fail-closed abort."""
    if not (workdir and os.path.isdir(workdir)):
        return True, ("workdir_absent_skip", workdir)
    snap = tar_snapshot(name, workdir)
    if not snap:
        return False, ("abort_no_snapshot", workdir)
    return True, ("snapshot", snap)

# ★기능2: 격리(휴지통) 루트 — 삭제는 전부 mv로 통일(백업 없는 rm 금지). cys-dept의 TRASH_ROOT와 동일
# 규약 · discover_depts glob(`cys-dept-*`)은 `cys-trash/`를 무매치 = 재발견 절단(부활 차단·복구 양립).
TRASH_ROOT = f"{HOME}/.local/state/cys-trash"

# ★D1a(purge-safety 2026-07-16): workdir 격리 자격 게이트 — deny-by-default allowlist.
# 실사고: 전 부서 레지스트리 cwd=$HOME라 --purge-workdir가 홈 전체를 스냅샷(TCC .Trash에서 사망)하고,
# 성공했다면 홈을 trash로 mv하는 파괴 경로였다. cwd는 에이전트 작업 디렉토리(공유 가능) 의미이므로,
# 스냅샷·격리는 레지스트리 엔트리가 workdir_owned=true로 소유권을 선언한 부서 전용 경로에만 허용한다.
_PROTECTED_ROOTS = ("/", "/Users", "/tmp", "/var", "/private/tmp", "/private/var")

def _workdir_quarantine_eligible(workdir, entry):
    """(eligible, label). 평가 순서: 부재 → 보호루트/홈 → 소유 미선언 → 홈 밖(realpath) → 적격."""
    if not (workdir and os.path.isdir(workdir)):
        return False, "workdir_absent_skip"
    real = os.path.realpath(workdir)
    home = os.path.realpath(HOME)
    if real == home or real in tuple(os.path.realpath(p) for p in _PROTECTED_ROOTS):
        return False, "workdir_protected_skip"
    if entry.get("workdir_owned") is not True:
        return False, "workdir_shared_skip"
    if not real.startswith(home + os.sep):
        return False, "workdir_outside_home_skip"
    return True, "workdir_owned"

def _quarantine(src, trash_dir, label):
    """삭제 대신 격리(mv → trash_dir/label). 대상(디렉토리) 없으면 None, 격리하면 action 튜플."""
    if not (src and os.path.isdir(src)):
        return None
    os.makedirs(trash_dir, exist_ok=True)
    dest = os.path.join(trash_dir, label)
    shutil.move(src, dest)
    return (f"quarantine_{label}", dest)

def destroy_dept(name, mission_key, purge=False, purge_workdir=False, purge_state=False):
    require_cso()  # 게이트를 효과 함수에 (R1 REVISE-1) — import 직접호출 우회 차단
    actions = []
    # ★기능2 단일 진입점(오케스트레이터): pack-dept·workdir는 여기서 격리, state 디렉토리는 하위
    # 프리미티브(cys-dept down --purge-state)에 위임한다 — 세 디렉토리 모두 동일 trash 하위(같은 ts).
    ts = os.environ.get("CYS_TRASH_STAMP", str(int(time.time())))
    trash_dir = f"{TRASH_ROOT}/{name}-{ts}"
    # workdir 경로는 down(레지스트리 해제) **전에** 포착한다 — down 이후엔 depts.json 엔트리가 사라져
    # 조회가 빈값이 된다(종전 post-down 재조회 rmtree는 이 때문에 실운영에서 no-op이던 잠복결함).
    reg = load_json(DEPTS, {"depts":{}}); e = reg["depts"].get(name, {})
    workdir = expand(e.get("cwd",""))
    # 1) 작업물 삭제는 의무 스냅샷 뒤에만 — 단 D1a 자격 게이트 통과(소유 선언) 경로에서만.
    #    부적격(공유 cwd·홈·보호루트·부재)=skip 후 진행(no-op — state·pack 격리는 계속).
    wd_eligible = False
    if purge_workdir:
        wd_eligible, wd_label = _workdir_quarantine_eligible(workdir, e)
        if not wd_eligible:
            actions.append((wd_label, workdir))
            sys.stderr.write("[destroy] %s: workdir 격리 skip(%s) — %s\n" % (name, wd_label, workdir))
        else:
            proceed, action = _snapshot_gate(name, workdir)
            actions.append(action)
            if not proceed:
                return actions  # abort_no_snapshot (workdir 존재+스냅샷 실패만)
    # 2) cys-dept down 위임 — purge_state면 --purge-state 전달(state 격리도 프리미티브에 위임·단일소유).
    #    CYS_TRASH_STAMP 공유로 프리미티브도 동일 trash 하위(<name>-<ts>/state)에 격리.
    # ★D1c(purge-safety 2026-07-16): PATH 의존 제거 — GUI(Finder 런칭) PATH엔 cys-dept가 없어
    #   FileNotFoundError로 죽던 잠복결함 + PATH 선두의 부서팩 forwarder가 base판을 가리던 드리프트 봉인.
    #   검증기(javis_purge_verify.py)와 동일하게 자기 디렉토리에서 해소. CYS_DEPT_BIN=테스트 주입용.
    cys_dept = os.environ.get("CYS_DEPT_BIN") or os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "cys-dept")
    if not os.path.exists(cys_dept):
        actions.append(("down", 127))
        sys.stderr.write("[destroy] %s: cys-dept 미발견(%s) — down 불가\n" % (name, cys_dept))
        return actions  # down 실패로 cmd_destroy가 비0 판정
    down_cmd = [cys_dept, "down", name] + (["--purge-state"] if purge_state else [])
    r = subprocess.run(down_cmd, capture_output=True, text=True,
                       env={**os.environ, "CYS_TRASH_STAMP": ts})
    actions.append(("down", r.returncode))
    # ★F1(reviewer1): down 실패(특히 --purge-state의 state 격리 실패=exit 3)를 삼키지 않는다 —
    #   사유를 stderr로 정직 보고하고 최종 exit는 cmd_destroy가 비0으로 판정한다. 부분 실패라도
    #   pack/workdir 격리는 best-effort로 진행(사용자 회수 표면 최대화).
    if r.returncode != 0:
        sys.stderr.write("[destroy] %s: cys-dept down 실패(rc=%d) — %s\n"
                         % (name, r.returncode, (r.stderr or "").strip()[:300]))
    # 3) pack-dept 격리(백업 없는 rmtree → mv 정합화) — forwarder는 self-reap(직접 회수 안 함)
    if purge:
        pack = f"{HOME}/.cys/pack-dept-{name}"
        a = _quarantine(pack, trash_dir, "pack")
        if a: actions.append(a)
    # 4) workdir 격리(down 전 포착 경로 사용) — D1a 자격 게이트 통과 시에만(스냅샷과 대칭)
    if purge_workdir and wd_eligible:
        a = _quarantine(workdir, trash_dir, "workdir")
        if a: actions.append(a)
    # ★F1(reviewer1): purge_state 성공 경로에 사후 결정론 검증기를 배선한다 — 라이브에서도 재등재0·
    #   묘비생존·형제무오염을 확인하고, 검증 실패도 비0로 전파(격리 미완/부활창을 "done"으로 오보하지
    #   않는다). down이 이미 실패면 검증 skip(중복 보고 회피 — cmd_destroy가 down 실패로 이미 비0).
    if purge_state and r.returncode == 0:
        verifier = os.path.join(os.path.dirname(os.path.abspath(__file__)), "javis_purge_verify.py")
        state_root = expand("~/.local/state")
        dept_roster = expand("~/.local/state/cys/phoenix/dept_roster.json")
        vr = subprocess.run(["python3", verifier, "--dept", name,
                             "--state-root", state_root, "--depts-json", DEPTS,
                             "--dept-roster", dept_roster], capture_output=True, text=True)
        actions.append(("verify", vr.returncode))
        if vr.returncode != 0:
            sys.stderr.write("[destroy] %s: 사후 검증 실패(rc=%d) — %s\n"
                             % (name, vr.returncode, (vr.stdout or vr.stderr or "").strip()[:400]))
    return actions

def cmd_destroy(args):
    require_cso()
    # ★D1d(purge-safety 2026-07-16): 예외가 raw traceback으로 GUI 토스트까지 새던 결함 봉인 —
    #   최상위에서 1줄 사유로 정직 보고하고 비0 반환(SystemExit=require_cso 등은 그대로 전파).
    try:
        reg = load_json(DEPTS, {"depts":{}})
        targets = list(reg.get("depts", {}).keys()) if args.all else ([args.dept] if args.dept else [])
        if not targets: sys.stderr.write("[destroy] 대상 없음(--dept 또는 --all)\n"); return 4
        allres = {}
        failed = False   # ★F1: down·검증 실패 누적 — 최종 exit 판정
        for name in targets:
            mk = reg["depts"].get(name, {}).get("mission_key")
            res = destroy_dept(name, mk, purge=args.purge, purge_workdir=args.purge_workdir,
                               purge_state=args.purge_state)
            allres[name] = res
            if any(a[0]=="abort_no_snapshot" for a in res):
                sys.stderr.write(f"[destroy] {name}: 스냅샷 실패 → 작업물 삭제 중단(fail-closed)\n"); return 1
            # ★F1(reviewer1): down·사후검증 실패는 최종 exit 비0 — 부분 성공을 "done"·0으로 오보 금지
            #   (state 잔존→재발견→부활 차단 붕괴를 조용히 통과시키던 결함 봉인).
            if any(a[0]=="down" and a[1]!=0 for a in res) or any(a[0]=="verify" and a[1]!=0 for a in res):
                failed = True
        print(json.dumps({"destroy": "done" if not failed else "incomplete", "targets": allres,
                          "note": "forwarder는 소켓 소멸 후 ~30s self-reap(직접 회수 안 함)"}, ensure_ascii=False))
        return 1 if failed else 0
    except Exception as ex:
        sys.stderr.write("[destroy] 실행 실패: %s: %s\n" % (type(ex).__name__, ex))
        return 1

def classify_dept(alive, intake):
    if not alive: return "redeploy"   # 소켓 死 → 멱등 apply 재실행(cys-dept REUSE_DEAD 재spawn)
    if not intake: return "hang"       # 소켓 alive·worker 미착수 → CSO 명시 개입 필요
    return "ok"

def correct_intake(name, e, m):
    """2단 생존창 후 분기 교정. redeploy=apply 재실행, hang=read-screen 진단+재각성 권고(CSO)."""
    time.sleep(8)  # 2단 생존창(뜨자마자 죽는 데몬 거짓양성 차단)
    st = dept_status(e["socket"])
    alive = st is not None
    intake = intake_ok((st or {}).get("surfaces", []))
    kind = classify_dept(alive, intake)
    if kind == "ok": return ("ok", name)
    if kind == "redeploy":
        rc, _ = create_dept(e.get("mission_key") or name)  # REUSE_DEAD 경로 재spawn
        return ("redeployed", name)
    # hang: 자동 down→create 금지(데이터 위험) — CSO 개입 지시 반환
    return ("hang_needs_cso", name)

def apply_plan(m):
    """부수효과 없는 실행계획. 순서: dept당 catalog→mission→ensure→create→backfill, 그 후 tasks."""
    plan = []
    for d in m["departments"]:
        plan.append(("catalog_upsert", d))
        plan.append(("write_mission", d))
        plan.append(("ensure_dirs", d))
        plan.append(("create_dept", d["key"]))
        plan.append(("backfill_mission_key", d))  # dept dict 보존(display까지 전달·R3-1)
    for t in m["tasks"]:
        plan.append(("dispatch_task", t))
    return plan

def create_dept(key):
    """cys-dept create 위임. 부서장 각성·미션주입·격리·멱등 전부 cys-dept 책임."""
    require_cso()  # 게이트를 효과 함수에 (R1 REVISE-1) — import 직접호출 우회 차단
    r = subprocess.run(["cys-dept", "create", key], capture_output=True, text=True,
                       env={**os.environ})  # 부모 role 상속(require_cso로 cso 보장·하류 가드 유지)
    return r.returncode, (r.stdout or "") + (r.stderr or "")

def dispatch_task(t):
    """첫 프로젝트 = 유일 실행 티켓(task-prompt). 미션 md의 첫프로젝트와 중복 금지(DUP-4)."""
    cmd = ["python3", f"{HOME}/.cys/pack/bin/javis_orchestra.py", "task-prompt",
           "--task", t["task"], "--scope", t["scope"], "--to", t.get("to","worker")]
    if t.get("success"): cmd += ["--success", t["success"]]
    if t.get("dont"): cmd += ["--dont", t["dont"]]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")

def apply_manifest(m):
    require_cso()  # 게이트를 효과 함수에 (R1 REVISE-1) — import 직접호출 우회 차단
    results = []
    for action, arg in apply_plan(m):
        if action == "catalog_upsert": catalog_upsert(CATALOG, arg); results.append((action, arg["key"], 0))
        elif action == "write_mission": write_mission(arg["key"], arg["mission_md"]); results.append((action, arg["key"], 0))
        elif action == "ensure_dirs": ensure_dirs(arg); results.append((action, arg["key"], 0))
        elif action == "create_dept":
            rc, out = create_dept(arg); results.append((action, arg, rc))
        elif action == "backfill_mission_key": backfill_mission_key(DEPTS, arg["key"], arg["key"], arg["display"]); results.append((action, arg["key"], 0))
        elif action == "dispatch_task":
            rc, out = dispatch_task(arg); results.append((action, arg["dept"], rc))
    return results

def cmd_apply(path):
    require_cso()
    try: m = load_json(path)
    except Exception as e: sys.stderr.write(f"[apply] 로드 실패: {e}\n"); return 2
    errs = validate_manifest(m)
    if errs:
        sys.stderr.write("[apply] validate FAIL — apply 중단:\n" + "\n".join(f"  - {e}" for e in errs) + "\n")
        return 1
    results = apply_manifest(m)
    fails = [(a,k) for a,k,rc in results if rc != 0]
    print(json.dumps({"apply": "done" if not fails else "partial",
                      "results": [[a,str(k),rc] for a,k,rc in results],
                      "fails": [[a,str(k)] for a,k in fails]}, ensure_ascii=False))
    return 0 if not fails else 1

def self_test():
    failures = []
    def chk(name, cond, msg=""):
        if not cond: failures.append(f"{name}: {msg}")
    # Task별로 케이스가 여기 누적된다.
    # --- Task2: v_schema ---
    good_dept = {"key":"future-research","display":"미래연구부","account":"cysinsight",
                 "cwd":"$HOME/Desktop/CYSjavis/미래연구부","mission_md":"# m","source_quote":"x"}
    m_ok = {"manifest_version":1,"kind":"org-manifest","reconcile_mode":"additive",
            "source":{"design_doc":"/d","design_doc_sha256":"a"},
            "departments":[good_dept],"tasks":[]}
    chk("schema-ok", v_schema(m_ok) == [], f"errs={v_schema(m_ok)}")
    chk("schema-bad-kind", any("kind" in e for e in v_schema({**m_ok,"kind":"x"})), "kind 위반 미검출")
    chk("schema-bad-to", any("to" in e for e in v_schema({**m_ok,
        "tasks":[{"dept":"future-research","to":"ceo","task":"t","scope":"s","source_quote":"q"}]})),
        "to enum 위반 미검출")
    chk("schema-miss-field", any("departments" in e for e in v_schema({k:v for k,v in m_ok.items() if k!="departments"})),
        "필수키 누락 미검출")
    # --- Task3: v_quote_binding (F1) ---
    doc = ("미래연구부는 모든 통찰의 원천 엔진이다. "
           "저술콘텐츠부는 통찰을 칼럼과 책으로 대중에 전파한다. "
           "이 문장은 부서 정체 토큰을 포함하지 않는 충분히 긴 고유 문장이다.")
    cat = {"accounts":{"cysinsight":"x","owner":"y"},
           "departments":{
             "future-research":{"display":"미래연구부","account":"cysinsight"},
             "authoring":{"display":"저술콘텐츠부","account":"owner"}}}
    d_ok = {"key":"future-research","display":"미래연구부","account":"cysinsight",
            "cwd":"$HOME/Desktop/CYSjavis/미래연구부",
            "source_quote":"미래연구부는 모든 통찰의 원천 엔진이다."}
    chk("f1-ok", v_quote_binding([d_ok], doc, cat) == [], f"errs={v_quote_binding([d_ok],doc,cat)}")
    # 오귀속: 실재(미래연구부) 문장을 엉뚱한 key/account(authoring/cysinsight)에 붙임 → FAIL
    d_mis = {**d_ok, "key":"authoring", "display":"미래연구부", "account":"cysinsight"}
    chk("f1-misattr", v_quote_binding([d_mis], doc, cat) != [], "오귀속 미검출")
    # key↔account 불일치 (authoring은 owner여야)
    d_acct = {"key":"authoring","display":"저술콘텐츠부","account":"cysinsight",
              "cwd":"$HOME/Desktop/CYSjavis/저술콘텐츠부",
              "source_quote":"저술콘텐츠부는 통찰을 칼럼과 책으로 대중에 전파한다."}
    chk("f1-account", any("account" in e for e in v_quote_binding([d_acct], doc, cat)), "계정 오배정 미검출")
    # quote가 부서정체 토큰 미포함
    d_noid = {**d_ok, "source_quote":"이 문장은 부서 정체 토큰을 포함하지 않는 충분히 긴 고유 문장이다."}
    chk("f1-noident", any("정체" in e for e in v_quote_binding([d_noid], doc, cat)), "정체 결속 미검출")
    # quote가 doc에 없음
    d_fab = {**d_ok, "source_quote":"존재하지 않는 긴 문장 어쩌고저쩌고 일이삼사오육칠팔."}
    chk("f1-absent", any("부재" in e for e in v_quote_binding([d_fab], doc, cat)), "부재 quote 미검출")
    # span 재사용: 두 부서가 같은 quote
    d_dup1 = {**d_ok}
    d_dup2 = {**d_ok, "key":"authoring", "display":"미래연구부"}
    chk("f1-span", any("재사용" in e for e in v_quote_binding([d_dup1,d_dup2], doc, cat)), "span 재사용 미검출")
    # 짧은 quote
    d_short = {**d_ok, "source_quote":"미래연구부"}
    chk("f1-short", any("길이" in e or "고유" in e for e in v_quote_binding([d_short], doc, cat)), "짧은 quote 미검출")
    # --- R1 BLOCK-2: 오귀속이 account 대조가 아니라 '결속/승인플래그'로 잡히는가 ---
    empty_cat = {"accounts":{"cysinsight":"x","owner":"y"},"departments":{}}
    # greenfield(empty catalog) fabricated 신규key + 실재 quote + display 위장 → 승인플래그 없으면 FAIL
    d_mis_empty = {**d_ok, "key":"shadow-ops", "display":"미래연구부", "account":"cysinsight",
                   "source_quote":"미래연구부는 모든 통찰의 원천 엔진이다."}
    chk("f1-misattr-empty", v_quote_binding([d_mis_empty], doc, empty_cat) != [], "empty-catalog 오귀속(신규key 승인없음) 미검출")
    # 승인 플래그 있으면 greenfield 신규 정상 통과 (account 유효 + 결속 OK)
    d_new_ok = {**d_mis_empty, "key":"future-research", "new_dept_approved":True}
    chk("f1-new-approved", v_quote_binding([d_new_ok], doc, empty_cat) == [], f"승인된 신규부서 오탐: {v_quote_binding([d_new_ok],doc,empty_cat)}")
    # 오귀속이 '결속(역인덱스)'으로 잡히는가: populated catalog, 실재quote↔허위key, account유효+승인있어도 FAIL
    d_keybind = {**d_ok, "key":"shadow-ops", "display":"미래연구부", "account":"cysinsight",
                 "new_dept_approved":True, "source_quote":"미래연구부는 모든 통찰의 원천 엔진이다."}
    chk("f1-keybind", any("오귀속" in e for e in v_quote_binding([d_keybind], doc, cat)), "결속(역인덱스)으로 오귀속 미검출")
    # --- Task4: v_refs / v_sha256 / validate_manifest ---
    chk("refs-ok", v_refs([good_dept],[{"dept":"future-research","to":"worker","task":"t","scope":"s","source_quote":"q"}])==[], "정상 참조 오류")
    chk("refs-bad", v_refs([good_dept],[{"dept":"no-such","to":"worker","task":"t","scope":"s","source_quote":"q"}])!=[], "붕뜬 task 미검출")
    chk("sha-ok", v_sha256("abc","abc")==[], "sha 일치 오탐")
    chk("sha-bad", v_sha256("abc","def")!=[], "sha 불일치 미검출")
    # --- Task5: catalog_upsert (격리 tmp) ---
    import tempfile as _tf
    td = _tf.mkdtemp()
    cpath = os.path.join(td, "catalog.json")
    json.dump({"version":1,"accounts":{"cysinsight":"x","owner":"y"},"departments":{}}, open(cpath,"w"))
    catalog_upsert(cpath, {"key":"authoring","display":"저술콘텐츠부","account":"owner",
                           "cwd":"$HOME/Desktop/CYSjavis/저술콘텐츠부"})
    catalog_upsert(cpath, {"key":"authoring","display":"저술콘텐츠부","account":"owner",
                           "cwd":"$HOME/Desktop/CYSjavis/저술콘텐츠부"})  # 멱등
    c2 = json.load(open(cpath))
    chk("cat-upsert", "authoring" in c2["departments"], "upsert 미반영")
    chk("cat-idem", len(c2["departments"])==1, "멱등 위반(중복)")
    chk("cat-mkey", c2["departments"]["authoring"]["mission_key"]=="authoring", "mission_key 미설정")
    # --- R1 BLOCK-1: backfill suffix 오탐 — basename 정확일치 (무관 레거시 메타 오염 차단) ---
    dpath = os.path.join(td, "depts.json")
    # cwd가 'future-research'로 끝나는 레거시(mission_key 없음) — key='research'로 backfill 시도
    json.dump({"depts":{"dept-1":{"cwd":"/x/future-research","socket":"s1"}}}, open(dpath,"w"))
    backfill_mission_key(dpath, "research", "research")  # endswith면 future-research에 오염
    r1 = json.load(open(dpath))
    chk("backfill-no-suffix-bleed", r1["depts"]["dept-1"].get("mission_key") != "research",
        "suffix 오탐: 무관 레거시(future-research)에 key='research' 오염")
    # 정확한 basename 일치는 backfill 됨
    json.dump({"depts":{"dept-2":{"cwd":"/x/research","socket":"s2"}}}, open(dpath,"w"))
    backfill_mission_key(dpath, "research", "research")
    r2 = json.load(open(dpath))
    chk("backfill-exact-match", r2["depts"]["dept-2"].get("mission_key") == "research",
        "정확 basename 일치인데 backfill 안 됨")
    # --- R3-1: 한글-display 레거시 cwd backfill 소급 성공 (라이브 규약 커버 — over-correction 해소) ---
    # 라이브 cwd 전건이 한글 display로 끝남(.../미래연구부) → 영문 key(future-research)와 영영 불일치였음
    json.dump({"depts":{"d3":{"cwd":"$HOME/Desktop/CYSjavis/미래연구부","socket":"s3"}}}, open(dpath,"w"))
    backfill_mission_key(dpath, "future-research", "future-research", "미래연구부")
    r3 = json.load(open(dpath))
    chk("backfill-hangul-display", r3["depts"]["d3"].get("mission_key") == "future-research",
        "한글 display 레거시 cwd backfill 소급 실패(over-correction)")
    # 한글 display여도 부분문자열 오탐은 여전 차단(정확일치라 suffix-bleed 회귀 없음)
    json.dump({"depts":{"d4":{"cwd":"$HOME/Desktop/CYSjavis/구미래연구부","socket":"s4"}}}, open(dpath,"w"))
    backfill_mission_key(dpath, "future-research", "future-research", "미래연구부")
    r4 = json.load(open(dpath))
    chk("backfill-hangul-no-bleed", r4["depts"]["d4"].get("mission_key") != "future-research",
        "한글 display 부분문자열(구미래연구부) 오탐")
    # --- R3-2: catalog display/cwd drift 거부 (기존 key 재할당 위장 차단) ---
    cat_drift = {"accounts":{"cysinsight":"x"},
                 "departments":{"future-research":{"display":"미래연구부","account":"cysinsight","cwd":"$HOME/Desktop/CYSjavis/미래연구부"}}}
    d_drift = [{"key":"future-research","display":"위장된딴부서","account":"cysinsight","cwd":"$HOME/Desktop/CYSjavis/미래연구부"}]
    chk("catalog-drift-display", any("display" in e for e in v_catalog_consistency(d_drift, cat_drift)), "display drift 미검출")
    d_cwd_drift = [{"key":"future-research","display":"미래연구부","account":"cysinsight","cwd":"$HOME/Desktop/CYSjavis/딴경로"}]
    chk("catalog-drift-cwd", any("cwd" in e for e in v_catalog_consistency(d_cwd_drift, cat_drift)), "cwd drift 미검출")
    d_consistent = [{"key":"future-research","display":"미래연구부","account":"cysinsight","cwd":"$HOME/Desktop/CYSjavis/미래연구부"}]
    chk("catalog-consistent", v_catalog_consistency(d_consistent, cat_drift) == [], f"정합인데 오탐: {v_catalog_consistency(d_consistent, cat_drift)}")
    chk("catalog-newkey-skip", v_catalog_consistency([{"key":"brand-new","display":"신규","cwd":"x"}], cat_drift) == [], "신규key를 drift로 오탐")
    # --- Task6: apply 분해(부수효과 없는 plan 생성) ---
    plan = apply_plan(m_ok)  # [(action, key/args), ...]
    chk("apply-order", plan[0][0]=="catalog_upsert" and "create_dept" in [p[0] for p in plan], "apply 순서/구성 오류")
    chk("apply-create-after-cat",
        [p[0] for p in plan].index("catalog_upsert") < [p[0] for p in plan].index("create_dept"),
        "catalog upsert가 create보다 먼저가 아님(cross-file 순서)")
    # --- Task7: intake_ok (부서장 자기 working 오집계 금지) ---
    only_master = [{"role":"master","agent":"claude","agent_alive":True,"idle_secs":3,"line_count":50,"status":"working"}]
    chk("intake-master-only", intake_ok(only_master)==False, "부서장만 있는데 착수 PASS(오집계)")
    with_worker = only_master + [{"role":"worker","agent":"claude","agent_alive":True,"idle_secs":2,"line_count":10,"status":"working"}]
    chk("intake-worker", intake_ok(with_worker)==True, "워커 착수인데 FAIL")
    dead_worker = only_master + [{"role":"worker","agent":"claude","agent_alive":False,"idle_secs":9999,"line_count":0,"status":None}]
    chk("intake-dead", intake_ok(dead_worker)==False, "죽은 워커를 착수로 오판")
    # --- Task8: classify_dept (데몬死 vs hang) ---
    chk("cls-dead", classify_dept(alive=False, intake=False)=="redeploy", "데몬死 분류 오류")
    chk("cls-hang", classify_dept(alive=True, intake=False)=="hang", "hang 분류 오류")
    chk("cls-ok", classify_dept(alive=True, intake=True)=="ok", "정상 분류 오류")
    # --- Task9: tar_snapshot fail-closed ---
    src = os.path.join(td, "workdir"); os.makedirs(src, exist_ok=True)
    open(os.path.join(src,"a.txt"),"w").write("data")
    snap = tar_snapshot("authoring", src, dest_dir=td)
    chk("snap-made", snap and os.path.exists(snap), "스냅샷 미생성")
    chk("snap-missing-src", tar_snapshot("x", os.path.join(td,"nope"), dest_dir=td) is None, "없는 소스에 스냅샷 성공(위험)")
    # --- R1 REVISE-2: workdir 부재=skip(no abort 영구락인), 존재=의무스냅샷 ---
    proceed_abs, act_abs = _snapshot_gate("x", os.path.join(td,"nope-wd"))
    chk("snapgate-absent-skip", proceed_abs and act_abs[0]=="workdir_absent_skip", "workdir 부재인데 abort(영구 락인)")
    proceed_pre, act_pre = _snapshot_gate("authoring", src)  # src는 위에서 생성된 실재 workdir
    chk("snapgate-present-snap", proceed_pre and act_pre[0]=="snapshot", "workdir 존재인데 스냅샷 안 됨")
    # --- D1a(purge-safety): workdir 격리 자격 게이트 truth table ---
    chk("wdgate-absent", _workdir_quarantine_eligible(os.path.join(td,"nope-wd2"), {}) == (False,"workdir_absent_skip"),
        "부재 workdir 판정 오류")
    chk("wdgate-home", _workdir_quarantine_eligible(HOME, {"workdir_owned": True}) == (False,"workdir_protected_skip"),
        "홈이 보호루트로 차단 안 됨(소유 선언으로도 우회 불가여야 — 실사고 재발 경로)")
    chk("wdgate-root", _workdir_quarantine_eligible("/", {"workdir_owned": True}) == (False,"workdir_protected_skip"),
        "루트가 차단 안 됨")
    chk("wdgate-shared", _workdir_quarantine_eligible(src, {}) == (False,"workdir_shared_skip"),
        "소유 미선언(공유) workdir이 skip 안 됨")
    chk("wdgate-outside", _workdir_quarantine_eligible(src, {"workdir_owned": True}) == (False,"workdir_outside_home_skip"),
        "홈 밖(tmp) workdir이 소유 선언만으로 통과(경계 붕괴)")
    _owned = os.path.join(HOME, ".cys", "selftest-wd-%d" % os.getpid())
    os.makedirs(_owned, exist_ok=True)
    try:
        chk("wdgate-owned", _workdir_quarantine_eligible(_owned, {"workdir_owned": True}) == (True,"workdir_owned"),
            "적격(홈 내부+소유 선언) workdir이 거부됨")
        _link = os.path.join(_owned, "esc-link")
        try:
            os.symlink(td, _link)
            chk("wdgate-symlink-escape", _workdir_quarantine_eligible(_link, {"workdir_owned": True})[0] is False,
                "심링크로 홈 밖 탈출이 통과(realpath 우회)")
        finally:
            if os.path.islink(_link): os.remove(_link)
    finally:
        os.rmdir(_owned)
    # --- D1b(purge-safety): tar_snapshot 예외 fail-closed(부분 tar 정리·None 반환) ---
    _real_ld = os.listdir
    def _deny_ld(p="."):
        if str(p).endswith("deny-dir"): raise PermissionError(1, "Operation not permitted", str(p))
        return _real_ld(p)
    src_deny = os.path.join(td, "wd-deny"); os.makedirs(os.path.join(src_deny, "deny-dir"), exist_ok=True)
    os.listdir = _deny_ld
    try:
        snapped = tar_snapshot("denytest", src_deny, dest_dir=td)
    finally:
        os.listdir = _real_ld
    chk("snap-exc-none", snapped is None, "읽기불가 예외인데 None 아님(traceback 전파 결함 재발)")
    chk("snap-exc-clean", not [f for f in _real_ld(td) if f.startswith("denytest-")], "실패 부분 tar 잔존")
    print(json.dumps({"self_test": "ok" if not failures else "fail",
                      "failures": failures}, ensure_ascii=False))
    return 1 if failures else 0

def main():
    ap = argparse.ArgumentParser(description="부서 자동 편성 브리지")
    ap.add_argument("--self-test", action="store_true", help="결정론 자기검증")
    sub = ap.add_subparsers(dest="cmd")
    v = sub.add_parser("validate", help="org-manifest 검증 (0=준수 1=위반 2=입출력)")
    v.add_argument("manifest")
    a = sub.add_parser("apply", help="매니페스트 적용 (CSO 전용)")
    a.add_argument("manifest")
    s = sub.add_parser("status", help="부서 착수확인 집계")
    s.add_argument("manifest", nargs="?")
    sub.add_parser("org-audit",
                   help="편제 read-only 파생 집계(0=수렴 1=미수렴 2=입출력 · 영속 0 — T10/P3-3)")
    d = sub.add_parser("destroy", help="부서 삭제 (CSO 전용)")
    d.add_argument("--dept"); d.add_argument("--all", action="store_true")
    d.add_argument("--purge", action="store_true")
    d.add_argument("--purge-workdir", action="store_true")
    d.add_argument("--purge-state", action="store_true",
                   help="★기능2: 부서 state 디렉토리(대화기억)까지 격리(cys-dept down --purge-state 위임)")
    args = ap.parse_args()
    if args.self_test: return self_test()
    if not args.cmd: ap.print_help(); return 2
    if args.cmd == "validate": return cmd_validate(args.manifest)
    if args.cmd == "apply": return cmd_apply(args.manifest)
    if args.cmd == "status": return cmd_status(args.manifest)
    if args.cmd == "org-audit": return cmd_org_audit()
    if args.cmd == "destroy": return cmd_destroy(args)
    return 2

if __name__ == "__main__":
    sys.exit(main())
