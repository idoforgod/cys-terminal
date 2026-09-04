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
    을 **거부**한다 — guard 는 워커 프로필 전용이고, 경고 훅(`--hook brief-warn`)만
    master 를 포함한다. 우회는 `--force-master` 명시 1회(사유 stdout 기록).
  · **등록 후 전수 재확인**: 인자로 받은 **모든** 대상을 다시 읽어 등록 여부·명령 문자열·중복
    개수를 표로 출력한다(쓴 것만이 아니라 전수 — "안 바뀐 대상"의 상태도 증거다).
  · 라이브 등록은 **[OT-2] 오너 승인 라인**이다. 이 도구는 승인을 대체하지 않는다.

★E1-2(BLOCKER R-01 · T0 D24 이행) — settings 훅 항목 `timeout` 필드:
  종전 등록은 `{"type":"command","command":...}` 만 넣어 상한 사다리의 **바깥 겹이 없었다**.
  래퍼(`hooks/completion-guard.sh`)의 `timeout 60` 은 coreutils `timeout` 이 있는 환경에서만
  존재하고, **이 배포 머신에는 부재**(`command -v timeout` rc=1 실측)라 "3중 상한 60>50>30"이
  사실이 아니었다. 이제 카탈로그가 훅별 `timeout` 을 갖고 등록 항목에 함께 쓴다:
    stop=60(guard 자체 데드라인 50 보다 바깥) · brief-warn=20(fail-open 경고 훅).
  이미 등록된 항목의 timeout 불일치는 **표에 loud 하게 적고 exit 1**(말없이 고쳐 쓰지 않는다).
  정정은 `--repair-timeout`(+`--apply`) 명시 1회.

★E3-1(BLOCKER R-04 잔여 · HOOK_TARGETS_CONTRACT.md §5-1 병합 티켓 이행) — 대상표 소비:
  종전 역할 경계 판정의 SOT 는 아래 `MASTER_PROFILE_BASENAMES=(".claude",)` **하드코딩**이었다.
  그 표는 실측 6프로필 중 **1개만** 알아서, 실제 라이브 master 세션 프로필(`~/.claude-3` —
  ps 실측 CLAUDE_CONFIG_DIR)에 `--hook stop --profile ~/.claude-3 --apply` 를 걸어도
  **거부되지 않았다**(조건 24② "비워커 프로필 등록=반려"가 기계 집행되지 않음 · E2 실측 GAP 7건).
  이제 이 도구는 `$PACK/state/hook-targets.json` 을 읽어 `profiles[].eligibility.<훅키>` 로
  허용/거부를 판정한다 — **코드가 표를 정하지 않고 표가 코드를 정한다**(계약 §1).
    · 표 **부재** = 같은 경로의 `.example`(배포 실물)을 **폴백 표로 로드** + 고지 1줄.
      ★E4-1(사인오프 R-04 잔여 · HIGH): 종전에는 표 부재가 곧 코드 하드코딩 폴백이었는데,
      배포 실물이 `hook-targets.json.example` 뿐이고 `.json` 을 만드는 설치 스텝이 팩·계약·
      OT 롯북 어디에도 없었다(전수 grep 0건) ⇒ **install 직후 기본 상태에서 기계 방어가 죽어
      있었다**(실측: 표 부재 모드 `--hook stop --profile ~/.claude-3` = WOULD-ADD·rc=0).
      이제 `.example` 이 폴백 표로 집행되므로 install 직후에도 판정이 살아 있다. 명시 설치
      (`cp state/hook-targets.json.example state/hook-targets.json`)는 여전히 운영 규율이고
      (OT-DOSSIER §2.0), 폴백은 그 규율이 빠졌을 때의 **안전망**이다(둘은 대체가 아니다).
    · 표·`.example` **둘 다 부재** = 종전 코드 하드코딩 폴백 + 경고 1줄(최종 안전망).
    · 표 **손상** = 폴백하지 않고 **exit 2 · 쓰기 0**(손상을 관대하게 접으면 침묵 무발동이
      되고, 그것이 이 Phase 가 잡고 있는 결함 부류다 — R-02 동형). `.example` 이 손상된
      경우도 같다(배포물 손상 = 포장 결함이므로 조용히 약한 판정으로 접지 않는다).
    · 표에 **없는** basename = `policy.unknown_profile`(=deny)에 따라 거부 · `--force-unknown`
      명시 1회로만 통과(새 프로필 `~/.claude-4` 가 조용히 대상이 되는 경로를 닫는다).
    · `--from-table` = 대상 프로필 목록 자체를 표의 allow 집합에서 파생(타이핑 오류 제거).
    · `--emit-expected`(C73)·`--emit-warn-targets`(C74)도 같은 표에서 파생 —
      세 표(코드·C73·C74)의 **파생원을 하나로** 만든다(계약 §5-1 ③).
  판정 어휘는 신설하지 않았다: 거부는 종전과 같은 `REFUSED` 행이고 문면 형식도 같다.

사용:
  python3 javis_guard_register.py --hook stop --profile ~/.claude-worker --profile ~/.cys/claude
  python3 javis_guard_register.py --hook brief-warn --profile ~/.claude --apply
  python3 javis_guard_register.py --hook stop --profile ~/.claude-worker --emit-expected out.json
  python3 javis_guard_register.py --self-test

exit: 0 = 정상(dry-run 포함) · 1 = 대상 오류(부재·손상·역할 경계 위반) · 2 = 인자 오류.
"""
import argparse
import copy
import datetime
import hashlib
import contextlib
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
        # E1-2(BLOCKER R-01 · T0 D24 명시 지시 이행): settings 훅 항목 자체의 상한.
        # 상한 사다리(바깥→안쪽): settings timeout 60 > guard 자체 데드라인 50 >
        # verify 개별 상한 30. **바깥 60 은 이 필드가 유일한 환경 무관 겹**이다 —
        # 래퍼의 `timeout 60` 은 coreutils 부재 환경(이 머신 실측 rc=1)에서 사라진다.
        "timeout": 60,
        "why": "completion-guard(Stop) — 워커 turn 종료 시 verify 강제",
    },
    "brief-warn": {
        "event": "PostToolUse",
        "script": "hooks/brief-lint-warn.sh",
        "master_allowed": True,       # §1-3: 경고 훅 = master `~/.claude` 포함
        "matcher": "Task|Agent",
        # 경고 훅은 fail-open(전 경로 exit 0)이라 상한은 짧게 — 주입 지연이 위임을 붙잡지
        # 않게 한다(차단력 0이므로 끊겨도 손실은 '경고 1건 미주입'뿐).
        "timeout": 20,
        "why": "brief-lint-warn(PostToolUse) — 인세션 위임 브리프 경고 주입(fail-open)",
    },
}
MASTER_PROFILE_BASENAMES = (".claude",)   # ★폴백 전용 — 표 부재 시에만 쓰인다(E3-1)

# 훅 키 → 대상표 eligibility 필드. 두 집합은 **다르다**(계약 §2): 파일을 합치는 것과 판정을
# 합치는 것은 다르고, 합치면 master pane 의 Stop 체인에 검증 블록이 걸린다(D5 경고).
HOOK_ELIGIBILITY_KEY = {"stop": "guard_stop", "brief-warn": "brief_warn"}
TARGETS_REL = os.path.join("state", "hook-targets.json")
TARGETS_EXAMPLE_SUFFIX = ".example"   # ★E4-1: 배포 실물 = <표>.example (폴백 표)


def _targets_path(pack, override=None):
    return override or os.path.join(pack, TARGETS_REL)


def _resolve_targets(pack, override=None):
    """(table|None, err|None, path, source) — 표 해석 2단(운영 표 → 배포 예시표 폴백).

    source ∈ {"file", "example", "none"}:
      · "file"    = `$PACK/state/hook-targets.json`(또는 --hook-targets override) 실물
      · "example" = 같은 경로 + `.example`(★E4-1 · install 직후 기본 상태)
      · "none"    = 둘 다 부재 → 호출자가 코드 하드코딩 폴백으로 간다(최종 안전망)

    폴백 규칙은 **경로 단위로 균일**하다 — override 를 줬으면 `<override>.example` 만 본다
    (팩 예시표로 몰래 되돌아가지 않는다: 운영자가 고른 출처를 도구가 바꾸지 않는다).
    손상은 어느 단계든 err 로 올라가고 호출자는 폴백 없이 멈춘다(exit 2 · 쓰기 0).
    """
    path = _targets_path(pack, override)
    table, err = _load_targets(path)
    if err:
        return None, err, path, "file"
    if table is not None:
        return table, None, path, "file"
    expath = path + TARGETS_EXAMPLE_SUFFIX
    table, err = _load_targets(expath)
    if err:
        return None, err, expath, "example"
    if table is not None:
        return table, None, expath, "example"
    return None, None, path, "none"


def _load_targets(path):
    """(table|None, err|None) — 부재는 err 가 아니다(폴백 경로). 손상만 err.

    반환 table 은 `{"doc":<원문 dict>, "index":{basename: profile}, "policy":{...},
    "sha256":<파일 해시>, "path":<경로>}`. 인덱스 키는 **basename 문자열**이다 —
    `~/.cys/claude` 의 basename 은 `claude`(선행 점 없음)이고 `.claude` 와 **다른 키**다
    (계약 profiles[].basename_note · 혼동 금지).
    """
    if not os.path.isfile(path):
        return None, None
    try:
        with open(path, "rb") as f:
            raw = f.read()
        doc = json.loads(raw.decode("utf-8"))
    except (OSError, ValueError, UnicodeDecodeError) as e:
        return None, "판독/파싱 실패: %s" % e
    if not isinstance(doc, dict):
        return None, "최상위가 객체가 아님(%s)" % type(doc).__name__
    if doc.get("schema_version") != 1:
        return None, "schema_version=%r (1 기대 — 세대 불일치)" % doc.get("schema_version")
    profiles = doc.get("profiles")
    if not isinstance(profiles, list) or not profiles:
        return None, "profiles 가 비어 있거나 배열이 아님"
    index = {}
    for ent in profiles:
        if not isinstance(ent, dict) or not ent.get("basename"):
            return None, "profiles 항목 형식 오류: %r" % (ent,)
        elig = ent.get("eligibility")
        if not isinstance(elig, dict):
            return None, "%s: eligibility 누락/형식 오류" % ent.get("basename")
        for k in HOOK_ELIGIBILITY_KEY.values():
            v = elig.get(k)
            if v not in ("allow", "deny"):
                return None, "%s: eligibility.%s=%r (allow|deny 기대)" % (
                    ent.get("basename"), k, v)
        if ent["basename"] in index:
            return None, "basename 중복: %s(판정이 둘로 갈린다)" % ent["basename"]
        index[ent["basename"]] = ent
    policy = doc.get("policy") if isinstance(doc.get("policy"), dict) else {}
    if policy.get("unknown_profile") not in ("deny", "allow"):
        return None, "policy.unknown_profile=%r (deny|allow 기대)" % policy.get("unknown_profile")
    return {"doc": doc, "index": index, "policy": policy, "path": path,
            "sha256": hashlib.sha256(raw).hexdigest()}, None


def _decide(table, base, hook_key, spec, force_master, force_unknown):
    """(ok: bool, note: str) — 이 프로필에 이 훅을 등록해도 되는가.

    표가 있으면 표가 정하고, 없으면 종전 하드코딩이 정한다. 거부 문면은 종전 REFUSED 문면
    형식을 그대로 쓴다(어휘 신설 0 · 계약 §5-1 ②).
    """
    if table is None:                                   # 폴백(표 부재) — 종전 판정 그대로
        if base in MASTER_PROFILE_BASENAMES and not spec["master_allowed"] and not force_master:
            return False, ("역할 경계: %s 는 워커 전용 훅이라 master 프로필(%s)에 등록하지 "
                           "않는다 — 대상표 분리 독립(§1-3 · 표 부재 폴백 판정). 의도했다면 "
                           "--force-master" % (hook_key, base))
        return True, ""
    ent = table["index"].get(base)
    if ent is None:                                     # 표 밖 = deny-by-default
        if table["policy"].get("unknown_profile") == "deny" and not force_unknown:
            return False, ("역할 경계: %s 는 대상표(%s)에 없는 미지 프로필이라 등록하지 않는다"
                           " — deny-by-default(policy.unknown_profile). 의도했다면 "
                           "--force-unknown" % (base, table["path"]))
        return True, ("표 밖 프로필 — %s" % ("--force-unknown 우회"
                                             if force_unknown else "policy=allow"))
    verdict = ent["eligibility"][HOOK_ELIGIBILITY_KEY[hook_key]]
    if verdict == "deny" and not force_master:
        return False, ("역할 경계: %s 는 대상표에서 %s=deny 다 — 역할 %s · 근거 %s. "
                       "의도했다면 --force-master"
                       % (base, HOOK_ELIGIBILITY_KEY[hook_key], ent.get("role") or "미기재",
                          (ent.get("why") or "미기재")[:120]))
    if verdict == "deny":
        return True, "표 deny 를 --force-master 로 우회(역할 %s)" % (ent.get("role") or "미기재")
    return True, ""


def _table_eligible(table, hook_key):
    """표에서 이 훅이 allow 인 프로필 항목 목록(사전순 — 파생표 3개의 공용 파생원)."""
    key = HOOK_ELIGIBILITY_KEY[hook_key]
    return [e for _b, e in sorted(table["index"].items())
            if e["eligibility"][key] == "allow"]


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


def _registered_timeouts(obj, event, command):
    """E1-2(R-01): 이미 등록된 같은 command 항목들의 `timeout` 값 목록(미기재 = None).

    멱등 비교의 단위는 여전히 **command 문자열**이다(중복 등록 = Stop 체인 2회 실행이므로
    같은 command 를 다시 넣지 않는다). 다만 timeout 필드가 빠졌거나 값이 다르면 상한
    사다리의 바깥 겹이 없는 상태이므로 **표에 loud 하게 적는다** — 이 도구는 남의 등록을
    말없이 고쳐 쓰지 않는다(정정은 `--repair-timeout` 명시).
    """
    out = []
    for grp in _groups(obj, event):
        if not isinstance(grp, dict):
            continue
        for h in grp.get("hooks") or []:
            if isinstance(h, dict) and str(h.get("command", "")).strip() == command:
                out.append(h.get("timeout"))
    return out


def _with_timeout_repaired(obj, spec, command):
    """등록된 같은 command 항목의 timeout 을 spec 값으로 맞춘 새 객체(원본 불변)."""
    new = copy.deepcopy(obj)
    want = int(spec["timeout"])
    for grp in _groups(new, spec["event"]):
        if not isinstance(grp, dict):
            continue
        for h in grp.get("hooks") or []:
            if isinstance(h, dict) and str(h.get("command", "")).strip() == command:
                h["timeout"] = want
    return new


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
    entry = {"type": "command", "command": command}
    if spec.get("timeout") is not None:       # E1-2(R-01): 상한 사다리의 바깥 겹
        entry["timeout"] = int(spec["timeout"])
    group = {"hooks": [entry]}
    if spec.get("matcher") is not None:
        group["matcher"] = spec["matcher"]
    lst.append(group)
    return new


def _serialize(obj, text):
    out = json.dumps(obj, ensure_ascii=False, indent=_detect_indent(text or ""))
    if (text or "").endswith("\n"):
        out += "\n"
    return out


# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   선례 `javis_preflight.py:33-35` · `javis_bootstrap.py:127-129`(append 인 이유도 같다:
#   발견이 목적이지 stdlib precedence 를 강등하지 않는다) · 계약 `tests/test_import_guard.py`.
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★공용 락 소비(2026-09-04 P0 · master 지시 ②). 이 모듈은 settings.json 의 **5 writer 중 하나**
#   인데 종전엔 자기 mkstemp+replace 만 했다 — 발행은 원자적이라 파손은 없지만 **직렬화가 없어
#   lost update 가 난다**(실측: python 3 writer × 6 프로세스 동시 실행 6/6 시행에서 marks 38~46/72
#   유실 · 파손은 0). preflight `_settings_rmw` 와 **같은 락 파일**(`<settings>.cys-lock`)을 잡아
#   같은 소유자를 경유하게 만든다.
#   import 실패(팩 스큐·부분 배포)는 등록을 죽이지 않는다 — 락 없이 진행하되 조용하지 않게 stderr.
try:
    import javis_lock as _lock
except Exception:
    _lock = None


@contextlib.contextmanager
def _settings_lock(settings_path):
    """settings.json 파일별 공용 락. 획득 실패·백엔드 부재는 **열화**이지 중단이 아니다
    (등록을 포기하면 훅이 사라진다 — 가용성 우선 · preflight 와 동일 판단)."""
    lk = None
    if _lock is not None:
        try:
            lk = _lock.FileLock(settings_path + ".cys-lock", owner="guard-register",
                                blocking=True, timeout=10.0, soft=True)
            lk.acquire()
            if lk.status != _lock.ACQUIRED:
                sys.stderr.write("[guard-register] settings 락 미획득(%s) — 직렬화 없이 진행: %s\n"
                                 % (lk.status, settings_path))
        except Exception as e:
            sys.stderr.write("[guard-register] settings 락 사용 불가(%s) — 직렬화 없이 진행\n" % e)
            lk = None
    try:
        yield
    finally:
        if lk is not None:
            try:
                lk.release()
            except Exception:
                pass


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
def process(profiles, hook_key, apply_, force_master, pack, out=None,
            repair_timeout=False, force_unknown=False, targets_path=None, table=None,
            table_source=None):
    # out 기본값을 def 시점에 sys.stdout 으로 **묶지 않는다** — 묶으면 호출자의
    # redirect_stdout 이 무효가 되고(자기검증 하네스가 출력을 회수하지 못한다) 그 무능이
    # "검증했다"로 오독된다(계측 타당성).
    out = sys.stdout if out is None else out
    spec = HOOKS[hook_key]
    command = _command_str(spec, pack)
    script_base = os.path.basename(spec["script"])
    hook_path = os.path.join(pack, spec["script"])
    rc = EXIT_OK
    rows = []

    # ★E3-1: 대상표 로드 — 판정 SOT. 부재=폴백 · 손상=중단(관대한 접기 금지).
    # ★E4-1: 부재 폴백이 2단이다 — `.example`(배포 실물) → 코드 하드코딩.
    tpath = _targets_path(pack, targets_path)
    tsource = table_source
    if table is None and tsource is None:
        table, terr, tpath, tsource = _resolve_targets(pack, targets_path)
        if terr:
            print("대상표 손상%s: %s — %s"
                  % ("(배포 예시표)" if tsource == "example" else "", tpath, terr), file=out)
            print("→ 등록 중단(쓰기 0). 손상된 표를 하드코딩으로 조용히 대체하지 않는다 — "
                  "그 침묵이 R-04/R-02 가 잡고 있는 결함이다. 표를 고치거나 지워라"
                  "(운영 표를 지우면 배포 예시표 폴백, 예시표까지 없으면 하드코딩 폴백 · "
                  "다른 표를 쓰려면 --hook-targets).", file=out)
            return EXIT_ARGS, [], command, spec
    elif table is not None:
        tpath, tsource = table["path"], (tsource or "file")

    print("훅: %s (%s · %s)" % (hook_key, spec["event"], spec["why"]), file=out)
    print("command: %s" % command, file=out)
    print("timeout: %s (settings 훅 항목 상한 — E1-2/R-01 · 사다리 바깥 겹)"
          % (spec.get("timeout") if spec.get("timeout") is not None else "미기재"), file=out)
    if table is None:
        print("대상표: 부재(%s) · 배포 예시표도 부재(%s%s) — **코드 하드코딩 폴백**"
              "(master=%s · 실측 6프로필 중 1개만 안다)"
              % (tpath, tpath, TARGETS_EXAMPLE_SUFFIX, ", ".join(MASTER_PROFILE_BASENAMES)),
              file=out)
        print("        ※ 이 상태에서는 조건 24②(비워커 프로필 등록=반려)가 기계 집행되지 "
              "않는다 — 표를 공급하라(HOOK_TARGETS_CONTRACT.md).", file=out)
    else:
        print("대상표: %s (schema v%d · sha256 %s… · 프로필 %d · 미지=%s · 실측 %s)"
              % (table["path"], table["doc"]["schema_version"], table["sha256"][:12],
                 len(table["index"]), table["policy"].get("unknown_profile"),
                 table["doc"].get("measured_at") or "미기재"), file=out)
        if tsource == "example":
            # ★E4-1 고지 1줄 — 폴백이 조용하면 그 자체가 이 Phase 가 잡는 결함 부류다.
            print("        ※ 운영 표(%s) 부재 → **배포 예시표 폴백으로 판정 중**(기계 방어는 "
                  "살아 있다). 운영 표를 확정하려면: cp %s%s %s (설치 후 measured_at 갱신)"
                  % (_targets_path(pack, targets_path), _targets_path(pack, targets_path),
                     TARGETS_EXAMPLE_SUFFIX, _targets_path(pack, targets_path)), file=out)
    print("모드: %s" % ("APPLY(쓰기)" if apply_ else "DRY-RUN(기본 — 쓰기 0)"), file=out)
    if not os.path.isfile(hook_path):
        print("경고: 훅 실물 부재 — %s (등록해도 래퍼가 없으면 무발동)" % hook_path, file=out)

    for prof in profiles:
        sp = _resolve_settings(prof)
        with _settings_lock(sp):   # ★읽기→쓰기 전 구간을 직렬화(lost update 차단)
            base = _profile_basename(sp)
            row = {"profile": base, "settings": sp, "action": None, "note": ""}

            # 역할 경계(§1-3 대상표 분리 독립) — 판정은 **쓰기 전**에 한다.
            ok, why = _decide(table, base, hook_key, spec, force_master, force_unknown)
            if not ok:
                row["action"] = "REFUSED"
                row["note"] = why
                rows.append(row)
                rc = EXIT_TARGET
                continue
            if why:
                row["note"] = why

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
                # E1-2(R-01): 기존 등록의 timeout 필드 대조 — 불일치는 loud 하게 적는다.
                want_to = spec.get("timeout")
                found = _registered_timeouts(obj, spec["event"], command)
                if want_to is not None and any(t != want_to for t in found):
                    row["timeout_mismatch"] = found
                    row["note"] = (row["note"] + " · " if row["note"] else "") + \
                        ("timeout 불일치 %s (기대 %s) — 상한 사다리 바깥 겹 부재/상이"
                         % (found, want_to))
                    if repair_timeout:
                        new_obj = _with_timeout_repaired(obj, spec, command)
                        new_text = _serialize(new_obj, text)
                        if not apply_:
                            row["action"] = "WOULD-FIX-TIMEOUT"
                        else:
                            backup = "%s.bak-guard-%s" % (sp, _now_tag())
                            shutil.copy2(sp, backup)
                            _atomic_write(sp, new_text)
                            row["action"] = "FIXED-TIMEOUT"
                            row["note"] += " · 백업 %s" % backup
                    else:
                        rc = EXIT_TARGET   # 무시 금지 — 호출자가 exit code 로 알아채야 한다
                        row["note"] += " · 정정: --repair-timeout"
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
        tos = None if err else _registered_timeouts(obj, spec["event"], command)
        verify.append({"profile": row["profile"], "settings": sp, "state": state,
                       "action": row["action"], "timeouts": tos})
        print("%-22s %-12s %s" % (row["profile"], state, detail), file=out)
    print(json.dumps({"hook": hook_key, "event": spec["event"], "command": command,
                      "timeout": spec.get("timeout"),
                      "targets_table": (None if table is None
                                        else {"path": table["path"],
                                              "sha256": table["sha256"],
                                              "eligibility_key": HOOK_ELIGIBILITY_KEY[hook_key]}),
                      "apply": bool(apply_), "results": verify}, ensure_ascii=False), file=out)
    return rc, rows, command, spec


def emit_expected(profiles, hook_key, pack, path, apply_, out=None, table=None):
    """C73 기대 집합 마커(guard-hook-expected.json) 실물 생성 — OT-2 절차의 도구화.

    dry-run 이면 **내용을 출력만** 한다(파일을 만들지 않는다). 스키마는 preflight C73 이
    읽는 것과 동일: profiles[].{settings, sha256, must_contain}.

    ★E3-1(계약 §5-1 ③): 표가 있으면 `derived_from` 으로 파생원을 박제하고, 인자 대상이
    표의 allow 집합을 벗어나면 **loud 하게 적는다** — C73 기대표가 코드/대상표와 다시
    갈라지는 경로(표 셋이 어긋났던 R-04 원인)를 닫는다.
    """
    out = sys.stdout if out is None else out
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
    if table is not None:
        allow = [e["basename"] for e in _table_eligible(table, hook_key)]
        off = sorted({_profile_basename(_resolve_settings(p)) for p in profiles} - set(allow))
        doc["derived_from"] = {"hook_targets": table["path"], "sha256": table["sha256"],
                               "eligibility_key": HOOK_ELIGIBILITY_KEY[hook_key],
                               "allow_basenames": allow}
        if off:
            doc["derived_from"]["off_table_targets"] = off
            print("경고: 기대 표 대상 중 대상표 allow 집합 밖 %d건: %s "
                  "(--from-table 로 파생하면 이 갈라짐이 생기지 않는다)"
                  % (len(off), ", ".join(off)), file=out)
    body = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"
    if not apply_:
        print("\n── 기대 표(미기록 · --apply 시 %s 에 기록) ──\n%s" % (path, body), file=out)
        return
    _atomic_write(path, body)
    print("\n기대 표 기록: %s (프로필 %d)" % (path, len(entries)), file=out)


def emit_warn_targets(table, path, apply_, out=None):
    """C74 경고 훅 대상표(`state/brief-warn-expected.txt`) 파생 — 계약 §5-1 ③.

    preflight C74 가 읽는 형식 그대로: **프로필 dotdir basename 한 줄씩**(`#` 주석 허용).
    종전 `brief-warn-expected.txt.example` 의 손타이핑 3줄에는 `.claude-3` 가 빠져 있었고,
    그 표대로 등록하면 실제 master 세션에 경고가 도달하지 않았다(R-04(b)). 파생하면 그
    누락이 구조적으로 불가능해진다.
    """
    out = sys.stdout if out is None else out
    allow = _table_eligible(table, "brief-warn")
    # ★소비자 파서 계약(preflight C74): `names = (ln.strip() for ln in f if ln.strip()
    #   and not ln.startswith("#"))` — **줄 전체가 basename**이다. 행말 주석을 붙이면
    #   basename 이 "`.claude   # master…`" 가 돼 어떤 프로필과도 일치하지 않고, 그 결과
    #   "대상표는 있는데 아무것도 매칭되지 않는" 침묵 미배선이 된다. 주석은 **독립 줄만**.
    lines = ["# C74 경고 훅 대상표 — %s 에서 파생(sha256 %s)"
             % (table["path"], table["sha256"][:16]),
             "# 손으로 고치지 마라. 대상표를 고치고 --emit-warn-targets 로 다시 파생하라.",
             "# 생성: %s" % _now_tag()]
    for e in allow:
        lines.append("# %s — %s" % (e["basename"], (e.get("role") or "미기재")[:70]))
        lines.append(e["basename"])
    body = "\n".join(lines) + "\n"
    if not apply_:
        print("\n── C74 대상표(미기록 · --apply 시 %s 에 기록) ──\n%s" % (path, body), file=out)
        return
    _atomic_write(path, body)
    print("\nC74 대상표 기록: %s (프로필 %d)" % (path, len(allow)), file=out)


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

        wdir, wset = mkprof(".claude-worker",
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

        # ══ E1-2(R-01) settings 훅 항목 timeout 필드 ═════════════════════════════
        # ⑨ 등록된 Stop 항목에 timeout=60 이 실재하고 dry-run 출력에도 노출된다
        entry = None
        for g in _groups(json.load(open(wset, encoding="utf-8")), "Stop"):
            for h in g.get("hooks") or []:
                if str(h.get("command", "")).strip() == cmd:
                    entry = h
        chk(entry is not None and entry.get("timeout") == HOOKS["stop"]["timeout"],
            "⑨ 등록 항목 timeout 부재/불일치: %s" % entry)
        buf = io.StringIO()
        process([wdir], "stop", False, False, pack, out=buf)
        chk('"timeout": 60' in buf.getvalue() or "timeout: 60" in buf.getvalue(),
            "⑨ dry-run 출력에 timeout 미노출")
        # 경고 훅도 자기 상한(20)을 갖는다
        me = None
        for g in _groups(json.load(open(mset, encoding="utf-8")), "PostToolUse"):
            for h in g.get("hooks") or []:
                if str(h.get("command", "")).strip() == cmd2:
                    me = h
        chk(me is not None and me.get("timeout") == HOOKS["brief-warn"]["timeout"],
            "⑨ 경고 훅 timeout 부재/불일치: %s" % me)

        # ⑩ 결함 재현 — 종전 형태(timeout 미기재)로 등록된 프로필은 표에 불일치 + exit 1
        odir, oset = mkprof(".claude-legacy",
                            json.dumps({"hooks": {"Stop": [{"hooks": [
                                {"type": "command", "command": cmd}]}]}},
                                       ensure_ascii=False, indent=2) + "\n")
        o_before = open(oset, encoding="utf-8").read()
        buf = io.StringIO()
        rc, rows, _c, _s = process([odir], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET, "⑩ timeout 미기재 등록이 exit 0 으로 조용히 통과(rc=%s)" % rc)
        chk(rows[0].get("timeout_mismatch") == [None], "⑩ 불일치 미검출: %s" % rows[0])
        chk("timeout 불일치" in buf.getvalue(), "⑩ 표에 불일치 문면 부재")
        chk(open(oset, encoding="utf-8").read() == o_before,
            "⑩ --repair-timeout 없이 남의 등록을 고쳐 씀")

        # ⑪ --repair-timeout: dry-run 은 미기록, apply 는 정정 + 백업 + 재실행 시 일치
        buf = io.StringIO()
        rc, rows, _c, _s = process([odir], "stop", False, False, pack, out=buf,
                                   repair_timeout=True)
        chk(rows[0]["action"] == "WOULD-FIX-TIMEOUT", "⑪ dry-run action=%s" % rows[0]["action"])
        chk(open(oset, encoding="utf-8").read() == o_before, "⑪ dry-run 인데 파일이 변경됨")
        buf = io.StringIO()
        rc, rows, _c, _s = process([odir], "stop", True, False, pack, out=buf,
                                   repair_timeout=True)
        chk(rows[0]["action"] == "FIXED-TIMEOUT", "⑪ apply action=%s" % rows[0]["action"])
        chk(_registered_timeouts(json.load(open(oset, encoding="utf-8")), "Stop", cmd) == [60],
            "⑪ 정정 후에도 timeout 불일치")
        chk(len([f for f in os.listdir(odir) if ".bak-guard-" in f]) == 1, "⑪ 백업 미생성")
        buf = io.StringIO()
        rc, rows, _c, _s = process([odir], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"].startswith("ALREADY"),
            "⑪ 정정 후 재실행이 여전히 불일치(rc=%s·%s)" % (rc, rows[0]["action"]))

        # ══ E3-1(BLOCKER R-04 잔여) 대상표 소비 배터리 ═══════════════════════════
        # 실측 6프로필과 **같은 이름**의 픽스처(격리 tmpdir · 라이브 무접촉).
        live3, live3_set = mkprof(".claude-3", '{\n  "hooks": {}\n}\n')      # 라이브 master
        unk_dir, unk_set = mkprof(".claude-brandnew", '{\n  "hooks": {}\n}\n')  # 표 밖
        fleet_dir, fleet_set = mkprof("claude", '{\n  "hooks": {}\n}\n')     # ~/.cys/claude
        two_dir, _two_set = mkprof(".claude-2", '{\n  "hooks": {}\n}\n')     # unclear

        def mktable(body, name="hook-targets.json"):
            sd = os.path.join(pack, "state")
            os.makedirs(sd, exist_ok=True)
            p = os.path.join(sd, name)
            with open(p, "w", encoding="utf-8") as f:
                f.write(body if isinstance(body, str)
                        else json.dumps(body, ensure_ascii=False, indent=1))
            return p

        def ent(bn, gs, bw, role="worker", path=None):
            return {"basename": bn, "path": path or ("~/" + bn), "role": role,
                    "eligibility": {"guard_stop": gs, "brief_warn": bw},
                    "why": "self-test 픽스처", "measured": "self-test"}

        tbl_doc = {"schema_version": 1, "measured_at": "self-test",
                   "policy": {"unknown_profile": "deny",
                              "unknown_override_flag": "--force-unknown",
                              "master_guard_override_flag": "--force-master"},
                   "profiles": [
                       ent(".claude", "deny", "allow", "master"),
                       ent(".claude-3", "deny", "allow", "master(라이브 실측)",
                           path=os.path.join(td, ".claude-3")),
                       ent(".claude-worker", "allow", "allow", "worker",
                           path=os.path.join(td, ".claude-worker")),
                       ent("claude", "allow", "allow", "worker(함대 공용)",
                           path=os.path.join(td, "claude")),
                       ent(".claude-2", "deny", "deny", "unclear"),
                       ent(".claude-dept", "deny", "deny", "dept 발행 전용"),
                   ]}

        # ⑫ 최종 안전망(운영 표·배포 예시표 **둘 다** 부재 = 코드 하드코딩 폴백) — 이때만
        #    라이브 master `.claude-3` 에 stop 이 거부되지 않는다(종전 동작 보존).
        #    ★E4-1 이후 이 조건은 팩에서 `.example` 까지 지워야 성립한다(⑱ 참조).
        l3_before = open(live3_set, encoding="utf-8").read()
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", False, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"] == "WOULD-ADD",
            "⑫ 표 부재 폴백이 종전과 다름(기대 WOULD-ADD): rc=%s %s" % (rc, rows[0]["action"]))
        chk("하드코딩 폴백" in buf.getvalue(), "⑫ 폴백 상태 고지 부재(침묵 폴백 금지)")
        chk(open(live3_set, encoding="utf-8").read() == l3_before, "⑫ dry-run 인데 파일 변경")

        # ⑬ 표 공급 → 같은 명령이 **REFUSED**(rc=1) · 파일 무변. GAP 핵심 행 해소.
        tp = mktable(tbl_doc)
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "REFUSED",
            "⑬ .claude-3 + stop 이 거부되지 않음: rc=%s %s" % (rc, rows[0]["action"]))
        chk("guard_stop=deny" in rows[0]["note"], "⑬ 거부 사유에 표 근거 부재: %s" % rows[0]["note"])
        chk(open(live3_set, encoding="utf-8").read() == l3_before, "⑬ 거부인데 파일이 변경됨")
        chk(tp in buf.getvalue() and "sha256" in buf.getvalue(), "⑬ 표 출처·해시 미표기")
        # 훅별 분리 — 같은 프로필에 brief-warn 은 allow(집합을 합치지 않는다)
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "brief-warn", False, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"] == "WOULD-ADD",
            "⑬ .claude-3 + brief-warn 이 allow 가 아님: %s" % rows[0]["action"])

        # ⑭ 미지 프로필(표 밖) = deny-by-default · --force-unknown 으로만 통과
        buf = io.StringIO()
        rc, rows, _c, _s = process([unk_dir], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "REFUSED",
            "⑭ 표 밖 프로필이 거부되지 않음: %s" % rows[0]["action"])
        chk("--force-unknown" in rows[0]["note"], "⑭ 우회 플래그 안내 부재")
        buf = io.StringIO()
        rc, rows, _c, _s = process([unk_dir], "stop", False, False, pack, out=buf,
                                   force_unknown=True)
        chk(rc == EXIT_OK and rows[0]["action"] == "WOULD-ADD",
            "⑭ --force-unknown 인데 통과 안 함: %s" % rows[0]["action"])
        # basename 혼동 금지: `claude`(함대 공용) ≠ `.claude`(master)
        buf = io.StringIO()
        rc, rows, _c, _s = process([fleet_dir], "stop", False, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"] == "WOULD-ADD",
            "⑭ ~/.cys/claude(basename 'claude')가 '.claude' 로 오판정: %s" % rows[0]["action"])
        buf = io.StringIO()
        rc, rows, _c, _s = process([two_dir], "brief-warn", False, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "REFUSED",
            "⑭ .claude-2 brief-warn=deny 미집행: %s" % rows[0]["action"])
        # 표 deny 도 --force-master 로 명시 우회는 가능(운영 탈출구)
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", False, True, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"] == "WOULD-ADD", "⑭ --force-master 우회 불가")

        # ⑮ 표 손상 = 폴백 금지 · exit 2 · 쓰기 0(손상을 관대하게 접지 않는다)
        for bad, why in ((b"{ not json", "파싱"),
                         (json.dumps({"schema_version": 2, "profiles": [],
                                      "policy": {"unknown_profile": "deny"}}), "세대"),
                         (json.dumps({"schema_version": 1, "policy": {"unknown_profile": "deny"},
                                      "profiles": [{"basename": ".claude"}]}), "eligibility"),
                         (json.dumps({"schema_version": 1, "policy": {},
                                      "profiles": [ent(".claude", "deny", "allow")]}), "policy")):
            mktable(bad.decode() if isinstance(bad, bytes) else bad)
            buf = io.StringIO()
            rc, rows, _c, _s = process([wdir], "stop", True, False, pack, out=buf)
            chk(rc == EXIT_ARGS and rows == [],
                "⑮ 손상 표(%s)가 exit 2 로 멈추지 않음: rc=%s rows=%d" % (why, rc, len(rows)))
            chk("대상표 손상" in buf.getvalue(), "⑮ 손상 고지 부재(%s)" % why)
        mktable(tbl_doc)   # 정상 표 복구

        # ⑯ 파생 3표의 파생원이 하나 — C73 기대표 derived_from + C74 대상표
        tbl, terr = _load_targets(tp)
        chk(terr is None and tbl is not None, "⑯ 정상 표 로드 실패: %s" % terr)
        chk([e["basename"] for e in _table_eligible(tbl, "stop")]
            == [".claude-worker", "claude"], "⑯ guard allow 집합 파생 오류")
        chk([e["basename"] for e in _table_eligible(tbl, "brief-warn")]
            == [".claude", ".claude-3", ".claude-worker", "claude"],
            "⑯ brief-warn allow 집합 파생 오류(.claude-3 누락 = R-04(b) 재발)")
        exp2 = os.path.join(td, "guard-hook-expected-2.json")
        emit_expected([wdir], "stop", pack, exp2, True, out=io.StringIO(), table=tbl)
        d2 = json.load(open(exp2, encoding="utf-8"))
        chk(d2["derived_from"]["sha256"] == tbl["sha256"], "⑯ 기대표 derived_from 해시 불일치")
        chk("off_table_targets" not in d2["derived_from"], "⑯ allow 대상인데 off-table 표기")
        buf = io.StringIO()
        emit_expected([live3], "stop", pack, exp2, False, out=buf, table=tbl)
        chk("allow 집합 밖" in buf.getvalue(), "⑯ 표 밖 기대표 대상이 loud 하지 않음")
        wt = os.path.join(td, "brief-warn-expected.txt")
        emit_warn_targets(tbl, wt, False, out=io.StringIO())
        chk(not os.path.exists(wt), "⑯ dry-run 인데 C74 대상표가 기록됨")
        emit_warn_targets(tbl, wt, True, out=io.StringIO())
        # ★소비자(preflight C74)와 **동일한 파서**로 읽는다 — 관대한 파서로 읽으면
        #   행말 주석 같은 형식 결함이 여기서 통과하고 실배선에서만 터진다(실제로 1차
        #   구현이 그랬다: `%-24s # role` 형태 → basename 이 통째로 오염).
        with open(wt, encoding="utf-8") as _f:
            names = [ln.strip() for ln in _f if ln.strip() and not ln.startswith("#")]
        chk(names == [".claude", ".claude-3", ".claude-worker", "claude"],
            "⑯ C74 대상표 파생 오류(소비자 파서 기준): %r" % names)

        # ⑰ --from-table 파생 — 손타이핑 없이 allow 집합만 대상이 된다
        import contextlib as _cl

        def run_main(argv):
            b = io.StringIO()
            try:
                with _cl.redirect_stdout(b), _cl.redirect_stderr(b):
                    return main(argv), b.getvalue()
            except SystemExit as e:                    # argparse ap.error()
                return (e.code if isinstance(e.code, int) else EXIT_ARGS), b.getvalue()

        rc, o = run_main(["--hook", "stop", "--from-table", "--pack", pack,
                          "--hook-targets", tp])
        chk(rc == EXIT_OK, "⑰ --from-table dry-run rc=%s" % rc)
        chk(".claude-worker" in o and "claude" in o and ".claude-3" not in o,
            "⑰ --from-table 파생 대상 오류(deny 프로필 혼입/allow 누락)")
        rc, _o = run_main(["--hook", "stop", "--from-table", "--profile", wdir,
                           "--pack", pack, "--hook-targets", tp])
        chk(rc == EXIT_ARGS, "⑰ --from-table 과 --profile 병용이 거부되지 않음: rc=%s" % rc)
        rc, o = run_main(["--hook", "stop", "--profile", live3, "--pack", pack,
                          "--hook-targets", tp])
        chk(rc == EXIT_TARGET and "REFUSED" in o,
            "⑰ CLI 경로에서 .claude-3+stop 이 거부되지 않음: rc=%s" % rc)

        # ══ E4-1(사인오프 R-04 잔여 HIGH) 배포 기본 상태 = `.example` 폴백 ═════════════
        # install 직후 실물은 `hook-targets.json.example` 뿐이다. 그 상태에서 기계 방어가
        # 죽어 있으면(WOULD-ADD·rc=0) R-04 원 결함이 배포 기본값으로 되살아난다.
        expath = mktable(tbl_doc, name="hook-targets.json.example")
        os.remove(tp)                                       # 운영 표 제거 = install 직후
        chk(not os.path.exists(tp) and os.path.exists(expath), "⑱ 픽스처 구성 실패")
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "REFUSED",
            "⑱ **배포 기본 상태(.example 만 존재)에서 .claude-3+stop 이 통과** — R-04 재발: "
            "rc=%s %s" % (rc, rows[0]["action"]))
        chk("guard_stop=deny" in rows[0]["note"], "⑱ 예시표 폴백 거부 사유에 표 근거 부재")
        chk(open(live3_set, encoding="utf-8").read() == l3_before, "⑱ 거부인데 파일이 변경됨")
        chk("배포 예시표 폴백" in buf.getvalue(), "⑱ 폴백 출처 고지 부재(침묵 폴백 금지)")
        chk("cp " in buf.getvalue() and "hook-targets.json.example" in buf.getvalue(),
            "⑱ 운영 표 설치 안내(cp) 부재")
        chk("하드코딩 폴백" not in buf.getvalue(), "⑱ 예시표가 있는데 하드코딩 폴백으로 고지")
        # 훅별 분리·allow 프로필은 예시표 폴백에서도 종전 그대로
        buf = io.StringIO()
        rc, rows, _c, _s = process([wdir], "stop", False, False, pack, out=buf)
        chk(rc == EXIT_OK and rows[0]["action"].startswith("ALREADY"),
            "⑱ 예시표 폴백에서 워커 프로필이 막힘: %s" % rows[0]["action"])
        # CLI 경로 + --from-table 도 예시표에서 파생된다(운영 표 부재여도 파생원 존재)
        rc, o = run_main(["--hook", "stop", "--profile", live3, "--pack", pack])
        chk(rc == EXIT_TARGET and "REFUSED" in o,
            "⑱ CLI 경로 예시표 폴백이 거부하지 않음: rc=%s" % rc)
        rc, o = run_main(["--hook", "stop", "--from-table", "--pack", pack])
        chk(rc == EXIT_OK and ".claude-worker" in o and ".claude-3" not in o,
            "⑱ --from-table 이 예시표에서 파생되지 않음: rc=%s" % rc)
        # 예시표 손상도 폴백 금지(exit 2·쓰기 0) — 배포물 손상을 관대하게 접지 않는다
        mktable("{ not json", name="hook-targets.json.example")
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_ARGS and rows == [], "⑱ 손상 예시표가 exit 2 로 멈추지 않음: rc=%s" % rc)
        chk("대상표 손상(배포 예시표)" in buf.getvalue(), "⑱ 예시표 손상 출처 미표기")
        # 운영 표가 있으면 예시표 손상은 무관(우선순위: 운영 표 > 예시표)
        mktable(tbl_doc)
        buf = io.StringIO()
        rc, rows, _c, _s = process([live3], "stop", True, False, pack, out=buf)
        chk(rc == EXIT_TARGET and rows[0]["action"] == "REFUSED",
            "⑱ 운영 표 존재 시에도 손상 예시표가 판정을 오염시킴: rc=%s" % rc)
        os.remove(expath)

    if fails:
        print("javis_guard_register self-test FAIL %d건:" % len(fails), file=sys.stderr)
        for f in fails:
            print("  - " + f, file=sys.stderr)
        return 1
    print("javis_guard_register self-test OK — dry-run 기본·apply 등록/백업·멱등 바이트동일·"
          "역할 경계 거부·경고훅 master 허용+matcher·손상 무파괴·기대 표 스키마·자동발견 0"
          " · E1-2(R-01) timeout 필드: 신규 등록 stop=60/brief-warn=20·dry-run 노출·"
          "종전 미기재 등록 검출 exit 1(무단 수정 0)·--repair-timeout dry/apply·정정 후 일치"
          " · E3-1(R-04) 대상표 소비: 부재=폴백(결함 재현 WOULD-ADD)·공급 시 .claude-3+stop"
          " REFUSED·훅별 분리(brief-warn allow)·미지 deny-by-default+--force-unknown·"
          "basename 'claude'≠'.claude'·손상 표 exit 2 무폴백 4종·C73/C74 파생원 단일화·"
          "--from-table 파생·CLI 경로 재확인"
          " · E4-1(R-04 배포 기본값) 예시표 폴백: `.example` 만 있는 install 직후 상태에서 "
          ".claude-3+stop REFUSED(기계 방어 생존)·폴백 출처+cp 설치 안내 고지·워커 프로필 "
          "무영향·CLI/--from-table 동일·예시표 손상 exit 2 무폴백·운영 표 우선")
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
                    help="역할 경계 우회(대상표 deny 프로필에 등록) — 의도 명시용")
    ap.add_argument("--force-unknown", action="store_true",
                    help="대상표에 없는 미지 프로필 등록 우회(E3-1 · deny-by-default 해제)")
    ap.add_argument("--hook-targets", metavar="PATH", default=None,
                    help="대상표 경로 override(기본 <pack>/state/hook-targets.json)")
    ap.add_argument("--from-table", action="store_true",
                    help="대상 프로필을 대상표의 allow 집합에서 파생(--profile 대신 · E3-1)")
    ap.add_argument("--emit-warn-targets", metavar="PATH", default=None,
                    help="C74 경고 훅 대상표(brief-warn-expected.txt)를 대상표에서 파생")
    ap.add_argument("--repair-timeout", action="store_true",
                    help="이미 등록된 항목의 timeout 필드를 카탈로그 값으로 정정(E1-2/R-01 · "
                         "--apply 와 함께여야 실제로 쓴다). 미지정 시 불일치는 표에 적고 exit 1")
    ap.add_argument("--pack", default=None, help="팩 경로(기본 CYS_PACK_DIR→~/.cys/pack)")
    ap.add_argument("--emit-expected", metavar="PATH", default=None,
                    help="C73 기대 집합 마커 생성(dry-run 이면 내용 출력만)")
    ap.add_argument("--self-test", action="store_true", help="격리 tmpdir 자기검증")
    a = ap.parse_args(argv)

    if a.self_test:
        return self_test()
    pack = a.pack or _pack_dir()
    # ★E4-1: 운영 표 → 배포 예시표 → 하드코딩 3단(손상은 어느 단계든 폴백 없이 exit 2).
    table, terr, tpath, tsource = _resolve_targets(pack, a.hook_targets)
    if terr:
        print("대상표 손상%s: %s — %s"
              % ("(배포 예시표)" if tsource == "example" else "", tpath, terr), file=sys.stderr)
        print("→ 등록 중단(쓰기 0). 표를 고치거나 지워라(운영 표를 지우면 배포 예시표 폴백, "
              "예시표까지 없으면 하드코딩 폴백 · 다른 표는 --hook-targets).", file=sys.stderr)
        return EXIT_ARGS

    profiles = list(a.profile)
    if a.from_table:
        # ★E3-1: 대상 목록도 표에서 파생 — 손타이핑 누락(R-04(b))을 구조적으로 제거한다.
        if table is None:
            ap.error("--from-table 인데 대상표가 없다(%s · 배포 예시표 %s%s 도 부재) — "
                     "표를 공급하거나 --profile 을 써라"
                     % (tpath, tpath, TARGETS_EXAMPLE_SUFFIX))
        if a.profile:
            ap.error("--from-table 과 --profile 은 함께 쓰지 않는다(파생원이 둘이 된다)")
        elig = _table_eligible(table, a.hook)
        profiles = [os.path.expanduser(e.get("path") or ("~/" + e["basename"])) for e in elig]
        print("--from-table: %s 에서 %s=allow %d개 파생 — %s"
              % (tpath, HOOK_ELIGIBILITY_KEY[a.hook], len(profiles),
                 ", ".join(e["basename"] for e in elig)))
    if not profiles:
        ap.error("--profile 이 최소 1개 필요하다 — 이 도구는 프로필을 스스로 찾지 않는다"
                 "(home-glob 일괄 등록 금지 · 조건 24). 표 기반 파생은 --from-table")

    rc, _rows, _cmd, _spec = process(profiles, a.hook, a.apply, a.force_master, pack,
                                     repair_timeout=a.repair_timeout,
                                     force_unknown=a.force_unknown,
                                     targets_path=a.hook_targets, table=table,
                                     table_source=tsource)
    if a.emit_expected:
        emit_expected(profiles, a.hook, pack, a.emit_expected, a.apply, table=table)
    if a.emit_warn_targets:
        if table is None:
            print("\nC74 대상표 파생 불가: 대상표 부재(%s) — 파생원이 없다" % tpath)
            rc = rc or EXIT_TARGET
        else:
            emit_warn_targets(table, a.emit_warn_targets, a.apply)
    if not a.apply:
        print("\n※ DRY-RUN 이었다 — 실제 등록은 --apply 이며, 라이브 등록은 [OT-2] 오너 승인 라인이다.")
    return rc


if __name__ == "__main__":
    sys.exit(main())
