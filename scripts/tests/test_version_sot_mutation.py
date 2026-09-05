#!/usr/bin/env python3
"""버전 SOT 8곳 게이트의 **음성 대조** — 한 곳만 누락돼도 version-check.sh 가 죽는가.

왜 필요한가(2026-09-04 W-C R2 ②): `scripts/version-check.sh` 는 8곳이 모두 같으면 통과한다.
그런데 "현재 8값이 같다"는 확인만으로는 **"다음 릴리스에서 한 곳이 빠지면 자동 실패한다"** 를
증명하지 못한다 — 통과만 보이는 증거는 '언제나 초록인 검사'와 구별되지 않는다.
그래서 selector 8종을 하나씩 구버전으로 되돌린 변조 8종이 **전부 비영 종료**하는지 확인한다.

계약:
  · 실작업트리를 수정하지 않는다 — 필요한 파일만 임시 디렉터리로 복사해 거기서만 변조한다.
  · 판정은 selector 를 재구현하지 않고 version-check.sh **자기 출력의 8행 표**를 파싱한다.
    (재구현하면 스크립트가 바뀔 때 검사기가 조용히 갈라진다.)
  · 각 변조는 '정확히 1개 SOT 만 구버전'이어야 유효하다 — 그 조건까지 단언한다.
  · 양성 대조(무변조 사본은 통과)를 함께 돌린다. 없으면 '검사기가 항상 빨간' 경우를 못 본다.

실행: python3 scripts/tests/test_version_sot_mutation.py     (stdlib 전용 · 네트워크 무접촉)
종료: 0=전건 기대대로 · 1=하나라도 어긋남
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = "scripts/version-check.sh"
NEEDED = [SCRIPT, "Cargo.toml", "src-tauri/Cargo.toml", "src-tauri/tauri.conf.json",
          "ui/package.json", "dist-win/cys.wxs", "dist-win/cys-x64.wxs", "Cargo.lock"]

# (id, version-check.sh 표 라벨, 파일, 대상 행 찾는 규칙) — 규칙은 그 스크립트의 selector 와 동형.
MUTATIONS = [
    ("S1", "Cargo.toml", "Cargo.toml", ("first", lambda l: l.startswith("version"))),
    ("S2", "src-tauri/Cargo.toml", "src-tauri/Cargo.toml", ("first", lambda l: l.startswith("version"))),
    ("S3", "src-tauri/tauri.conf.json", "src-tauri/tauri.conf.json", ("first", lambda l: '"version"' in l)),
    ("S4", "ui/package.json", "ui/package.json", ("first", lambda l: '"version"' in l)),
    ("S5", "dist-win/cys.wxs", "dist-win/cys.wxs", ("first", lambda l: "Product" in l)),
    ("S6", "dist-win/cys-x64.wxs", "dist-win/cys-x64.wxs", ("first", lambda l: "Product" in l)),
    ("S7", "Cargo.lock [cys-terminal]", "Cargo.lock", ("after_name", "cys-terminal")),
    ("S8", "Cargo.lock [cys-app]", "Cargo.lock", ("after_name", "cys-app")),
]


def current_version():
    """실트리의 현재 버전 — 변조 전 값(NEW). 고정 상수로 박으면 범프 때마다 테스트가 죽는다."""
    for line in open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8"):
        if line.startswith("version"):
            m = re.search(r'"([^"]+)"', line)
            if m:
                return m.group(1)
    raise SystemExit("Cargo.toml 에서 version 을 못 읽었다 — 추출기 파손(fail-closed)")


def older(ver):
    """같은 형식의 '이전' 버전 문자열 하나 — 패치 자리를 1 내린다(실재 릴리스일 필요 없다)."""
    parts = ver.split(".")
    if len(parts) < 3 or not parts[-1].isdigit():
        return ver + "-old"
    parts[-1] = str(max(0, int(parts[-1]) - 1))
    return ".".join(parts)


def seed(dst):
    for rel in NEEDED:
        d = os.path.join(dst, rel)
        os.makedirs(os.path.dirname(d), exist_ok=True)
        shutil.copy2(os.path.join(ROOT, rel), d)


def apply_mutation(root, rel, rule, new, old):
    path = os.path.join(root, rel)
    lines = open(path, encoding="utf-8").read().split("\n")
    kind, arg = rule
    idx = None
    if kind == "first":
        for i, line in enumerate(lines):
            if arg(line):
                idx = i
                break
    else:  # after_name — `name = "<pkg>"` 다음의 첫 version 필드(lockver 와 같은 규칙)
        for i, line in enumerate(lines):
            if line.strip() == 'name = "%s"' % arg:
                for j in range(i + 1, min(i + 8, len(lines))):
                    if lines[j].split()[:1] == ["version"]:
                        idx = j
                        break
                break
    if idx is None:
        raise SystemExit("대상 행을 못 찾았다: %s (%r) — 파일 구조 변경 의심(fail-closed)" % (rel, rule))
    if new not in lines[idx]:
        raise SystemExit("대상 행에 현재 버전이 없다: %s" % lines[idx])
    lines[idx] = lines[idx].replace(new, old, 1)
    open(path, "w", encoding="utf-8").write("\n".join(lines))
    return idx + 1


def run(root, *args):
    p = subprocess.run(["sh", SCRIPT, *args], cwd=root, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def parse_rows(out):
    """version-check.sh 가 찍는 `  <라벨패딩30> <값>` 8행을 딕셔너리로."""
    rows = {}
    for line in out.split("\n"):
        if line.startswith("  ") and len(line) > 33 and not line.lstrip().startswith("↳"):
            label, value = line[2:32].strip(), line[33:].strip()
            if label:
                rows[label] = value
    return rows


def main():
    new = current_version()
    old = older(new)
    fails = []
    print("버전 SOT mutation 음성 대조 — NEW=%s OLD=%s" % (new, old))

    with tempfile.TemporaryDirectory(prefix="cys-vsot-") as tmp:
        # ── 양성 대조: 무변조 사본은 반드시 통과 (검사기가 무조건 빨갛지 않음을 먼저 증명) ──
        base = os.path.join(tmp, "pristine")
        seed(base)
        rc, out = run(base)
        rct, _ = run(base, "v" + new)
        rows = parse_rows(out)
        ok = rc == 0 and rct == 0 and len(rows) == 8 and all(v == new for v in rows.values())
        print("  [양성] 무변조 사본 rc=%d rc(tag)=%d 표 %d행 → %s" % (rc, rct, len(rows), "ok" if ok else "FAIL"))
        if not ok:
            fails.append("양성 대조 실패 — 무변조 사본이 통과하지 않는다: %r" % out[-400:])

        # ── 음성 대조 8종: 하나씩 구버전으로 되돌리면 전부 비영 종료해야 한다 ──
        for sid, label, rel, rule in MUTATIONS:
            work = os.path.join(tmp, "mut_" + sid)
            seed(work)
            line_no = apply_mutation(work, rel, rule, new, old)
            rc, out = run(work)
            rct, _ = run(work, "v" + new)
            rows = parse_rows(out)
            olds = [k for k, v in rows.items() if v == old]
            news = [k for k, v in rows.items() if v == new]
            single = olds == [label] and len(news) == 7
            nonzero = rc != 0 and rct != 0
            print("  [음성] %s %-28s %s:%-4d rc=%d rc(tag)=%d 단일변조=%s → %s"
                  % (sid, label, rel, line_no, rc, rct, single, "ok" if (single and nonzero) else "FAIL"))
            if not single:
                fails.append("%s: 단일 변조가 아니다(구버전 표기 %r · 신버전 %d행)" % (sid, olds, len(news)))
            if not nonzero:
                fails.append("%s: 누락을 만들었는데 게이트가 통과했다(rc=%d rc_tag=%d)" % (sid, rc, rct))

    print()
    if fails:
        for f in fails:
            print("FAIL " + f)
        print("=== %d건 어긋남 ===" % len(fails))
        return 1
    print("=== 양성 1 + 음성 8 = 9건 전건 기대대로 (8곳 중 어느 하나가 누락돼도 게이트가 죽는다) ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
