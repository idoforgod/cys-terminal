#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_guard_register.py — Phase 1 Wave D: 훅 등록 실행 도구(OT-2 runbook 집행부).

설계 SOT: _work/phase1-impl-package/DESIGN-DECISIONS.md §2-1 (c)·§1-3(등록 대상표 분리 독립)·
§6(OT dossier). 조건 24(훅 공통 모드 배포 결함)·조건 31(denylist 접점 격리)·조건 37(pane 카나리아).

★이 도구가 존재하는 이유(조건 24 실측): 종전 배선은 `javis_preflight` 계열의 **home-glob 전
프로필 일괄 등록**이었다 — `~/.claude*` 를 훑어 발견되는 모든 프로필에 같은 훅을 심는 방식.
그 방식은 ①대상표를 코드가 아니라 파일시스템 상태가 정하고 ②워커용 훅이 master 프로필에
섞여 들어가며 ③"어디에 등록했는지"를 사후에 아무도 모른다. 이 도구는 셋 다 거부한다:
**명시 `--profile` 인자만** 대상이 되고, home-glob·와일드카드 확장·자동 발견은 **없다**.

안전 계약(전부 기본값):
  · **`--dry-run` 이 기본**이다. 쓰기는 `--apply` 를 명시해야만 일어난다(플래그 부재 = 미리보기).
  · **byte-identical 멱등**: 새 텍스트가 기존 파일과 바이트 동일하면 **쓰지 않는다**(mtime 무변).
    이미 같은 command 가 등록돼 있으면 그룹을 추가하지 않는다(중복 등록 = Stop 체인 2회 실행).
  · **등록 전 백업**: `<settings>.bak-guard-<UTC타임스탬프>` 생성 후에만 원본을 바꾼다.
  · **원자 쓰기**: 같은 디렉터리 temp + `os.replace`(부분 쓰기로 settings 를 깨뜨리지 않는다).
  · **역할 경계 집행**(§1-3 대상표 분리 독립): `--hook stop`(completion-guard)은 master 프로필
    (`.claude`)을 **거부**한다 — guard 는 워커 2프로필 전용이고, 경고 훅(`--hook brief-warn`)만
    master 를 포함한다. 우회는 `--force-master` 명시 1회(사유 stdout 기록).
  · **등록 후 전수 재확인**: 인자로 받은 **모든** 대상을 다시 읽어 등록 여부·명령 문자열·중복
    개수를 표로 출력한다(쓴 것만이 아니라 전수 — "안 바뀐 대상"의 상태도 증거다).
  · 라이브 등록은 **[OT-2] 오너 승인 라인**이다. 이 도구는 승인을 대체하지 않는다.

사용:
  python3 javis_guard_register.py --hook stop --profile ~/.claude-cysinsight --profile ~/.cys/claude
  python3 javis_guard_register.py --hook brief-warn --profile ~/.claude --apply
  python3 javis_guard_register.py --hook stop --profile ~/.claude-cysinsight --emit-expected out.json
  python3 javis_guard_register.py --self-test

exit: 0 = 정상(dry-run 포함) · 1 = 대상 오류(부재·손상·역할 경계 위반) · 2 = 인자 오류.
"""
import argparse
import copy
import datetime
import hashlib
import json
import os
import shutil
import sys
import tempfile

EXIT_OK, EXIT_TARGET, EXIT_ARGS = 0, 1, 2

# 훅 카탈로그 — 이벤트·스크립트·대상 경계가 한곳에 모인다(코드가 대상표의 SOT).
HOOKS = {
    "stop": {
        "event": "Stop",
        "script": "hooks/completion-guard.sh",
        "master_allowed": False,      # §1-3: guard = 워커 2프로필 전용
        "why": "completion-guard(Stop) — 워커 turn 종료 시 verify 강제",
    },
    "brief-warn": {
        "event": "PostToolUse",
        "script": "hooks/brief-lint-warn.sh",
        "master_allowed": True,       # §1-3: 경고 훅 = master `~/.claude` 포함
        "matcher": "Task|Agent",
        "why": "brief-lint-warn(PostToolUse) — 인세션 위임 브리프 경고 주입(fail-open)",
    },
}
MASTER_PROFILE_BASENAMES = (".claude",)   # 역할 경계 판정 대상(dotdir basename)


def _now_tag():
    return datetime.datetime.utcnow().strftime("%Y%m%dT%H%M%SZ")


def _pack_dir():
    """src/pack.rs pack_dir() 4단 폴백 미러(javis_preflight.PACK_DIR_ENV_KEYS 와 동일 순서)."""
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def _command_str(spec, pack):
    """등록될 command 문자열 — 훅 실물 절대경로. 팩 경로가 바뀌면 문자열도 바뀐다(의도)."""
    return "sh %s" % os.path.join(pack, spec["script"])


def _resolve_settings(profile):
    """--profile 인자 → settings.json 절대경로. 디렉터리면 그 아래 settings.json.

    자동 발견·glob 확장은 하지 않는다: 인자가 가리키는 **정확히 그 파일 1개**만 대상이다.
    """
    p = os.path.abspath(os.path.expanduser(profile))
    if os.path.isdir(p):
        return os.path.join(p, "settings.json")
    return p


def _profile_basename(settings_path):
    return os.path.basename(os.path.dirname(os.path.abspath(settings_path)))


def _detect_indent(text):
    """원문 들여쓰기 폭 추정 — 재직렬화가 사용자 파일을 통째로 재포맷하지 않게 한다."""
    for line in text.splitlines():
        stripped = line.lstrip(" ")
        if stripped.startswith('"') and len(line) > len(stripped):
            return len(line) - len(stripped)
    return 2


def _read_settings(path):
    """(text, obj, err) — 판독 실패는 예외가 아니라 err 문자열로 돌려준다(전수 표 유지)."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        return None, None, "판독 불가: %s" % e
    try:
        obj = json.loads(text)
    except ValueError as e:
        return text, None, "JSON 손상: %s" % e
    if not isinstance(obj, dict):
        return text, None, "최상위가 객체가 아님(%s)" % type(obj).__name__
    return text, obj, None


def _groups(obj, event):
    h = obj.get("hooks")
    if not isinstance(h, dict):
        return []
    g = h.get(event)
    return g if isinstance(g, list) else []


def _count_registered(obj, event, command):
    """해당 이벤트에 이 command 가 몇 번 등록돼 있는가(중복 탐지 — 2 이상 = Stop 체인 2회 실행)."""
    n = 0
    for grp in _groups(obj, event):
        if not isinstance(grp, dict):
            continue
        for h in grp.get("hooks") or []:
            if isinstance(h, dict) and str(h.get("command", "")).strip() == command:
                n += 1
    return n


def _script_registered_any(obj, event, script_basename):
    """같은 스크립트가 **다른 경로 문자열**로 등록돼 있는가(팩 경로 이전 흔적 탐지)."""
    hits = []
    for grp in _groups(obj, event):
        if not isinstance(grp, dict):
            continue
        for h in grp.get("hooks") or []:
            if isinstance(h, dict) and script_basename in str(h.get("command", "")):
                hits.append(str(h.get("command")))
    return hits


def _with_hook(obj, spec, command):
    """등록된 새 객체(원본 불변) — 기존 키 순서·다른 이벤트는 손대지 않는다."""
    new = copy.deepcopy(obj)
    hooks = new.setdefault("hooks", {})
    if not isinstance(hooks, dict):
        raise ValueError("settings.hooks 가 객체가 아님(%s)" % type(hooks).__name__)
    lst = hooks.setdefault(spec["event"], [])
    if not isinstance(lst, list):
        raise ValueError("settings.hooks.%s 가 배열이 아님" % spec["event"])
    group = {"hooks": [{"type": "command", "command": command}]}
    if spec.get("matcher") is not None:
        group["matcher"] = spec["matcher"]
    lst.append(group)
    return new


def _serialize(obj, text):
    out = json.dumps(obj, ensure_ascii=False, indent=_detect_indent(text or ""))
    if (text or "").endswith("\n"):
        out += "\n"
    return out


def _atomic_write(path, data):
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(prefix=".guardreg-", dir=d)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(data)
        os.replace(tmp, path)
    except BaseException:
        with open(os.devnull, "w"):
            pass
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def _sha(data):
    return hashlib.sha256(data.encode("utf-8")).hexdigest()


# ── 처리 ────────────────────────────────────────────────────────────────────
def process(profiles, hook_key, apply_, force_master, pack, out=sys.stdout):
    spec = HOOKS[hook_key]
    command = _command_str(spec, pack)
    script_base = os.path.basename(spec["script"])
    hook_path = os.path.join(pack, spec["script"])
    rc = EXIT_OK
    rows = []

    print("훅: %s (%s · %s)" % (hook_key, spec["event"], spec["why"]), file=out)
    print("command: %s" % command, file=out)
    print("모드: %s" % ("APPLY(쓰기)" if apply_ else "DRY-RUN(기본 — 쓰기 0)"), file=out)
    if not os.path.isfile(hook_path):
        print("경고: 훅 실물 부재 — %s (등록해도 래퍼가 없으면 무발동)" % hook_path, file=out)

    for prof in profiles:
        sp = _resolve_settings(prof)
        base = _profile_basename(sp)
        row = {"profile": base, "settings": sp, "action": None, "note": ""}

        # 역할 경계(§1-3 대상표 분리 독립) — 판정은 **쓰기 전**에 한다.
        if base in MASTER_PROFILE_BASENAMES and not spec["master_allowed"] and not force_master:
            row["action"] = "REFUSED"
            row["note"] = ("역할 경계: %s 는 워커 전용 훅이라 master 프로필(%s)에 등록하지 않는다"
                           " — 대상표 분리 독립(§1-3). 의도했다면 --force-master" % (hook_key, base))
            rows.append(row)
            rc = EXIT_TARGET
            continue

        text, obj, err = _read_settings(sp)
        if err:
            row["action"] = "ERROR"
            row["note"] = err
            rows.append(row)
            rc = EXIT_TARGET
            continue

        n = _count_registered(obj, spec["event"], command)
        stale = [c for c in _script_registered_any(obj, spec["event"], script_base) if c != command]
        if stale:
            row["note"] = "다른 경로로 등록된 동일 스크립트 %d건: %s" % (len(stale), "; ".join(stale))
        if n >= 1:
            row["action"] = "ALREADY(%d)" % n
            if n > 1:
                row["note"] = (row["note"] + " · " if row["note"] else "") + \
                    "중복 등록 %d건 — Stop 체인 %d회 실행(수동 정리 필요·이 도구는 삭제하지 않는다)" % (n, n)
            rows.append(row)
            continue

        try:
            new_obj = _with_hook(obj, spec, command)
        except ValueError as e:
            row["action"] = "ERROR"
            row["note"] = str(e)
            rows.append(row)
            rc = EXIT_TARGET
            continue
        new_text = _serialize(new_obj, text)

        if new_text == text:      # byte-identical — 쓸 것이 없다(멱등)
            row["action"] = "IDENTICAL"
            rows.append(row)
            continue

        if not apply_:
            row["action"] = "WOULD-ADD"
            row["note"] = (row["note"] + " · " if row["note"] else "") + \
                "sha256 %s → %s" % (_sha(text)[:12], _sha(new_text)[:12])
            rows.append(row)
            continue

        backup = "%s.bak-guard-%s" % (sp, _now_tag())
        shutil.copy2(sp, backup)
        _atomic_write(sp, new_text)
        row["action"] = "ADDED"
        row["note"] = (row["note"] + " · " if row["note"] else "") + "백업 %s" % backup
        rows.append(row)

    # ── 등록 후 전수 재확인(쓴 것만이 아니라 인자 전부를 다시 읽는다) ──
    print("\n── 전수 재확인(인자 대상 %d개 · 재판독) ──" % len(profiles), file=out)
    print("%-22s %-12s %s" % ("PROFILE", "STATE", "DETAIL"), file=out)
    verify = []
    for row in rows:
        sp = row["settings"]
        _t, obj, err = _read_settings(sp)
        if err:
            state, detail = "UNREADABLE", err
        else:
            n = _count_registered(obj, spec["event"], command)
            state = "REGISTERED(%d)" % n if n else "ABSENT"
            detail = "%s | %s" % (row["action"], row["note"] or "-")
        verify.append({"profile": row["profile"], "settings": sp, "state": state,
                       "action": row["action"]})
        print("%-22s %-12s %s" % (row["profile"], state, detail), file=out)
    print(json.dumps({"hook": hook_key, "event": spec["event"], "command": command,
                      "apply": bool(apply_), "results": verify}, ensure_ascii=False), file=out)
    return rc, rows, command, spec


def emit_expected(profiles, hook_key, pack, path, apply_, out=sys.stdout):
    """C73 기대 집합 마커(guard-hook-expected.json) 실물 생성 — OT-2 절차의 도구화.

    dry-run 이면 **내용을 출력만** 한다(파일을 만들지 않는다). 스키마는 preflight C73 이
    읽는 것과 동일: profiles[].{settings, sha256, must_contain}.
    """
    spec = HOOKS[hook_key]
    command = _command_str(spec, pack)
    entries = []
    for prof in profiles:
        sp = _resolve_settings(prof)
        try:
            raw = open(sp, "rb").read()
        except OSError as e:
            print("기대 표 생성 건너뜀(%s): %s" % (sp, e), file=out)
            continue
        entries.append({"settings": sp,
                        "sha256": hashlib.sha256(raw).hexdigest(),
                        "must_contain": [command]})
    doc = {"schema_version": 1, "generated_at": _now_tag(),
           "note": ("C73 기대 집합 마커 — must_contain 이 1급 신호이고 sha256 불일치는 "
                    "must_contain 통과 시 WARN 강등(G7g). settings 를 정당 변경한 주체가 "
                    "같은 변경에서 이 파일의 sha256 을 재계산·갱신할 책임을 진다."),
           "profiles": entries}
    body = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"
    if not apply_:
        print("\n── 기대 표(미기록 · --apply 시 %s 에 기록) ──\n%s" % (path, body), file=out)
        return
    _atomic_write(path, body)
    print("\n기대 표 기록: %s (프로필 %d)" % (path, len(entries)), file=out)


# ── self-test(격리 tmpdir · 라이브 무접촉) ──────────────────────────────────
def self_test():
    import io
    fails = []

    def chk(cond, msg):
        if not cond:
            fails.append(msg)

    with tempfile.TemporaryDirectory(prefix="guardreg-selftest-") as td:
        pack = os.path.join(td, "pack")
        os.makedirs(os.path.join(pack, "hooks"))
        for s in ("completion-guard.sh", "brief-lint-warn.sh"):
            open(os.path.join(pack, "hooks", s), "w").write("#!/bin/sh\nexit 0\n")

        def mkprof(name, body):
            d = os.path.join(td, name)
            os.makedirs(d, exist_ok=True)
            p = os.path.join(d, "settings.json")
            open(p, "w", encoding="utf-8").write(body)
            return d, p

        wdir, wset = mkprof(".claude-cysinsight",
                            '{\n  "model": "opus",\n  "hooks": {\n    "Stop": []\n  }\n}\n')
        mdir, mset = mkprof(".claude", '{\n  "hooks": {}\n}\n')

        # ① dry-run 기본 = 쓰기 0
        before = open(wset, encoding="utf-8").read()
        buf = io.StringIO()
        rc, rows, cmd, _s = process([wdir], "stop", False, False, pack, out=buf)
        chk(rc == EXIT_OK, "① dry-run rc=%s" % rc)
        chk(rows[0]["action"] == "WOULD-ADD", "① action=%s" % rows[0]["action"])
        chk(open(wset, encoding="utf-8").read() == before, "① dry-run 인데 파일이 변경됨")
        chk("DRY-RUN" in buf.getvalue(), "① 모드 표기 부재")

        # ② --apply = 등록 + 백업 생성 + JSON 유효
        buf = io.StringIO()
        rc, rows, cmd, _s = process([wdir], "stop", True, False, pack, out=buf)
        chk(rows[0]["action"] == "ADDED", "② action=%s" % rows[0]["action"])
        baks = [f for f in os.listdir(wdir) if ".bak-guard-" in f]
        chk(len(baks) == 1, "② 백업 %d개(1 기대)" % len(baks))
        obj = json.load(open(wset, encoding="utf-8"))
        chk(_count_registered(obj, "Stop", cmd) == 1, "② 등록 계수 불일치")
        chk(obj.get("model") == "opus", "② 기존 키 소실")
        chk("REGISTERED(1)" in buf.getvalue(), "② 전수 재확인 출력 부재")

        # ③ 멱등 — 재실행은 ALREADY·바이트 동일·백업 추가 0
        sha_before = _sha(open(wset, encoding="utf-8").read())
        buf = io.StringIO()
        _rc, rows, _c, _s = process([wdir], "stop", True, False, pack, out=buf)
        chk(rows[0]["action"].startswith("ALREADY"), "③ action=%s" % rows[0]["action"])
        chk(_sha(open(wset, encoding="utf-8").read()) == sha_before, "③ 멱등 위반(바이트 변경)")
        chk(len([f for f in os.listdir(wdir) if ".bak-guard-" in f]) == 1, "③ 백업이 추가됨")

        # ④ 역할 경계 — master 프로필에 stop 훅은 거부(rc=1)·파일 무변
        m_before = open(mset, encoding="utf-8").read()
        buf = io.StringIO()
        rc, rows, _c, _s = process([mdir], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET, "④ rc=%s(1 기대)" % rc)
        chk(rows[0]["action"] == "REFUSED", "④ action=%s" % rows[0]["action"])
        chk(open(mset, encoding="utf-8").read() == m_before, "④ 거부인데 파일이 변경됨")

        # ⑤ 경고 훅은 master 허용(대상표 분리 독립) + matcher 부착
        buf = io.StringIO()
        rc, rows, cmd2, _s = process([mdir], "brief-warn", True, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"] == "ADDED", "⑤ master 경고 훅 등록 실패")
        mo = json.load(open(mset, encoding="utf-8"))
        grp = [g for g in _groups(mo, "PostToolUse")
               if any(h.get("command") == cmd2 for h in g.get("hooks") or [])]
        chk(grp and grp[0].get("matcher") == "Task|Agent", "⑤ matcher 미부착")

        # ⑥ 손상 settings = ERROR·rc 1·원본 무변(파괴 금지)
        bdir, bset = mkprof(".claude-broken", "{ this is not json ")
        b_before = open(bset, encoding="utf-8").read()
        buf = io.StringIO()
        rc, rows, _c, _s = process([bdir], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "ERROR", "⑥ 손상 처리 실패")
        chk(open(bset, encoding="utf-8").read() == b_before, "⑥ 손상 파일이 변경됨")
        chk("UNREADABLE" in buf.getvalue(), "⑥ 전수 재확인에 UNREADABLE 부재")

        # ⑦ 기대 표 — dry-run 은 미기록, apply 는 스키마 3필드
        exp = os.path.join(td, "guard-hook-expected.json")
        buf = io.StringIO()
        emit_expected([wdir], "stop", pack, exp, False, out=buf)
        chk(not os.path.exists(exp), "⑦ dry-run 인데 기대 표가 기록됨")
        emit_expected([wdir], "stop", pack, exp, True, out=io.StringIO())
        d = json.load(open(exp, encoding="utf-8"))
        chk(d["schema_version"] == 1 and d["profiles"][0]["must_contain"] == [cmd],
            "⑦ 기대 표 스키마 위반")
        chk(len(d["profiles"][0]["sha256"]) == 64, "⑦ sha256 형식 위반")

        # ⑧ home-glob 부재 — 인자 없이는 어떤 대상도 만들어지지 않는다
        buf = io.StringIO()
        rc, rows, _c, _s = process([], "stop", True, False, pack, out=buf)
        chk(rows == [], "⑧ 인자 0인데 대상이 생김(자동 발견 금지)")

    if fails:
        print("javis_guard_register self-test FAIL %d건:" % len(fails), file=sys.stderr)
        for f in fails:
            print("  - " + f, file=sys.stderr)
        return 1
    print("javis_guard_register self-test OK — dry-run 기본·apply 등록/백업·멱등 바이트동일·"
          "역할 경계 거부·경고훅 master 허용+matcher·손상 무파괴·기대 표 스키마·자동발견 0")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Phase 1 훅 등록(명시 프로필만·dry-run 기본·byte-identical 멱등)")
    ap.add_argument("--hook", choices=sorted(HOOKS), default="stop",
                    help="등록할 훅(기본 stop=completion-guard)")
    ap.add_argument("--profile", action="append", default=[], metavar="PATH",
                    help="대상 프로필 디렉터리 또는 settings.json (반복 지정 · home-glob 금지)")
    ap.add_argument("--apply", action="store_true",
                    help="실제 쓰기(미지정 시 dry-run — 기본값이 안전측)")
    ap.add_argument("--force-master", action="store_true",
                    help="역할 경계 우회(워커 전용 훅을 master 프로필에 등록) — 의도 명시용")
    ap.add_argument("--pack", default=None, help="팩 경로(기본 CYS_PACK_DIR→~/.cys/pack)")
    ap.add_argument("--emit-expected", metavar="PATH", default=None,
                    help="C73 기대 집합 마커 생성(dry-run 이면 내용 출력만)")
    ap.add_argument("--self-test", action="store_true", help="격리 tmpdir 자기검증")
    a = ap.parse_args(argv)

    if a.self_test:
        return self_test()
    if not a.profile:
        ap.error("--profile 이 최소 1개 필요하다 — 이 도구는 프로필을 스스로 찾지 않는다"
                 "(home-glob 일괄 등록 금지 · 조건 24)")
    pack = a.pack or _pack_dir()
    rc, _rows, _cmd, _spec = process(a.profile, a.hook, a.apply, a.force_master, pack)
    if a.emit_expected:
        emit_expected(a.profile, a.hook, pack, a.emit_expected, a.apply)
    if not a.apply:
        print("\n※ DRY-RUN 이었다 — 실제 등록은 --apply 이며, 라이브 등록은 [OT-2] 오너 승인 라인이다.")
    return rc


if __name__ == "__main__":
    sys.exit(main())
