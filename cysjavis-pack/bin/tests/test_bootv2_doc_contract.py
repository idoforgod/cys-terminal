#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""test_bootv2_doc_contract.py — 부트 v2 문서 계약 검체 (0.14.30 W-A A5 · Phase 3-b).

무엇을 막는가(전부 **실동작** 축이다 — 문구 취향이 아니라 고장이 나는 자리다):
  ① **게이트는 있는데 지침이 없는 상태**. `javis_orchestra.py` 는 이미 각성 ACK 게이트를
     갖고 있어(`CHECK_EXIT_ACK_PENDING = 12` · `ack_gate_precheck`) ACK 가 없으면
     `review-prompt`·`round-init` 를 exit 12 로 막는다. 그런데 REVIEWER_DIRECTIVE 에 ACK
     **계산·제출 절차**가 없으면 리뷰어는 그 게이트를 통과할 방법이 없다 — 리뷰가 통째로
     영구 봉쇄된다. 게이트와 지침은 **짝**이어야 한다.
  ② **에코 ACK 위조**(시뮬 T1-7 · 치명). 판정 토큰이 디렉티브 본문에 실려 있으면 모델이
     아무것도 안 하고 **붙여넣기 에코**만 해도 `ack_source=echo` 가 참이 된다. 그래서 파생값
     `sha256(nonce)[:8]` 과 논스 원문은 본문에 **없어야** 한다 — 이 축은 **음성 방향**이다.
  ③ **유령 exit 계약**. `javis_bootstrap.py` 는 13/14/15 를 내지 않는데(코드 상수) 문서가 그
     값을 그 주체에 적으면, master 는 존재하지 않는 신호를 기다리며 처방을 뒤집는다. H-DOC-4
     가 `javis_bootstrap.py` 헤더만 스캔하고 MASTER_DIRECTIVE 는 읽지 않으므로 이 자리는
     기계 검증 밖이었다. 여기서 **주체 귀속**과 **코드 상수 대조**로 닫는다.
  ④ **사전 ACK 봉인 우회**(시뮬 T1-8). Claude 결정론 ACK 는 `cys hook`(UserPromptSubmit)이
     한다. `session-start.sh` 는 세션 시작(=주입·arm **이전**)에 돌므로 거기서 ACK 를 쏘면
     데몬이 무시하거나, 더 나쁘게는 각성 없이 각성 신고가 성립한다. 그 줄은 **없어야** 한다.
  ⑤ **CEO_TEMPLATE 손편집 드리프트**. CEO_TEMPLATE 은 100% 생성물(fragment + 구분선 +
     MASTER_DIRECTIVE 바이트 무수정)인데 손편집이 실재했다(2026-09-04 워크트리 실측).

밀폐: `tempfile.mkdtemp()` + `CYS_PACK_DIR`/`JAVIS_ROOT` env 덮어쓰기 — 라이브 팩·데몬·홈
무접촉. 읽는 것은 **이 repo 트리의 파일뿐**이고 쓰기는 임시 디렉터리의 변조 사본뿐이다.
서브프로세스 0 · 네트워크 0 · 서버 0.

  ① ACK 규약 문안 실재(게이트↔지침 짝 결속 — 코드 쪽 게이트 실재도 함께 단언)
  ② 파생값·논스 리터럴 **부재**(T1-7 · 음성 방향)
  ③ terminal 11종 등재 · exit 12/13/14/15 **주체 귀속** · 코드 상수 대조(유령 0)
  ④ session-start.sh 에 ack 줄 **부재**(T1-8 · 음성 방향)
  ⑤ CEO_TEMPLATE == 생성기 합성식 출력(바이트 등가)
  ⑥ ★음성 대조(계측 타당성) — 위 다섯 축을 각각 무력화한 **변조 사본**은 통과하지 못한다
출력: PASS/FAIL 행 · 실패 시 exit 1 · 전부 통과 시 종료 토큰 BOOTV2-DOC-CONTRACT-OK.
실행 규약(CI 동형): CYS_PACK_DIR="$(mktemp -d)" python3 bin/tests/test_bootv2_doc_contract.py
"""
import io
import os
import re
import shutil
import sys
import tempfile

SELF = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.dirname(SELF)
PACK = os.path.dirname(BIN)
REPO = os.path.dirname(PACK)
DIRECTIVES = os.path.join(PACK, "directives")
HOOKS = os.path.join(PACK, "hooks")
SCRIPTS = os.path.join(REPO, "scripts")

REVIEWER_MD = os.path.join(DIRECTIVES, "REVIEWER_DIRECTIVE.md")
MASTER_MD = os.path.join(DIRECTIVES, "MASTER_DIRECTIVE.md")
CEO_MD = os.path.join(DIRECTIVES, "CEO_TEMPLATE.md")
FRAGMENT = os.path.join(SCRIPTS, "ceo_template_header.md")
SESSION_START = os.path.join(HOOKS, "session-start.sh")
MANUAL_MD = os.path.join(REPO, "USER-MANUAL.md")
CYS_RS = os.path.join(REPO, "src", "bin", "cys.rs")
ORCHESTRA_PY = os.path.join(BIN, "javis_orchestra.py")
BOOTSTRAP_PY = os.path.join(BIN, "javis_bootstrap.py")

fails = []


def check(name, cond, detail=""):
    """PASS/FAIL 행 1줄. detail 은 **실패했을 때만** 붙인다 — 통과 행에 실패 사유 문구가
    함께 찍히면 보고서를 읽는 사람이 통과를 실패로 오독한다(A5 R1 자기교정)."""
    print("%s %s%s" % ("PASS" if cond else "FAIL", name,
                       (" — " + detail) if (detail and not cond) else ""))
    if not cond:
        fails.append(name)


def read(path):
    with io.open(path, encoding="utf-8") as f:
        return f.read()


def read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


# ─────────────────────── 순수 판정 함수 (텍스트 → 실패 목록) ───────────────────────
# ★전부 "텍스트를 받아 실패 목록을 낸다" 형태다. 그래야 같은 함수로 **원본과 변조 사본을
#   똑같이** 잴 수 있다(음성 대조의 전제 — 검사자가 둘로 갈리면 계측 타당성이 사라진다).

# ACK 절차의 **실행 가능성** 요소. 문구 취향이 아니라 "리뷰어가 이 줄만 보고 값을 만들 수
# 있는가"의 최소 집합이다: 형식 · 해시 도구(양 플랫폼) · 8자 절단 · 신고 플래그.
ACK_PROCEDURE_TOKENS = [
    ("ACK <sha256(", "판정 토큰 형식(파생값 규약)"),
    ("[:8]", "8자 절단 규약"),
    ("shasum -a 256", "macOS 해시 도구"),
    ("sha256sum", "Linux 해시 도구"),
    ("cut -c1-8", "8자 절단 실행형"),
    ("set-status --ack", "데몬 신고 경로"),
    ("CYS_BOOT_NONCE", "논스 수신 경로"),
]

# spec §3-2 terminal 대수 — 11종이 전부다.
TERMINAL_KINDS = [
    "completed", "completed_degraded", "declined", "session_error", "aborted",
    "crashed", "superseded", "expired", "attempts_exhausted", "skipped_inflight",
    "state_unreadable",
]

# exit → 그 값을 **소유하는 주체**(문서가 그 주체의 하위 불릿에 적어야 하는 값).
EXIT_OWNER = {
    "**12 ack_pending**": "javis_orchestra.py check",
    "**13 aborted**": "cys boot-run",
    "**14 completed_degraded**": "cys boot-run",
    "**15 crashed**": "cys boot-run",
}


def ack_procedure_missing(reviewer_text):
    """① ACK 절차 실재 — 부재 토큰 목록을 낸다(빈 목록 = 통과)."""
    return [lbl for tok, lbl in ACK_PROCEDURE_TOKENS if tok not in reviewer_text]


def forgeable_literals(reviewer_text):
    """② 위조 가능 리터럴 — 발견 목록을 낸다(빈 목록 = 통과 · **음성 방향**).

    ⓐ `ACK <8자 16진>` 완성형 = 붙여넣기만으로 참이 되는 판정 토큰 그 자체.
    ⓑ 16자 이상 연속 16진 = 논스 원문 후보(난수를 본문에 실으면 파생값 계산이 공짜가 된다).
    ⓒ `CYS_BOOT_NONCE=<값>` 대입 = 논스를 문서가 고정하는 형태."""
    found = []
    for m in re.finditer(r"\bACK\s+([0-9a-fA-F]{8})\b", reviewer_text):
        found.append("ACK 완성형 리터럴: %r" % m.group(0))
    for m in re.finditer(r"\b[0-9a-fA-F]{16,}\b", reviewer_text):
        found.append("논스 원문 후보(16진 %d자): %r" % (len(m.group(0)), m.group(0)[:24]))
    for m in re.finditer(r"CYS_BOOT_NONCE\s*=\s*\S+", reviewer_text):
        found.append("논스 대입 리터럴: %r" % m.group(0))
    return found


def section_0a(master_text):
    """MASTER_DIRECTIVE §0-A 본문(§0-B 직전까지). 못 찾으면 None."""
    m = re.search(r"### 0-A\..*?(?=\n### 0-B\.)", master_text, re.S)
    return m.group(0) if m else None


def sub_bullets(section_text):
    """§0-A 안의 **2단 불릿**(2칸 이상 들여쓴 `- `)을 (주체, 전문)으로 쪼갠다.

    주체 = 그 불릿 첫 줄의 첫 백틱 토큰. 계속 줄은 다음 2단 불릿/1단 불릿/표/빈 줄 전까지."""
    out = []
    cur = None
    for line in section_text.split("\n"):
        if re.match(r"^ {2,}- ", line):
            subj = re.search(r"`([^`]+)`", line)
            cur = [subj.group(1) if subj else "", [line]]
            out.append(cur)
        elif cur is not None and (re.match(r"^- ", line) or line.startswith("|")
                                  or line.strip() == ""):
            cur = None
        elif cur is not None:
            cur[1].append(line)
    return [(subj, "\n".join(body)) for subj, body in out]


def terminal_kinds_missing(master_text):
    """③a terminal 11종 등재 — 부재 목록(빈 목록 = 통과). 백틱 인용만 인정한다."""
    return [k for k in TERMINAL_KINDS if ("`%s`" % k) not in master_text]


def setstatus_has_ack(cys_rs_text):
    """`cys set-status --ack` 가 **코드에 실재하는가**(clap `SetStatus` 블록 안의 필드).

    ★파일 전역이 아니라 블록으로 좁히는 이유: `ack` 는 이 리포에서 흔한 낱말이라 전역 검색은
      항상 참이 된다(게이트가 상수로 퇴화). 판정 범위는 `SetStatus {` 부터 그 블록의 닫는 `},` 까지다.
    반환 True(실재) · False(부재) · **None(판정 불가 — 소스 부재·블록 미검출)**.
    측정 불능을 False 로 접지 않는 이유는 이 팩의 exit 2 계약과 같다."""
    if not cys_rs_text:
        return None
    i = cys_rs_text.find("\n    SetStatus {")
    if i < 0:
        return None
    j = cys_rs_text.find("\n    },", i)
    if j < 0:
        return None
    block = cys_rs_text[i:j]
    return ("ack:" in block) or ("--ack" in block)


def exit_owner_violations(master_text):
    """③b exit 주체 귀속 — 위반 목록(빈 목록 = 통과).

    각 exit 토큰은 §0-A 안에 정확히 1회 등장해야 하고, 그 토큰을 담은 2단 불릿의 주체가
    기대 주체여야 한다. 값이 남의 주체 밑으로 옮겨가면(유령 계약) 여기서 잡힌다."""
    bad = []
    sec = section_0a(master_text)
    if sec is None:
        return ["§0-A 절을 찾지 못했다(구조 붕괴)"]
    bullets = sub_bullets(sec)
    for token, owner in EXIT_OWNER.items():
        holders = [subj for subj, body in bullets if token in body]
        if not holders:
            bad.append("%s 미등재" % token)
        elif len(holders) > 1:
            bad.append("%s 가 여러 주체에 중복(%s)" % (token, holders))
        elif holders[0] != owner:
            bad.append("%s 의 주체가 %r 여야 하는데 %r 다" % (token, owner, holders[0]))
    return bad


def _int_consts(source_text, prefix):
    """소스에서 `<PREFIX>NAME = <정수>` 상수 값 집합을 뽑는다(사본 금지 — 코드가 SOT)."""
    return set(int(v) for v in
               re.findall(r"^%s[A-Z_]+ = (\d+)" % re.escape(prefix), source_text, re.M))


def ghost_exit_violations(bootstrap_src, orchestra_src):
    """③c 코드 상수 대조 — 위반 목록(빈 목록 = 통과).

    ⓐ 문서가 "javis_bootstrap.py 는 13·14·15 를 내지 않는다"고 단언하므로, 코드에 그 값이
      생기면 **문서가 거짓이 된 순간** 적색이어야 한다(양방향 결박).
    ⓑ 문서가 check 의 12 를 계약으로 적으므로, 코드에 12 가 실재해야 한다(유령 반대 방향)."""
    bad = []
    boot = _int_consts(bootstrap_src, "EXIT_")
    orc = _int_consts(orchestra_src, "CHECK_EXIT_")
    for v in (13, 14, 15):
        if v in boot:
            bad.append("javis_bootstrap.py 에 exit %d 상수가 생겼다 — §0-A 의 '내지 않는다' "
                       "단언이 거짓이 됐다(문서 갱신 필요)" % v)
    if 12 not in orc:
        bad.append("javis_orchestra.py 에 CHECK_EXIT_* 12 가 없다 — §0-A 의 ack_pending "
                   "계약이 유령이다")
    if not boot:
        bad.append("javis_bootstrap.py EXIT_* 상수를 하나도 못 읽었다(측정 실패)")
    return bad


ACK_LINE_RE = re.compile(
    r"(^|[^A-Za-z0-9._-])(--ack|CYS_BOOT_NONCE|set-status)([^A-Za-z0-9._-]|$)")


def session_start_ack_lines(hook_text):
    """④ session-start.sh 의 ack 줄 — 발견 목록(빈 목록 = 통과 · **음성 방향** · T1-8).

    낱말 경계를 쓴다: `pack`·`.pack-accepted.json` 같은 무관한 부분일치를 ack 로 세면
    검체가 늘 빨개져 무의미해진다."""
    return ["%d: %s" % (i, l.strip()[:100])
            for i, l in enumerate(hook_text.split("\n"), 1) if ACK_LINE_RE.search(l)]


def ceo_drift(fragment_bytes, master_bytes, ceo_bytes, separator):
    """⑤ CEO_TEMPLATE 드리프트 — 위반 목록(빈 목록 = 통과). 합성식은 생성기와 동형."""
    frag = fragment_bytes if fragment_bytes.endswith(b"\n") else fragment_bytes + b"\n"
    want = frag + separator + master_bytes
    if want != ceo_bytes:
        return ["재합성 %dB != 파일 %dB (손편집·재생성 누락)" % (len(want), len(ceo_bytes))]
    return []


root = tempfile.mkdtemp()
try:
    # 밀폐: 라이브 팩·라운드 무접촉(이 검체는 repo 트리만 읽지만 계약을 지킨다)
    os.environ["CYS_PACK_DIR"] = root
    os.environ["JAVIS_ROOT"] = root
    for k in ("JAVIS_PACK_DIR", "AITERM_PACK_DIR", "AITERM_JARVIS_DIR"):
        os.environ.pop(k, None)

    reviewer = read(REVIEWER_MD)
    master = read(MASTER_MD)
    hook = read(SESSION_START)
    orchestra_src = read(ORCHESTRA_PY)
    bootstrap_src = read(BOOTSTRAP_PY)

    # ─────────── ① ACK 규약 문안 실재 (게이트↔지침 짝 결속) ───────────
    check("1a 코드 쪽 ACK 게이트 실재(짝의 반대편)",
          "CHECK_EXIT_ACK_PENDING = 12" in orchestra_src
          and "ack_gate_precheck" in orchestra_src,
          "게이트가 없으면 이 검체의 전제가 사라진다")
    miss = ack_procedure_missing(reviewer)
    check("1b REVIEWER §1-1 ACK 절차 실재", not miss, "부재: %s" % miss)
    check("1c ACK 절이 §2 앵커보다 앞(추출 정규식 무해)",
          "## 1-1." in reviewer
          and reviewer.index("## 1-1.") < reviewer.index("## 2. 엄격 제약"))
    # ★1d 는 **양방향 결박**이다(R2 적대검증 MAJOR 반영). 종전 판정은 "'미착지' 문자열이 있는가"
    #   한 방향뿐이라 W-B B5 가 `--ack` 를 착지시키면 두 갈래 모두 틀린 신호를 냈다 —
    #   ⓐ정직한 편집자가 낡은 경고를 지우면(문서가 옳아진 순간) FAIL 하고
    #   ⓑ경고를 방치해 문서가 거짓이 되면 그대로 PASS 한다.
    #   그래서 **코드의 실재 여부**를 읽어 문서 주장과 대조한다(같은 파일 3c 가 이미 쓰는 규약).
    _ack_in_code = setstatus_has_ack(read(CYS_RS))
    _doc_says_missing = ("아직 없다" in reviewer) or ("미착지" in reviewer)
    if _ack_in_code is None:
        check("1d `--ack` 문서↔코드 결박(판정 불가 — Rust 소스 부재)", True,
              "측정 불능은 실패가 아니다(배포 팩 경로)")
    elif _ack_in_code:
        check("1d 코드에 `--ack` 가 착지했으면 문서의 '미착지' 경고는 **사라져야** 한다",
              not _doc_says_missing, "B5 착지 후에도 문서가 '없다'고 말한다 — 거짓 서술")
    else:
        check("1d 코드에 `--ack` 가 없으면 문서가 그 사실을 **병기해야** 한다",
              _doc_says_missing, "현행 CLI 실측 고지 부재 — 거짓 처방")

    # ─────────── ② 파생값·논스 리터럴 부재 (T1-7 · 음성 방향) ───────────
    forged = forgeable_literals(reviewer)
    check("2 위조 가능 리터럴 0(파생값·논스 미기재)", not forged, "발견: %s" % forged[:3])

    # ─────────── ③ terminal 11종 · exit 주체 귀속 · 코드 대조 ───────────
    tmiss = terminal_kinds_missing(master)
    check("3a terminal 대수 11종 전수 등재", not tmiss, "부재: %s" % tmiss)
    owner_bad = exit_owner_violations(master)
    check("3b exit 12/13/14/15 주체 귀속 정확(유령 계약 0)", not owner_bad,
          "위반: %s" % owner_bad)
    ghost = ghost_exit_violations(bootstrap_src, orchestra_src)
    check("3c 코드 상수 대조(bootstrap 13/14/15 부재 · check 12 실재)", not ghost,
          "위반: %s" % ghost)

    # ─────────── ④ USER-MANUAL 교차 결박 (R2 적대검증 MAJOR 반영) ───────────
    # ★USER-MANUAL 이 terminal 11종 enum 을 **통째로 복제**하는데 3a 는 MASTER 쪽만 핀했다.
    #   enum 이 바뀌면 MASTER 는 적색으로 잡히고 매뉴얼은 **조용히 드리프트**해 사용자에게
    #   존재하지 않는 kind 를 계속 안내한다(사본은 낡는다 — 이 리포의 반복 형상).
    manual = read(MANUAL_MD)
    check("4a USER-MANUAL 이 부트 v2 절을 담는다(티켓 4항 산출물의 기계 검증)",
          bool(manual) and "completed_degraded" in manual, "매뉴얼에 부트 v2 서술이 없다")
    _mmiss = [k for k in TERMINAL_KINDS if k not in manual] if manual else TERMINAL_KINDS
    check("4b 매뉴얼의 terminal 열거가 MASTER 와 **같은 11종**(사본 드리프트 차단)",
          not _mmiss, "매뉴얼 부재: %s" % _mmiss)
    check("4c 매뉴얼에도 위조 가능 리터럴 0(T1-7 동일 규율)",
          not forgeable_literals(manual or ""), "발견: %s" % forgeable_literals(manual or "")[:3])

    # ─────────── ④ session-start.sh ack 줄 부재 (T1-8 · 음성 방향) ───────────
    ack_lines = session_start_ack_lines(hook)
    check("4 session-start.sh 에 ack 줄 0(추가하지 않음 계약)", not ack_lines,
          "발견: %s" % ack_lines[:3])

    # ─────────── ⑤ CEO_TEMPLATE == 생성기 출력 ───────────
    if os.path.exists(FRAGMENT):
        sys.path.insert(0, SCRIPTS)
        import gen_ceo_template as gen          # 합성식 SOT(사본 금지)
        drift = ceo_drift(read_bytes(FRAGMENT), read_bytes(MASTER_MD),
                          read_bytes(CEO_MD), gen.SEPARATOR)
        check("5 CEO_TEMPLATE 이 생성기 합성식과 바이트 등가", not drift, "%s" % drift)
    else:
        print("SKIP 5 CEO_TEMPLATE 합성 대조 — fragment 부재(배포 팩 트리 · repo 전용 축)")
        gen = None

    # ─────────── ⑥ ★음성 대조(계측 타당성) ───────────
    # 각 축을 무력화한 변조 사본이 **같은 판정 함수**에서 실패해야 한다.
    m_rev = re.sub(r"## 1-1\..*?(?=## 2\. 엄격 제약)", "", reviewer, flags=re.S)
    check("6a 변조① §1-1 삭제본은 ACK 절차 축을 통과 못 함",
          bool(ack_procedure_missing(m_rev)), "변조본이 통과했다(계측 무효)")

    m_forge = reviewer.replace("직접 계산한 **8자 소문자 16진값**",
                               "직접 계산한 값(예: ACK deadbeef)", 1)
    check("6b 변조② 파생값 리터럴 주입본은 위조 축을 통과 못 함",
          m_forge != reviewer and bool(forgeable_literals(m_forge)),
          "변조본이 통과했다(계측 무효)")

    m_terminal = re.sub(r"^\| `superseded` \|.*\n", "", master, flags=re.M)
    check("6c 변조③a terminal 행 삭제본은 11종 축을 통과 못 함",
          m_terminal != master and "superseded" in terminal_kinds_missing(m_terminal),
          "변조본이 통과했다(계측 무효)")

    # 13 을 boot-run 불릿에서 떼어 bootstrap 불릿으로 옮긴다(= 유령 계약 재현)
    m_ghost = master.replace("**13 aborted** · ", "", 1)
    m_ghost = m_ghost.replace("`javis_bootstrap.py`(현행 폴백 경로) — 0·3·4·5·6·7·8·9·10·11·64.",
                              "`javis_bootstrap.py`(현행 폴백 경로) — 0·3·4·5·6·7·8·9·10·11·64 · "
                              "**13 aborted**.", 1)
    check("6d 변조③b 주체 뒤바꾼 본은 귀속 축을 통과 못 함",
          m_ghost != master and bool(exit_owner_violations(m_ghost)),
          "변조본이 통과했다(계측 무효)")

    m_boot_src = bootstrap_src + "\nEXIT_ABORTED = 13\n"
    check("6e 변조③c bootstrap 에 13 상수를 넣으면 코드 대조 축이 적색",
          bool(ghost_exit_violations(m_boot_src, orchestra_src)),
          "변조본이 통과했다(계측 무효)")

    m_hook = hook + '\ncys set-status --ack "$CYS_BOOT_NONCE" 2>/dev/null || true\n'
    check("6f 변조④ ack 줄 주입본은 부재 축을 통과 못 함",
          bool(session_start_ack_lines(m_hook)), "변조본이 통과했다(계측 무효)")

    if gen is not None:
        check("6g 변조⑤ CEO 1바이트 추가본은 드리프트 축을 통과 못 함",
              bool(ceo_drift(read_bytes(FRAGMENT), read_bytes(MASTER_MD),
                             read_bytes(CEO_MD) + b"x", gen.SEPARATOR)),
              "변조본이 통과했다(계측 무효)")
    # ★변조⑥⑦ — 이번 라운드 신설 두 축(1d 양방향 · 4b 매뉴얼 교차)이 실제로 재고 있는가.
    _rs = read(CYS_RS)
    if _rs:
        _mut_rs = _rs.replace("\n    SetStatus {",
                              "\n    SetStatus {\n        #[arg(long)]\n        ack: Option<String>,", 1)
        check("6h 변조⑥ 앵커 실재(SetStatus 블록)", _mut_rs != _rs)
        check("6i 변조⑥ 코드에 --ack 를 넣으면 1d 가 **방향을 뒤집는다**",
              setstatus_has_ack(_mut_rs) is True and setstatus_has_ack(_rs) is False,
              "양방향 결박이 아니면 두 값이 같다: %r/%r"
              % (setstatus_has_ack(_mut_rs), setstatus_has_ack(_rs)))
    if manual:
        _mut_manual = manual.replace("attempts_exhausted", "attempts_gone", 1)
        check("6j 변조⑦ 앵커 실재(매뉴얼 enum)", _mut_manual != manual)
        check("6k 변조⑦ 매뉴얼에서 kind 1종을 바꾸면 교차 결박이 잡는다",
              [k for k in TERMINAL_KINDS if k not in _mut_manual] == ["attempts_exhausted"],
              "매뉴얼 드리프트가 무측정이면 이 축은 장식이다")

finally:
    shutil.rmtree(root, ignore_errors=True)

if fails:
    print("\n%d FAIL: %s" % (len(fails), ", ".join(fails)))
    sys.exit(1)
print("\nALL PASS")
print("BOOTV2-DOC-CONTRACT-OK")
sys.exit(0)
