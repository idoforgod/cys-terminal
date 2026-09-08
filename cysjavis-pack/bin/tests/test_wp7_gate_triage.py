#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_wp7_gate_triage.py — P5-WP7-gate 독립 재유도(triage) 회귀 핀 (0.14.31).

이 파일은 **판정자(독립 재유도)** 가 남긴 핀이다. 잔여 지적 5건을 각각 HEAD 에서 실패하는
최소 검체로 옮겨 놓았다. 통과하면 그 지적이 수리된 것이고, 실패하면 아직 살아 있는 것이다.

핀 목록
  ① 래치 병합 쓰기 경합 — 재읽기~`os.replace` 사이에 남의 `expired=True` 가 끼어들면
     오래된 `expired=False` 가 그것을 덮는다(codex blocking). 결정론적 인터리브로 잰다.
  ② 소유권 오탐 — 열거하지 않은 런타임 값 옵션의 **값**과 반복 하위명령이 함대 실행 주체로
     승격된다(codex major · B2 계열 과대계상).
  ③ CI 레인 등재 — WP-7 이 만든/옮긴 검체와 게이트 self-test 가 3레인 중 **0레인**에서 돈다
     (reviewer-claude major · codex major · "안 도는 검체는 게이트가 아니다").
  ④ 발행 차단 게이트 — WP-7 이 만진 파일이 `scripts/secret-scan.sh` 에 개인경로로 걸린다
     (reviewer-claude major · H-SECRET-1).
  ⑤ ★R2 수렴 — 소유권 **리콜**(미계상 방향의 과교정). 기준선(2008b063)이 옳게 세던 실형상이
     HEAD 에서 사라져 함대 CPU 합이 0 이 되고 hard 가 **안 걸린다**(reviewer-codex major ·
     reviewer-claude minor). 방향은 '막는 쪽' 이 아니라 '안 막는 쪽' 이라 축이 조용히 얇아진다.
  ⑥ ★R2 수렴 — 만료 표식 되읽기가 `held` 갈래에만 있고 `arm` 갈래에 없다(양 리뷰어 minor).
  ⑦ ★R2 수렴 — 만료 표식 **생성 실패**가 사유에 안 남는다(양 리뷰어 minor · 침묵 금지 규율).

밀폐: 게이트는 `CYS_STATE_DIR` 임시 디렉터리 · 소켓 env 제거 · 라이브 ps/데몬 무접촉(스폰 0).
실행 규약: CYS_PACK_DIR="$(mktemp -d)" JAVIS_ROOT="$(mktemp -d)" python3 bin/tests/test_wp7_gate_triage.py
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 WP7-GATE-TRIAGE-OK.

★워커 수정 1건(판정 의미 무관): 음성대조 명령줄의 `/Users/u/` → `/Users/user/`. `u` 는
  `scripts/secret-scan.sh` 의 `dummy_names` 가 아니라 이 파일 자체가 `--all` 스캔(전 레인 발행
  게이트)에서 2건으로 걸렸다 — 핀 ④가 막으려는 바로 그 회귀를 핀 자신이 만들던 상태다.
  소유권 판정은 basename(`codex`)만 보므로 username 은 결과에 관여하지 않는다.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
ROOT = os.path.dirname(PACK)

_TMP = tempfile.mkdtemp(prefix="wp7-triage-")
os.environ["CYS_STATE_DIR"] = os.path.join(_TMP, "state")
os.environ.setdefault("CYS_PACK_DIR", os.path.join(_TMP, "pack"))
os.environ.pop("CYS_SOCKET", None)
os.environ.pop("CYS_GATE_LANE_SOCKET", None)
sys.path.insert(0, BIN)

import javis_resource_gate as G      # noqa: E402

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


# ── ① 래치 병합 쓰기 경합(codex blocking) ────────────────────────────────────
# 저장 상태: since=1000 부터 연속 hard. A 는 now=1899(미만료) · B 는 now=1900(만료).
# B 가 A 의 **재읽기와 교체 사이**에 끼어들면 A 의 오래된 expired=False 가 B 의 공개된
# 만료를 덮는다. 그 뒤 시계 역행이 오면 `below` 관측 없이 재무장해 899초를 다시 막는다.
path = G._fleet_hold_path(None)
os.makedirs(os.path.dirname(path), exist_ok=True)
G._fleet_hold_write(path, {"since": 1000.0, "last": 1000.0, "expired": False})

_real_replace = os.replace
_st = {"fired": False, "b": None}


def _hooked_replace(src, dst):
    if not _st["fired"] and str(dst) == path:
        _st["fired"] = True
        _st["b"] = G._fleet_hard_hold("hard", now=1900.0)   # 다른 호출이 만료를 공개한다
    return _real_replace(src, dst)


os.replace = _hooked_replace
try:
    a = G._fleet_hard_hold("hard", now=1899.0)
finally:
    os.replace = _real_replace

b = _st["b"]
check("1a-경합-B가만료를공개", bool(b) and b[2] is True and b[1].startswith("expired"),
      "B=%r" % (b,))
rec, why = G._fleet_hold_read(path)
check("1b-공개된만료가영속한다", bool(rec) and rec["expired"] is True,
      "저장값=%r(why=%r) · A=%r — 오래된 expired=False 가 덮었다" % (rec, why, a))
c = G._fleet_hard_hold("hard", now=997.0)      # 시계 역행
check("1c-역행이재무장을부르지않는다", c[1] != "armed" and c[2] is True,
      "C=%r — below 관측 없이 재무장하면 만료가 취소된다" % (c,))
d = G._fleet_hard_hold("hard", now=1896.0)
check("1d-차단이다시열리지않는다", d[2] is True,
      "D=%r — 공개된 완화 뒤 899초 재차단" % (d,))

# ── ② 소유권 오탐(codex major · B2 계열) ─────────────────────────────────────
# 열거하지 않은 런타임 값 옵션의 **값**과 반복 하위명령이 실행 주체로 승격되면, 우리와
# 무관한 프로세스의 CPU 가 함대 합에 실려 기동 거부(hard)를 만든다.
_FP = [
    ("node --diagnostic-dir /tmp/codex /tmp/report.js", "미열거 node 값 옵션의 값"),
    ("node --cpu-prof-dir /tmp/codex /tmp/report.js", "node 값 옵션의 값(표 등재분)"),
    ("node --test-reporter-destination /tmp/serena /tmp/x.js", "node 값 옵션의 값(표 등재분)"),
    # ★R2 수렴: 위 세 이름은 이제 `_JS_VALUE_LONG` 에 있다(리콜 복원). '표에 아예 없는 이름' 의
    #   음성 대조를 따로 둔다 — 미지 옵션 뒤 토큰은 여전히 소유권 근거가 아니어야 한다.
    ("node --zzz-not-a-real-flag /tmp/codex /tmp/report.js", "표에 없는 옵션의 뒤 토큰"),
    ("python3 --zzz-not-a-real-flag /tmp/serena /tmp/x.py", "표에 없는 옵션의 뒤 토큰"),
    ("node --run=build /tmp/codex", "= 형이라도 실행 모드를 바꾸는 이름"),
    ("uv run --script=/tmp/x.py serena", "런처 실행 자리를 옮기는 = 형"),
    ("npx -p @openai/codex node /tmp/report.js", "런처 짧은 값 옵션(-p)의 값"),
    ("npm exec exec codex", "허용 하위명령 반복 소비"),
    ("uv run run codex", "허용 하위명령 반복 소비"),
]
for cmd, tag in _FP:
    got = G._fleet_owner(cmd)
    check("2-미계상:%s" % tag, got is None, "%r → %r (실행 대상이 아니다)" % (cmd, got))
# 음성 대조(계측 타당성): 진짜 형상은 계속 잡혀야 한다 — 미계상 방향으로 과교정하면 축이 죽는다.
for cmd, want in (("node /Users/user/.local/bin/codex exec", "codex"),
                  ("uv run serena start-mcp-server", "serena"),
                  ("node --require /x/pre.js /Users/user/.local/bin/codex", "codex")):
    check("2-음성대조:%s" % want, G._fleet_owner(cmd) == want, "%r → %r" % (cmd, G._fleet_owner(cmd)))

# ── ③ CI 레인 등재(reviewer-claude/codex major) ──────────────────────────────
_LANES = {
    "ci-branch": os.path.join(ROOT, ".github/workflows/ci-branch.yml"),
    "release": os.path.join(ROOT, ".github/workflows/release.yml"),
    "pack-release": os.path.join(ROOT, ".github/workflows/pack-release.yml"),
}
_lane_txt = {}
for lane, p in _LANES.items():
    try:
        _lane_txt[lane] = "".join(re.sub(r"#.*$", "", ln) for ln in
                                  open(p, encoding="utf-8").read().splitlines(True))
    except OSError as e:
        _lane_txt[lane] = ""
        check("3-레인파일판독:%s" % lane, False, str(e))

for suite in ("test_resource_gate", "test_resource_gate_fleet_cpu", "test_inject_context_role_seat"):
    absent = sorted(l for l, t in _lane_txt.items() if not re.search(r"\b%s\b" % suite, t))
    check("3-등재:%s" % suite, not absent, "미등재 레인=%s (등재 0 = CI 실행 0회)" % (absent or "없음"))
# 게이트 self-test 도 어느 레인에서도 돌지 않는다 — R2 수리의 핀 다수가 그 안에만 있다.
absent = sorted(l for l, t in _lane_txt.items() if "javis_resource_gate.py" not in t)
check("3-등재:javis_resource_gate --self-test", not absent, "미등재 레인=%s" % (absent or "없음"))

# ── ④ 발행 차단 게이트(reviewer-claude major · H-SECRET-1) ───────────────────
_SCAN = os.path.join(ROOT, "scripts", "secret-scan.sh")
_TARGETS = ["cysjavis-pack/bin/javis_resource_gate.py",
            "cysjavis-pack/hooks/inject-context.sh",
            "cysjavis-pack/bin/tests/test_inject_context_role_seat.py"]
if not os.path.isfile(_SCAN):
    check("4-secret-scan존재", False, _SCAN)
else:
    r = subprocess.run(["bash", _SCAN] + _TARGETS, cwd=ROOT, capture_output=True, text=True)
    hits = [ln for ln in (r.stdout or "").splitlines()
            if ln.startswith(("PATH\t", "WIN-PATH\t", "PROFILE\t", "EMAIL\t", "SECRET\t", "HANDLE\t"))]
    check("4-WP7파일이발행게이트를통과한다", r.returncode == 0,
          "rc=%d · 적발 %d건 (예: %s)" % (r.returncode, len(hits),
                                          (hits[0][:120] if hits else "-")))

# ── ⑤ 소유권 리콜(reviewer-codex major · reviewer-claude minor) ─────────────
# 기준선이 세던 실형상이 HEAD 에서 None 이 되면 그 프로세스의 CPU 가 함대 합에서 빠진다 —
# **hard 가 안 걸리는** 방향이라 조용하다. 아래는 두 리뷰어가 실측으로 지목한 전 형상이다.
_RECALL = [
    # reviewer-codex 표 6행 (기준선 대조 실측)
    ("node -- /opt/bin/codex exec", "codex"),
    ("python3 -- /opt/bin/serena", "serena"),
    ("node --inspect /opt/bin/codex exec", "codex"),
    ("npm exec --package @openai/codex codex", "codex"),
    ("uv run --no-progress serena start-mcp-server", "serena"),
    ("bun --smol /opt/bin/codex exec", "codex"),
    # reviewer-claude 표 (= 형 거부의 순손실분)
    ("node --max-http-header-size=16384 /x/bin/codex", "codex"),
    ("node --dns-result-order=ipv4first /x/bin/codex", "codex"),
    ("node --unhandled-rejections=strict /x/bin/claude", "claude"),
    ("node --stack-size=4000 /x/bin/claude", "claude"),
    ("node --experimental-strip-types /x/bin/codex", "codex"),
    ("uv run --active serena", "serena"),
    ("npx --loglevel=silly codex", "codex"),
    ("npx --no-audit codex", "codex"),
    ("yarn dlx --quiet codex", "codex"),
    ("pnpm dlx --reporter=silent codex", "codex"),
    ("bunx --version codex", "codex"),
    # 표 등재의 부수 이득 — 값 옵션의 값을 건너뛰고 **그 뒤의 진짜 실행 대상**을 되찾는다.
    ("node --diagnostic-dir /tmp/x /x/bin/codex", "codex"),
    ("node --unknown-but-self-contained=1 /x/bin/codex", "codex"),
]
for cmd, want in _RECALL:
    got = G._fleet_owner(cmd)
    check("5-리콜:%s" % cmd.split()[1], got == want,
          "%r → %r (기대 %r · 미계상은 hard 가 안 걸리는 방향)" % (cmd, got, want))

# ★막는 방향의 최종 핀(codex 실패 시나리오 그대로): 전 코어를 태우는 함대 1행이
#   '측정 성공한 0%' 로 보고되면 hard/soft 보류가 영영 무장하지 않는다.
_LINES = ["    7 100.0 node -- /opt/bin/codex exec"]
_tot, _why = G._fleet_cpu_percent(_LINES, self_pid=999999)
check("5-포화행이합에실린다", _tot == 100.0 and _why is None,
      "%r/%r — 100%% 를 태우는 함대 행이 0 으로 보고되면 hard 가 안 걸린다" % (_tot, _why))

# ★표 정합 — 표를 늘리는 수리의 자기 함정(같은 이름이 두 통에 들어가면 판정이 이름 순서에
#   좌우된다). 겹침은 코드가 아니라 표의 결함이라 여기서 기계로 막는다.
for _k, _o in G.FLEET_RUNNER_OPTS.items():
    _ov = ((_o["value"] & _o["bool"]) | (_o["value"] & _o["abort"]) | (_o["bool"] & _o["abort"]))
    check("5-표겹침:%s" % _k, not _ov, "겹침=%s" % sorted(_ov))
check("5-표겹침:JS", not (G._JS_VALUE_LONG & G._JS_BOOL_LONG),
      sorted(G._JS_VALUE_LONG & G._JS_BOOL_LONG))
check("5-표겹침:PY", not (G._PY_VALUE_LONG & G._PY_BOOL_LONG),
      sorted(G._PY_VALUE_LONG & G._PY_BOOL_LONG))
check("5-표겹침:모드변경", not (G._LONG_MODE_CHANGE_FLAGS & (G._JS_VALUE_LONG | G._JS_BOOL_LONG)),
      "모드변경 이름이 표에 있으면 `=` 형 거부가 무력해진다")

# ── ⑥ 만료 표식 되읽기가 arm 갈래에도 있다 ──────────────────────────────────
# 저장 상태 since=1000. A 는 now=997(FUTURE_SLACK 초과 역행) → **재무장 갈래**로 들어간다.
# A 의 `os.replace` 직전에 B(now=1900)가 만료를 공개하면, A 의 반환도 만료여야 한다
# ('만료는 모든 호출자에게 동일하게 보인다' 는 이 수리의 계약 · 갈래별 구멍 금지).
_p6 = G._fleet_hold_path(6.0)
os.makedirs(os.path.dirname(_p6), exist_ok=True)
G._fleet_hard_hold("below", now=1000.0, thr=6.0)          # 표식·레코드 정리
G._fleet_hold_write(_p6, {"since": 1000.0, "last": 1000.0, "expired": False})
_st6 = {"fired": False, "b": None}


def _hooked6(src, dst):
    if not _st6["fired"] and str(dst) == _p6:
        _st6["fired"] = True
        _st6["b"] = G._fleet_hard_hold("hard", now=1900.0, thr=6.0)
    return _real_replace(src, dst)


os.replace = _hooked6
try:
    a6 = G._fleet_hard_hold("hard", now=997.0, thr=6.0)
finally:
    os.replace = _real_replace
check("6a-B가만료를공개", bool(_st6["b"]) and _st6["b"][2] is True, "B=%r" % (_st6["b"],))
check("6b-arm갈래도같은만료를본다", a6[2] is True and a6[1] != "armed",
      "A=%r — 파일=만료인데 반환만 armed/False 면 그 호출은 hard 를 다시 본다" % (a6,))
G._fleet_hard_hold("below", now=2000.0, thr=6.0)

# ── ⑦ 표식 생성 실패는 사유로 들린다 ────────────────────────────────────────
# 상태 디렉터리가 쓰기 불가(ENOSPC/EACCES)면 표식이 안 서고 만료가 가변 레코드 비트로만 남는다 —
# 이 라운드가 고친 잃어버린 갱신 경합이 되살아난 상태다. 그 사실이 사유에 남아야 한다.
_p7 = G._fleet_hold_path(7.0)
os.makedirs(os.path.dirname(_p7), exist_ok=True)
G._fleet_hard_hold("below", now=1000.0, thr=7.0)
G._fleet_hold_write(_p7, {"since": 1000.0, "last": 1000.0, "expired": False})
_real_open = os.open


def _hooked_open(p, flags, *a, **k):
    if str(p).endswith(".expired"):
        raise OSError(28, "No space left on device")
    return _real_open(p, flags, *a, **k)


os.open = _hooked_open
try:
    r7 = G._fleet_hard_hold("hard", now=1000.0 + G.FLEET_CPU_HARD_MAX_HOLD_SECS + 1, thr=7.0)
finally:
    os.open = _real_open
check("7a-만료는그대로공개된다", r7[2] is True, "R=%r" % (r7,))
check("7b-표식생성실패가사유에남는다", r7[1].startswith("expired_unmarked"),
      "R=%r — `below` 의 삭제 실패는 clear_failed 로 남는데 생성 실패만 침묵할 수 없다" % (r7,))
check("7c-표식이서면사유는평범한expired",
      G._fleet_hard_hold("hard", now=1000.0 + G.FLEET_CPU_HARD_MAX_HOLD_SECS + 2,
                         thr=7.0)[1] == "expired")
G._fleet_hard_hold("below", now=99000.0, thr=7.0)

shutil.rmtree(_TMP, ignore_errors=True)
if fails:
    print("\n실패 %d건: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nWP7-GATE-TRIAGE-OK")
