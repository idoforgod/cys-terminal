#!/usr/bin/env python3
"""javis_waiver.py — VIBECODING CONSTITUTION §C2.3 만료형 waiver 도구.

계약(출처: _research/vibecoding-mastery/PROPOSAL-jarvis-vibecoding-system-v3.md §C2.3 L149):
  waiver = {waiver_id, 대상 규칙, 사유, 승인자(§C1.2 기준), 발급일, **만료일(필수)**, 해소 계획}.
  만료 시 자동 fail — 연장은 재승인 필요(silent 연장 금지). 즉 이 도구는 만료일을 지나면
  절대 자동 연장하지 않는다. 만료된 waiver로 게이트를 통과시키려는 시도는 fail-closed로 막힌다.

§C1.2 승인자 규칙의 기계화:
  waiver 발급 = master(만료형·저위험) / doctor(고위험). 따라서 --risk high 는 --approver doctor
  가 아니면 발급 거부(exit 2). "요청자 자기주장은 근거 아님"(§C1.3) — 승인자는 명시 필드로만 기록.

저장: $JAVIS_ROOT/.vibecoding/waivers.jsonl (append-only — 폐기도 status 전이가 아니라
      새 레코드로만. 이 도구는 issue/check/list만 제공하며 기존 줄을 절대 수정·삭제하지 않는다).

exit codes: 0 ok · 1 유효 waiver 없음/만료(check) · 2 usage/승인자 규칙 위반
"""
import argparse
import datetime
import json
import os
import re
import sys

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD
VIBE_DIR = os.path.join(ROOT, ".vibecoding")
WAIVERS = os.path.join(VIBE_DIR, "waivers.jsonl")

EXIT_OK, EXIT_NONE, EXIT_USAGE = 0, 1, 2

APPROVERS = ("master", "doctor")
RISKS = ("low", "high")
_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def _today():
    return datetime.date.today()


def _parse_date(s):
    if not _DATE_RE.match(s or ""):
        return None
    try:
        return datetime.date.fromisoformat(s)
    except ValueError:
        return None


def _read_all():
    if not os.path.exists(WAIVERS):
        return []
    out = []
    with open(WAIVERS, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                continue  # 손상 줄은 건너뛰되 유효 waiver로 취급하지 않음(fail-closed)
    return out


def _next_id(today):
    prefix = f"WVR-{today.strftime('%Y%m%d')}-"
    n = 0
    for rec in _read_all():
        wid = rec.get("waiver_id", "")
        if wid.startswith(prefix):
            try:
                n = max(n, int(wid[len(prefix):]))
            except ValueError:
                pass
    return f"{prefix}{n + 1:03d}"


def cmd_issue(args):
    expiry = _parse_date(args.expiry)
    if expiry is None:
        print("ERROR: --expiry 는 필수이며 YYYY-MM-DD 형식이어야 합니다(만료일 없는 waiver 금지).", file=sys.stderr)
        return EXIT_USAGE
    if args.approver not in APPROVERS:
        print(f"ERROR: --approver 는 {APPROVERS} 중 하나여야 합니다.", file=sys.stderr)
        return EXIT_USAGE
    if args.risk not in RISKS:
        print(f"ERROR: --risk 는 {RISKS} 중 하나여야 합니다.", file=sys.stderr)
        return EXIT_USAGE
    # §C1.2 기계화: 고위험 waiver 는 doctor 만 발급 가능(master 발급은 fail-closed).
    if args.risk == "high" and args.approver != "doctor":
        print("ERROR: 고위험(--risk high) waiver 는 doctor(owner — 사람 승인자) 승인만 유효합니다(§C1.2). master 발급 거부.", file=sys.stderr)
        return EXIT_USAGE

    today = _today()
    if expiry < today:
        print(f"ERROR: 만료일({expiry})이 발급일({today}) 이전입니다 — 발급 즉시 만료되는 waiver 금지.", file=sys.stderr)
        return EXIT_USAGE

    rec = {
        "waiver_id": _next_id(today),
        "target_rule": args.rule,
        "reason": args.reason,
        "approver": args.approver,
        "risk": args.risk,
        "issued": today.isoformat(),
        "expiry": expiry.isoformat(),
        "remediation": args.remediation,
        "ts": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
    }
    os.makedirs(VIBE_DIR, exist_ok=True)
    with open(WAIVERS, "a", encoding="utf-8") as f:
        f.write(json.dumps(rec, ensure_ascii=False) + "\n")
    print(json.dumps(rec, ensure_ascii=False, indent=2))
    return EXIT_OK


def cmd_check(args):
    """대상 규칙에 유효(미만료) waiver 가 있으면 exit 0, 없거나 전부 만료면 exit 1.

    만료 판정은 항상 '오늘' 기준으로 재계산 — 저장된 상태에 의존하지 않으므로 silent 연장 불가능.
    """
    today = _today()
    matches = [r for r in _read_all() if r.get("target_rule") == args.rule]
    valid, expired = [], []
    for r in matches:
        ed = _parse_date(r.get("expiry", ""))
        if ed is not None and ed >= today:
            valid.append(r)
        else:
            expired.append(r)
    if valid:
        # 만료일이 가장 늦은 것을 대표로 출력.
        valid.sort(key=lambda r: r.get("expiry", ""), reverse=True)
        print(json.dumps({"status": "valid", "waiver": valid[0], "count": len(valid)}, ensure_ascii=False, indent=2))
        return EXIT_OK
    if expired:
        print(json.dumps({"status": "expired", "expired_count": len(expired),
                          "note": "만료된 waiver는 자동 fail — 재승인(신규 issue) 필요(silent 연장 금지)."},
                         ensure_ascii=False, indent=2), file=sys.stderr)
        return EXIT_NONE
    print(json.dumps({"status": "none", "rule": args.rule}, ensure_ascii=False), file=sys.stderr)
    return EXIT_NONE


def cmd_list(args):
    today = _today()
    rows = _read_all()
    if args.rule:
        rows = [r for r in rows if r.get("target_rule") == args.rule]
    annotated = []
    for r in rows:
        ed = _parse_date(r.get("expiry", ""))
        r = dict(r)
        r["_state"] = "valid" if (ed is not None and ed >= today) else "expired"
        annotated.append(r)
    print(json.dumps(annotated, ensure_ascii=False, indent=2))
    return EXIT_OK


def main(argv=None):
    p = argparse.ArgumentParser(description="§C2.3 만료형 waiver 발급·검사·열람")
    sub = p.add_subparsers(dest="cmd", required=True)

    pi = sub.add_parser("issue", help="waiver 발급(만료일·승인자 필수)")
    pi.add_argument("--rule", required=True, help="대상 규칙(예: 4조-regression)")
    pi.add_argument("--reason", required=True, help="사유")
    pi.add_argument("--approver", required=True, help="승인자(master|doctor — doctor = owner(사용자)의 식별자) — §C1.2")
    pi.add_argument("--expiry", required=True, help="만료일 YYYY-MM-DD(필수)")
    pi.add_argument("--remediation", required=True, help="해소 계획(만료 전 부채 해소)")
    pi.add_argument("--risk", default="low", help="위험도(low|high) — high는 doctor(owner) 승인만")
    pi.set_defaults(func=cmd_issue)

    pc = sub.add_parser("check", help="대상 규칙에 유효 waiver 존재? (없거나 만료=exit 1)")
    pc.add_argument("--rule", required=True)
    pc.set_defaults(func=cmd_check)

    pl = sub.add_parser("list", help="waiver 목록(만료 상태 주석 포함)")
    pl.add_argument("--rule", default=None)
    pl.set_defaults(func=cmd_list)

    args = p.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
