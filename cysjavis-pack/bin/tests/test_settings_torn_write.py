#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_settings_torn_write.py — '이어붙음'(Extra data) 서명의 출처를 못박는다 (0.14.30 W-A · master 지시 2차).

무엇을 막는가: release 레인 H-CONC-3 이 `JSONDecodeError: Extra data: line 1 column 52` 로
**판정이 아니라 크래시**로 죽었다. 그 서명은 두 가지를 동시에 뜻한다 —
  ⓐ 어떤 파일이 "완결 JSON + 잔여 꼬리" 가 됐다(잘림이 아니라 **이어붙음**),
  ⓑ 그것을 읽은 자리가 맨몸 `json.loads` 라 증거를 남기지 못하고 traceback 으로 죽었다.

기제(결정론): `open(p, 'w')` 의 truncate 는 **열 때** 일어나고 쓰기는 각 fd 의 offset 에서 난다.
  ① P1 이 연다(0으로 잘림) ② P2 가 연다(0으로 잘림) ③ P2 가 긴 본문을 쓴다
  ④ P1 이 짧은 본문을 자기 offset 0 에 쓴다 → 앞부분만 덮이고 뒤는 P2 의 꼬리로 남는다.
즉 **제자리 쓰기**(tmp+rename 없음)의 서명이며, `os.replace` 를 쓰는 writer 에서는 나오지 않는다.

이 검체가 고정하는 것:
  1 기제 재현 — 위 순서를 강제하면 정확히 `Extra data: line 1 column 52` 가 난다(청크 쓰기로 창 확대).
  2 생산 writer 무죄 — settings.json 을 쓰는 writer 전수가 **고유 tmp + os.replace** 다
    (제자리 쓰기 0 · 고정 `.tmp` 0). 이 서명은 생산 경로에서 나올 수 없다.
  3 서명 불가능성 — `_settings_rmw` 는 `indent=2` 로 쓴다. 1행이 `{` 하나이므로 '1행 52열'에
    완결 JSON 이 끝나는 일 자체가 불가능하다(=적색의 출처는 settings.json 이 아니다).
  4 오라클 내성 — H-CONC-3 의 **음성 대조군**(직렬화 없는 RMW = 파손이 정상 결과)을 읽는 자리가
    맨몸 `json.loads` 가 아니어야 한다. 파손은 **판정**(검출)이지 크래시가 아니다.

출력: PASS/FAIL 행 · 실패 시 exit 1 · 종료 토큰 SETTINGS-TORN-WRITE-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_settings_torn_write.py
"""
import io
import json
import os
import re
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


def _read(p):
    try:
        with io.open(p, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def _code(src):
    """주석·독스트링을 걷어낸 '코드 줄'만 — 금지 패턴을 설명하는 산문이 판별자를 통과하지
    않게 한다(2026-09-04 실측: 개정 사유를 적은 주석이 구본 판정을 통과시킨 적이 있다)."""
    out = []
    for line in src.split("\n"):
        st = line.strip()
        if st.startswith("#"):
            continue
        out.append(line)
    return "\n".join(out)


# ── 1. 기제 재현 (결정론 · 청크 쓰기로 창 확대) ──────────────────────────────────
d = tempfile.mkdtemp(prefix="torn-")
p = os.path.join(d, "naive.json")
# 음성 대조군 자식이 실제로 만드는 형상 그대로: 마크 5건 = 정확히 51바이트.
short = json.dumps({"marks": ["n0-0", "n1-0", "n2-0", "n3-0", "n0-1"]})
long_ = json.dumps({"marks": ["n0-0", "n1-0", "n2-0", "n3-0", "n0-1", "n1-1"]})
check("0 대조군 형상: 마크 5건 = 51바이트(=서명의 '52열'의 근거)", len(short) == 51,
      "len=%d" % len(short))

f1 = io.open(p, "w", encoding="utf-8")      # ① 열기 = truncate
f2 = io.open(p, "w", encoding="utf-8")      # ② 열기 = truncate
for i in range(0, len(long_), 8):           # ③ 긴 본문을 **청크로**(창 확대)
    f2.write(long_[i:i + 8])
    f2.flush()
for i in range(0, len(short), 8):           # ④ 짧은 본문이 앞부분만 덮는다
    f1.write(short[i:i + 8])
    f1.flush()
f1.close()
f2.close()
raw = _read(p)
err = None
try:
    json.loads(raw)
except ValueError as e:
    err = str(e)
check("1 제자리 쓰기가 '완결 JSON + 잔여 꼬리'를 만든다", err is not None,
      "파일=%r" % raw)
check("1b 서명이 CI 적색과 같다(Extra data: line 1 column 52)",
      bool(err) and "Extra data" in err and "line 1 column 52" in err, err or "(파싱 성공)")

# ── 2. 생산 writer 무죄 — settings.json writer 전수가 고유 tmp + replace ─────────
#    (master 지목 6종: preflight 등록기 · guard_register · dept_migrate · pack.rs 병합기 ·
#     factory_reset · init-pack 시드)
PY_WRITERS = ["javis_preflight.py", "javis_guard_register.py", "javis_dept_migrate.py"]
for mod in PY_WRITERS:
    mp = os.path.join(BIN, mod)
    if not os.path.isfile(mp):
        check("2 %s 실재" % mod, False, "파일 부재")
        continue
    code = _code(_read(mp))
    check("2 %s: 고정 .tmp 재구현 0" % mod, 'settings_path + ".tmp"' not in code)
    # 제자리 쓰기: settings 경로를 'w' 로 직접 여는 곳이 없어야 한다.
    inplace = re.findall(r"open\(\s*settings_path\s*,\s*['\"][wa]", code)
    check("2 %s: settings 제자리 쓰기 0" % mod, not inplace, repr(inplace))
    check("2 %s: mkstemp 경유" % mod, "mkstemp(" in code or "atomic_write" in code)

for rs, needle in (("src/pack.rs", "write_atomic(settings_path"),
                   ("src/bin/cys.rs", "std::fs::write(settings_path")):
    rp = os.path.join(REPO, rs)
    if not os.path.isfile(rp):
        print("SKIP 2 %s 부재(팩 단독 배포)" % rs)
        continue
    body = _code(_read(rp))
    if rs.endswith("pack.rs"):
        check("2 %s: 병합기가 원자 쓰기" % rs, needle in body)
        check("2 %s: 비원자 settings write 0" % rs, "std::fs::write(settings" not in body)
    else:
        check("2 %s: init-pack 비원자 write 0" % rs, needle not in body)

# ── 3. 서명 불가능성 — settings.json 은 indent=2 라 1행이 '{' 하나다 ──────────────
pf = _read(os.path.join(BIN, "javis_preflight.py"))
i = pf.find("def _settings_rmw(")
body = pf[i:pf.find("\ndef ", i + 10)] if i > 0 else ""
check("3 _settings_rmw 실재", i > 0)
check("3 settings 는 indent 로 쓴다(1행=중괄호 하나 → '1행 52열' 서명 불가)",
      "indent=indent" in body and "def _settings_rmw(settings_path, mutate, indent=2)" in pf,
      "indent 기본값 2")
probe = json.dumps({"theme": "dark", "hooks": {}}, ensure_ascii=False, indent=2)
check("3b 실증: indent=2 산출의 1행 길이 ≤ 2", len(probe.split("\n")[0]) <= 2,
      "1행=%r" % probe.split("\n")[0])

# ── 4. 오라클 내성 — 대조군을 읽는 자리가 맨몸 json.loads 가 아니다 ──────────────
hp = os.path.join(SELF, "run_bootstrap_health.py")
h = _read(hp)
check("4 run_bootstrap_health 실재", bool(h))
j = h.find("def h_conc_3(")
hb = h[j:h.find("\n@specimen", j + 10)] if j > 0 else ""
check("4 H-CONC-3 실재", bool(hb))
check("4 대조군 파손이 크래시가 아니라 검출이다(맨몸 json.loads 0)",
      "nmarks = (json.loads(_read(naive) or" not in hb,
      "맨몸 파싱이 남아 있으면 대조군 파손 시 검체가 traceback 으로 죽는다")
check("4b 대조군 읽기가 try/except 로 감싸여 있다",
      "nraw = _read(naive)" in hb and "except ValueError" in hb)
check("4c 자식 비정상종료를 관측한다(무직렬화의 또 다른 증거)",
      "ndied" in hb and "stderr=subprocess.PIPE" in hb)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("SETTINGS-TORN-WRITE-OK")
sys.exit(0)
