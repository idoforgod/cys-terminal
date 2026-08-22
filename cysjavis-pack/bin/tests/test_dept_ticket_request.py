#!/usr/bin/env python3
"""test_dept_ticket_request.py — 부서 레인 CEO 티켓 **자동 요청** 회귀 핀 (2026-08-22 결함 #2).

## 재현 대상
부서 레인 부트의 ③″ CEO 티켓 게이트가 티켓 부재를 감지하고도 **안내문만 출력하고 끝났다**.
그래서 부서장은 팀 없이 대기했고, 오너가 추가 명령을 쳐야만 CEO 에게 요청이 갔다 —
실측 2026-08-22: 06:20 단독각성 → 06:26 **오너 추가입력** → 06:29 요청 → 06:30 발급 → 팀 기동.
오너 절대규칙은 "선언자는 새 부서장이 되며 팀(cso·워커·리뷰어들)이 **기동돼야 한다**"이다.

## 이 파일이 못박는 것 (실 데몬 무접촉 — 임시 HOME + PATH 선두 `cys` 스텁)
  1) 티켓 부재 + 살아있는 CEO → 요청 push **발사**(명령 조립: `cys send --queued --to master`,
     `env -u CYS_SOCKET` 동형으로 base 레인 도달, 문안에 발급 명령 동봉) · exit 0(단독 각성)
  2) 요청 마커 TTL 안 재부팅 → **중복 push 없음**(멱등) / TTL 밖 → 재요청(영구 침묵 금지)
  3) push 실패(스텁 send 비0) → 경고만, **exit 0**(fail-open · 부트 무중단)
  4) CEO 부재(status 로스터 빈) → 요청 보류 + exit 0 (종전 동작 그대로)
  5) 요청 대기 중 티켓이 도착하면 **팀 기동까지 이어간다**(④ cys boot 호출·티켓 1회성 소비)
  6) #4-a: 단독 각성 보고가 원인·현재 상태·다음 단계를 말한다(요청함/못 보냄 두 갈래)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(SELF, "..", "javis_bootstrap.py")
PY = sys.executable or "python3"

fails = []


def check(name, cond, detail=""):
    print("%s %s%s" % ("PASS" if cond else "FAIL", name, (" — " + detail) if detail else ""))
    if not cond:
        fails.append(name)


DEPT = "dept-9"

# 스텁 cys — base 레인(CYS_SOCKET 미설정)에서만 살아있는 CEO 를 보고한다.
# `send` 는 send_exit 로, 티켓 발급은 `ticket_on_send` 로 흉내낸다(요청→발급 왕복 재현).
_CYS = """#!/bin/sh
echo "cys $@ [sock=${CYS_SOCKET:-base}]" >> "%(t)s/calls.log"
case "$1" in
  ping) exit 0;;
  claim-role) exit 0;;
  boot) exit 0;;
  list) exit 0;;
  --version) echo 'cys 0.0.0-stub'; exit 0;;
  status)
    if [ -z "${CYS_SOCKET:-}" ] && [ %(ceo)d -eq 1 ]; then
      echo '{"surfaces":[{"surface_id":9,"role":"master","exited":false,"agent":"claude","agent_alive":true,"seat":"occupied"}]}'
    else
      echo '{"surfaces":[]}'
    fi
    exit 0;;
  send)
    if [ %(ticket_on_send)d -eq 1 ]; then
      mkdir -p "%(tickets)s"
      printf '{"dept":"%(dept)s","issued_at":%(now)s,"issuer":"stub-ceo"}\\n' \\
        > "%(tickets)s/%(dept)s.ticket"
    fi
    exit %(send_exit)d;;
esac
exit 0
"""


def make_env(tmp, *, ceo_alive=True, send_exit=0, ticket_on_send=0, wait_s=6, dept=DEPT):
    """임시 HOME + 가짜 부서 팩 + 스텁 → (env, home). 부서 레인(CYS_SOCKET=부서 소켓).
    ★dept 는 소켓과 팩 이름을 **함께** 정한다 — 어긋나면 레인↔팩 가드(exit 8)가 ③″ 앞에서 끊는다."""
    home = os.path.join(tmp, "home")
    pack = os.path.join(home, ".cys", "pack-dept-%s" % dept)
    bindir = os.path.join(tmp, "stubbin")
    tickets = os.path.join(home, ".cys", "state", "dept-boot-tickets")
    for d in (os.path.join(pack, "bin"), bindir):
        os.makedirs(d, exist_ok=True)

    def w(path, body, mode=0o755):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        os.chmod(path, mode)

    import time as _t
    w(os.path.join(bindir, "cys"), _CYS % {"t": tmp, "ceo": 1 if ceo_alive else 0,
                                           "send_exit": send_exit,
                                           "ticket_on_send": ticket_on_send,
                                           "tickets": tickets, "dept": dept,
                                           "now": repr(_t.time())})
    w(os.path.join(pack, "bin", "javis_preflight.py"), "import sys; sys.exit(0)\n", 0o644)
    w(os.path.join(pack, "bin", "javis_orchestra.py"),
      "import sys; sys.exit(0)\n", 0o644)
    w(os.path.join(pack, "bin", "cys-dept"), "#!/bin/sh\nexit 0\n")

    env = dict(os.environ)
    env.update({"HOME": home, "PATH": bindir + os.pathsep + env.get("PATH", ""),
                "CYS_PACK_DIR": pack,
                "CYS_SOCKET": os.path.join(tmp, "state", "cys-dept-%s" % dept, "cys.sock"),
                "CYS_SURFACE_ID": "7",
                "CYS_BOOT_CHECK_RETRIES": "2", "CYS_BOOT_CHECK_INTERVAL_S": "0.05",
                # 유계 대기를 테스트 예산으로 좁힌다(상수는 env 로 노출돼 있다)
                "CYS_DEPT_TICKET_WAIT_S": str(wait_s),
                "CYS_DEPT_TICKET_WAIT_INTERVAL_S": "1"})
    env.pop("CYS_BOOT_GATE", None)
    return env, home


def run(env):
    r = subprocess.run([PY, SCRIPT], capture_output=True, text=True,
                       encoding="utf-8", env=env, timeout=120)
    return r.returncode, r.stdout, r.stderr


def calls(tmp):
    p = os.path.join(tmp, "calls.log")
    return open(p, encoding="utf-8").read() if os.path.exists(p) else ""


def send_lines(tmp):
    return [l for l in calls(tmp).splitlines() if l.startswith("cys send ")]


def summary_of(out):
    try:
        return json.loads(out.strip().splitlines()[-1])
    except Exception:
        return {}


def req_marker(home):
    return os.path.join(home, ".cys", "state", "dept-ticket-requests", "%s.request" % DEPT)


# ── 1. 티켓 부재 + CEO 생존 → 요청 push 발사 · 명령 조립 검증 · exit 0 ──
tmp = tempfile.mkdtemp(prefix="tq-t1-")
env, home = make_env(tmp)
code, out, err = run(env)
check("1a 티켓 부재 부트 exit 0(단독 각성 · fail-open)", code == 0, "exit=%d err=%s" % (code, err[-300:]))
sends = send_lines(tmp)
check("1b ★요청 push 1회 발사(오너 개입 0)", len(sends) == 1, "sends=%r" % sends)
line = sends[0] if sends else ""
check("1c 명령 조립: --queued --to master", "cys send --queued --to master" in line, line)
check("1d 문안에 부서명·발급 명령 동봉",
      ("[부서장→CEO 요청] %s 부서장입니다" % DEPT) in line
      and ("issue-ticket --dept %s" % DEPT) in line, line)
check("1e ★base 레인 도달(부서 소켓 미상속 — env -u CYS_SOCKET 동형)",
      "[sock=base]" in line, line)
check("1f 요청 마커 기록(멱등 근거)", os.path.exists(req_marker(home)))
s = summary_of(out)
check("1g 요약 JSON: 단독각성 + 요청 사실 기계 노출",
      s.get("solo_awakening") is True and s.get("ticket_requested") is True, json.dumps(s)[:200])
check("1h #4-a 보고가 '요청했고 대기 중·도착 시 자동 기동'을 말한다",
      "요청했습니다" in err and "자동 기동" in err, err[-400:])
check("1i 팀 기동 생략(티켓 미도착)",
      not any(l.startswith("cys boot") for l in calls(tmp).splitlines()), calls(tmp)[-300:])
shutil.rmtree(tmp)

# ── 2. 멱등: TTL 안 재부팅 = 중복 push 0 / TTL 밖 = 재요청 ──
tmp = tempfile.mkdtemp(prefix="tq-t2-")
env, home = make_env(tmp)
run(env)
check("2a 1회차 요청 발사", len(send_lines(tmp)) == 1)
run(env)
check("2b ★TTL 안 재부팅은 중복 push 없음(CEO 큐 스팸 차단)",
      len(send_lines(tmp)) == 1, "sends=%r" % send_lines(tmp))
# 마커 시각을 TTL 밖으로 밀면 다시 요청한다(영구 침묵 금지)
m = json.load(open(req_marker(home), encoding="utf-8"))
m["requested_at"] = m["requested_at"] - 100000
with open(req_marker(home), "w", encoding="utf-8") as f:
    json.dump(m, f)
run(env)
check("2c ★TTL 경과 후에는 재요청(영구 침묵 금지)",
      len(send_lines(tmp)) == 2, "sends=%r" % send_lines(tmp))
shutil.rmtree(tmp)

# ── 3. push 실패 → 경고만·exit 0(fail-open) ──
tmp = tempfile.mkdtemp(prefix="tq-t3-")
env, home = make_env(tmp, send_exit=1)
code, out, err = run(env)
check("3a ★push 실패해도 exit 0(부트 무중단)", code == 0, "exit=%d err=%s" % (code, err[-300:]))
check("3b 요청 실패가 기계 필드로 드러난다",
      summary_of(out).get("ticket_requested") is False, json.dumps(summary_of(out))[:200])
check("3c 요청 마커 미기록(다음 부트가 재시도한다)", not os.path.exists(req_marker(home)))
check("3d #4-a 보고에 수동 발급 명령 안내",
      "보내지 못했습니다" in err and "issue-ticket --dept %s" % DEPT in err, err[-400:])
shutil.rmtree(tmp)

# ── 4. CEO 부재 → 요청 보류·exit 0 (종전 동작 보존) ──
tmp = tempfile.mkdtemp(prefix="tq-t4-")
env, home = make_env(tmp, ceo_alive=False)
code, out, err = run(env)
check("4a CEO 부재에서도 exit 0", code == 0, "exit=%d" % code)
check("4b 요청 push 미발사(없는 수신자에게 던지지 않는다)", not send_lines(tmp),
      "sends=%r" % send_lines(tmp))
check("4c 요약에 요청 안 됨 명시", summary_of(out).get("ticket_requested") is False)
shutil.rmtree(tmp)

# ── 5. 요청 → 티켓 도착 → **팀 기동까지 이어간다**(결함 #2 의 목적) ──
tmp = tempfile.mkdtemp(prefix="tq-t5-")
env, home = make_env(tmp, ticket_on_send=1)
code, out, err = run(env)
check("5a 티켓 도착 부트 exit 0", code == 0, "exit=%d err=%s" % (code, err[-400:]))
check("5b ★팀 기동으로 이어짐(cys boot 호출)",
      any(l.startswith("cys boot") for l in calls(tmp).splitlines()), calls(tmp)[-400:])
check("5c 단독 각성이 아니다", summary_of(out).get("solo_awakening") is not True,
      json.dumps(summary_of(out))[:200])
check("5d 티켓 1회성 소비(.used)",
      os.path.exists(os.path.join(home, ".cys", "state", "dept-boot-tickets",
                                  "%s.ticket.used" % DEPT)))
shutil.rmtree(tmp)

# ── 6. ★적대검증 중대③ — 부서명 규약 비대칭(대문자·`_` 부서가 티켓을 영영 못 받는다) ──
#    생성기 `cys-dept::dept_name_ok` = ^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$ 인데 발급기는
#    `[a-z0-9][a-z0-9-]*` 였다. 오너가 `Sales`·`dept_1` 로 부서를 만들면 결함 #1 이 그대로 남는다.
tmp = tempfile.mkdtemp(prefix="tq-t6-")
env, home = make_env(tmp)
env_base = dict(env)
env_base.pop("CYS_SOCKET", None)
for name in ("Sales", "dept_1", "A9_x-y"):
    r = subprocess.run([PY, SCRIPT, "issue-ticket", "--dept", name],
                       capture_output=True, text=True, encoding="utf-8", env=env_base, timeout=60)
    check("6a ★생성기가 허용하는 이름 %r 을 발급기도 허용" % name, r.returncode == 0,
          "exit=%d %s" % (r.returncode, (r.stderr or "")[-160:]))
    check("6b 티켓 파일 생성 %r" % name,
          os.path.exists(os.path.join(home, ".cys", "state", "dept-boot-tickets",
                                      "%s.ticket" % name)))
for name in ("Bad/Name", "-lead", "a.b"):
    r = subprocess.run([PY, SCRIPT, "issue-ticket", "--dept", name],
                       capture_output=True, text=True, encoding="utf-8", env=env_base, timeout=60)
    check("6c 경로 위험 이름 %r 은 여전히 거부" % name, r.returncode == 2, "exit=%d" % r.returncode)
shutil.rmtree(tmp)

# ── 7. ★중대③ 후반 — 발급기가 거부할 이름이면 CEO 큐에 요청을 넣지 않는다 ──
#    종전 배치는 요청이 정규식 경고보다 **먼저** 나가, "실행하면 exit 2 가 나오는 명령"이
#    억제 TTL 주기로 CEO 큐에 쌓였다(수신자가 할 수 있는 일이 없는 요청 = 소음).
tmp = tempfile.mkdtemp(prefix="tq-t7-")
BAD = "Bad.Name"
env, home = make_env(tmp, dept=BAD)
code, out, err = run(env)
check("7a 불량 부서명이어도 부트 exit 0(fail-open)", code == 0, "exit=%d" % code)
check("7b ★요청 push 미발사(실패할 명령을 큐에 넣지 않는다)", not send_lines(tmp),
      "sends=%r" % send_lines(tmp))
check("7c 단독 각성 보고에 재생성 안내", "재생성" in err, err[-300:])
shutil.rmtree(tmp)

print("\n%d FAIL" % len(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
