#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""javis_computer_use — computer-use **탑재 감사**의 단일 소유자 (오너 명령 2026-10-17).

This tool audits whether computer-use is enabled in each Claude node's config. The rule that asks for it is
owner-local, not in this pack (the shipped soul.md and directives carry no such section).
그런데 이 설정은 **프로젝트(작업 폴더)별**이다 — `.claude.json` 의 `projects["<cwd>"].enabledMcpServers` 에 `"computer-use"`.

## 이 도구가 막는 결함 (왜 감사가 필요한가)
탑재는 한 번 해두면 끝나는 값이 아니다. 노드가 **새 작업 폴더**에서 뜨면 그 폴더에는
`enabledMcpServers` 가 없고, 도구는 조용히 사라진다. 실패가 에러로 나지 않고 **도구 목록의
부재**로 나타나기 때문에, 아무도 보고하지 않는 채로 오너 명령이 썩는다. 2026-10-17 실측에서
설정 7종 중 5종이 이 방식으로 미탑재였다(base·dept-1 만 탑재).

또한 팩의 어느 도구도 `enabledMcpServers` 를 보지 않는다 — preflight 가 보는 것은 커스텀
서버 등록(`mcpServers`)이고, 내장 서버의 opt-in 화이트리스트는 사각이다.

## 경계 — 쓰기는 사람 게이트를 통과해야 한다
팩 규약은 "등록(.mcp.json)은 기계가, **활성화(노드 .claude.json)는 사람 전용**, 기계는 gap
탐지·WARN 만"이다(javis_preflight C43 주석). 그래서 이 도구의 **기본값은 감사뿐**이고,
`--fix` 는 호출자가 명시적으로 넘겨야 한다 — 그 플래그가 사람 게이트다. 기계가 스스로
`--fix` 를 붙이는 것은 자기인가다.

## 결정론 판정 (자연어 추론 금지 — exit code 가 사실이다)
  exit 0 : 발견된 전 Claude 노드 cwd 가 해당 설정에 탑재돼 있다
  exit 1 : 미탑재 cwd 가 1개 이상 있다 (--fix 로 메울 수 있다)
  exit 2 : 설정·데몬을 판독할 수 없어 **판정 불가** — 0 으로 보고하지 말라

★계측의 유효 범위: 이 도구는 **디스크의 설정**과 **지금 살아 있는 데몬의 좌석 목록**을 본다.
살아 있는 노드 프로세스가 그 설정을 실제로 적재했는지는 보지 않는다 — 적재는 프로세스 기동
시점에 결정되므로, 설정이 OK 라도 그보다 먼저 뜬 프로세스에는 도구가 없다. 그 판정은 해당
좌석만 할 수 있다(자기 도구 목록 조회). 여기서 'OK' 는 "다음 기동에 적재된다"는 뜻이다.

★자격 전제(owner-local rule, not in this pack)는 탑재와 별개다. 이 도구가 재는 것은 ① 플랜 ③ 인증 경로 ④ 플랫폼
이고, ② 대화형 여부는 **좌석 기동 방식**이라 밖에서 재지 못한다 — `claude -p` 좌석은 설정이
OK 여도 computer-use 가 도구 목록에 아예 없다.
"""
import json
import os
import platform
import subprocess
import sys
import tempfile

SRV = "computer-use"
# The names cys reads its socket from, in priority order (src/lib.rs env_compat) - the same tuple as
# javis_role.SOCKET_ENV_KEYS; the test cross-checks all three.
SOCKET_ENV_KEYS = ("CYS_SOCKET", "JAVIS_SOCKET", "AITERM_SOCKET")
HOME = os.path.expanduser("~")
PROVIDER_ENV = ("CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX",
                "ANTHROPIC_BEDROCK_BASE_URL", "ANTHROPIC_VERTEX_BASE_URL",
                "CLAUDE_CODE_USE_FOUNDRY")


def lanes():
    """(레인명, 설정디렉터리, 데몬소켓|None) — base 는 기본 소켓."""
    out = [("base", os.path.join(HOME, ".cys", "claude"), None)]
    root = os.path.join(HOME, ".cys")
    for name in sorted(os.listdir(root)) if os.path.isdir(root) else []:
        if not name.startswith("claude-default-dept-"):
            continue
        dept = name[len("claude-default-dept-"):]
        sock = os.path.join(HOME, ".local", "state", "cys-dept-dept-%s" % dept, "cys.sock")
        out.append(("dept-%s" % dept, os.path.join(root, name), sock))
    return out


def claude_node_cwds(sock):
    """살아 있는 데몬에서 agent=claude 좌석의 cwd 집합. 판독 실패는 None.

    The base lane (sock=None) means the DEFAULT socket, so the caller's socket variables are not passed on.
    From a dept seat CYS_SOCKET names the dept daemon; inherited, the base lane would judge that daemon's
    seats against the base config and --fix would write dept cwds into ~/.cys/claude/.claude.json (D1,
    measured 2026-10-20). cys reads three names (src/lib.rs env_compat), so all three are removed - the
    convention of javis_formation._queue_push."""
    env = dict(os.environ)
    for k in SOCKET_ENV_KEYS:
        env.pop(k, None)
    if sock:
        env["CYS_SOCKET"] = sock
    try:
        p = subprocess.run(["cys", "list"], capture_output=True, text=True,
                           timeout=25, env=env)
    except (OSError, subprocess.SubprocessError):
        return None
    if p.returncode != 0:
        return None
    cwds = {}
    for line in p.stdout.splitlines():
        cols = line.split("\t")
        if len(cols) < 2 or not cols[0].startswith("surface:"):
            continue
        label, cwd = cols[-2], cols[-1].strip()
        # label 형태: "<role>-<agent> · <basename>"
        agent = label.split("·")[0].strip().rsplit("-", 1)[-1]
        if agent == "claude" and cwd:
            cwds.setdefault(cwd, []).append(cols[0])
    return cwds


def read_cfg(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def mounted(cfg, cwd):
    ent = (cfg.get("projects") or {}).get(cwd) or {}
    en = ent.get("enabledMcpServers")
    return isinstance(en, list) and SRV in en


def mount(path, cwds):
    cfg = read_cfg(path)
    if cfg is None:
        return "판독 실패 — 쓰기 거부"
    for cwd in cwds:
        ent = cfg.setdefault("projects", {}).setdefault(cwd, {})
        en = ent.get("enabledMcpServers")
        en = en if isinstance(en, list) else []
        if SRV not in en:
            en.append(SRV)
        ent["enabledMcpServers"] = en
    d = os.path.dirname(path)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".claude.json.cu-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(cfg, f, ensure_ascii=False, indent=2)
        os.chmod(tmp, os.stat(path).st_mode & 0o7777)
        os.replace(tmp, path)
    except OSError as e:
        if os.path.exists(tmp):
            os.unlink(tmp)
        return "원자적 쓰기 실패: %s" % e
    return None


def main():
    do_fix = "--fix" in sys.argv[1:]
    gaps, unreadable, ok_n = [], [], 0

    print("[computer-use audit] 탑재 = projects[<cwd>].enabledMcpServers 에 %r" % SRV)
    if platform.system() != "Darwin":
        print("  ★전제④ 위반: 플랫폼이 %s — computer-use 는 macOS 전용이다" % platform.system())
    prov = [k for k in PROVIDER_ENV if os.environ.get(k)]
    if prov:
        print("  ★전제③ 위반 가능: 타사 프로바이더 env 설정됨 %s" % prov)

    for lane, cfg_dir, sock in lanes():
        path = os.path.join(cfg_dir, ".claude.json")
        cfg = read_cfg(path)
        if cfg is None:
            print("  판정불가 %-9s .claude.json 판독 실패" % lane)
            unreadable.append(lane)
            continue
        plan = ((cfg.get("oauthAccount") or {}).get("organizationType")) or "?"
        if plan not in ("claude_max", "claude_pro"):
            print("  ★전제① %-9s orgType=%r — Pro/Max 가 아니면 탑재해도 도구가 없다"
                  % (lane, plan))
        cwds = claude_node_cwds(sock)
        if cwds is None:
            print("  판정불가 %-9s 데몬 좌석 목록 판독 실패(소켓 %s)" % (lane, sock or "기본"))
            unreadable.append(lane)
            continue
        if not cwds:
            print("  좌석없음 %-9s 살아 있는 Claude 노드 0 — 탑재 판정 대상 없음" % lane)
            continue
        miss = [c for c in sorted(cwds) if not mounted(cfg, c)]
        for c in sorted(cwds):
            tag = "미탑재" if c in miss else "OK   "
            print("  %s   %-9s %-45s seats=%s" % (tag, lane, c, ",".join(cwds[c])))
        ok_n += len(cwds) - len(miss)
        if miss:
            gaps.append((lane, path, miss))

    if gaps and do_fix:
        print("\n[--fix] 사람 게이트 통과(명시 플래그) — 미탑재 cwd 를 메운다")
        still = []
        for lane, path, miss in gaps:
            err = mount(path, miss)
            if err:
                print("  실패 %-9s %s" % (lane, err))
                still.append((lane, path, miss))
            else:
                cfg = read_cfg(path)
                bad = [c for c in miss if not (cfg and mounted(cfg, c))]
                if bad:
                    print("  검증실패 %-9s 재독 후에도 미탑재: %s" % (lane, bad))
                    still.append((lane, path, bad))
                else:
                    print("  탑재 %-9s %s" % (lane, ", ".join(miss)))
        gaps = still

    if unreadable:
        print("\n[판정] exit 2 — 판독 불가 레인 %s 가 있어 0 을 주장할 수 없다" % unreadable)
        return 2
    if gaps:
        n = sum(len(m) for _, _, m in gaps)
        print("\n[판정] exit 1 — 미탑재 cwd %d개 (레인 %s)"
              % (n, [l for l, _, _ in gaps]))
        print("  메우려면: python3 %s --fix" % os.path.basename(__file__))
        return 1
    print("\n[판정] exit 0 — 발견된 Claude 노드 cwd %d개 전부 탑재. "
          "단 'OK' 는 \"다음 기동에 적재된다\"는 뜻이고, 지금 살아 있는 프로세스의 적재 여부는 "
          "해당 좌석만 판정할 수 있다." % ok_n)
    return 0


if __name__ == "__main__":
    sys.exit(main())
