#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_refl_fleet_latch_gen.py — 성찰 R4 N2·N4·N15: 함대 CPU hard 래치의 세대·대체 자리·부트 유예.

무엇을 막는가
  ① N2 [blocking] **두 파일 래치 해제가 다음 포화 구간을 즉시 면제**했다(hard→soft).
     종전 사슬: `below` 가 본체를 지운 **뒤** 만료 표식을 지우는 사이에 들어온 hard 호출이
     남은 표식을 보고 '만료된 본체' 를 되만들고 → `below` 가 표식을 마저 지우면 → **다음 포화가
     처음부터 soft(권고)로 시작**한다. 즉 한 번 포화를 겪은 레인은 그 뒤 15분 유계를 통째로
     건너뛴다(§7 ① 방향 — 전 코어를 태우는 중에도 편성이 계속 스폰).
     수리: 본체·표식·묘비를 **포화 세대**(`gen`)로 묶고, `below` 는 본체 삭제 **전에** 묘비를
     세운다. 끝난 세대의 표식은 아무것도 되살리지 못한다.
  ② N2 반대 방향(과교정 금지): **같은 포화의 확정 만료는 취소되지 않는다** — 시계 역행이나
     후속 호출이 이미 공개된 만료를 되돌리면 899초가 다시 막힌다(봉인표 ③ 역행).
  ③ N15 [minor] 부트 유예 300초가 hard 보류 상한 900초를 **같이 태웠다** — 유예 창은 게이트가
     아무것도 막지 않는 구간인데 래치의 `since` 는 그동안 흘러서 실제 차단 가능 시간이 600초로
     줄었다. 수리: 유예 안의 관측은 `since` 를 지금으로 다시 놓고 사유에 `boot_grace` 를 남긴다.
  ④ N4 [major] 상태 디렉터리 기록 불능이면 hard 가 **첫 관측에서 soft 로 강등**됐다(`unbounded_io`
     = 즉시 만료). 같은 릴리스가 `load_ratio` 의 hard 를 뺐으므로 CPU hard 축이 하나도 남지
     않는다. 발화 조건은 특별하지 않다(읽기전용 마운트·다른 uid 소유·디스크 만석).
     수리: 대체 래치 자리(시스템 임시 디렉터리)로 유계를 잇고(`latch=tmp`), 둘 다 못 쓰면 종전
     정책(만료)으로 접되 **`measure_errors: fleet_cpu(latch)`** 로 조용한 완화를 막는다.

밀폐: 모든 상태는 임시 `CYS_STATE_DIR`/`CYS_PACK_DIR`/`TMPDIR` 안에서만 만들어진다.
      `~/.cys`·`~/.local/state`·라이브 데몬·라이브 소켓 무접촉(`CYS_SOCKET` 제거).
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 REFL-FLEET-LATCH-GEN-OK.
실행: python3 cysjavis-pack/bin/tests/test_refl_fleet_latch_gen.py
"""
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
GATE = os.path.join(BIN, "javis_resource_gate.py")

_ROOT = tempfile.mkdtemp(prefix="refl-latch-")
os.environ["CYS_STATE_DIR"] = os.path.join(_ROOT, "state")
os.environ.setdefault("CYS_PACK_DIR", os.path.join(_ROOT, "pack"))
os.environ.pop("CYS_SOCKET", None)          # 레인 키가 라이브 소켓을 가리키지 않게
sys.path.insert(0, BIN)

import javis_resource_gate as G          # noqa: E402

fails = []
ROSTER = '{"active":0,"seats":0,"errors":[],"depts":[]}'
HOLD_MAX = G.FLEET_CPU_HARD_MAX_HOLD_SECS       # 900.0
GRACE = G.BOOT_GRACE_SECS                       # 300.0


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def lane(tag):
    """새 레인(빈 상태 디렉터리) → 래치 본체 경로. 각 단계가 남의 잔재를 보지 않게 한다."""
    d = os.path.join(_ROOT, "lane-" + tag)
    os.makedirs(d, exist_ok=True)
    os.environ["CYS_STATE_DIR"] = d
    return G._fleet_hold_path(None)


def hold(state, now, grace=False):
    return G._fleet_hard_hold(state, None, now, None, grace)


def gen_of(path):
    rec, _why = G._fleet_hold_read(path, ended_ok=True)
    return None if rec is None else rec["gen"]


try:
    # ── ① N2: below 의 삭제 창에 끼어든 hard 가 '남의 만료' 로 본체를 되살리지 못한다 ──────────
    path = lane("n2-interleave")
    t0 = 1000.0
    hold("hard", t0)                                     # 무장(세대 G1)
    g1 = gen_of(path)
    check("1a 첫 hard 관측이 세대를 세운다", G._fleet_gen_ok(g1 or ""), "gen=%r" % g1)
    h, why, exp = hold("hard", t0 + HOLD_MAX + 1)         # 상한 초과 → 만료 공개
    check("1b 상한 초과에서 만료가 공개된다", exp is True and why.startswith("expired"),
          "hold=%r reason=%r" % (h, why))
    check("1c 그 만료 표식은 **그 세대의 것**이다",
          os.path.exists(G._fleet_expired_mark_path(path, g1)))

    # `below` 의 내부 순서(묘비 → 본체 삭제 → 표식 삭제) 한가운데에서 hard 호출을 실행한다.
    # 종전 판본은 이 자리에서 '만료된 본체' 가 되살아나 다음 포화가 즉시 soft 였다.
    fired = {}
    _real_remove = G.os.remove

    def _spy_remove(p):
        _real_remove(p)
        if str(p) == path and "hit" not in fired:
            fired["hit"] = "pending"                      # 재진입 방지 먼저
            fired["hit"] = hold("hard", t0 + HOLD_MAX + 2)

    G.os.remove = _spy_remove
    try:
        below = hold("below", t0 + HOLD_MAX + 3)
    finally:
        G.os.remove = _real_remove
    check("1d below 의 본체 삭제 창에서 hard 호출이 실제로 끼어들었다(계측 타당성)",
          fired.get("hit") not in (None, "pending"), "fired=%r" % (fired.get("hit"),))
    check("1e ★그 hard 는 만료가 아니라 **새 무장**이다(다음 포화가 hard 로 시작)",
          fired.get("hit", (None, None, None))[2] is False
          and fired.get("hit", (None, None, None))[1] == "armed",
          "interleaved=%r" % (fired.get("hit"),))
    g2 = gen_of(path)
    check("1f 되살아난 것이 아니라 **다른 세대**다", g2 is not None and g2 != g1,
          "g1=%r g2=%r" % (g1, g2))
    check("1g below 자체는 성공으로 끝난다", below[1] == "cleared", "below=%r" % (below,))
    h2, why2, exp2 = hold("hard", t0 + HOLD_MAX + 4)
    check("1h 새 포화의 후속 관측도 hard 다(즉시 면제 없음)",
          exp2 is False and why2 in ("held", "armed"), "hold=%r reason=%r" % (h2, why2))

    # ★음성 대조(계측 타당성): 세대가 **끝나지 않았으면** 표식만 남아도 만료는 그대로 되살아난다 —
    #   이 검체가 통과하는 이유가 '되살림을 통째로 없앴기 때문' 이 아님을 보인다.
    path = lane("n2-lone-mark")
    G._fleet_expired_mark_set(path, "legacy")             # 구 판 표식(본체 없음 · 묘비 없음)
    h3, why3, exp3 = hold("hard", 5000.0)
    check("1i 음성 대조 — 끝나지 않은 세대의 표식만 남으면 만료는 유지된다(구 판 호환)",
          exp3 is True and why3.startswith("expired"), "reason=%r" % (why3,))

    # ── ② N2 반대 방향: 같은 포화의 확정 만료는 취소되지 않는다 ────────────────────────────
    path = lane("n2-no-cancel")
    hold("hard", 1000.0)
    _, _, e_first = hold("hard", 1000.0 + HOLD_MAX + 1)
    check("2a 만료 공개", e_first is True)
    _, why_back, e_back = hold("hard", 900.0)             # 시계 역행(저장 since 가 미래)
    check("2b 시계 역행이 확정 만료를 취소하지 않는다",
          e_back is True and why_back.startswith("expired"), "reason=%r" % (why_back,))
    _, why_next, e_next = hold("hard", 1000.0 + HOLD_MAX + 2)
    check("2c 후속 hard 관측도 만료를 유지한다",
          e_next is True and why_next.startswith("expired"), "reason=%r" % (why_next,))

    # ── ③ N15: 부트 유예 창은 보류 시계를 태우지 않는다 ───────────────────────────────────
    path = lane("n15-grace")
    hg0 = hold("hard", 1000.0, grace=True)
    check("3a 유예 안의 hard 관측은 사유가 boot_grace 다",
          hg0[1] == "boot_grace" and hg0[2] is False, "ret=%r" % (hg0,))
    hg1 = hold("hard", 1000.0 + GRACE - 1, grace=True)    # 유예 끝 무렵의 마지막 관측
    check("3b 유예 안에서는 시계가 다시 놓인다(0초)", hg1[0] == 0.0, "hold=%r" % (hg1[0],))
    rec, _ = G._fleet_hold_read(path)
    check("3c 래치 since 가 유예 종료 무렵으로 밀렸다",
          rec is not None and rec["since"] == 1000.0 + GRACE - 1, "since=%r" % (rec or {}).get("since"))
    # 유예 종료 뒤 899초 — 종전 판본이라면 (유예 300 을 같이 태워) 이미 만료였다.
    t_after = 1000.0 + GRACE - 1 + HOLD_MAX - 1
    ha = hold("hard", t_after, grace=False)
    check("3d ★유예 종료 시점부터 900s 를 센다(600s 로 줄지 않는다)",
          ha[2] is False and ha[0] == HOLD_MAX - 1, "ret=%r" % (ha,))
    hb = hold("hard", t_after + 2, grace=False)
    check("3e 그 900s 를 넘기면 그때 만료한다", hb[2] is True, "ret=%r" % (hb,))

    # ── ④ N4: 상태 디렉터리 기록 불능 → 대체 자리로 유계를 잇는다 ──────────────────────────
    ro_state = os.path.join(_ROOT, "ro-state")
    os.makedirs(ro_state, exist_ok=True)
    tmp_ok = os.path.join(_ROOT, "tmp-ok")
    os.makedirs(tmp_ok, exist_ok=True)
    os.chmod(ro_state, stat.S_IRUSR | stat.S_IXUSR)        # 읽기·탐색만
    writable = True
    try:
        probe = os.path.join(ro_state, ".probe")
        os.mkdir(probe)
        os.rmdir(probe)
    except OSError:
        writable = False
    if writable:
        # root 로 돌면 읽기전용 모드가 무의미하다 — 그 형상에서는 이 축을 재지 않는다(정직 표기).
        check("4a 읽기전용 상태 디렉터리 형상(uid 0 에서는 적용불가)", True, "SKIP — 쓰기가 막히지 않음")
        check("4b 대체 자리(tmp)로 유계를 잇는다", True, "SKIP")
        check("4c 대체 자리에서도 hard 가 유지된다", True, "SKIP")
    else:
        _tmpdir_saved = tempfile.tempdir
        os.environ["CYS_STATE_DIR"] = ro_state
        tempfile.tempdir = tmp_ok
        try:
            r0 = G._fleet_hard_hold_ex("hard", None, 2000.0, None, False)
            check("4a 상태 디렉터리에 못 쓰면 만료로 접히지 않는다",
                  r0[2] is False and r0[1] == "armed", "ret=%r" % (r0,))
            check("4b 래치가 **대체 자리**에 섰다고 보고한다", r0[3] == "tmp", "latch=%r" % (r0[3],))
            r1 = G._fleet_hard_hold_ex("hard", None, 2100.0, None, False)
            check("4c 대체 자리에서도 연속 시계가 이어진다(유계 보존)",
                  r1[0] == 100.0 and r1[2] is False and r1[3] == "tmp", "ret=%r" % (r1,))
            r2 = G._fleet_hard_hold_ex("hard", None, 2000.0 + HOLD_MAX + 1, None, False)
            check("4d 대체 자리의 상한도 그대로 집행된다(무기한 차단 금지)",
                  r2[2] is True, "ret=%r" % (r2,))
        finally:
            tempfile.tempdir = _tmpdir_saved
            os.environ["CYS_STATE_DIR"] = os.path.join(_ROOT, "state")

    # ── ⑤ N4 종단: 게이트 JSON 이 그 사실을 나른다 ────────────────────────────────────────
    def gate(env_extra, *extra):
        env = dict(os.environ)
        env.update(env_extra)
        argv = [sys.executable, GATE, "check", "--json",
                "--dept-roster-override", ROSTER, "--boot-elapsed-override", "99999",
                "--servers-override", "0", "--nodes-override", "0", "--load-override", "0.0",
                "--fleet-cpu-soft", "0", "--fleet-cpu-hard", "0"] + list(extra)
        r = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=90,
                           env=env)
        try:
            return r.returncode, json.loads(r.stdout.strip())
        except ValueError:
            return r.returncode, {}

    rc_ok, doc_ok = gate({"CYS_STATE_DIR": os.path.join(_ROOT, "e2e-ok")})
    m_ok = doc_ok.get("measured") or {}
    ps_ok = m_ok.get("fleet_cpu_ratio") is not None      # ps 없는 플랫폼에서는 이 축이 없다
    if not ps_ok:
        check("5a 정상 형상에서 hard_block", True, "SKIP — 이 플랫폼에 함대 CPU 축이 없다(ps 부재)")
        check("5b 읽기전용 상태 디렉터리에서도 hard_block", True, "SKIP")
        check("5c 대체 자리도 못 쓰면 measure_errors 로 드러난다", True, "SKIP")
    else:
        check("5a 정상 형상: 임계 초과 → hard_block · latch=state",
              rc_ok == 2 and doc_ok.get("verdict") == "hard_block"
              and m_ok.get("fleet_cpu_hold_latch") == "state",
              "rc=%r verdict=%r latch=%r" % (rc_ok, doc_ok.get("verdict"),
                                             m_ok.get("fleet_cpu_hold_latch")))
        if writable:
            check("5b 읽기전용 상태 디렉터리에서도 hard_block(적용불가)", True, "SKIP")
            check("5c 대체 자리도 못 쓰면 measure_errors(적용불가)", True, "SKIP")
        else:
            rc_ro, doc_ro = gate({"CYS_STATE_DIR": ro_state, "TMPDIR": tmp_ok})
            m_ro = doc_ro.get("measured") or {}
            check("5b ★상태 디렉터리 기록 불능에서도 hard 가 hard 다(첫 관측 soft 강등 없음)",
                  rc_ro == 2 and doc_ro.get("verdict") == "hard_block"
                  and m_ro.get("fleet_cpu_hold_expired") is False
                  and m_ro.get("fleet_cpu_hold_latch") == "tmp",
                  "rc=%r verdict=%r latch=%r expired=%r" % (
                      rc_ro, doc_ro.get("verdict"), m_ro.get("fleet_cpu_hold_latch"),
                      m_ro.get("fleet_cpu_hold_expired")))
            # 5c 는 **대체 자리 자체를 낼 수 없는** 형상이다. `TMPDIR` 로는 재현할 수 없다 —
            # CPython 의 `tempfile.gettempdir()` 은 후보의 쓰기 가능성을 스스로 검사해 `/tmp` 로
            # 폴백하므로, 읽기전용 `TMPDIR` 을 줘도 대체 자리는 여전히 선다(그리고 시스템 `/tmp` 에
            # 읽기전용 자리를 심는 것은 병렬 실행 중인 남의 검체를 깨뜨린다). 그래서 이 축만
            # **같은 프로세스 안에서** 해소기를 None 으로 눌러 잰다.
            captured = {}
            _real_measure = G.measure

            def _cap(a):
                captured["a"] = a
                return _real_measure(a)

            G.measure = _cap
            _out = io.StringIO()
            _stdout = sys.stdout
            try:
                sys.stdout = _out
                try:
                    G.main(["check", "--json", "--dept-roster-override", ROSTER,
                            "--boot-elapsed-override", "99999", "--servers-override", "0",
                            "--nodes-override", "0", "--load-override", "0.0",
                            "--fleet-cpu-soft", "0", "--fleet-cpu-hard", "0"])
                except SystemExit:
                    pass
            finally:
                sys.stdout = _stdout
                G.measure = _real_measure
            a = captured.get("a")
            _fb_saved = G._fleet_hold_fallback_path
            _st_saved = os.environ.get("CYS_STATE_DIR")
            os.environ["CYS_STATE_DIR"] = ro_state
            G._fleet_hold_fallback_path = lambda thr=None: None      # 대체 자리도 없다
            try:
                m_bad = G.measure(a) if a is not None else {}
            finally:
                G._fleet_hold_fallback_path = _fb_saved
                if _st_saved is None:
                    os.environ.pop("CYS_STATE_DIR", None)
                else:
                    os.environ["CYS_STATE_DIR"] = _st_saved
            check("5c ★둘 다 못 쓰면 완화는 **조용하지 않다**(measure_errors: fleet_cpu(latch))",
                  "fleet_cpu(latch)" in (m_bad.get("measure_errors") or [])
                  and m_bad.get("fleet_cpu_hold_latch") == "none"
                  and m_bad.get("fleet_cpu_hold_expired") is True,
                  "errors=%r latch=%r" % (m_bad.get("measure_errors"),
                                          m_bad.get("fleet_cpu_hold_latch")))
finally:
    for _d in (os.path.join(_ROOT, "ro-state"), os.path.join(_ROOT, "ro-tmp")):
        try:
            os.chmod(_d, stat.S_IRWXU)
        except OSError:
            pass
    shutil.rmtree(_ROOT, ignore_errors=True)

print("")
if fails:
    print("FAILED %d: %s" % (len(fails), " · ".join(fails)))
    sys.exit(1)
print("ALL PASS")
print("REFL-FLEET-LATCH-GEN-OK")
