#!/usr/bin/env python3
# -*- coding: utf-8 -*-
r"""javis_teamtoken — **대화 승인 → 1회용 팀 생성 토큰**의 단일 소유자 (0.14.42 · 팀 만들기 확인 창 무반응 수정).

## 왜 존재하는가 (2026-09-23 실측)
팀 만들기의 마지막 단계(승인 Feed 카드 [확인 창 열기] → [만들기])가 Control Center 패널 뒤에 깔려
4시간 넘게 막혔다. 오너 방향(티켓 보정): "버튼을 누르는 방식을 쓰지 말고, 대화가 끝나고 주인이 최종
승인하면 master 가 전자동으로 진행". 규약 변경 허용: "안전장치를 만들고 규약을 바꾸는 것은 괜찮다".
이 모듈이 그 **안전장치**다 — 오너가 대화에서 직접 친 짧은 승인에만, 한 번 쓰면 사라지는 토큰을 준다.
설계 정본: 팀만들기-확인창-무반응-수정설계안-최종-20260923.md §6-3~§6-6 · §7-3 · §10 · §11 R11 · §12.

## 승인의 진위는 문장의 내용이 아니라 **출처**로 판정한다 (§6-3 v2 3중 좁힘 + §6-4)
  ⓐ ask 게이트 — master 가 `ask` 로 '질문 열림' 레코드를 먼저 연다(좌석·제안 id·본문 sha256·TTL 300s).
     열린 질문이 없으면 어떤 발화도 승인으로 읽지 않는다. 한 질문은 **사람의 첫 답 1회**로 소비된다
     (승인이든 아니든 — 다른 질문에 대한 '응'이 팀을 만들지 않게). 기계 발화는 질문을 소비하지 않는다.
     ★추가 결박(설계서 밖 · 강화 방향): 질문은 **그 제안을 발행한 좌석**에서만 열 수 있다.
  ⓑ 전문 일치 — 정규화(NFC·앞뒤 공백·끝 문장부호·끝 존칭 어미 1회·내부 공백 제거·소문자) 뒤
     짧은 긍정 화이트리스트와 **통째로 같을 때만** 승인. 부분 문자열 매칭을 쓰지 않는다. 길이 상한 20자.
  ⓒ 거부 신호 선검사 — 부정어·의문형·조건/예시 표현이 원문(또는 공백 제거형)에 있으면 승인 아님.
  ⓓ 애매하면 발급 거부 + 되묻기.
  오너 실키 입력 판별은 `javis_mission.machine_origin`(층1 배달 원장 sha256 · 층2 push 라벨)을 **호출만**
  한다(사본 금지) — harness 내부 알림(`harness_origin`)도 같은 모듈에서 호출한다.

## 발급 조건 (§6-6 · 전부 충족 · 하나라도 불충족이면 발급 0)
  1 ask 열림(이 좌석·미만료·미소비) 2 배달 원장 상태 = ok(부재·판독불가 거부) 3 machine_origin == 사람
  4 발화가 승인 전문 일치 5 그 제안이 대기(pending) 6 제안 본문 sha256 이 ask 시점과 같음
  (+ 대기 팀 제안이 정확히 1건 — 0건·2건 이상이면 되묻기 · §12-1 제안 결박)

## ★설계 공백 해소 — 1회성 토큰으로 ① create 와 ⑦ allow 를 모순 없이 인가하는 **2단 권한**
설계서는 ①`cys-dept create --team-token` 에서 토큰을 검증·소비하고, ⑦생성 성공 뒤 `feed reply allow` 를
token_ok 로 허용한다. 토큰이 1회성이면 ①에서 소비된 토큰으로 ⑦을 인가할 근거가 없다. 선택:
  **소비는 비가역 행위(생성)에서 한 번 일어나고, 그 소비가 '같은 제안에 대한 allow 1회' 라는 더 좁은
  권한을 낳는다. 그 권한은 생성이 성공했다는 기록(settle created)이 있어야만 무장된다.**
  · verify(비소비)→create→consume 순서를 택하지 않은 이유: 검증과 소비 사이에 같은 토큰으로 생성이 두 번
    달릴 수 있고(경합), 생성이 1회성의 보호를 받지 못한다. 비가역 쪽이 원자 소비를 가져야 한다.
  · allow 단계는 반대로 verify → (데몬 해소) → consume 이 안전하다: allow 는 데몬의 pending 항목 해소라
    두 번째 해소가 데몬에서 `item already resolved` 로 막힌다(멱등) — 해소 실패 시 재시도가 가능해진다.
  | 상태      | 사건                              | 다음 상태 | 원장 기록                        |
  |-----------|-----------------------------------|-----------|----------------------------------|
  | issued    | consume --phase create 성공       | consumed  | consumed{phase:create}           |
  | issued    | 120초 경과                        | (만료)    | — (소비 시 token_expired)        |
  | consumed  | settle --outcome created          | created   | settled{created, dept, grant_expires_at} |
  | consumed  | settle --outcome failed           | failed    | settled{failed, code}(종결)      |
  | consumed  | settle 없음(cys-dept 중단)        | consumed  | — allow 영구 불가(grant_not_armed) |
  | created   | consume --phase allow 성공        | done      | consumed{phase:allow}(종결)      |
  | created   | 1800초 경과                       | (만료)    | — allow 거부 grant_expired(팀은 있음) |
  부분 실패: ⓐ생성 실패·토큰 소비됨 → settle failed → allow 영구 거부 · 제안은 pending 그대로 · 재승인은
  새 ask → 새 토큰. ⓑ생성 성공·allow 실패 → verify 는 비소비라 권한 TTL 안에서 재시도 가능 · 해소가
  끝난 뒤 consume 이 권한을 닫는다. ⓒ생성 도중 중단(settle 없음) → allow 불가(실패 방향 = 카드 잔존).
  allow 로 create 를 건너뛸 수 없다(issued 토큰의 allow = grant_not_armed · 토큰은 보존된다).

## 원장 (append-only · 물리 삭제 없음)
경로: `<CYS_STATE_DIR ‖ ~/.cys/state>/teamtoken-<lane>.jsonl` — 상태 디렉터리·레인 키는
`javis_bootstrap.state_dir()`·`lane_key()` 를 호출한다(배달 원장 `delivery-<lane>.jsonl` 과 같은 '항상 레인
접미' 규약). 락: 같은 경로 + `.lock`(`javis_lock.FileLock` — posix fcntl.flock · Windows msvcrt).
읽기-판정-쓰기 전 구간을 락 안에서 한다(1회성은 경합 하에서도 성립). 권한 0600(토큰 보관).
레코드(한 줄 JSON · 공통 `v:1`·`kind`·`event`):
  team-create-ask   ask_opened  {ask_id(16hex), proposal_id, surface, body_digest(64hex), opened_at, expires_at, pid}
  team-create-ask   ask_closed  {ask_id, surface, proposal_id, why, at}
      why ∈ approved | answered_rejected | answered_ambiguous | expired | superseded | proposal_gone |
            multiple_pending | body_changed | feed_unreadable | proposal_body_invalid
  team-create-token token_issued {token(32hex), proposal_id, surface, body_digest, issued_at, expires_at,
                                  consumed:false, ask_id, pid, ppid, via:"hook", hook_session, hook_input,
                                  hook_input_mtime}   ← 훅 목격 증거 — 검증·소비는 이것 없는 레코드를 인가하지 않는다
  team-create-token consumed    {token, phase:create|allow, proposal_id, surface, body_digest, consumed:true, at}
  team-create-token settled     {token, outcome:created|failed, dept?, code?, at, grant_expires_at?}
  team-create-audit issue_refused|consume_refused|settle_refused|ask_refused {code, detail, at, …}
  team-create-audit torn_tail_sealed {at, fragment_sha256, fragment_bytes}
판독은 전부 fail-closed: 해석 불가 줄·미지 사건·v≠1·필드 결측·전이 위반(발급 없는 소비 등)·상한 초과
→ ledger_corrupt. 예외: **개행 없는 마지막 조각**(찢긴 꼬리 = 완료되지 않은 쓰기)은 없었던 일로 보고,
다음 append 가 개행 + torn_tail_sealed 로 봉인한다(봉인된 조각은 계속 무시된다 — 판정이 흔들리지 않는다).
한 번에 두 레코드를 쓸 때는 **닫힘을 먼저** 쓴다(찢기면 토큰이 사라지는 쪽 = 안전 방향).
열린 질문 표지 `<상태>/teamtoken-open-<lane>`(리뷰 F3): 원장에서 파생된 캐시 — 답 없이 열린 질문(만료 + 고지 여유
ASK_NOTICE_GRACE_S 안)이 있을 때만 있다. UserPromptSubmit 런처의 비용 게이트가 셸 글롭 존재 검사로만 본다(외부
명령 0). ask 가 먼저 세우고(못 세우면 질문을 열지 않는다) issue·status 가 원장에 맞춰 걷거나 되살린다.

## CLI (stdout = JSON 1줄 · 단 issue 의 exit 3 은 무출력)
  ask     --proposal <tp-id>                       # master · 좌석은 env(CYS_SURFACE_ID) — 인자로 바꿀 수 없다
  issue   --payload-file F                          # ★UserPromptSubmit 훅 전용 — F = 런처가 **방금** 쓴
                                                    #   <상태>/hook-input-<N>.json(출처 확인 · stdin 경로 없음)
  verify  --token T --proposal P --surface S (--body-digest D | --body-b64 B) [--phase create|allow]
  consume --token T --proposal P --surface S (--body-digest D | --body-b64 B) [--phase create|allow]
  settle  --token T --proposal P --surface S --outcome created|failed [--dept NAME] [--code N]
  inspect --token T                                 # 결박·상태 조회(인가 아님)
  status  [--proposal P]                            # §7-3 관측 — 답 대기 awaiting_answer · 답 없이 만료 ask_expired
  path | messages | digest (--body-b64 B | --body-file F)
종료코드: 0 성공 · 1 거부(사유 코드) · 2 인자 오류(bad_args · argparse) · 3 issue 무동작(열린 질문 없음 등
  — 훅은 아무것도 출력하지 않는다) · 4 기반 고장(ledger_corrupt · lock_unavailable · internal_error).
  **0 이 아닌 모든 값은 '인가 없음'이다.** 시각은 인자로 주입할 수 없다(만료 우회 차단).
사유 코드 전량·오너 문구: `messages` 서브커맨드(§10 원문 행 + 가장 가까운 행 매핑 + 신설 행 표시).

## 보장 범위 (과대 주장 금지 · §12-1 · docs/THREAT-MODEL-mission-gate.md 와 같은 경계)
닫는 것은 **평시 정상 동작 경로**다 — 에이전트의 실수·오해, 기계 push 오인, 재사용·재생·모호성·오탐.
발급자 = 훅(리뷰 SEC-1·M2·SEC-2 · 0.14.42 수정 라운드 1 · RR1-SEC-A 라운드 2): **공개 발급 입구는
`issue_from_hook_file` 하나**(공식 CLI `issue --payload-file` 이 이것을 부른다)이고, 목격 증거는 그 안에서 상태 폴더의
실제 훅 입력 파일로만 만든다 — 런처가 방금 쓴 `hook-input-<N>.json`(정규 파일·내 소유·실시계 60초 안·질문보다 뒤) +
UserPromptSubmit 형식. 공개 모듈 API `issue()`·`issue_from_payload()` 는 호출자가 내민 증거 dict(meta·witness)를
**읽지 않고** 판정 없이 거부한다(not_hook_caller · 질문 무소비 · 감사 1줄 — 종전엔 dict 모양만 봐서 파일·훅·오너
키입력 없이 via=hook 토큰이 나왔다). 발급 레코드의 목격 증거(via=hook·hook_session)가 없으면 검증·소비가 인가하지
않는다. **남는 우회는 고의 위조뿐이다** — 비공개 함수(`_issue_witnessed` 등)를 직접 부르는 것, 같은 UID 로 훅 입력
파일을 형식대로 흉내 내 쓰는 것, 원장·배달 원장·feed 를 직접 쓰는 것(§12-1 경계). 발급자는 훅뿐이라는 것은 규약 R4 와
형식 가드이고, 원장은 위조의 **감사 흔적**이다(사전 차단이 아니다 — 흉내 낸 입력으로 난 발급도 via=hook 으로 남는다).
데몬이 발급을 목격·인증하는 근본 통제는 제품 재설계 과제다(오너 결정 대기).
"""
import argparse
import base64
import hashlib
import json
import os
import re
import secrets
import sys
sys.dont_write_bytecode = True  # SEAL-1 층4: 호출자 env 와 무관하게 형제 import 의 __pycache__ 기록 차단
import time
import unicodedata

# ★번들 파이썬(Windows embeddable · python312._pth) 경로 가드 — 형제 모듈 import 보장(javis_mission 선례).
_SELF_DIR = os.path.dirname(os.path.abspath(__file__))
if _SELF_DIR not in sys.path:
    sys.path.append(_SELF_DIR)

# ★로케일 비의존 I/O(선례 javis_mission) — cp949·LC_ALL=C 파이프에서 한글 JSON 이 죽지 않게.
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

SCHEMA_VERSION = 1

# ── 수치 스펙(설계 §6-5 · 값을 바꾸면 test_teamtoken 이 같은 상수를 읽는다) ─────────────
ASK_TTL_S = 300.0            # 질문 열림 수명
TOKEN_TTL_S = 120.0          # 승인 발화 → 생성 집행
ALLOW_GRANT_TTL_S = 1800.0   # 생성 성공 → ⑦ allow(부트 티켓·편성·각성이 사이에 있다 · §8-2 단계 2~6)
UTTER_MAX_CHARS = 20         # 정규화 후 승인 발화 길이 상한
LOCK_TIMEOUT_S = 5.0         # 락 대기 상한 — 넘기면 거부(허용으로 새지 않는다)
# ── 발급자 = 훅(리뷰 SEC-1·M2) — 훅 입력 파일의 출처 확인 ──
HOOK_INPUT_MAX_AGE_S = 60.0  # 런처가 **방금** 쓴 입력만 — 묵은 파일의 재생 차단(정상 경로는 수백 ms)
HOOK_INPUT_SKEW_S = 2.0      # 파일 시각 ↔ 질문 시각 비교의 여유(파일시스템 mtime 해상도·시계 오차)
HOOK_INPUT_MAX_BYTES = 1024 * 1024
# ── 열린 질문 표지(리뷰 F3) — 훅 비용 게이트가 보는 파생 캐시 ──
ASK_NOTICE_GRACE_S = 300.0   # 답 없이 만료된 질문에 '확인 시간이 지나'(§10)를 1회 말해 줄 여유 — 그 뒤 표지를 걷는다
LEDGER_MAX_BYTES = 8 * 1024 * 1024   # 이 이상은 판독 불가(자르지 않는다 — javis_mission 원장 판독과 같은 태도)
FEED_MAX_BYTES = 64 * 1024 * 1024

TEAM_KIND = "team-create-request"    # src/team_spec.rs KIND
KIND_ASK = "team-create-ask"
KIND_TOKEN = "team-create-token"
KIND_AUDIT = "team-create-audit"

EXIT_OK = 0
EXIT_REFUSED = 1
EXIT_USAGE = 2
EXIT_NO_ASK = 3
EXIT_INTERNAL = 4

# ── 오너 안내 문구 (§10 — 전부 1줄 + 다음 한 걸음) ───────────────────────────────────
_MSG_SAFE_STOP = ("승인을 확인할 근거 기록이 없어 만들지 않았습니다(안전 정지). "
                  "앱을 재시작한 뒤 다시 말씀해 주세요.")
_MSG_CHANGED = "제안 내용이 그사이 바뀌어 만들지 않았습니다 — 바뀐 내용을 다시 확인해 주세요."
_MSG_NOT_RECEIVED = ("승인 말씀이 시스템에 닿지 않았습니다 — 한 번만 더 '만들어'라고 쳐 주세요"
                     "(또는 화면 카드에서 [확인 창 열기] → [만들기]).")
_MSG_NOT_OPEN = "지금은 승인 대기 상태가 아닙니다 — '만들까요?'를 다시 여쭙겠습니다."
# ★(리뷰 m4) §10 행 "토큰 재사용·본문 변경"은 둘을 한 문구로 묶어, 생성 실패 뒤 같은 토큰 재시도에도 "제안 내용이
#   그사이 바뀌어"라고 말했다(본문은 그대로였다) — 재사용은 전용 문구로 분리한다(본문 변경 쪽은 §10 원문 그대로).
_MSG_REUSED = "이 승인은 이미 한 번 쓰였습니다 — 다시 만들려면 '만들까요?'를 한 번 더 여쭙겠습니다."
# ★(리뷰 m5) 설계 밖 강화 코드 surface_not_publisher 가 ask_not_open 문구("다시 여쭙겠습니다")를 빌려, 좌석이 바뀐 뒤엔
#   **영원히** 열 수 없는 질문을 다시 약속했다(순환). 이 좌석에서는 대화 승인이 불가하다는 사실과 화면 경로를 말한다
#   (새 좌석은 옛 제안을 거둘 수도 없다 — superseded 는 발행 좌석 전용 · 대기 1건 잠금이라 다시 올리려면 오너의
#   [만들지 않기]가 먼저다).
_MSG_SEAT_MOVED = ("제안을 올린 자리가 바뀌어 이 자리에서는 대화로 승인받을 수 없습니다 — 화면 카드에서 "
                   "[확인 창 열기] → [만들기]로 만들거나, [만들지 않기] 뒤 다시 부탁해 주세요.")

OWNER_MESSAGES = {
    # ── §10 표 원문(글자 그대로) ──
    "ask_not_open": _MSG_NOT_OPEN,
    "ask_expired": "확인 시간이 지나 다시 여쭙습니다 — 이 내용으로 만들까요?",
    "utterance_ambiguous": "'만들어'라고 짧게 한 번만 말씀해 주시면 바로 만들겠습니다.",
    "utterance_rejected": "네, 아직 만들지 않았습니다. 만들 때 말씀해 주세요.",
    "machine_origin": "방금 문장은 시스템이 보낸 메시지로 확인됩니다 — 주인님이 직접 한 번만 쳐 주세요.",
    "ledger_absent": _MSG_SAFE_STOP,
    "ledger_unreadable": _MSG_SAFE_STOP,
    "no_pending": "지금 대기 중인 팀 제안이 없습니다 — 만들 팀을 먼저 정해 주세요.",
    "multiple_pending": ("대기 중인 제안이 {n}건이라 어느 것인지 확실하지 않습니다 — "
                         "팀 이름을 한 번 말씀해 주세요."),
    "token_expired": "확인이 오래 걸려 승인이 만료됐습니다 — '만들어'라고 한 번만 더 말씀해 주세요.",
    "token_body_mismatch": _MSG_CHANGED,     # §10 행 "토큰 재사용·본문 변경" 의 본문 변경 쪽
    "approval_not_received": _MSG_NOT_RECEIVED,
    "boot_ticket_failed": ("팀은 만들었지만 팀원 자리를 띄우는 티켓 발급에 실패했습니다 — "
                           "지금은 팀장만 깨어 있습니다."),
    "formation_partial": "팀은 만들었지만 자리 {n}개가 아직 뜨지 않았습니다 — 다시 채울까요?",
    "create_failed": "{reason} 제안은 그대로 남아 있습니다.",   # {reason} = 현행 teamCreateErrorText
    # ── §10 에 행이 없는 코드 — 가장 가까운 §10 행을 쓴다 ──
    "body_changed": _MSG_CHANGED,
    "proposal_not_pending": _MSG_CHANGED,
    "token_proposal_mismatch": _MSG_CHANGED,
    "token_missing": _MSG_NOT_RECEIVED,
    "token_unknown": _MSG_NOT_RECEIVED,
    "token_surface_mismatch": _MSG_NOT_RECEIVED,
    # 훅 밖 발급 시도(직접 호출·출처 불명 입력) — 오너 쪽 사실은 "승인이 훅을 거쳐 닿지 않았다"이고, 회복 경로도
    # 같다(오너가 한 번 더 치면 그 프롬프트의 훅이 판정한다).
    "not_hook_caller": _MSG_NOT_RECEIVED,
    "surface_unknown": _MSG_SAFE_STOP,
    "proposal_publisher_unknown": _MSG_SAFE_STOP,
    "proposal_body_invalid": _MSG_SAFE_STOP,
    "feed_unreadable": _MSG_SAFE_STOP,
    "hook_payload_invalid": _MSG_SAFE_STOP,
    "ledger_corrupt": _MSG_SAFE_STOP,
    "lock_unavailable": _MSG_SAFE_STOP,
    "internal_error": _MSG_SAFE_STOP,
    "bad_args": _MSG_SAFE_STOP,
    "not_consumed": _MSG_SAFE_STOP,
    "already_settled": _MSG_SAFE_STOP,
    # ── 분리 행(§10 의 뭉친 행이 사실과 어긋나던 코드 · 리뷰 m4·m5) ──
    "token_consumed": _MSG_REUSED,
    "surface_not_publisher": _MSG_SEAT_MOVED,
    # ── 신설 행(2단 권한의 ⑦ allow 단계 — §10 에 해당 행이 없다) ──
    "grant_not_armed": "팀 생성이 확인되지 않아 승인 카드를 그대로 두었습니다 — 제안은 그대로 남아 있습니다.",
    "grant_revoked": "팀 생성에 실패해 승인 카드를 그대로 두었습니다 — 제안은 그대로 남아 있습니다.",
    "grant_expired": ("팀은 이미 만들어졌습니다 — 승인 카드 정리 시간이 지나 카드만 남아 있으니 "
                      "화면에서 정리해 주세요."),
    # ── 성공·상태 코드(ask_opened 는 master 가 오너에게 할 질문 — §11 R2 문안) ──
    "ask_opened": ("이 내용으로 만들까요? **만들어**라고 말씀해 주시면 바로 만들겠습니다. "
                   "(화면에서 직접 하시려면 Control Center → 승인 Feed 카드의 [확인 창 열기] → [만들기])"),
    "token_issued": "",
    "verified": "",
    "consumed": "",
    "settled": "",
    "inspected": "",
    "awaiting_answer": "",
    "token_ready": "",
    "token_used": "",
    "path": "",
    "messages": "",
    "digest": "",
}
SECTION10_CODES = frozenset([
    "ask_not_open", "ask_expired", "utterance_ambiguous", "utterance_rejected", "machine_origin",
    "ledger_absent", "ledger_unreadable", "no_pending", "multiple_pending", "token_expired",
    "token_body_mismatch", "approval_not_received", "boot_ticket_failed",
    "formation_partial", "create_failed"])
# §10 밖의 행 — 신설(2단 권한 ⑦ allow) + 분리(§10 의 뭉친 행이 사실과 어긋나던 코드).
NEW_ROW_CODES = frozenset(["grant_not_armed", "grant_revoked", "grant_expired",
                           "token_consumed", "surface_not_publisher"])
# 이 모듈이 **거부**로 돌려줄 수 있는 사유 코드 전량(P4 훅·P5 데몬이 그대로 쓴다).
REFUSAL_CODES = (
    "ask_not_open", "ask_expired", "utterance_ambiguous", "utterance_rejected", "machine_origin",
    "ledger_absent", "ledger_unreadable", "no_pending", "multiple_pending", "proposal_not_pending",
    "body_changed", "surface_unknown", "surface_not_publisher", "proposal_publisher_unknown",
    "proposal_body_invalid", "feed_unreadable", "hook_payload_invalid", "not_hook_caller",
    "token_missing", "token_unknown", "token_consumed", "token_expired", "token_proposal_mismatch",
    "token_surface_mismatch", "token_body_mismatch", "grant_not_armed", "grant_revoked",
    "grant_expired", "not_consumed", "already_settled",
    "bad_args", "ledger_corrupt", "lock_unavailable", "internal_error",
)
# 다른 부품(P5·P7)이 오너에게 보일 §10 행 — 문구 단일 출처라 여기 둔다(이 모듈은 돌려주지 않는다).
EXTERNAL_CODES = ("boot_ticket_failed", "formation_partial", "create_failed", "approval_not_received")
_INTERNAL_CODES = frozenset(["ledger_corrupt", "lock_unavailable", "internal_error"])


def owner_message(code, n=2, reason=""):
    """사유 코드 → 오너에게 보일 1줄. 미지 코드는 빈 문자열(호출자가 그대로 드러낸다)."""
    text = OWNER_MESSAGES.get(code, "")
    return text.replace("{n}", str(n)).replace("{reason}", reason or "").strip()


# ══════════════════════════════════════════════════════════════════════════════
# ⓑⓒ 승인 판정 — 부분 문자열 매칭 금지 · 거부 신호 선검사 · 애매 = 거부
# ══════════════════════════════════════════════════════════════════════════════
# 화이트리스트(§6-3 초안 그대로). ★넓히면 오탐이 돌아온다 — 넓힐 때마다 표 B 오탐 스위트를 다시 돌린다(§14-7).
APPROVE_EXACT = frozenset([
    "네", "예", "응", "어", "그래", "좋아", "좋다", "ㅇㅇ", "yes", "ok", "okay",
    "만들어", "만들어줘", "만들어라", "만들자", "만드세요", "만들어주세요",
    "네만들어", "응만들어", "그래만들어", "예만들어", "좋아만들어",
    "승인", "승인한다", "승인해", "진행", "진행해", "진행하자", "고", "가자",
])
# 거부 신호 — 하나라도 있으면 승인이 아니다(화이트리스트보다 먼저 본다).
#   §6-3 초안 + 이식 시 추가(뒤 10개 · '만들지 마'가 되묻기가 아니라 '아직 만들지 않았습니다'로 답하게).
#   추가는 거부 쪽으로만 움직인다 — 승인 집합(화이트리스트)은 넓히지 않았다.
NEGATORS = ("안 ", "안돼", "안 돼", "하지마", "하지 마", "말고", "말아", "아니", "아직", "나중",
            "취소", "그만", "보류", "빼고", "없이", "지마",
            "싫", "안해", "안 해", "하지말", "멈춰", "기다려", "잠깐", "no", "stop", "cancel")
INTERROGATIVES = ("?", "？", "까", "나요", "을까", "ㄹ까", "어때", "인지", "건지", "하죠")
CONDITIONALS = ("면 ", "하면", "라면", "예를", "예시", "가정", "혹시", "만약", "대신")

_PUNCT_TAIL = re.compile(r"[\s.!?~,·…！。，～．]+$")
_HONORIFIC_TAIL = re.compile(r"(주세요|주십시오|하세요|해주세요|합니다|해요|요)$")
_WS = re.compile(r"\s+")


def normalize_utterance(text):
    """승인 판정용 정규화 — NFC → 앞뒤 공백 → 끝 문장부호 → 끝 존칭 어미(1회) → 내부 공백 제거 → 소문자.

    NFC 인 이유: 조합형(NFD) 한글로 들어온 '만들어'가 다른 문자열로 남지 않게(정규화가 넓히는 것은
    같은 글자의 다른 인코딩뿐이다 — 판정은 여전히 화이트리스트 전문 일치다).
    """
    s = unicodedata.normalize("NFC", text or "").strip()
    s = _PUNCT_TAIL.sub("", s)
    s = _HONORIFIC_TAIL.sub("", s)
    s = _WS.sub("", s)
    return s.lower()


def approval_verdict(text):
    """(verdict, 사유) — verdict ∈ {"approve", "reject", "ambiguous"}.

    reject = 명백한 부정·질문·조건(되묻지 않고 '아직 만들지 않았습니다') ·
    ambiguous = 승인 확정 불가(발급 거부 + 되묻기). **실패 방향: 판정이 흔들리면 승인이 아닌 쪽**이다 —
    승인이 되는 유일한 경로는 화이트리스트 전문 일치다.
    """
    raw = unicodedata.normalize("NFC", text or "").strip()
    if not raw:
        return "reject", "빈 발화"
    low = raw.lower()
    compact = _WS.sub("", low)       # '만들지 마' → '만들지마' 도 '지마' 로 잡는다
    for w in NEGATORS:
        if w in low or w in compact:
            return "reject", "부정어 포함(%r) — 승인으로 읽지 않는다" % w
    for w in INTERROGATIVES:
        if w in low or w in compact:
            return "reject", "의문형 포함(%r) — 질문이지 승인이 아니다" % w
    for w in CONDITIONALS:
        if w in low or w in compact:
            return "reject", "조건·예시 표현 포함(%r) — 승인이 아니다" % w
    norm = normalize_utterance(raw)
    # ★존칭 어미를 걷기 **전** 형태도 전문 일치로 본다: 화이트리스트의 '만드세요'는 어미 규칙이 끝 '요'를
    #   걷어 '만드세'가 되므로 시제품 v2 에서는 **영영 일치하지 않는 원소**였다(이식 중 불변식 검사가 적발).
    #   이 형태도 화이트리스트와 통째로 같아야만 승인이다 — 부분 일치는 여전히 없다.
    keep = _WS.sub("", _PUNCT_TAIL.sub("", raw)).lower()
    if len(norm) > UTTER_MAX_CHARS:
        return "ambiguous", ("정규화 길이 %d자 > 상한 %d자 — 긴 문장은 승인으로 읽지 않는다(되묻기)"
                             % (len(norm), UTTER_MAX_CHARS))
    if norm in APPROVE_EXACT or keep in APPROVE_EXACT:
        return "approve", "짧은 긍정 전문 일치(%r)" % (norm if norm in APPROVE_EXACT else keep)
    return "ambiguous", "긍정 화이트리스트와 전문 일치하지 않음(%r) — 되묻기" % norm


# ══════════════════════════════════════════════════════════════════════════════
# 제안 본문 결박 — 스키마 SOT = src/team_spec.rs parse_body(키 4개 · v=1 · 문자열)
# ══════════════════════════════════════════════════════════════════════════════
_TP_ID = re.compile(r"tp-[A-Za-z0-9_\-]{1,61}")      # team_spec.rs validate_id · cys-dept team_spec_check
_HEX16 = re.compile(r"[0-9a-f]{16}")
_HEX32 = re.compile(r"[0-9a-f]{32}")
_HEX64 = re.compile(r"[0-9a-f]{64}")
_DEPT = re.compile(r"[A-Za-z0-9_\-]{1,64}")


def parse_team_body(body):
    """(dict|None, 사유) — 팀 제안 본문 판독. 숨은 필드·버전·타입 위반은 None(결박 불가)."""
    if not isinstance(body, str) or not body:
        return None, "제안 본문이 없다"
    try:
        d = json.loads(body)
    except ValueError as e:
        return None, "제안 본문이 JSON 이 아니다(%s)" % e
    if not isinstance(d, dict) or sorted(d) != ["display", "id", "purpose", "v"]:
        return None, "제안 본문 키는 v·id·display·purpose 넷이어야 한다"
    if d.get("v") != 1 or isinstance(d.get("v"), bool):
        return None, "제안 본문 버전이 1 이 아니다"
    if not all(isinstance(d.get(k), str) and d.get(k) for k in ("id", "display", "purpose")):
        return None, "제안 본문의 id·display·purpose 가 비었거나 문자열이 아니다"
    if not _TP_ID.fullmatch(d["id"]):
        return None, "제안 id 형식 위반"
    return d, ""


def body_digest(body):
    """제안 본문의 결박 해시(64hex) — **정규형** sha256: 키 정렬 · 구분자 `,`/`:` · 비ASCII 원문 UTF-8.

    원문 바이트가 아니라 정규형인 이유: 데몬은 본문을 의미로 대조한다(`team_spec::match_pending` 이
    `parse_body` 결과를 비교) — 재직렬화로 키 순서·공백이 달라져도 같은 제안은 같은 해시여야 한다.
    P5(Rust)는 사본을 만들지 말고 `digest --body-b64` 를 부르거나 consume 에 `--body-b64` 를 넘긴다.
    판독 불가 본문은 ValueError(결박 불가 = 거부).
    """
    d, err = parse_team_body(body)
    if d is None:
        raise ValueError(err)
    canon = json.dumps({"v": 1, "id": d["id"], "display": d["display"], "purpose": d["purpose"]},
                       ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canon.encode("utf-8")).hexdigest()


# ══════════════════════════════════════════════════════════════════════════════
# 경로 · 좌석 — 규약 소유자를 호출한다(사본 금지)
# ══════════════════════════════════════════════════════════════════════════════
def _mission():
    """판별 모듈 — `machine_origin`·`read_delivery`·`harness_origin` 의 단일 출처. 부재 = 예외(→ 거부)."""
    import javis_mission
    return javis_mission


def ledger_path():
    """이 레인의 토큰 원장 경로. 상태 루트·레인 키 규약은 javis_bootstrap 이 소유한다."""
    import javis_bootstrap
    return os.path.join(javis_bootstrap.state_dir(), "teamtoken-%s.jsonl" % javis_bootstrap.lane_key())


def open_marker_path():
    """이 레인의 **열린 질문 표지** 경로 — UserPromptSubmit 런처(role-bootstrap.sh ⑤-b)의 비용 게이트가
    셸 글롭 `teamtoken-open-*` 존재 검사 하나(외부 명령 0)로 본다(리뷰 F3).

    표지는 원장에서 **파생된 캐시**다(판정 근거가 아니다 — 판정은 언제나 원장 내용으로 한다). 있어야 할 때:
    답 없이 열린 질문이 있고 그 만료 + ASK_NOTICE_GRACE_S 가 아직 지나지 않았다. 종전 게이트(원장 mtime 10분 창)는
    거부 감사 레코드 한 줄에도 10분 동안 모든 좌석의 모든 프롬프트가 발급기를 띄웠고(맥 약 +130ms), 원장이 한 번
    생긴 기계는 영구히 date·stat 2회(약 +8ms)를 냈다. 실패 방향: 표지를 쓸 수 없으면 질문을 열지 않는다(open_ask) ·
    표지를 걷지 못하면 발급기가 한 번 더 불릴 뿐이다(비용 쪽 · 판정 불변) · 표지가 사라졌으면 status 가 되살린다."""
    import javis_bootstrap
    return os.path.join(javis_bootstrap.state_dir(), "teamtoken-open-%s" % javis_bootstrap.lane_key())


def _live_asks(asks, now):
    """표지를 세워 둘 질문 — 답 없이 열린 것 중 만료 + 고지 여유가 아직 남은 것."""
    return [a for a in asks.values()
            if a["closed"] is None and now <= float(a["rec"]["expires_at"]) + ASK_NOTICE_GRACE_S]


def _write_marker(asks_live):
    """표지 기록(원자 교체). 실패는 예외 — 부르는 쪽이 방향을 정한다(open_ask = 질문을 열지 않는다)."""
    p = open_marker_path()
    body = json.dumps({"v": SCHEMA_VERSION, "asks": sorted(a["rec"]["ask_id"] for a in asks_live),
                       "until": max(float(a["rec"]["expires_at"]) for a in asks_live) + ASK_NOTICE_GRACE_S},
                      sort_keys=True) + "\n"
    # 임시 이름은 점(.)으로 시작한다 — 런처 글롭 `teamtoken-open-*` 에 걸리지 않게(찢긴 임시 파일이 표지로 읽히면 안 된다).
    tmp = os.path.join(os.path.dirname(p), ".%s.%d.tmp" % (os.path.basename(p), os.getpid()))
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as f:
            f.write(body)
        os.replace(tmp, p)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def _drop_marker():
    try:
        os.remove(open_marker_path())
    except FileNotFoundError:
        pass


def _sync_marker(asks, now):
    """표지를 원장 파생 상태에 맞춘다(best-effort — 캐시라 실패가 판정을 바꾸지 않는다)."""
    try:
        live = _live_asks(asks, now)
        if live:
            if not os.path.isfile(open_marker_path()):
                _write_marker(live)
        else:
            _drop_marker()
    except Exception:  # noqa: BLE001 — 캐시 동기화 실패는 비용 쪽으로만 샌다
        pass


def feed_jsonl_path():
    """데몬 feed 영속 파일 — 데몬 상태 디렉터리/feed.jsonl(state.rs persist_feed_item · last-wins).

    posix = 소켓 부모 디렉터리(javis_cycle_verifier·javis_resource_gate 와 같은 규칙) ·
    Windows = named pipe 슬러그 매핑(단일 출처 javis_state_snapshot — 복제하지 않는다).
    ★이 파일은 발급 **전 검사**의 근거일 뿐이다 — 생성 시점의 정본 대조는 데몬(P5)이 자기 메모리로 한다.
    """
    sock = (os.environ.get("CYS_SOCKET", "").strip() or os.environ.get("AITERM_SOCKET", "").strip())
    if os.name == "nt":
        import javis_state_snapshot
        return os.path.join(javis_state_snapshot._win_state_dir_for_socket(sock or "\\\\.\\pipe\\cys"),
                            "feed.jsonl")
    if sock:
        return os.path.join(os.path.dirname(os.path.abspath(sock)), "feed.jsonl")
    return os.path.join(os.path.expanduser("~"), ".local", "state", "cys", "feed.jsonl")


def _surface_key(ref):
    """좌석 참조 정규형(숫자부) — `javis_bootstrap.my_surface_key` 와 같은 규칙의 **인자판**
    ('12'·'surface:12' → '12' · 숫자 없는 이름은 원문). 빈 값은 빈 값(결측은 값이 아니다)."""
    raw = str(ref if ref is not None else "").strip()
    if not raw:
        return ""
    return re.sub(r"[^0-9]", "", raw) or raw


def _env_surface():
    try:
        import javis_bootstrap
        return javis_bootstrap.my_surface_key() or ""
    except Exception:
        return _surface_key(os.environ.get("CYS_SURFACE_ID", "") or os.environ.get("AITERM_SURFACE_ID", ""))


def _publisher(item):
    """제안 발행 좌석 — 커널 peer 로 각인된 publisher_surface 우선, 없으면 surface_id."""
    pub = item.get("publisher_surface")
    if pub is None:
        pub = item.get("surface_id")
    return _surface_key(pub) if pub is not None else ""


def read_team_proposals(path=None):
    """(팀 제안 항목 목록|None, 사유) — feed.jsonl last-wins. 부재·상한 초과·손상 줄 = None(판독 불가).

    개행 없는 마지막 조각은 데몬이 지금 쓰는 중일 수 있어 건너뛴다(데몬 락 밖에서 읽는다).
    """
    p = path or feed_jsonl_path()
    if not os.path.isfile(p):
        return None, "feed.jsonl 부재: %s" % p
    try:
        if os.path.getsize(p) > FEED_MAX_BYTES:
            return None, "feed.jsonl 이 상한 %d 바이트 초과: %s" % (FEED_MAX_BYTES, p)
        with open(p, "rb") as f:
            text = f.read().decode("utf-8")
    except (OSError, UnicodeDecodeError) as e:
        return None, "feed.jsonl 판독 실패(%s): %s" % (e, p)
    lines = text.split("\n")
    lines.pop()                                  # 개행으로 끝나면 "" · 아니면 쓰는 중인 조각
    last = {}
    for i, ln in enumerate(lines):
        if not ln.strip():
            continue
        try:
            r = json.loads(ln)
        except ValueError:
            return None, "feed.jsonl %d행 해석 불가: %s" % (i + 1, p)
        if isinstance(r, dict) and isinstance(r.get("request_id"), str):
            last[r["request_id"]] = r
    return [r for r in last.values() if r.get("kind") == TEAM_KIND], ""


def _pending(items):
    return [i for i in items if i.get("kind") == TEAM_KIND and i.get("status") == "pending"]


# ══════════════════════════════════════════════════════════════════════════════
# 원장 판독 · 검증 · 파생 (fail-closed)
# ══════════════════════════════════════════════════════════════════════════════
class _Corrupt(Exception):
    pass


class _LockFail(Exception):
    pass


def _is_num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _is_str(v):
    return isinstance(v, str) and bool(v)


def _re(rx):
    return lambda v: isinstance(v, str) and bool(rx.fullmatch(v))


_REQ = {
    (KIND_ASK, "ask_opened"): {"ask_id": _re(_HEX16), "proposal_id": _re(_TP_ID), "surface": _is_str,
                               "body_digest": _re(_HEX64), "opened_at": _is_num, "expires_at": _is_num},
    (KIND_ASK, "ask_closed"): {"ask_id": _re(_HEX16), "why": _is_str, "at": _is_num},
    (KIND_TOKEN, "token_issued"): {"token": _re(_HEX32), "proposal_id": _re(_TP_ID), "surface": _is_str,
                                   "body_digest": _re(_HEX64), "issued_at": _is_num,
                                   "expires_at": _is_num, "ask_id": _re(_HEX16)},
    (KIND_TOKEN, "consumed"): {"token": _re(_HEX32), "phase": lambda v: v in ("create", "allow"),
                               "at": _is_num},
    (KIND_TOKEN, "settled"): {"token": _re(_HEX32), "outcome": lambda v: v in ("created", "failed"),
                              "at": _is_num},
    (KIND_AUDIT, "issue_refused"): {"code": _is_str, "at": _is_num},
    (KIND_AUDIT, "consume_refused"): {"code": _is_str, "at": _is_num},
    (KIND_AUDIT, "settle_refused"): {"code": _is_str, "at": _is_num},
    (KIND_AUDIT, "ask_refused"): {"code": _is_str, "at": _is_num},
    (KIND_AUDIT, "torn_tail_sealed"): {"at": _is_num},
}


def _validate(rec):
    if not isinstance(rec, dict):
        return "레코드가 객체가 아니다"
    v = rec.get("v")
    if v != SCHEMA_VERSION or isinstance(v, bool):
        return "미지 스키마 v=%r" % (v,)
    req = _REQ.get((rec.get("kind"), rec.get("event")))
    if req is None:
        return "미지 사건 %r/%r" % (rec.get("kind"), rec.get("event"))
    for k, ok in req.items():
        if not ok(rec.get(k)):
            return "%s 의 필드 %r 결측·형식 위반" % (rec.get("event"), k)
    if rec.get("event") == "settled" and rec.get("outcome") == "created":
        if not _re(_DEPT)(rec.get("dept")) or not _is_num(rec.get("grant_expires_at")):
            return "settled(created) 의 dept·grant_expires_at 결측"
    return ""


def _is_seal_line(line):
    try:
        r = json.loads(line)
    except ValueError:
        return False
    return isinstance(r, dict) and r.get("kind") == KIND_AUDIT and r.get("event") == "torn_tail_sealed"


def _load(path):
    """원장 → 검증된 레코드 목록. 부재 = []. 손상 = _Corrupt(fail-closed)."""
    if not os.path.exists(path):
        return []
    if os.path.isdir(path):
        raise _Corrupt("원장 자리가 디렉터리다: %s" % path)
    size = os.path.getsize(path)
    if size > LEDGER_MAX_BYTES:
        raise _Corrupt("원장 %d 바이트가 상한 %d 초과 — 자르지 않고 판독 불가로 접는다: %s"
                       % (size, LEDGER_MAX_BYTES, path))
    with open(path, "rb") as f:
        raw = f.read()
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as e:
        raise _Corrupt("원장 UTF-8 판독 실패(%s): %s" % (e, path))
    lines = text.split("\n")
    lines.pop()                      # 개행 없는 마지막 조각 = 완료되지 않은 쓰기 → 없었던 일
    out = []
    for i, ln in enumerate(lines):
        if i + 1 < len(lines) and _is_seal_line(lines[i + 1]):
            continue                 # 봉인된 찢긴 조각(항상 무시 — 판정이 흔들리지 않게)
        try:
            rec = json.loads(ln)
        except ValueError:
            raise _Corrupt("원장 %d행 해석 불가: %s" % (i + 1, path))
        err = _validate(rec)
        if err:
            raise _Corrupt("원장 %d행 — %s: %s" % (i + 1, err, path))
        out.append(rec)
    return out


def _derive(recs):
    """(asks, tokens, audits) — 전이 규칙대로 재생한다. 규칙 위반 = 위조 정황 → _Corrupt.

    asks[ask_id]   = {"rec": ask_opened, "closed": ask_closed|None, "idx": n}
    tokens[token]  = {"issued": …, "state": issued|consumed|created|failed|done, "settled": …, "idx": n}
    """
    asks, tokens, audits = {}, {}, []
    for idx, r in enumerate(recs):
        ev = r["event"]
        if ev == "ask_opened":
            if r["ask_id"] in asks:
                raise _Corrupt("질문 id 중복(%s)" % r["ask_id"])
            asks[r["ask_id"]] = {"rec": r, "closed": None, "idx": idx}
        elif ev == "ask_closed":
            a = asks.get(r["ask_id"])
            if a is None or a["closed"] is not None:
                raise _Corrupt("없는 질문을 닫거나 이중으로 닫았다(%s)" % r["ask_id"])
            a["closed"] = r
        elif ev == "token_issued":
            a = asks.get(r["ask_id"])
            if r["token"] in tokens:
                raise _Corrupt("토큰 중복 발급")
            if a is None or a["closed"] is None or a["closed"].get("why") != "approved":
                raise _Corrupt("승인으로 닫힌 질문 없이 발급된 토큰")
            if a.get("token"):
                raise _Corrupt("한 질문에 토큰 둘(%s)" % r["ask_id"])
            a["token"] = r["token"]
            ar = a["rec"]
            if (ar["proposal_id"], ar["surface"], ar["body_digest"]) != (
                    r["proposal_id"], r["surface"], r["body_digest"]):
                raise _Corrupt("토큰 결박이 질문 결박과 다르다")
            tokens[r["token"]] = {"issued": r, "state": "issued", "settled": None, "idx": idx}
        elif ev == "consumed":
            t = tokens.get(r["token"])
            if t is None:
                raise _Corrupt("발급 없는 소비")
            if r["phase"] == "create":
                if t["state"] != "issued":
                    raise _Corrupt("create 소비 전이 위반(%s)" % t["state"])
                t["state"] = "consumed"
            else:
                if t["state"] != "created":
                    raise _Corrupt("allow 소비 전이 위반(%s)" % t["state"])
                t["state"] = "done"
        elif ev == "settled":
            t = tokens.get(r["token"])
            if t is None or t["state"] != "consumed":
                raise _Corrupt("소비 없는 settled(전이 위반)")
            t["state"] = "created" if r["outcome"] == "created" else "failed"
            t["settled"] = r
        else:
            audits.append(r)
    return asks, tokens, audits


class _Locked:
    """원장 락 — 읽기·판정·쓰기 전 구간. 획득 실패(점유·불가)는 _LockFail(→ 거부)."""

    def __init__(self, path):
        self.path = path + ".lock"
        self.lk = None

    def __enter__(self):
        import javis_lock
        self.lk = javis_lock.FileLock(self.path, owner="javis_teamtoken", blocking=True,
                                      timeout=LOCK_TIMEOUT_S)
        st = self.lk.acquire()
        if st != javis_lock.ACQUIRED:
            raise _LockFail("원장 락 %s(%s) — %.1fs 대기 후 거부: %s"
                            % (st, self.lk.detail, LOCK_TIMEOUT_S, self.path))
        return self

    def __exit__(self, *_exc):
        if self.lk is not None:
            self.lk.release()
        return False


def _dumps(rec):
    return json.dumps(rec, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _append(path, recs):
    """레코드 append(락 안에서만 부른다). 찢긴 꼬리가 있으면 개행 + 봉인 레코드를 먼저 쓴다. fsync."""
    d = os.path.dirname(path)
    if d:
        os.makedirs(d, exist_ok=True)
    payload = "".join(_dumps(r) + "\n" for r in recs).encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT | getattr(os, "O_BINARY", 0), 0o600)
    try:
        size = os.fstat(fd).st_size
        if size > 0:
            with open(path, "rb") as f:
                f.seek(max(0, size - 65536))
                tail = f.read()
            if not tail.endswith(b"\n"):
                frag = tail.rsplit(b"\n", 1)[-1]
                seal = {"v": SCHEMA_VERSION, "kind": KIND_AUDIT, "event": "torn_tail_sealed",
                        "at": time.time(), "fragment_bytes": len(frag),
                        "fragment_sha256": hashlib.sha256(frag).hexdigest()}
                payload = b"\n" + (_dumps(seal) + "\n").encode("utf-8") + payload
        view = memoryview(payload)
        while view:
            n = os.write(fd, view)
            view = view[n:]
        os.fsync(fd)
        if hasattr(os, "fchmod"):
            os.fchmod(fd, 0o600)
    finally:
        os.close(fd)


def _audit(path, event, code, detail, **fields):
    """거부 감사 1줄(best-effort) — 감사 쓰기 실패가 거부를 허용으로 바꾸지 않는다."""
    rec = {"v": SCHEMA_VERSION, "kind": KIND_AUDIT, "event": event, "code": code,
           "detail": (detail or "")[:300], "at": time.time()}
    rec.update(fields)
    try:
        with _Locked(path):
            _append(path, [rec])
    except Exception:
        pass


# ══════════════════════════════════════════════════════════════════════════════
# 결과 · 공통 가드
# ══════════════════════════════════════════════════════════════════════════════
def _result(ok, code, detail="", exit_code=None, n=2, **extra):
    if exit_code is None:
        if ok:
            exit_code = EXIT_OK
        elif code in _INTERNAL_CODES:
            exit_code = EXIT_INTERNAL
        elif code == "bad_args":
            exit_code = EXIT_USAGE
        else:
            exit_code = EXIT_REFUSED
    r = {"ok": bool(ok), "code": code, "message": owner_message(code, n=n), "detail": detail or "",
         "exit": exit_code}
    r.update(extra)
    return r


def _guarded(fn):
    """예외·락 실패·원장 손상은 전부 거부로 접는다(크래시가 허용으로 새지 않는다)."""
    def wrapper(*a, **k):
        try:
            return fn(*a, **k)
        except _LockFail as e:
            return _result(False, "lock_unavailable", str(e))
        except _Corrupt as e:
            return _result(False, "ledger_corrupt", str(e))
        except Exception as e:  # noqa: BLE001 — fail-closed 가 목적이다
            return _result(False, "internal_error", "%s: %s" % (type(e).__name__, e))
    wrapper.__name__ = fn.__name__
    wrapper.__doc__ = fn.__doc__
    return wrapper


def _now(now):
    return time.time() if now is None else float(now)


def _feed(feed_items):
    if feed_items is not None:
        return list(feed_items), ""
    return read_team_proposals()


# ══════════════════════════════════════════════════════════════════════════════
# ask — 질문 열기(master)
# ══════════════════════════════════════════════════════════════════════════════
@_guarded
def open_ask(proposal_id, surface=None, now=None, feed_items=None):
    """'이 제안으로 만들까요?' 질문을 레코드로 연다. 같은 좌석의 열린 질문은 superseded 로 닫는다.

    실패 방향: 선검사 하나라도 불충족이면 질문을 열지 않는다(열린 질문이 없으면 발급도 0).
    """
    now = _now(now)
    pid = (proposal_id or "").strip()
    if not _TP_ID.fullmatch(pid):
        return _result(False, "bad_args", "제안 id 형식 위반(%r)" % pid)
    surf = _env_surface() if surface is None else _surface_key(surface)
    if not surf:
        return _result(False, "surface_unknown", "이 pane 의 좌석을 모른다(CYS_SURFACE_ID 부재)")
    path = ledger_path()

    def refuse(code, detail, n=2):
        _audit(path, "ask_refused", code, detail, proposal_id=pid, surface=surf)
        return _result(False, code, detail, n=n, proposal_id=pid, surface=surf)

    # 원장 선검사 — 층1 근거가 없으면 발급이 불가능하므로 질문부터 열지 않는다(§14-2 가용성 대가).
    m = _mission()
    _deliv, lstatus, ldetail = m.read_delivery(now=now)
    if lstatus != m.LEDGER_OK:
        return refuse("ledger_absent" if lstatus == m.LEDGER_ABSENT else "ledger_unreadable", ldetail)
    items, ferr = _feed(feed_items)
    if items is None:
        return refuse("feed_unreadable", ferr)
    pend = _pending(items)
    if not pend:
        return refuse("no_pending", "대기 중인 팀 제안 0건")
    if len(pend) > 1:
        return refuse("multiple_pending", "대기 중인 팀 제안 %d건" % len(pend), n=len(pend))
    it = pend[0]
    if it.get("request_id") != pid:
        return refuse("proposal_not_pending", "이 제안(%s)은 대기 중이 아니다 — 대기 중: %s"
                      % (pid, it.get("request_id")))
    pub = _publisher(it)
    if not pub:
        return refuse("proposal_publisher_unknown", "제안 발행 좌석 기록이 없다")
    if pub != surf:
        return refuse("surface_not_publisher", "질문은 제안을 올린 좌석(%s)에서만 연다 — 이 좌석=%s"
                      % (pub, surf))
    try:
        dig = body_digest(it.get("body"))
    except ValueError as e:
        return refuse("proposal_body_invalid", str(e))
    spec, _e = parse_team_body(it.get("body"))
    ask_id = secrets.token_hex(8)
    with _Locked(path):
        asks, _t, _a = _derive(_load(path))
        closes = [{"v": SCHEMA_VERSION, "kind": KIND_ASK, "event": "ask_closed", "ask_id": aid,
                   "surface": a["rec"]["surface"], "proposal_id": a["rec"]["proposal_id"],
                   "why": "superseded", "at": now}
                  for aid, a in sorted(asks.items(), key=lambda kv: kv[1]["idx"])
                  if a["closed"] is None and a["rec"]["surface"] == surf]
        rec = {"v": SCHEMA_VERSION, "kind": KIND_ASK, "event": "ask_opened", "ask_id": ask_id,
               "proposal_id": pid, "surface": surf, "body_digest": dig, "opened_at": now,
               "expires_at": now + ASK_TTL_S, "pid": os.getpid()}
        # ★(리뷰 F3) 표지를 **먼저** 세운다 — 못 세우면 질문을 열지 않는다(예외 → internal_error · 원장 무기록).
        #   질문만 열리고 표지가 없으면 훅 비용 게이트가 발급기를 부르지 않아 오너의 "만들어"가 조용히 발급 0 이 된다.
        #   반대로 표지만 남는 경우(아래 append 실패)는 발급기가 한 번 더 불린 뒤 걷힌다(비용 쪽 · 판정 불변).
        superseded = {c["ask_id"] for c in closes}
        live = [a for aid, a in asks.items() if aid not in superseded] + [{"rec": rec, "closed": None}]
        _write_marker(_live_asks({i: a for i, a in enumerate(live)}, now))
        _append(path, closes + [rec])
    return _result(True, "ask_opened", "질문 열림(TTL %ds)" % ASK_TTL_S, ask_id=ask_id,
                   proposal_id=pid, surface=surf, display=spec["display"], body_digest=dig,
                   expires_at=now + ASK_TTL_S, superseded=len(closes))


# ══════════════════════════════════════════════════════════════════════════════
# issue — 훅 전용 발급
# ══════════════════════════════════════════════════════════════════════════════
def _open_ask_for(asks, surf):
    cand = [a for a in asks.values() if a["closed"] is None and a["rec"]["surface"] == surf]
    return max(cand, key=lambda a: a["idx"]) if cand else None


def _judge(prompt, surf, now, feed_items, ask, meta):
    """열린 질문에 대한 판정 → (결과, append 할 레코드). 락 안에서 불린다."""
    a = ask["rec"]
    aid, pid = a["ask_id"], a["proposal_id"]
    psha = hashlib.sha256(prompt.encode("utf-8")).hexdigest()
    base = {"ask_id": aid, "proposal_id": pid, "surface": surf}

    def refuse(code, detail, close_why=None, n=2):
        recs = []
        if close_why:
            recs.append({"v": SCHEMA_VERSION, "kind": KIND_ASK, "event": "ask_closed", "ask_id": aid,
                         "surface": surf, "proposal_id": pid, "why": close_why, "at": now})
        recs.append({"v": SCHEMA_VERSION, "kind": KIND_AUDIT, "event": "issue_refused", "code": code,
                     "detail": (detail or "")[:300], "at": now, "ask_id": aid, "proposal_id": pid,
                     "surface": surf, "prompt_sha256": psha, "prompt_chars": len(prompt),
                     "ask_closed": bool(close_why)})
        return _result(False, code, detail, n=n, ask_closed=bool(close_why), **base), recs

    if now > float(a["expires_at"]):
        # ★만료와 미개설을 구분한다(§10 끝 구현 주의) — 만료 고지는 1회, 질문은 여기서 닫힌다.
        return refuse("ask_expired", "질문 TTL %ds 경과(%.0fs 초과)" % (ASK_TTL_S, now - float(a["expires_at"])),
                      close_why="expired")
    # ★(리뷰 SEC-1) 답은 질문 **뒤**에 쳐진 것이어야 한다 — 훅 입력 파일은 그 프롬프트의 훅이 도는 순간에 쓰이므로
    #   정상 경로에서는 언제나 질문보다 뒤다. 앞이면 질문 전에 친 문장의 재생이다(질문은 소비하지 않는다).
    #   입력 시각 = now − (실시계로 잰 파일 나이). 운영(now 미주입)에서는 파일 mtime 그 자체다(issue_from_hook_file 이
    #   같은 실시각을 나이와 now 양쪽에 쓴다) — 모듈 API 의 now 주입(시험 이음매)에서도 같은 시간축으로 비교된다.
    age = (meta or {}).get("hook_input_age")
    typed_at = now - float(age) if _is_num(age) else None
    if typed_at is None or typed_at < float(a["opened_at"]) - HOOK_INPUT_SKEW_S:
        return refuse("not_hook_caller", "훅 입력이 질문보다 먼저 쓰였다(입력 %s · 질문 %.0f) — 질문 전 발화의 재생"
                      % (typed_at, float(a["opened_at"])))
    m = _mission()
    deliv, lstatus, ldetail = m.read_delivery(now=now)
    if lstatus != m.LEDGER_OK:
        # 사람의 답인지 판정할 수 없다 — 질문은 소비하지 않는다(실패 방향: 발급 0 · 질문 유지)
        return refuse("ledger_absent" if lstatus == m.LEDGER_ABSENT else "ledger_unreadable", ldetail)
    is_machine, why = m.machine_origin(prompt, deliv, lstatus)
    if is_machine:
        return refuse("machine_origin", why)          # 기계 발화는 질문을 소비하지 않는다
    is_harness, hwhy = m.harness_origin(prompt)
    if is_harness:
        return refuse("machine_origin", "harness 내부 알림 — %s" % hwhy)
    # ── 여기부터 '사람의 답' — 승인이든 아니든 이 답이 질문을 1회 소비한다 ──
    verdict, vwhy = approval_verdict(prompt)
    if verdict == "reject":
        return refuse("utterance_rejected", vwhy, close_why="answered_rejected")
    if verdict != "approve":
        return refuse("utterance_ambiguous", vwhy, close_why="answered_ambiguous")
    items, ferr = _feed(feed_items)
    if items is None:
        return refuse("feed_unreadable", ferr, close_why="feed_unreadable")
    pend = _pending(items)
    if not pend:
        return refuse("no_pending", "대기 중인 팀 제안 0건", close_why="proposal_gone")
    if len(pend) > 1:
        return refuse("multiple_pending", "대기 중인 팀 제안 %d건" % len(pend),
                      close_why="multiple_pending", n=len(pend))
    it = pend[0]
    if it.get("request_id") != pid:
        return refuse("proposal_not_pending", "질문한 제안(%s)은 대기 중이 아니다 — 대기 중: %s"
                      % (pid, it.get("request_id")), close_why="proposal_gone")
    try:
        cur = body_digest(it.get("body"))
    except ValueError as e:
        return refuse("proposal_body_invalid", str(e), close_why="proposal_body_invalid")
    if cur != a["body_digest"]:
        return refuse("body_changed", "질문을 연 뒤 제안 본문이 바뀌었다(%s… → %s…)"
                      % (a["body_digest"][:12], cur[:12]), close_why="body_changed")
    token = secrets.token_hex(16)
    recs = [
        # 닫힘을 먼저 쓴다 — 쓰기가 찢기면 토큰이 사라지는 쪽(안전 방향)이 되게.
        {"v": SCHEMA_VERSION, "kind": KIND_ASK, "event": "ask_closed", "ask_id": aid, "surface": surf,
         "proposal_id": pid, "why": "approved", "at": now},
        # 목격 증거(via·hook_session·hook_input) — 검증·소비(_judge_token)가 이것 없는 발급 레코드를 인가하지 않는다.
        {"v": SCHEMA_VERSION, "kind": KIND_TOKEN, "event": "token_issued", "token": token,
         "proposal_id": pid, "surface": surf, "body_digest": cur, "issued_at": now,
         "expires_at": now + TOKEN_TTL_S, "consumed": False, "ask_id": aid,
         "pid": os.getpid(), "ppid": os.getppid(), "via": "hook",
         "hook_session": meta.get("session_id"), "hook_input": meta.get("hook_input"),
         "hook_input_mtime": meta.get("hook_input_mtime")},
    ]
    return _result(True, "token_issued", vwhy, token=token, expires_at=now + TOKEN_TTL_S,
                   body_digest=cur, next="cys-dept create --team-token %s" % token, **base), recs


def _no_ask(prompt, surf, feed_items):
    """열린 질문이 없다. 승인처럼 들리는 말 + 이 좌석이 올린 대기 제안이 있을 때만 알린다(그 밖 무출력)."""
    verdict, _w = approval_verdict(prompt)
    quiet = _result(False, "ask_not_open", "열린 질문 없음", exit_code=EXIT_NO_ASK, surface=surf)
    if verdict != "approve":
        return quiet
    items, _e = _feed(feed_items)
    if items is None:
        return quiet
    mine = [i for i in _pending(items) if _publisher(i) == surf]
    if not mine:
        return quiet
    return _result(False, "ask_not_open", "열린 질문이 없는데 승인처럼 들리는 발화 — master 가 먼저 "
                   "`ask` 로 질문을 열어야 한다", surface=surf, proposal_id=mine[0].get("request_id"))


def _is_hook_meta(meta):
    """훅 목격 증거의 모양 — 비공개 판정 입구의 방어 한 겹일 뿐이다(증거의 **출처**는 모양이 아니라 만든 자리다:
    `_hook_witness` 가 상태 폴더의 실제 훅 입력 파일에서만 만든다 · 공개 입구는 호출자가 내민 dict 를 받지 않는다)."""
    return (isinstance(meta, dict) and meta.get("via") == "hook" and _is_str(meta.get("session_id"))
            and _is_num(meta.get("hook_input_mtime")) and _is_num(meta.get("hook_input_age")))


def issue(prompt, surface=None, now=None, feed_items=None, meta=None):
    """공개 모듈 API 의 옛 발급 입구 — **판정하지 않고 거부한다**(not_hook_caller · exit 1 · 질문 무소비 · 감사 1줄).

    ★(리뷰 RR1-SEC-A) 종전 이 입구는 `meta` 의 **모양**(via=hook·session_id·시각)만 봤다 — dict 리터럴 하나로 파일도
      훅도 오너 키입력도 없이 토큰이 나왔고, 원장엔 via=hook(거짓 훅 증언)으로 남아 데몬 consume 이 그대로 받았다.
      호출자가 내민 증거는 증거가 아니다: 목격 증거는 공개 발급 입구 `issue_from_hook_file` 이 상태 폴더의 실제 훅
      입력 파일에서만 만들고(`_hook_witness`), 판정은 비공개 `_issue_witnessed` 가 그 증거로만 한다. 여기서는 `meta` 를
      읽지 않는다(실재 파일 이름을 대도 같다 — 이름을 빌린 증거로 다른 문장을 판정받는 길이 생기지 않게).
    """
    prompt = prompt if isinstance(prompt, str) else ""
    return _refuse_unwitnessed("모듈 API issue() 직접 호출 — 호출자가 내민 증거(meta)는 증거가 아니다",
                               surface=surface, prompt=prompt)


def _issue_witnessed(prompt, witness, surface=None, now=None, feed_items=None):
    """(비공개) 훅 목격 증거를 가진 발급 판정. 결과 dict — ok 이면 token 동봉.

    매 프롬프트마다 불린다: 열린 질문이 없고 승인처럼 들리지도 않으면 **무기록·무출력**(exit 3).
    기반 고장(원장 손상·락)도 승인처럼 들리는 말이 아니면 조용히 접는다(발급은 어느 쪽이든 0).
    `witness` 는 `issue_from_hook_file` → `_hook_witness` 산출물만 들어온다(공개 입구는 호출자 dict 를 받지 않는다).
    이 함수를 직접 부르는 것은 **고의 위조**다 — 모듈 머리말 '보장 범위'가 닫지 못한다고 적은 경계 쪽이다.
    """
    prompt = prompt if isinstance(prompt, str) else ""
    if not _is_hook_meta(witness):
        return _refuse_unwitnessed("훅 목격 증거가 없는 판정 요청(비공개 입구)", surface=surface, prompt=prompt)
    try:
        return _issue_impl(prompt, surface, _now(now), feed_items, witness)
    except (_LockFail, _Corrupt, Exception) as e:  # noqa: BLE001 — fail-closed
        code = ("lock_unavailable" if isinstance(e, _LockFail)
                else "ledger_corrupt" if isinstance(e, _Corrupt) else "internal_error")
        if isinstance(e, _Corrupt):
            # 손상 원장은 발급이 불가능하다 — 표지를 걷어 매 프롬프트의 발급기 기동을 멈춘다(비용 쪽).
            # 관측은 status(ledger_corrupt)가 한다 · 이 호출의 고지(승인 유사 발화면 판정 불가)는 그대로.
            try:
                _drop_marker()
            except Exception:  # noqa: BLE001
                pass
        loud = approval_verdict(prompt)[0] == "approve"
        return _result(False, code, "%s: %s" % (type(e).__name__, e),
                       exit_code=EXIT_INTERNAL if loud else EXIT_NO_ASK)


def _issue_impl(prompt, surface, now, feed_items, meta):
    surf = _env_surface() if surface is None else _surface_key(surface)
    if not surf:
        return _result(False, "surface_unknown", "좌석 미상 — 판정하지 않는다", exit_code=EXIT_NO_ASK)
    path = ledger_path()
    if os.path.exists(path):
        with _Locked(path):
            recs0 = _load(path)
            asks, _t, _a = _derive(recs0)
            ask = _open_ask_for(asks, surf)
            if ask is not None:
                res, recs = _judge(prompt, surf, now, feed_items, ask, meta)
                if recs:
                    _append(path, recs)
                    asks, _t, _a = _derive(recs0 + recs)
                _sync_marker(asks, now)
                return res
            _sync_marker(asks, now)       # 다른 좌석의 만료 질문 표지도 여유가 지나면 여기서 걷힌다
    else:
        _sync_marker({}, now)             # 원장 없는 표지 = 잔재
    return _no_ask(prompt, surf, feed_items)


def _hook_witness(path, t_real=None):
    """(목격 증거|None, 사유) — `--payload-file` 이 **이 레인의 UserPromptSubmit 런처가 방금 쓴 입력**인가.

    런처(role-bootstrap.sh ⑤)는 훅 입력을 `<상태 폴더>/hook-input-<pid>.json` 에 받아 이 파일 경로를 넘긴다.
    확인: 이름 형식 · 상태 폴더(javis_bootstrap.state_dir) 직속 · 심볼릭 링크 아님 · 정규 파일 · 내 소유(posix) ·
    크기 상한 · 나이 ≤ HOOK_INPUT_MAX_AGE_S(재생 차단). 나이는 **실시계**(파일 mtime 과 같은 시계)로 잰다 —
    모듈 API 의 now 주입(시험 이음매)으로 묵은·미래 파일을 '방금 쓴 입력'으로 만들 수 없다.
    ★보장 경계: 같은 UID 가 이 조건을 갖춘 파일을 **일부러** 써서 부르면 통과한다 — 막는 것은 공식 CLI·공개 모듈
      API 를 훅 밖에서 부르는 평시 실수 경로다(설계 §12-1 경계 · 원장은 그 위조의 감사 흔적).
    """
    if not isinstance(path, str) or not path:
        return None, "훅 입력 파일 경로가 없다(stdin·인자 직접 호출은 훅이 아니다)"
    name = os.path.basename(path)
    if not re.fullmatch(r"hook-input-[0-9]+\.json", name):
        return None, "훅 입력 파일 이름이 런처 형식(hook-input-<N>.json)이 아니다: %s" % name
    try:
        import javis_bootstrap
        sdir = javis_bootstrap.state_dir()
        if not os.path.samefile(os.path.dirname(os.path.abspath(path)), sdir):
            return None, "훅 입력 파일이 상태 폴더(%s) 직속이 아니다: %s" % (sdir, path)
        st = os.lstat(path)
    except (OSError, ValueError) as e:
        return None, "훅 입력 파일 확인 실패(%s): %s" % (e, path)
    import stat as _stat
    if _stat.S_ISLNK(st.st_mode) or not _stat.S_ISREG(st.st_mode):
        return None, "훅 입력이 정규 파일이 아니다(링크·특수 파일): %s" % path
    if hasattr(os, "geteuid") and os.name == "posix" and st.st_uid != os.geteuid():
        return None, "훅 입력 파일 소유자(uid %d)가 이 프로세스가 아니다" % st.st_uid
    if st.st_size > HOOK_INPUT_MAX_BYTES:
        return None, "훅 입력 파일이 상한 %d 바이트 초과" % HOOK_INPUT_MAX_BYTES
    age = (time.time() if t_real is None else float(t_real)) - st.st_mtime
    if age > HOOK_INPUT_MAX_AGE_S or age < -HOOK_INPUT_SKEW_S:
        return None, "훅 입력 파일 나이 %.0fs — 런처가 방금 쓴 입력이 아니다(상한 %ds · 재생 차단)" % (
            age, HOOK_INPUT_MAX_AGE_S)
    return {"via": "hook", "hook_input": name, "hook_input_mtime": st.st_mtime, "hook_input_age": age}, ""


def issue_from_hook_file(path, now=None, feed_items=None):
    """**공개 발급 입구(유일)** — 훅 입력 파일(런처가 방금 쓴 것) → 출처 확인 → 판정. 출처 불명 = not_hook_caller
    (판정 없음 · 질문 무소비 · 감사 1줄). 공식 CLI `issue --payload-file` 이 이것을 부른다.

    `now` 는 모듈 API 의 시험 이음매다(CLI 는 시각을 주입받지 않는다 — X4): 판정 시각에만 쓰이고, 출처 확인(파일
    나이)은 언제나 실시계다. 운영(now 미주입)에서는 같은 실시각을 나이와 판정에 함께 써서, 판정의 '입력 시각'이
    파일 mtime 과 정확히 같다(`_judge` 의 질문-뒤 검사).
    """
    t_real = time.time()
    witness, why = _hook_witness(path, t_real)
    if witness is None:
        return _refuse_unwitnessed(why)
    try:
        with open(path, "rb") as f:
            text = f.read(HOOK_INPUT_MAX_BYTES + 1).decode("utf-8", "replace")
    except OSError as e:
        return _result(False, "hook_payload_invalid", "훅 입력 판독 실패(%s)" % e)
    return _issue_from_payload(text, witness, now=t_real if now is None else now, feed_items=feed_items)


def _refuse_unwitnessed(why, surface=None, prompt=None):
    """훅 밖 발급 시도 — 판정하지 않고(질문 무소비) 거부 · 원장이 있으면 감사 1줄(best-effort · 원장을 새로 만들지 않는다)."""
    surf = _env_surface() if surface is None else _surface_key(surface)
    path = ledger_path()
    if os.path.exists(path):
        extra = {"surface": surf}
        if isinstance(prompt, str):
            extra["prompt_sha256"] = hashlib.sha256(prompt.encode("utf-8")).hexdigest()
        _audit(path, "issue_refused", "not_hook_caller", why, **extra)
    return _result(False, "not_hook_caller", "발급은 UserPromptSubmit 훅만 한다 — %s" % why, surface=surf)


def issue_from_payload(payload_text, now=None, feed_items=None, witness=None):
    """공개 모듈 API 의 옛 입구 — `issue()` 와 같다: 호출자가 내민 `witness` 를 믿지 않고 **판정 없이 거부**한다
    (not_hook_caller · 감사 1줄 · 리뷰 RR1-SEC-A). 훅 입력의 형식 판독은 `issue_from_hook_file` 안에서만 한다."""
    prompt = None
    try:
        obj = json.loads(payload_text)
        if isinstance(obj, dict) and isinstance(obj.get("prompt"), str):
            prompt = obj["prompt"]
    except (TypeError, ValueError):
        pass
    return _refuse_unwitnessed("모듈 API issue_from_payload() 직접 호출 — 호출자가 내민 증거(witness)는 증거가 아니다",
                               prompt=prompt)


def _issue_from_payload(payload_text, witness, now=None, feed_items=None):
    """(비공개) 훅 입력(JSON) → 판정. UserPromptSubmit 형식(hook_event_name·session_id)이 **필수**다(리뷰 SEC-1 — 종전엔
    hook_event_name 이 없어도 통과해 `{"prompt":"그래 만들어"}` 한 줄이 발급 입력이 됐다). `witness` 는
    `issue_from_hook_file` 이 방금 만든 것만 들어온다."""
    try:
        obj = json.loads(payload_text)
    except (TypeError, ValueError) as e:
        return _result(False, "hook_payload_invalid", "훅 JSON 판독 실패(%s)" % e)
    if not isinstance(obj, dict) or not isinstance(obj.get("prompt"), str):
        return _result(False, "hook_payload_invalid", "훅 JSON 에 문자열 prompt 가 없다")
    ev = obj.get("hook_event_name")
    if ev != "UserPromptSubmit":
        return _result(False, "hook_payload_invalid", "UserPromptSubmit 훅 사건이 아니다(hook_event_name=%r)" % (ev,))
    sid = obj.get("session_id")
    if not _is_str(sid):
        return _result(False, "hook_payload_invalid", "훅 입력에 session_id 가 없다")
    meta = dict(witness or {})
    meta["session_id"] = sid
    return _issue_witnessed(obj["prompt"], meta, now=now, feed_items=feed_items)


# ══════════════════════════════════════════════════════════════════════════════
# verify · consume · settle · inspect — 데몬·cys-dept 쪽(검증·소비)
# ══════════════════════════════════════════════════════════════════════════════
def _token_args(token, proposal_id, surface, digest, phase=None):
    """인자 검증 → (정규화 값들, 거부 결과|None). 결측은 값이 아니다 — 빈 값은 전부 거부."""
    if token is None or token == "":
        return None, _result(False, "token_missing", "토큰이 주어지지 않았다")
    if not isinstance(token, str) or not _HEX32.fullmatch(token):
        return None, _result(False, "token_unknown", "토큰 형식(32 소문자 hex)이 아니다 — 발급된 적 없는 값")
    pid = proposal_id.strip() if isinstance(proposal_id, str) else ""
    if not _TP_ID.fullmatch(pid):
        return None, _result(False, "bad_args", "제안 id 결측·형식 위반")
    surf = _surface_key(surface)
    if not surf:
        return None, _result(False, "bad_args", "좌석 결측")
    if digest is not None or phase is not None:
        if not isinstance(digest, str) or not _HEX64.fullmatch(digest):
            return None, _result(False, "bad_args", "본문해시 결측·형식 위반(64 소문자 hex)")
        if phase not in ("create", "allow"):
            return None, _result(False, "bad_args", "phase 는 create|allow")
    return (token, pid, surf, digest, phase), None


def _judge_token(t, pid, surf, digest, phase, now):
    if t is None:
        return _result(False, "token_unknown", "원장에 없는 토큰 — 발급된 적 없다")
    iss, st = t["issued"], t["state"]
    ext = {"proposal_id": iss["proposal_id"], "surface": iss["surface"], "state": st, "phase": phase}
    # ★(리뷰 SEC-2) 발급 레코드의 훅 목격 증거가 **인가 판정에 참여한다** — 종전엔 pid·ppid·hook_session 이 쓰이기만
    #   하고 어느 판정도 읽지 않아(순수 사후 기록), 훅 밖에서 만든 토큰과 훅 발급 토큰이 소비 시점에 구별되지 않았다.
    #   증거 없는 발급 레코드(모듈 API 직접 호출·옛 형식·증거 없이 끼워 넣은 줄)는 '발급된 적 없는 토큰'으로 본다.
    if iss.get("via") != "hook" or not _is_str(iss.get("hook_session")):
        return _result(False, "token_unknown", "훅 목격 증거(via=hook·hook_session) 없는 발급 레코드 — 인가하지 않는다",
                       **ext)
    if phase == "create":
        if st != "issued":
            return _result(False, "token_consumed", "이미 소비된 토큰(상태 %s)" % st, **ext)
    else:
        if st == "done":
            return _result(False, "token_consumed", "allow 권한도 이미 쓰였다", **ext)
        if st == "failed":
            return _result(False, "grant_revoked", "생성 실패로 settle 된 토큰 — allow 불가", **ext)
        if st != "created":
            return _result(False, "grant_not_armed", "생성 성공 기록(settle created) 없음(상태 %s)" % st, **ext)
    if iss["proposal_id"] != pid:
        return _result(False, "token_proposal_mismatch", "토큰은 다른 제안(%s)에 발급됐다" % iss["proposal_id"], **ext)
    if iss["surface"] != surf:
        return _result(False, "token_surface_mismatch", "토큰은 다른 좌석(%s)에서 발급됐다" % iss["surface"], **ext)
    if phase == "create":
        if now > float(iss["expires_at"]):
            return _result(False, "token_expired", "토큰 TTL %ds 초과(%.0fs)"
                           % (TOKEN_TTL_S, now - float(iss["expires_at"])), **ext)
    else:
        if now > float(t["settled"]["grant_expires_at"]):
            return _result(False, "grant_expired", "allow 권한 TTL %ds 초과" % ALLOW_GRANT_TTL_S, **ext)
    if iss["body_digest"] != digest:
        return _result(False, "token_body_mismatch", "승인 시점과 제안 본문이 다르다(%s… / %s…)"
                       % (iss["body_digest"][:12], digest[:12]), **ext)
    return _result(True, "verified", "유효", **ext)


def _token_op(token, proposal_id, surface, digest, phase, now, do_consume):
    now = _now(now)
    vals, bad = _token_args(token, proposal_id, surface, digest, phase)
    if bad is not None:
        return bad
    token, pid, surf, digest, phase = vals
    path = ledger_path()
    with _Locked(path):
        _a, tokens, _au = _derive(_load(path))
        res = _judge_token(tokens.get(token), pid, surf, digest, phase, now)
        if not do_consume:
            return res
        if res["ok"]:
            _append(path, [{"v": SCHEMA_VERSION, "kind": KIND_TOKEN, "event": "consumed", "token": token,
                            "phase": phase, "proposal_id": pid, "surface": surf, "body_digest": digest,
                            "consumed": True, "at": now, "pid": os.getpid()}])
            return _result(True, "consumed", "%s 단계 1회 소비" % phase,
                           **{k: res[k] for k in ("proposal_id", "surface", "phase")})
        _append(path, [{"v": SCHEMA_VERSION, "kind": KIND_AUDIT, "event": "consume_refused",
                        "code": res["code"], "detail": res["detail"][:300], "at": now,
                        "token_prefix": token[:8], "proposal_id": pid, "surface": surf, "phase": phase}])
        return res


@_guarded
def verify(token, proposal_id, surface, body_digest_hex, phase="create", now=None):
    """비소비 검증(인가 판정은 consume 과 같다). allow 단계의 '검증 → 데몬 해소 → consume' 에 쓴다."""
    return _token_op(token, proposal_id, surface, body_digest_hex, phase, now, False)


@_guarded
def consume(token, proposal_id, surface, body_digest_hex, phase="create", now=None):
    """검증 + 1회 소비(원자 · 락 안). create = 생성 직전 · allow = 데몬 해소 뒤 권한 닫기."""
    return _token_op(token, proposal_id, surface, body_digest_hex, phase, now, True)


@_guarded
def settle(token, proposal_id, surface, outcome, dept=None, code=None, now=None):
    """create 소비 뒤 생성 결과를 기록한다 — created 만 allow 권한을 무장한다(failed 는 종결)."""
    now = _now(now)
    vals, bad = _token_args(token, proposal_id, surface, None)
    if bad is not None:
        return bad
    token, pid, surf, _d, _p = vals
    if outcome not in ("created", "failed"):
        return _result(False, "bad_args", "outcome 은 created|failed")
    if outcome == "created" and not (isinstance(dept, str) and _DEPT.fullmatch(dept)):
        return _result(False, "bad_args", "created 는 부서 이름(--dept) 필수")
    if code is not None and (isinstance(code, bool) or not isinstance(code, int)):
        return _result(False, "bad_args", "code 는 정수")
    path = ledger_path()
    with _Locked(path):
        _a, tokens, _au = _derive(_load(path))
        t = tokens.get(token)
        res = None
        if t is None:
            res = _result(False, "token_unknown", "원장에 없는 토큰")
        elif t["state"] == "issued":
            res = _result(False, "not_consumed", "create 소비 전에는 settle 할 수 없다")
        elif t["state"] != "consumed":
            res = _result(False, "already_settled", "이미 결과가 기록된 토큰(상태 %s)" % t["state"])
        elif t["issued"]["proposal_id"] != pid:
            res = _result(False, "token_proposal_mismatch", "토큰은 다른 제안에 발급됐다")
        elif t["issued"]["surface"] != surf:
            res = _result(False, "token_surface_mismatch", "토큰은 다른 좌석에서 발급됐다")
        if res is not None:
            _append(path, [{"v": SCHEMA_VERSION, "kind": KIND_AUDIT, "event": "settle_refused",
                            "code": res["code"], "detail": res["detail"], "at": now,
                            "token_prefix": token[:8], "proposal_id": pid, "surface": surf}])
            return res
        rec = {"v": SCHEMA_VERSION, "kind": KIND_TOKEN, "event": "settled", "token": token,
               "outcome": outcome, "proposal_id": pid, "surface": surf, "at": now}
        if outcome == "created":
            rec.update(dept=dept, grant_expires_at=now + ALLOW_GRANT_TTL_S)
        if code is not None:
            rec["code"] = code
        _append(path, [rec])
    return _result(True, "settled", outcome, outcome=outcome, proposal_id=pid, surface=surf,
                   grant_expires_at=rec.get("grant_expires_at"))


@_guarded
def inspect(token):
    """토큰의 결박·상태 조회(인가 아님 — P5 가 제안 id 를 얻는 용도). 미지 토큰 = 거부."""
    if not isinstance(token, str) or not _HEX32.fullmatch(token):
        return _result(False, "token_missing" if not token else "token_unknown", "토큰 형식 아님")
    path = ledger_path()
    with _Locked(path):
        _a, tokens, _au = _derive(_load(path))
    t = tokens.get(token)
    if t is None:
        return _result(False, "token_unknown", "원장에 없는 토큰")
    iss = t["issued"]
    return _result(True, "inspected", "", state=t["state"], proposal_id=iss["proposal_id"],
                   surface=iss["surface"], body_digest=iss["body_digest"], expires_at=iss["expires_at"],
                   grant_expires_at=(t["settled"] or {}).get("grant_expires_at"))


# ══════════════════════════════════════════════════════════════════════════════
# status — §7-3 관측(질문 열림 · 발급 0 = 승인 미도달)
# ══════════════════════════════════════════════════════════════════════════════
@_guarded
def status(surface=None, proposal_id=None, now=None):
    now = _now(now)
    surf = _env_surface() if surface is None else _surface_key(surface)
    counts = {k: 0 for k in ("ask_opened", "ask_closed", "token_issued", "issue_refused",
                             "consumed_create", "consumed_allow", "settled")}
    if not surf:
        return _result(False, "surface_unknown", "좌석 미상")
    path = ledger_path()
    recs = []
    if os.path.exists(path):
        with _Locked(path):
            recs = _load(path)
            asks, tokens, audits = _derive(recs)
            _sync_marker(asks, now)        # 표지는 파생 캐시 — 관측 때 원장에 맞춰 되살리거나 걷는다(리뷰 F3)
    else:
        _sync_marker({}, now)
    asks, tokens, audits = _derive(recs)
    for r in recs:
        if r.get("surface") != surf:
            continue
        ev = r["event"]
        if ev == "consumed":
            counts["consumed_%s" % r["phase"]] += 1
        elif ev in counts:
            counts[ev] += 1
    mine = [a for a in asks.values() if a["rec"]["surface"] == surf
            and (proposal_id is None or a["rec"]["proposal_id"] == proposal_id)]
    ask = max(mine, key=lambda a: a["idx"]) if mine else None
    toks = [t for t in tokens.values() if t["issued"]["surface"] == surf
            and (proposal_id is None or t["issued"]["proposal_id"] == proposal_id)]
    tok = max(toks, key=lambda t: t["idx"]) if toks else None
    ask_view = None
    if ask is not None:
        ar, cl = ask["rec"], ask["closed"]
        ask_view = {"ask_id": ar["ask_id"], "proposal_id": ar["proposal_id"], "opened_at": ar["opened_at"],
                    "expires_at": ar["expires_at"], "closed": cl is not None,
                    "why": cl.get("why") if cl else None}
    tok_view = None
    if tok is not None:
        iss = tok["issued"]
        tok_view = {"token": iss["token"], "state": tok["state"], "proposal_id": iss["proposal_id"],
                    "expires_at": iss["expires_at"], "ask_id": iss["ask_id"]}
    if tok is not None and ask is not None and tok["issued"]["ask_id"] == ask["rec"]["ask_id"]:
        if tok["state"] == "issued":
            code = "token_ready" if now <= float(tok["issued"]["expires_at"]) else "token_expired"
        else:
            code = "token_used"
    elif ask is None:
        code = "ask_not_open"
    elif ask["closed"] is None:
        # ★(리뷰 m2) 답 없이 만료된 질문은 ask_expired — 종전 approval_not_received("한 번만 더 쳐 주세요")는 질문이
        #   이미 만료라 다시 쳐도 성공할 수 없었다(다시 친 답 = ask_expired). 질문을 새로 열어야 한다(디렉티브 절차 5
        #   '3 부터 다시'). 승인이 닿지 않은 경우(§7-2 턴 도중 입력)는 TTL 안에서 awaiting_answer 로 잡힌다.
        code = "awaiting_answer" if now <= float(ask["rec"]["expires_at"]) else "ask_expired"
    else:
        last = [r for r in audits if r.get("event") == "issue_refused"
                and r.get("ask_id") == ask["rec"]["ask_id"]]
        code = last[-1]["code"] if last else {"superseded": "ask_not_open",
                                              "expired": "ask_expired"}.get(ask["closed"].get("why"),
                                                                            "ask_not_open")
    return _result(True, code, "", exit_code=EXIT_OK, surface=surf, ask=ask_view, token=tok_view,
                   counts=counts, ledger=path)


# ══════════════════════════════════════════════════════════════════════════════
# CLI
# ══════════════════════════════════════════════════════════════════════════════
def _body_arg(args):
    """--body-digest / --body-b64 → (digest|None, 거부 결과|None). 둘 다 주면 일치해야 한다."""
    dig = args.body_digest or None
    if args.body_b64:
        try:
            body = base64.urlsafe_b64decode(args.body_b64.encode("ascii")).decode("utf-8")
            d2 = body_digest(body)
        except Exception as e:  # noqa: BLE001
            return None, _result(False, "bad_args", "--body-b64 판독 실패(%s)" % e)
        pid = (parse_team_body(body)[0] or {}).get("id")
        if getattr(args, "proposal", None) and pid != args.proposal:
            return None, _result(False, "bad_args", "본문의 제안 id(%s) ≠ --proposal(%s)" % (pid, args.proposal))
        if dig and dig != d2:
            return None, _result(False, "bad_args", "--body-digest 와 --body-b64 가 다르다")
        dig = d2
    return dig, None


def _emit(res):
    sys.stdout.write(json.dumps(res, ensure_ascii=False, sort_keys=True) + "\n")
    sys.stdout.flush()
    return int(res.get("exit", EXIT_INTERNAL))


def _build_parser():
    ap = argparse.ArgumentParser(prog="javis_teamtoken.py",
                                 description="대화 승인 → 1회용 팀 생성 토큰(ask·issue·verify·consume·settle)")
    sub = ap.add_subparsers(dest="cmd")
    p = sub.add_parser("ask", help="질문 열기(master)")
    p.add_argument("--proposal", default="")
    p = sub.add_parser("issue", help="UserPromptSubmit 훅 전용 발급(런처가 방금 쓴 <상태>/hook-input-<N>.json · 직접 호출 거부)")
    p.add_argument("--payload-file", default=None)
    for name in ("verify", "consume"):
        p = sub.add_parser(name)
        p.add_argument("--token", default="")
        p.add_argument("--proposal", default="")
        p.add_argument("--surface", default="")
        p.add_argument("--body-digest", default="")
        p.add_argument("--body-b64", default="")
        p.add_argument("--phase", default="create")
    p = sub.add_parser("settle")
    p.add_argument("--token", default="")
    p.add_argument("--proposal", default="")
    p.add_argument("--surface", default="")
    p.add_argument("--outcome", default="")
    p.add_argument("--dept", default=None)
    p.add_argument("--code", default=None)
    p = sub.add_parser("inspect")
    p.add_argument("--token", default="")
    p = sub.add_parser("status")
    p.add_argument("--proposal", default=None)
    sub.add_parser("path")
    sub.add_parser("messages")
    p = sub.add_parser("digest")
    p.add_argument("--body-b64", default="")
    p.add_argument("--body-file", default="")
    return ap


def main(argv=None):
    args = _build_parser().parse_args(argv)
    try:
        return _main(args)
    except Exception as e:  # noqa: BLE001 — CLI 도 fail-closed(JSON 1줄 · exit 4)
        return _emit(_result(False, "internal_error", "%s: %s" % (type(e).__name__, e)))


def _main(args):
    cmd = args.cmd
    if cmd == "ask":
        return _emit(open_ask(args.proposal))
    if cmd == "issue":
        # ★(리뷰 SEC-1) stdin 입력 경로는 없다 — 발급 입력은 런처가 방금 쓴 훅 입력 파일(--payload-file)뿐이고, 그 출처를
        #   _hook_witness 가 확인한다. stdin·임의 파일로 부르면 판정 없이 not_hook_caller(exit 1 · 질문 무소비).
        res = issue_from_hook_file(args.payload_file)
        if res.get("exit") == EXIT_NO_ASK:
            return EXIT_NO_ASK                  # 무출력 — 훅은 열린 질문이 있는 동안 매 프롬프트마다 부른다
        return _emit(res)
    if cmd in ("verify", "consume"):
        dig, bad = _body_arg(args)
        if bad is not None:
            return _emit(bad)
        fn = verify if cmd == "verify" else consume
        return _emit(fn(args.token, args.proposal, args.surface, dig or "", phase=args.phase))
    if cmd == "settle":
        code = None
        if args.code is not None:
            try:
                code = int(args.code)
            except ValueError:
                return _emit(_result(False, "bad_args", "--code 는 정수"))
        return _emit(settle(args.token, args.proposal, args.surface, args.outcome, dept=args.dept,
                            code=code))
    if cmd == "inspect":
        return _emit(inspect(args.token))
    if cmd == "status":
        return _emit(status(proposal_id=args.proposal))
    if cmd == "path":
        return _emit(_result(True, "path", "", path=ledger_path(), feed=feed_jsonl_path()))
    if cmd == "messages":
        return _emit(_result(True, "messages", "", messages={c: owner_message(c) for c in OWNER_MESSAGES},
                             section10=sorted(SECTION10_CODES), new_rows=sorted(NEW_ROW_CODES),
                             refusal_codes=list(REFUSAL_CODES), external_codes=list(EXTERNAL_CODES)))
    if cmd == "digest":
        try:
            if args.body_b64:
                body = base64.urlsafe_b64decode(args.body_b64.encode("ascii")).decode("utf-8")
            elif args.body_file:
                with open(args.body_file, encoding="utf-8") as f:
                    body = f.read()
            else:
                return _emit(_result(False, "bad_args", "--body-b64 또는 --body-file 필요"))
            return _emit(_result(True, "digest", "", body_digest=body_digest(body)))
        except Exception as e:  # noqa: BLE001
            return _emit(_result(False, "bad_args", "본문 판독 실패(%s)" % e))
    _build_parser().print_usage(sys.stderr)
    return EXIT_USAGE


if __name__ == "__main__":
    sys.exit(main())
