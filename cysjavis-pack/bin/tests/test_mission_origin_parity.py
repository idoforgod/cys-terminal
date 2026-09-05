#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_mission_origin_parity.py — 기계유래 판정표 fixture 파리티 검체 (0.14.30 W-A A3).

무엇을 막는가(명세 §2-3 · 빌드플랜 A3): 층0(harness)·층0-c(기동 명령문)·층1(배달 원장)·
층2(라벨) 판정이 python 과 Rust(`src/mission_gate.rs`) **두 벌**로 갈라지는 것. 갈라짐은
조용하다 — 양쪽 테스트가 각자 초록인 채로 층1 이 죽고, 무라벨 기계 push 가 오너 임무가 되어
자율주행이 잔무 큐로 달린다(2026-08-01 사고 기제). 그래서 판정표를 **한 파일**
(`tests/fixtures/mission-origin-corpus.json`)에 두고 양쪽이 그것만 소비하게 못박는다.
구체적으로 다음 다섯 가지를 막는다:
  ① fixture 와 코드 상수(`ANOMALY_CODES`·`HARNESS_MARKERS`·수치)가 갈리는 것 — 파리티 대상
     목록 자체가 틀리면 그 아래 모든 대조는 무의미하다.
  ② fixture 가 **한 방향만** 재는 것(전부 접기·전부 통과시키기가 초록으로 지나가는 형태).
  ③ 발행되는 이상징후 코드가 층1 코퍼스에서 **한 번도 관측되지 않는 것**(등재만 되고 미검증).
  ④ `record` deprecated 경고가 **stdout 을 오염**시키는 것 — 훅이 stdout 을 라인 프로토콜로
     파싱한다(hooks/role-bootstrap-legacy.sh). 경고가 평시 경로(`hook-triage`)로 새는 것도.
  ⑤ 계측 타당성 미증명 — 구현/ fixture 를 무력화한 변조본이 이 검체를 통과하는 것.

밀폐: 모든 서브프로세스는 `CYS_PACK_DIR`·`CYS_STATE_DIR` 를 임시 디렉터리로 덮고
`CYS_MISSION`·`CYS_DELIVERY_WINDOW_S`·`CYS_MISSION_TTL_S`·`AITERM_SURFACE_ID` 를 벗긴다 —
라이브 팩·실 사용자 임무 대장·배달 원장 무접촉이고 데몬 스폰은 0이다. 변조본은 임시
디렉터리 안에 `javis_mission.py` + `tests/fixtures/` 사본으로 만들고 `PYTHONPATH` 로 형제
모듈(javis_detect·javis_bootstrap)만 실팩에서 빌려 쓴다(실팩 파일 쓰기 0).

  1  fixture 실재·판독·최상위 스키마(4개 층 + $constants + anomaly_codes + harness_markers)
  2  ★ANOMALY_CODES ↔ fixture anomaly_codes **양방향 정확 일치**(티켓 A3 ③)
  3  HARNESS_NOTIFY/CONTEXT_MARKERS ↔ fixture harness_markers **순서까지** 일치
  4  $constants ↔ 모듈 상수(env 를 벗긴 서브프로세스에서 무조건 잰다 — 창/TTL 기본값 포함)
  5  fixture ⊇ 내장 MISSION_CORPUS_FALLBACK(단일 원본 불변식 · 이름·입력·기대값 전부)
  6  케이스명 유일 · 축마다 **양방향**(접힘 1건 이상 ∧ 통과 1건 이상)
  7  env 유래 3종을 뺀 이상징후 코드 전수가 층1 코퍼스에서 **실제로 관측**된다(미검증 0)
  8  `--self-test` 가 fixture 를 소비하고 exit 0(밀폐 실행)
  9  `record` deprecated: stderr 1줄 · **stdout 0바이트** · exit 계약(0/2) 무변경
 10  `hook-triage` 는 경고 무발행 + stdout 라인 프로토콜(record→machine-origin→path) 무변형
 11  ★음성 대조 a — fixture 기대값을 뒤집은 사본은 self-test 를 **통과하지 못한다**
 12  ★음성 대조 b — anomaly_codes 1건을 뺀 사본은 열거 파리티에서 잡힌다
 13  ★음성 대조 c — 구조 축 발화조건(`== 0`)을 무력화한 소스 사본이 층0 케이스에서 잡힌다
 14  ★음성 대조 d — deprecation 줄을 지운 소스 사본은 경고 토큰을 내지 않는다

출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 MISSION-ORIGIN-PARITY-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_mission_origin_parity.py
"""
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
MISSION = os.path.join(BIN, "javis_mission.py")
FIXTURE = os.path.join(SELF, "fixtures", "mission-origin-corpus.json")
PY = sys.executable or "python3"
SECTIONS = ("layer0", "layer0c", "layer1", "layer2")
DEPRECATION_TOKEN = "★deprecated: `record`"
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def make_env(tmp):
    """밀폐 env — 라이브 팩·실 대장·실 원장 무접촉. 창/TTL env 는 반드시 벗긴다:
    남아 있으면 import 시점에 env 이상징후가 심겨 층1 기대값이 통째로 어긋난다."""
    env = dict(os.environ)
    env["CYS_PACK_DIR"] = os.path.join(tmp, "pack")
    env["CYS_STATE_DIR"] = os.path.join(tmp, "state")
    env["PYTHONPATH"] = BIN + os.pathsep + env.get("PYTHONPATH", "")
    for k in ("CYS_MISSION", "CYS_DELIVERY_WINDOW_S", "CYS_MISSION_TTL_S",
              "AITERM_SURFACE_ID", "JAVIS_PACK_DIR", "AITERM_PACK_DIR"):
        env.pop(k, None)
    env["CYS_SURFACE_ID"] = "parity"
    return env


def run(args, env, script=None, stdin=None):
    return subprocess.run([PY, script or MISSION] + args, capture_output=True, text=True,
                          timeout=180, env=env, input=stdin)


def make_mutant(root, tag, src_text, fixture_obj):
    """변조본 1벌 = <root>/<tag>/javis_mission.py + 그 아래 tests/fixtures/<fixture>.

    fixture 경로는 모듈이 `__file__` 상대로 찾으므로(mission_corpus_path), 사본을 함께 깔아야
    '부재 → 내장 폴백'으로 조용히 새지 않는다."""
    d = os.path.join(root, tag)
    os.makedirs(os.path.join(d, "tests", "fixtures"))
    with io.open(os.path.join(d, "javis_mission.py"), "w", encoding="utf-8", newline="\n") as f:
        f.write(src_text)
    with io.open(os.path.join(d, "tests", "fixtures", "mission-origin-corpus.json"),
                 "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(fixture_obj, ensure_ascii=False, indent=1) + "\n")
    return os.path.join(d, "javis_mission.py")


# ── 1 fixture 실재·판독·스키마 ───────────────────────────────────────────────
check("1a fixture 실재", os.path.isfile(FIXTURE), FIXTURE)
data = None
if os.path.isfile(FIXTURE):
    try:
        with io.open(FIXTURE, encoding="utf-8") as f:
            data = json.load(f)
    except Exception as e:
        check("1b fixture 판독", False, repr(e))
if data is None:
    print("\nfixture 를 읽지 못해 이후 대조 불가")
    sys.exit(1)
check("1b fixture 판독", True)
check("1c 최상위 스키마",
      isinstance(data, dict) and all(isinstance(data.get(s), list) and data[s] for s in SECTIONS)
      and isinstance(data.get("$constants"), dict)
      and isinstance(data.get("anomaly_codes"), list)
      and isinstance(data.get("harness_markers"), dict)
      and isinstance(data.get("$doc"), list) and len(data["$doc"]) >= 5,
      repr(sorted(data.keys()) if isinstance(data, dict) else type(data)))

sys.path.insert(0, BIN)
os.environ.pop("CYS_MISSION", None)
os.environ.pop("CYS_DELIVERY_WINDOW_S", None)
os.environ.pop("CYS_MISSION_TTL_S", None)
import javis_mission as jm                                             # noqa: E402

# ── 2 ★ANOMALY_CODES 열거 파리티(양방향) ─────────────────────────────────────
fx_codes, code_codes = set(data["anomaly_codes"]), set(jm.ANOMALY_CODES)
check("2a fixture 에만 있는 이상징후 코드 0", not (fx_codes - code_codes),
      repr(sorted(fx_codes - code_codes)))
check("2b 등재소에만 있는 이상징후 코드 0", not (code_codes - fx_codes),
      repr(sorted(code_codes - fx_codes)))
check("2c 코드 개수 일치(중복 등재 0)", len(data["anomaly_codes"]) == len(fx_codes) == len(code_codes),
      "fixture=%d(uniq %d) 코드=%d" % (len(data["anomaly_codes"]), len(fx_codes), len(code_codes)))

# ── 3 harness 마커 열거 파리티(순서까지) ─────────────────────────────────────
check("3a notify 마커 순서까지 일치",
      list(data["harness_markers"].get("notify") or []) == list(jm.HARNESS_NOTIFY_MARKERS),
      repr(data["harness_markers"].get("notify")))
check("3b context 마커 순서까지 일치",
      list(data["harness_markers"].get("context") or []) == list(jm.HARNESS_CONTEXT_MARKERS),
      repr(data["harness_markers"].get("context")))

# ── 4 $constants 파리티(창·TTL 기본값 포함 — 이 프로세스는 env 를 벗겼다) ────
LIVE_CONSTS = {
    "SCHEMA_VERSION": jm.SCHEMA_VERSION, "MISSION_MIN_CHARS": jm.MISSION_MIN_CHARS,
    "MISSION_MAX_CHARS": jm.MISSION_MAX_CHARS,
    "HARNESS_SCAN_MAX_CHARS": jm.HARNESS_SCAN_MAX_CHARS,
    "HARNESS_SCAN_PREFIX_CHARS": jm.HARNESS_SCAN_PREFIX_CHARS,
    "PREVIEW_CHARS": jm.PREVIEW_CHARS, "PART_PREVIEW_CHARS": jm.PART_PREVIEW_CHARS,
    "DELIVERY_PART_MIN_CHARS": jm.DELIVERY_PART_MIN_CHARS,
    "DELIVERY_WITHIN_MIN_CHARS": jm.DELIVERY_WITHIN_MIN_CHARS,
    "DELIVERY_SPAN_OCC_BUDGET": jm.DELIVERY_SPAN_OCC_BUDGET,
    "DELIVERY_CAPPED_FOLD_S": jm.DELIVERY_CAPPED_FOLD_S,
    "DELIVERY_SCAN_LINES": jm.DELIVERY_SCAN_LINES,
    "LEDGER_MAX_READ_BYTES": jm.LEDGER_MAX_READ_BYTES,
    "DELIVERY_WINDOW_S_DEFAULT": jm.DELIVERY_WINDOW_S,
    "MISSION_TTL_S_DEFAULT": jm.MISSION_TTL_S,
}
bad = {k: (data["$constants"].get(k), v) for k, v in LIVE_CONSTS.items()
       if data["$constants"].get(k) != v}
check("4a $constants 전건 일치(창·TTL 기본값 포함)", not bad, repr(bad))
check("4b $constants 에 미지 키 0",
      not (set(data["$constants"]) - set(LIVE_CONSTS)),
      repr(sorted(set(data["$constants"]) - set(LIVE_CONSTS))))

# ── 5 fixture ⊇ 내장 리터럴(단일 원본 불변식) ────────────────────────────────
missing = []
for sec in SECTIONS:
    have = {c["name"]: c for c in data[sec]}
    for lit in jm.MISSION_CORPUS_FALLBACK[sec]:
        got = have.get(lit["name"])
        if got is None:
            missing.append("%s/%s 부재" % (sec, lit["name"]))
            continue
        for k, v in lit.items():
            if got.get(k) != v:
                missing.append("%s/%s 필드 %s 불일치" % (sec, lit["name"], k))
check("5 fixture ⊇ 내장 MISSION_CORPUS_FALLBACK", not missing, repr(missing))

# ── 6 케이스명 유일 · 축마다 양방향 ──────────────────────────────────────────
dupes = []
for sec in SECTIONS:
    names = [c["name"] for c in data[sec]]
    dupes += ["%s/%s" % (sec, n) for n in names if names.count(n) > 1]
check("6a 케이스명 유일(층 안)", not dupes, repr(sorted(set(dupes))))
BOTH = (("layer0", "harness"), ("layer0c", "boot_command"),
        ("layer1", "machine"), ("layer2", "label"))
oneway = []
for sec, field in BOTH:
    vals = set(c["expect"].get(field) for c in data[sec] if field in c["expect"])
    if vals != {True, False}:
        oneway.append("%s.%s=%r" % (sec, field, sorted(vals, key=repr)))
check("6b 축마다 양방향(접힘·통과 각 1건 이상)", not oneway, repr(oneway))

# ── 7 이상징후 코드 전수가 층1 코퍼스에서 실제로 관측된다 ────────────────────
#    env_* 3종은 **import 시점**(env 오버라이드 판독)에만 발행되므로 층1 코퍼스로는 관측할 수
#    없다 — 그 사실을 이름 규칙으로 명시하고 나머지 전수를 요구한다.
observed = set()
for c in data["layer1"]:
    observed |= set(c["expect"].get("anomalies") or [])
expect_observable = set(k for k in jm.ANOMALY_CODES if not k.startswith("env_"))
check("7 층1 코퍼스가 env 외 이상징후 %d종을 전수 관측" % len(expect_observable),
      expect_observable <= observed, repr(sorted(expect_observable - observed)))

root = tempfile.mkdtemp(prefix="mission-parity-")
try:
    # ── 8 --self-test 가 fixture 를 소비하고 통과 ────────────────────────────
    env = make_env(os.path.join(root, "st"))
    r = run(["--self-test"], env)
    check("8a --self-test exit 0", r.returncode == 0, repr(r.stdout[-400:] + r.stderr[-400:]))
    check("8b fixture 부재 폴백으로 새지 않았다(NOTE 무발화)",
          "mission-origin-corpus fixture 부재" not in r.stderr, repr(r.stderr[-200:]))
    check("8c 소비 사실이 요약에 남는다",
          "mission-origin-corpus.json" in r.stdout, repr(r.stdout[-200:]))

    # ── 9 record deprecated — stderr 1줄 · stdout 0바이트 · exit 계약 무변경 ──
    env9 = make_env(os.path.join(root, "rec"))
    r9 = run(["record"], env9, stdin=json.dumps({"prompt": "윈도우 실사고 T1 근본수정 진행해"}))
    check("9a record stderr 에 deprecation 1줄", DEPRECATION_TOKEN in r9.stderr,
          repr(r9.stderr[-300:]))
    check("9b record stdout 0바이트(라인 프로토콜 무오염)", r9.stdout == "", repr(r9.stdout))
    check("9c deprecation 은 정확히 1줄",
          sum(1 for ln in r9.stderr.splitlines() if DEPRECATION_TOKEN in ln) == 1,
          repr([ln for ln in r9.stderr.splitlines() if DEPRECATION_TOKEN in ln]))
    check("9d record exit=0(임무 기록 → EXIT_HAVE) — 경고가 exit 를 바꾸지 않았다",
          r9.returncode == 0, repr((r9.returncode, r9.stderr[-200:])))
    r9b = run(["record"], make_env(os.path.join(root, "rec2")), stdin="{깨진 JSON")
    check("9e record 파싱 실패 exit=2(EXIT_UNREADABLE · fail-closed 계약 보존)",
          r9b.returncode == 2 and r9b.stdout == "", repr((r9b.returncode, r9b.stdout)))
    check("9f 대체 경로가 경고 문안에 실린다",
          "cys hook user-prompt-submit" in r9.stderr and "hook-triage" in r9.stderr)

    # ── 10 hook-triage 는 경고 무발행 + stdout 프로토콜 무변형 ────────────────
    env10 = make_env(os.path.join(root, "triage"))
    r10 = run(["hook-triage"], env10, stdin=json.dumps({"prompt": "윈도우 실사고 T1 근본수정 진행해"}))
    check("10a hook-triage 에 deprecation 무발행(평시 훅 경로가 시끄러워지지 않는다)",
          DEPRECATION_TOKEN not in r10.stderr, repr(r10.stderr[-300:]))
    lines = [ln for ln in r10.stdout.splitlines() if ln.strip()]
    check("10b stdout 라인 프로토콜 순서 record→machine-origin→path 무변형",
          len(lines) == 3 and lines[0].startswith("record: rc=")
          and lines[1].startswith("machine-origin: ") and lines[2].startswith("path: "),
          repr(lines))

    # ── 11 ★음성 대조 a — fixture 기대값을 뒤집은 사본 ───────────────────────
    src = io.open(MISSION, encoding="utf-8").read()
    fx_flip = json.loads(json.dumps(data))
    flipped = []
    for sec, field in BOTH:
        lit_names = set(l["name"] for l in jm.MISSION_CORPUS_FALLBACK[sec])
        for c in fx_flip[sec]:
            if c["name"] not in lit_names and field in c["expect"]:
                c["expect"][field] = not c["expect"][field]
                flipped.append("%s/%s" % (sec, c["name"]))
                break
    check("11a 변조 앵커 실재(층 4개 전부)", len(flipped) == 4, repr(flipped))
    m11 = make_mutant(root, "mut_fx", src, fx_flip)
    r11 = run(["--self-test"], make_env(os.path.join(root, "m11")), script=m11)
    hit = [n for n in flipped if ("mission-origin-corpus %s:" % n) in r11.stdout + r11.stderr]
    check("11b 뒤집은 fixture 는 self-test 를 통과하지 못한다(계측 타당성)",
          r11.returncode != 0 and len(hit) == 4, repr((r11.returncode, hit)))

    # ── 12 ★음성 대조 b — anomaly_codes 1건 제거 ─────────────────────────────
    fx_drop = json.loads(json.dumps(data))
    dropped = fx_drop["anomaly_codes"].pop()
    m12 = make_mutant(root, "mut_codes", src, fx_drop)
    r12 = run(["--self-test"], make_env(os.path.join(root, "m12")), script=m12)
    check("12 anomaly_codes 1건을 빼면 열거 파리티가 잡는다",
          r12.returncode != 0
          and ("mission-origin-corpus anomaly_codes/%s" % dropped) in r12.stdout + r12.stderr,
          repr((dropped, r12.returncode)))

    # ── 13 ★음성 대조 c — 구조 축 발화조건 무력화 ────────────────────────────
    anchor = "if nblocks and _meaningful_chars(free) == 0:"
    check("13a 구조 축 변조 앵커 실재", src.count(anchor) == 1, repr(src.count(anchor)))
    m13 = make_mutant(root, "mut_axis", src.replace(
        anchor, "if nblocks and _meaningful_chars(free) < MISSION_MIN_CHARS:", 1), data)
    r13 = run(["--self-test"], make_env(os.path.join(root, "m13")), script=m13)
    check("13b 발화조건을 `< MISSION_MIN_CHARS` 로 되돌리면 층0 케이스가 잡는다"
          "(오너 프롬프트 삼킴 재발 방지 핀)",
          r13.returncode != 0
          and "mission-origin-corpus layer0/L0-generic-block-with-free-text:" in
              r13.stdout + r13.stderr,
          repr((r13.returncode, r13.stderr[-300:])))

    # ── 14 ★음성 대조 d — deprecation 줄 제거 ────────────────────────────────
    i0 = src.find('    sys.stderr.write("[mission] ★deprecated: `record`')
    i1 = src.find("    return _record_step(_read_hook_stdin())")
    check("14a deprecation 변조 앵커 실재", 0 < i0 < i1, repr((i0, i1)))
    m14 = make_mutant(root, "mut_dep", src[:i0] + src[i1:], data)
    r14 = run(["record"], make_env(os.path.join(root, "m14")), script=m14,
              stdin=json.dumps({"prompt": "윈도우 실사고 T1 근본수정 진행해"}))
    check("14b 줄을 지운 사본은 경고 토큰을 내지 않는다(이 검체가 재는 것이 그 줄이다)",
          DEPRECATION_TOKEN not in r14.stderr and r14.returncode == 0,
          repr((r14.returncode, r14.stderr[-200:])))
finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("MISSION-ORIGIN-PARITY-OK")
sys.exit(0)
