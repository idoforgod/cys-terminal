#!/bin/sh
# teamtoken-issue.sh — 대화 승인 1회용 팀 생성 토큰의 **훅 쪽 발급 배선** (0.14.42 · 설계 §6-4·§11 R11·§13 P4)
#
# ★이 파일은 settings.json 에 등록하는 훅이 아니다. UserPromptSubmit 런처(`role-bootstrap.sh` ⑤-b)가
#   **이 좌석의 열린 질문 표지**(`<상태>/teamtoken-open-<레인>-s<좌석>` · fatal-fix R3-F2)가 있을 때만 부른다 — "열린 질문이 없으면 공짜"라는 비용
#   게이트는 런처가 소유한다(인터프리터를 띄우기 전에 셸 글롭 하나로 거른다 · 리뷰 F3). 여기까지 왔다는 것은
#   이 좌석에 답을 기다리는(또는 막 만료돼 고지 여유 안의) 질문(ask)이 있다는 뜻이다(다른 레인의 같은 번호 좌석은 상위집합).
#   ★발급기는 입력의 출처를 본다(리뷰 SEC-1·RR1-SEC-B): 넘기는 $1 은 런처가 **방금** 쓴 `<상태>/hook-input-<좌석>-<pid>.json`
#   (이름의 좌석 = 이 좌석)이어야 한다 — 다른 파일·stdin 으로 부르면 판정 없이 not_hook_caller 다(직접 호출 = 규약 위반 ·
#   감사 기록 · 거부 문구는 어느 조건에서 막혔는지 말하지 않는다 — 구체 사유는 원장에만).
#
# 왜 런처인가(본체 `role-bootstrap-legacy.sh` 가 아니라): 신 파이프라인(`cys hook user-prompt-submit
#   --input`)은 **기계 유래 프롬프트를 처리완료(rc 6)로 닫아 본체를 건너뛴다.** 발급 호출이 본체 안에
#   있으면 바로 그 경로 — 기계 배달된 "그래 만들어" — 에서 거부 사유가 원장에 남지 않는다(§13 P4 의
#   "issued 0줄(원장에 사유 기록)"이 실 CLI 에서 거짓이 된다). 모든 프롬프트가 지나는 한 점은 런처뿐이다.
#   런처는 프리루드 규약 심볼(인터프리터 해소 등)을 쓰지 않는 자기완결 파일이라(test_hook_launcher_split
#   SELF-1d), 인터프리터가 필요한 이 일은 프리루드를 소비하는 이 파일로 떼어 냈다.
#
# 계약:
#   인자   $1 = 훅 입력 파일(런처가 받은 UserPromptSubmit JSON · POSIX 표기)
#          $2 = 고지 출력 파일 — 2줄: ① 종류(issued|refused|unjudged) ② 모델에 갈 additionalContext 본문 1줄.
#               종류가 따로 있는 이유: 런처는 본체와 고지가 겹칠 수 있는 발화에서 **거부·판정 불가 고지만**
#               뺀다 — 발급 고지(토큰)는 빼지 않는다(`role-bootstrap.sh` ⑦ exec 직전).
#   stdout 무출력 — 런처가 /dev/null 로 버린다. 훅 stdout 계약(JSON 1줄 또는 무출력)은 런처가 지킨다:
#          런처는 이 파일의 고지를 자기 고지와 **한 줄로 합치거나**, 본체가 고지를 낼 수 있는 발화면 뺀다.
#   종료   언제나 0. 판정 불가·모듈 부재·시간 초과는 발급 0(fail-closed) + stderr 1줄이다.
#   판정   **하지 않는다.** 발급·거부는 `bin/javis_teamtoken.py issue` 가 단일 소유한다(ask 게이트 ·
#          배달 원장 대조 `javis_mission.machine_origin` · 승인 전문 일치 · 제안·본문 결박 — 설계 §6-6).
#          이 파일이 모듈에서 읽는 것은 **고지를 낼지** 가르는 `approval_verdict` 하나뿐이다(아래 ②).
#
# 고지(설계 §10 · 훅 stdout 의 additionalContext 1줄 — 이 팩의 관례: `role-bootstrap-legacy.sh` 의 note):
#   ① 발급(rc 0)  : 토큰 문자열 + 제안 id + 만료 + 다음 명령(`cys-dept create --team-token <토큰>`).
#      ★토큰을 모델에 보이는 이유: master 가 그 문자열을 인자로 넘겨야 생성이 집행된다. 노출된 토큰은
#        발급 좌석·그 제안 id·그 본문 sha256·TTL 120초·1회 소비에 결박돼 있어, 다른 좌석·다른 제안·
#        바뀐 본문·재사용·만료 어느 쪽으로도 쓸 수 없다(설계 §6-5 · 검증·소비는 데몬 쪽 P5).
#   ② 거부(rc 1)  : 사유 코드 + 오너에게 할 §10 문구 1줄 + "토큰 없음 → 만들지 마라".
#      질문을 **닫은** 거부(사람의 답 · 만료 — 만료는 사람 발화에서만 닫힌다)와 열린 질문 없는 승인 발화(ask_not_open)는
#      언제나 알린다. 질문을 닫지 않은 거부(기계 배달 · 배달 원장 부재/손상 · 입력 확인 실패)는 모듈이 **승인처럼 들린다고
#      판정한 발화일 때만** 알린다(판정 불가 = 알리지 않는다 · fatal-fix R1-01) — 질문이 열린 동안 master 좌석에 들어오는
#      모든 기계 push 마다 "시스템이 보낸 메시지"를 말하게 하면 그 자체가 잡음 폭주다. 거부 사실은 원장(issue_refused)에 남는다.
#   ③ 기반 고장(rc 2·4) : 모듈이 승인처럼 들린다고 판정한 발화일 때만 알린다(fatal-fix R1-05 · R3-F4).
#   ④ 결과 없음(시간 초과·해석 불가 출력·모듈 import 불가) : 승인처럼 들리거나(모듈 판정) 모듈 없이도 승인일 수 있는
#      모양(기계 라벨 없는 짧은 발화)일 때만 판정 불가 고지(fatal-fix R2-1). 모듈을 쓸 수 없으면 until 이 지난 표지를 여기서 걷는다.
#   무동작(rc 3 · 열린 질문 없음 등)은 아무것도 쓰지 않는다 — 고지 파일 자체가 생기지 않는다.
#   고지 본문에서 인용부호·역슬래시·제어문자를 걷어 낸다: 런처의 발행기는 셸 printf 라 그것들을 실을 수
#   없다(`role-bootstrap.sh` ③ · `_static_ctx` 와 같은 문안 규율).
set +e

# ★(fatal-fix WIN-5) 프리루드는 **읽기 가능 여부를 먼저 보고** 소스한다 — `.` 는 POSIX 특수 내장이라 대상이 없으면
#   비대화형 sh 가 그 자리에서 종료한다(`. a || . b || {…}` 의 `||` 로도 못 막는다: /bin/sh rc 1·dash rc 2 무음 실측).
#   런처가 이 파일을 `sh` 로 부르므로 종전 사슬은 _lib.sh 부재 시 '훅 강등' 고지 없이 조용히 죽었다(발급 0 = fail-closed 이나 무음).
#   런처 ①-b 와 같은 규율이다(2단 해소 순서 불변 · 형제 → 레인 팩).
_TT_LIB="$(dirname "$0")/_lib.sh"
[ -r "$_TT_LIB" ] || _TT_LIB="${CYS_PACK_DIR:-$HOME/.cys/pack}/hooks/_lib.sh"
[ -r "$_TT_LIB" ] || { echo "[cys-hook] _lib.sh 소실 — 훅 강등(teamtoken-issue)" >&2; exit 0; }
. "$_TT_LIB" 2>/dev/null || { echo "[cys-hook] _lib.sh 판독 실패 — 훅 강등(teamtoken-issue)" >&2; exit 0; }
command -v cys_lane_redirect >/dev/null 2>&1 && cys_lane_redirect "$@"

TT_IN="${1:-}"
TT_NOTE="${2:-}"
[ -n "$TT_IN" ] && [ -f "$TT_IN" ] && [ -n "$TT_NOTE" ] || exit 0

# 모듈 해소 — 형제 팩(`hooks/../bin`) → 레인 팩(CYS_PACK_DIR). 본체의 감지기 해소와 같은 2단 순서다.
TT_MOD="$(dirname "$0")/../bin/javis_teamtoken.py"
[ -f "$TT_MOD" ] || TT_MOD="${CYS_PACK_DIR:-$HOME/.cys/pack}/bin/javis_teamtoken.py"
if [ -z "${CYS_PY:-}" ]; then
  # 판정할 수 없다 = 발급 0(fail-closed). 모델 고지를 내지 않는 이유: 인터프리터가 없으면 승인처럼 들리는
  # 발화인지조차 가를 수 없어, 최근 창 안의 **모든 좌석·모든 프롬프트**에 같은 말을 붙이게 된다.
  # master 쪽 관측은 §7-3(질문 열림·발급 0 = javis_teamtoken.py status 의 approval_not_received)이 맡는다.
  echo "[cys-hook] teamtoken-issue: 인터프리터 미해소 — 대화 승인 판정 불가(발급 0 · fail-closed)" >&2
  exit 0
fi
if [ ! -f "$TT_MOD" ]; then
  echo "[cys-hook] teamtoken-issue: bin/javis_teamtoken.py 부재 — 대화 승인 판정 불가(발급 0 · fail-closed)" >&2
  exit 0
fi

# ── ① 발급 판정 1회(단일 소유자 호출) ─────────────────────────────────────────────────────
# 데드라인은 모듈의 원장 락 대기(LOCK_TIMEOUT_S=5s)보다 **길게** 잡는다 — 락 경합이면 모듈이 스스로
# lock_unavailable 로 fail-closed 판정을 내고, 우리가 먼저 죽여서 그 사유를 지우지 않게 한다.
# 죽이게 되더라도 안전 방향이다: 모듈은 닫힘을 토큰보다 먼저 쓰고 찢긴 꼬리를 없었던 일로 읽는다.
CYS_TT_ISSUE_TIMEOUT_S=6
CYS_TT_NOTE_TIMEOUT_S=3
TT_MOD_N="$(cys_native_path "$TT_MOD")"
TT_IN_N="$(cys_native_path "$TT_IN")"
TT_OUT="$(cys_timeout_run "$CYS_TT_ISSUE_TIMEOUT_S" "$CYS_PY" "$TT_MOD_N" issue --payload-file "$TT_IN_N" </dev/null)"
TT_RC=$?
# rc 3 = 무동작(열린 질문 없음 · 좌석 미상 · 비승인 발화의 기반 고장) — 모듈은 아무것도 출력하지 않았다.
[ "$TT_RC" = "3" ] && exit 0

# ── ② 고지 본문 1줄(판정 결과의 서술 — 판정을 다시 하지 않는다) ─────────────────────────────
# ★(fatal-fix R1-01·R1-05·R3-F4·R2-1) 고지 정책 — **모듈이 승인처럼 들린다고 판정한 발화에만** 거부·기반 고장을 알린다.
#   종전엔 판독 불가(None: 입력 파일 소실·모듈 import 실패)를 '시끄럽게'로 읽었고(rc 1) rc 2·4 는 발화와 무관하게 알렸다 —
#   질문이 열린 동안 워커·리뷰어·부서 좌석이 받은 평범한 기계 push([wakeup]·[report_gate]·[CYCLE])에 "규약 위반 · 오너에게
#   다시 쳐 달라 · 앱을 재시작" 같은 엉뚱한 지시가 붙었다(③ '이해 안 되는 말'의 씨앗 · 옮기면 ①). 이제:
#     · 질문을 닫은 거부(사람의 답 · 만료)와 ask_not_open(모듈이 승인 유사 + 이 좌석 제안일 때만 냄) → 알린다.
#     · 그 밖의 거부(rc 1 · 기계 유래 · 배달 원장 · 입력 확인 실패)와 rc 2·4 → approval_like() is True 일 때만.
#     · 결과 없음(시간 초과·해석 불가 · 모듈 import 불가) → maybe_approval()(모듈 판정 또는 '승인일 수 있는 모양') 일 때만.
#   ★모듈을 쓸 수 없으면(tt 없음 · 결과 없음) 표지를 걷는 주체가 없어진다 — 여기서 until 이 지난 표지를 걷는다(R2-1 영구 반복 차단).
# ★(fatal-fix WIN-2) 발급기 출력은 ensure_ascii=False 인 한글 UTF-8 이다 — stdin 을 로케일 인코딩(한국어 윈도우 cp949 ·
#   LC_ALL=ko_KR.eucKR)으로 읽으면 UnicodeDecodeError 로 발급 고지가 사라진다. 바이트로 읽어 UTF-8 로 푼다.
CYS_TT_NOTE_PY='import json, os, re, sys, time
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass
rc_raw, note_p, payload_p, bindir, deadline = sys.argv[1:6]
marker_dir = sys.argv[6] if len(sys.argv) > 6 else ""
rc = int(rc_raw) if rc_raw.isdigit() else -1
res = None
try:
    _raw_in = sys.stdin.buffer.read().decode("utf-8", "replace")
except Exception:
    _raw_in = ""
for ln in reversed(_raw_in.splitlines()):
    ln = ln.strip()
    if ln.startswith("{"):
        try:
            res = json.loads(ln)
        except ValueError:
            res = None
        break
if not isinstance(res, dict):
    res = None
tt = None
try:
    sys.path.insert(0, bindir)
    import javis_teamtoken as tt
except BaseException:
    tt = None


def prompt_text():
    try:
        with open(payload_p, "rb") as f:
            obj = json.loads(f.read().decode("utf-8", "replace"))
        p = obj.get("prompt") if isinstance(obj, dict) else None
        return p if isinstance(p, str) else None
    except BaseException:
        return None


def approval_like():
    """True/False = 승인처럼 들리는가(모듈 판정) · None = 판정 불가(모듈·입력 판독 실패)."""
    if tt is None:
        return None
    p = prompt_text()
    if p is None:
        return None
    try:
        return tt.approval_verdict(p)[0] == "approve"
    except BaseException:
        return None


def maybe_approval():
    """판정 불가 고지용 — 모듈이 판정하면 그 답 · 못 하면 승인일 수 있는 모양(기계 라벨 [..] 로 시작하지 않는 짧은 발화).
    승인 화이트리스트 원소는 어느 것도 [ 로 시작하지 않고 길이 상한이 짧다 — 사본이 아니라 그 필요조건이다."""
    al = approval_like()
    if al is not None:
        return al
    p = prompt_text()
    if p is None:
        return False
    s = p.strip()
    return bool(s) and not s.startswith("[") and len(s) <= 200


def sweep_markers():
    """모듈 없이 until 이 지난 열린 질문 표지를 걷는다(판독 불가면 mtime + 질문 TTL + 고지 여유)."""
    if not marker_dir:
        return
    grace = 600.0
    if tt is not None:
        try:
            grace = float(tt.ASK_TTL_S) + float(tt.ASK_NOTICE_GRACE_S)
        except Exception:
            grace = 600.0
    now = time.time()
    try:
        names = os.listdir(marker_dir)
    except OSError:
        return
    for n in names:
        if not n.startswith("teamtoken-open-"):
            continue
        p = os.path.join(marker_dir, n)
        until = None
        try:
            with open(p, "rb") as f:
                d = json.loads(f.read(4096).decode("utf-8"))
            u = d.get("until") if isinstance(d, dict) else None
            if isinstance(u, (int, float)) and not isinstance(u, bool):
                until = float(u)
        except Exception:
            until = None
        try:
            if until is not None:
                if until < now:
                    os.remove(p)
            elif os.path.getmtime(p) + grace < now:
                os.remove(p)
        except OSError:
            pass


def clean(s, cap=400):
    s = re.sub(r"[\x00-\x1f\x7f]+", " ", str(s or ""))
    s = s.replace(chr(92), "/").replace(chr(34), chr(39))
    s = re.sub(r" {2,}", " ", s).strip()
    return s if len(s) <= cap else s[:cap - 1] + "…"


def hhmmss(ts):
    try:
        return time.strftime("%H:%M:%S", time.localtime(float(ts)))
    except Exception:
        return "?"


if tt is None or res is None or rc not in (0, 1):
    sweep_markers()
note = ""
kind = "refused"
code = (res or {}).get("code") or ""
if rc == 0 and res and res.get("ok") and code == "token_issued" and res.get("token"):
    kind = "issued"
    tok = res["token"]
    ttl = int(getattr(tt, "TOKEN_TTL_S", 120)) if tt is not None else 120
    note = ("[팀 만들기 대화 승인 - 1회용 생성 토큰 발급] 오너가 이 좌석에서 직접 친 승인으로 확인됐다"
            "(배달 원장 대조: 기계 배달 아님). token=%s · 제안 %s · 유효 %d초(만료 %s) · 1회용. "
            "다음: %s — 이후 순서는 MASTER_DIRECTIVE §4-A-2(생성 → 부트 티켓 확인 → 생성 성공 뒤 feed reply allow "
            "→ 오너 1줄 보고로 턴 종료). 각성 지시·첫 과제는 이 턴에 부서장에게 보내지 않는다 — 편성 도구가 "
            "부서장에게 각성 지시를, 너에게 편성 알림을 보낸다 · 알림이 오면 그대로 1회 한다(이 턴에서 기다리지 않는다 · sleep·반복 조회 금지). "
            "이 토큰은 이 좌석·이 제안·지금 본문에만 한 번 유효하다(다른 제안·"
            "다른 좌석·본문 변경·재사용·만료는 거부된다). 토큰 문자열은 명령 인자로만 쓰고 오너 보고문·"
            "다른 좌석 전달문에 싣지 마라."
            % (tok, res.get("proposal_id") or "?", ttl, hhmmss(res.get("expires_at")),
               res.get("next") or ("cys-dept create --team-token %s" % tok)))
elif res and rc in (1, 2, 4):
    closed = bool(res.get("ask_closed"))
    if closed or code == "ask_not_open":
        loud = True
    else:
        loud = approval_like() is True
    if loud:
        if closed and code == "ask_expired":
            state = "질문(ask)은 만료로 닫혔다 — 다시 승인받으려면 ask 부터 다시 연다"
        elif closed:
            state = "질문(ask)은 이 답으로 닫혔다 — 다시 승인받으려면 ask 부터 다시 연다"
        elif code == "ask_not_open":
            state = ("열린 질문(ask)이 없다 — 먼저 javis_teamtoken.py ask --proposal %s 로 질문을 연 뒤 "
                     "다시 여쭤라" % (res.get("proposal_id") or "<제안 id>"))
        elif code == "machine_origin":
            state = "질문(ask)은 열린 채 남아 있다 — 이 발화는 기계 배달로 판정됐다(오너 답으로 세지 않았다)"
        elif code == "not_hook_caller":
            state = "질문(ask)은 열린 채 남아 있다 — 이 프롬프트의 훅 입력을 확인하지 못해 판정하지 않았다"
        elif rc == 1:
            state = "질문(ask)은 열린 채 남아 있다(이 발화는 오너 답으로 세지 않았다)"
        else:
            state = "발급기 기반 고장(fail-closed)"
        owner = res.get("message") or ""
        if code == "machine_origin":
            say = ("오너가 방금 직접 친 말일 때만 오너에게 1줄로: 「%s」 — 오너가 치지 않았다면(에이전트·시스템 push) "
                   "오너에게 아무 말도 하지 말고 질문을 연 채 기다려라." % owner)
        else:
            say = "오너에게 1줄로: 「%s」" % owner
        why = ("훅 입력 확인 실패(경합·시계 — 구체 사유는 원장 issue_refused)" if code == "not_hook_caller"
               else clean(res.get("detail"), 160))
        note = ("[팀 만들기 대화 승인 - 토큰 미발급 · %s] %s. %s 토큰이 없으므로 "
                "cys-dept create 를 부르지 마라(우회 금지). 근거: %s · 상태 확인: javis_teamtoken.py status"
                % (code or "?", state, say, why))
    else:
        sys.stderr.write("[cys-hook] teamtoken-issue: 거부(%s · rc=%s) — 승인처럼 들리지 않는(또는 판정 불가) 발화라 "
                         "고지 생략(원장 issue_refused 에 기록됨)\n" % (code, rc_raw))
else:
    kind = "unjudged"
    if maybe_approval():
        why = ("데드라인 %s초 안에 끝나지 않았다" % deadline) if rc == 124 else \
              ("판독 가능한 결과 없이 끝났다(rc=%s)" % rc_raw)
        tail = ""
        if tt is not None:
            try:
                tail = " 오너에게 필요하면: 「%s」" % tt.owner_message("approval_not_received")
            except Exception:
                tail = ""
        note = ("[팀 만들기 대화 승인 - 판정 불가] 발급기(javis_teamtoken.py issue)가 %s. 이 발화에 토큰이 "
                "발급됐는지 확정할 수 없다 — javis_teamtoken.py status 로 확인하고 token_ready 가 아니면 "
                "만들지 마라.%s" % (why, tail))
    else:
        sys.stderr.write("[cys-hook] teamtoken-issue: 판정 불가(rc=%s) — 승인일 수 없는 발화라 고지 생략(발급 0)\n"
                         % rc_raw)
if note:
    with open(note_p, "w", encoding="utf-8", newline="\n") as f:
        f.write(kind + "\n" + clean(note, 1200) + "\n")'
# ★(fatal-fix WIN-2) `-X utf8`: 비-UTF-8 로케일(LC_ALL=ko_KR.eucKR 등)에서 파이썬은 `-c` 프로그램 **텍스트 자체**를 로케일
#   인코딩으로 푼다 — 한글이 든 이 프로그램이 SyntaxError 로 죽어 발급 고지가 통째로 사라졌다(실측). UTF-8 모드는 argv·stdin 을
#   UTF-8 로 푼다(파이썬 ≥3.7 · 번들 3.12 포함).
printf '%s' "$TT_OUT" | cys_timeout_run "$CYS_TT_NOTE_TIMEOUT_S" "$CYS_PY" -X utf8 -c "$CYS_TT_NOTE_PY" \
  "$TT_RC" "$(cys_native_path "$TT_NOTE")" "$TT_IN_N" "$(cys_native_path "${TT_MOD%/*}")" "$CYS_TT_ISSUE_TIMEOUT_S" \
  "$(cys_native_path "${TT_IN%/*}")"
echo "[cys-hook] teamtoken-issue: 발급 판정 rc=$TT_RC(0=발급 1=거부 4=기반 고장 124=시간 초과)" >&2
exit 0
