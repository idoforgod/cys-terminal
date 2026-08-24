#!/usr/bin/env python3
"""verify_evidence.py — `docs/evidence/` 보존 계약의 기계 집행자 (U-26 ② · 2026-08-24).

    python3 scripts/verify_evidence.py               # exit 0 = 계약 충족
    python3 scripts/verify_evidence.py --json        # 기계 판독
    python3 scripts/verify_evidence.py --self-test   # ★탐지 능력 자체를 합성 표본으로 시험

계약 본문은 `docs/evidence/README.md` 다. 이 파일은 그 문서가 **문서로만 남지 않게** 한다 —
집행자 없는 계약은 계약이 아니라 소원이다.

★`--self-test` 가 왜 필수인가(MEMORY '디버깅 계측 타당성 게이트' 3칙):
  정상 트리에는 위반이 **0건**이다. 그러면 검사기가 완전히 고장 나 있어도(예: 규칙 루프가
  한 번도 안 돌아도) 결과는 똑같이 초록이다. 그래서 '위반 0' 은 검사기가 살아 있다는 증거가
  못 된다. `--self-test` 는 규칙마다 **일부러 위반한 합성 표본**을 만들어 검사기가 실제로
  그것을 잡는지 본다 — 하나라도 못 잡으면 검사기 자신이 적색이다.

stdlib 만 사용. 네트워크 0. 읽기 전용(어떤 파일도 쓰지 않는다 — self-test 는 tmp 안에서만).
"""
import argparse
import json
import os
import re
import sys
import tempfile

SCHEMA = "cys-evidence/1"
KINDS = ("probe", "capture", "bench", "run")
NAME_RE = re.compile(r"^(%s)-(\d{4}-\d{2}-\d{2})-([a-z0-9][a-z0-9-]*)\.json$" % "|".join(KINDS))
REQUIRED = ("schema", "id", "title", "measured_on", "tool", "platform",
            "repro", "provenance", "observations")

# 금지 패턴 — **정밀도 우선**. 넓은 정규식은 오탐으로 커밋을 막고, 그러면 다음 사람이
# 검사기를 꺼 버린다(게이트의 자살). 각 패턴은 '이 문자열이 증거 파일에 있을 정당한 이유가
# 없다'가 자명한 것만 넣는다.
SECRET_PATS = [
    ("anthropic-api-key", re.compile(r"sk-ant-[A-Za-z0-9_-]{8,}")),
    ("github-token", re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}")),
    ("aws-access-key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("private-key-block", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
]
# 사용자 홈 절대경로 — 재현 명령은 자리표시자를 써야 한다(다른 기계에서 그대로 안 돈다).
HOME_PATS = [
    ("posix-home", re.compile(r"/(?:Users|home)/(?!<)[A-Za-z0-9._-]+/")),
    ("windows-home", re.compile(r"[A-Za-z]:\\\\?Users\\\\?(?!<)[A-Za-z0-9._-]+")),
]
EMAIL_RE = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
# 대장(README §6) 표에서 파일명을 뽑는 패턴.
INDEX_ROW = re.compile(r"\|\s*`([^`]+\.json)`\s*\|")


def _repo_root(start=None):
    here = os.path.dirname(os.path.abspath(start or __file__))
    return os.path.dirname(here)


def check_dir(evdir):
    """반환: (violations[str], stats{}) — 예외를 던지지 않는다(호출부가 판정한다)."""
    v = []
    stats = {"files": 0, "observations": 0, "indexed": 0}
    readme = os.path.join(evdir, "README.md")
    if not os.path.isdir(evdir):
        return (["증거 디렉터리 부재: %s — 계약이 착지하지 않았다" % evdir], stats)
    if not os.path.isfile(readme):
        return (["보존 계약 문서 부재: %s (docs/evidence/README.md 가 계약 정본이다)" % readme],
                stats)
    rtext = _read(readme)
    for token in ("cys-evidence/1", "retention", "provenance", "repro"):
        if token not in rtext:
            v.append("계약 문서에 필수 조항 토큰 부재: %r (%s)" % (token, readme))

    indexed = set(INDEX_ROW.findall(rtext))
    stats["indexed"] = len(indexed)

    present = set()
    for name in sorted(os.listdir(evdir)):
        p = os.path.join(evdir, name)
        if not os.path.isfile(p) or name == "README.md":
            continue
        if not name.endswith(".json"):
            v.append("증거 아닌 파일: %s — 이 디렉터리는 기계 판독 아티팩트 전용이다" % name)
            continue
        present.add(name)
        stats["files"] += 1
        v.extend(_check_file(evdir, name))

    # ★보존(retention) hard gate — 대장과 실물의 1:1. 파일을 조용히 지우는 경로를 없앤다.
    for miss in sorted(indexed - present):
        v.append("대장(README §6)에 있으나 파일이 없다: %s — 증거 삭제는 조용히 일어날 수 없다"
                 "(값이 틀렸다면 새 측정으로 덮되 옛 파일은 superseded_by 로 남긴다)" % miss)
    for extra in sorted(present - indexed):
        v.append("파일이 있으나 대장(README §6)에 없다: %s — 대장 등재가 커밋 규율이다" % extra)

    for name in sorted(present):
        try:
            d = json.loads(_read(os.path.join(evdir, name)))
            stats["observations"] += len(d.get("observations") or [])
        except ValueError:
            pass
    return (v, stats)


def _read(p):
    with open(p, encoding="utf-8", errors="replace") as f:
        return f.read()


def _check_file(evdir, name):
    v = []
    p = os.path.join(evdir, name)
    m = NAME_RE.match(name)
    if not m:
        v.append("파일명 규약 위반: %s — `<kind>-<YYYY-MM-DD>-<slug>.json` (kind ∈ %s)"
                 % (name, "|".join(KINDS)))
    raw = _read(p)
    try:
        d = json.loads(raw)
    except ValueError as e:
        return v + ["JSON 파싱 실패: %s (%s)" % (name, e)]
    if not isinstance(d, dict):
        return v + ["최상위가 객체가 아니다: %s" % name]

    for k in REQUIRED:
        if k not in d or d[k] in ("", None, [], {}):
            v.append("필수 키 결손: %s.%s" % (name, k))
    if d.get("schema") not in (None, SCHEMA):
        v.append("스키마 불일치: %s.schema=%r (기대 %r)" % (name, d.get("schema"), SCHEMA))
    if m:
        if d.get("id") != name[:-5]:
            v.append("id 불일치: %s.id=%r (파일명 기준 %r)" % (name, d.get("id"), name[:-5]))
        if d.get("measured_on") != m.group(2):
            v.append("측정일 불일치: %s.measured_on=%r (파일명 %r) — 커밋일이 아니라 측정일이다"
                     % (name, d.get("measured_on"), m.group(2)))

    obs = d.get("observations")
    if isinstance(obs, list):
        if not obs:
            v.append("관측 0건: %s — 결론만 있고 관측이 없는 파일은 증거가 아니다" % name)
        for i, o in enumerate(obs):
            if not isinstance(o, dict):
                v.append("%s.observations[%d] 가 객체가 아니다" % (name, i))
                continue
            for k in ("fact", "evidence"):
                if not o.get(k):
                    v.append("%s.observations[%d].%s 결손" % (name, i, k))
            if o.get("derived") and not o.get("derived_from"):
                v.append("%s.observations[%d] 는 추론(derived)인데 derived_from 이 없다 — "
                         "어느 관측에서 나왔는지 밝히지 않은 추론은 증거로 세지 않는다" % (name, i))
    elif obs is not None:
        v.append("%s.observations 가 배열이 아니다" % name)

    for label, pat in SECRET_PATS:
        if pat.search(raw):
            v.append("자격증명 유출 의심(%s): %s" % (label, name))
    for label, pat in HOME_PATS:
        hit = pat.search(raw)
        if hit:
            v.append("사용자 홈 절대경로(%s): %s — %r · 자리표시자($HOME·%%USERPROFILE%%·<cwd>)를 써라"
                     % (label, name, hit.group(0)[:60]))
    for e in EMAIL_RE.findall(raw):
        # 예시 도메인·제품 도메인은 개인 식별자가 아니다.
        if not e.lower().endswith(("@example.com", "@anthropic.com", "@claude.com")):
            v.append("개인 이메일로 보이는 문자열: %s — %s" % (name, e))
    return v


# ── 자기검증(합성 표본) ──────────────────────────────────────────────────────

# ★검체용 **가짜** 자격증명 — 조각으로 두고 런타임에 조립한다(2026-08-24).
#
#   왜 리터럴로 두지 않는가: 발행 게이트 `scripts/secret-scan.sh` 는 *정적 패턴 매칭*이라
#   소스에 통짜로 박힌 `sk-ant-…` 를 진짜 유출과 구별할 수 없다. 그리고 그것이 옳다 —
#   형태로 막지 않으면 진짜가 새는 날 구별할 방법이 없다. 그래서 **스캐너가 보는 형태**만
#   피하고 **검체가 보는 값**은 종전과 한 글자도 다르지 않게 둔다.
#
#   왜 지우면 안 되는가: 이 값은 아래 `_mutants()` 의 "자격증명 유출" 변형이 쓰는 **양성
#   표본**이다. `SECRET_PATS` 가 이것을 못 잡으면 `--self-test` 가 그 라벨을 blind 로 올린다.
#   지우면 자격증명 탐지기가 **한 번도 시험되지 않은 채** 초록이 된다(계측기의 자살).
#   조립이 깨져도 같은 검체가 즉시 적색이므로, 이 상수는 자기 자신의 파수꾼이기도 하다.
_FAKE_API_KEY = "sk-" + "ant-" + "api03-AAAAAAAABBBBBBBB"

_GOOD = {
    "schema": SCHEMA, "id": "probe-2026-01-02-sample", "title": "합성 표본",
    "measured_on": "2026-01-02", "tool": "self-test", "platform": "tmp",
    "repro": "python3 scripts/verify_evidence.py --self-test",
    "provenance": "self-test 가 만든 합성 표본(원측정 아님)",
    "observations": [{"fact": "합성", "evidence": "합성"}],
}
_README = ("# evidence\ncys-evidence/1 · retention · provenance · repro\n\n"
           "| 파일 | 측정일 | 무엇 |\n|---|---|---|\n"
           "| `probe-2026-01-02-sample.json` | 2026-01-02 | 합성 |\n")


def _mutants():
    """(라벨, docs 변형 함수) — 각 변형은 **반드시** 위반으로 잡혀야 한다."""
    def wr(d, **kw):
        x = dict(d)
        x.update(kw)
        return x

    def drop(d, key):
        x = dict(d)
        x.pop(key, None)
        return x

    return [
        ("필수 키 결손(repro)", lambda: ({"probe-2026-01-02-sample.json": drop(_GOOD, "repro")}, _README)),
        ("필수 키 결손(provenance)",
         lambda: ({"probe-2026-01-02-sample.json": drop(_GOOD, "provenance")}, _README)),
        ("관측 0건", lambda: ({"probe-2026-01-02-sample.json": wr(_GOOD, observations=[])}, _README)),
        ("추론인데 출처 없음",
         lambda: ({"probe-2026-01-02-sample.json":
                   wr(_GOOD, observations=[{"fact": "f", "evidence": "e", "derived": True}])}, _README)),
        ("스키마 불일치", lambda: ({"probe-2026-01-02-sample.json": wr(_GOOD, schema="x/9")}, _README)),
        ("id 불일치", lambda: ({"probe-2026-01-02-sample.json": wr(_GOOD, id="다른이름")}, _README)),
        ("측정일 불일치", lambda: ({"probe-2026-01-02-sample.json": wr(_GOOD, measured_on="2020-01-01")},
                              _README)),
        ("파일명 규약 위반", lambda: ({"aaa.json": _GOOD}, _README + "| `aaa.json` | x | y |\n")),
        ("자격증명 유출",
         lambda: ({"probe-2026-01-02-sample.json":
                   wr(_GOOD, tool=_FAKE_API_KEY)}, _README)),
        # ★자리표시자 선택 근거(2026-08-24) — 두 검체 모두 `secret-scan.sh` 가 **자기 소스에서**
        #   오탐하지 않는 형태를 쓰되, 여기서 시험하는 검사 축(HOME_PATS·EMAIL_RE)은 그대로다.
        #   · `/Users/x/…` = 스캐너가 `dummy_user_re` 에 등재한 더미 username(리포 관례 ·
        #     `ui/src/deptlabel.test.ts:33`). 이쪽 HOME_PATS 에는 더미 예외가 없으므로
        #     (`(?!<)` 뿐) 종전과 **똑같이** 잡힌다.
        #   · `example.invalid` = RFC 2606 예약 TLD(영원히 해석되지 않는다). 스캐너의 TLD 목록
        #     (com|net|org|io|dev) 밖이고, 이쪽 허용목록(@example.com·@anthropic.com·@claude.com)
        #     안에는 **없다** — 그래서 여전히 위반으로 잡혀야 한다.
        #   실주소·실경로로 되돌리지 마라. 검사 능력은 한 톨도 늘지 않고 발행만 막힌다.
        ("홈 절대경로",
         lambda: ({"probe-2026-01-02-sample.json":
                   wr(_GOOD, repro="cd /Users/x/dev && run")}, _README)),
        ("개인 이메일",
         lambda: ({"probe-2026-01-02-sample.json":
                   wr(_GOOD, provenance="측정자 someone@example.invalid")}, _README)),
        ("보존 위반 — 대장에 있는데 파일이 없다", lambda: ({}, _README)),
        ("보존 위반 — 파일이 있는데 대장에 없다",
         lambda: ({"probe-2026-01-02-sample.json": _GOOD,
                   "probe-2026-01-03-extra.json": wr(_GOOD, id="probe-2026-01-03-extra",
                                                     measured_on="2026-01-03")}, _README)),
        ("계약 문서 조항 삭제", lambda: ({"probe-2026-01-02-sample.json": _GOOD},
                                _README.replace("retention", "보존"))),
        ("증거 아닌 파일 혼입",
         lambda: ({"probe-2026-01-02-sample.json": _GOOD, "notes.txt": None}, _README)),
    ]


def self_test():
    """합성 표본으로 **탐지 능력 자체**를 시험한다. 반환 (blind[], total)."""
    blind = []
    total = 0
    with tempfile.TemporaryDirectory() as tmp:
        # ① 정상 표본은 통과해야 한다(오탐 0 — 오탐이 나면 이 게이트는 곧 꺼진다).
        d = os.path.join(tmp, "good")
        _materialize(d, {"probe-2026-01-02-sample.json": _GOOD}, _README)
        v, _ = check_dir(d)
        total += 1
        if v:
            blind.append("정상 표본 오탐: %s" % v[:3])
        # ② 각 변형은 반드시 잡혀야 한다.
        for i, (label, make) in enumerate(_mutants()):
            files, readme = make()
            d = os.path.join(tmp, "m%02d" % i)
            _materialize(d, files, readme)
            v, _ = check_dir(d)
            total += 1
            if not v:
                blind.append(label)
    return blind, total


def _materialize(d, files, readme):
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "README.md"), "w", encoding="utf-8", newline="\n") as f:
        f.write(readme)
    for name, body in files.items():
        with open(os.path.join(d, name), "w", encoding="utf-8", newline="\n") as f:
            f.write("not json\n" if body is None else json.dumps(body, ensure_ascii=False))


def main(argv=None):
    for _s in (sys.stdout, sys.stderr):
        try:
            _s.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    ap = argparse.ArgumentParser(description="docs/evidence 보존 계약 집행자")
    ap.add_argument("--dir", default=None, help="증거 디렉터리(기본: <repo>/docs/evidence)")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--self-test", action="store_true", help="합성 표본으로 탐지 능력 시험")
    a = ap.parse_args(argv)

    if a.self_test:
        blind, total = self_test()
        out = {"mode": "self-test", "cases": total, "blind": blind,
               "verdict": "GREEN" if not blind else "RED"}
        print(json.dumps(out, ensure_ascii=False, indent=1) if a.json else
              ("self-test %s — 합성 %d건 중 미탐지 %d건%s"
               % (out["verdict"], total, len(blind),
                  ("" if not blind else ": " + ", ".join(blind)))))
        return 0 if not blind else 1

    evdir = a.dir or os.path.join(_repo_root(), "docs", "evidence")
    v, stats = check_dir(evdir)
    out = {"mode": "check", "dir": evdir, "violations": v, "stats": stats,
           "verdict": "GREEN" if not v else "RED"}
    if a.json:
        print(json.dumps(out, ensure_ascii=False, indent=1))
    else:
        print("%s — 증거 %d건 · 관측 %d건 · 대장 %d행 · 위반 %d건"
              % (out["verdict"], stats["files"], stats["observations"], stats["indexed"], len(v)))
        for x in v:
            print("  - %s" % x)
    return 0 if not v else 1


if __name__ == "__main__":
    sys.exit(main())
