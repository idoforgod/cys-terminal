#!/usr/bin/env python3
"""javis_hook_inventory.py — 교차-scope 훅 인벤토리(CU-5B · **read-only 산출물 전용**).

무엇을 세는가
  claude 계정마다 `settings.json`의 hooks 명령이 **어느 팩(scope)의 스크립트**를 가리키는지 뽑아,
  그 계정이 **소속된 scope**와 다른 항목(=교차-scope 훅)을 표로 낸다. 부서 계정이 base 팩 훅을
  물고 있거나(그 반대) 폐기된 부서 팩을 가리키는 잔해가 이 표에 걸린다.

무엇을 하지 않는가 (★계약 · R1 오너 결정 자료)
  **수정·삭제를 하지 않는다.** 이 파일에 쓰기(open(...,'w')·remove·rename·chmod) 코드는 0이다.
  잔해 정리 집행은 오너 결재 게이트 뒤의 별개 작업이고, 이 도구의 산출물이 그 입력이다.

scope 규칙(정본 정합)
  scope_id = 팩 디렉터리의 basename(`~/.cys/pack`→`pack`, `~/.cys/pack-dept-dept-2`→`pack-dept-dept-2`).
  Rust `cys::pack::scope_id_of`(src/pack.rs:149)·Python `javis_orchestra._pack_identity`와 같은 규칙 —
  하드코딩 금지, 팩 이름 자체가 정체성이다.

계정 → 소속 scope
  ① `depts.json`의 `account_dir` → 그 부서의 `pack_dir` scope (부서 계정 · **권위**)
  ② 그 외(`~/.claude*`·`~/.cys/claude*`) → base 팩 scope(기본 `pack`)
  ★②의 함정(실측): allocate-born 부서 계정은 `account_dir` 미기록(CU-5A가 고치는 결손)이라 ②로 떨어져
    **base 계정으로 오분류**된다 — 그 상태로 "교차"라 부르면 정상 배선을 잔해로 오인해 청소를 유도한다.
    그래서 계정명이 `…-<dept-N>`이고 그 부서가 레지스트리에 실재하면 `scope_hint`로 기록하고, 훅이
    그 hint를 가리키면 `cross-unverified`(교차 확정 아님·미등재 신호)로 **격리 분류**한다. 단정하지 않는다.

사용
  python3 javis_hook_inventory.py            # 마크다운 표(stdout)
  python3 javis_hook_inventory.py --json     # 기계 판독용 JSON(stdout)
  python3 javis_hook_inventory.py --all      # 일치 항목까지 전부(기본은 불일치·보류만)
  CYS_HOOK_INV_HOME=<dir> …                  # HOME 오버라이드(테스트 · HOME 자체도 존중)
  CYS_DEPTS_JSON=<file> …                    # 레지스트리 오버라이드(cys-dept와 동일 env)

exit code는 항상 0(관측이 상태를 바꾸지 않듯, 관측이 게이트를 대신하지도 않는다 — 판정은 표가 한다).
"""
import argparse
import json
import os
import re
import sys
import time

# 훅 명령에서 팩 경로를 뽑는 앵커: `…/.cys/<pack…>`. 경로 경계(/ 또는 인용·공백·구분자)까지만 먹는다.
# `pack-dept-*`·`pack-ceo`·`pack.prev` 등 pack 계열 전부 매치하고, `/pack/`이 아닌 우연한 단어는 배제.
PACK_RE = re.compile(r'(?:[A-Za-z]:)?[^\s"\'<>|&;]*[/\\]\.cys[/\\](?P<pack>pack[^/\\\s"\'<>|&;]*)')
# 런타임에 결정되는 팩 참조 — 정적으로는 어느 scope인지 알 수 없다(단정 금지·보류로 분류).
DYN_RE = re.compile(r'\$\{?CYS_PACK_DIR\b')


def home_dir():
    """HOME 오버라이드 지원 — 테스트가 라이브 홈을 만지지 않게 하는 유일한 손잡이."""
    return os.environ.get("CYS_HOOK_INV_HOME") or os.path.expanduser("~")


def scope_id_of(path):
    """팩 경로 → scope 식별자(basename). 정본: src/pack.rs:149 `scope_id_of`."""
    if not path:
        return ""
    return os.path.basename(os.path.abspath(os.path.normpath(path)))


def _real(p):
    try:
        return os.path.realpath(p)
    except OSError:
        return os.path.normpath(p)


def _short(p, home):
    return "~" + p[len(home):] if home and p.startswith(home + os.sep) else p


def _mtime(p):
    try:
        return time.strftime("%Y-%m-%d %H:%M", time.localtime(os.stat(p).st_mtime))
    except OSError:
        return "?"


def dept_accounts(home, warnings):
    """depts.json → ({realpath(account_dir): (scope, dept)}, {dept명: scope}). 손상·부재는 경고 후 빈 맵."""
    reg = os.environ.get("CYS_DEPTS_JSON") or os.path.join(home, ".cys", "depts.json")
    out, by_name = {}, {}
    if not os.path.isfile(reg):
        return out, by_name
    try:
        with open(reg, encoding="utf-8") as f:
            depts = (json.load(f) or {}).get("depts", {})
    except Exception as e:                                   # 손상 레지스트리 = 부분 커버리지로 강등
        warnings.append("depts.json 파싱 실패(%s) — 부서 계정 매핑 없이 진행: %s" % (reg, e))
        return out, by_name
    if not isinstance(depts, dict):
        warnings.append("depts.json depts 형이 dict가 아님 — 부서 계정 매핑 생략")
        return out, by_name
    for name, e in depts.items():
        if not isinstance(e, dict):
            continue
        acct = e.get("account_dir")
        pack = e.get("pack_dir") or os.path.join(home, ".cys", "pack-dept-%s" % name)
        by_name[name] = scope_id_of(pack)
        if acct:
            out[_real(acct)] = (scope_id_of(pack), name)
    return out, by_name


NAME_HINT_RE = re.compile(r'-(dept-[A-Za-z0-9_]+)$')


def discover_accounts(home, dept_map, dept_by_name, warnings):
    """스캔 대상 계정 목록 → [{dir, settings, scope, scope_source, scope_hint, origin}].
    존재하는 디렉터리만·사전순·중복 제거(realpath)."""
    cands = []
    try:
        for n in sorted(os.listdir(home)):
            if n == ".claude" or n.startswith(".claude-"):
                cands.append(os.path.join(home, n))
    except OSError as e:
        warnings.append("HOME 나열 실패(%s): %s" % (home, e))
    cysdir = os.path.join(home, ".cys")
    try:
        for n in sorted(os.listdir(cysdir)):
            if n == "claude" or n.startswith("claude-"):
                cands.append(os.path.join(cysdir, n))
    except OSError:
        pass                                                 # ~/.cys 부재 = 신규 기계(정상)
    cands.extend(sorted(dept_map))                           # 레지스트리 등재 계정(홈 밖일 수 있음)

    base_scope = scope_id_of(os.path.join(home, ".cys", "pack"))
    accounts, seen = [], set()
    for d in cands:
        if not os.path.isdir(d):
            continue
        rp = _real(d)
        if rp in seen:
            continue
        seen.add(rp)
        scope, origin, source, hint = base_scope, "base", "base-default", ""
        if rp in dept_map:
            scope, dept = dept_map[rp]
            origin, source = "dept:%s" % dept, "registry"
        else:
            # 레지스트리 미등재 부서 계정 추정(CU-5A 결손 신호) — 계정명 접미 `-dept-N`이 **실재 부서**를
            # 가리킬 때만. 이 값으로 판정을 뒤집지 않고 `cross-unverified` 격리에만 쓴다.
            m = NAME_HINT_RE.search(os.path.basename(os.path.normpath(d)))
            if m and m.group(1) in dept_by_name:
                hint = dept_by_name[m.group(1)]
                origin = "base?(추정 %s)" % m.group(1)
        accounts.append({"dir": d, "settings": os.path.join(d, "settings.json"),
                         "scope": scope, "scope_source": source, "scope_hint": hint,
                         "origin": origin})
    return accounts


def iter_hooks(settings_path, warnings):
    """settings.json → (event, matcher, command) 산출. 파손·이형 구조는 skip+경고(crash 금지)."""
    try:
        with open(settings_path, encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        return
    except Exception as e:
        warnings.append("settings 파싱 실패 — skip: %s (%s)" % (settings_path, e))
        return
    hroot = data.get("hooks") if isinstance(data, dict) else None
    if not isinstance(hroot, dict):
        if hroot is not None:
            warnings.append("hooks 형이 dict가 아님 — skip: %s" % settings_path)
        return
    for ev, blocks in sorted(hroot.items()):
        if not isinstance(blocks, list):
            warnings.append("hooks.%s 형이 list가 아님 — skip: %s" % (ev, settings_path))
            continue
        for blk in blocks:
            if not isinstance(blk, dict):
                continue
            matcher = blk.get("matcher", "") if isinstance(blk.get("matcher", ""), str) else ""
            for h in blk.get("hooks", []) or []:
                if not isinstance(h, dict):
                    continue
                cmd = h.get("command")
                if isinstance(cmd, str) and cmd.strip():
                    yield ev, matcher, cmd


def classify(cmd, own_scope, scope_hint=""):
    """훅 명령 1건 → (verdict, target_scope, target_pack_path).

    verdict: match(자기 scope) | cross(교차 — 남의 팩·**확정**)
             | cross-unverified(소속이 레지스트리 미등재라 추정 scope와 일치 — 교차로 단정 못 함)
             | dynamic(런타임 결정·판정 보류) | none(팩 경로 없음 — 팩 밖 스크립트·인라인 명령)
    ★단정 금지: `$CYS_PACK_DIR` 참조는 실행 시점 env가 정하므로 교차로 단정하지 않는다(보류).
    """
    if DYN_RE.search(cmd):
        return "dynamic", "", ""
    m = PACK_RE.search(cmd)
    if not m:
        return "none", "", ""
    pack = m.group("pack")
    if pack == own_scope:
        return "match", pack, m.group(0)
    if scope_hint and pack == scope_hint:
        return "cross-unverified", pack, m.group(0)
    return "cross", pack, m.group(0)


def scan(home):
    warnings = []
    dept_map, dept_by_name = dept_accounts(home, warnings)
    accounts = discover_accounts(home, dept_map, dept_by_name, warnings)
    for a in accounts:
        if a["scope_hint"]:
            warnings.append("계정 %s 는 레지스트리 미등재 — 소속을 base로 가정하고 추정 scope(%s)와의 "
                            "일치는 cross-unverified로 격리(depts.json account_dir 미기록 신호)"
                            % (_short(a["dir"], home), a["scope_hint"]))
    rows = []
    for a in accounts:
        if not os.path.isfile(a["settings"]):
            continue
        mt = _mtime(a["settings"])
        for ev, matcher, cmd in iter_hooks(a["settings"], warnings):
            verdict, tscope, tpath = classify(cmd, a["scope"], a["scope_hint"])
            rows.append({
                "account": _short(a["dir"], home), "origin": a["origin"],
                "own_scope": a["scope"], "event": ev, "matcher": matcher,
                "command": cmd, "verdict": verdict, "target_scope": tscope,
                "target_pack": _short(tpath, home) if tpath else "",
                "target_exists": bool(tpath) and os.path.isdir(tpath),
                "settings": _short(a["settings"], home), "settings_mtime": mt,
            })
    counts = {}
    for r in rows:
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
    return {
        "generated_at": time.strftime("%Y-%m-%d %H:%M:%S"),
        "home": home,
        "base_scope": scope_id_of(os.path.join(home, ".cys", "pack")),
        "accounts": [{"dir": _short(a["dir"], home), "scope": a["scope"], "origin": a["origin"],
                      "scope_source": a["scope_source"], "scope_hint": a["scope_hint"],
                      "settings_present": os.path.isfile(a["settings"])} for a in accounts],
        "rows": rows,
        "warnings": warnings,
        "summary": {"accounts": len(accounts), "hooks": len(rows),
                    "cross": counts.get("cross", 0),
                    "cross_unverified": counts.get("cross-unverified", 0),
                    "dynamic": counts.get("dynamic", 0),
                    "match": counts.get("match", 0), "none": counts.get("none", 0)},
    }


def _table(rows, cols, headers):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(["---"] * len(headers)) + "|"]
    for r in rows:
        cells = []
        for c in cols:
            v = r[c]
            v = "✓" if v is True else ("✗" if v is False else str(v))
            cells.append(v.replace("|", "\\|"))
        out.append("| " + " | ".join(cells) + " |")
    return out


def render(res, show_all):
    s = res["summary"]
    L = ["# 교차-scope 훅 인벤토리 (read-only · 수정·삭제 없음)", "",
         "- 생성: %s · HOME=`%s` · base scope=`%s`" % (res["generated_at"], res["home"], res["base_scope"]),
         "- 계정 %d · 훅 %d — **교차(확정) %d** · 교차?(미검증) %d · 보류(동적) %d · 일치 %d · 팩밖 %d"
         % (s["accounts"], s["hooks"], s["cross"], s["cross_unverified"], s["dynamic"],
            s["match"], s["none"]), ""]
    L += ["## 스캔한 계정", ""]
    L += _table(res["accounts"], ["dir", "scope", "scope_source", "scope_hint", "origin",
                                  "settings_present"],
                ["계정", "소속 scope", "소속 근거", "추정 scope", "출처", "settings"])
    cross = [r for r in res["rows"] if r["verdict"] == "cross"]
    L += ["", "## 교차-scope 훅 — 확정 (계정의 소속 scope ≠ 훅이 가리키는 팩)", ""]
    if cross:
        L += _table(cross, ["account", "event", "own_scope", "target_scope", "target_exists",
                            "command", "settings_mtime"],
                    ["계정", "이벤트", "소속 scope", "지시 scope", "팩 실재", "명령", "settings mtime"])
    else:
        L += ["(없음)"]
    unv = [r for r in res["rows"] if r["verdict"] == "cross-unverified"]
    L += ["", "## 교차? — 미검증 (계정이 레지스트리 미등재 · 추정 소속과 일치 → **청소 대상 아님**)", ""]
    L += _table(unv, ["account", "event", "own_scope", "target_scope", "command", "settings_mtime"],
                ["계정", "이벤트", "가정 scope", "지시 scope", "명령", "settings mtime"]) if unv else ["(없음)"]
    dyn = [r for r in res["rows"] if r["verdict"] == "dynamic"]
    L += ["", "## 판정 보류 — `$CYS_PACK_DIR` 런타임 결정(정적 단정 금지)", ""]
    L += _table(dyn, ["account", "event", "own_scope", "command", "settings_mtime"],
                ["계정", "이벤트", "소속 scope", "명령", "settings mtime"]) if dyn else ["(없음)"]
    if show_all:
        rest = [r for r in res["rows"] if r["verdict"] in ("match", "none")]
        L += ["", "## 그 외(일치·팩밖) — `--all`", ""]
        L += _table(rest, ["account", "event", "own_scope", "verdict", "command"],
                    ["계정", "이벤트", "소속 scope", "판정", "명령"]) if rest else ["(없음)"]
    if res["warnings"]:
        L += ["", "## 경고(파손·이형 — skip 처리)", ""] + ["- %s" % w for w in res["warnings"]]
    return "\n".join(L) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description="교차-scope 훅 인벤토리(read-only)")
    ap.add_argument("--json", action="store_true", help="JSON 산출(마크다운 대신)")
    ap.add_argument("--all", action="store_true", help="일치·팩밖 항목까지 표에 포함")
    ap.add_argument("--home", default=None, help="스캔 HOME 지정(기본: CYS_HOOK_INV_HOME 또는 $HOME)")
    args = ap.parse_args(argv)
    res = scan(args.home or home_dir())
    sys.stdout.write(json.dumps(res, ensure_ascii=False, indent=2) + "\n"
                     if args.json else render(res, args.all))
    return 0


if __name__ == "__main__":
    sys.exit(main())
