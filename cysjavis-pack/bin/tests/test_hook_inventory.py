#!/usr/bin/env python3
"""test_hook_inventory.py — CU-5B 교차-scope 훅 인벤토리 단위(픽스처 settings 3종).

설계 정본 DESIGN_scope-first-class.md §4 CU-5B: read-only 스캐너 · 픽스처 정상/교차/파손 ·
파손은 skip+경고(crash 금지) · 산출은 마크다운 표 + `--json`.

전부 mktemp 샌드박스 HOME(라이브 `~/.claude*`·`~/.cys/*` 무접촉 — CYS_HOOK_INV_HOME·CYS_DEPTS_JSON
오버라이드로만 스캔한다). 스캐너 자체가 read-only라 샌드박스도 읽기만 당한다.

픽스처 계정 5종:
  ~/.claude                        base 계정 · 정상(base 팩 훅) + 동적($CYS_PACK_DIR) 1건
  ~/.cys/claude-cat-dept-1         부서 계정(레지스트리 등재) · **교차**(base 팩 훅) + 자기 팩 훅
  ~/.cys/claude-cat-dept-2         부서 계정 · 파손 JSON(skip+경고)
  ~/.claude-legacy                 base 계정 · 폐기된 부서 팩(pack-dept-dept-9 미실재) 지시 = 교차+미실재
  ~/.cys/claude-default-dept-1     **레지스트리 미등재** 부서 계정(allocate-born · CU-5A 결손 재현) —
                                   자기 부서 팩 훅이 base 오분류 탓에 교차로 보이는 함정 → cross-unverified 격리
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(SELF, "..", "javis_hook_inventory.py")
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def w(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def hooks(*items):
    """(event, command) 목록 → claude settings.json hooks 구조."""
    h = {}
    for ev, cmd in items:
        h.setdefault(ev, []).append({"matcher": "Write|Edit", "hooks": [{"type": "command", "command": cmd}]})
    return json.dumps({"hooks": h}, ensure_ascii=False, indent=2)


def build():
    tmp = tempfile.mkdtemp(prefix="hookinv-")
    home = os.path.join(tmp, "home")
    cys = os.path.join(home, ".cys")
    for d in ("pack", "pack-dept-dept-1", "claude-cat-dept-1", "claude-cat-dept-2"):
        os.makedirs(os.path.join(cys, d), exist_ok=True)
    os.makedirs(os.path.join(home, ".claude"), exist_ok=True)
    os.makedirs(os.path.join(home, ".claude-legacy"), exist_ok=True)
    reg = os.path.join(cys, "depts.json")
    w(reg, json.dumps({"depts": {
        "dept-1": {"socket": "s1", "pack_dir": os.path.join(cys, "pack-dept-dept-1"),
                   "role": "dept-master", "account_dir": os.path.join(cys, "claude-cat-dept-1")},
        "dept-2": {"socket": "s2", "pack_dir": os.path.join(cys, "pack-dept-dept-2"),
                   "role": "dept-master", "account_dir": os.path.join(cys, "claude-cat-dept-2")},
    }}))
    # ① 정상 base 계정: base 팩 훅 + 동적 참조
    w(os.path.join(home, ".claude", "settings.json"),
      hooks(("PostToolUse", "bash %s/pack/hooks/pack-guard.sh" % cys),
            ("SessionStart", 'python3 "${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_session_start.py"')))
    # ② 부서 계정: 자기 팩 훅(정상) + base 팩 훅(교차 — 이 도구가 찾아야 할 것)
    w(os.path.join(cys, "claude-cat-dept-1", "settings.json"),
      hooks(("PostToolUse", "bash %s/pack-dept-dept-1/hooks/pack-guard.sh" % cys),
            ("SessionStart", "python3 %s/pack/bin/javis_session_start.py" % cys)))
    # ③ 파손 JSON(skip+경고·crash 금지)
    w(os.path.join(cys, "claude-cat-dept-2", "settings.json"), '{"hooks": {broken,,,')
    # ④ 폐기 부서 팩 지시(교차 + 팩 미실재)
    w(os.path.join(home, ".claude-legacy", "settings.json"),
      hooks(("PostToolUse", "bash %s/pack-dept-dept-9/hooks/pack-guard.sh" % cys)))
    # ⑤ 레지스트리 미등재 부서 계정(allocate-born 잔재) — 자기 부서 팩 훅 + 진짜 교차(base) 1건
    os.makedirs(os.path.join(cys, "claude-default-dept-1"), exist_ok=True)
    w(os.path.join(cys, "claude-default-dept-1", "settings.json"),
      hooks(("PostToolUse", "bash %s/pack-dept-dept-1/hooks/pack-guard.sh" % cys),
            ("Stop", "python3 %s/pack-dept-dept-9/bin/x.py" % cys)))
    return tmp, home, reg


def run(home, reg, *args):
    env = dict(os.environ)
    env.update({"CYS_HOOK_INV_HOME": home, "CYS_DEPTS_JSON": reg})
    for k in ("CYS_PACK_DIR", "CYS_ACCOUNT_DIR"):
        env.pop(k, None)
    r = subprocess.run([sys.executable, TOOL] + list(args), capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=60)
    return r.returncode, r.stdout, r.stderr


tmp, home, reg = build()
before = sorted(os.walk(home).__next__()[1])

# ── JSON 산출 ──
code, out, err = run(home, reg, "--json")
check("H1 exit 0(read-only 도구)", code == 0, "rc=%d err=%s" % (code, err.strip()[:160]))
try:
    res = json.loads(out)
except Exception as e:
    res = None
    check("H2 --json 파싱", False, str(e))
if res:
    check("H2 --json 파싱", True)
    cross = [r for r in res["rows"] if r["verdict"] == "cross"]
    xs = {(r["account"], r["own_scope"], r["target_scope"]) for r in cross}
    check("H3 부서 계정의 base 팩 훅 = 교차 탐지",
          ("~/.cys/claude-cat-dept-1", "pack-dept-dept-1", "pack") in xs, str(xs))
    check("H4 폐기 부서 팩 지시 = 교차 + 팩 미실재",
          any(r["target_scope"] == "pack-dept-dept-9" and r["target_exists"] is False for r in cross),
          str(xs))
    check("H5 교차(확정)는 정확히 3건(일치·미검증 오탐 0)", len(cross) == 3,
          "cross=%d rows=%d verdicts=%s" % (len(cross), len(res["rows"]),
                                            sorted({r["verdict"] for r in res["rows"]})))
    # ★H5b: 미등재 부서 계정의 **자기 부서 팩** 훅은 cross 확정에서 격리(청소 오유도 차단),
    #   같은 계정의 진짜 남의 팩(dept-9) 훅은 확정 cross로 남는다.
    unv = [r for r in res["rows"] if r["verdict"] == "cross-unverified"]
    check("H5b 미등재 부서 계정의 자기 팩 훅 = cross-unverified 격리",
          len(unv) == 1 and unv[0]["account"] == "~/.cys/claude-default-dept-1"
          and unv[0]["target_scope"] == "pack-dept-dept-1", str(unv))
    check("H5c 같은 계정의 남의 팩(dept-9) 훅은 확정 cross 유지",
          any(r["account"] == "~/.cys/claude-default-dept-1"
              and r["target_scope"] == "pack-dept-dept-9" for r in cross))
    check("H5d 미등재 신호 경고 노출",
          any("레지스트리 미등재" in wm for wm in res["warnings"]), str(res["warnings"])[:200])
    check("H6 자기 팩 훅은 match(오탐 없음)",
          any(r["verdict"] == "match" and r["own_scope"] == "pack-dept-dept-1" for r in res["rows"]))
    check("H7 base 계정의 base 팩 훅은 match", any(
        r["verdict"] == "match" and r["account"] == "~/.claude" for r in res["rows"]))
    check("H8 $CYS_PACK_DIR 참조는 dynamic(단정 금지)",
          any(r["verdict"] == "dynamic" and r["account"] == "~/.claude" for r in res["rows"])
          and res["summary"]["dynamic"] == 1, json.dumps(res["summary"], ensure_ascii=False))
    check("H9 파손 settings = skip + 경고(crash 0)",
          any("claude-cat-dept-2" in wmsg and "파싱 실패" in wmsg for wmsg in res["warnings"]),
          str(res["warnings"]))
    check("H10 계정 소속 scope 매핑(depts.json account_dir → 부서 팩 scope)",
          {a["dir"]: a["scope"] for a in res["accounts"]}.get("~/.cys/claude-cat-dept-1")
          == "pack-dept-dept-1")
    check("H11 base 계정 소속 scope = base 팩 basename",
          {a["dir"]: a["scope"] for a in res["accounts"]}.get("~/.claude") == "pack")
    check("H12 mtime 컬럼 존재(증거 열)",
          all(r["settings_mtime"] and r["settings_mtime"] != "?" for r in res["rows"]))

# ── 마크다운 산출 ──
code, md, err = run(home, reg)
check("H13 마크다운 exit 0", code == 0, err.strip()[:120])
check("H14 마크다운 표 헤더·교차 절 존재",
      "| 계정 | 이벤트 |" in md and "## 교차-scope 훅" in md and "pack-dept-dept-9" in md,
      md[:200])
check("H15 경고 절 노출(파손 가시화)", "## 경고" in md and "claude-cat-dept-2" in md)
code, mdall, _ = run(home, reg, "--all")
check("H17 --all 절 추가", "그 외(일치·팩밖)" in mdall and len(mdall) > len(md))

# ── read-only 계약 ──
after = sorted(os.walk(home).__next__()[1])
check("H18 스캔이 HOME 구조를 바꾸지 않음", before == after, "%s → %s" % (before, after))
# ★H19 read-only 계약은 **AST로** 검사한다 — 문자열 포함 검사는 docstring의 `open(...,'w')` 설명
#   문구까지 잡아 오탐(실측 1회)하고, 반대로 `getattr(os,"rem"+"ove")` 류를 놓친다. 호출 노드로 본다.
import ast  # noqa: E402  (검사 전용 · 도구 자체는 표준 import만 쓴다)

_BANNED = {"os.remove", "os.unlink", "os.rename", "os.replace", "os.chmod", "os.mkdir",
           "os.makedirs", "os.rmdir", "os.symlink", "os.truncate", "shutil.rmtree",
           "shutil.move", "shutil.copyfile", "shutil.copy", "shutil.copytree"}


def _dotted(fn):
    """호출 대상의 점 표기 이름 — `v.replace`(문자열 메서드)와 `os.replace`(파일 교체)를 가른다."""
    if isinstance(fn, ast.Name):
        return fn.id
    if isinstance(fn, ast.Attribute):
        base = _dotted(fn.value)
        return (base + "." + fn.attr) if base else fn.attr
    return ""


_viol = []
for node in ast.walk(ast.parse(open(TOOL, encoding="utf-8").read(), TOOL)):
    if not isinstance(node, ast.Call):
        continue
    name = _dotted(node.func)
    if name in _BANNED:
        _viol.append("line %d: %s()" % (node.lineno, name))
    if name == "open":                       # 모드 인자가 없거나 'r' 계열이어야 한다
        mode = node.args[1] if len(node.args) > 1 else None
        for kw in node.keywords:
            if kw.arg == "mode":
                mode = kw.value
        if mode is not None and not (isinstance(mode, ast.Constant)
                                     and isinstance(mode.value, str)
                                     and set(mode.value) <= set("rbt")):
            _viol.append("line %d: open(mode=쓰기)" % node.lineno)
check("H19 ★read-only 계약: 쓰기·삭제 호출 0(AST)", not _viol, "; ".join(_viol))

# ── 빈 기계(신규 온보딩) — crash 0 ──
tmp2 = tempfile.mkdtemp(prefix="hookinv-empty-")
code, out2, err2 = run(os.path.join(tmp2), os.path.join(tmp2, "nope.json"), "--json")
check("H20 빈 HOME(신규 기계) crash 0", code == 0 and json.loads(out2)["summary"]["hooks"] == 0,
      err2.strip()[:160])
shutil.rmtree(tmp2)
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
