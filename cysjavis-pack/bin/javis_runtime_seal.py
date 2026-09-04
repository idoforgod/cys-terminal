#!/usr/bin/env python3
"""javis_runtime_seal.py — 동봉 런타임 트리 봉인 매니페스트 (부트 v2 티켓 C1 · 명세 §2-10 G5).

무엇을 푸는가
  설치본의 `runtime/**`(Python·git·uv·node 동봉 트리)가 설치 후에 **변조**되는 계급을 검출한다.
  실사고 형태는 추상적 위협이 아니다 — 2026-09-04 실측: 오너 머신의 `/Applications/cys.app` 에
  `npm i -g @openai/codex` 가 **번들 안으로** 설치되어 코드서명 봉인이 깨져 있었다
  (`codesign --verify` → "a sealed resource is missing or invalid" + file added 11줄).
  같은 오염이 Windows 에서 일어나면 **탐지 수단이 아예 없다** — 코드서명 봉인이 없기 때문이다.
  이 모듈이 그 공백을 메운다.

왜 mac 에도 두는가 (정직한 위치 고지)
  mac 에서 이 매니페스트는 **코드서명보다 약하다**. `Contents/_CodeSignature/CodeResources` 가
  이미 runtime/** 의 파일별 해시 목록이고(실측: "runtime/" 참조 15,787건), 서명 키 없이는
  재생성할 수 없다. 반면 이 매니페스트는 공격자가 다시 만들 수 있다. 그래서 명세 §2-10 은
  mac 에서 `codesign --verify --deep --strict` **병기**를 요구한다 — 여기서도 그 관계를 뒤집지
  않는다. mac 에서 이 파일의 값어치는 "이식 가능하고 사람이 읽을 수 있는 층"이지 권위가 아니다.
  **권위는 Windows 에서만 이 파일에 있다.**

계약
  · 읽기 전용이다. 이 모듈은 어떤 경우에도 파일을 만들거나 지우거나 고치지 않는다
    (`emit` 의 산출 파일 1개만이 유일한 쓰기다). 자동 삭제 0 — 명세 §2-10.
  · 심볼릭링크를 **파일로 접지 않는다**. 배송 트리에는 심링크가 157개 있고(실측 v0.14.29),
    그중 node/bin/{npm,npx,corepack} 는 링크가 실복사본으로 바뀌면 MODULE_NOT_FOUND 로
    모든 번들 npm 호출이 깨진다(scripts/restore-runtime-symlinks.sh:11-13). `find -type f` 는
    심링크를 조용히 건너뛰므로 그 실패 모드에 **구조적으로 눈이 먼다**. 따라서 심링크는
    `link:<target>` 으로 따로 봉인한다. 반대로 `followlinks=True` 로 걸으면 git-core 링크 145개가
    각 3.4MB 실파일로 중복 계수된다 — 그것도 하지 않는다.
  · 판정은 3분류다: `missing`(매니페스트에 있는데 디스크에 없음) · `added`(디스크에 있는데
    매니페스트에 없음) · `changed`(둘 다 있는데 값이 다름 — 해시·링크대상·종류 변경 포함).
  · 결정론: 키 정렬 + 압축 구분자. 같은 트리 → 같은 바이트. (뮤테이션 검체가 이것에 의존한다.)
  · `digest` 는 JSON 표기와 무관하게 항목 목록만으로 계산한다(포매팅 변경에 흔들리지 않는다).

번들 안 실행 금지
  이 모듈은 대상 트리의 파일을 **읽기만** 한다. 절대 exec 하지 않는다 — .app 안 바이너리를 한 번
  이라도 실행하면 macOS 가 앱 번들 보호를 걸어 SIGKILL·codesign "Operation not permitted" 를
  만든다(2026-08-01 실측 · scripts/precompile-bundled-python.sh:45-49). 호출측은 **번들 밖**
  python3 로 이 모듈을 돌려야 한다(scripts/release-gate-gatekeeper.sh 의 $GATE_PY 선택과 동형).

사용
  emit:   python3 javis_runtime_seal.py emit --root <runtime dir> --out <manifest.json>
                                             [--app-version V] [--source <라벨>]
  verify: python3 javis_runtime_seal.py verify --root <runtime dir> --manifest <manifest.json>
                                               [--json] [--max-list N]
          exit 0=일치 · 1=불일치(3분류 출력) · 2=판정 불가(대상 부재·판독 실패 — 통과 아님)
"""

import argparse
import hashlib
import json
import os
import stat
import sys

SCHEMA = 1
MANIFEST_BASENAME = "runtime-manifest.json"

# 판독 청크. 726MB 트리에서 1MB 청크로 전수 sha256 이 웜캐시 0.49s(실측 2026-09-04) —
# 표본화가 필요 없다. 표본화는 "측정한 척"이라 여기서는 쓰지 않는다.
_CHUNK = 1 << 20


def _sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(_CHUNK)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def walk_entries(root, exclude=()):
    """트리를 걸어 `{상대경로: 항목}` 을 만든다. 상대경로 구분자는 항상 '/'(플랫폼 중립).

    항목 종류
      {"t": "f", "sha256": "...", "x": bool}   일반 파일 (x = 실행 비트 보유)
      {"t": "l", "target": "..."}              심볼릭링크 (대상 문자열 그대로 · 해소하지 않음)
      {"t": "o", "mode": "0o..."}              그 밖(fifo·소켓·디바이스) — 배송 트리엔 없어야 한다

    `exclude` 는 루트 기준 상대경로 집합. 매니페스트 자신이 트리 안에 놓이는 사고를 막는다.
    """
    entries = {}
    root = os.path.abspath(root)

    def _raise(err):
        """★`os.walk` 는 기본값(onerror=None)에서 디렉터리 판독 오류를 **조용히 삼킨다**.
        그러면 권한 없는 디렉터리가 통째로 스캔에서 빠져 그 아래 항목이 전부 '누락(missing)'
        으로 보고된다 — 즉 **판정 불가가 봉인 파손으로 둔갑**한다. 여기서 올려 보내
        호출측이 exit 2 로 닫게 한다(R1 codex #7)."""
        raise err

    for dirpath, dirnames, filenames in os.walk(root, followlinks=False, onerror=_raise):
        dirnames.sort()
        filenames.sort()
        # os.walk 는 심링크 디렉토리를 dirnames 에 넣고 followlinks=False 면 내려가지 않는다.
        # 그 디렉토리 심링크 자신도 봉인 대상이므로 여기서 항목으로 거둔다(누락 방지).
        for name in list(dirnames):
            p = os.path.join(dirpath, name)
            if os.path.islink(p):
                rel = os.path.relpath(p, root).replace(os.sep, "/")
                if rel not in exclude:
                    entries[rel] = {"t": "l", "target": os.readlink(p)}
        for name in filenames:
            p = os.path.join(dirpath, name)
            rel = os.path.relpath(p, root).replace(os.sep, "/")
            if rel in exclude:
                continue
            if os.path.islink(p):
                entries[rel] = {"t": "l", "target": os.readlink(p)}
                continue
            st = os.lstat(p)
            if stat.S_ISREG(st.st_mode):
                entries[rel] = {
                    "t": "f",
                    "sha256": _sha256_file(p),
                    "x": bool(st.st_mode & 0o111),
                }
            else:
                entries[rel] = {"t": "o", "mode": oct(stat.S_IMODE(st.st_mode))}
    return entries


def _canonical_line(rel, ent):
    """digest 계산용 정규 1줄. JSON 포매팅·키 순서와 독립이다."""
    t = ent.get("t")
    if t == "f":
        return "%s\0f\0%s\0%s" % (rel, ent.get("sha256", ""), "1" if ent.get("x") else "0")
    if t == "l":
        return "%s\0l\0%s" % (rel, ent.get("target", ""))
    return "%s\0o\0%s" % (rel, ent.get("mode", ""))


def entries_digest(entries):
    h = hashlib.sha256()
    for rel in sorted(entries):
        h.update(_canonical_line(rel, entries[rel]).encode("utf-8"))
        h.update(b"\n")
    return h.hexdigest()


def build_manifest(root, app_version="", source=""):
    entries = walk_entries(root, exclude={MANIFEST_BASENAME})
    files = sum(1 for e in entries.values() if e.get("t") == "f")
    links = sum(1 for e in entries.values() if e.get("t") == "l")
    other = len(entries) - files - links
    return {
        "schema": SCHEMA,
        "runtime_root": "runtime",
        "app_version": app_version,
        "source": source,
        "counts": {"files": files, "symlinks": links, "other": other},
        "entries": entries,
        "digest": entries_digest(entries),
    }


def dumps(manifest):
    """결정론 직렬화 — 같은 트리는 같은 바이트."""
    return json.dumps(manifest, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":")) + "\n"


def classify(manifest, root):
    """매니페스트 ↔ 실제 트리 3분류. 어떤 파일도 건드리지 않는다."""
    want = manifest.get("entries") or {}
    have = walk_entries(root, exclude={MANIFEST_BASENAME})
    missing = sorted(set(want) - set(have))
    added = sorted(set(have) - set(want))
    changed = sorted(
        rel for rel in (set(want) & set(have))
        if _canonical_line(rel, want[rel]) != _canonical_line(rel, have[rel])
    )
    return {"missing": missing, "added": added, "changed": changed}


def load_manifest(path):
    with open(path, "r", encoding="utf-8") as f:
        m = json.load(f)
    if not isinstance(m, dict) or "entries" not in m:
        raise ValueError("매니페스트 형식 아님(entries 부재): %s" % path)
    got = m.get("schema")
    if got != SCHEMA:
        raise ValueError("매니페스트 schema %r — 이 도구는 %d 만 판독한다" % (got, SCHEMA))
    return m


def _cmd_emit(a):
    if not os.path.isdir(a.root):
        print("✗ 대상 트리 없음: %s" % a.root, file=sys.stderr)
        return 2
    # ★판독 실패는 traceback 이 아니라 **판정 불가(exit 2)** 다(R1 codex #7).
    #   권한 거부·Windows 공유위반(WinError 32)·경로 소멸은 "봉인이 깨졌다"가 아니라
    #   "재지 못했다"이며, 그 둘을 같은 코드로 내보내면 소비자가 구분할 수 없다.
    try:
        m = build_manifest(a.root, app_version=a.app_version or "", source=a.source or "")
        out = os.path.abspath(a.out)
        os.makedirs(os.path.dirname(out), exist_ok=True)
        with open(out, "w", encoding="utf-8") as f:
            f.write(dumps(m))
    except OSError as e:
        print("✗ 트리 판독·산출 실패(%s) — 판정 불가(측정 불능은 통과가 아니다)" % e,
              file=sys.stderr)
        return 2
    c = m["counts"]
    print("✓ runtime-manifest: 파일 %d · 심링크 %d · 기타 %d · digest %s → %s"
          % (c["files"], c["symlinks"], c["other"], m["digest"][:16], out))
    if c["other"]:
        print("  ⚠ 비정규 엔트리 %d개 — 배송 트리에 있어선 안 된다(fifo·소켓·디바이스)" % c["other"])
    return 0


def _cmd_verify(a):
    if not os.path.isdir(a.root):
        print("✗ 대상 트리 없음: %s — 판정 불가(측정 불능은 통과가 아니다)" % a.root, file=sys.stderr)
        return 2
    # ★단일 판정 불가 경계(R2 codex #5). 매니페스트 판독과 트리 대조를 **한 경계**로 감싼다.
    #   R1 #7 은 classify 만 OSError 로 닫았고 load_manifest 는 FileNotFoundError·ValueError 만
    #   잡았다. 그래서 매니페스트 open 의 **PermissionError·Windows 공유위반(WinError 32)** 이
    #   uncaught 로 새고, 파이썬 기본 종료코드 1 = 이 계약의 '불일치'가 되어 **없는 파손을 보고**
    #   했다. 게다가 그 경로에서는 `--json` 이 객체를 한 줄도 내지 않아 소비자가 아무것도 못 읽었다.
    #   이제 어느 갈래로 실패하든 `--json` 은 객체 1줄, 종료는 반드시 2다.
    try:
        m = load_manifest(a.manifest)
        d = classify(m, a.root)
    except (OSError, ValueError, json.JSONDecodeError) as e:
        # 사람용 문장만 갈래를 구분한다(기계 계약은 rc 2 + undecidable 하나로 동일).
        if isinstance(e, FileNotFoundError) and getattr(e, "filename", None) == a.manifest:
            why = "매니페스트 없음: %s" % a.manifest
        elif isinstance(e, OSError):
            why = "판독 실패(%s)" % e
        else:
            why = "매니페스트 형식 오류(%s)" % e
        if a.json:
            print(json.dumps({"ok": False, "undecidable": True, "reason": str(e)},
                             ensure_ascii=False, sort_keys=True))
        print("✗ %s — 판정 불가(측정 불능은 통과가 아니다)" % why, file=sys.stderr)
        return 2
    total = len(d["missing"]) + len(d["added"]) + len(d["changed"])
    if a.json:
        print(json.dumps({"ok": total == 0, "counts": {k: len(v) for k, v in d.items()},
                          **{k: v[:a.max_list] for k, v in d.items()}},
                         ensure_ascii=False, sort_keys=True))
    else:
        if total == 0:
            print("✓ 런타임 봉인 무결 — 항목 %d개 일치 (%s)" % (len(m.get("entries") or {}), a.root))
        else:
            print("✗ 런타임 봉인 파손 — 누락 %d · 추가 %d · 변경 %d"
                  % (len(d["missing"]), len(d["added"]), len(d["changed"])))
            for kind, label in (("missing", "누락"), ("added", "추가"), ("changed", "변경")):
                for rel in d[kind][:a.max_list]:
                    print("  [%s] %s" % (label, rel))
                if len(d[kind]) > a.max_list:
                    print("  [%s] … 외 %d건" % (label, len(d[kind]) - a.max_list))
    return 0 if total == 0 else 1


def main(argv=None):
    ap = argparse.ArgumentParser(description="동봉 런타임 트리 봉인 매니페스트(생성·검증)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    e = sub.add_parser("emit", help="트리를 걸어 매니페스트를 산출한다")
    e.add_argument("--root", required=True, help="runtime 트리 경로")
    e.add_argument("--out", required=True, help="산출 매니페스트 경로")
    e.add_argument("--app-version", default="", help="이 산출물이 실릴 앱 버전")
    e.add_argument("--source", default="", help="생성 지점 라벨(mac-app · win-src 등)")
    e.set_defaults(fn=_cmd_emit)

    v = sub.add_parser("verify", help="매니페스트와 실제 트리를 3분류로 대조한다")
    v.add_argument("--root", required=True)
    v.add_argument("--manifest", required=True)
    v.add_argument("--json", action="store_true")
    v.add_argument("--max-list", type=int, default=20, help="갈래별 출력 상한(기본 20)")
    v.set_defaults(fn=_cmd_verify)

    a = ap.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
