#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""test_teamtoken_hook.py — UserPromptSubmit 경로의 **대화 승인 토큰 발급 배선** 회귀 핀 (0.14.42 · 설계 §13 P4).

설계 정본: 팀만들기-확인창-무반응-수정설계안-최종-20260923.md §6-4(오너 실키 입력 판별)·§6-6(발급 조건)·
  §7-3(발급 0 관측)·§10(안내 문구)·§11 R11(훅 배선)·§12-1(fail-closed)·§13 P4(수용 기준)·§14-6(거짓 음성).
발급 판정 자체의 회귀 핀은 test_teamtoken.py(P3)다. 이 파일은 **훅이 그것을 부르는가**만 잰다 —
판정을 다시 재현하지 않는다(사본 금지).

§13 P4 수용 기준은 "수동 1회: 좌석에 '그래 만들어' 타이핑 → 원장 issued 1줄 · 기계 push 로 같은 문장 →
issued 0줄(원장에 사유)" 이다. 이 검체는 그 수동 절차를 **샌드박스 자동 판정**으로 대체한다:
  · 오너 실키 입력 = 배달 원장에 해시가 **없는** 문장이 훅 stdin JSON 으로 도착하는 것.
  · 기계 배달     = 데몬이 pane 주입 **직전에** 원장에 정규화 본문 sha256 을 남긴 문장(delivery.rs 계약)이
                    같은 형태로 도착하는 것. 훅 입장에서 두 경로의 차이는 원장 기록 하나뿐이다 — 그래서
                    원장 1줄을 넣고 빼는 것이 두 경로를 가르는 유일한 조작이다(음성 대조 = P4-1 ↔ P4-2).

실험실(라이브 무접촉): 임시 디렉터리에 팩 사본(런처 role-bootstrap.sh · 프리루드 _lib.sh · 발급기
teamtoken-issue.sh · bin/*.py)을 두고 HOME·CYS_SOCKET·CYS_PACK_DIR·CYS_CONFIG_DIR·CYS_PACK_CAPTURES_DIR·
CYS_STATE_DIR 전부 임시 경로 + CYS_NO_PERSONAL_HOOK_MERGE=1 + CYS_NO_AUTOSTART=1. `cys` 는 PATH 선두의
**스텁**이다(설치본 /usr/local/bin/cys 를 절대 실행하지 않는다 — 라이브 데몬에 붙는다). 본체
(role-bootstrap-legacy.sh)는 기본이 스텁이고 R 스위트만 실 본체를 태운다.
인터프리터 호출 수는 CYS_PY 를 계수 래퍼로 두어 **실측**한다(비용 0 주장을 말이 아니라 숫자로).

스위트
  P  수용 기준(§13 P4)  — 오너 입력 issued 1 · 기계 배달 issued 0 + issue_refused(machine_origin) 기록 ·
                          신 파이프라인이 처리완료(rc 6)로 본체를 건너뛰어도 · 구 CLI 로 본체가 돌아도 같다 ·
                          기계 배달은 질문을 소비하지 않는다(뒤이은 오너 승인 발급) · §14-6 거짓 음성 핀.
  C  비용 게이트        — 토큰 원장 부재 = 인터프리터 0회 · 원장이 오래됨 = 0회 · 최근이면 발급기 1회.
  V  안내(§10)          — 거절·만료·다른 좌석·기계 비승인 발화의 고지 유무와 문구.
  O  stdout 계약        — 훅 stdout 은 JSON 1줄이거나 무출력: 본체 고지·런처 자체 고지와 겹치면 합치거나 뺀다.
  F  실패 정책          — 발급기 손상·행·부재에서도 exit 0 · 본체 진행 · 발급 0 · 유계 시간.
  R  실 본체 통합       — 실 role-bootstrap-legacy.sh 를 태워도 stdout 1줄 · 발급 1.
  S  구조               — 발급기의 프리루드 2단 source·레인 redirect·네이티브 경로 · 런처의 프리루드 심볼 0 ·
                          발급이 신 파이프라인 위임(⑥)보다 앞.

    CYS_PACK_DIR="$(mktemp -d)" python3 cysjavis-pack/bin/tests/test_teamtoken_hook.py
종료: 0 = 전 스위트 통과(말미 TEAMTOKEN-HOOK-OK) · 1 = 실패 1건 이상.
"""
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True   # 검체가 저장소 bin/ 에 __pycache__ 를 남기지 않게(형제 import 전)

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
HOOKS = os.path.join(PACK, "hooks")
LAUNCHER = os.path.join(HOOKS, "role-bootstrap.sh")
ISSUER = os.path.join(HOOKS, "teamtoken-issue.sh")
PRELUDE = os.path.join(HOOKS, "_lib.sh")
LEGACY = os.path.join(HOOKS, "role-bootstrap-legacy.sh")
PY = sys.executable

SURFACE = "22"
OTHER_SURFACE = "24"
PROPOSAL = "tp-1790300000-4242"          # 샌드박스 전용 가짜 제안(라이브 대기 제안과 무관)
APPROVE = "그래 만들어"
APPROVE_2 = "응 만들어"
REDIRECT = 'command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"'

RESULTS = []


def check(suite, cid, cond, detail=""):
    RESULTS.append((suite, cid, bool(cond)))
    print("[%s] %s %s%s" % ("PASS" if cond else "FAIL", suite, cid, (" — " + str(detail)) if detail else ""))
    return bool(cond)


def w(path, body, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    os.chmod(path, mode)


def rd(path):
    try:
        with io.open(path, encoding="utf-8", newline="") as f:
            return f.read()
    except OSError:
        return ""


ROOT = tempfile.mkdtemp(prefix="teamtoken-hook-test-")
# 팩 bin 사본 1벌(검체용) — 실험실은 이것을 가리킨다(저장소 bin 에 아무것도 쓰지 않는다).
SHARED_BIN = os.path.join(ROOT, "shared-bin")
shutil.copytree(BIN, SHARED_BIN, ignore=shutil.ignore_patterns("tests", "__pycache__", "*.pyc"))

STUB_CYS = r"""#!/bin/sh
# 스텁 cys — 호출을 기록하고 신 파이프라인(`cys hook user-prompt-submit --input`)의 rc 를 흉내 낸다.
printf '%s\n' "cys $*" >> "$CYSLOG"
if [ "$1" = hook ] && [ "$3" = --help ]; then
  if [ "${STUB_NEWCLI:-1}" = 1 ]; then
    printf 'Usage: cys hook user-prompt-submit [OPTIONS]\n\nOptions:\n      --input <FILE>  hook payload\n'
  else
    printf 'Usage: cys hook user-prompt-submit [OPTIONS]\n\nOptions:\n  -h, --help  Print help\n'
  fi
  exit 0
fi
if [ "$1" = hook ] && [ "$3" = --input ]; then
  [ -n "${STUB_INPUT_SLEEP:-}" ] && sleep "$STUB_INPUT_SLEEP"
  exit "${STUB_RC:-5}"
fi
exit 0
"""

STUB_BODY = r"""#!/bin/sh
# 스텁 본체 — 실행 사실만 남기고(필요하면 자기 고지 1줄을 낸다) 입력 파일을 회수한다.
echo "BODY-RAN" >> "$MARK"
[ -n "${1:-}" ] && rm -f "$1"
if [ -n "${STUB_BODY_NOTE:-}" ]; then
  printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"%s"}}\n' "$STUB_BODY_NOTE"
fi
exit 0
"""


class Lab(object):
    """런처·프리루드·발급기·(스텁|실) 본체·bin 사본을 갖춘 격리 실험실 1개."""

    def __init__(self, name, body="stub", issuer=True, bin_mut=None):
        self.d = os.path.join(ROOT, name)
        self.pack = os.path.join(self.d, "pack")
        self.hooks = os.path.join(self.pack, "hooks")
        self.stub = os.path.join(self.d, "stub")
        self.state = os.path.join(self.d, "state")
        self.run_dir = os.path.join(self.d, "run")
        for p in (self.hooks, self.stub, self.state, self.run_dir, os.path.join(self.d, "home"),
                  os.path.join(self.d, "config"), os.path.join(self.d, "captures")):
            os.makedirs(p, exist_ok=True)
        shutil.copy(LAUNCHER, os.path.join(self.hooks, "role-bootstrap.sh"))
        shutil.copy(PRELUDE, os.path.join(self.hooks, "_lib.sh"))
        if issuer and os.path.isfile(ISSUER):
            shutil.copy(ISSUER, os.path.join(self.hooks, "teamtoken-issue.sh"))
        if body == "stub":
            w(os.path.join(self.hooks, "role-bootstrap-legacy.sh"), STUB_BODY, 0o755)
        elif body == "real":
            shutil.copy(LEGACY, os.path.join(self.hooks, "role-bootstrap-legacy.sh"))
        binp = os.path.join(self.pack, "bin")
        if bin_mut is None and os.name == "posix":
            os.symlink(SHARED_BIN, binp)
        else:
            shutil.copytree(SHARED_BIN, binp)
            if bin_mut is not None:
                bin_mut(binp)
        self.bin = binp
        w(os.path.join(self.stub, "cys"), STUB_CYS, 0o755)
        # 인터프리터 계수 래퍼 — 발급 경로가 python 을 몇 번 띄웠는지 실측한다.
        self.pylog = os.path.join(self.d, "py.log")
        w(os.path.join(self.stub, "pylog"),
          '#!/bin/sh\nprintf "PYCALL %%s\\n" "${1:-}" >> "%s"\nexec "%s" "$@"\n' % (self.pylog, PY), 0o755)
        self.mark = os.path.join(self.d, "mark")
        self.cyslog = os.path.join(self.d, "cys.log")
        self.sock = os.path.join(self.run_dir, "cys.sock")

    # ── env ──
    def env(self, surface=SURFACE, **extra):
        e = {}
        for k in ("PATH", "LANG", "LC_ALL", "TMPDIR", "SYSTEMROOT", "OSTYPE"):
            if os.environ.get(k):
                e[k] = os.environ[k]
        e.update({
            "PATH": self.stub + os.pathsep + os.environ.get("PATH", ""),
            "HOME": os.path.join(self.d, "home"),
            "USERPROFILE": os.path.join(self.d, "home"),
            "LOCALAPPDATA": os.path.join(self.d, "localappdata"),
            "CYS_SOCKET": self.sock,
            "CYS_PACK_DIR": self.pack,
            "CYS_CONFIG_DIR": os.path.join(self.d, "config"),
            "CYS_PACK_CAPTURES_DIR": os.path.join(self.d, "captures"),
            "CYS_STATE_DIR": self.state,
            "CYS_NO_PERSONAL_HOOK_MERGE": "1",
            "CYS_NO_AUTOSTART": "1",
            "CYS_SURFACE_ID": surface,
            "CYS_PY": os.path.join(self.stub, "pylog"),
            "MARK": self.mark,
            "CYSLOG": self.cyslog,
        })
        for k, v in extra.items():
            if v is None:
                e.pop(k, None)
            else:
                e[k] = str(v)
        return e

    # ── 모듈 호출(검체 쪽 준비 · 계수 래퍼 밖) — 항상 **온전한** 공유 사본으로 부른다(F 스위트는
    #    실험실 bin 을 일부러 망가뜨린다 · 경로 규약은 env 만 보므로 어느 사본이든 같은 경로를 준다) ──
    def py(self, code, *args, surface=SURFACE):
        r = subprocess.run([PY, "-B", "-c", "import sys; sys.path.insert(0, %r)\n" % SHARED_BIN + code]
                           + [str(a) for a in args], capture_output=True, text=True, encoding="utf-8",
                           env=self.env(surface=surface), timeout=120)
        if r.returncode != 0:
            raise RuntimeError("검체 준비 실패: %s" % r.stderr[-400:])
        return json.loads(r.stdout.strip().splitlines()[-1])

    def paths(self):
        return self.py("import json, javis_teamtoken as t, javis_mission as m\n"
                       "print(json.dumps({'ledger': t.ledger_path(), 'feed': t.feed_jsonl_path(),"
                       " 'delivery': m.delivery_ledger_path()}))")

    def seed(self, now=None, extra_delivery=()):
        """배달 원장(기동 표식 1줄 = 상태 ok) + feed(이 좌석이 올린 대기 제안 1건)."""
        now = time.time() if now is None else now
        p = self.paths()
        body = json.dumps({"v": 1, "id": PROPOSAL, "display": "일반저술출판부",
                           "purpose": "일반 교양서를 기획·저술·출판한다."}, ensure_ascii=False)
        item = {"request_id": PROPOSAL, "kind": "team-create-request", "title": "팀 만들기 제안",
                "body": body, "surface_id": int(SURFACE), "publisher_surface": int(SURFACE),
                "status": "pending", "decision": None, "created_at": now - 30}
        w(p["feed"], json.dumps(item, ensure_ascii=False) + "\n")
        recs = [{"v": 1, "sha256": "0" * 64, "ts_epoch": now - 600, "surface": SURFACE, "chars": 0,
                 "preview": "", "origin": "boot"}]
        w(p["delivery"], "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in recs))
        for text in extra_delivery:
            self.deliver(text)
        self.p = p
        return p

    def deliver(self, text, ts=None):
        """데몬의 기계 배달을 흉내 낸다 — pane 주입 **직전** 원장에 정규화 본문 sha256 1줄(delivery.rs 계약)."""
        rec = self.py("import json, time, javis_mission as m\n"
                      "t = sys.argv[1]; n = m._normalize_delivery(t)\n"
                      "print(json.dumps({'v': m.SCHEMA_VERSION, 'sha256': m.delivery_digest(t),"
                      " 'ts_epoch': float(sys.argv[2]), 'surface': sys.argv[3], 'chars': len(n),"
                      " 'preview': n[:m.PREVIEW_CHARS], 'origin': 'daemon'}, ensure_ascii=False))",
                      text, time.time() if ts is None else ts, SURFACE)
        with io.open(self.p["delivery"], "a", encoding="utf-8", newline="\n") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

    def ask(self, now=None):
        """master 가 질문을 연다 — now 를 주면 모듈 API 로(과거 시각의 질문 · 만료 재현용)."""
        if now is None:
            r = subprocess.run([PY, "-B", os.path.join(SHARED_BIN, "javis_teamtoken.py"), "ask",
                                "--proposal", PROPOSAL], capture_output=True, text=True,
                               encoding="utf-8", env=self.env(), timeout=120)
            return json.loads(r.stdout.strip().splitlines()[-1])
        return self.py("import json, javis_teamtoken as t\n"
                       "print(json.dumps(t.open_ask(sys.argv[1], now=float(sys.argv[2]))))",
                       PROPOSAL, now)

    def status(self, surface=SURFACE):
        r = subprocess.run([PY, "-B", os.path.join(SHARED_BIN, "javis_teamtoken.py"), "status"],
                           capture_output=True, text=True, encoding="utf-8",
                           env=self.env(surface=surface), timeout=120)
        return json.loads(r.stdout.strip().splitlines()[-1])

    def records(self):
        p = self.paths()["ledger"]
        out = []
        for ln in rd(p).splitlines():
            ln = ln.strip()
            if ln:
                out.append(json.loads(ln))
        return out

    def events(self, name):
        return [r for r in self.records() if r.get("event") == name]

    def ledger_bytes(self):
        p = self.paths()["ledger"]
        try:
            with open(p, "rb") as f:
                return f.read()
        except OSError:
            return b""

    # ── 훅 1회 실행(= 오너 또는 기계가 이 좌석에 프롬프트를 제출한 순간) ──
    def hook(self, prompt, surface=SURFACE, newcli=True, stub_rc=0, shell="sh", timeout=90, cwd=None,
             **extra):
        for f in (self.mark, self.pylog, self.cyslog):
            if os.path.exists(f):
                os.remove(f)
        payload = json.dumps({"session_id": "sess-p4-test", "transcript_path": "/tmp/t.jsonl",
                              "cwd": cwd or self.d, "permission_mode": "default",
                              "hook_event_name": "UserPromptSubmit", "prompt": prompt}, ensure_ascii=False)
        env = self.env(surface=surface, STUB_NEWCLI="1" if newcli else "0", STUB_RC=stub_rc, **extra)
        t0 = time.time()
        r = subprocess.run([shell, os.path.join(self.hooks, "role-bootstrap.sh")], input=payload,
                           capture_output=True, text=True, encoding="utf-8", env=env, timeout=timeout)
        r.elapsed = time.time() - t0
        r.lines = [ln for ln in r.stdout.splitlines() if ln.strip()]
        r.body_runs = rd(self.mark).count("BODY-RAN")
        r.pycalls = [ln for ln in rd(self.pylog).splitlines() if ln.startswith("PYCALL")]
        r.tt_calls = [ln for ln in r.pycalls if "javis_teamtoken.py" in ln]
        return r

    def residue(self):
        return sorted(n for n in os.listdir(self.state)
                      if n.startswith("teamtoken-note-") or n.startswith("hook-input-"))


def ctx_of(line):
    try:
        return json.loads(line)["hookSpecificOutput"]["additionalContext"]
    except Exception:  # noqa: BLE001 — 파싱 불가 자체가 판정 대상이다
        return None


def one_json_line(r):
    """훅 stdout 계약: 비어 있거나 hookSpecificOutput JSON 정확히 1줄."""
    if not r.lines:
        return True
    return len(r.lines) == 1 and ctx_of(r.lines[0]) is not None \
        and json.loads(r.lines[0])["hookSpecificOutput"]["hookEventName"] == "UserPromptSubmit"


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# 설계 §10 원문(글자 그대로 — 발급기 모듈 상수를 import 하지 않고 설계서에서 옮긴다: 사본이 아니라 기대값)
MSG_MACHINE = "방금 문장은 시스템이 보낸 메시지로 확인됩니다 — 주인님이 직접 한 번만 쳐 주세요."
MSG_REJECTED = "네, 아직 만들지 않았습니다. 만들 때 말씀해 주세요."
MSG_ASK_EXPIRED = "확인 시간이 지나 다시 여쭙습니다 — 이 내용으로 만들까요?"


# ══════════════════════════════════════════════════════════════════════════════
# P — §13 P4 수용 기준(자동 판정판)
# ══════════════════════════════════════════════════════════════════════════════
def suite_acceptance():
    # P4-1 오너 실키 입력: 배달 원장에 해시가 없는 '그래 만들어'. 신 파이프라인은 사람 발화를 proceed(0)로
    #      넘겨 본체가 돈다(실 CLI 의 오너 비선언 프롬프트 경로 그대로).
    lab = Lab("p1")
    lab.seed()
    a = lab.ask()
    check("P", "P4-0 전제: master 가 질문을 열었다(ask_opened)", a.get("code") == "ask_opened", a)
    r = lab.hook(APPROVE, stub_rc=0)
    iss = lab.events("token_issued")
    tok = iss[0]["token"] if len(iss) == 1 else None
    check("P", "P4-1a 오너 실키 입력 → 원장 token_issued 정확히 1줄", len(iss) == 1,
          "issued=%d stderr=%r" % (len(iss), r.stderr[-300:]))
    check("P", "P4-1b 훅 exit 0 · stdout JSON 1줄", r.returncode == 0 and len(r.lines) == 1 and one_json_line(r),
          "rc=%s lines=%r" % (r.returncode, r.lines))
    ctx = ctx_of(r.lines[0]) if r.lines else ""
    check("P", "P4-1c 고지에 토큰 문자열과 다음 명령(--team-token <토큰>)이 실린다(master 가 쓸 수 있다)",
          bool(tok) and ("--team-token %s" % tok) in (ctx or "") and PROPOSAL in (ctx or ""), (ctx or "")[:240])
    check("P", "P4-1d 발급 레코드 결박 = 이 좌석·이 제안 · 훅 세션 기록",
          len(iss) == 1 and iss[0]["surface"] == SURFACE and iss[0]["proposal_id"] == PROPOSAL
          and iss[0].get("hook_session") == "sess-p4-test", iss[:1])
    check("P", "P4-1e 발급 판정 뒤에도 본체가 정확히 1회 돈다(부트 경로 불가침)", r.body_runs == 1, r.body_runs)
    check("P", "P4-1f 발급 판정은 발급기 1회(javis_teamtoken issue)", len(r.tt_calls) == 1, r.pycalls)
    check("P", "P4-1g 상태 디렉터리 잔재 0(고지 임시 파일·입력 파일)", not lab.residue(), lab.residue())

    # P4-2 기계 배달: 같은 문장이 원장에 해시로 기록된 뒤 도착. 실 CLI 는 기계 유래를 처리완료(rc 6)로
    #      닫아 **본체를 건너뛴다** — 발급 판정이 본체 안에 있었다면 여기서 사유 기록이 사라진다.
    lab2 = Lab("p2")
    lab2.seed()
    lab2.ask()
    lab2.deliver(APPROVE)
    before = lab2.status()
    r2 = lab2.hook(APPROVE, stub_rc=6)
    ref = lab2.events("issue_refused")
    check("P", "P4-2a 기계 배달 → token_issued 0줄", len(lab2.events("token_issued")) == 0,
          lab2.events("token_issued"))
    check("P", "P4-2b 거부 사유 기록 = issue_refused 1줄 · code=machine_origin",
          len(ref) == 1 and ref[0].get("code") == "machine_origin", ref)
    check("P", "P4-2c 사유 레코드는 발화 원문이 아니라 해시만 싣는다(prompt_sha256 = sha256(문장))",
          len(ref) == 1 and ref[0].get("prompt_sha256") == sha(APPROVE)
          and APPROVE not in json.dumps(ref[0], ensure_ascii=False), ref[:1])
    check("P", "P4-2d 신 파이프라인 처리완료(rc 6)로 본체가 건너뛰어져도 판정·기록이 일어났다",
          r2.body_runs == 0 and len(ref) == 1, "body=%d" % r2.body_runs)
    after = lab2.status()
    check("P", "P4-2e 기계 발화는 질문을 소비하지 않는다(awaiting_answer 유지)",
          before.get("code") == "awaiting_answer" and after.get("code") == "awaiting_answer"
          and not (after.get("ask") or {}).get("closed"), (before.get("code"), after.get("code")))
    ctx2 = ctx_of(r2.lines[0]) if r2.lines else ""
    check("P", "P4-2f 승인처럼 들리는 기계 배달은 master 에게 §10 문구로 고지된다(1줄)",
          r2.returncode == 0 and len(r2.lines) == 1 and "machine_origin" in (ctx2 or "")
          and MSG_MACHINE in (ctx2 or "") and "--team-token" not in (ctx2 or ""), (ctx2 or "")[:240])

    # P4-3 같은 기계 배달을 구 CLI(신 파이프라인 없음 → 본체가 돈다)에서도: 판정은 파이프라인과 무관하다.
    lab3 = Lab("p3")
    lab3.seed()
    lab3.ask()
    lab3.deliver(APPROVE)
    r3 = lab3.hook(APPROVE, newcli=False)
    ref3 = lab3.events("issue_refused")
    check("P", "P4-3 구 CLI(본체 실행) 경로에서도 issued 0 · machine_origin 기록 · stdout 1줄",
          len(lab3.events("token_issued")) == 0 and len(ref3) == 1
          and ref3[0].get("code") == "machine_origin" and r3.body_runs == 1 and len(r3.lines) == 1
          and one_json_line(r3), "issued=%d refused=%s body=%d lines=%r"
          % (len(lab3.events("token_issued")), [x.get("code") for x in ref3], r3.body_runs, r3.lines))

    # P4-4 질문은 기계 배달을 견딘다 → 그 뒤 오너가 **다른** 짧은 승인을 치면 발급된다.
    #      ★§14-6 거짓 음성: 오너가 기계 배달과 정규화 후 **같은** 문장을 치면 기계로 접힌다(의도된 비대칭).
    r4a = lab2.hook(APPROVE, stub_rc=0)
    ref4 = lab2.events("issue_refused")
    check("P", "P4-4a §14-6 오너가 기계 배달과 같은 문장을 쳐도 기계로 접힌다(발급 0 · 기록 2줄째)",
          len(lab2.events("token_issued")) == 0 and len(ref4) == 2
          and ref4[-1].get("code") == "machine_origin", [x.get("code") for x in ref4])
    r4b = lab2.hook(APPROVE_2, stub_rc=0)
    iss4 = lab2.events("token_issued")
    check("P", "P4-4b 다른 짧은 승인('응 만들어') → 발급 1 · 고지에 그 토큰",
          len(iss4) == 1 and len(r4b.lines) == 1
          and ("--team-token %s" % iss4[0]["token"]) in (ctx_of(r4b.lines[0]) or ""),
          "issued=%d lines=%r" % (len(iss4), r4b.lines))
    _ = r4a


# ══════════════════════════════════════════════════════════════════════════════
# C — 비용 게이트(열린 질문이 없으면 거의 공짜)
# ══════════════════════════════════════════════════════════════════════════════
def suite_cost():
    lab = Lab("c1")
    lab.seed()                                   # 배달 원장·feed 는 있어도 토큰 원장은 없다
    r = lab.hook(APPROVE, stub_rc=0)
    check("C", "C-1 토큰 원장 부재 → 인터프리터 0회 · stdout 무출력 · 본체 1회",
          r.returncode == 0 and not r.pycalls and not r.lines and r.body_runs == 1,
          "py=%r lines=%r body=%d" % (r.pycalls, r.lines, r.body_runs))
    check("C", "C-1b 원장 부재 경로는 원장을 만들지 않는다(무기록)", not lab.ledger_bytes(), "")

    lab2 = Lab("c2")
    lab2.seed()
    old = time.time() - 3600
    lab2.ask(now=old)                            # 한 시간 전에 열린(이미 만료된) 질문
    lp = lab2.paths()["ledger"]
    os.utime(lp, (old, old))
    snap = lab2.ledger_bytes()
    r2 = lab2.hook(APPROVE, stub_rc=0)
    check("C", "C-2 원장이 오래됨(최근 움직임 없음) → 인터프리터 0회 · 무출력 · 원장 무변경",
          r2.returncode == 0 and not r2.pycalls and not r2.lines and lab2.ledger_bytes() == snap,
          "py=%r lines=%r" % (r2.pycalls, r2.lines))

    lab3 = Lab("c3")
    lab3.seed()
    lab3.ask()
    snap3 = lab3.ledger_bytes()
    r3 = lab3.hook(APPROVE, surface=OTHER_SURFACE, stub_rc=0)
    check("C", "C-3 최근 원장 + 이 좌석엔 질문 없음 → 발급기 1회 · 무출력 · 원장 무변경",
          r3.returncode == 0 and len(r3.tt_calls) == 1 and not r3.lines and lab3.ledger_bytes() == snap3,
          "tt=%r lines=%r" % (r3.tt_calls, r3.lines))
    r3b = lab3.hook("오늘 할 일 정리해줘", stub_rc=0)
    check("C", "C-4 질문이 열린 좌석의 평문 답도 판정은 1회(발급기 1회)", len(r3b.tt_calls) == 1, r3b.pycalls)

    # C-5 게이트의 mtime 판독 두 갈래 — macOS(BSD `stat -f %m`)는 위에서 쟀다. GNU(`stat -c %Y`)를 흉내
    #     내는 스텁으로 리눅스 갈래도 같은 결론(오래된 원장 → 0회)인지 잰다.
    gnu = ('#!/bin/sh\nif [ "$1" = -c ] && [ "$2" = %%Y ]; then\n'
           '  exec "%s" -c "import os,sys; print(int(os.stat(sys.argv[1]).st_mtime))" "$3"\nfi\n'
           'echo "stat: cannot read file system information" >&2\nexit 1\n' % PY)
    for tag, stub_body, want in (("C-5 GNU stat 갈래 — 오래된 원장 → 발급기 0회", gnu, 0),
                                 ("C-6 mtime 판독 불가(쓰레기 출력) → 실패 방향은 '발급기를 부른다'(1회)"
                                  " · 그래도 발급 0", "#!/bin/sh\necho garbage\nexit 0\n", 1)):
        labx = Lab("c5-%d" % want)
        w(os.path.join(labx.stub, "stat"), stub_body, 0o755)
        labx.seed()
        labx.ask(now=old)
        os.utime(labx.paths()["ledger"], (old, old))
        rx = labx.hook(APPROVE, stub_rc=0)
        check("C", tag, rx.returncode == 0 and len(rx.tt_calls) == want
              and not labx.events("token_issued"), "tt=%r" % rx.tt_calls)


# ══════════════════════════════════════════════════════════════════════════════
# V — §10 안내(고지 유무·문구)
# ══════════════════════════════════════════════════════════════════════════════
def suite_voice():
    lab = Lab("v1")
    lab.seed()
    lab.ask()
    r = lab.hook("아직 만들지 마", stub_rc=0)
    ctx = ctx_of(r.lines[0]) if r.lines else ""
    closed = [x for x in lab.events("ask_closed") if x.get("why") == "answered_rejected"]
    check("V", "V-1 거절 답 → 발급 0 · 질문 닫힘 · §10 '아직 만들지 않았습니다' 고지 1줄",
          not lab.events("token_issued") and len(closed) == 1 and len(r.lines) == 1
          and MSG_REJECTED in (ctx or "") and "utterance_rejected" in (ctx or ""), (ctx or "")[:200])

    lab2 = Lab("v2")
    lab2.seed()
    lab2.ask()
    wake = "[wakeup] 다음 액션 확인"
    lab2.deliver(wake)
    r2 = lab2.hook(wake, stub_rc=6)
    ref = lab2.events("issue_refused")
    check("V", "V-2 승인처럼 들리지 않는 기계 배달 → 기록은 남고(machine_origin) 고지는 없다(잡음 0)",
          len(ref) == 1 and ref[0].get("code") == "machine_origin" and not r2.lines
          and lab2.status().get("code") == "awaiting_answer", "refused=%r lines=%r" % (ref, r2.lines))

    lab3 = Lab("v3")
    lab3.seed()
    lab3.ask(now=time.time() - 400)              # TTL 300s 를 넘긴 질문(원장은 방금 쓰였다)
    r3 = lab3.hook(APPROVE, stub_rc=0)
    ctx3 = ctx_of(r3.lines[0]) if r3.lines else ""
    check("V", "V-3 질문 TTL 만료 → 발급 0 · '확인 시간이 지나' 고지(만료≠미개설)",
          not lab3.events("token_issued") and "ask_expired" in (ctx3 or "")
          and MSG_ASK_EXPIRED in (ctx3 or ""), (ctx3 or "")[:200])

    lab4 = Lab("v4")
    lab4.seed()
    lab4.ask()
    r4 = lab4.hook(APPROVE, surface=OTHER_SURFACE, stub_rc=0)
    check("V", "V-4 다른 좌석의 '그래 만들어' → 발급 0 · 무출력(그 좌석엔 제안도 질문도 없다)",
          not lab4.events("token_issued") and not r4.lines, r4.lines)


# ══════════════════════════════════════════════════════════════════════════════
# O — stdout 계약(JSON 1줄 또는 무출력)
# ══════════════════════════════════════════════════════════════════════════════
def suite_stdout():
    # O-1 선언 발화 + 본체가 자기 고지를 내는 경로: 두 줄이 되면 안 된다 → 발급 고지를 싣지 않는다.
    lab = Lab("o1")
    lab.seed()
    lab.ask()
    r = lab.hook("너는 마스터다", newcli=False, STUB_BODY_NOTE="BODY-NOTE-MARK")
    ref = lab.events("issue_refused")
    check("O", "O-1 선언 발화(본체 고지) → stdout 정확히 1줄 = 본체 고지 · 거부는 원장에 기록",
          len(r.lines) == 1 and "BODY-NOTE-MARK" in (ctx_of(r.lines[0]) or "")
          and len(ref) == 1 and not lab.events("token_issued"),
          "lines=%r refused=%r" % (r.lines, [x.get("code") for x in ref]))
    check("O", "O-1b 싣지 않은 발급 고지는 stderr 로 남는다(조용히 사라지지 않는다)",
          "teamtoken" in r.stderr or "대화 승인" in r.stderr, r.stderr[-300:])

    # O-2 신 파이프라인 데드라인(런처 자체 고지) + 발급 고지 → 한 줄로 합친다.
    lab2 = Lab("o2")
    lab2.seed()
    lab2.ask()
    r2 = lab2.hook(APPROVE, stub_rc=0, STUB_INPUT_SLEEP="3", CYS_HOOK_INPUT_DEADLINE_S="1")
    ctx2 = ctx_of(r2.lines[0]) if r2.lines else ""
    iss2 = lab2.events("token_issued")
    check("O", "O-2 런처 데드라인 고지 + 발급 고지 → JSON 1줄에 둘 다",
          len(r2.lines) == 1 and "지연" in (ctx2 or "") and len(iss2) == 1
          and ("--team-token %s" % iss2[0]["token"]) in (ctx2 or ""), r2.lines)

    # O-3 본체 부재(런처 자체 고지) + 발급 고지 → 한 줄.
    lab3 = Lab("o3", body=None)
    lab3.seed()
    lab3.ask()
    r3 = lab3.hook(APPROVE, newcli=False)
    ctx3 = ctx_of(r3.lines[0]) if r3.lines else ""
    iss3 = lab3.events("token_issued")
    check("O", "O-3 본체 부재 고지 + 발급 고지 → JSON 1줄에 둘 다",
          r3.returncode == 0 and len(r3.lines) == 1 and "부재" in (ctx3 or "") and len(iss3) == 1
          and ("--team-token %s" % iss3[0]["token"]) in (ctx3 or ""), r3.lines)

    # O-5 ★발급 고지는 빼지 않는다: 훅 입력 전문(cwd·transcript_path)에 'master' 글자가 있어도, 승인 발화는
    #     선언일 수 없어 본체의 선언 고지와 겹치지 않는다. 전문 술어로 토큰 고지를 버리면 그런 폴더의
    #     오너는 대화 승인이 영영 안 된다(음성 대조: 같은 폴더의 거부 고지는 O-1 처럼 빠진다).
    lab5 = Lab("o5")
    lab5.seed()
    lab5.ask()
    r5 = lab5.hook(APPROVE, newcli=False, cwd="/Users/x/master-plan")
    iss5 = lab5.events("token_issued")
    check("O", "O-5 cwd 에 'master' 가 있어도 발급 고지는 나간다(본체 정상 = 1줄)",
          len(iss5) == 1 and len(r5.lines) == 1
          and ("--team-token %s" % iss5[0]["token"]) in (ctx_of(r5.lines[0]) or ""), r5.lines)
    lab6 = Lab("o6")
    lab6.seed()
    lab6.ask()
    r6 = lab6.hook("아직 만들지 마", newcli=False, cwd="/Users/x/master-plan")
    check("O", "O-6 음성 대조: 같은 폴더의 **거부** 고지는 빠진다(원장엔 기록 · stdout 무출력 · stderr 고지)",
          not r6.lines and len(lab6.events("issue_refused")) == 1 and "고지 생략" in r6.stderr,
          "lines=%r" % r6.lines)

    # O-4 인용부호·역슬래시가 섞인 사유(detail)도 JSON 을 깨지 않는다 — 전 실험의 줄을 json.loads 로 이미 쟀다.
    lab4 = Lab("o4")
    lab4.seed()
    lab4.ask()
    r4 = lab4.hook('그래 "만들어" \\ 말고', stub_rc=0)
    check("O", "O-4 인용부호·역슬래시 발화 → stdout 은 여전히 유효 JSON 1줄 이하", one_json_line(r4), r4.lines)


# ══════════════════════════════════════════════════════════════════════════════
# F — 실패 정책(훅 실패가 프롬프트 제출을 막지 않는다 · 발급 0)
# ══════════════════════════════════════════════════════════════════════════════
def _break_module(binp):
    p = os.path.join(binp, "javis_teamtoken.py")
    w(p, "raise SystemExit(9)\n")


def _hang_module(binp):
    p = os.path.join(binp, "javis_teamtoken.py")
    w(p, "import time\ntime.sleep(40)\n")


def suite_failure():
    lab = Lab("f1", bin_mut=_break_module)
    lab.seed()
    # 원장을 손으로 최근 시각에 둔다(깨진 모듈로는 ask 를 열 수 없다) — 내용이 아니라 '최근'만 필요하다.
    w(lab.paths()["ledger"], "")
    r = lab.hook(APPROVE, stub_rc=0)
    check("F", "F-1 발급기 손상 → exit 0 · 본체 진행 · 발급 0 · stdout JSON 1줄 이하",
          r.returncode == 0 and r.body_runs == 1 and one_json_line(r) and not lab.events("token_issued"),
          "rc=%s body=%d lines=%r" % (r.returncode, r.body_runs, r.lines))
    check("F", "F-1b 승인처럼 들리는 발화의 판정 불가는 조용히 접히지 않는다(고지 1줄)",
          len(r.lines) == 1 and "판정 불가" in (ctx_of(r.lines[0]) or ""), r.lines)

    lab2 = Lab("f2", bin_mut=_hang_module)
    lab2.seed()
    w(lab2.paths()["ledger"], "")
    r2 = lab2.hook(APPROVE, stub_rc=0, timeout=120)
    check("F", "F-2 발급기 행 → 유계 시간 안에 exit 0 · 본체 진행(프롬프트 먹통 없음)",
          r2.returncode == 0 and r2.body_runs == 1 and r2.elapsed < 20 and one_json_line(r2),
          "rc=%s elapsed=%.1fs body=%d" % (r2.returncode, r2.elapsed, r2.body_runs))

    lab3 = Lab("f3", issuer=False)
    lab3.seed()
    lab3.ask()
    r3 = lab3.hook(APPROVE, stub_rc=0)
    check("F", "F-3 발급기 파일 부재 → exit 0 · 본체 진행 · 발급 0 · stdout 무출력 · stderr 고지",
          r3.returncode == 0 and r3.body_runs == 1 and not lab3.events("token_issued") and not r3.lines
          and "teamtoken-issue.sh" in r3.stderr, "lines=%r err=%r" % (r3.lines, r3.stderr[-200:]))

    lab4 = Lab("f4")
    lab4.seed()
    lab4.ask()
    w(lab4.paths()["delivery"], "")              # 0바이트 = 손상(판독 불가) — 안전 정지
    r4 = lab4.hook(APPROVE, stub_rc=0)
    ctx4 = ctx_of(r4.lines[0]) if r4.lines else ""
    check("F", "F-4 배달 원장 손상 → 발급 0 · 안전 정지 고지(ledger_unreadable)",
          not lab4.events("token_issued") and "ledger_unreadable" in (ctx4 or ""), (ctx4 or "")[:200])


# ══════════════════════════════════════════════════════════════════════════════
# R — 실 본체 통합
# ══════════════════════════════════════════════════════════════════════════════
def suite_real_body():
    lab = Lab("r1", body="real")
    lab.seed()
    lab.ask()
    r = lab.hook(APPROVE, newcli=False, timeout=120)
    iss = lab.events("token_issued")
    check("R", "R-1 실 본체 + 오너 승인 → exit 0 · 발급 1 · stdout JSON 정확히 1줄(발급 고지)",
          r.returncode == 0 and len(iss) == 1 and len(r.lines) == 1 and one_json_line(r)
          and ("--team-token %s" % iss[0]["token"]) in (ctx_of(r.lines[0]) or ""),
          "rc=%s issued=%d lines=%r err=%r" % (r.returncode, len(iss), r.lines, r.stderr[-300:]))


# ══════════════════════════════════════════════════════════════════════════════
# S — 구조
# ══════════════════════════════════════════════════════════════════════════════
def suite_structure():
    src = rd(ISSUER)
    check("S", "S-1 발급기 파일 실재(hooks/teamtoken-issue.sh)", bool(src), ISSUER)
    lines = src.splitlines()
    i = next((k for k, ln in enumerate(lines) if ln.startswith(". ") and "_lib.sh" in ln), None)
    check("S", "S-2 프리루드 2단 source + loud-skip",
          i is not None and 'CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh' in src and "_lib.sh 소실" in src)
    end = next((k for k in range(i, len(lines)) if lines[k].rstrip().endswith("exit 0; }")), None) \
        if i is not None else None
    follow = next((ln for ln in lines[end + 1:] if ln.strip()), "") if end is not None else ""
    check("S", "S-3 프리루드 직후 레인 redirect 줄(R-8 census 규약)", follow == REDIRECT, follow)
    check("S", "S-4 판정은 javis_teamtoken.py issue 에 위임(발급기는 판정하지 않는다 · 인터프리터는 $CYS_PY)",
          "javis_teamtoken.py" in src and " issue " in src and '"$CYS_PY"' in src
          and "machine_origin(" not in src and "approval_verdict(" in src)
    check("S", "S-5 네이티브 경로 변환(모듈·입력·고지 파일) — Windows 2벌 규약",
          src.count("cys_native_path") >= 3)
    lsrc = rd(LAUNCHER)
    check("S", "S-6 런처는 여전히 프리루드 규약 심볼을 쓰지 않는다(자기완결 · SELF-1d 와 같은 목록)",
          not any(m in lsrc for m in ("CYS_PY", "PYBIN", "python3", "cys_norm_", "cys_is_abs",
                                      "cys_native_path", "cys_require_surface", "cys_have_surface",
                                      "cys_path_has_prefix", "cys_shquote")))
    code = [ln for ln in lsrc.splitlines() if not ln.lstrip().startswith("#")]
    body = "\n".join(code)
    ia = body.find("teamtoken-issue.sh")
    ib = body.find("user-prompt-submit --input")
    check("S", "S-7 발급 판정이 신 파이프라인 위임(⑥ · rc 6/3 이면 본체를 건너뛴다)보다 앞",
          0 <= ia < ib, "issuer@%d delegate@%d" % (ia, ib))
    check("S", "S-8 런처 비주석 줄에 python 글자 없음(pyseal census ⓒ(i) 소비 훅 판정 밖 유지)",
          "python" not in body)
    for shbin in ("sh", "dash", "bash"):
        if shutil.which(shbin) is None:
            print("SKIP S-9 %s 부재" % shbin)
            continue
        for p in (ISSUER, LAUNCHER):
            if not os.path.isfile(p):
                continue
            rr = subprocess.run([shbin, "-n", p], capture_output=True, text=True)
            check("S", "S-9 %s -n %s" % (shbin, os.path.basename(p)), rr.returncode == 0, rr.stderr[-200:])


def main():
    for fn in (suite_structure, suite_acceptance, suite_cost, suite_voice, suite_stdout, suite_failure,
               suite_real_body):
        try:
            fn()
        except Exception as e:  # noqa: BLE001 — 스위트 예외는 FAIL 로 센다(조용한 누락 금지)
            import traceback
            traceback.print_exc()
            check(fn.__name__, "예외", False, repr(e))
    print("\n===== 요약 =====")
    suites = []
    for s, _c, _o in RESULTS:
        if s not in suites:
            suites.append(s)
    fails = 0
    for s in suites:
        rows = [o for ss, _c, o in RESULTS if ss == s]
        fails += rows.count(False)
        print("%s %d/%d" % (s, rows.count(True), len(rows)))
    shutil.rmtree(ROOT, ignore_errors=True)
    print("TEAMTOKEN-HOOK-%s" % ("OK" if fails == 0 else "FAIL(%d)" % fails))
    return 0 if fails == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
