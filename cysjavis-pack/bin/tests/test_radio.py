#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_radio.py 게이트별 단위 테스트 (RADIO_SPEC_v4 · 2026-08-13 · R2 수리분 반영).

배포 제외: 경로 컴포넌트 `tests` 는 build.rs 가 임베드 대상에서 제외한다(build.rs:56-59).

설계 원칙
  · **임시 디렉터리 격리** — 케이스마다 `JAVIS_ROOT` 를 새 tmpdir 로 갈아끼운다. 실장부
    (`_round/radio/`)에는 한 바이트도 쓰지 않는다.
  · **데몬 비의존** — `CYS_RADIO_DISABLE_CYS=1` 로 cys 통로를 끈다. 네트워크 0.
  · **게이트당 1케이스** — '기능이 돈다'가 아니라 '위반이 정확한 exit 로 막힌다'를 잠근다.
    통과 경로만 확인하는 테스트는 게이트가 통째로 제거돼도 초록이라 쓸모가 없다.
  · **수리마다 그 수리를 잠그는 테스트** — 회귀는 '고쳤다'가 아니라 '다시 깨지면 빨개진다'
    로만 방지된다. 로테이션 소실·펜스 격리·pause 보류가 여기에 해당한다.

실행: python3 cysjavis-pack/bin/tests/test_radio.py   (0 = 전건 PASS)
"""
import contextlib
import io
import json
import os
import shutil
import sys
import tempfile

# 경로 가드 — 형태 규약은 javis_wakeup.py:47-57 과 동일(append + 중복 검사 · insert(0) 금지).
_BIN = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
if _BIN not in sys.path:
    sys.path.append(_BIN)

import javis_radio as R  # noqa: E402

_results = []
TH = R.DEFAULT_THREAD


def check(name, cond, detail=""):
    _results.append((name, bool(cond), detail))
    if not cond:
        sys.stderr.write("FAIL %s%s\n" % (name, (" — " + str(detail)) if detail else ""))


class Lane(object):
    """케이스 1개의 밀폐 레인 — tmpdir + env 복원."""

    def __init__(self):
        self.tmp = None
        self._bak = {}

    def __enter__(self):
        self.tmp = tempfile.mkdtemp(prefix="radio-t-")
        for k, v in (("JAVIS_ROOT", self.tmp), ("CYS_RADIO_DISABLE_CYS", "1")):
            self._bak[k] = os.environ.get(k)
            os.environ[k] = v
        return self

    def __exit__(self, *exc):
        for k, v in self._bak.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)
        return False

    # ── 조작 ──
    def run(self, argv):
        """(exit code, stdout). 게이트 판정은 **exit code 가 사실이다**."""
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = R.main(argv)
        return rc, buf.getvalue()

    def open(self, ticket="T1", participants="w1,w2"):
        # A1-R7 — 무결성 서명 영수증. 종전엔 raw {"ok":True} 를 썼으나 능력 게이트가 서명을
        #   검증하게 강화됐으므로 정식 발급 경로(write_capability_receipt)로 발급한다.
        for n in participants.split(","):
            R.write_capability_receipt(n.strip())
        return self.run(["open", "--ticket", ticket, "--participants", participants])

    def probe_file(self, body="첫줄\n표식-ALPHA\n"):
        p = os.path.join(self.tmp, "probe.txt")
        with open(p, "w", encoding="utf-8") as f:
            f.write(body)
        return p

    def send(self, node="w1", grade="NORMAL", epistemic="HYPOTHESIS", text="본문",
             ticket="T1", **kw):
        argv = ["send", "--ticket", ticket, "--node", node, "--grade", grade,
                "--epistemic", epistemic, "--text", text]
        if epistemic == "HYPOTHESIS" and "confidence" not in kw:
            kw["confidence"] = "low"
        for k, v in kw.items():
            flag = "--" + k.replace("_", "-")
            if isinstance(v, list):
                for item in v:
                    argv += [flag, item]
            else:
                argv += [flag, str(v)]
        return self.run(argv)

    def recs(self, ticket="T1", thread=None):
        return R.read_thread(ticket, thread or TH)[0]

    def drain(self, ticket="T1", nodes=("w1", "w2")):
        """close (iv) 드레인 게이트를 정상 경로로 통과시킨다 — 각 워커가 델타를 표면화한다."""
        for n in nodes:
            self.run(["wait", "--ticket", ticket, "--node", n, "--once", "--interval", "0"])

    def pause_on(self):
        d = os.path.join(self.tmp, "_round")
        os.makedirs(d, exist_ok=True)
        p = os.path.join(d, R.javis_wakeup.PAUSED_BASENAME)
        open(p, "w").close()
        return p

    def pause_off(self):
        p = os.path.join(self.tmp, "_round", R.javis_wakeup.PAUSED_BASENAME)
        if os.path.exists(p):
            os.unlink(p)


# ════════════════════════ send 게이트 ════════════════════════
def t_open_reviewer_rejected():
    with Lane() as L:
        R.javis_lock.atomic_write_json(R.capability_receipt("reviewer-codex"), {"ok": True})
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1,reviewer-codex"])
        check("open: 리뷰어 참여자 거부 exit 6", rc == R.EXIT_REVIEWER, rc)


def t_open_capability_gate():
    with Lane() as L:
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("open: 능력 영수증 부재 exit 3(fail-closed)", rc == R.EXIT_CAPABILITY, rc)
        R.write_capability_receipt("w1", ok=False)
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("open: 실패 영수증도 거부", rc == R.EXIT_CAPABILITY, rc)
        # ★A1-R7 — 서명 없는 위조 {"ok":true} 는 통과하지 못한다(무결성 검증 강화)
        R.javis_lock.atomic_write_json(R.capability_receipt("w1"), {"ok": True})
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("★A1-R7: 서명 없는 위조 영수증 거부", rc == R.EXIT_CAPABILITY, rc)
        R.write_capability_receipt("w1")
        rc, out = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("open: 무결성 서명 통과 영수증이면 개통", rc == R.EXIT_OK, rc)
        check("스레드 2종 = worklog·results(§2.5 명명 정합)",
              json.loads(out)["threads"] == ["worklog", "results"], out)


def t_thread_names_fixed():
    check("스레드: 2종 수량 불변식", len(R.THREADS) == 2, R.THREADS)
    check("스레드: worklog(발견)·results(결과) 명명", R.THREADS == ("worklog", "results"), R.THREADS)
    check("스레드: 'master 스레드' 부재(A10(c))", "master" not in R.THREADS)


def t_fact_demotion():
    with Lane() as L:
        L.open()
        rc, _ = L.send(epistemic="FACT", text="허위", evidence="nope.py:1:zzz", grade="NORMAL")
        check("강등: send 는 성공한다(강등≠거부)", rc == R.EXIT_OK, rc)
        r = L.recs()[-1]
        check("강등: epistemic:=HYPOTHESIS", r.get("epistemic") == "HYPOTHESIS", r.get("epistemic"))
        check("강등: confidence:=UNVERIFIED", r.get("confidence") == "UNVERIFIED", r.get("confidence"))
        check("강등: demoted_from=FACT", r.get("demoted_from") == "FACT", r.get("demoted_from"))
        check("강등: [DEMOTED:...] 접두 의무", (r.get("text") or "").startswith("[DEMOTED:"), r.get("text"))
        check("강등: grade 는 불변(§3.8 — epistemic 과 배달특권 분리)",
              r.get("grade") == "NORMAL", r.get("grade"))


def t_fact_without_evidence_rejected():
    """§3.2 — 'exit 5 거부는 evidence 자체 부재 시만'. 무증거 FACT 를 강등 저장으로 받으면
    근거 없는 사실 주장이 스레드에 상시 축적된다."""
    with Lane() as L:
        L.open()
        rc, _ = L.send(epistemic="FACT", text="근거 없는 사실 주장")
        check("FACT: --evidence 부재는 exit 5 거부(강등 저장 금지)", rc == R.EXIT_NO_EVIDENCE, rc)
        check("FACT: 거부 건은 스레드에 미기록", len(L.recs()) == 0, len(L.recs()))


def t_question_epistemic():
    """§3.1 기본 열거값 — 어떤 수정 조문도 QUESTION 을 삭제하지 않았다."""
    with Lane() as L:
        L.open()
        rc, _ = L.send(epistemic="QUESTION", text="이 경로가 의도된 것인가?")
        check("epistemic: QUESTION 송신 가능", rc == R.EXIT_OK, rc)
        check("epistemic: QUESTION 은 confidence·evidence 불요",
              L.recs()[-1].get("epistemic") == "QUESTION", L.recs()[-1])
        check("전방호환: QUESTION 은 미지 레코드가 아니다",
              not R.is_unknown({"epistemic": "QUESTION"}))


def t_fact_verified():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        rc, _ = L.send(epistemic="FACT", text="참", evidence="%s:2:표식-ALPHA" % p)
        r = L.recs()[-1]
        check("검증: 진짜 근거는 FACT 유지", rc == R.EXIT_OK and r.get("epistemic") == "FACT", r)
        check("검증: verified=true", r.get("verified") is True, r.get("verified"))
        check("검증: line_hash 저장(사후 재검증 입력)",
              bool((r.get("evidence") or [{}])[0].get("line_hash")))
        ok, _why = R.recheck_evidence(r["evidence"])
        check("재검증: 무변조 파일은 통과", ok)
        with open(p, "w", encoding="utf-8") as f:
            f.write("첫줄\n변조됨\n")
        ok, why = R.recheck_evidence(r["evidence"])
        check("재검증: 사후 변조 탐지", not ok, why)
        # 라인 번호는 맞지만 스니펫이 그 라인에 없으면 불일치로 잡아야 한다
        vok, _w, _e = R.verify_evidence([{"file": p, "line": 1, "snippet": "표식-ALPHA"}])
        check("검증: 라인 어긋난 스니펫 불일치 탐지", not vok)


def t_blocker_evidence_and_reason():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        rc, _ = L.send(node="w2", grade="BLOCKER", epistemic="FACT", text="막힘",
                       reason="빌드가 완전히 깨져 진행 불가")
        check("BLOCKER: evidence 부재 exit 5", rc == R.EXIT_NO_EVIDENCE, rc)
        rc, _ = L.send(node="w2", grade="BLOCKER", epistemic="FACT", text="막힘",
                       reason="짧음", evidence="%s:2:표식-ALPHA" % p)
        check("BLOCKER: 사유 하한(8자) exit 5", rc == R.EXIT_NO_EVIDENCE, rc)


def t_blocker_bad_evidence_grade_demotion():
    with Lane() as L:
        L.open()
        rc, _ = L.send(node="w2", grade="BLOCKER", epistemic="FACT", text="가짜 막힘",
                       reason="근거 없는 최고 등급 시도", evidence="nope.py:1:zzz")
        r = L.recs()[-1]
        check("BLOCKER ⓪진위: 정지가 아니라 강등(exit 0)", rc == R.EXIT_OK, rc)
        check("BLOCKER ⓪진위: grade→URGENT (stdin 특권 박탈)",
              r.get("grade") == "URGENT", r.get("grade"))
        check("BLOCKER ⓪진위: 수신자 stdin 배달 대장에 기록 없음",
              not os.path.exists(R.delivery_ledger("T1", "w1")))
        # ★AA27 — 강등돼도 master 사본은 **강등 사유를 동봉해** 유지된다(남용 즉시 가시화)
        rows, _ = R._jsonl_read(R._tp("T1", ".master-copy-log"))
        check("AA27: 강등 BLOCKER 도 master 사본 유지", len(rows) == 1, rows)
        check("AA27: 사본에 강등 사유 동봉",
              bool(rows and rows[0].get("demotion_reason")), rows)


def t_blocker_cooldown_clock():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        kw = dict(node="w2", grade="BLOCKER", epistemic="FACT", text="진짜 막힘",
                  reason="빌드가 완전히 깨져 진행 불가", evidence="%s:2:표식-ALPHA" % p)
        rc, _ = L.send(**kw)
        check("BLOCKER: 정상 송신", rc == R.EXIT_OK, rc)
        before = (R._read_json(R.cooldown_path("T1", "w2")) or {}).get("BLOCKER")
        rc, _ = L.send(**kw)
        check("BLOCKER: 쿨다운 위반 exit 8(일시 거부)", rc == R.EXIT_THROTTLED, rc)
        after = (R._read_json(R.cooldown_path("T1", "w2")) or {}).get("BLOCKER")
        check("BLOCKER: 거부 건은 시계 미소모(A26)", before == after, (before, after))
        n_before = len(L.recs())
        check("BLOCKER: 거부는 무언 큐잉·강등이 아니라 미기록", n_before == 1, n_before)


def t_urgent_cooldown_independent():
    with Lane() as L:
        L.open()
        rc1, _ = L.send(node="w2", grade="URGENT", text="긴급1")
        rc2, _ = L.send(node="w2", grade="URGENT", text="긴급2")
        check("URGENT: 2분 쿨다운 발동 exit 8", rc1 == R.EXIT_OK and rc2 == R.EXIT_THROTTLED,
              (rc1, rc2))
        rc3, _ = L.send(node="w2", grade="NORMAL", text="보통")
        check("URGENT: NORMAL 은 쿨다운 무관", rc3 == R.EXIT_OK, rc3)


def t_breaker_global():
    with Lane() as L:
        # burst 는 발신자이므로 참여자로 등록한다(신원 게이트 A1-R3 강화 — 비참여자 send 거부).
        L.open("T1", "w1,w2,burst")
        L.open("T2", "w1,w2,burst")
        rcs = [L.send(node="burst", grade="FYI", text="x")[0] for _ in range(R.BREAKER_MAX)]
        check("차단기: 상한까지는 통과", all(r == R.EXIT_OK for r in rcs), rcs)
        rc, _ = L.send(node="burst", grade="FYI", text="x")
        check("차단기: 상한 초과 exit 8", rc == R.EXIT_THROTTLED, rc)
        # ★전역 스코프 — 티켓을 갈아타도 우회되지 않는다(A35(d))
        rc, _ = L.send(node="burst", grade="FYI", text="x", ticket="T2")
        check("차단기: 티켓 전환으로 우회 불가(발신자 전역)", rc == R.EXIT_THROTTLED, rc)


def t_scrub_applied():
    with Lane() as L:
        L.open()
        L.send(text="열쇠는 api_key=SUPERSECRETVALUE123 이다")
        r = L.recs()[-1]
        check("scrub: 저장 전 마스킹", "SUPERSECRETVALUE123" not in json.dumps(r, ensure_ascii=False), r)
        rc, out = L.run(["read", "--ticket", "T1"])
        check("scrub: read-side 2차 통과", "SUPERSECRETVALUE123" not in out)


def t_stdin_leg_injected_vs_enqueued():
    """§3.11(c)(d) — '기배달' 은 state=INJECTED 기록이 있을 때만 참이다(추정 금지).
    미주입(ENQUEUED)이면 전문 동승 + 취소 요청 대장 기록(취소 불능은 감사 가능한 예외)."""
    with Lane() as L:
        L.open()
        p = L.probe_file()
        bak_send, bak_probe = R.cys_send_queued, R.probe_injected
        R.cys_send_queued = lambda target, text: (True, "enqueued")
        try:
            R.probe_injected = lambda target: True          # 주입 확인 성공
            L.send(node="w1", grade="BLOCKER", epistemic="FACT", text="막힘1", to="w2",
                   reason="배달 대장 시험용 사유", evidence="%s:2:표식-ALPHA" % p)
            st = R.delivery_state("T1", "w2")
            check("배달 대장: 주입 확인 시 INJECTED 기록",
                  st.get(1, {}).get("recipient") == "INJECTED", st)
            recs, _ = R.read_thread("T1", TH)
            payload, _d, _s, _h = R.build_delta("T1", "w2", TH, 0, recs, R.read_meta("T1"))
            check("중복 종결: INJECTED 는 '기배달(stdin)' 1줄 압축",
                  "기배달(stdin) seq=1" in payload, payload)

            R.probe_injected = lambda target: False         # 조회 불능 = 미주입(fail-closed)
            L.send(node="w2", grade="BLOCKER", epistemic="FACT", text="막힘2", to="w1",
                   reason="미주입 경로 시험용 사유", evidence="%s:2:표식-ALPHA" % p)
            recs, _ = R.read_thread("T1", TH)
            payload, _d, _s, _h = R.build_delta("T1", "w1", TH, 0, recs, R.read_meta("T1"))
            check("미주입: 전문 동승(소실 0회 우선)", "막힘2" in payload, payload)
            legs = R.delivery_state("T1", "w1")
            check("미주입: 동승 직후 취소 요청 대장 기록(AA30(d))",
                  legs.get(2, {}).get("recipient") == "CANCEL_REQUESTED", legs)
        finally:
            R.cys_send_queued, R.probe_injected = bak_send, bak_probe


# ════════════════════════ seq · 로테이션 · 오염 ════════════════════════
def t_seq_monotonic():
    with Lane() as L:
        L.open()
        for i in range(5):
            L.send(text="m%d" % i)
        seqs = [r["seq"] for r in L.recs()]
        check("seq: 1..N 단조 연속", seqs == list(range(1, 6)), seqs)
        ids = [r["msg_id"] for r in L.recs()]
        check("seq: msg_id 유일", len(set(ids)) == len(ids), ids)


def t_rotation_no_message_loss():
    """★치명 회귀(AA20 §2.3(a)) — tombstone 이 seq 를 소비하지 않으면 신규 메시지와 seq 가
    겹치고, read_thread 의 seq dedup 이 파일 순서상 앞선 tombstone 만 남겨 **메시지를 영구
    은닉**한다(수리 전 12건 중 10건 소실 실측 · '전문 표면화 0회 금지' 최상위 불변식 붕괴).

    이 케이스는 '가시 레코드의 seq 연속'만 보지 않는다 — 그 검사는 소실을 통과시킨다.
    보내진 **본문 전수가 판독에 살아있는지**를 직접 단언한다."""
    with Lane() as L:
        L.open("T1", "w1,w2,rotw")     # rotw = 발신자(참여자 등록 · A1-R3)
        bak = R.ROTATE_BYTES
        R.ROTATE_BYTES = 400          # 로테이션을 결정론으로 유발(매 건 로테이션)
        try:
            sent = []
            for i in range(10):
                rc, _ = L.send(node="rotw", text="레코드 본문 %d" % i)
                check("로테이션: send %d 성공(차단기 오염 없음)" % i, rc == R.EXIT_OK, rc)
                sent.append("레코드 본문 %d" % i)
        finally:
            R.ROTATE_BYTES = bak
        recs = L.recs()
        texts = [r.get("text") for r in recs if r.get("type") == R.MSG_TYPE]
        missing = [t for t in sent if t not in texts]
        check("★로테이션: 메시지 소실 0건(전 본문 판독 가능)", missing == [], missing)
        tombs = [r for r in recs if r.get("type") == "ROTATED"]
        check("로테이션: tombstone 기록됨", len(tombs) >= 1, len(tombs))
        # ★tombstone 도 임계구역에서 seq 를 **소비**한다 — 메시지와 번호가 겹치지 않는다
        tomb_seqs = {r["seq"] for r in tombs}
        msg_seqs = {r["seq"] for r in recs if r.get("type") == R.MSG_TYPE}
        check("★로테이션: tombstone seq 와 메시지 seq 무충돌", not (tomb_seqs & msg_seqs),
              sorted(tomb_seqs & msg_seqs))
        check("로테이션: seq 는 로테이션을 가로질러 연속",
              [r["seq"] for r in recs] == list(range(1, len(recs) + 1)),
              [r["seq"] for r in recs][:20])
        check("로테이션: 총 레코드 수 = 메시지 + tombstone",
              len(recs) == len(msg_seqs) + len(tomb_seqs), (len(recs), len(msg_seqs), len(tomb_seqs)))
        check("로테이션: tombstone grade:=FYI 자동 부여(§3.9(a))",
              all(t.get("grade") == "FYI" for t in tombs))
        check("로테이션: 3중 대조(prev_final_seq·레코드수·SHA-256) 무결",
              R.verify_rotation("T1", TH) == [], R.verify_rotation("T1", TH))
        # ★표류 주입 — 아카이브를 변조하면 3중 대조가 반드시 잡아야 한다(파일명 단독 대조 금지)
        arch = [p for p in R.segment_paths("T1", TH)
                if os.path.basename(p) != "%s.jsonl" % TH]
        if arch:
            with open(arch[0], "a", encoding="utf-8") as f:
                f.write(json.dumps({"seq": 999, "type": "MSG", "text": "표류"}) + "\n")
            check("로테이션: 레코드 표류를 3중 대조가 검출", R.verify_rotation("T1", TH) != [])
        else:
            check("로테이션: 아카이브 세그먼트 생성", False, "아카이브 없음")


def t_rotation_delta_surfaces_all():
    """로테이션이 표면화 경로에서도 무손실인가 — 델타에 전 메시지가 실린다(소실 재현 시나리오)."""
    with Lane() as L:
        L.open("T1", "w1,w2,rotw")     # rotw = 발신자(참여자 등록 · A1-R3)
        bak = R.ROTATE_BYTES
        R.ROTATE_BYTES = 400
        try:
            for i in range(8):
                L.send(node="rotw", grade="NORMAL", text="델타 본문 %d" % i)
        finally:
            R.ROTATE_BYTES = bak
        recs, _ = R.read_thread("T1", TH)
        payload, _d, _s, _h = R.build_delta("T1", "w2", TH, 0, recs, R.read_meta("T1"))
        missing = [i for i in range(8) if ("델타 본문 %d" % i) not in payload]
        check("★로테이션: 표면화 델타에도 소실 0건", missing == [], missing)


def t_frozen_fallback_no_false_alarm():
    """§2.3(c) Windows rename 폴백 — 동결 마커 + 신규 세그먼트. 아카이브 파일명이 없으므로
    파일명 대조를 그대로 돌리면 **상시 오탐**(불필요 URGENT)이 난다."""
    with Lane() as L:
        L.open()
        L.send(node="w1", text="폴백 앞")
        bak_rot, bak_rename = R.ROTATE_BYTES, R.os.rename
        R.ROTATE_BYTES = 200

        def boom(a, b):
            raise OSError("공유 위반(Windows 폴백 재현)")

        R.os.rename = boom
        try:
            rc, _ = L.send(node="w1", text="폴백 뒤")
            check("폴백: rename 실패해도 send 성공", rc == R.EXIT_OK, rc)
        finally:
            R.ROTATE_BYTES, R.os.rename = bak_rot, bak_rename
        recs = L.recs()
        check("폴백: ROTATED_FROZEN 동결 마커 기록",
              any(r.get("type") == "ROTATED_FROZEN" for r in recs), [r.get("type") for r in recs])
        check("폴백: 신규 세그먼트 개시(META active_segment)",
              bool((R.read_meta("T1").get("active_segment") or {}).get(TH)), R.read_meta("T1"))
        check("폴백: 본문 소실 0",
              {"폴백 앞", "폴백 뒤"} <= {r.get("text") for r in recs},
              [r.get("text") for r in recs])
        check("폴백: seq 연속", [r["seq"] for r in recs] == list(range(1, len(recs) + 1)),
              [r["seq"] for r in recs])
        check("폴백: 상시 오탐 없음(아카이브 부재를 불일치로 세지 않는다)",
              R.verify_rotation("T1", TH) == [], R.verify_rotation("T1", TH))
        # 동결 마커 이후 구파일에 레코드가 나타나면 반드시 검출된다(§2.3(c) 독자 의무)
        seg = R._tp("T1", "%s.jsonl" % TH)
        R._jsonl_append(seg, {"seq": 999, "type": "MSG", "text": "동결 후 잠입"})
        check("폴백: 동결 마커 이후 구파일 레코드는 검출", R.verify_rotation("T1", TH) != [])


def t_halfline_recovery():
    with Lane() as L:
        L.open()
        L.send(text="앞")
        with open(R.thread_path("T1", TH), "ab") as f:
            f.write(b'{"crash": ')       # 크래시 반줄
        rc, _ = L.send(text="뒤")
        check("반줄: 후속 send 성공", rc == R.EXIT_OK, rc)
        recs, diag = R.read_thread("T1", TH)
        check("반줄: 융합으로 인한 후속 레코드 소실 0",
              [r.get("text") for r in recs] == ["앞", "뒤"], [r.get("text") for r in recs])
        check("반줄: 오염 라인으로 계수", len(diag["corrupt"]) == 1, diag)


def t_corrupt_skip_and_gap():
    with Lane() as L:
        L.open()
        for i in range(4):
            L.send(text="m%d" % i)
        p = R.thread_path("T1", TH)
        lines = open(p, encoding="utf-8").read().split("\n")
        lines[1] = "{이건 JSON 이 아니다"        # seq=2 자리를 오염시킨다
        open(p, "w", encoding="utf-8").write("\n".join(lines))
        recs, diag = R.read_thread("T1", TH)
        check("오염: 파싱 불능 라인은 skip 하고 판독 계속(crash 금지)",
              len(recs) == 3 and len(diag["corrupt"]) == 1, (len(recs), diag))
        cur = R.advance_cursor(0, recs, {1})
        check("오염: GAP 봉인 전 커서는 직전 seq 에서 대기", cur == 1, cur)
        rc, _ = L.run(["seal-gap", "--ticket", "T1", "--node", "master",
                       "--from", "2", "--to", "2", "--reason", "오염"])
        check("GAP: --confirm 없으면 거부(정상 데이터 봉인 차단)", rc == R.EXIT_USAGE, rc)
        rc, _ = L.run(["seal-gap", "--ticket", "T1", "--node", "master",
                       "--from", "2", "--to", "2", "--reason", "오염", "--confirm"])
        check("GAP: --confirm 이면 봉인", rc == R.EXIT_OK, rc)
        recs, _ = R.read_thread("T1", TH)
        cur = R.advance_cursor(0, recs, {1, 3, 4})
        check("GAP: 봉인 구간을 '존재'로 취급해 커서 전진 재개", cur >= 4, cur)


# ════════════════════════ 커서 · 격리 · 델타 ════════════════════════
def t_cursor_dual_and_self_echo():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="w1 발신")
        L.send(node="w2", text="w2 발신")
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("wait: 정상 종료", rc == R.EXIT_OK, rc)
        check("자기 에코 제외: 자기 발신은 표면화되지 않는다", "w2 발신" not in out, out)
        check("델타: 타인 발신은 표면화된다", "w1 발신" in out, out)
        surf = R._read_cursor(R.cursor_path("T1", "w2"))
        check("커서: 표면화 커서 전진(자기발신도 정착 처리)", surf == 2, surf)
        ack = R._read_cursor(R.ack_path("T1", "w2"))
        check("커서: 수용 커서는 별개(자동 전진 금지)", ack == 0, ack)
        check("커서: ack 지시자 payload 동봉 의무", "ack" in out)
        L.run(["ack", "--ticket", "T1", "--node", "w2", "2"])
        check("커서: ack 기록", R._read_cursor(R.ack_path("T1", "w2")) == 2)


def t_fence_surfacing_quarantine():
    """★§6.1 — 종전 build_delta 는 META 펜스를 판정에 **전혀 쓰지 않아** 비BLOCKER 가 펜스
    중에도 그대로 표면화됐다(freeze 오염 차단이라는 §6 의 존재 이유가 무력)."""
    with Lane() as L:
        L.open()
        rc, out = L.run(["fence", "--ticket", "T1", "--target", "w2",
                         "--reason", "리뷰 격리"])
        check("펜스: 설정 명령 존재(master 옵저버 §8.2)", rc == R.EXIT_OK, out)
        check("펜스: META 에 fence_seq 기입",
              isinstance((R.read_meta("T1").get("fences") or {}).get("w2"), dict),
              R.read_meta("T1").get("fences"))
        L.send(node="w1", grade="NORMAL", text="펜스 중 비BLOCKER 발신")
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("★펜스: 비BLOCKER 도 표면화 격리", "펜스 중 비BLOCKER 발신" not in out, out)
        q = R.quarantined_seqs("T1", "w2")
        check("펜스: 격리 대장에 **즉시** 기입(A12 §6.3)", 1 in q and not q[1]["released"], q)
        check("펜스: 표면화 커서 미전진(방류가 유일 해소 경로)",
              R._read_cursor(R.cursor_path("T1", "w2")) == 0)
        # verdict 도착 → unfence → 방류 → 다음 wake 에 전문 1회
        rc, out = L.run(["unfence", "--ticket", "T1", "--target", "w2"])
        check("§6.2: unfence 로 펜스 해제 + 방류", rc == R.EXIT_OK and "released" in out, out)
        check("§6.2: 펜스 해제 후 META 비움",
              (R.read_meta("T1").get("fences") or {}).get("w2") is None, R.read_meta("T1"))
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("§6.2: 방류분 전문 1회 표면화", "펜스 중 비BLOCKER 발신" in out, out)
        check("§6.2: 방류 후 커서 전진", R._read_cursor(R.cursor_path("T1", "w2")) == 1)
        # 두 번째 wake 에서는 전문 재표기 금지(기표시 압축)
        rc, out2 = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("§6.2: 방류 전문 표면화는 1회뿐", "펜스 중 비BLOCKER 발신" not in out2, out2)


def t_fence_retro_notice():
    """AA29 §6.1 — 펜스 직후 ≤1폴 창에서 이미 표면화된 건은 소급 격리가 불가하므로
    '펜스 후 표면화됨' 라벨로 master 에 1회 통지한다(리뷰 오염 판정 입력)."""
    with Lane() as L:
        L.open()
        L.send(node="w1", text="펜스 직전 누출")
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        L.run(["fence", "--ticket", "T1", "--target", "w2", "--seq", "0"])
        fresh = R.fence_retro_notice("T1", "w2", R.read_meta("T1"), L.recs())
        check("소급 통지: 펜스 후 표면화된 seq 검출", fresh == [1], fresh)
        again = R.fence_retro_notice("T1", "w2", R.read_meta("T1"), L.recs())
        check("소급 통지: 1회만(멱등)", again == [], again)


def t_quarantine_release_idempotent():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="격리될 건")
        # 대장에 **중복 행**을 물리적으로 심는다(양측 기입 사고 재현)
        for _ in range(3):
            R._jsonl_append(R.fence_ledger("T1", "w2"),
                            {"seq": 1, "grade": "NORMAL", "from": "w1", "stdin_delivered": False})
        q = R.quarantined_seqs("T1", "w2")
        check("격리: 대장은 seq 기준 멱등 집합", list(q.keys()) == [1], q)
        recs, _ = R.read_thread("T1", TH)
        payload, disp, settled, _h = R.build_delta("T1", "w2", TH, 0, recs, R.read_meta("T1"))
        check("격리: 방류 전에는 표면화 안 됨", "격리될 건" not in payload, payload)
        check("격리: 표면화 커서 미전진(방류가 유일 해소 경로)",
              R.advance_cursor(0, recs, settled) == 0)
        rc, _ = L.run(["unfence", "--ticket", "T1", "--target", "w2"])
        check("격리: 방류 수행", rc == R.EXIT_OK, rc)
        q2 = R.quarantined_seqs("T1", "w2")
        check("격리: 방류 후 released=true", q2[1].get("released") is True, q2)
        check("격리: stdin_delivered 는 불변 사실로 보존(방류≠기배달)",
              q2[1].get("stdin_delivered") is False, q2)
        rel = [r for r in R._jsonl_read(R.fence_ledger("T1", "w2"))[0] if r.get("released")]
        check("격리: 중복 3행이어도 방류 기입은 1회(seq 멱등)", len(rel) == 1, len(rel))
        recs2, _ = R.read_thread("T1", TH)
        p2, _d, s2, _h = R.build_delta("T1", "w2", TH, 0, recs2, R.read_meta("T1"))
        check("격리: 방류 후 전문 1회 표면화", "격리될 건" in p2, p2)


def t_pause_hold_and_resume_release():
    """★§9.1·§9.2 — watcher 가 pause 를 확인하지 않으면 kill-switch 중에도 델타가 워커
    컨텍스트로 유입된다(pause 허용 상한 = 관측·저장·보고 위반 방향의 fail-open)."""
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="pause 중 도착 건", to="w2")
        L.pause_on()
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("★pause: 표면화 보류(델타 유입 0)", "pause 중 도착 건" not in out, out)
        check("pause: 커서 미전진", R._read_cursor(R.cursor_path("T1", "w2")) == 0)
        q = R.quarantined_seqs("T1", "w2")
        check("pause: 보류분은 pause 격리 대장에 편입(소실 0)",
              1 in q and q[1]["kind"] == "pause", q)
        check("pause: hb 는 계속 기록(관측 지속)", os.path.exists(R.hb_path("T1", "w2")))
        # pause 지속 중 방류 시도는 fail-closed
        rc, _ = L.run(["resume-release", "--ticket", "T1", "--node", "w2"])
        check("pause: 지속 중 방류는 exit 4 거부(fail-closed)", rc == R.EXIT_PAUSED, rc)
        L.pause_off()
        rc, out = L.run(["resume-release", "--ticket", "T1", "--node", "w2"])
        check("resume: 일괄 방류 수행", rc == R.EXIT_OK, out)
        batches = R.open_batches("T1", "w2")
        check("§9.3: 노드당 방류 요약(batch) 1건 생성", len(batches) == 1, batches)
        check("§9.3: B/U 는 개별 확인 대상으로 목록화", batches[0].get("bu") == [1], batches)
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("resume: 방류분 전문 1회 표면화", "pause 중 도착 건" in out, out)
        check("§9.2: '착수 전 master 확인' 라벨 부착", R.PAUSE_RELEASE_LABEL in out, out)
        # 확인 레코드 없으면 done 은 거부되고 **워커 귀책 아님**이 기록에 남는다
        L.run(["ack", "--ticket", "T1", "--node", "w2", "1"])
        L.run(["resolve", "--ticket", "T1", "--node", "w2", "1",
               "--action", "reflected", "--note", "반영 완료 — 근거 있음"])
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("A25(c): 확인 레코드 부재 시 done 거부", rc == R.EXIT_NO_EVIDENCE, out)
        check("A25(c): 거부 사유에 '워커 귀책 아님' 기계 명시", "워커 귀책 아님" in out, out)
        # ★A4-R04 — URGENT(B/U)는 --batch 만으로는 부족하고 --seqs 개별 확인이 필요하다.
        rc, _ = L.run(["confirm-release", "--ticket", "T1", "--node", "w2",
                       "--batch", batches[0]["batch"], "--note", "batch만"])
        rc_bu, out_bu = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("★A4-R04: B/U 는 batch 확인만으로 done 통과 못 함",
              rc_bu == R.EXIT_NO_EVIDENCE, out_bu)
        rc, _ = L.run(["confirm-release", "--ticket", "T1", "--node", "w2",
                       "--batch", batches[0]["batch"], "--seqs", "1", "--note", "개별 확인"])
        check("AA25(a): 확인 레코드 기록 명령 존재", rc == R.EXIT_OK, rc)
        check("AA25(a): 확인은 기계 판독 레코드로 성립",
              os.path.exists(R.confirm_path("T1", "w2")))
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("A25(c)+A4-R04: B/U 개별 seq 확인 후 done 통과", rc == R.EXIT_OK, out)


def t_recover_cycle_redelivery():
    """§10.3③·AA22 — clear 후 **ack 기준** 미수용 델타 회수. 표면화 커서만으로 회수하는
    구현은 금지다(표면화≠통합 — clear 로 소실된 건은 표면화 커서상 '완료'다)."""
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="clear 로 날아간 긴급", to="w2")
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("회수 전제: 표면화 커서는 전진(ack 는 미기록)",
              R._read_cursor(R.cursor_path("T1", "w2")) == 1
              and R._read_cursor(R.ack_path("T1", "w2")) == 0)
        rc, out = L.run(["recover", "--ticket", "T1", "--node", "w2"])
        check("recover: 실행 표면 존재(사장 코드 아님)", rc == R.EXIT_OK, rc)
        check("recover: 미수용분 전문 재표기", "clear 로 날아간 긴급" in out, out)
        check("recover: 'cycle 복원 재배달' 라벨", R.RECOVERY_LABEL in out, out)
        # AA39(d) 가드 — 죽은 티켓은 회수하지 않는다(스테일 URGENT 부활 차단)
        L.drain()
        L.run(["close", "--ticket", "T1", "--node", "master"])
        rc, out = L.run(["recover", "--ticket", "T1", "--node", "w2"])
        check("AA39(d): close 티켓 회수는 스킵 + 고아 보고",
              rc == R.EXIT_OK and "skipped" in out, out)


def t_fyi_digest_coalescing():
    """§5.2(d) — FYI 단독으로는 wake 를 발동하지 않는다(digest 코얼레싱). 비FYI 가 오면 동승."""
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="FYI", text="사소한 알림 1")
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("★FYI: 단독 신규는 즉시 wake 하지 않는다", "사소한 알림 1" not in out, out)
        check("FYI: 커서 미전진(다음 동승까지 보류)",
              R._read_cursor(R.cursor_path("T1", "w2")) == 0)
        L.send(node="w1", grade="NORMAL", text="반영 필요한 보통 건")
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("FYI: 비FYI 도착 시 동승 표면화",
              "사소한 알림 1" in out and "반영 필요한 보통 건" in out, out)
        # 30분 경과 시 단독 digest 1회
        L.send(node="w1", grade="FYI", text="사소한 알림 2")
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        hp = R._tp("T1", ".fyi-hold-w2.json")
        st = R._read_json(hp) or {}
        R.javis_lock.atomic_write_json(hp, dict(st, first_ts=R._now() - R.FYI_DIGEST_SEC - 1))
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("FYI: 30분 경과 시 단독 digest 1회", "사소한 알림 2" in out, out)
        check("FYI: digest 라벨 표기", "digest" in out, out)


def t_compact_schema_and_priority():
    with Lane() as L:
        L.open("T1", "w1,w2")
        L.send(node="w1", grade="FYI", text="FYI 건")
        L.send(node="w1", grade="BLOCKER", text="블로커", reason="사유가 충분히 길다",
               evidence="%s:2:표식-ALPHA" % L.probe_file(), epistemic="FACT")
        recs, _ = R.read_thread("T1", TH)
        payload, _d, _s, _h = R.build_delta("T1", "w2", TH, 0, recs, R.read_meta("T1"))
        lines = [l for l in payload.split("\n") if l.strip()]
        check("절단: 등급 우선(BLOCKER 가 FYI 보다 먼저)",
              lines and "블로커" in "\n".join(lines[:4]), lines[:4])
        # §4.9 압축 최소 스키마 — 스텁 금지
        c = R.compact_line({"seq": 7, "from": "w1", "grade": "FYI", "epistemic": "HYPOTHESIS",
                            "confidence": "low", "demoted_from": "FACT",
                            "text": "본문" * 200, "evidence": [{}]})
        for token in ("seq=7", "from=w1", "grade=FYI", "epistemic=HYPOTHESIS",
                      "confidence=low", "demoted_from=FACT", "evidence=있음"):
            check("압축 최소 스키마: %s 포함" % token, token in c, c)
        check("압축: text 140자 절단", len(c.split("| ", 1)[1]) <= R.COMPACT_TEXT_CHARS)


def t_cap_truncation_indicator():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        big = "가" * 3000
        # ★멘션(--to)을 붙여 전문 표기를 강제한다 — 압축 표기로 접히면 캡에 닿지 않아
        #   '절단이 일어나지 않았다'가 아니라 '시험 자체가 성립하지 않는다'가 된다.
        for i in range(12):
            L.send(node="w1", grade="NORMAL", epistemic="FACT", to="w2",
                   evidence="%s:2:표식-ALPHA" % p, text="%d %s" % (i, big))
        recs, _ = R.read_thread("T1", TH)
        payload, disp, settled, hidden = R.build_delta("T1", "w2", TH, 0, recs,
                                                       R.read_meta("T1"))
        check("캡: 16KB 초과분은 은닉", len(hidden) > 0, len(hidden))
        check("캡: 은닉분 {seq,grade,from} 1줄 요약 의무",
              all(("seq=%s" % h["seq"]) in payload for h in hidden))
        check("캡: 조기 열람 경로 명시(read --from)", "--from" in payload)
        cur = R.advance_cursor(0, recs, settled)
        check("캡: 은닉분에는 커서 미전진(다음 wake 자동 재포함)",
              cur < R.final_seq(recs), (cur, R.final_seq(recs)))


def t_read_pagination_and_limit_cap():
    with Lane() as L:
        # p0..p4 = 발신자(참여자 등록). 5개 발신자로 분산해 차단기(12/분·발신자 전역)를 피한다.
        L.open("T1", "w1,w2,p0,p1,p2,p3,p4")
        for i in range(50):
            L.send(node="p%d" % (i // 10), text="m%d" % i)
        rc, out = L.run(["read", "--ticket", "T1", "--limit", "10"])
        check("read: 기본 캡 적용", rc == R.EXIT_OK and out.count("── seq=") == 10, out.count("── seq="))
        check("read: 다음 페이지 지시자", "--from" in out, out[-200:])
        # ★§4.10 — --limit 상한. 상한이 없으면 캡의 구제 경로가 캡 목적을 무효화한다.
        for i in range(300):
            R._jsonl_append(R.thread_path("T1", TH),
                            {"schema_version": 1, "type": "MSG", "seq": 100 + i, "from": "bulk",
                             "grade": "FYI", "epistemic": "HYPOTHESIS", "confidence": "low",
                             "text": "대량 %d" % i, "msg_id": "T1:%s:%d" % (TH, 100 + i)})
        rc, out = L.run(["read", "--ticket", "T1", "--limit", "999999"])
        n = out.count("── seq=")
        check("★read: --limit 상한 %d 캡" % R.READ_LIMIT_MAX, n == R.READ_LIMIT_MAX, n)
        check("read: 캡 적용 사실 고지", "상한" in out, out[:200])


# ════════════════════════ 철회 · resolve · done ════════════════════════
def t_retract_transitive_closure():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", epistemic="FACT", text="근거 A", evidence="%s:2:표식-ALPHA" % p)
        m1 = L.recs()[-1]["msg_id"]
        L.send(node="w1", text="가설 B", refs=m1)
        m2 = L.recs()[-1]["msg_id"]
        L.send(node="w1", text="결론 C", refs=m2)
        m3 = L.recs()[-1]["msg_id"]
        recs = L.recs()
        closure = R.transitive_refs([m3], recs)
        check("폐쇄: 3단 체인 전수 도달(직접 집합 검사 금지)",
              {m1, m2, m3} <= closure, closure)
        rc, out = L.run(["retract", "--ticket", "T1", "--node", "w1", m1, "--reason", "오류"])
        check("retract: 성공", rc == R.EXIT_OK, out)
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", m3])
        check("done: 3단 체인 너머의 철회를 검출해 거부", rc == R.EXIT_NO_EVIDENCE, out)
        check("done: 철회 사유가 명시된다", "철회된 근거 인용" in out, out)


def t_retract_grade_and_cycle():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="원본")
        m = L.recs()[-1]["msg_id"]
        for n in ("w1", "w2", "w1"):
            L.send(node=n, text="인용", refs=m)
        L.run(["retract", "--ticket", "T1", "--node", "w1", m, "--reason", "무효"])
        r = L.recs()[-1]
        check("retract: 피인용 3+ 는 URGENT 자동 격상(§7.5)", r.get("grade") == "URGENT", r.get("grade"))
        check("retract: FYI 부여 금지(하한 NORMAL)", r.get("grade") in ("NORMAL", "URGENT"))
        # 순환 refs 를 심어도 폐쇄 산출이 멈춰야 한다(방문 집합)
        recs = L.recs()
        recs.append({"msg_id": "X", "refs": ["Y"], "seq": 99})
        recs.append({"msg_id": "Y", "refs": ["X"], "seq": 100})
        check("폐쇄: 순환 차단", R.transitive_refs(["X"], recs) == {"X", "Y"})


def t_retract_backtrace_via_pair_evidence():
    """AA28 §7.4(b) 확장 — 철회 대상이 어떤 가설의 **짝 증거**였다면 그 가설에 의존해
    done 을 통과한 티켓도 재검토 대상이다. 이 의존은 역방향이라 forward closure 스캔
    으로는 절대 잡히지 않는다(철회 실효화의 두 갈래 중 하나)."""
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", text="가설 H", confidence="low")
        h = L.recs()[-1]["msg_id"]
        L.send(node="w2", epistemic="FACT", text="w2 짝 증거", refs=h,
               evidence="%s:2:표식-ALPHA" % p)
        pair = L.recs()[-1]["msg_id"]
        tasks = os.path.join(L.tmp, "_round", "tasks")
        os.makedirs(tasks, exist_ok=True)
        with open(os.path.join(tasks, "TSK1.json"), "w", encoding="utf-8") as f:
            json.dump({"id": "TSK1", "status": "done", "refs": [h]}, f)
        rc, out = L.run(["retract", "--ticket", "T1", "--node", "w2", pair,
                         "--reason", "짝 증거가 틀렸다"])
        check("retract: 성공", rc == R.EXIT_OK, out)
        check("★AA28: 짝 증거 철회 시 그 가설에 의존한 done 티켓도 재검토 플래그",
              "TSK1" in json.loads(out)["recheck_flagged"], out)
        rows, _ = R._jsonl_read(R._tp("T1", ".recheck-flags.jsonl"))
        check("AA28: 재검토 플래그 대장 영속", rows and "TSK1" in rows[0]["flagged"], rows)


def t_record_carries_thread_field():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        r = L.recs()[-1]
        check("§3.1: 레코드에 thread 필드(msg_id 파싱 없이 소속 판독)",
              r.get("thread") == TH, r.get("thread"))


def t_open_capability_bypass_non_hidden():
    with Lane() as L:
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1,w2",
                       "--skip-capability-gate"])
        check("우회: --skip-reason 없으면 거부", rc == R.EXIT_NO_EVIDENCE, rc)
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1,w2",
                       "--skip-capability-gate", "--skip-reason", "긴급 개통 — master 판단"])
        check("우회: 사유 동반 시 개통", rc == R.EXIT_OK, rc)
        rows, _ = R._jsonl_read(R._tp("T1", ".gate-bypass.jsonl"))
        check("우회: 원장 기록(비은닉)", len(rows) == 1 and rows[0]["gate"] == "capability", rows)


def t_retract_after_close():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        m = L.recs()[-1]["msg_id"]
        L.drain()
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("close: 정상 경로 종결(게이트 통과)", rc == R.EXIT_OK, out)
        rc, _ = L.send(node="w1", text="닫힌 뒤")
        check("close: 이후 send 는 exit 7(영구 닫힘)", rc == R.EXIT_CLOSED, rc)
        rc, _ = L.run(["retract", "--ticket", "T1", "--node", "w1", m, "--reason", "사후"])
        check("close: RETRACT 만 예외 허용(§7.4(a))", rc == R.EXIT_OK, rc)


def t_close_toctou_single_critical_section():
    """AA37 — CLOSE append 와 META closed=true 가 갈리면 그 창에서 CLOSE 이후 seq 에
    append 가 exit 0 으로 성공하는 유령 레코드가 생긴다."""
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        L.drain()
        L.run(["close", "--ticket", "T1", "--node", "master"])
        recs = L.recs()
        close_recs = [r for r in recs if r.get("type") == "CLOSE"]
        check("close: CLOSE 레코드 기록", len(close_recs) == 1, len(close_recs))
        check("close: CLOSE 가 스레드 최종 seq(이후 유령 레코드 0)",
              close_recs[0]["seq"] == max(r["seq"] for r in recs), recs[-1])
        check("close: META closed=true 동시 커밋", R.read_meta("T1").get("closed") is True)
        rc, _ = L.send(node="w1", text="유령")
        check("close: 이후 append 는 임계구역 안에서 exit 7", rc == R.EXIT_CLOSED, rc)
        check("close: 거부 건은 스레드에 미기록", len(L.recs()) == len(recs), len(L.recs()))


def t_resolve_and_done_gate():
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="반영 필요", to="w2")
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("done: 미표면화 URGENT 잔존 시 거부(스레드 직독)", rc == R.EXIT_NO_EVIDENCE, out)
        check("done: 거부 출력에 강제 표면화 후 재시도 지시(AA38①)",
              "강제 표면화" in out and "재시도" in out, out)
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        L.run(["ack", "--ticket", "T1", "--node", "w2", "1"])
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("done: resolve 레코드 부재 시 거부(§5.6 단일 판정입력)",
              rc == R.EXIT_NO_EVIDENCE and "resolve" in out, out)
        rc, _ = L.run(["resolve", "--ticket", "T1", "--node", "w2", "1",
                       "--action", "rejected", "--note", "짧"])
        check("resolve: --note 하한 8자 강제", rc == R.EXIT_NO_EVIDENCE, rc)
        L.run(["resolve", "--ticket", "T1", "--node", "w2", "1",
               "--action", "rejected", "--note", "중복 보고라 기각한다"])
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("done: resolve 기록 후 통과", rc == R.EXIT_OK, out)


def t_done_gate_unknown_record_exempt():
    """AA32(a) — 미지(전방호환) 레코드는 '반영 의무 부과 없이' 압축 표기 대상이다.
    종전 구현은 미래 schema_version + BLOCKER 하나로 done 을 영구 봉쇄했다."""
    with Lane() as L:
        L.open()
        R._jsonl_append(R.thread_path("T1", TH),
                        {"schema_version": R.SCHEMA_VERSION + 5, "type": "MSG", "seq": 1,
                         "from": "w1", "grade": "BLOCKER", "epistemic": "FACT",
                         "text": "미래 스키마 블로커", "msg_id": "T1:%s:1" % TH})
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("★AA32(a): 미지 레코드는 resolve 의무 면제", rc == R.EXIT_OK, out)


def t_pair_evidence_gate():
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", text="가설", confidence="medium")
        h = L.recs()[-1]["msg_id"]
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("짝 증거: 0건이면 done 거부(§5.5)", rc == R.EXIT_NO_EVIDENCE and "짝 증거" in out, out)
        # 타인의 FACT 는 짝이 아니다(수신자 **자신**의 독립 증거여야 한다)
        L.send(node="w1", epistemic="FACT", text="w1 의 확인", refs=h,
               evidence="%s:2:표식-ALPHA" % p)
        rc, _ = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("짝 증거: 타인 발신은 짝으로 불인정", rc == R.EXIT_NO_EVIDENCE, rc)
        L.send(node="w2", epistemic="FACT", text="w2 의 독립 재유도", refs=h,
               evidence="%s:2:표식-ALPHA" % p)
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("짝 증거: 자기 검증 FACT 1건이면 통과", rc == R.EXIT_OK, out)


def t_pair_evidence_scope_is_direct_refs():
    """A19 — 짝 요구 범위는 '산출물이 refs 인용한' **직접 집합**이다. 이행적 폐쇄 전체에
    요구하면 간접 참조 가설에까지 본인 재유도를 요구해 오거부 경로가 재개방된다."""
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", text="간접 가설", confidence="low")
        h_indirect = L.recs()[-1]["msg_id"]
        L.send(node="w1", text="직접 가설", confidence="low", refs=h_indirect)
        h_direct = L.recs()[-1]["msg_id"]
        L.send(node="w2", epistemic="FACT", text="w2 재유도", refs=h_direct,
               evidence="%s:2:표식-ALPHA" % p)
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h_direct])
        check("★A19: 간접 폐쇄 가설에는 짝을 요구하지 않는다(오거부 차단)",
              rc == R.EXIT_OK, out)


def t_pair_evidence_recheck_wired():
    """AA34(b) — 저장 시점 verified 플래그만 보면 사후 파일 변조·근거 소멸이 무탐지로
    done 을 통과한다. 짝 검증·done 게이트에 라인 해시 재검증이 배선돼야 한다."""
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", text="가설", confidence="medium")
        h = L.recs()[-1]["msg_id"]
        L.send(node="w2", epistemic="FACT", text="w2 재유도", refs=h,
               evidence="%s:2:표식-ALPHA" % p)
        rc, _ = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("재검증: 무변조 상태에서는 통과", rc == R.EXIT_OK, rc)
        with open(p, "w", encoding="utf-8") as f:
            f.write("첫줄\n근거가 사라졌다\n")
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("★AA34(b): 사후 변조 시 짝 자격 상실 → done 거부",
              rc == R.EXIT_NO_EVIDENCE, out)
        check("AA34(b): 거부 사유에 재검증 실패 명시", "재검증 실패" in out, out)


# ════════════════════════ close 시퀀스 ════════════════════════
def t_close_sequence_gates():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="미드레인 건")
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("close (iv): 드레인 미달이면 거부", rc == R.EXIT_NO_EVIDENCE and "CLOSE-BLOCK" in out, out)
        # ★A4-R01 — 게이트 판정을 상태 변형 **전에** 한다. 펜스 격리 잔존은 (v) 잔존0 게이트를
        #   막고, 거부 시 close 는 펜스·격리를 **건드리지 않는다**(종전엔 게이트 거부에도
        #   격리를 released:true 로 비가역 방류해 §6 리뷰 격리가 붕괴했다).
        L.run(["fence", "--ticket", "T1", "--target", "w2", "--seq", "0"])
        L.drain()
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("★A4-R01: 격리 미방류면 close 거부(exit 5)",
              rc == R.EXIT_NO_EVIDENCE and "CLOSE-BLOCK" in out, out)
        check("★A4-R01: 거부 시 격리 미방류(released 아님 — 펜스 보존)",
              R.quarantined_seqs("T1", "w2").get(1, {}).get("released") is not True,
              R.quarantined_seqs("T1", "w2"))
        check("★A4-R01: 거부 시 META 펜스 무손상",
              (R.read_meta("T1").get("fences") or {}).get("w2") is not None,
              R.read_meta("T1").get("fences"))
        # verdict 도착 → unfence 로 방류·표면화 → close 통과(정상 해소 경로)
        L.run(["unfence", "--ticket", "T1", "--target", "w2"])
        L.drain()
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("close: 드레인·잔존0 통과 시 정상 종결", rc == R.EXIT_OK, out)
        check("close: 통과 시 격리 방류 수행",
              R.quarantined_seqs("T1", "w2").get(1, {}).get("released") is True,
              R.quarantined_seqs("T1", "w2"))
        meta = R.read_meta("T1")
        check("close: META closed=true", meta.get("closed") is True, meta)
        check("close: 펜스 강제 해제", meta.get("fences") == {}, meta.get("fences"))
        recs, _ = R.read_thread("T1", TH)
        check("close: CLOSE 레코드 기록", any(r.get("type") == "CLOSE" for r in recs))
        check("close: CLOSE 는 관리 레코드(grade:=FYI)",
              all(r.get("grade") == "FYI" for r in recs if r.get("type") == "CLOSE"))
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("close: watcher 는 CLOSED sentinel(재기동 금지)", R.SENTINEL_CLOSED in out, out)


def t_close_force_is_non_hidden():
    """--force 는 '전문 표면화 0회 금지' 불변식의 기계 집행을 무력화하는 행위다 —
    제거하지 않되 사유를 강제하고 우회를 원장으로 가시화한다."""
    with Lane() as L:
        L.open()
        L.send(node="w1", text="미표면화 잔존")
        rc, _ = L.run(["close", "--ticket", "T1", "--node", "master", "--force"])
        check("★--force: --force-reason 없으면 거부", rc == R.EXIT_NO_EVIDENCE, rc)
        rc, _ = L.run(["close", "--ticket", "T1", "--node", "master", "--force",
                       "--force-reason", "짧음"])
        check("--force: 사유 하한(8자) 강제", rc == R.EXIT_NO_EVIDENCE, rc)
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master", "--force",
                         "--force-reason", "고아 티켓 정리 — master 판단"])
        check("--force: 사유 동반 시 종결", rc == R.EXIT_OK, out)
        rows, _ = R._jsonl_read(R._tp("T1", ".gate-bypass.jsonl"))
        check("--force: 우회 원장 1줄 기록", len(rows) == 1, rows)
        check("--force: 원장에 사유·미달 항목 보존",
              rows and rows[0].get("reason") and rows[0].get("drain_short"), rows)


def t_watcher_singleton_and_generation():
    with Lane() as L:
        L.open()
        lk = R.javis_lock.FileLock(R.watcher_lock_path("T1", "w2"), owner="test", soft=True)
        check("watcher: 락 선점 준비", lk.acquire() == R.javis_lock.ACQUIRED)
        rc, _ = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("watcher: 싱글턴 — 중복 기동은 exit 9", rc == R.EXIT_CONFLICT, rc)
        lk.release()
        check("watcher: 락 스코프는 노드×티켓(겸무 워커)",
              R.watcher_lock_path("T1", "w2") != R.watcher_lock_path("T2", "w2"))


def t_sentinel_exited_generation_and_ttl():
    """A36(b)② — 세대 교체·TTL 만료는 **재기동** sentinel 이다(CLOSED 와 뭉뚱그리면
    CLOSE 재소비 무한 루프 또는 영구 난청이 된다)."""
    with Lane() as L:
        L.open()
        bak = R._generation
        seq = {"n": 0}

        def gen():
            seq["n"] += 1
            return "gen-%d" % seq["n"]          # 폴마다 달라진다 = 세대 교체

        R._generation = gen
        try:
            rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2",
                             "--max-polls", "3", "--interval", "0"])
        finally:
            R._generation = bak
        check("sentinel②: 세대 교체 시 EXITED(재기동)", R.SENTINEL_EXITED in out, out)
        check("sentinel②: 세대 교체 사유 표기", "세대 교체" in out, out)
        bak_ttl = R.WATCHER_TTL_SEC
        R.WATCHER_TTL_SEC = -1
        try:
            rc, out = L.run(["wait", "--ticket", "T1", "--node", "w1",
                             "--max-polls", "2", "--interval", "0"])
        finally:
            R.WATCHER_TTL_SEC = bak_ttl
        check("sentinel②: TTL 만료(미close)는 EXITED", R.SENTINEL_EXITED in out, out)
        check("sentinel: TTL 만료는 CLOSED 가 아니다", R.SENTINEL_CLOSED not in out, out)


def t_status_command():
    """§8.3·AA31(d)(e) — master 점검·복원 절차의 기계 판독 표면(hb 신선도·배선단절·
    미확인 batch·재통지 due). HB_STALE_SEC 이 상수로만 있고 판독 주체가 없으면 백스톱은
    실행 불능이다."""
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="점검 대상", to="w2")
        rc, out = L.run(["status", "--ticket", "T1"])
        check("status: 기계 판독 JSON", rc == R.EXIT_OK, rc)
        data = json.loads(out)
        nodes = {n["node"]: n for n in data["tickets"][0]["nodes"]}
        check("status: hb 미기록 노드는 stale", nodes["w2"]["hb"]["stale"] is True, nodes["w2"])
        check("status: 배선단절 신호(표면화 정체 + 신규 seq)",
              nodes["w2"]["deaf_signal"] is True, nodes["w2"])
        check("status: 미resolve B/U 목록", nodes["w2"]["unresolved_bu"] == [1], nodes["w2"])
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        data = json.loads(L.run(["status", "--ticket", "T1"])[1])
        nodes = {n["node"]: n for n in data["tickets"][0]["nodes"]}
        check("status: hb 기록 후 신선", nodes["w2"]["hb"]["stale"] is False, nodes["w2"]["hb"])
        check("status: 표면화 후 배선단절 해소", nodes["w2"]["deaf_signal"] is False, nodes["w2"])
        # AA31(e) 재통지 주기 — B/U 10분 · N/F-only 30분
        R._jsonl_append(R.batch_ledger("T1", "w2"),
                        {"batch": "B-old", "node": "w2", "ts": R._now() - 700,
                         "count": 1, "bu": [1], "seqs": [1]})
        R._jsonl_append(R.batch_ledger("T1", "w1"),
                        {"batch": "B-nf", "node": "w1", "ts": R._now() - 700,
                         "count": 1, "bu": [], "seqs": [2]})
        data = json.loads(L.run(["status", "--ticket", "T1"])[1])
        nodes = {n["node"]: n for n in data["tickets"][0]["nodes"]}
        b_bu = nodes["w2"]["open_batches"][0]
        b_nf = nodes["w1"]["open_batches"][0]
        check("AA31(e): B/U batch 는 10분 주기 재통지 due",
              b_bu["renotify_period_sec"] == R.CONFIRM_RENOTIFY_BU_SEC and b_bu["renotify_due"],
              b_bu)
        check("AA31(e): N/F-only batch 는 30분 주기(700초 경과로는 미도래)",
              b_nf["renotify_period_sec"] == R.CONFIRM_RENOTIFY_NF_SEC
              and not b_nf["renotify_due"], b_nf)
        L.run(["confirm-release", "--ticket", "T1", "--node", "w2", "--batch", "B-old"])
        data = json.loads(L.run(["status", "--ticket", "T1"])[1])
        nodes = {n["node"]: n for n in data["tickets"][0]["nodes"]}
        check("status: 확인된 batch 는 미확인 목록에서 제외",
              nodes["w2"]["open_batches"] == [], nodes["w2"]["open_batches"])


def t_forward_compat():
    check("전방호환: 미지 필드는 무시(오염 아님)",
          not R.is_unknown({"schema_version": 1, "type": "MSG", "grade": "FYI",
                            "epistemic": "FACT", "미래필드": 1}))
    check("전방호환: 상한 초과 schema_version 은 미지 레코드",
          R.is_unknown({"schema_version": R.SCHEMA_VERSION + 1}))
    check("전방호환: 미지 enum 은 미지 레코드", R.is_unknown({"grade": "SUPER"}))
    check("전방호환: 미지 type 은 미지 레코드", R.is_unknown({"type": "FUTURE"}))


def t_naming_and_selftest():
    fails = R._check_naming(R.build_parser())
    check("명명: argv 표면이 SERVER_PATTERNS·금지 플래그와 무충돌", fails == [], fails)
    surface = R._argv_surface(R.build_parser())
    check("명명: 금지 플래그 부재",
          not [f for f in R.FORBIDDEN_FLAGS if f in surface], surface)
    with Lane():
        rc = R._self_test()
        check("--self-test: exit 0", rc == 0, rc)
        # ★능력 영수증은 self-test 의 임시 루트가 아니라 **실 루트**에 남아야 한다
        #   (임시 루트에 쓰면 rmtree 와 함께 사라져 open 게이트가 영영 열리지 않는다).
        rc = R._self_test(record_capability="w9")
        rcpt = R._read_json(R.capability_receipt("w9"))
        check("--record-capability: env 복원 후 실 루트에 영수증 영속",
              rc == 0 and rcpt and rcpt.get("ok") is True, rcpt)


# ════════════════════════ javis_task done 훅 ════════════════════════
def t_task_done_radio_hook():
    """AA38 — done 게이트가 수동 단독 명령으로만 존재하면 '워커가 안 부르면 안 걸리는'
    게이트다. javis_task done 전이에 훅이 배선돼야 묵살 금지가 기계로 집행된다."""
    import subprocess
    with Lane() as L:
        task = os.path.join(_BIN, "javis_task.py")
        radio = os.path.join(_BIN, "javis_radio.py")
        check("훅: javis_task 에 radio 게이트 배선", "javis_radio" in open(task, encoding="utf-8").read())
        env = dict(os.environ, JAVIS_ROOT=L.tmp, CYS_RADIO_DISABLE_CYS="1",
                   CYS_TASK_EVIDENCE_ARTIFACT_GATE="off")

        def task_run(argv, **kw):
            e = dict(env)
            e.update(kw.pop("env_extra", {}))
            r = subprocess.run([sys.executable, task] + argv, capture_output=True, env=e)
            return r.returncode, (r.stdout + r.stderr).decode("utf-8", "replace")

        rc, out = task_run(["create", "라디오 훅 시험", "--id", "T1"])
        check("훅: 티켓 생성", rc == 0, out)
        task_run(["checkout", "T1", "--owner", "w2"])
        art = os.path.join(L.tmp, "art.txt")
        with open(art, "w", encoding="utf-8") as f:
            f.write("검증 산출물\n")
        # ★A1-R3(G2) — 판정 노드는 task.owner 로 **고정**된다(--radio-node 무시). 성공 done 은
        #   owner 를 소거하므로 게이트를 걸려면 매번 재체크아웃해 owner 를 세운다.
        done = ["set-status", "T1", "done", "--evidence", "테스트 31/31 PASS",
                "--settled-override", "테스트", "--skip-reason", "radio 훅 시험"]
        # radio 미개통 티켓 = 비대상(회귀 0 · done 성공 → owner 소거)
        rc, out = task_run(done)
        check("훅: radio 미개통 티켓에는 무간섭", rc == 0, out)
        # 재개통 후 미표면화 URGENT 잔존 → strict 는 거부(판정 노드=owner)
        task_run(["set-status", "T1", "in_progress"])
        task_run(["checkout", "T1", "--owner", "w2"])
        L.open("T1", "w1,w2")
        L.send(node="w1", grade="URGENT", text="반영 필요", to="w2")
        rc, out = task_run(done)
        check("★훅(strict): radio done-check 비0이면 done 거부", rc == 5, out)
        check("훅: 거부 출력에 강제 표면화 지시", "강제 표면화" in out, out[-500:])
        # ★A1-R3(G2) — --radio-node 로 '깨끗한 노드'를 지정해도 판정 노드는 owner 로 고정된다.
        #   (owner=w2 는 의무 잔존 · w1 은 의무 없음이지만 무시되어 여전히 거부)
        rc, out3 = task_run(done + ["--radio-node", "w1"])
        check("★A1-R3: --radio-node 로 판정 노드 회피 불가(owner 고정)",
              rc == 5 and "강제 표면화" in out3, out3[-300:])
        # 판정 노드(owner) 부재 = 측정 불능 → strict 는 통과가 아니다. owner 없는(체크아웃
        #   이력 없는) radio 티켓으로 재현한다(release 후 owner 소거 타이밍에 의존하지 않는다).
        task_run(["create", "노드 미상 시험", "--id", "T2"])
        L.open("T2", "w1,w2")
        rc, out2 = task_run(["set-status", "T2", "done", "--evidence", "e 31/31",
                             "--settled-override", "x", "--skip-reason", "미상 시험"])
        check("훅(strict): 판정 노드(owner) 미상은 거부(측정 불능≠통과)",
              rc == 5 and "판정 노드 미상" in out2, out2[-300:])
        rc, out = task_run(done, env_extra={"CYS_TASK_RADIO_GATE": "warn"})
        check("훅(warn): 통과 + 경고", rc == 0 and "radio warn" in out, out[-300:])
        task_run(["set-status", "T1", "in_progress"])
        rc, out = task_run(done, env_extra={"CYS_TASK_RADIO_GATE": "off"})
        check("훅(off): 검사 생략", rc == 0 and "radio" not in out, out[-300:])
        # refs 기록(AA33(b)① · §7.4(b) 역추적 입력)
        task_run(["set-status", "T1", "in_progress"])
        rc, out = task_run(done + ["--refs", "T1:%s:1" % TH],
                           env_extra={"CYS_TASK_RADIO_GATE": "off"})
        tj = json.load(open(os.path.join(L.tmp, "_round", "tasks", "T1.json"), encoding="utf-8"))
        check("훅: --refs 가 task.refs 로 영속(킬 지표·역추적 입력)",
              tj.get("refs") == ["T1:%s:1" % TH], tj.get("refs"))
        _ = radio, art


# ════════════════════════ R2 적대적 검증 확정 결함 회귀 잠금 ════════════════════════
# 각 케이스는 '위반이 막히는 방향'으로 단언한다(공격 재현 → 차단 실증).

def t_atk_empty_snippet_fact_demoted():   # A1-R1 · A3-R03
    with Lane() as L:
        L.open()
        p = L.probe_file()
        rc, _ = L.send(node="w1", epistemic="FACT", text="빈 스니펫 허위",
                       evidence="%s:2:" % p)   # 스니펫 없음
        r = L.recs()[-1]
        check("★A1-R1: 빈 스니펫 FACT 는 verified=false", r.get("verified") is not True, r)
        check("A1-R1: 빈 스니펫 FACT 는 HYPOTHESIS 강등", r.get("epistemic") == "HYPOTHESIS", r)
        rc, _ = L.send(node="w2", grade="BLOCKER", epistemic="FACT", text="빈 스니펫 블로커",
                       reason="근거 없는 최고 등급 시도", evidence="%s:2:" % p)
        r2 = L.recs()[-1]
        check("★A1-R1: 빈 스니펫 BLOCKER→URGENT 강등(stdin 특권 박탈)",
              r2.get("grade") == "URGENT", r2)
        vok, _w, _e = R.verify_evidence([{"file": p, "line": 2, "snippet": "짧"}])
        check("A1-R1: 6자 미만 스니펫도 검증 실패(자명 통과 봉쇄)", not vok)


def t_atk_self_forged_pair_same_source():   # A1-R2
    with Lane() as L:
        L.open()
        p = L.probe_file("표식-ALPHA\n표식-BETA-LINE\n")
        L.send(node="w1", epistemic="HYPOTHESIS", confidence="low", text="가설",
               evidence="%s:1:표식-ALPHA" % p)
        h = L.recs()[-1]["msg_id"]
        L.send(node="w2", epistemic="FACT", text="자작 재유도(동일 출처)", refs=h,
               evidence="%s:1:표식-ALPHA" % p)
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("★A1-R2: 원 가설과 동일 출처 짝 증거는 자작 재유도로 거부",
              rc == R.EXIT_NO_EVIDENCE, out)
        L.send(node="w2", epistemic="FACT", text="독립 재유도(다른 출처)", refs=h,
               evidence="%s:2:표식-BETA-LINE" % p)
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", h])
        check("A1-R2: 다른 출처의 독립 재유도는 통과", rc == R.EXIT_OK, out)


def t_atk_node_rotation_blocked():   # A1-R3
    with Lane() as L:
        L.open("T1", "w1,w2")
        rc, _ = L.send(node="spoofA", grade="BLOCKER", epistemic="FACT",
                       text="위장", reason="위장 발신 시도임", evidence="x:1:zzzzzz")
        check("★A1-R3: 비참여자 --node send 거부", rc == R.EXIT_USAGE, rc)
        rc, _ = L.send(node="spoofB", grade="BLOCKER", epistemic="FACT",
                       text="위장2", reason="회전 발신 시도임", evidence="x:1:zzzzzz")
        check("A1-R3: --node 회전(spoofB)도 거부(쿨다운·차단기 우회 봉쇄)", rc == R.EXIT_USAGE, rc)
        rc, _ = L.send(node="w1", text="정상 참여자")
        check("A1-R3: 참여자 send 는 정상", rc == R.EXIT_OK, rc)


def t_atk_reviewer_send_wait_blocked():   # A1-R4
    with Lane() as L:
        L.open("T1", "w1,w2")
        rc, _ = L.send(node="reviewer-gemini", text="리뷰어 라이브 개입")
        check("★A1-R4: 리뷰어 send 거부(exit 6)", rc == R.EXIT_REVIEWER, rc)
        rc, _ = L.run(["wait", "--ticket", "T1", "--node", "reviewer-gemini",
                       "--once", "--interval", "0"])
        check("★A1-R4: 리뷰어 wait 거부(exit 6)", rc == R.EXIT_REVIEWER, rc)
        rc, _ = L.send(node="gemini", text="어댑터 키 별칭 우회")
        check("A1-R4: 어댑터 키 gemini send 거부", rc == R.EXIT_REVIEWER, rc)
        rc, _ = L.run(["open", "--ticket", "T2", "--participants", "codex,w1"])
        check("A1-R4: 어댑터 키 codex open 등록 거부(exit 6)", rc == R.EXIT_REVIEWER, rc)


def t_atk_retract_seq_form_normalized():   # A1-R5
    with Lane() as L:
        L.open()
        p = L.probe_file()
        L.send(node="w1", epistemic="FACT", text="근거", evidence="%s:2:표식-ALPHA" % p)
        m = L.recs()[-1]["msg_id"]
        seq = L.recs()[-1]["seq"]
        rc, _ = L.run(["retract", "--ticket", "T1", "--node", "w1", str(seq), "--reason", "오류"])
        check("A1-R5: seq 형태 retract 수용", rc == R.EXIT_OK, rc)
        check("★A1-R5: 철회 집합에 정규 msg_id 로 등재(형식 불일치 해소)",
              m in R.retracted_ids(L.recs()), R.retracted_ids(L.recs()))
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2", "--refs", m])
        check("★A1-R5: seq 형태 철회도 done 게이트가 검출해 거부", rc == R.EXIT_NO_EVIDENCE, out)


def t_atk_capability_forgery_rejected():   # A1-R7
    with Lane() as L:
        R.javis_lock.atomic_write_json(R.capability_receipt("forge"), {"ok": True})
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "forge"])
        check("★A1-R7: 서명 없는 위조 {\"ok\":true} 영수증 거부(exit 3)",
              rc == R.EXIT_CAPABILITY, rc)
        R.write_capability_receipt("forge")
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "forge"])
        check("A1-R7: 정식 서명 영수증이면 개통", rc == R.EXIT_OK, rc)


def t_atk_done_gate_results_thread():   # A2-CONC-01
    with Lane() as L:
        L.open()
        L.send(node="w2", grade="URGENT", text="results STOP", to="w1", thread="results")
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w1"])   # 훅 동형(--thread 미지정)
        check("★A2-CONC-01: results 스레드 미표면화 URGENT 도 done 거부", rc == R.EXIT_NO_EVIDENCE, out)
        check("A2-CONC-01: 거부 사유에 [results] 스레드 명시", "results" in out, out)


def t_atk_dual_thread_no_cursor_suppression():   # A2-CONC-02
    with Lane() as L:
        L.open()
        for i in range(5):
            L.send(node="w2", text="wl%d" % i)
        L.send(node="w2", text="results 건", to="w1", thread="results")
        L.run(["wait", "--ticket", "T1", "--node", "w1", "--once", "--interval", "0"])
        check("A2-CONC-02: worklog 커서 전진",
              R._read_cursor(R.cursor_path("T1", "w1", "worklog")) == 5)
        check("A2-CONC-02: watcher 락 스코프는 노드×스레드(동시 감시 가능)",
              R.watcher_lock_path("T1", "w1", "worklog")
              != R.watcher_lock_path("T1", "w1", "results"))
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w1", "--once",
                         "--interval", "0", "--thread", "results"])
        check("★A2-CONC-02: results 메시지가 worklog 커서에 억제되지 않고 표면화",
              "results 건" in out, out)
        check("A2-CONC-02: results 커서 별도 전진",
              R._read_cursor(R.cursor_path("T1", "w1", "results")) == 1)


def t_atk_ack_monotonic_guard():   # A2-CONC-03
    with Lane() as L:
        L.open()
        L.run(["ack", "--ticket", "T1", "--node", "w1", "10"])
        check("ack: 10 기록", R._read_cursor(R.ack_path("T1", "w1")) == 10)
        L.run(["ack", "--ticket", "T1", "--node", "w1", "3"])
        check("★A2-CONC-03: 역순 ack 는 커서를 후퇴시키지 않는다(단조)",
              R._read_cursor(R.ack_path("T1", "w1")) == 10,
              R._read_cursor(R.ack_path("T1", "w1")))


def t_atk_confirm_release_concurrent():   # A2-CONC-04
    if not hasattr(os, "fork"):
        check("A2-CONC-04: fork 미지원 — 스킵(락으로 구조 보증)", True)
        return
    with Lane() as L:
        L.open()
        N = 8
        pids = []
        for i in range(N):
            pid = os.fork()
            if pid == 0:
                with open(os.devnull, "w") as dn:
                    with contextlib.redirect_stdout(dn), contextlib.redirect_stderr(dn):
                        try:
                            R.main(["confirm-release", "--ticket", "T1", "--node", "w1",
                                    "--seqs", str(i + 1), "--note", "동시 확인 %d" % i])
                        except Exception:
                            pass
                os._exit(0)
            pids.append(pid)
        for pid in pids:
            os.waitpid(pid, 0)
        confs = R.read_confirms("T1", "w1")
        check("★A2-CONC-04: 동시 confirm-release 8건 전부 영속(lost-update 0)",
              len(confs) == N, len(confs))


def t_atk_cooldown_toctou_atomic():   # A2-CONC-05
    if not hasattr(os, "fork"):
        check("A2-CONC-05: fork 미지원 — 스킵(락으로 구조 보증)", True)
        return
    with Lane() as L:
        L.open()
        pids = []
        for i in range(6):     # 동일 참여자 w1 이 URGENT 를 동시 6건 발신
            pid = os.fork()
            if pid == 0:
                rc = 99
                with open(os.devnull, "w") as dn:
                    with contextlib.redirect_stdout(dn), contextlib.redirect_stderr(dn):
                        try:
                            rc = R.main(["send", "--ticket", "T1", "--node", "w1",
                                         "--grade", "URGENT", "--epistemic", "HYPOTHESIS",
                                         "--confidence", "low", "--text", "동시 %d" % i])
                        except Exception:
                            rc = 98
                os._exit(0 if rc == R.EXIT_OK else 1)
            pids.append(pid)
        oks = 0
        for pid in pids:
            _p, st = os.waitpid(pid, 0)
            if os.WIFEXITED(st) and os.WEXITSTATUS(st) == 0:
                oks += 1
        urg = [r for r in L.recs() if r.get("grade") == "URGENT"]
        check("★A2-CONC-05: 동시 URGENT 6건 중 쿨다운 통과 정확히 1건(check→consume 원자화)",
              oks == 1 and len(urg) == 1, (oks, len(urg)))


def t_atk_seal_gap_range_cap():   # A3-R02
    with Lane() as L:
        L.open()
        for i in range(3):
            L.send(text="m%d" % i)
        rc, _ = L.run(["seal-gap", "--ticket", "T1", "--node", "master",
                       "--from", "1", "--to", str(10 ** 12), "--reason", "거대", "--confirm"])
        check("★A3-R02: 거대 GAP 범위 봉인 거부(상한)", rc == R.EXIT_USAGE, rc)
        rc, _ = L.run(["seal-gap", "--ticket", "T1", "--node", "master",
                       "--from", "1", "--to", "999", "--reason", "초과", "--confirm"])
        check("A3-R02: final_seq 초과 봉인 거부", rc == R.EXIT_USAGE, rc)
        # 파일에 거대 GAP 레코드가 있어도 sealed_seqs 는 materialize 하지 않는다(hang/OOM 방지)
        R._jsonl_append(R.thread_path("T1", TH),
                        {"type": "GAP", "gap_from": 5, "gap_to": 10 ** 9, "seq": 99,
                         "schema_version": 1})
        recs, _ = R.read_thread("T1", TH)
        sealed = R.sealed_seqs(recs)
        check("★A3-R02: sealed_seqs 는 구간 멤버십(거대 범위 무-materialize)",
              (7 in sealed) and (10 ** 9 in sealed) and (3 not in sealed))


def t_atk_ticket_path_traversal_blocked():   # A3-R04
    with Lane() as L:
        for n in ("w1", "w2"):
            R.write_capability_receipt(n)
        rc, _ = L.run(["open", "--ticket", "../../ESCAPED_RADIO", "--participants", "w1,w2"])
        check("★A3-R04: ../ ticket 개통 거부", rc == R.EXIT_USAGE, rc)
        rc, _ = L.run(["open", "--ticket", "a/b/c", "--participants", "w1,w2"])
        check("A3-R04: 경로 구분자 포함 ticket 거부", rc == R.EXIT_USAGE, rc)
        check("A3-R04: radio 밖 ESCAPED 디렉터리 미생성",
              not os.path.exists(os.path.join(L.tmp, "ESCAPED_RADIO")))


def t_atk_evidence_outside_root_rejected():   # A3-R05
    with Lane() as L:
        L.open()
        outside = os.path.join(tempfile.gettempdir(), "radio_outside_ev.txt")
        with open(outside, "w", encoding="utf-8") as f:
            f.write("첫줄\n표식-OUTSIDE-LINE\n")
        try:
            rc, _ = L.send(node="w1", epistemic="FACT", text="외부 파일 인용",
                           evidence="%s:2:표식-OUTSIDE-LINE" % outside)
            r = L.recs()[-1]
            check("★A3-R05: 워크스페이스 밖 evidence 는 verified=false(강등)",
                  r.get("verified") is not True and r.get("epistemic") == "HYPOTHESIS", r)
            vok, why, _e = R.verify_evidence(
                [{"file": outside, "line": 2, "snippet": "표식-OUTSIDE-LINE"}])
            check("A3-R05: verify_evidence 가 ROOT 밖 파일 거부", not vok and "밖" in why, why)
        finally:
            os.unlink(outside)


def t_atk_bom_and_control_first_record():   # A3-R06
    with Lane() as L:
        L.open()
        p = R.thread_path("T1", TH)
        rec1 = json.dumps({"seq": 1, "type": "MSG", "text": "BOM 앞", "msg_id": "T1:%s:1" % TH})
        rec2 = json.dumps({"seq": 2, "type": "MSG", "text": "널포함", "msg_id": "T1:%s:2" % TH})
        rec2 = rec2.replace("널포함", "널\x00포함")     # 값 내부에 raw 널바이트
        with open(p, "w", encoding="utf-8") as f:
            f.write("\ufeff" + rec1 + "\n" + rec2 + "\n")
        recs, _diag = R.read_thread("T1", TH)
        seqs = {r.get("seq") for r in recs}
        check("★A3-R06: 선두 BOM 레코드(seq1) 회생(첫 레코드 유실 방지)", 1 in seqs, seqs)
        check("★A3-R06: 널바이트 포함 정상 레코드(seq2) 회생", 2 in seqs, seqs)


def t_atk_jsonl_trailing_semantics():   # A3-R07 · A5-R04
    with Lane() as L:
        L.open()
        p = R.thread_path("T1", TH)
        a = json.dumps({"seq": 1, "type": "MSG", "text": "a", "msg_id": "T1:%s:1" % TH})
        b = json.dumps({"seq": 2, "type": "MSG", "text": "b", "msg_id": "T1:%s:2" % TH})
        with open(p, "w", encoding="utf-8") as f:
            f.write(a + "\n" + b + "\n")
        check("§2.7: 개행 종료 파일은 전 레코드 판독",
              {1, 2} <= {r["seq"] for r in R.read_thread("T1", TH)[0]})
        with open(p, "w", encoding="utf-8") as f:
            f.write(a + "\n" + b)          # b 는 개행 없음 = 기록 진행 중
        check("★A3-R07: 개행 없는 말미 라인은 무시(기록 진행 중 · 죽은 삼항 제거 의도 고정)",
              {r["seq"] for r in R.read_thread("T1", TH)[0]} == {1},
              {r["seq"] for r in R.read_thread("T1", TH)[0]})


def t_atk_pause_gates_state_ops():   # A4-R02
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        L.pause_on()
        rc, _ = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("★A4-R02: pause 중 close 금지(exit 4)", rc == R.EXIT_PAUSED, rc)
        rc, _ = L.run(["fence", "--ticket", "T1", "--target", "w2"])
        check("★A4-R02: pause 중 fence 금지(exit 4)", rc == R.EXIT_PAUSED, rc)
        rc, _ = L.run(["unfence", "--ticket", "T1", "--target", "w2"])
        check("★A4-R02: pause 중 unfence 금지(exit 4)", rc == R.EXIT_PAUSED, rc)
        check("A4-R02: pause 중 거부된 close 는 META closed 무변경(타 노드 watcher 무영향)",
              R.read_meta("T1").get("closed") is not True)
        L.pause_off()
        L.drain()
        rc, _ = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("A4-R02: resume 후 close 정상", rc == R.EXIT_OK, rc)


def t_atk_evidence_parser_colon_snippet():   # A4-R03
    out, _errs = R.parse_evidence(["a.py:5:x:10:y"])
    check("★A4-R03: 왼쪽부터 파일·라인 분리(스니펫 내 :숫자: 오파싱 방지)",
          out and out[0]["file"] == "a.py" and out[0]["line"] == 5
          and out[0]["snippet"] == "x:10:y", out)
    out2, _ = R.parse_evidence([r"C:\x.py:12:snip:val"])
    check("A4-R03: Windows 드라이브 경로도 정상 파싱",
          out2 and out2[0]["file"] == r"C:\x.py" and out2[0]["line"] == 12
          and out2[0]["snippet"] == "snip:val", out2)


def t_atk_confirm_batch_insufficient_for_bu():   # A4-R04
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="pause 중 BU", to="w2")
        L.pause_on()
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        L.pause_off()
        L.run(["resume-release", "--ticket", "T1", "--node", "w2"])
        b = R.open_batches("T1", "w2")[0]["batch"]
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        L.run(["ack", "--ticket", "T1", "--node", "w2", "1"])
        L.run(["resolve", "--ticket", "T1", "--node", "w2", "1",
               "--action", "reflected", "--note", "반영 완료 — 근거 첨부함"])
        L.run(["confirm-release", "--ticket", "T1", "--node", "w2", "--batch", b])
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("★A4-R04: B/U 는 batch 확인만으로 done 통과 못 함", rc == R.EXIT_NO_EVIDENCE, out)
        L.run(["confirm-release", "--ticket", "T1", "--node", "w2", "--batch", b, "--seqs", "1"])
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("A4-R04: B/U 개별 seq 확인 후 done 통과", rc == R.EXIT_OK, out)


def t_atk_single_oversize_record_capped():   # A5-R01 · A5-R06
    with Lane() as L:
        L.open()
        p = L.probe_file()
        big = "가" * 200000                # ≈600KB
        L.send(node="w1", grade="NORMAL", epistemic="FACT", to="w2",
               evidence="%s:2:표식-ALPHA" % p, text=big)
        recs, _ = R.read_thread("T1", TH)
        payload, _disp, _settled, _hidden = R.build_delta("T1", "w2", TH, 0, recs,
                                                          R.read_meta("T1"))
        check("★A5-R01: 단일 초대형 레코드 payload 는 캡 이내로 제한",
              len(payload.encode("utf-8")) < 3 * R.SURFACE_CAP_BYTES,
              len(payload.encode("utf-8")))
        check("A5-R01: 초대형 절단 지시자 표기", "초대형" in payload and "read --from" in payload)
        check("A5-R01: 600KB 본문이 전량 표면화되지 않음", payload.count("가") < 20000)


def t_atk_rotation_notify_dedup():   # A5-R02
    with Lane() as L:
        L.open("T1", "w1,w2,rotw")
        bak = R.ROTATE_BYTES
        R.ROTATE_BYTES = 300
        try:
            for i in range(6):
                L.send(node="rotw", text="rot %d" % i)
        finally:
            R.ROTATE_BYTES = bak
        arch = [q for q in R.segment_paths("T1", TH) if os.path.basename(q) != "%s.jsonl" % TH]
        if arch:
            with open(arch[0], "a", encoding="utf-8") as f:
                f.write(json.dumps({"seq": 999, "type": "MSG", "text": "표류"}) + "\n")
        check("전제: verify_rotation 불일치 지속", R.verify_rotation("T1", TH) != [])
        before = len(R._jsonl_read(R._tp("T1", ".notify-fallback.jsonl"))[0])
        for _ in range(4):
            L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        after = len(R._jsonl_read(R._tp("T1", ".notify-fallback.jsonl"))[0])
        check("★A5-R02: 동일 로테이션 불일치는 dedup(폴마다 긴급 통지 폭주 안 함)",
              after - before <= 1, (before, after))


def t_atk_watcher_poll_skips_unchanged():   # A5-R03
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        calls = {"n": 0}
        bak = R.verify_rotation

        def counting(ticket, thread):
            calls["n"] += 1
            return bak(ticket, thread)

        R.verify_rotation = counting
        try:
            L.run(["wait", "--ticket", "T1", "--node", "w2",
                   "--max-polls", "3", "--interval", "0"])
        finally:
            R.verify_rotation = bak
        check("★A5-R03: 무변경 폴은 verify_rotation(재-SHA) 재실행 안 함(≤1)",
              calls["n"] <= 1, calls["n"])


def t_atk_second_scrub_failclosed_no_advance():   # A5-R05
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="긴급 반영", to="w2")
        bak = R.javis_scrub.scrub

        def boom(x):
            raise RuntimeError("scrub 불능 재현")

        R.javis_scrub.scrub = boom
        try:
            rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2",
                             "--once", "--interval", "0"])
        finally:
            R.javis_scrub.scrub = bak
        check("★A5-R05: scrub fail-closed 시 플레이스홀더 출력(내용 미노출)",
              "출력 보류" in out, out)
        check("★A5-R05: fail-closed 시 표면화 커서 미전진(다음 폴 재시도)",
              R._read_cursor(R.cursor_path("T1", "w2")) == 0)
        check("A5-R05: fail-closed 시 surfaced 대장 미기록(스텁 영구 축소 방지)",
              R.surfaced_seqs("T1", "w2") == set())


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("t_") and callable(v)]
    for t in tests:
        try:
            t()
        except Exception as e:
            import traceback
            check(t.__name__ + " (예외)", False, traceback.format_exc()[-400:])
            _ = e
    passed = sum(1 for _n, ok, _d in _results if ok)
    total = len(_results)
    print("test_radio: PASS %d / FAIL %d (총 %d · 케이스 함수 %d)"
          % (passed, total - passed, total, len(tests)))
    if passed != total:
        for n, ok, d in _results:
            if not ok:
                print("  FAIL %s — %s" % (n, d))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
