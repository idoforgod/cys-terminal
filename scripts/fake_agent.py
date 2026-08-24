#!/usr/bin/env python3
"""fake_agent.py — 첫기동 관문 결정론 TUI 스텁 · 14 시나리오 (U-26 ③ · 2026-08-24).

    python3 scripts/fake_agent.py --scenario bypass-dialog     # 실제로 띄운다(키를 stdin 으로 받는다)
    python3 scripts/fake_agent.py --list --json                # 시나리오 대장(기계 판독)
    python3 scripts/fake_agent.py --self-test                  # 선언 자체의 무결성 검사

★이것이 왜 있는가 — 이 캠페인이 확정한 4대 근본원인 중 하나가 **"진짜 e2e 테스트가 0개"** 였다.
  관문 판정·키 주입·좌색 생사는 지금까지 전부 '소스 문자열 핀' 으로만 검증됐다. 소스 핀은
  "그렇게 쓰여 있다"를 증명할 뿐 **"그 키를 누르면 실제로 그렇게 된다"** 를 증명하지 못한다.
  이 스텁은 실제 프로세스로 뜨고, 실제 키(이스케이프 시퀀스 포함)를 받고, 실제 종료코드로 죽는다.

★★가장 중요한 계약 — **틀린 키를 박제하지 않는다**
  구 설계는 면책 대화상자(bypass) 통과 액션을 `Left + Return` 으로 적었다. 실측은 그 반대다:
    · 기본 포커스가 `1. No, exit` 이고 **Return 한 발이 rc 1 로 좌석을 죽인다**
    · 세로 번호 리스트라 `Left` 는 포커스를 움직이지 못한다 → `Left+Return` 도 **rc 1**
    · 통과는 `아래방향+Return` 또는 숫자 `2`
  스텁이 구 설계대로 만들어졌다면 **결함이 CI 초록으로 승인**된다(계측기 타당성 위반).
  그래서 `key_oracle == "measured"` 인 시나리오의 화면·기본 포커스·종료코드는 손으로 쓰지 않고
  `docs/evidence/probe-2026-08-23-first-run-gates.json`(실측 캡처)에서 **적재**한다.
  실측하지 않은 화면은 `key_oracle` 을 `"measured"` 로 선언할 수 없다(--self-test 가 막는다).

★안전(이 저장소 제1 계약 — 오살이 오탐보다 위험하다)
  이 스텁은 **아무것도 죽이지 않는다**: 자기 자신만 종료하고, 파일을 쓰지 않고, 네트워크를
  열지 않고, cys 데몬·좌석·프로필을 건드리지 않는다. 브라우저 실행도 **문자열 모사**다.

키 어휘: `Up/Down/Left/Right`(ANSI 이스케이프) · `Return`(CR/LF) · 숫자 · `Esc` · `Ctrl-D`(최종보고).
종료코드: 0=생존 후 최종보고 · 1=시나리오가 선언한 거절 종료 · 3=입력 없이 유휴 타임아웃(측정 실패).
"""
import argparse
import json
import os
import queue
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
EVIDENCE = os.path.join(REPO, "docs", "evidence", "probe-2026-08-23-first-run-gates.json")
EV_REF = "docs/evidence/probe-2026-08-23-first-run-gates.json"
CORPUS_REF = "src/first_run_gates.rs"

RC_ALIVE = 0
RC_REJECTED = 1
RC_IDLE = 3

# ★롤백/변조 단일 지점 — 이 env 하나만 스텁 의미론을 바꾼다. 기본값(미설정)이 실측 의미론이다.
#   용도는 **계측 타당성 시험 전용**이다: 오라클이 틀린 의미론을 실제로 적발하는지 보려면
#   틀린 의미론을 만들어 볼 수 있어야 한다. 운영 경로에는 이 값을 넣지 않는다.
MUTATE_ENV = "CYS_FAKE_AGENT_MUTATE"
MUTATIONS = {
    "left-moves-focus": "Left/Right 가 포커스를 옮긴다(구 설계의 `Left+Return` 오라클이 참이 되는 세계)",
    "return-accepts": "Return 이 포커스와 무관하게 언제나 수락한다(‘Return 은 안전하다’ 가정)",
    "no-exit": "거절 선택이 프로세스를 죽이지 않는다(rc 1 사망을 감춘다)",
    "digit-ignored": "숫자 키를 무시한다(코퍼스가 선언한 리터럴 `2` 통과가 무력화된다)",
}


def _mutation():
    m = (os.environ.get(MUTATE_ENV) or "").strip()
    return m if m in MUTATIONS else ""


# ── 실측 캡처 적재 ───────────────────────────────────────────────────────────
def load_evidence(path=EVIDENCE):
    """실측 캡처를 적재한다. **부재는 조용한 폴백이 아니라 즉시 실패다** — 스텁이 손으로 쓴
    화면으로 조용히 대체되는 순간 '실측 픽스처' 라는 주장 전체가 거짓이 된다."""
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
    if d.get("schema") != "cys-evidence/1":
        raise SystemExit("증거 스키마 불일치: %s" % path)
    return d


def _ev_screen(ev, gate_id):
    s = (ev.get("screens") or {}).get(gate_id)
    if not s:
        raise SystemExit("실측 캡처에 화면이 없다: %s#screens.%s" % (EV_REF, gate_id))
    return s


# ── 시나리오 정의 ────────────────────────────────────────────────────────────
def build_scenarios(ev=None):
    ev = ev or load_evidence()
    k = ev.get("key_facts") or {}
    theme = _ev_screen(ev, "theme-select")
    login = _ev_screen(ev, "login-select")
    oauth = _ev_screen(ev, "oauth-code-prompt")
    trust = _ev_screen(ev, "folder-trust")
    byp = _ev_screen(ev, "bypass-disclaimer")

    def menu(sc, **kw):
        """실측 캡처 한 화면 → 메뉴 단계. **거절 경로의 등급을 명시**한다.

        ★`reject_mode` 가 왜 필요한가: 실측한 것은 대개 '기본 포커스에서 Return 을 눌렀을 때'
          하나다. 반대편 선택지를 눌렀을 때 무슨 일이 나는지는 재지 않은 화면이 많다. 그걸
          '아무 일 없음' 으로 모델링하면 스텁이 **재지 않은 사실을 단언**하게 된다 —
          이 파일이 막으려는 바로 그 오염이다. 그래서 미측정 거절은 `unmeasured` 로 표시하고
          오라클을 만들지 않는다."""
        rc = sc.get("exit_rc")
        st = {"kind": "menu", "screen": sc["text"], "options": sc["options"],
              "default_index": sc["default_index"], "accept_index": sc.get("accept_index"),
              "reject_rc": rc, "reject_mode": ("exit" if rc else "unmeasured"),
              "echo": sc.get("echo")}
        st.update(kw)
        return st

    S = []

    # ① 양성 대조 — 관문이 하나도 없는 건강한 프롬프트.
    S.append({
        "id": "ready-prompt", "gate_id": None, "provenance": "synthetic", "key_oracle": "none",
        "source": "엔진 양성 대조(관문 없음)",
        "note": "관문 판정이 **아무 화면에나** 걸리지 않는지 보는 음성 대조축이다.",
        "steps": [{"kind": "ready", "screen": "❯ ", "reply": "(echo) %s"}],
    })

    # ② 온보딩 테마 선택 — 실측 측정1 ①.
    S.append({
        "id": "theme-select", "gate_id": "theme", "provenance": "measured", "key_oracle": "measured",
        "source": EV_REF + "#screens.theme-select",
        "note": "신규 프로필의 **첫 관문은 면책이 아니라 온보딩**이다(B-4).",
        "steps": [menu(theme, accept_index=theme["default_index"]),
                  {"kind": "stuck", "screen": "(온보딩 다음 화면 — 이 시나리오의 관측 범위 밖)\n"}],
    })

    # ③ 로그인 방법 선택 — Return 이 브라우저를 연다(부작용 모사) · 프로세스는 계속 산다.
    S.append({
        "id": "login-select", "gate_id": "login-method-select", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.login-select",
        "note": "기계가 통과시킬 수 없는 관문(human_only). Return 은 브라우저를 여는 **부작용**이다.",
        "steps": [menu(login, accept_index=login["default_index"],
                       side_effect="(모사) 브라우저를 여는 시늉만 한다 — 실제로 열지 않는다"),
                  {"kind": "prompt", "screen": oauth["text"], "on_return": "loop",
                   "loop_text": ev["screens"]["oauth-code-prompt"]["loop_text"]}],
    })

    # ④ OAuth 코드 입력 — 빈 Return 무한 재시도 · 영원히 살아 있다(B-6).
    S.append({
        "id": "oauth-code-prompt", "gate_id": "oauth-code-prompt", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.oauth-code-prompt",
        "note": "생존만 보는 판정(alive_presumed)이 이 좌석을 영원히 '준비됨' 으로 오탐한다.",
        "steps": [{"kind": "prompt", "screen": oauth["text"], "on_return": "loop",
                   "loop_text": oauth["loop_text"]}],
    })

    # ⑤ 이미 루프 안에 있는 상태 — 몇 번을 눌러도 rc 가 생기지 않는다.
    S.append({
        "id": "oauth-loop-alive", "gate_id": "oauth-code-prompt", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#observations.B-6",
        "note": "'죽지 않는다' 를 증명하는 축. 죽음을 기다리는 회수 경로(seat_death_confirmed)가 "
                "영원히 완료되지 않는 좌석의 재현이다.",
        "steps": [{"kind": "prompt", "screen": oauth["loop_text"], "on_return": "loop",
                   "loop_text": oauth["loop_text"]}],
    })

    # ⑥ 폴더 신뢰 — 기본 포커스가 Yes 라 **Return 이 안전하다**(B-3).
    S.append({
        "id": "folder-trust", "gate_id": "folder-trust", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.folder-trust",
        "note": "2026-07-29 실사고의 범인은 이 창이 아니었다 — 이 창의 Return 은 안전하다.",
        "steps": [menu(trust),
                  {"kind": "ready", "screen": (trust.get("echo") or "") + "❯ ", "reply": "(echo) %s"}],
    })

    # ⑦ 신뢰 통과 직후 면책이 뜬다 — **연발 Return 킬체인**(2026-07-29 사고의 실체).
    S.append({
        "id": "trust-echo-double-return", "gate_id": "bypass-disclaimer", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.folder-trust,bypass-disclaimer",
        "note": "1발째는 안전하게 통과하고 **2발째가 죽인다**. 확인 에코가 화면에 남아 구 needle 에 "
                "재매칭되면 자동응답이 2발째를 쏜다 — 그것이 킬체인이다.",
        "steps": [menu(trust),
                  menu(byp, echo_prefix=trust.get("echo") or "")],
    })

    # ⑧ ★면책 대화상자 — 키 방향 교정의 본체.
    S.append({
        "id": "bypass-dialog", "gate_id": "bypass-disclaimer", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.bypass-disclaimer",
        "note": "Return 단독 = rc 1 사망 · Left+Return = **여전히 rc 1**(세로 리스트라 Left 무의미) · "
                "통과는 Down+Return 또는 숫자 2.",
        "forbid_keys": [["Return"], ["Left", "Return"], ["Right", "Return"]],
        "steps": [menu(byp)],
    })

    # ⑨~⑪ 코퍼스가 선언한(제품 소스에 기록된) 관문들 — **실측 라벨을 붙이지 않는다**.
    S.append({
        "id": "api-key-dialog", "gate_id": "custom-api-key", "provenance": "product-corpus",
        "key_oracle": "none", "source": CORPUS_REF + " · custom-api-key",
        "note": "기계 통과 액션 없음(human_only). 감지 전용 축 — 키 오라클을 만들지 않는다.",
        "steps": [{"kind": "prompt",
                   "screen": "Do you want to use this API key?\nAPI key: sk-ant-…(마스킹)\n"
                             "Enter to confirm · Esc to cancel\n",
                   "on_return": "loop",
                   "loop_text": "(재프롬프트) 자격증명은 사람만 넣을 수 있다\n"}],
    })
    S.append({
        "id": "platform-setup", "gate_id": "platform-setup", "provenance": "product-corpus",
        "key_oracle": "none", "source": CORPUS_REF + " · platform-setup",
        "note": "3rd-party 플랫폼 선택. 감지 전용.",
        "steps": [{"kind": "menu", "screen": "Select your platform:\n❯ 1. Amazon Bedrock\n"
                                             "  2. Google Vertex AI\n\nEnter to confirm · Esc to cancel\n",
                   "options": ["Amazon Bedrock", "Google Vertex AI"], "default_index": 1,
                   "accept_index": None, "reject_rc": None,
                   "reject_mode": "unmeasured", "echo": None}],
    })
    S.append({
        "id": "feature-announce", "gate_id": "feature-announce-fullscreen", "provenance": "product-corpus",
        "key_oracle": "corpus-declared", "source": CORPUS_REF + " · feature-announce-fullscreen",
        "note": "코퍼스는 기본 포커스 `1. Yes, try it` 을 **따르지 않고** `2. Not now` 를 고른다 — "
                "전체화면 렌더러 수락이 화면 판독 전제를 흔들기 때문이다. 여기서 Return 단독은 "
                "죽이지는 않지만 **관측 전제를 바꾸는** 선택이다(관측 가능한 회귀 표식을 남긴다).",
        "steps": [{"kind": "menu",
                   "screen": "Try the new fullscreen renderer?\n❯ 1. Yes, try it\n  2. Not now\n\n"
                             "Enter to confirm · Esc to cancel\n",
                   "options": ["Yes, try it", "Not now"], "default_index": 1,
                   "accept_index": 2, "reject_rc": None, "reject_mode": "nonfatal",
                   "reject_text": "\x1b[?1049h(대체 화면 진입 — 화면 판독 계약이 바뀌었다)\n",
                   "echo": None}],
    })

    # ⑫ 온보딩 연쇄 — 전 화면에 `❯` 가 있다(B-5).
    S.append({
        "id": "onboarding-chain", "gate_id": "theme", "provenance": "measured",
        "key_oracle": "measured", "source": EV_REF + "#screens.theme-select,login-select,oauth-code-prompt",
        "note": "관문 화면 전부가 `❯` 를 갖는다 = ready_marker 단독 판정은 **비변별자**다. "
                "여기서 ready 를 선언하면 64~118KB 디렉티브가 관문 화면에 쏟아진다.",
        "steps": [menu(theme, accept_index=theme["default_index"]),
                  menu(login, accept_index=login["default_index"]),
                  {"kind": "prompt", "screen": oauth["text"], "on_return": "loop",
                   "loop_text": oauth["loop_text"]}],
    })

    # ⑬ 시드는 다 됐는데 인증만 없는 상태 — 화면은 완벽히 '준비됨' 으로 보인다.
    S.append({
        "id": "seeded-unauthenticated", "gate_id": None, "provenance": "measured",
        "key_oracle": "none", "source": EV_REF + "#provenance(V-e E-2 실측: 심어도 Not logged in)",
        "note": "관문 화면이 하나도 없고 프롬프트도 정상이다. 그런데 **질의만 실패**한다 — "
                "화면·생존 어느 축으로도 잡히지 않는 허위 READY 의 마지막 형태.",
        "steps": [{"kind": "ready", "screen": "❯ ",
                   "reply": "Not logged in · Please run /login"}],
    })

    # ⑭ 미지 관문 — 어떤 키로도 나아가지 않고 죽지도 않는다.
    S.append({
        "id": "gated-alive-forever", "gate_id": None, "provenance": "synthetic", "key_oracle": "none",
        "source": "엔진 축(alive_but_gated)",
        "note": "코퍼스에 없는 **미지 관문**. 살아 있으므로 생존 판정은 충족되고, 화면 needle 은 "
                "하나도 안 맞는다 — deny 목록 단독 판정이 뚫리는 지점.",
        "steps": [{"kind": "stuck",
                   "screen": "A new thing needs your attention.\n(코퍼스에 없는 미지 문면)\n"}],
    })

    if k.get("left_right_move_focus") is not False:
        raise SystemExit("실측 캡처가 'Left/Right 는 포커스를 옮기지 않는다' 를 확인해 주지 않는다 — "
                         "키 오라클의 전제가 무너졌다(%s#key_facts)" % EV_REF)
    return S


# ── 키 파싱 ──────────────────────────────────────────────────────────────────
_ESC_MAP = {b"A": "Up", b"B": "Down", b"C": "Right", b"D": "Left"}


class KeyReader(object):
    """stdin 을 바이트로 읽어 키 어휘로 바꾼다. PTY·파이프 양쪽에서 같은 코드로 동작한다."""

    def __init__(self, fd=0):
        self.fd = fd
        self.q = queue.Queue()
        self.eof = False
        t = threading.Thread(target=self._pump)
        t.daemon = True
        t.start()

    def _pump(self):
        while True:
            try:
                b = os.read(self.fd, 1)
            except OSError:
                b = b""
            if not b:
                self.q.put(None)
                return
            self.q.put(b)

    def _raw(self, timeout):
        try:
            return self.q.get(timeout=timeout)
        except queue.Empty:
            return "timeout"

    def key(self, timeout):
        b = self._raw(timeout)
        if b == "timeout":
            return "timeout"
        if b is None:
            return "EOF"
        if b == b"\x1b":
            nxt = self._raw(0.08)
            if nxt in ("timeout", None) or nxt != b"[":
                return "Esc"
            third = self._raw(0.08)
            if third in ("timeout", None):
                return "Esc"
            return _ESC_MAP.get(third, "Unknown")
        if b in (b"\r", b"\n"):
            return "Return"
        if b == b"\x04":
            return "Ctrl-D"
        if b == b"\x03":
            return "Ctrl-C"
        try:
            ch = b.decode("utf-8", "replace")
        except Exception:
            return "Unknown"
        return ch


# ── 엔진 ────────────────────────────────────────────────────────────────────
def _emit(ev, **kw):
    parts = " ".join("%s=%s" % (k, v) for k, v in sorted(kw.items()))
    sys.stdout.write("[fake-agent] event=%s %s\n" % (ev, parts))
    sys.stdout.flush()


def run(scn, idle_timeout=5.0, max_steps=200):
    """시나리오를 실제로 돌린다. 반환 = 프로세스 종료코드.

    ★단계 전진은 **인덱스 이동**이다(재귀 아님) — 재귀로 감으면 긴 연쇄에서 스택이 늘고
      '몇 번째 단계에서 죽었는가' 라는 관측이 흐려진다."""
    mut = _mutation()
    _emit("start", scenario=scn["id"], provenance=scn["provenance"],
          oracle=scn["key_oracle"], mutate=(mut or "none"))
    steps = scn["steps"]
    i = 0
    focus = None
    typed = ""
    printed = -1
    acted = 0
    while True:
        st = steps[i]
        if printed != i:
            sys.stdout.write((st.get("echo_prefix") or "") + st["screen"])
            sys.stdout.flush()
            focus = st.get("default_index")
            printed = i
            _emit("screen", step=i, kind=st["kind"], focus=focus)
        k = KEYS.key(idle_timeout)
        if k == "timeout":
            _emit("idle-timeout", step=i)
            return RC_IDLE
        if k in ("EOF", "Ctrl-D", "Ctrl-C"):
            _emit("final", alive=1, step=i, step_kind=st["kind"], reason=k)
            return RC_ALIVE
        acted += 1
        if acted > max_steps:
            _emit("max-steps", step=i)
            return RC_IDLE

        kind = st["kind"]
        if kind == "stuck":
            _emit("ignored", step=i, key=k)
            continue

        if kind == "ready":
            if k == "Return":
                reply = st.get("reply") or ""
                sys.stdout.write(((reply % typed) if "%s" in reply else reply) + "\n\u276f ")
                sys.stdout.flush()
                _emit("submitted", step=i, text_len=len(typed))
                typed = ""
            elif len(k) == 1 and k.isprintable():
                typed += k
            continue

        if kind == "prompt":
            if k == "Return":
                if st.get("on_return") == "loop":
                    sys.stdout.write(st.get("loop_text") or "")
                    sys.stdout.flush()
                    _emit("loop", step=i)
                elif i + 1 < len(steps):
                    i += 1
                    _emit("advance", step=i)
                else:
                    _emit("final", alive=1, step=i, step_kind="prompt", reason="no-more-steps")
                    return RC_ALIVE
            elif len(k) == 1 and k.isprintable():
                typed += k
            continue

        # ── kind == "menu" ──────────────────────────────────────────────────
        n = len(st["options"])
        if k in ("Up", "Down"):
            focus = 1 if focus is None else focus
            focus = ((focus - 2) % n + 1) if k == "Up" else (focus % n + 1)
            _emit("focus", step=i, focus=focus)
            continue
        if k in ("Left", "Right"):
            # ★실측(B-2): 세로 번호 리스트라 좌우는 포커스를 옮기지 못한다. 구 설계의
            #   `Left+Return` 오라클이 성립하는 세계는 아래 변조본에서만 존재한다.
            if mut == "left-moves-focus":
                focus = (1 if focus is None else focus) % n + 1
                _emit("focus", step=i, focus=focus, mutated=mut)
            else:
                _emit("ignored", step=i, key=k)
            continue

        sel = None
        if k.isdigit():
            if mut == "digit-ignored":
                _emit("ignored", step=i, key=k, mutated=mut)
                continue
            d = int(k)
            if not (1 <= d <= n):
                _emit("ignored", step=i, key=k)
                continue
            sel = d
        elif k == "Return":
            sel = focus
            if mut == "return-accepts" and st.get("accept_index"):
                sel = st["accept_index"]
                _emit("mutated", step=i, kind="return-accepts")
        else:
            _emit("ignored", step=i, key=k)
            continue

        verdict, rc = _confirm(st, sel, i, mut)
        if verdict == "exit":
            return rc
        if i + 1 < len(steps):
            i += 1
            _emit("advance", step=i)
            continue
        _emit("final", alive=1, step=i, step_kind=verdict, reason="no-more-steps")
        return RC_ALIVE


def _confirm(st, sel, i, mut):
    """선택 확정. 반환 (`"exit"`, rc) 또는 (`"accepted"|"rejected-*"`, None) = 계속."""
    label = st["options"][sel - 1] if sel and 1 <= sel <= len(st["options"]) else "?"
    _emit("confirm", step=i, index=sel, label=str(label).replace(" ", "_"))
    if st.get("accept_index") is not None and sel == st["accept_index"]:
        if st.get("echo"):
            sys.stdout.write(st["echo"])
        if st.get("side_effect"):
            sys.stdout.write(st["side_effect"] + "\n")
            _emit("side-effect", step=i)
        sys.stdout.flush()
        _emit("accepted", step=i, index=sel)
        return ("accepted", None)

    mode = st.get("reject_mode") or ("exit" if st.get("reject_rc") else "unmeasured")
    if st.get("reject_text"):
        sys.stdout.write(st["reject_text"])
        sys.stdout.flush()
    if mode == "exit" and mut != "no-exit":
        _emit("rejected", step=i, index=sel, rc=st["reject_rc"])
        return ("exit", st["reject_rc"])
    if mode == "exit":                      # mut == "no-exit" — 사망을 감춘 세계
        _emit("rejected-hidden", step=i, index=sel, mutated=mut)
        return ("rejected-hidden", None)
    if mode == "nonfatal":
        _emit("rejected-nonfatal", step=i, index=sel)
        return ("rejected-nonfatal", None)
    # ★미측정 경로 — 스텁은 재지 않은 것을 단언하지 않는다. 큰 소리로 그 사실만 남긴다.
    _emit("rejected-unmeasured", step=i, index=sel)
    return ("rejected-unmeasured", None)



# ── 자기검증 ────────────────────────────────────────────────────────────────
def self_test():
    """선언 무결성 — 손으로 쓴 값이 실측 라벨을 훔쳐 쓰지 못하게 한다."""
    bad = []
    ev = load_evidence()
    S = build_scenarios(ev)
    ids = [s["id"] for s in S]
    if len(S) != 14:
        bad.append("시나리오 수 %d (계약 14)" % len(S))
    if len(set(ids)) != len(ids):
        bad.append("중복 id: %s" % ids)
    for s in S:
        if s["provenance"] not in ("measured", "product-corpus", "synthetic"):
            bad.append("%s: 알 수 없는 출처 등급 %r" % (s["id"], s["provenance"]))
        if s["key_oracle"] not in ("measured", "corpus-declared", "none"):
            bad.append("%s: 알 수 없는 키 오라클 등급 %r" % (s["id"], s["key_oracle"]))
        # ★핵심 규율: 실측하지 않은 화면에 '실측 키 오라클' 을 붙일 수 없다.
        if s["key_oracle"] == "measured" and s["provenance"] != "measured":
            bad.append("%s: 실측이 아닌데 키 오라클을 measured 로 선언했다" % s["id"])
        if s["key_oracle"] == "measured" and EV_REF not in s["source"]:
            bad.append("%s: measured 인데 실측 캡처를 출처로 대지 않았다" % s["id"])
        if not s.get("steps"):
            bad.append("%s: 단계 없음" % s["id"])
        for st in s["steps"]:
            if st["kind"] == "menu" and st.get("accept_index") is not None:
                if s["key_oracle"] == "none":
                    bad.append("%s: 키 오라클 없음인데 기계 통과 경로를 선언했다(틀린 키 박제 위험)"
                               % s["id"])
                if not (1 <= st["accept_index"] <= len(st["options"])):
                    bad.append("%s: accept_index 범위 밖" % s["id"])
    # 실측 화면은 캡처 원문과 **바이트 동일**해야 한다(손으로 고쳐 쓰면 적발).
    for gid in ("theme-select", "login-select", "folder-trust", "bypass-disclaimer"):
        txt = _ev_screen(ev, gid)["text"]
        if not any(txt in st.get("screen", "") or st.get("screen", "") == txt
                   for s in S for st in s["steps"]):
            bad.append("실측 화면이 어느 시나리오에도 실리지 않았다: %s" % gid)
    # 면책 창의 실측 상수 3종(기본 포커스·통과 인덱스·종료코드)이 그대로인지 못박는다.
    byp = _ev_screen(ev, "bypass-disclaimer")
    if (byp["default_index"], byp.get("accept_index"), byp.get("exit_rc")) != (1, 2, 1):
        bad.append("면책 창 실측 상수가 바뀌었다: %r — 키 방향 교정의 전제다" % byp)
    return bad, len(S)


IDLE = 5.0
KEYS = None


def main(argv=None):
    global KEYS, IDLE
    for _s in (sys.stdout, sys.stderr):
        try:
            _s.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    ap = argparse.ArgumentParser(description="첫기동 관문 결정론 TUI 스텁")
    ap.add_argument("--scenario", default="")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--idle-timeout", type=float, default=5.0)
    a = ap.parse_args(argv)

    if a.self_test:
        bad, n = self_test()
        if a.json:
            print(json.dumps({"verdict": "GREEN" if not bad else "RED",
                              "scenarios": n, "violations": bad}, ensure_ascii=False, indent=1))
        else:
            print("self-test %s — 시나리오 %d · 위반 %d%s"
                  % ("GREEN" if not bad else "RED", n, len(bad),
                     "" if not bad else ": " + " / ".join(bad)))
        return 0 if not bad else 1

    S = build_scenarios()
    if a.list:
        if a.json:
            rows = []
            for s in S:
                r = {k: v for k, v in s.items() if k != "steps"}
                r["steps"] = len(s["steps"])
                r["step_kinds"] = [x["kind"] for x in s["steps"]]
                r["accept_paths"] = [x.get("accept_index") for x in s["steps"]
                                     if x["kind"] == "menu"]
                r["exit_rcs"] = [x.get("reject_rc") for x in s["steps"] if x["kind"] == "menu"]
                rows.append(r)
            print(json.dumps({"count": len(S), "mutations": MUTATIONS,
                              "evidence": EV_REF, "scenarios": rows},
                             ensure_ascii=False, indent=1))
        else:
            for s in S:
                print("%-26s %-14s oracle=%-15s %s"
                      % (s["id"], s["provenance"], s["key_oracle"], s["note"].splitlines()[0][:60]))
        return 0

    by = {s["id"]: s for s in S}
    if a.scenario not in by:
        print("알 수 없는 시나리오: %r (가능: %s)" % (a.scenario, ", ".join(sorted(by))),
              file=sys.stderr)
        return 2
    IDLE = a.idle_timeout
    KEYS = KeyReader(0)
    return run(by[a.scenario], idle_timeout=a.idle_timeout)


if __name__ == "__main__":
    sys.exit(main())
