#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_mission — **임무 게이트**의 단일 소유자 (2026-08-01 윈도우 실사고 T1 근본수정).

사고(실측 3장 + 노드 자체 진단): 오너가 부트스트랩 선언만 하고 **아무 임무도 주지 않았는데**
5개 노드가 무한 작업에 들어가 7일 사용량 72%를 태웠다. master 자체 진단이 원인을 정확히 짚었다 —

    "근본 원인은 프로젝트 미지정 상태에서 §0의 next-action 자율 착수 규칙이
     **이전 세션의 잔무 큐를 집어 온 것**."

구 계약(MASTER_DIRECTIVE §0-⑥ · §14 축1/축3 · role-bootstrap 훅 note)은 부팅 직후
`javis_orchestra.py next-action` 이 **exit 0(항목 있음)** 이기만 하면 자율 착수하라고 지시했다.
그런데 그 큐(`pack/round/SESSION_STATE.md`)는 **master 자신이 쓴 파일**이다 — 즉 산출자가
자기 산출물로 자기 착수 권한을 발급하는 **자기인가(self-authorization) 루프**였다. 오너가
아무 말도 하지 않은 세션에서도 큐가 비어 있지 않으면 무조건 달렸다.

## 이 모듈이 세우는 계약 (한 줄)
**자율 착수 권한은 오너 채널에서만 나온다. master가 쓰는 파일은 착수 권한의 근거가 아니다.**

  · 큐(SESSION_STATE 다음 액션) = "무엇을" 의 출처       ← master가 쓴다(권한 아님)
  · 임무 대장(이 파일)          = "지금 달려도 되는가" 의 출처 ← 오너 프롬프트에서만 파생

'이전 세션 잔무'는 **보고 대상**이지 자동 착수 대상이 아니다.

## 결정론 판정 (자연어 추론 금지 — 이 도구의 exit code 가 사실이다)
`status` exit 0(임무 있음) 은 아래 중 **하나라도** 성립할 때만이다:
  ① 환경변수 `CYS_MISSION` 이 비어 있지 않다 (기동 시점 명시 지정 — `cys launch-agent` 등)
  ② 이 레인의 임무 대장에 `mission` 이 non-null 이고 `surface` 가 현재 pane 과 일치한다
그 밖 전부 exit 1(임무 없음 → **보고하고 멈춘다**). 판독 불가(손상 JSON 등)는 exit 2 이고,
**소비자는 2를 '없음'과 같게 취급한다**(fail-closed — 판정 불가가 자율주행을 열지 않는다).

## 임무 대장은 누가 쓰는가 (오너 채널만)
  · `record`  — **UserPromptSubmit 훅**(hooks/role-bootstrap.sh)이 매 프롬프트마다 1회 호출한다.
                훅은 오너가 실제로 친 문장을 보는 유일한 결정론 관측점이다.
                - **기계 유래 프롬프트는 대장을 건드리지 않는다**(`machine_origin` — 아래 참조).
                - 마스터 **선언** 프롬프트(javis_detect fire) = 임무 대장 **재개장(reset)** 후
                  잔여문 평가. → "너는 마스터다" 단독이면 `mission=null` 이 기록된다(=사고 재현 차단).
                - 비선언 프롬프트 = **상향만**(있던 임무를 지우지 않는다 — "응"·"고마워"가
                  진행 중 임무를 취소해 버리는 역효과 차단).

## 기계 유래 배제 — 자기인가 우회로 봉합 (2026-08-01 T1 적대검증 FAIL)
훅은 **오너가 친 문장**과 **기계가 master stdin 에 밀어 넣은 문장**을 같은 자리에서 본다.
그래서 대장을 "훅이 본 모든 프롬프트"로 열면 자기인가 루프가 **채널만 바꿔** 되살아난다.
검증자가 실증한 결정적 우회로 2종:
  ① 자기 예약 wake — `cys schedule add --text "[wakeup] 다음 액션 착수" --to master`
     (문안은 `CLAUDE.md.template:44` 에 그대로 실존). 시간이 지나면 master 자신이 예약한
     문장이 stdin 으로 돌아와 임무로 기록 → exit 3 이 0 으로 뒤집힌다.
  ② 워커 완료 push — `cys send --to master "[worker-1 완료] … 다음 지시 주세요"`
     (프로젝트 CLAUDE.md §7 정상 규약). 첫 위임 사이클 직후 게이트가 항구 개방된다.
**규칙: 임무 대장 기록은 오너 유래 프롬프트만.** 판별 근거는 발명하지 않고 **실존 규약**에서 취한다 —
push 문안은 `[라벨] 본문` 형태이며 그 규약의 실물 생산자가 리포에 전부 있다:
  · `javis_wakeup.py:348` `[wakeup <W-id>] task=…`  · `:388` `[wakeup digest <N>건] target=…`
  · `hooks/role-bootstrap.sh` `_notify_bg` → `cys send --queued --to master "[<제목>] <본문>"`
  · `CLAUDE.md.template:44` `[wakeup] 다음 액션 착수`  · 프로젝트 CLAUDE.md §7 `"[라벨]…"`
오탐 비대칭은 그대로다 — **거짓 양성(기계→임무)이 치명**이므로 라벨이 붙은 프롬프트는
**본문에 오너 문장이 섞여 있어도 통째로** 임무에서 제외한다(부분 추출 금지). 오너가 push 안에
새 임무를 실어 보냈다면 그 임무는 **오너 채널로 다시 들어와야** 대장을 연다(오너 직접 입력, 또는
오너 발화를 들은 master 의 `set`). 판별 자체가 불가능한 경우(모듈 부재·타임아웃)도 임무 아님이다.
**잔여 갭(고지)**: 라벨 없이 들어온 기계 push 는 오너 입력과 문자열이 동일해 in-band 로는
구분 불가다 — 그 경로는 `cys send` 원장이 생기기 전까지 닫히지 않는다(§ 아래 MACHINE_LABEL 주석).
  · `set`     — 오너가 구두로 임무를 준 직후 master가 기록하는 명시 채널.
                ★한계 고지: 이 채널은 LLM 이 호출하므로 **위조 불가능하지 않다**. 배포 안전의
                무게중심은 `record`(훅 관측)와 **부팅 기본값 = 임무 없음** 쪽에 있다.
  · `clear`   — 작업 단위 종료·오너 정지 지시 시 대장 폐기.

## 오탐 비대칭 (설계 의도 — 튜닝 시 반드시 보존)
  · 거짓 양성(잡음 → 임무): **자율주행이 잔무 큐로 달린다** = 이번 사고 그 자체. 치명.
  · 거짓 음성(임무 → 잡음): master가 "이어서 하시겠습니까?"를 한 번 더 묻는다. 경미.
따라서 애매하면 **임무 없음**으로 접는다(MISSION_MIN_CHARS·질의절 배제·ack 어휘 배제).

## 사용
    javis_mission.py record            # stdin=UserPromptSubmit hook JSON (훅 전용)
    javis_mission.py status [--json]   # 0=임무 있음 / 1=없음 / 2=판독 불가(=없음 취급)
    javis_mission.py set "<임무>" [--source owner-confirm]
    javis_mission.py clear [--reason "<사유>"]
    javis_mission.py path              # 이 레인 임무 대장 경로 1줄
    javis_mission.py --self-test       # 밀폐 corpus 배터리(preflight/CI 관례)

의존성: 파이썬 표준 라이브러리 + 형제 모듈 `javis_detect`(어휘 단일 출처)·`javis_bootstrap`
(레인 경로 규약 단일 소유자 — `lane_state_path("mission")`). 둘 다 **부재 시 폴백하지 않고**
fail-closed 로 접는다(임무 없음). 침묵 폴백 금지.
"""
import json
import os
import re
import sys
import time

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장.
#   선례·근거는 javis_orchestra.py:71-81 과 동일(append — precedence 강등 금지).
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★로케일 비의존 I/O(선례 javis_detect.py:44-49): LC_ALL=C·Windows cp949 파이프에서 한글 출력이
#   UnicodeEncodeError 로 죽으면 '판정 불가'가 아니라 **훅 전체가 깨진다**.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

SCHEMA_VERSION = 1

# ── 수치 스펙(단일 진실원천 — 문서·주석·테스트가 이 상수를 가리킨다) ──────────────
# 임무로 인정하는 잔여문 최소 길이(**문자** · 공백 접은 뒤). 3 = "진행해"·"확인해줘" 통과,
# "응"·"ok"·"ㅇㅇ" 불통과. 값을 낮추면 거짓 양성(=사고 재현) 쪽으로 움직인다 — 위 비대칭 참조.
MISSION_MIN_CHARS = 3
# 대장에 보존하는 임무 문자열 상한(컨텍스트·디스크 보호 · 판정에는 영향 없음).
MISSION_MAX_CHARS = 400

# 단독 절이면 임무가 아닌 응답·인사 어휘(**절 전체가 이것일 때만** 배제 — 부분일치 금지:
# "네이버 API 연동해줘"의 '네'가 임무를 삼키면 안 된다).
ACK_CLAUSES = frozenset([
    "네", "넵", "예", "응", "어", "ㅇㅇ", "ㅇㅋ", "ok", "okay", "오케이", "그래", "좋아",
    "hi", "hello", "안녕", "안녕하세요", "고마워", "고맙다", "감사", "감사합니다", "수고",
    "ㅋㅋ", "ㅎㅎ", "yes", "y", "n", "no", "아니", "잠깐", "대기",
])

_WS = re.compile(r"\s+")

# ── 기계 유래 판별 상수(2026-08-01 T1 봉합) ────────────────────────────────
# 라벨 스캔 창(문자). 실물 라벨 최장은 `[wakeup digest 12건] `(≈22자)·`[부트스트랩 판정 불가
# (python 부재)]`(≈24자) 수준이라 80 은 넉넉한 상한이다. 창을 두는 이유는 오너가 문장 중간에
# 대괄호를 쓰는 긴 텍스트를 라벨로 오인하지 않기 위함이다(라벨은 **선두 짧은 토큰**이다).
MACHINE_LABEL_MAX = 80
# 선두 `[…]` = push 규약 라벨. 개행·중첩 대괄호는 라벨이 아니다(본문 인용문 오탐 차단).
_MACHINE_LABEL = re.compile(r"^\s*\[[^\[\]\n]{1,%d}\]" % MACHINE_LABEL_MAX)


def machine_origin(prompt):
    """(bool, 사유) — 이 프롬프트가 **기계 채널**(wake 예약·노드 push·훅 알림)로 왔는가.

    ★in-band 판별만 가능하다는 한계를 정직하게 적는다: `cys send` 는 pane 에 **문자를 타이핑**할
      뿐이고 데몬의 `last_injected` 는 메모리 상 `Instant` 라(state.rs:145) 훅이 조회할 원장이
      없다. 따라서 근거는 **문안 규약**뿐이며, 라벨을 붙이지 않은 기계 push 는 오너 입력과
      구별 불가다(잔여 갭 — 닫으려면 `cys send` 가 배달 원장을 남겨야 한다).
    ★그래서 판별은 **한 방향으로만 공격적**이다: 라벨이 있으면 무조건 기계로 접는다.
      거짓 양성(기계→임무)은 자율주행 폭주고, 거짓 음성(오너→임무 아님)은 한 번 더 묻는 것이다.
    """
    m = _MACHINE_LABEL.match(prompt or "")
    if m:
        return True, "push 규약 라벨 선두 매치 %r — 기계 채널(wake/노드 push/훅 알림)" % m.group(0)
    return False, ""


def _fail_closed(reason):
    """의존 모듈 소실·판독 불가 — 조용히 접지 않고 stderr 1줄(선례 javis_detect CS-8⑤)."""
    sys.stderr.write("[mission] 판정 불가(fail-closed · 임무 없음으로 취급): %s\n" % reason)


def _detect_mod():
    try:
        import javis_detect
        return javis_detect
    except Exception as e:                      # 팩 스큐·배포 결손
        _fail_closed("javis_detect 미적재(%s)" % e)
        return None


def ledger_path():
    """이 레인의 임무 대장 경로. 경로 규약의 **단일 소유자는 javis_bootstrap.lane_state_path** 다
    (G15 · CS-7② — 사본 금지). 그 모듈이 없으면 경로를 **짐작하지 않고** None 을 돌려준다."""
    try:
        import javis_bootstrap
        return javis_bootstrap.lane_state_path("mission")
    except Exception as e:
        _fail_closed("javis_bootstrap.lane_state_path 미소비(%s)" % e)
        return None


def _surface():
    return os.environ.get("CYS_SURFACE_ID", "") or ""


# ══════════════════════════════════════════════════════════════════════════════
# 순수 함수 — 프롬프트 → 임무 (self-test 박제 대상)
# ══════════════════════════════════════════════════════════════════════════════
def split_clauses(text, detect):
    """절 경계로 분해. 경계 어휘는 javis_detect.CLAUSE_BOUNDARY 를 **그대로** 쓴다(사본 금지).

    ★경계 문자는 **앞 절에 포함**한다 — javis_detect._clause_bounds 와 동일 규약이다
      ("…무슨 뜻?" 의 '?' 가 그 절의 억제 마커로 평가돼야 한다). 이걸 버리면 물음표가 사라져
      `QUESTION` 의 `\\?` 가 영영 매치되지 않고 "오늘 뭐부터 할까?"가 **임무로 오탐**된다
      (self-test 로 박제 — 초안이 실제로 이 결함이었다).
    """
    out, buf = [], []
    for ch in text or "":
        buf.append(ch)
        if ch in detect.CLAUSE_BOUNDARY:
            out.append("".join(buf))
            buf = []
    out.append("".join(buf))
    return [c for c in (s.strip() for s in out) if c]


def extract_mission(prompt, detect):
    """프롬프트 → (임무 문자열 or None, 사유). **순수 함수**(부작용 0 · 로케일 비의존).

    절 단위로 걸러낸다 — 문자 오프셋을 다루지 않으므로 javis_detect 의 200자 감지창에
    갇히지 않는다(임무는 선언 뒤 어디에나 올 수 있다).
      ① 선언절(DECL_KO/DECL_EN 매치) 제외      — "너는 마스터다" 자체는 임무가 아니다
      ② 질의·인용절(QUESTION 매치) 제외        — "오늘 뭐부터 할까?" 는 **보고 요구**지 임무가 아니다
      ③ ack 단독절 제외                        — "응"·"ok"
      ④ 남은 문자수 < MISSION_MIN_CHARS → 임무 없음
    """
    clauses = split_clauses(prompt, detect)
    kept, dropped = [], []
    for c in clauses:
        if detect.DECL_KO.search(c) or detect.DECL_EN.search(c):
            dropped.append(("선언절", c))
            continue
        if detect.QUESTION.search(c):
            dropped.append(("질의·인용절", c))
            continue
        if _WS.sub("", c).lower() in ACK_CLAUSES:
            dropped.append(("ack절", c))
            continue
        kept.append(c)
    body = _WS.sub(" ", " ".join(kept)).strip()
    if len(body) < MISSION_MIN_CHARS:
        return None, ("잔여문 %d자 < 최소 %d자(제외: %s)"
                      % (len(body), MISSION_MIN_CHARS,
                         ", ".join(d[0] for d in dropped) or "없음"))
    return body[:MISSION_MAX_CHARS], "잔여문 %d자 — 오너 임무로 인정" % len(body)


# ══════════════════════════════════════════════════════════════════════════════
# 대장 I/O
# ══════════════════════════════════════════════════════════════════════════════
def read_ledger():
    """(record dict or None, 판독불가 사유 or None)."""
    p = ledger_path()
    if not p:
        return None, "레인 경로 판독 불가"
    if not os.path.isfile(p):
        return None, None                       # 부재 = 정상(임무 없음) — 오류 아님
    try:
        with open(p, "rb") as f:
            rec = json.loads(f.read().decode("utf-8", "replace"))
    except Exception as e:
        return None, "대장 판독 실패(%s): %s" % (e, p)
    if not isinstance(rec, dict):
        return None, "대장 형식 오류(dict 아님): %s" % p
    return rec, None


def write_ledger(mission, source, reason, prompt=None):
    p = ledger_path()
    if not p:
        return None
    rec = {
        "schema": SCHEMA_VERSION,
        "mission": mission,
        "source": source,
        "reason": reason,
        "surface": _surface(),
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    if prompt is not None:
        rec["prompt_chars"] = len(prompt)
    try:
        d = os.path.dirname(p)
        if d and not os.path.isdir(d):
            os.makedirs(d)
        tmp = p + ".tmp"
        with open(tmp, "wb") as f:              # 원자 교체(부분쓰기 판독 방지)
            f.write(json.dumps(rec, ensure_ascii=False).encode("utf-8"))
        os.replace(tmp, p)
    except Exception as e:
        _fail_closed("대장 쓰기 실패(%s): %s" % (e, p))
        return None
    return rec


# ══════════════════════════════════════════════════════════════════════════════
# 게이트 판정 — 이 함수가 '자율 착수 가능한가'의 유일한 정의처
# ══════════════════════════════════════════════════════════════════════════════
EXIT_HAVE = 0        # 임무 있음 — 자율 착수 가
EXIT_NONE = 1        # 임무 없음 — 보고하고 멈춘다
EXIT_UNREADABLE = 2  # 판독 불가 — 소비자는 '없음'과 같게 취급(fail-closed)


def gate():
    """반환 (exit_code, verdict dict). 자연어 추론 금지 — 소비자는 이 exit 만 본다."""
    env = (os.environ.get("CYS_MISSION", "") or "").strip()
    if env:
        return EXIT_HAVE, {"have_mission": True, "source": "env:CYS_MISSION",
                           "mission": env[:MISSION_MAX_CHARS],
                           "reason": "기동 시점 환경변수 명시 지정"}
    rec, err = read_ledger()
    if err:
        return EXIT_UNREADABLE, {"have_mission": False, "source": "ledger",
                                 "mission": None, "reason": err}
    if not rec:
        return EXIT_NONE, {"have_mission": False, "source": "ledger",
                           "mission": None,
                           "reason": "임무 대장 없음 — 이 세션에 오너 임무 지정이 없다"}
    mission = rec.get("mission")
    if not mission:
        return EXIT_NONE, {"have_mission": False, "source": rec.get("source"),
                           "mission": None,
                           "reason": "부팅 선언만 관측됨(임무 미지정): %s"
                                     % (rec.get("reason") or "-")}
    # ★pane 일치 요구: 다른 surface(다른 오너 세션)의 임무로 이 pane 이 달리지 않는다.
    if rec.get("surface", "") != _surface():
        return EXIT_NONE, {"have_mission": False, "source": rec.get("source"),
                           "mission": None,
                           "reason": "임무 대장의 surface(%r)가 이 pane(%r)과 다르다 — 남의 임무"
                                     % (rec.get("surface", ""), _surface())}
    return EXIT_HAVE, {"have_mission": True, "source": rec.get("source"),
                       "mission": mission, "reason": rec.get("reason") or "오너 지정 임무"}


# ══════════════════════════════════════════════════════════════════════════════
# CLI
# ══════════════════════════════════════════════════════════════════════════════
def cmd_record(argv):
    """stdin(UserPromptSubmit hook JSON) → 대장 갱신. 훅 전용(1왕복).

    ★반환값 = **갱신 후 `gate()` 의 판정**이다(자기 판단이 아니라 정의처의 판정). 훅이 이 exit
      하나로 노드 spawn 여부를 가르므로(hooks/role-bootstrap.sh — D4-a 순서 결함 수리), 여기서
      독자 규칙으로 답하면 `status` 와 갈릴 수 있다. 판정처는 언제나 `gate()` 하나다.
    """
    detect = _detect_mod()
    if detect is None:
        return EXIT_UNREADABLE
    try:
        raw = sys.stdin.buffer.read() if hasattr(sys.stdin, "buffer") else sys.stdin.read()
        payload = raw.decode("utf-8", "replace") if isinstance(raw, bytes) else raw
        obj = json.loads(payload)
        prompt = obj.get("prompt", "") if isinstance(obj, dict) else ""
    except Exception as e:
        _fail_closed("hook JSON 파싱 실패(%s) — 대장 무변경" % e)
        return EXIT_UNREADABLE
    if not isinstance(prompt, str) or not prompt.strip():
        return gate()[0]
    # ── ★기계 유래 배제(T1 적대검증 FAIL 봉합) — 다른 무엇보다 **먼저** ────────────────
    # 자기 예약 wake(`[wakeup] …`)·노드 완료 push(`[worker-1 완료] …`)·훅 알림은 오너 채널이
    # 아니다. 대장을 **읽지도 쓰지도 않고** 그대로 둔다:
    #   · 쓰지 않는 이유 — 기계가 자기 착수 권한을 발급하는 것이 이번 사고의 본체다.
    #   · 지우지도 않는 이유 — 진행 중 오너 임무를 워커 push 가 취소해 버리면 반대 방향 사고다.
    # 선언 감지보다 앞이라 **push 본문에 섞인 "너는 마스터다"도 대장을 재개장하지 못한다**.
    is_machine, why = machine_origin(prompt)
    if is_machine:
        sys.stderr.write("[mission] 기계 유래 프롬프트 — 대장 무변경(임무 아님): %s\n" % why)
        return gate()[0]
    v = detect.detect(prompt)
    mission, reason = extract_mission(prompt, detect)
    if v.get("fire"):
        # 선언 = 세션 재개장. 잔여문이 없으면 **명시적으로 mission=null 을 박아** 직전 세션의
        # 임무가 새 부팅으로 새어 들어오지 않게 한다(사고의 '이전 세션 잔무' 경로 차단).
        write_ledger(mission, "declaration_residual", reason, prompt)
        return gate()[0]
    # 비선언 프롬프트 = **상향만**: 있던 임무를 지우지 않는다.
    if mission:
        write_ledger(mission, "prompt", reason, prompt)
    return gate()[0]


def cmd_status(argv):
    rc, v = gate()
    if "--json" in argv:
        print(json.dumps(v, ensure_ascii=False))
    else:
        if rc == EXIT_HAVE:
            print("[mission] 임무 있음(%s): %s" % (v.get("source"), v.get("mission")))
        else:
            print("[mission] 임무 없음 — 자율 착수 금지. 사유: %s" % v.get("reason"),
                  file=sys.stderr)
            print("  오너가 이 세션에 임무를 지정하면 해제된다(훅이 프롬프트에서 자동 기록). "
                  "구두 지시는 `javis_mission.py set \"<임무>\"` 로 기록한다.", file=sys.stderr)
    return rc


def cmd_set(argv):
    text = " ".join(a for a in argv if not a.startswith("--")).strip()
    if not text:
        sys.stderr.write("usage: javis_mission.py set \"<임무>\" [--source owner-confirm]\n")
        return 64
    source = "owner_explicit"
    for i, a in enumerate(argv):
        if a == "--source" and i + 1 < len(argv):
            source = argv[i + 1]
            text = " ".join(x for j, x in enumerate(argv)
                            if j not in (i, i + 1) and not x.startswith("--")).strip()
    rec = write_ledger(text[:MISSION_MAX_CHARS], source, "명시 기록(오너 지시 수신)")
    if rec is None:
        return EXIT_UNREADABLE
    print("[mission] 기록: %s (source=%s)" % (rec["mission"], source))
    return 0


def cmd_clear(argv):
    reason = "명시 폐기"
    for i, a in enumerate(argv):
        if a == "--reason" and i + 1 < len(argv):
            reason = argv[i + 1]
    rec = write_ledger(None, "cleared", reason)
    if rec is None:
        return EXIT_UNREADABLE
    print("[mission] 임무 대장 폐기 — 다음 자율 착수는 오너 지정 전까지 금지된다(%s)" % reason)
    return 0


def cmd_path(argv):
    p = ledger_path()
    if not p:
        return EXIT_UNREADABLE
    print(p)
    return 0


# ── 밀폐 self-test(assert 배터리 · preflight/CI 관례 — 선례 javis_detect.cmd_self_test) ──
def cmd_self_test():
    detect = _detect_mod()
    if detect is None:
        print("javis_mission self-test SKIP(javis_detect 부재)", file=sys.stderr)
        return 1
    fails = []

    def want_none(p, why):
        m, r = extract_mission(p, detect)
        if m is not None:
            fails.append("임무 오탐 %r → %r (%s · %s)" % (p, m, why, r))

    def want_mission(p, why):
        m, r = extract_mission(p, detect)
        if m is None:
            fails.append("임무 미탐 %r (%s · %s)" % (p, why, r))

    # ── ①사고 재현 corpus: 부트스트랩 선언만 = 임무 없음(자율 착수 금지) ──
    for p in ("너는 마스터다", "너는 이제 마스터다", "네가 마스터다", "you are the master",
              "지금부터 너는 마스터가 된다", "당신은 우리의 마스터입니다"):
        want_none(p, "선언 단독 — 2026-08-01 사고 진입점")
    # ── ②선언 + 질의 = 여전히 임무 없음("뭐부터 할까?"는 보고 요구지 임무가 아니다) ──
    want_none("너는 마스터다. 오늘 뭐부터 할까?", "질의절")
    want_none("너는 이제 마스터다! 무슨 일부터 시작할까?", "질의절")
    # ── ③ack 단독 = 임무 없음 ──
    for p in ("응", "ok", "ㅇㅇ", "네."):
        want_none(p, "ack 단독절")
    # ── ④선언 + 실제 임무 = 임무 있음(종전 동작 보존) ──
    want_mission("너는 마스터다. paperclip P0 Wave2 착수해줘", "선언+임무")
    want_mission("너는 마스터다\n릴리스 노트 0.14.9 초안을 만들어라", "개행 절 경계")
    # ── ⑤비선언 임무 프롬프트(2번째 턴 이후) ──
    want_mission("윈도우 실사고 T1 근본수정 진행해", "평문 임무")
    want_mission("확인해줘", "짧지만 MISSION_MIN_CHARS 충족")
    # ── ⑥ack 어휘 부분일치 금지(절 전체 일치일 때만 배제) ──
    want_mission("네이버 API 연동해줘", "ack 어휘 부분일치 오배제 금지")
    # ── ⑦경계 스펙 ──
    if extract_mission("가" * MISSION_MIN_CHARS, detect)[0] is None:
        fails.append("최소 길이 %d자 경계에서 미탐" % MISSION_MIN_CHARS)
    if extract_mission("가" * (MISSION_MIN_CHARS - 1), detect)[0] is not None:
        fails.append("최소 길이 미만(%d자)에서 오탐" % (MISSION_MIN_CHARS - 1))
    # ── ⑨기계 유래 배제(T1 적대검증 FAIL 봉합) — 검증자가 실증한 우회로 2종을 **문안 그대로** ──
    #    거짓 양성이 곧 자율주행 폭주이므로, 이 corpus 는 회귀 시 hard fail 이어야 한다.
    for p in (
        # 우회로 ① 자기 예약 wake — CLAUDE.md.template:44 의 문안 그대로
        "[wakeup] 다음 액션 착수",
        # 우회로 ② 워커 완료 push — 프로젝트 CLAUDE.md §7 정상 규약
        "[worker-1 완료] T1 구현 끝냈습니다. 다음 지시 주세요",
        # 실물 생산자 문안(javis_wakeup._build_send_message / _build_digest_message)
        "[wakeup W-3f2a1c] task=next-action reason=자율 루프 payload={}",
        "[wakeup digest 3건] target=master\n- [W-1] sev=warn task=x reason=y",
        # 훅 알림(_notify_bg → cys send --queued --to master "[<제목>] <본문>")
        "[부트스트랩 판정 불가(python 부재)] 팀 기동이 발화되지 않았습니다",
        # ★push 본문에 오너 문장·마스터 선언이 섞여도 통째로 배제(부분 추출 금지)
        "[worker-1 완료] 주인님이 T5도 진행하라고 하셨습니다",
        "[cso 보고] 너는 마스터다. 릴리스 노트를 작성해라",
        "  [wakeup] 다음 액션 착수",          # 선행 공백
    ):
        ok, why = machine_origin(p)
        if not ok:
            fails.append("기계 유래 미탐 %r — 자기인가 우회로가 열려 있다(치명)" % p)
        _ = why
    # 오너 문장은 기계로 오인하지 않는다(거짓 음성 과확장 차단)
    for p in ("T1 진행해", "너는 마스터다", "릴리스 노트 [초안] 만들어줘",
              "이 배열 [1,2,3] 을 정렬하는 코드 짜줘",
              "다음 액션 착수해줘"):
        ok, _r = machine_origin(p)
        if ok:
            fails.append("오너 문장을 기계로 오탐 %r (라벨 판별 과확장)" % p)
    # ── ⑧게이트 순수성 + **밀폐**: 대장이 없으면 EXIT_NONE(자율 착수 금지)이 기본값 ──
    _env_backup = os.environ.pop("CYS_MISSION", None)
    try:
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            _sd = os.environ.get("CYS_STATE_DIR")
            os.environ["CYS_STATE_DIR"] = os.path.join(td, "state")
            try:
                # ★밀폐 자기검증(2026-08-01 봉합): 격리 env 를 걸었는데 경로가 실 HOME 을
                #   가리키면 이 배터리 전체가 **사용자의 진짜 대장을 읽는다** — 임무를 정상
                #   수신한 세션에서 self-test 가 FAIL 하고 그대로 preflight C77 FAIL 이 됐다.
                #   경로 계약이 다시 갈리면 여기서 즉시 잡는다.
                _lp = ledger_path()
                if not _lp or not os.path.abspath(_lp).startswith(os.path.abspath(td)):
                    fails.append("밀폐 붕괴: CYS_STATE_DIR 격리가 무시됐다(path=%r) — "
                                 "javis_bootstrap.state_dir() 경로 계약 확인" % _lp)
                # 대장 부재 → 없음
                if _lp and gate()[0] != EXIT_NONE:
                    fails.append("대장 부재인데 게이트가 열렸다(기본값이 fail-open — 치명)")
                # 대장에 임무가 있어도 **surface 가 다르면** 남의 임무다
                write_ledger("남의 임무", "prompt", "test", None)
                _sf = os.environ.get("CYS_SURFACE_ID")
                os.environ["CYS_SURFACE_ID"] = (_sf or "") + "-other"
                try:
                    if gate()[0] != EXIT_NONE:
                        fails.append("다른 surface 의 임무로 게이트가 열렸다")
                finally:
                    if _sf is None:
                        os.environ.pop("CYS_SURFACE_ID", None)
                    else:
                        os.environ["CYS_SURFACE_ID"] = _sf
                if gate()[0] != EXIT_HAVE:
                    fails.append("같은 surface 의 기록된 임무를 게이트가 인정하지 않는다")
                # env 지정 → 있음
                os.environ["CYS_MISSION"] = "명시 임무"
                if gate()[0] != EXIT_HAVE:
                    fails.append("CYS_MISSION 지정이 게이트를 열지 못한다")
                os.environ.pop("CYS_MISSION", None)
            finally:
                if _sd is None:
                    os.environ.pop("CYS_STATE_DIR", None)
                else:
                    os.environ["CYS_STATE_DIR"] = _sd
    finally:
        if _env_backup is not None:
            os.environ["CYS_MISSION"] = _env_backup
    if fails:
        print("javis_mission self-test FAIL (%d):" % len(fails), file=sys.stderr)
        for f in fails:
            print("  -", f, file=sys.stderr)
        return 1
    print("javis_mission self-test OK (선언단독=임무없음 · 질의절 배제 · ack 배제 · "
          "선언+임무=임무있음 · 최소 %d자 경계 · 게이트 기본값 fail-closed · "
          "기계유래 배제 corpus 8종 + 오너문장 무오탐 5종 · CYS_STATE_DIR 밀폐 · surface 결박)"
          % MISSION_MIN_CHARS)
    return 0


_USAGE = """usage: javis_mission.py [record|status|set <임무>|clear|path] [--self-test]
  record : stdin=UserPromptSubmit hook JSON → 임무 대장 갱신(훅 전용)
  status : 0=임무 있음(자율 착수 가) / 1=임무 없음(보고·정지) / 2=판독 불가(=없음 취급)
  set    : 오너 구두 지시를 명시 기록  ·  clear : 대장 폐기  ·  path : 대장 경로 1줄
"""


def main(argv):
    if "--self-test" in argv:
        return cmd_self_test()
    cmd = argv[1] if len(argv) > 1 else "status"
    rest = argv[2:]
    table = {"record": cmd_record, "status": cmd_status, "set": cmd_set,
             "clear": cmd_clear, "path": cmd_path}
    fn = table.get(cmd)
    if fn is None:
        sys.stderr.write(_USAGE)
        return 64                                # EX_USAGE — 미지 서브커맨드 거부(fail-closed)
    return fn(rest)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
