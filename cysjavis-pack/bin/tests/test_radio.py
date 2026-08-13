#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""javis_radio.py 게이트별 단위 테스트 (RADIO_SPEC_v4 · 2026-08-13).

배포 제외: 경로 컴포넌트 `tests` 는 build.rs 가 임베드 대상에서 제외한다(build.rs:56-59).

설계 원칙
  · **임시 디렉터리 격리** — 케이스마다 `JAVIS_ROOT` 를 새 tmpdir 로 갈아끼운다. 실장부
    (`_round/radio/`)에는 한 바이트도 쓰지 않는다.
  · **데몬 비의존** — `CYS_RADIO_DISABLE_CYS=1` 로 cys 통로를 끈다. 네트워크 0.
  · **게이트당 1케이스** — '기능이 돈다'가 아니라 '위반이 정확한 exit 로 막힌다'를 잠근다.
    통과 경로만 확인하는 테스트는 게이트가 통째로 제거돼도 초록이라 쓸모가 없다.

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
        """(exit code, stdout). 게이트 판정은 **exit code 가 사실**이다."""
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = R.main(argv)
        return rc, buf.getvalue()

    def open(self, ticket="T1", participants="w1,w2"):
        for n in participants.split(","):
            R.javis_lock.atomic_write_json(R.capability_receipt(n.strip()), {"ok": True})
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

    def recs(self, ticket="T1", thread="main"):
        return R.read_thread(ticket, thread)[0]


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
        R.javis_lock.atomic_write_json(R.capability_receipt("w1"), {"ok": False})
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("open: 실패 영수증도 거부", rc == R.EXIT_CAPABILITY, rc)
        R.javis_lock.atomic_write_json(R.capability_receipt("w1"), {"ok": True})
        rc, _ = L.run(["open", "--ticket", "T1", "--participants", "w1"])
        check("open: 통과 영수증이면 개통", rc == R.EXIT_OK, rc)


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
        check("BLOCKER ⓪진위: stdin 배달 대장에 기록 없음",
              not os.path.exists(R.delivery_ledger("T1", "w1")))


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
        L.open()
        L.run(["open", "--ticket", "T2", "--participants", "w1,w2"])
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


def t_rotation_tombstone():
    with Lane() as L:
        L.open()
        bak = R.ROTATE_BYTES
        R.ROTATE_BYTES = 400          # 로테이션을 결정론으로 유발
        try:
            for i in range(12):
                L.send(text="레코드 본문 %d" % i)
        finally:
            R.ROTATE_BYTES = bak
        recs = L.recs()
        tombs = [r for r in recs if r.get("type") == "ROTATED"]
        check("로테이션: tombstone 기록됨", len(tombs) >= 1, len(tombs))
        check("로테이션: seq 는 로테이션을 가로질러 연속",
              [r["seq"] for r in recs] == list(range(1, len(recs) + 1)),
              [r["seq"] for r in recs][:20])
        check("로테이션: tombstone grade:=FYI 자동 부여(§3.9(a))",
              all(t.get("grade") == "FYI" for t in tombs))
        check("로테이션: 3중 대조(prev_final_seq·레코드수·SHA-256) 무결",
              R.verify_rotation("T1", "main") == [], R.verify_rotation("T1", "main"))
        # ★표류 주입 — 아카이브를 변조하면 3중 대조가 반드시 잡아야 한다(파일명 단독 대조 금지)
        arch = [p for p in R.segment_paths("T1", "main") if ".jsonl" in os.path.basename(p)
                and os.path.basename(p) != "main.jsonl"]
        if arch:
            with open(arch[0], "a", encoding="utf-8") as f:
                f.write(json.dumps({"seq": 999, "type": "MSG", "text": "표류"}) + "\n")
            check("로테이션: 레코드 표류를 3중 대조가 검출", R.verify_rotation("T1", "main") != [])
        else:
            check("로테이션: 아카이브 세그먼트 생성", False, "아카이브 없음")


def t_halfline_recovery():
    with Lane() as L:
        L.open()
        L.send(text="앞")
        with open(R.thread_path("T1", "main"), "ab") as f:
            f.write(b'{"crash": ')       # 크래시 반줄
        rc, _ = L.send(text="뒤")
        check("반줄: 후속 send 성공", rc == R.EXIT_OK, rc)
        recs, diag = R.read_thread("T1", "main")
        check("반줄: 융합으로 인한 후속 레코드 소실 0",
              [r.get("text") for r in recs] == ["앞", "뒤"], [r.get("text") for r in recs])
        check("반줄: 오염 라인으로 계수", len(diag["corrupt"]) == 1, diag)


def t_corrupt_skip_and_gap():
    with Lane() as L:
        L.open()
        for i in range(4):
            L.send(text="m%d" % i)
        p = R.thread_path("T1", "main")
        lines = open(p, encoding="utf-8").read().split("\n")
        lines[1] = "{이건 JSON 이 아니다"        # seq=2 자리를 오염시킨다
        open(p, "w", encoding="utf-8").write("\n".join(lines))
        recs, diag = R.read_thread("T1", "main")
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
        recs, _ = R.read_thread("T1", "main")
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
        recs, _ = R.read_thread("T1", "main")
        payload, disp, settled, _h = R.build_delta("T1", "w2", "main", 0, recs, R.read_meta("T1"))
        check("격리: 방류 전에는 표면화 안 됨", "격리될 건" not in payload, payload)
        check("격리: 표면화 커서 미전진(방류가 유일 해소 경로)",
              R.advance_cursor(0, recs, settled) == 0)
        rc, _ = L.run(["close", "--ticket", "T1", "--node", "master", "--force"])
        check("격리: close (i) 방류 수행", rc == R.EXIT_OK, rc)
        q2 = R.quarantined_seqs("T1", "w2")
        check("격리: 방류 후 released=true", q2[1].get("released") is True, q2)
        check("격리: stdin_delivered 는 불변 사실로 보존(방류≠기배달)",
              q2[1].get("stdin_delivered") is False, q2)
        rel = [r for r in R._jsonl_read(R.fence_ledger("T1", "w2"))[0] if r.get("released")]
        check("격리: 중복 3행이어도 방류 기입은 1회(seq 멱등)", len(rel) == 1, len(rel))
        recs2, _ = R.read_thread("T1", "main")
        p2, _d, s2, _h = R.build_delta("T1", "w2", "main", 0, recs2, R.read_meta("T1"))
        check("격리: 방류 후 전문 1회 표면화", "격리될 건" in p2, p2)


def t_compact_schema_and_priority():
    with Lane() as L:
        L.open("T1", "w1,w2")
        L.send(node="w1", grade="FYI", text="FYI 건")
        L.send(node="w1", grade="BLOCKER", text="블로커", reason="사유가 충분히 길다",
               evidence="%s:2:표식-ALPHA" % L.probe_file(), epistemic="FACT")
        recs, _ = R.read_thread("T1", "main")
        payload, _d, _s, _h = R.build_delta("T1", "w2", "main", 0, recs, R.read_meta("T1"))
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
        big = "가" * 3000
        # ★멘션(--to)을 붙여 전문 표기를 강제한다 — 압축 표기로 접히면 캡에 닿지 않아
        #   '절단이 일어나지 않았다'가 아니라 '시험 자체가 성립하지 않는다'가 된다.
        for i in range(12):
            L.send(node="w1", grade="NORMAL", epistemic="FACT", to="w2",
                   text="%d %s" % (i, big))
        recs, _ = R.read_thread("T1", "main")
        payload, disp, settled, hidden = R.build_delta("T1", "w2", "main", 0, recs,
                                                       R.read_meta("T1"))
        check("캡: 16KB 초과분은 은닉", len(hidden) > 0, len(hidden))
        check("캡: 은닉분 {seq,grade,from} 1줄 요약 의무",
              all(("seq=%s" % h["seq"]) in payload for h in hidden))
        check("캡: 조기 열람 경로 명시(read --from)", "--from" in payload)
        cur = R.advance_cursor(0, recs, settled)
        check("캡: 은닉분에는 커서 미전진(다음 wake 자동 재포함)",
              cur < R.final_seq(recs), (cur, R.final_seq(recs)))


def t_read_pagination():
    with Lane() as L:
        L.open()
        for i in range(50):
            L.send(text="m%d" % i)
        rc, out = L.run(["read", "--ticket", "T1", "--limit", "10"])
        check("read: 기본 캡 적용", rc == R.EXIT_OK and out.count("── seq=") == 10, out.count("── seq="))
        check("read: 다음 페이지 지시자", "--from" in out, out[-200:])


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


def t_retract_after_close():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="본문")
        m = L.recs()[-1]["msg_id"]
        L.run(["close", "--ticket", "T1", "--node", "master", "--force"])
        rc, _ = L.send(node="w1", text="닫힌 뒤")
        check("close: 이후 send 는 exit 7(영구 닫힘)", rc == R.EXIT_CLOSED, rc)
        rc, _ = L.run(["retract", "--ticket", "T1", "--node", "w1", m, "--reason", "사후"])
        check("close: RETRACT 만 예외 허용(§7.4(a))", rc == R.EXIT_OK, rc)


def t_resolve_and_done_gate():
    with Lane() as L:
        L.open()
        L.send(node="w1", grade="URGENT", text="반영 필요", to="w2")
        rc, out = L.run(["done-check", "--ticket", "T1", "--node", "w2"])
        check("done: 미표면화 URGENT 잔존 시 거부(스레드 직독)", rc == R.EXIT_NO_EVIDENCE, out)
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


# ════════════════════════ close 시퀀스 ════════════════════════
def t_close_sequence_gates():
    with Lane() as L:
        L.open()
        L.send(node="w1", text="미드레인 건")
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master"])
        check("close (iv): 드레인 미달이면 거부", rc == R.EXIT_NO_EVIDENCE and "CLOSE-BLOCK" in out, out)
        R._jsonl_append(R.fence_ledger("T1", "w2"),
                        {"seq": 1, "grade": "NORMAL", "from": "w1", "stdin_delivered": False})
        L.run(["wait", "--ticket", "T1", "--node", "w1", "--once", "--interval", "0"])
        L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        rc, out = L.run(["close", "--ticket", "T1", "--node", "master", "--force"])
        check("close: --force 로 종결", rc == R.EXIT_OK, out)
        meta = R.read_meta("T1")
        check("close: META closed=true", meta.get("closed") is True, meta)
        check("close: 펜스 강제 해제", meta.get("fences") == {}, meta.get("fences"))
        recs, _ = R.read_thread("T1", "main")
        check("close: CLOSE 레코드 기록", any(r.get("type") == "CLOSE" for r in recs))
        check("close: CLOSE 는 관리 레코드(grade:=FYI)",
              all(r.get("grade") == "FYI" for r in recs if r.get("type") == "CLOSE"))
        rc, out = L.run(["wait", "--ticket", "T1", "--node", "w2", "--once", "--interval", "0"])
        check("close: watcher 는 CLOSED sentinel(재기동 금지)", R.SENTINEL_CLOSED in out, out)


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
