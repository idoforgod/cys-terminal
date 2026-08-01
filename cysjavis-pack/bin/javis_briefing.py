#!/usr/bin/env python3
"""javis_briefing.py — W0-B 음성 브리핑 배달층 (EVT v1 → 발화 정책 → 3중 억제 → TTS)

계약(출처: _research/Paperclip_박사급_연구보고서.md §5 · _round/EVENT_CONTRACT.md 발화 정책 열):
- "성공은 조용, 실패·결정·경보만": 이벤트 타입별 발화 정책이 계약에 고정돼 있다.
    speak_now  = run.failed, agent.error, approval.needed, resource.hard, briefing
    critical만 = agent.silent(level=critical만; suspicious는 침묵)
    silent     = run.queued, run.started, run.succeeded, task.unblocked, resource.soft, task.blocked
                 (soft/blocked는 다음 briefing에 집계될 몫 — 즉시 발화 안 함)
- 3중 억제(Paperclip LiveUpdatesProvider의 클린룸 번안):
    ① rate limit — 타입당 10초 창 최대 3건 (상태: $JAVIS_ROOT/_round/briefing/rate.json)
    ② 자기 에코 억제 — --suppress-actor <name>: 방금 지시한 대상의 이벤트는 읽지 않음
    ③ 현재 대화 억제 — --focus <name>: 지금 대화(주시) 중인 대상의 이벤트는 침묵(화면 인라인 몫)
- TTS = macOS `say`(잠금 결정: 입=플랫폼 네이티브 무료). 기본은 드라이런(문장 출력만),
  --speak일 때만 실제 발화. say 부재(비mac)면 자동 드라이런 강등(무음 실패 금지 — 사유 출력).
- 모든 판정은 원장에 기록: $JAVIS_ROOT/_round/briefing/ledger.jsonl (spoken|suppressed + 사유)

사용:  echo '<EVT v1 line>' | javis_briefing.py deliver [--speak] [--suppress-actor m] [--focus w]
       javis_briefing.py deliver --line '<EVT v1 line>' ...
exit: 0 발화(또는 드라이런 발화) · 1 억제(정책/3중 억제 — 정상 동작) · 6 invalid
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import javis_event  # noqa: E402  (동일 폴더 계약 구현 재사용 — parse/speak)

ROOT = os.environ.get("JAVIS_ROOT") or os.getcwd()  # 개인경로 하드코딩 금지(pack scan gate) — env 또는 CWD(워크스페이스 루트에서 호출)
BRIEF_DIR = os.path.join(ROOT, "_round", "briefing")
RATE_PATH = os.path.join(BRIEF_DIR, "rate.json")
LEDGER = os.path.join(BRIEF_DIR, "ledger.jsonl")

EXIT_SPOKEN, EXIT_SUPPRESSED, EXIT_INVALID = 0, 1, 6

RATE_WINDOW_SEC = 10.0
RATE_MAX_PER_WINDOW = 3

SPEAK_NOW = {"run.failed", "agent.error", "approval.needed", "resource.hard", "briefing"}
CRITICAL_ONLY = {"agent.silent"}
# 그 외 전 타입 = silent (성공은 조용 · soft/blocked는 briefing 집계 몫)


def _now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def _ledger(entry):
    os.makedirs(BRIEF_DIR, exist_ok=True)
    entry["ts"] = _now()
    with open(LEDGER, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def _policy_verdict(evt_type, payload):
    """발화 정책 판정: ('speak'|'silent', 사유)"""
    if evt_type in SPEAK_NOW:
        return "speak", "policy:speak_now"
    if evt_type in CRITICAL_ONLY:
        if payload.get("level") == "critical":
            return "speak", "policy:critical"
        return "silent", "policy:below_critical"
    return "silent", "policy:quiet_type(성공은 조용)"


def _rate_limited(evt_type, now=None):
    """타입당 10초 창 3건 상한. 상태는 파일(재시작 생존). True=억제."""
    now = now if now is not None else time.time()
    try:
        with open(RATE_PATH, encoding="utf-8") as f:
            state = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        state = {}
    recent = [t for t in state.get(evt_type, []) if now - t < RATE_WINDOW_SEC]
    if len(recent) >= RATE_MAX_PER_WINDOW:
        state[evt_type] = recent
        _save_rate(state)
        return True
    recent.append(now)
    state[evt_type] = recent
    _save_rate(state)
    return False


def _save_rate(state):
    os.makedirs(BRIEF_DIR, exist_ok=True)
    tmp = f"{RATE_PATH}.tmp.{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f)
    os.replace(tmp, RATE_PATH)


def _tts_available():
    return sys.platform == "darwin" and shutil.which("say") is not None


VOICE_JARVIS_DIR = os.path.join(ROOT, "_work", "voice-jarvis")
SPEAKER_LOCK = os.path.join(VOICE_JARVIS_DIR, "logs", "speaker.lock")


def _voice_mode():
    """TTS 라우팅 판정 — 'vm'(voice-jarvis 경유)|'plain'(say 직접)|'text'(발화 불가·텍스트만).

    에코 루프 방어(2026-07-03 실기 결함의 재발 방지): 음성 노드가 청취 중일 때 briefing이
    독자적으로 say를 부르면 자비스 발화가 오너 발화로 오인·재주입된다. 노드 가동 중이면
    반드시 voice_mcp.tool_speak(화자잠금+speak_start 로그=노드의 에코 방어 원천) 경유.
    테스트 주입: JAVIS_BRIEFING_VOICE_MODE=vm|plain|text
    """
    override = os.environ.get("JAVIS_BRIEFING_VOICE_MODE")
    if override in ("vm", "plain", "text"):
        return override
    node_running = False
    try:
        r = subprocess.run(["pgrep", "-f", "voice_node.py"], capture_output=True,
                           text=True, timeout=5)
        node_running = bool(r.stdout.strip())
    except (subprocess.SubprocessError, OSError):
        pass
    if node_running or os.path.exists(SPEAKER_LOCK):
        return "vm"
    return "plain" if _tts_available() else "text"


def _speak_via_vm(sentence, voice):
    """voice-jarvis의 speak 계약 경유(파일 무수정 — 모듈 호출만). 성공 시 mode 문자열."""
    sys.path.insert(0, VOICE_JARVIS_DIR)
    try:
        import voice_mcp as vm  # 잠금·이벤트 로그·비밀 마스킹은 vm이 소유
        args = {"text": sentence}
        if voice:
            args["voice"] = voice
        res = vm.tool_speak(args)
        if isinstance(res, dict) and res.get("error"):
            return f"vm_refused({res['error']})"
        return "vm_tts"
    except Exception as e:  # 노드 가동 중 bare say 폴백은 에코 위험 — 텍스트로만 강등
        return f"vm_failed({type(e).__name__}:{e})"
    finally:
        try:
            sys.path.remove(VOICE_JARVIS_DIR)
        except ValueError:
            pass


def deliver(line, speak_flag, suppress_actors, focus, voice=None):
    """반환: (exit_code, 설명 문자열)"""
    try:
        evt_type, payload = javis_event.parse_wire(line)
    except ValueError as e:
        return EXIT_INVALID, f"invalid: {e}"

    subject = str(payload.get("agent", payload.get("task", "")))

    # ② 자기 에코 억제 — 방금 내가 지시한 대상의 이벤트는 읽지 않음
    if subject and subject in (suppress_actors or []):
        _ledger({"verdict": "suppressed", "why": "self_echo", "type": evt_type,
                 "subject": subject})
        return EXIT_SUPPRESSED, f"suppressed(자기 에코): {subject}"

    # ③ 현재 대화 대상 억제 — 지금 주시 중인 대상은 화면 인라인 몫
    if subject and focus and subject == focus:
        _ledger({"verdict": "suppressed", "why": "focused", "type": evt_type,
                 "subject": subject})
        return EXIT_SUPPRESSED, f"suppressed(현재 대화 대상): {subject}"

    # 발화 정책(계약 표)
    verdict, why = _policy_verdict(evt_type, payload)
    if verdict == "silent":
        _ledger({"verdict": "suppressed", "why": why, "type": evt_type, "subject": subject})
        return EXIT_SUPPRESSED, f"suppressed({why})"

    # ① rate limit — 타입당 10초 3건
    if _rate_limited(evt_type):
        _ledger({"verdict": "suppressed", "why": "rate_limit(10s/3)", "type": evt_type,
                 "subject": subject})
        return EXIT_SUPPRESSED, "suppressed(rate_limit 10초 3건 상한)"

    sentence = javis_event.speak(evt_type, payload)
    spoken_mode = "dryrun"
    if speak_flag:
        mode = _voice_mode()
        if mode == "vm":
            spoken_mode = _speak_via_vm(sentence, voice)
        elif mode == "plain":
            cmd = ["say"] + (["-v", voice] if voice else []) + [sentence]
            try:
                subprocess.run(cmd, check=True, timeout=60)
                spoken_mode = "plain_tts"
            except (subprocess.SubprocessError, OSError) as e:
                spoken_mode = f"tts_failed({e})"  # 무음 실패 금지 — 사유 기록·텍스트로 강등
        else:
            spoken_mode = "tts_unavailable→dryrun"
    _ledger({"verdict": "spoken", "mode": spoken_mode, "type": evt_type,
             "subject": subject, "sentence": sentence})
    print(sentence)
    if spoken_mode not in ("vm_tts", "plain_tts", "dryrun"):
        print(f"(주의: {spoken_mode})", file=sys.stderr)
    return EXIT_SPOKEN, sentence


def cmd_deliver(a):
    line = a.line if a.line else sys.stdin.readline()
    code, msg = deliver(line, a.speak, a.suppress_actor or [], a.focus, a.voice)
    if code != EXIT_SPOKEN:
        print(msg, file=sys.stderr)
    return code


def main(argv=None):
    p = argparse.ArgumentParser(description="음성 브리핑 배달층 — 정책+3중 억제+TTS (W0-B)")
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("deliver")
    c.add_argument("--line", help="EVT v1 라인(생략 시 stdin)")
    c.add_argument("--speak", action="store_true", help="실제 TTS 발화(기본 드라이런)")
    c.add_argument("--voice", help="say 음성(예: Yuna)")
    c.add_argument("--suppress-actor", action="append",
                   help="자기 에코 억제 대상(반복 가능) — 방금 지시한 워커")
    c.add_argument("--focus", help="현재 대화(주시) 중인 대상 — 그 이벤트는 침묵")
    c.set_defaults(fn=cmd_deliver)

    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
