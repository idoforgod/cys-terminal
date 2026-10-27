"""G-1-GUARD (0.14.55-P8 H-10) — 릴리스 게이트의 태그-레인 판정이 **정확한지**를 지키는 검사.

게이트(`release.yml` 의 `gate_pop.py`)는 태그 레인을 이름으로 가른다:
`event == "push" ∧ fnmatch(head_branch, "v*")`. 런 payload 에 ref-type 필드가 없고 그 잡에는
checkout 이 없어 `git tag --points-at` 도 못 쓰기 때문이다(둘 다 실측). 이름 판정은 **조건부로만**
정확하다 — `windows-health.yml` 의 `on.push.branches` 중 어떤 패턴도 `v` 로 시작하는 이름을
만들어 낼 수 없을 때만. 그 조건이 깨지면 **여기서 붉어진다**.

⛔표본으로 증명하지 않는다. 후보 이름 몇 개를 넣어 보고 PASS 를 찍는 것은 **반례를 못 찾았다**는
뜻이지 두 언어가 서로소라는 뜻이 아니다(R2 의 guard 가 `v[ab]z` 에서 정확히 그렇게 틀렸다).
그래서 이 검사는 ⑴다룰 수 있는 문법으로 입력을 **제한**하고(밖이면 **거부**), ⑵그 안에서
**닫힌 형태**로 판정한다.

종료 코드: 0 PASS · 1 FAIL(충돌) · 3 REFUSED(판정 불가 — ⛔통과로 읽지 마라).

    python3 scripts/coupling_guard.py [<windows-health.yml>]
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_WF = os.path.normpath(os.path.join(HERE, "..", ".github", "workflows", "windows-health.yml"))
EXIT_PASS, EXIT_FAIL, EXIT_REFUSED = 0, 1, 3

# 닫힌 형태로 판정할 수 있는 글자만 — 리터럴 · `/` · `-` · `.` · `*`(`**` 포함).
DECIDABLE = re.compile(r"^[A-Za-z0-9_./*-]+$")


class Refused(Exception):
    """판정 불가. ⛔통과가 아니다."""


def _flow_list(value):
    """`['a', "b"]` 한 줄 목록 → 리스트. 모양이 조금이라도 다르면 거부한다."""
    v = value.strip()
    if not (v.startswith("[") and v.endswith("]")):
        raise Refused("inline 목록이 아니다: %r" % value)
    inner = v[1:-1].strip()
    if not inner:
        raise Refused("빈 목록이다 — 빈 값을 '충돌 없음'으로 읽지 않는다")
    items = []
    for part in inner.split(","):
        part = part.strip()
        if len(part) >= 2 and part[0] == part[-1] and part[0] in "'\"":
            items.append(part[1:-1])
        elif part and DECIDABLE.match(part):
            items.append(part)             # 따옴표 없는 스칼라
        else:
            raise Refused("목록 원소를 읽지 못했다: %r" % part)
    return items


def read_push_trigger(path):
    """`on.push` 의 `branches` 와 `tags` 를 읽는다 — inline 목록과 블록 목록 **둘 다**.

    ⛔fail-open 금지: 키가 없거나, 값을 읽지 못했거나, 일부만 읽혔으면 **거부**한다.
    (codex r3 minor 2: 종전 판은 `branches: ["feat/**","v*"]` 를 `[]` 로 읽고 PASS 를 냈다.)
    """
    with open(path, encoding="utf-8") as fh:
        lines = fh.read().replace("\r\n", "\n").split("\n")
    # `on:` → `push:` 블록을 들여쓰기로 찾는다(CI 의 파이썬에 PyYAML 이 없다).
    i = next((n for n, l in enumerate(lines) if re.match(r"^(on|\"on\"|'on'):\s*$", l)), None)
    if i is None:
        raise Refused("`on:` 매핑을 찾지 못했다")
    j = None
    for n in range(i + 1, len(lines)):
        l = lines[n]
        if l.strip() and not l.startswith(" "):
            break                                  # on: 블록의 끝
        if re.match(r"^\s+push:\s*$", l):
            j = n
            break
    if j is None:
        raise Refused("`on.push:` 매핑을 찾지 못했다")
    push_indent = len(lines[j]) - len(lines[j].lstrip())
    out = {}
    n = j + 1
    while n < len(lines):
        l = lines[n]
        if l.strip() == "" or l.strip().startswith("#"):
            n += 1
            continue
        ind = len(l) - len(l.lstrip())
        if ind <= push_indent:
            break                                  # push: 블록의 끝
        m = re.match(r"^\s+(branches|tags|branches-ignore|tags-ignore|paths|paths-ignore):\s*(.*)$", l)
        if not m:
            raise Refused("`on.push` 안에서 읽을 수 없는 줄: %r" % l)
        key, rest = m.group(1), m.group(2).split("#")[0].strip()
        if rest:
            out[key] = _flow_list(rest)            # inline 목록
            n += 1
            continue
        items, n = [], n + 1                       # 블록 목록
        while n < len(lines):
            b = lines[n]
            if b.strip() == "" or b.strip().startswith("#"):
                n += 1
                continue
            bm = re.match(r"^(\s+)-\s+(.*)$", b)
            if not bm or len(bm.group(1)) <= push_indent:
                break
            item = bm.group(2).split("#")[0].strip()
            if len(item) >= 2 and item[0] == item[-1] and item[0] in "'\"":
                item = item[1:-1]
            if not item:
                raise Refused("블록 목록 원소를 읽지 못했다: %r" % b)
            items.append(item)
            n += 1
        if not items:
            raise Refused("`%s:` 의 값을 읽지 못했다(빈 목록으로 접지 않는다)" % key)
        out[key] = items
    for forbidden in ("branches-ignore", "tags-ignore"):
        if forbidden in out:
            raise Refused("`%s` 가 있다 — 이 검사의 닫힌 형태 밖이다" % forbidden)
    if "branches" not in out or "tags" not in out:
        raise Refused("`on.push` 에 branches 와 tags 가 둘 다 있어야 한다 (읽은 키: %s)"
                      % sorted(out))
    return out


def admits_v_initial(pattern):
    """제한된 문법 안에서 **닫힌 형태로** 판정: 이 패턴이 `v` 로 시작하는 이름을 받을 수 있는가.

    `v` 로 시작하는 이름을 만들 수 있는 경우는 둘뿐이다 — 첫 글자가 리터럴 `v` 이거나,
    첫 글자가 와일드카드(`*`)여서 무엇이든 받을 수 있거나.
    """
    if not pattern:
        raise Refused("빈 패턴")
    if not DECIDABLE.match(pattern):
        raise Refused("닫힌 형태로 판정할 수 없는 문법: %r "
                      "(브래킷 클래스·`?`·중괄호·이스케이프는 이 검사의 밖이다)" % pattern)
    return pattern[0] in ("v", "*")


def check(path):
    """(exit_code, [lines]) — 부작용 없는 판정."""
    msg = []
    try:
        trig = read_push_trigger(path)
    except Refused as e:
        return EXIT_REFUSED, ["REFUSED  %s" % e,
                              "  ⛔G-1 의 정확성은 여기서 **미정**이다. 통과로 읽지 마라."]
    msg.append("on.push.branches = %r" % (trig["branches"],))
    msg.append("on.push.tags     = %r" % (trig["tags"],))
    if trig["tags"] != ["v*"]:
        return EXIT_REFUSED, msg + [
            "REFUSED  태그 트리거가 `['v*']` 가 아니다 — 게이트의 판정식을 다시 유도해야 한다.",
            "  ⛔G-1 의 정확성은 미정이다. 통과로 읽지 마라."]
    try:
        bad = [p for p in trig["branches"] if admits_v_initial(p)]
    except Refused as e:
        return EXIT_REFUSED, msg + ["REFUSED  %s" % e,
                                    "  ⛔G-1 의 정확성은 미정이다. 통과로 읽지 마라."]
    if bad:
        return EXIT_FAIL, msg + [
            "FAIL  `v` 로 시작하는 이름을 받을 수 있는 브랜치 패턴: %r" % (bad,),
            "  ⇒ `event==push ∧ head_branch~v*` 는 더 이상 태그 레인만 고르지 않는다."]
    return EXIT_PASS, msg + [
        "PASS  어떤 브랜치 패턴도 `v` 로 시작하는 이름을 만들 수 없다 ⇒ 이 핀에서 G-1 은 정확하다.",
        "  ⚠PASS 의 범위: **이 핀의, 판정 가능한 문법에 대한** 정확성이다. 모든 트리거 목록에"
        " 대한 주장이 아니다 — 그래서 이 검사가 게이트 옆에 등록돼 함께 돈다."]


SELFTEST_CASES = [
    # (branches/tags YAML 본문, 기대 종료 코드, 무엇을 지키는가)
    ("    branches: ['feat/**', 'fix/**']\n    tags: ['v*']\n", EXIT_PASS, "핀의 모양"),
    ("    branches: ['feat/**', 'v*']\n    tags: ['v*']\n", EXIT_FAIL, "inline 홑따옴표 충돌"),
    ('    branches: ["feat/**", "v*"]\n    tags: ["v*"]\n', EXIT_FAIL,
     "★inline 쌍따옴표 충돌 — 종전 판은 이것을 []로 읽고 PASS 를 냈다"),
    ("    branches:\n      - feat/**\n      - v*\n    tags:\n      - v*\n", EXIT_FAIL,
     "★블록 목록 충돌 — 종전 판은 이것도 []로 읽었다"),
    ("    branches: ['feat/**', 'fix/**', 'v[ab]z']\n    tags: ['v*']\n", EXIT_REFUSED,
     "브래킷 클래스 — 판정 불가이므로 거부"),
    ("    branches: ['feat/**', '**']\n    tags: ['v*']\n", EXIT_FAIL, "`**` 는 무엇이든 받는다"),
    ("    branches: []\n    tags: ['v*']\n", EXIT_REFUSED, "빈 목록을 충돌 없음으로 접지 않는다"),
    ("    branches: feat/**\n    tags: ['v*']\n", EXIT_REFUSED,
     "★목록이 아닌 스칼라 — 읽지 못한 값을 빈 목록으로 접으면 여기서 통과해 버린다"),
    ("    branches: {a: b}\n    tags: ['v*']\n", EXIT_REFUSED,
     "★매핑 — 같은 fail-open 경로"),
    ("    tags: ['v*']\n", EXIT_REFUSED, "branches 키 자체가 없다"),
    ("    branches: ['feat/**']\n    tags: ['v*', 'release-*']\n", EXIT_REFUSED,
     "태그 트리거의 언어가 바뀌었다"),
]


def selftest(tmpdir):
    """모든 갈래를 **단언**한다 — 찍어 보기만 하면 장식이다(codex r3 minor 2)."""
    import tempfile
    results = []
    for n, (body, want, why) in enumerate(SELFTEST_CASES, 1):
        p = os.path.join(tmpdir, "wf-%02d.yml" % n)
        with open(p, "w", encoding="utf-8") as fh:
            fh.write("name: t\non:\n  push:\n" + body + "jobs:\n  x:\n    runs-on: ubuntu-latest\n")
        rc, _ = check(p)
        ok = rc == want
        results.append((n, why, want, rc, ok))
        if not ok:
            raise AssertionError("self-test %d (%s): 기대 %d, 실제 %d" % (n, why, want, rc))
    return results


def main(argv):
    path = argv[1] if len(argv) > 1 else DEFAULT_WF
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        rows = selftest(d)
    print("self-test %d/%d 통과(PASS·FAIL·REFUSED 세 갈래 모두 단언됨)" % (len(rows), len(rows)))
    for n, why, want, rc, _ in rows:
        print("  #%d %-52s 기대=%d 실제=%d" % (n, why, want, rc))
    rc, lines = check(path)
    print("대상: %s" % path)
    for l in lines:
        print(l)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
