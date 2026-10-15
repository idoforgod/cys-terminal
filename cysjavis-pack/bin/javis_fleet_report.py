#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_fleet_report — read-only fleet 채택 + 비용 digest (스케줄 발화, 핫패스 밖).

master의 막연한 '철저 감독'을 결정론 일일 채택+비용 대시보드로 환원한다. 전
`~/.local/state/*/analytics.db`를 **read-only(mode=ro)** 로 열어
  ① 도구/서브에이전트/스킬 채택(events)  ② 실제 $ 지출·토큰·캐시 히트율(usage_records)
를 집계하고, events에 명령 컬럼이 없으므로 `cys recall` FTS로 고빈도 수동 idiom(U1/U2 필터의
missed-savings 후보)을 마이닝한다.

★불변: analytics.db·데몬 테이블에 **절대 쓰지 않는다**(WAL 소유=cysd). file:...?mode=ro 만.
한 DB 불량에 hard-fail 금지(skip+기록+계속). 데이터 문제에 절대 non-zero exit 안 함.

사용: python3 javis_fleet_report.py [--json] [--push] [--days N=7] [--limit N=200] [--round-dir D]
  (무플래그) -> 단일화면 digest stdout(스케줄러 action:push 가 캡처·배달)
  --json     -> 풀 머신리더블 JSON stdout
  --push     -> digest stdout + `cys send --queued --to master`(CLI --command 경로용 자체 배달)
의존성: 파이썬 표준 라이브러리(sqlite3 포함) + PATH의 `cys`(recall/send; 없으면 그 단계 skip).

CORE-DEPENDENT(부재 cys.app/cysd 소스 필요 — 본 스크립트는 정직 디스코프):
  exit_code 전 행 단일값·duration_ms 전 행 NULL → fail-rate/latency 불가.
  usage_records.role NULL → per-role 비용 귀속 불가(agent/model 그룹만).
  events에 command/tool_input 컬럼 없음 → 토큰-loss 랭킹은 recall FTS 만(HEURISTIC).
"""

import argparse
import glob
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import time

# fleet DB 라벨(개행 든 손상 dir·무관 dir claude/cmux/gh/pnpm/fnm_multishells 배제).
LABEL_RE = re.compile(r"^(cys|cys-ceo|cys-dept-[A-Za-z0-9-]+|aiterm)$")

# missed-savings 마이닝 시드(U1/U2 rewrite-map 타깃과 정합 — recall FTS 빈도 랭킹).
MANUAL_IDIOMS = ["git status", "git diff", "git log", "pytest", "vitest",
                 "eslint", "ruff check", "npm test", "cargo test", "cargo build"]


def pack_dir():
    for key in ("CYS_PACK_DIR", "JAVIS_PACK_DIR", "AITERM_JARVIS_DIR"):
        v = os.environ.get(key, "")
        if v:
            return v
    return os.path.join(os.path.expanduser("~"), ".cys/pack")


def state_dir():
    return os.path.join(os.path.expanduser("~"), ".local", "state")


def round_dir(arg):
    return (arg or os.environ.get("CYS_ROUND_DIR")
            or os.path.join(os.getcwd(), "_round"))


# ── DB 발견 (label 정규식 + 개행/손상 방어) ──────────────────────────────────────────
def _db_candidates():
    """(label, path) 후보. unix: ~/.local/state/<label>/analytics.db(평면).
    Windows: 데몬 정본 %LOCALAPPDATA%/cys(state.rs state_dir/pipe_slug — javis_phoenix
    _live_state_dir·javis_state_snapshot _win_state_dir_for_socket 와 동일 규약). 기본 데몬 DB 는
    루트 직하 cys/analytics.db, 부서 DB 는 그 하위 cys/cys-dept-*/analytics.db(중첩)."""
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or os.path.join(
            os.path.expanduser("~"), "AppData", "Local")
        root = os.path.join(base, "cys")
        out = [("cys", os.path.join(root, "analytics.db"))]
        pattern = os.path.join(root, "*", "analytics.db")
    else:
        out = []
        pattern = os.path.join(state_dir(), "*", "analytics.db")
    for path in sorted(glob.glob(pattern)):
        out.append((os.path.basename(os.path.dirname(path)), path))
    return out


def discover_dbs():
    found = []
    for label, path in _db_candidates():
        if "\n" in label or "\r" in label:           # 개행 든 손상 dir 방어 skip
            continue
        if not LABEL_RE.match(label):                 # 무관 dir 배제
            continue
        if os.path.isfile(path):
            found.append((label, path))
    return found


def _ro_connect(path):
    return sqlite3.connect("file:%s?mode=ro" % path, uri=True, timeout=5)


def read_db(path, cutoff_ts):
    """단일 DB read-only 집계. 실패 시 ('error', 사유). cutoff_ts 이후만(윈도)."""
    out = {"tools": [], "subagents": [], "skills": [], "cost": [],
           "cost_all_time": None, "bash_count": 0}
    try:
        con = _ro_connect(path)
        con.execute("SELECT count(*) FROM sqlite_master").fetchone()  # 손상 DB 를 0 합산 대신 skip 으로
    except sqlite3.Error as e:
        return {"error": "connect: %s" % e}
    try:
        cur = con.cursor()
        w = " AND ts >= ?" if cutoff_ts else ""
        a = (cutoff_ts,) if cutoff_ts else ()

        def q(sql, args=()):
            try:
                return cur.execute(sql, args).fetchall()
            except sqlite3.Error:
                return []

        out["tools"] = [{"tool": t, "count": c} for t, c in q(
            "SELECT tool_name,COUNT(*) FROM events WHERE event_type='PRE_TOOL'%s "
            "GROUP BY tool_name ORDER BY 2 DESC" % w, a)]
        out["subagents"] = [{"agent_type": t or "?", "count": c} for t, c in q(
            "SELECT agent_type,COUNT(*) FROM events WHERE is_agent=1%s "
            "GROUP BY agent_type ORDER BY 2 DESC" % w, a)]
        out["skills"] = [{"skill": s or "?", "count": c} for s, c in q(
            "SELECT skill_name,COUNT(*) FROM events WHERE is_skill=1%s "
            "GROUP BY skill_name ORDER BY 2 DESC" % w, a)]
        for r in out["tools"]:
            if r["tool"] == "Bash":
                out["bash_count"] = r["count"]
        rows = q("SELECT agent,model,SUM(cost_usd),SUM(input_tokens),SUM(output_tokens),"
                 "SUM(cache_creation),SUM(cache_read) FROM usage_records WHERE 1=1%s "
                 "GROUP BY agent,model ORDER BY 3 DESC" % w, a)
        for ag, md, cost, intok, outtok, cc, cr in rows:
            cc = cc or 0; cr = cr or 0; intok = intok or 0
            denom = cr + cc + intok
            out["cost"].append({
                "agent": ag, "model": md, "cost_usd": round(cost or 0, 2),
                "input": intok, "output": outtok or 0, "cache_creation": cc,
                "cache_read": cr,
                "cache_hit_ratio_pct": round(100.0 * cr / denom, 1) if denom else 0.0})
        at = q("SELECT SUM(cost_usd),SUM(cache_read),SUM(cache_creation),SUM(input_tokens) "
               "FROM usage_records")
        if at and at[0][0] is not None:
            c, cr, cc, it = at[0]
            denom = (cr or 0) + (cc or 0) + (it or 0)
            out["cost_all_time"] = {
                "cost_usd": round(c or 0, 2), "cache_read": cr or 0,
                "cache_hit_ratio_pct": round(100.0 * (cr or 0) / denom, 1) if denom else 0.0}
    finally:
        con.close()
    return out


def _sum_lists(dst, src, key, idkey):
    idx = {d[idkey]: d for d in dst}
    for r in src:
        if r[idkey] in idx:
            idx[r[idkey]][key] += r[key]
        else:
            d = dict(r); dst.append(d); idx[r[idkey]] = d


def fleet_aggregate(per_db):
    tools, subs, skills = [], [], []
    cost_usd = 0.0
    cache_read = cache_creation = inp = 0
    for label, d in per_db.items():
        if "error" in d:
            continue
        _sum_lists(tools, d["tools"], "count", "tool")
        _sum_lists(subs, d["subagents"], "count", "agent_type")
        _sum_lists(skills, d["skills"], "count", "skill")
        for c in d["cost"]:
            cost_usd += c["cost_usd"]; cache_read += c["cache_read"]
            cache_creation += c["cache_creation"]; inp += c["input"]
    tools.sort(key=lambda x: -x["count"]); subs.sort(key=lambda x: -x["count"])
    skills.sort(key=lambda x: -x["count"])
    denom = cache_read + cache_creation + inp
    return {"tools": tools, "subagents": subs, "skills": skills,
            "cost_usd_window": round(cost_usd, 2),
            "cache_hit_ratio_pct": round(100.0 * cache_read / denom, 1) if denom else 0.0,
            "cache_read": cache_read}


# ── missed-savings 마이닝 (cys recall FTS — events엔 명령 컬럼 없음) ─────────────────────
def mine_missed_savings(days, limit):
    cys = shutil.which("cys")
    if not cys:
        return None
    rows = []
    for idiom in MANUAL_IDIOMS:
        try:
            r = subprocess.run([cys, "recall", idiom, "--days", str(days),
                               "--limit", str(limit)], capture_output=True,
                              text=True, timeout=20)
            hits = sum(1 for ln in r.stdout.splitlines() if "● Bash(" in ln)
        except Exception:
            hits = 0
        if hits:
            rows.append({"idiom": idiom, "recall_hits": hits})
    rows.sort(key=lambda x: -x["recall_hits"])
    return rows


# ── PACK/버전 advisory (javis_semver.py 위임 — 순수 advisory·db쓰기0·exit 영향0) ─────────────
def pack_version_advisory():
    """installed `cys --version` vs source Cargo.toml version 을 결정론으로 판정(advisory only).

    ★불변 보존: analytics.db 미접근·non-zero exit 미발생·어떤 비가역/외부발행 행동도 0.
    javis_semver.py(부재 가능)·cys(부재 가능)·소스 트리(부재 가능) 중 하나라도 없으면
    조용히 None 반환(skip+기록). 실패는 절대 fleet_report 를 죽이지 않는다(라인 11 불변).
    """
    try:
        bindir = os.path.join(pack_dir(), "bin")
        semver = os.path.join(bindir, "javis_semver.py")
        cys = shutil.which("cys")
        if not (os.path.isfile(semver) and cys):
            return None
        out = subprocess.run([cys, "--version"], capture_output=True, text=True,
                             timeout=15).stdout
        m = re.search(r"(\d+\.\d+\.\d+\S*)", out or "")
        if not m:
            return None
        installed = m.group(1)
        # 소스 후보 = cys-terminal Cargo.toml(있으면). 없으면 advisory 불가 → skip.
        cargo = os.path.expanduser("~/dev/cys-terminal/Cargo.toml")
        if not os.path.isfile(cargo):
            return None
        r = subprocess.run([sys.executable, semver, "gate", "--local-file", cargo,
                           "--remote", installed, "--field", "version", "--json"],
                          capture_output=True, text=True, timeout=15)
        # exit 10=UPDATE_AVAILABLE 도 정상 advisory 신호(여기선 흡수·non-zero 전파 안 함).
        data = json.loads(r.stdout or "{}")
        return {"verdict": data.get("verdict"), "installed_cys": installed,
                "source_cargo": (data.get("local") or {}).get("raw", "").strip(),
                "evidence": data.get("evidence")}
    except Exception:
        return None  # skip+기록(아래 build_report 가 None 을 notes 로 표기)


def learn_summary(round_arg):
    """RSI 학습 결과 지표(설계 G5 — quota-filling 봉쇄: '추천 수' 등 활동량 지표 금지·결과만).
    데이터원=CYS_ROUND_DIR 규약의 learn 상태(javis_learn과 동일 해석 — learn_state.json 우선·
    legacy state.json 폴백). 부재/불량=None(digest 행 생략 관용 — 라인 11 불변 준수)."""
    try:
        d = os.path.join(round_dir(round_arg), "learn")
        # ★P0-2 canonical 동조 — CYS_ROUND_DIR==~/.cys/state면 데몬 state.json 라운드를 사설에 union
        #   (stale 사설만 읽어 adopted=0 오신호→"게이트 비용 재조정" 거짓 트리거 봉쇄).
        canonical = False
        root = os.environ.get("CYS_ROUND_DIR")
        if root:
            try:
                canonical = os.path.realpath(root) == os.path.realpath(os.path.expanduser("~/.cys/state"))
            except OSError:
                canonical = False

        def _rd(fn):
            p = os.path.join(d, fn)
            if os.path.isfile(p):
                try:
                    with open(p, encoding="utf-8") as fh:
                        v = json.load(fh)
                    return v if isinstance(v, dict) else None
                except (OSError, ValueError):
                    return None
            return None

        priv = _rd("learn_state.json")
        if canonical:
            st = priv if priv is not None else {}
            daemon = _rd("state.json")
            if daemon:
                dr = daemon.get("rounds", {})
                sr = st.setdefault("rounds", {})
                for rid, rec in (dr.items() if isinstance(dr, dict) else []):
                    if rid not in sr:
                        sr[rid] = rec
            if priv is None and not daemon:
                st = None
        else:
            st = priv if priv is not None else _rd("state.json")
        if not isinstance(st, dict):
            return None
        counts = {"confirmed": 0, "provisional": 0, "challenged": 0, "tombstone": 0}
        effects = {"improved": 0, "none": 0}
        last_ts = None
        for r in (st.get("rounds") or {}).values():
            if not isinstance(r, dict):
                continue
            for it in (r.get("stored") or []) + (r.get("harness") or []):
                if not isinstance(it, dict):
                    continue
                s = it.get("state")
                if s in counts:
                    counts[s] += 1
                for e in it.get("effect_log") or []:
                    eff = (e or {}).get("effect") if isinstance(e, dict) else None
                    if eff in effects:
                        effects[eff] += 1
                ts = it.get("ts")
                if isinstance(ts, (int, float)):
                    last_ts = max(last_ts or 0, ts)
        # 채택(효력)=confirmed+provisional+challenged(challenged는 효력 유지 명문 — javis_learn C4).
        adopted = counts["confirmed"] + counts["provisional"] + counts["challenged"]
        return {"adopted": adopted, "by_state": counts, "effects": effects,
                "last_episode": (time.strftime("%Y-%m-%d", time.localtime(last_ts))
                                 if last_ts else None),
                "gate_cost_review": adopted == 0}
    except Exception:
        return None  # 부재/불량=행 생략(fleet_report 는 데이터 문제로 절대 죽지 않는다)


def build_report(days, limit, round_arg):
    cutoff = time.time() - days * 86400 if days else None
    dbs = discover_dbs()
    per_db, skipped = {}, []
    for label, path in dbs:
        d = read_db(path, cutoff)
        if "error" in d:
            skipped.append({"label": label, "path": path, "reason": d["error"]})
        per_db[label] = d
    fleet = fleet_aggregate(per_db)
    fleet["missed_savings"] = mine_missed_savings(days, limit)
    fleet["pack_version_advisory"] = pack_version_advisory()  # None 가능(skip)
    fleet["learn"] = learn_summary(round_arg)  # None 가능(learn 상태 부재=행 생략 관용)
    return {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "days_window": days,
        "dbs_scanned": [l for l, _ in dbs],
        "dbs_skipped": skipped,
        "fleet": fleet,
        "per_db": per_db,
        "notes": [
            "skill 채택 과소(대부분 skill은 프롬프트 실행, Skill-tool-only 카운트)",
            "missed_savings는 HEURISTIC trigram recall-hit-count(토크나이저 측정 아님·$/토큰 환산 금지)",
            "per-role 비용 불가(usage_records.role NULL) · fail-rate/duration 불가(exit_code 단일·duration NULL)",
            "cost_usd_window=최근 N일 · cost_all_time(per_db)=누적",
        ],
    }


def render_digest(rep):
    f = rep["fleet"]
    L = ["[일일 fleet 채택/비용 digest · 결정론 read-only 집계 (최근 %d일)]" % rep["days_window"]]
    L.append("• DB 스캔: %d개 (%s)%s" % (
        len(rep["dbs_scanned"]), ", ".join(rep["dbs_scanned"][:6])
        + (" …" if len(rep["dbs_scanned"]) > 6 else ""),
        "  ⚠skip %d" % len(rep["dbs_skipped"]) if rep["dbs_skipped"] else ""))
    if not rep["dbs_scanned"]:                        # 조용한 0 방지 — 아래 0은 측정값이 아니다
        L.append("• ⚠ analytics.db 를 하나도 찾지 못함 — 아래 수치 0은 실측이 아님(경로 확인)")
    top = ", ".join("%s=%d" % (t["tool"], t["count"]) for t in f["tools"][:6])
    L.append("• 도구 채택(PRE_TOOL): %s" % (top or "없음"))
    if f["subagents"]:
        L.append("• 서브에이전트: %s" % ", ".join(
            "%s=%d" % (s["agent_type"], s["count"]) for s in f["subagents"][:5]))
    L.append("• 비용(최근 %d일): $%.2f · 캐시 히트율 %.1f%% (cache_read %s)" % (
        rep["days_window"], f["cost_usd_window"], f["cache_hit_ratio_pct"],
        "{:,}".format(f["cache_read"])))
    ms = f.get("missed_savings")
    if ms:
        L.append("• 고빈도 명령(HEURISTIC recall-hit): %s" %
                 ", ".join("%s×%d" % (m["idiom"], m["recall_hits"]) for m in ms[:6]))
    elif ms is None:
        L.append("• 고빈도 명령: cys recall 불가(스캔 skip)")
    pv = f.get("pack_version_advisory")
    if pv and pv.get("verdict"):
        L.append("• PACK/버전 advisory(javis_semver·결정론·비행동): installed cys %s vs source %s → %s"
                 % (pv.get("installed_cys"), pv.get("source_cargo"), pv["verdict"]))
    ln = f.get("learn")
    if ln:  # G5 결과 지표 — 활동량('추천 수') 금지: 채택 학습물·사후 효과·tombstone 만.
        bs, ef = ln["by_state"], ln["effects"]
        L.append("• 학습(RSI·결과 지표): 채택(효력) %d (confirmed %d·provisional %d·challenged %d)"
                 " · 사후효과 improved %d/none %d · tombstone %d · 마지막 에피소드 %s"
                 % (ln["adopted"], bs["confirmed"], bs["provisional"], bs["challenged"],
                    ef["improved"], ef["none"], bs["tombstone"], ln["last_episode"] or "-"))
        if ln["gate_cost_review"]:
            L.append("  ↳ 채택 학습물 0건 — 알람 아님: '게이트 비용 재조정 검토' 트리거(G5)")
    L.append("(요약금지 carve-out: mechanical 대시보드 — 풀 무압축 데이터는 JSON sidecar)")
    return "\n".join(L)


def write_rollup(rep, round_arg):
    try:
        d = os.path.join(round_dir(round_arg), "fleet")
        os.makedirs(d, exist_ok=True)
        fn = os.path.join(d, "fleet_report_%s.json" % time.strftime("%Y%m%d"))
        tmp = fn + ".tmp"
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(rep, fh, ensure_ascii=False, indent=2)
        os.replace(tmp, fn)
        return fn
    except Exception:
        return None


def push_to_master(text):
    cys = shutil.which("cys")
    if not cys:
        return False
    try:
        subprocess.run([cys, "send", "--queued", "--to", "master", text],
                      capture_output=True, timeout=15)
        return True
    except Exception:
        return False


def main():
    ap = argparse.ArgumentParser(description="read-only fleet 채택+비용 digest")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--push", action="store_true",
                    help="digest 를 cys send --queued --to master 로 자체 배달(CLI --command 경로)")
    ap.add_argument("--days", type=int, default=7)
    ap.add_argument("--limit", type=int, default=200, help="recall 마이닝 --limit")
    ap.add_argument("--round-dir")
    args = ap.parse_args()

    rep = build_report(args.days, args.limit, args.round_dir)
    rollup = write_rollup(rep, args.round_dir)
    rep["rollup_path"] = rollup

    if args.json:
        print(json.dumps(rep, ensure_ascii=False, indent=2))
    else:
        digest = render_digest(rep)
        print(digest)
        if args.push:
            push_to_master(digest)
    return 0


if __name__ == "__main__":
    sys.exit(main())
