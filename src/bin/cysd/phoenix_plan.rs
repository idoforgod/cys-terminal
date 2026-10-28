//! ★(0.14.56 PHX · DESIGN r1 §5-4 · r2 §5-2 · r2 §7-1) phoenix 계획과 authority 결과의 **순수** 판정.
//!
//! 데몬은 부팅마다 phoenix 에게 "지금 복원하면 누구를 띄우는가"를 묻고(`javis_phoenix.py plan`), **certified
//! `Empty` 에서만** 자기 authority 복원을 건너뛴다. 그 판정이 여기 있다 — 프로세스를 띄우지 않는 순수 함수라
//! 단위 검체가 모든 갈래에 닿는다(spawn·대기·파일 판독은 호출자 몫이고, 그 결과만 받는다).
//!
//! 불변식 하나: **모르는 것은 건너뛰지 않는다.** `permits_skip()` 은 `Empty` 에서만 참이고, `Empty` 는
//! 정확히 한 줄의 일관된 계획에서만 나온다. 판독 불가·모순·잠금 불명은 전부 `Empty` 가 아니다.

use serde_json::Value;

use crate::reclaim::LeaseState;

/// 데몬이 계획 출력에서 읽는 상한(r1 §5-2 · r2 §4-1). 호출자는 **상한 + 1 바이트까지** 읽어 넘긴다 —
/// 넘친 출력은 판독하지 않고 `Garbled` 로 접는다(잘린 계획 줄을 계획으로 읽지 않는다).
pub const PLAN_OUTPUT_CAP: usize = 64 * 1024;

/// 계획 한 줄의 접두 — phoenix `PLAN_FRAME_PREFIX` 와 같다(`cysjavis-pack/bin/javis_phoenix.py`).
pub const PLAN_FRAME_PREFIX: &[u8] = b"phoenix-plan/1 ";

/// exit 0 인데 결과 채널이 없던 시도(결과 파일 자리를 비우지 못해 변수를 넘기지 않았다)의 사유(r2 §7-1).
pub const CAUSE_NO_RESULT_CHANNEL: &str = "no_result_channel";

/// 계획을 얻지 못한 이유 — 어느 것도 건너뛰기를 허락하지 않는다(통지 문구에만 쓰인다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanFailure {
    SpawnFailed(String),
    Timeout,
    Signal,
    Exit(i32),
    NoOutput,
    Garbled(&'static str),
    ProtocolMismatch(String),
    WaitFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutcome {
    /// `permits_skip()` 이 참인 유일한 갈래.
    Empty,
    Targets(Vec<String>),
    /// phoenix 의 사유와 데몬의 사유(잠금 보유·판정 불능).
    Uncertified(Vec<String>),
    Unobtainable(PlanFailure),
}

impl PlanOutcome {
    pub fn permits_skip(&self) -> bool {
        matches!(self, PlanOutcome::Empty)
    }
}

/// 계획 호출 하나의 결과 → `PlanOutcome` (r1 §5-4 표). 순서가 계약이다: 프로세스 사실(spawn·기한·신호·exit)이
/// 출력보다 먼저다 — exit ≠ 0 이면 줄에 무엇이 적혔든 `Unobtainable` 이다(phoenix 의 `outcome:"error"` 줄 포함).
pub fn classify_plan(
    spawn: Result<(), String>,
    exit: Option<i32>,
    timed_out: bool,
    out: &[u8],
) -> PlanOutcome {
    let fail = PlanOutcome::Unobtainable;
    if let Err(e) = spawn {
        return fail(PlanFailure::SpawnFailed(e));
    }
    if timed_out {
        return fail(PlanFailure::Timeout);
    }
    let code = match exit {
        None => return fail(PlanFailure::Signal),
        Some(c) => c,
    };
    if code != 0 {
        return fail(PlanFailure::Exit(code));
    }
    if out.len() > PLAN_OUTPUT_CAP {
        return fail(PlanFailure::Garbled("output-over-64KiB"));
    }
    let mut framed = out
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .filter(|l| l.starts_with(PLAN_FRAME_PREFIX));
    let line = match (framed.next(), framed.next()) {
        (None, _) => return fail(PlanFailure::NoOutput),
        (Some(_), Some(_)) => return fail(PlanFailure::Garbled("multiple-framed-lines")),
        (Some(l), None) => &l[PLAN_FRAME_PREFIX.len()..],
    };
    let v: Value = match serde_json::from_slice(line) {
        Ok(v) => v,
        Err(_) => return fail(PlanFailure::Garbled("invalid-json")),
    };
    let Some(obj) = v.as_object() else {
        return fail(PlanFailure::Garbled("not-an-object"));
    };
    // 프로토콜이 내용보다 먼저다 — 다른 판의 계획은 필드 뜻이 다를 수 있다. 정수 1 만(참·1.0 은 아니다).
    match obj.get("phoenix_plan") {
        Some(p) if p.as_u64() == Some(1) => {}
        other => {
            let seen = other.map_or_else(|| "missing".to_string(), |p| one_line(&p.to_string()));
            return fail(PlanFailure::ProtocolMismatch(seen));
        }
    }
    let Some(targets) = string_list(obj.get("targets")) else {
        return fail(PlanFailure::Garbled("targets-not-a-string-list"));
    };
    let Some(reasons) = string_list(obj.get("reasons")) else {
        return fail(PlanFailure::Garbled("reasons-not-a-string-list"));
    };
    match obj.get("outcome").and_then(Value::as_str) {
        Some("targets") => {
            if targets.is_empty() {
                return fail(PlanFailure::Garbled("targets-outcome-without-targets"));
            }
            PlanOutcome::Targets(targets)
        }
        Some("uncertified") => {
            if !targets.is_empty() {
                return fail(PlanFailure::Garbled("targets-listed-while-not-targets"));
            }
            if reasons.is_empty() {
                return fail(PlanFailure::Garbled("uncertified-without-reasons"));
            }
            PlanOutcome::Uncertified(reasons)
        }
        Some("empty") => {
            if !targets.is_empty() {
                return fail(PlanFailure::Garbled("empty-with-targets"));
            }
            if !reasons.is_empty() {
                return fail(PlanFailure::Garbled("empty-with-reasons"));
            }
            // phoenix 가 내서는 안 되는 모순 — 신원(cys CLI identity)이 검증되지 않은 계산은 비었다고 증명하지 못한다.
            if obj.get("identity").and_then(Value::as_str) != Some("verified") {
                return fail(PlanFailure::Garbled("empty-without-verified-identity"));
            }
            PlanOutcome::Empty
        }
        _ => fail(PlanFailure::Garbled("unknown-outcome")),
    }
}

/// `Empty` 뒤의 잠금 관문(r2 §5-2) — 두 복원 잠금(`restore.lease` · `restore.run.lock`)이 **둘 다** `Free`
/// 일 때만 `Empty` 가 선다. 어느 하나라도 `Held` 면 아직 복원 중인 누군가가 있다(r1 §5-4 "either") —
/// `Held` 가 `Unavailable` 보다 앞선다. `Unavailable` 은 건너뛰기를 허락하지 않는다(reclaim 과 같은 규칙).
/// `Empty` 가 아닌 계획은 그대로 돌려준다.
pub fn gate_empty(plan: PlanOutcome, lease: LeaseState, runlock: LeaseState) -> PlanOutcome {
    if plan != PlanOutcome::Empty {
        return plan;
    }
    let reason = if lease == LeaseState::Held || runlock == LeaseState::Held {
        "restore_lock_held"
    } else if lease == LeaseState::Unavailable || runlock == LeaseState::Unavailable {
        "restore_lock_unavailable"
    } else {
        return PlanOutcome::Empty;
    };
    PlanOutcome::Uncertified(vec![reason.to_string()])
}

/// authority 복원 루프의 **마지막 시도** 판정(r2 §7-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityOutcome {
    Done,
    /// phoenix 가 LEASE_HELD 로 물러났다 — 다른 복원이 진행 중이었다.
    Retreat,
    /// exit 0 인데 무엇이 일어났는지 읽을 수 없다 — done 이 아니다(한 번 재확인한다).
    Ambiguous(String),
    /// exit ≠ 0 또는 신호 — 루프의 기존 exit 처리가 그대로 맡는다(결과 파일은 로그에만).
    Exited(Option<i32>),
}

/// phoenix 가 exit 0 으로 끝내는 완료 판정(`javis_phoenix.py` `restore_exit_code` 의 0 목록). 그 함수는 모르는
/// 판정도 0 으로 떨어뜨린다(catch-all) — 그래서 "exit 0" 만으로는 완료가 아니고, 이름으로 아는 것만 완료다.
const DONE_VERDICTS: [&str; 4] = ["VERIFIED", "VERIFIED_FRESH", "VERIFIED_SEAT_ONLY", "NOOP"];

pub fn classify_authority(exit: Option<i32>, result: Option<&[u8]>) -> AuthorityOutcome {
    if exit != Some(0) {
        return AuthorityOutcome::Exited(exit);
    }
    let amb = |c: &str| AuthorityOutcome::Ambiguous(c.to_string());
    let Some(bytes) = result else {
        return amb("result_missing");
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return amb("result_empty");
    }
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return amb("result_not_json");
    };
    let Some(verdict) = v.get("phoenix_restore").and_then(Value::as_str) else {
        return amb("verdict_missing");
    };
    let exit_field = v.get("exit");
    if !exit_field.is_some_and(|e| e.as_u64() == Some(0)) {
        let seen = exit_field.map_or_else(|| "missing".to_string(), |e| one_line(&e.to_string()));
        return AuthorityOutcome::Ambiguous(format!("exit_field:{seen}"));
    }
    if DONE_VERDICTS.contains(&verdict) {
        AuthorityOutcome::Done
    } else if verdict == "LEASE_HELD" {
        AuthorityOutcome::Retreat
    } else {
        AuthorityOutcome::Ambiguous(format!("verdict:{}", one_line(verdict)))
    }
}

/// 문자열 배열이면 그 값, 아니면 None(필드 부재 포함).
fn string_list(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array()?
        .iter()
        .map(|x| x.as_str().map(str::to_string))
        .collect()
}

/// 통지·로그 한 줄에 실을 데이터 — 제어 문자를 `?` 로 바꾸고 64자에서 자른다(외부 입력이 로그 줄을 쪼개지 못한다).
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { '?' } else { c }).take(64).collect()
}

#[cfg(test)]
mod plan_outcome_tests {
    use super::*;

    fn framed(json: &str) -> Vec<u8> {
        format!("log before\nphoenix-plan/1 {json}\nlog after\n").into_bytes()
    }

    const EMPTY_OK: &str = r#"{"phoenix_plan":1,"outcome":"empty","targets":[],"reasons":[],"identity":"verified","observed":{}}"#;

    fn ok(out: &[u8]) -> PlanOutcome {
        classify_plan(Ok(()), Some(0), false, out)
    }

    fn garbled(out: &[u8]) -> &'static str {
        match ok(out) {
            PlanOutcome::Unobtainable(PlanFailure::Garbled(w)) => w,
            other => panic!("expected Garbled, got {other:?}"),
        }
    }

    /// 양성 대조 — 일관된 empty 한 줄만이 건너뛰기를 허락한다. CRLF 줄끝도 같은 줄이다.
    #[test]
    fn plan_consistent_empty_permits_skip() {
        assert_eq!(ok(&framed(EMPTY_OK)), PlanOutcome::Empty);
        assert!(ok(&framed(EMPTY_OK)).permits_skip());
        let crlf = format!("a\r\nphoenix-plan/1 {EMPTY_OK}\r\nb\r\n").into_bytes();
        assert!(ok(&crlf).permits_skip(), "CRLF 줄끝의 empty 를 읽지 못했다");
    }

    /// 프로세스 사실이 출력보다 먼저다 — 출력에 완벽한 empty 가 있어도 spawn 실패·기한·신호·exit≠0 은 Unobtainable.
    #[test]
    fn process_facts_override_a_perfect_empty_line() {
        let out = framed(EMPTY_OK);
        let cases = [
            (classify_plan(Err("no such file".into()), None, false, &out),
             PlanFailure::SpawnFailed("no such file".into())),
            (classify_plan(Ok(()), Some(0), true, &out), PlanFailure::Timeout),
            (classify_plan(Ok(()), None, false, &out), PlanFailure::Signal),
            (classify_plan(Ok(()), Some(1), false, &out), PlanFailure::Exit(1)),
            (classify_plan(Ok(()), Some(2), false, &out), PlanFailure::Exit(2)),
            (classify_plan(Ok(()), Some(6), false, &out), PlanFailure::Exit(6)),
        ];
        for (got, want) in cases {
            assert_eq!(got, PlanOutcome::Unobtainable(want.clone()));
            assert!(!got.permits_skip(), "{want:?} permitted a skip");
        }
    }

    #[test]
    fn exit0_without_a_framed_line_is_no_output() {
        for out in [&b""[..], b"log only\n", b"phoenix-plan/2 {\"phoenix_plan\":2}\n", b" phoenix-plan/1 {}\n"] {
            assert_eq!(ok(out), PlanOutcome::Unobtainable(PlanFailure::NoOutput), "{out:?}");
        }
    }

    /// r1 §5-4 Garbled 행 전부 + 내 추가(outcome 과 그 근거의 모순 둘) — 하나도 건너뛰기를 허락하지 않는다.
    #[test]
    fn plan_garbled_is_not_empty() {
        let two = format!("phoenix-plan/1 {EMPTY_OK}\nphoenix-plan/1 {EMPTY_OK}\n").into_bytes();
        assert_eq!(garbled(&two), "multiple-framed-lines");
        assert_eq!(garbled(&framed("{not json")), "invalid-json");
        assert_eq!(garbled(&framed("[1]")), "not-an-object");
        let rows = [
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":["w"],"reasons":[],"identity":"verified"}"#, "empty-with-targets"),
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":[],"reasons":["r"],"identity":"verified"}"#, "empty-with-reasons"),
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":[],"reasons":[],"identity":"degraded-unverified"}"#, "empty-without-verified-identity"),
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":[],"reasons":[]}"#, "empty-without-verified-identity"),
            (r#"{"phoenix_plan":1,"outcome":"uncertified","targets":["w"],"reasons":["r"],"identity":"verified"}"#, "targets-listed-while-not-targets"),
            (r#"{"phoenix_plan":1,"outcome":"uncertified","targets":[],"reasons":[],"identity":"verified"}"#, "uncertified-without-reasons"),
            (r#"{"phoenix_plan":1,"outcome":"targets","targets":[],"reasons":[],"identity":"verified"}"#, "targets-outcome-without-targets"),
            (r#"{"phoenix_plan":1,"outcome":"error","targets":[],"reasons":[],"identity":"verified"}"#, "unknown-outcome"),
            (r#"{"phoenix_plan":1,"targets":[],"reasons":[],"identity":"verified"}"#, "unknown-outcome"),
            (r#"{"phoenix_plan":1,"outcome":"empty","reasons":[],"identity":"verified"}"#, "targets-not-a-string-list"),
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":[1],"reasons":[],"identity":"verified"}"#, "targets-not-a-string-list"),
            (r#"{"phoenix_plan":1,"outcome":"empty","targets":[],"reasons":"x","identity":"verified"}"#, "reasons-not-a-string-list"),
        ];
        for (json, want) in rows {
            assert_eq!(garbled(&framed(json)), want, "{json}");
            assert!(!ok(&framed(json)).permits_skip());
        }
    }

    /// 64 KiB 상한: 상한 안의 계획은 읽고, 상한을 넘긴 출력은 그 안에 일관된 empty 가 있어도 읽지 않는다.
    #[test]
    fn output_over_the_cap_is_garbled() {
        let mut at_cap = framed(EMPTY_OK);
        at_cap.resize(PLAN_OUTPUT_CAP, b'x');
        assert_eq!(ok(&at_cap), PlanOutcome::Empty, "상한 정확히의 출력을 거부했다");
        let mut over = at_cap.clone();
        over.push(b'x');
        assert_eq!(garbled(&over), "output-over-64KiB");
    }

    #[test]
    fn plan_protocol_skew_is_not_empty() {
        let rows = [
            (r#"{"phoenix_plan":2,"outcome":"empty","targets":[],"reasons":[],"identity":"verified"}"#, "2"),
            (r#"{"phoenix_plan":true,"outcome":"empty","targets":[],"reasons":[],"identity":"verified"}"#, "true"),
            (r#"{"phoenix_plan":1.0,"outcome":"empty","targets":[],"reasons":[],"identity":"verified"}"#, "1.0"),
            (r#"{"phoenix_plan":"1","outcome":"empty","targets":[],"reasons":[],"identity":"verified"}"#, "\"1\""),
            (r#"{"outcome":"empty","targets":[],"reasons":[],"identity":"verified"}"#, "missing"),
            // 프로토콜이 내용 검사보다 먼저다 — 다른 판의 알 수 없는 outcome 은 Garbled 가 아니라 ProtocolMismatch.
            (r#"{"phoenix_plan":2,"outcome":"novel"}"#, "2"),
        ];
        for (json, seen) in rows {
            assert_eq!(
                ok(&framed(json)),
                PlanOutcome::Unobtainable(PlanFailure::ProtocolMismatch(seen.into())),
                "{json}"
            );
        }
    }

    #[test]
    fn targets_and_uncertified_carry_their_lists() {
        let t = r#"{"phoenix_plan":1,"outcome":"targets","targets":["worker-2","cso"],"reasons":[],"identity":"degraded-unverified"}"#;
        assert_eq!(ok(&framed(t)), PlanOutcome::Targets(vec!["worker-2".into(), "cso".into()]));
        let u = r#"{"phoenix_plan":1,"outcome":"uncertified","targets":[],"reasons":["liveness_unknown","p"],"identity":"verified"}"#;
        assert_eq!(ok(&framed(u)), PlanOutcome::Uncertified(vec!["liveness_unknown".into(), "p".into()]));
        assert!(!ok(&framed(t)).permits_skip() && !ok(&framed(u)).permits_skip());
    }

    /// 데이터는 한 줄을 넘지 못한다 — 프로토콜 값은 JSON 표기(줄바꿈은 `\n` 이스케이프)로 실리고 64자에서 잘린다.
    #[test]
    fn protocol_value_is_folded_to_one_line() {
        let long = format!(r#"{{"phoenix_plan":"a\nb{}"}}"#, "z".repeat(200));
        match ok(&framed(&long)) {
            PlanOutcome::Unobtainable(PlanFailure::ProtocolMismatch(s)) => {
                assert!(!s.contains('\n') && !s.contains('\r'), "{s:?}");
                assert_eq!(s.chars().count(), 64);
            }
            other => panic!("{other:?}"),
        }
    }

    /// ★G-empty (r2 §9): 9 조합 전수 — `Free`/`Free` 만 `Empty` 가 선다. 비-Empty 입력은 그대로 통과한다.
    #[test]
    fn gate_empty_requires_free_on_both() {
        use LeaseState::{Free, Held, Unavailable};
        let held = PlanOutcome::Uncertified(vec!["restore_lock_held".into()]);
        let unav = PlanOutcome::Uncertified(vec!["restore_lock_unavailable".into()]);
        let table = [
            (Free, Free, PlanOutcome::Empty),
            (Free, Held, held.clone()),
            (Free, Unavailable, unav.clone()),
            (Held, Free, held.clone()),
            (Held, Held, held.clone()),
            (Held, Unavailable, held.clone()),
            (Unavailable, Free, unav.clone()),
            (Unavailable, Held, held.clone()),
            (Unavailable, Unavailable, unav.clone()),
        ];
        for (lease, run, want) in table {
            let got = gate_empty(PlanOutcome::Empty, lease, run);
            assert_eq!(got, want, "lease={lease:?} run={run:?}");
            assert_eq!(got.permits_skip(), lease == Free && run == Free);
        }
        let t = PlanOutcome::Targets(vec!["w".into()]);
        assert_eq!(gate_empty(t.clone(), Free, Free), t, "비-Empty 계획을 바꿨다");
        let u = PlanOutcome::Unobtainable(PlanFailure::Timeout);
        assert_eq!(gate_empty(u.clone(), Held, Held), u);
    }

    // ── classify_authority (r2 §7-1 · D3-a/b) ──────────────────────────────────────────────

    fn res(json: &str) -> AuthorityOutcome {
        classify_authority(Some(0), Some(json.as_bytes()))
    }

    /// D3-b 대조 — 이름으로 아는 완료 판정 + exit 필드 0 만 Done, LEASE_HELD 는 Retreat.
    #[test]
    fn exit0_verified_or_noop_is_done() {
        for v in DONE_VERDICTS {
            assert_eq!(res(&format!(r#"{{"phoenix_restore":"{v}","exit":0}}"#)), AuthorityOutcome::Done, "{v}");
        }
        assert_eq!(res(r#"{"phoenix_restore":"LEASE_HELD","exit":0}"#), AuthorityOutcome::Retreat);
    }

    /// ★D3-a — exit 0 이지만 읽을 수 없는 결과는 전부 Ambiguous(done 이 아니다). 실패 방향은 '재확인'이다.
    #[test]
    fn exit0_ambiguous_result_rechecks() {
        let amb = |c: &str| AuthorityOutcome::Ambiguous(c.into());
        assert_eq!(classify_authority(Some(0), None), amb("result_missing"));
        assert_eq!(classify_authority(Some(0), Some(b"")), amb("result_empty"));
        assert_eq!(classify_authority(Some(0), Some(b" \n\t")), amb("result_empty"));
        assert_eq!(res("{garbled"), amb("result_not_json"));
        assert_eq!(res(r#"["VERIFIED"]"#), amb("verdict_missing"));
        assert_eq!(res(r#"{"exit":0}"#), amb("verdict_missing"));
        assert_eq!(res(r#"{"phoenix_restore":null,"exit":0}"#), amb("verdict_missing"));
        assert_eq!(res(r#"{"phoenix_restore":"NEW_VALUE","exit":0}"#), amb("verdict:NEW_VALUE"));
        assert_eq!(res(r#"{"phoenix_restore":"FAILED","exit":0}"#), amb("verdict:FAILED"));
        assert_eq!(res(r#"{"phoenix_restore":"VERIFIED","exit":1}"#), amb("exit_field:1"));
        assert_eq!(res(r#"{"phoenix_restore":"FAILED","exit":1}"#), amb("exit_field:1"));
        assert_eq!(res(r#"{"phoenix_restore":"LEASE_HELD","exit":3}"#), amb("exit_field:3"));
        assert_eq!(res(r#"{"phoenix_restore":"VERIFIED"}"#), amb("exit_field:missing"));
        assert_eq!(res(r#"{"phoenix_restore":"VERIFIED","exit":false}"#), amb("exit_field:false"));
        assert_eq!(res(r#"{"phoenix_restore":"VERIFIED","exit":0.0}"#), amb("exit_field:0.0"));
        assert_eq!(res(r#"{"phoenix_restore":"VERIFIED","exit":"0"}"#), amb("exit_field:\"0\""));
    }

    /// exit ≠ 0·신호는 결과 파일이 무엇을 말하든 기존 처리로 간다(파일은 로그에만).
    #[test]
    fn nonzero_or_signal_keeps_the_existing_exit_handling() {
        let done = br#"{"phoenix_restore":"VERIFIED","exit":0}"#;
        assert_eq!(classify_authority(Some(1), Some(done)), AuthorityOutcome::Exited(Some(1)));
        assert_eq!(classify_authority(Some(5), None), AuthorityOutcome::Exited(Some(5)));
        assert_eq!(classify_authority(None, Some(done)), AuthorityOutcome::Exited(None));
    }

    /// 판정 값은 외부 입력이다 — 푸터 한 줄에 실리므로 줄바꿈이 접히고 64자에서 잘린다.
    #[test]
    fn verdict_cause_is_one_bounded_line() {
        let v = format!("X\r\nY{}", "z".repeat(300));
        match res(&format!(r#"{{"phoenix_restore":{},"exit":0}}"#, serde_json::to_string(&v).unwrap())) {
            AuthorityOutcome::Ambiguous(c) => {
                assert!(!c.contains('\n') && !c.contains('\r'), "{c:?}");
                assert_eq!(c.chars().count(), "verdict:".len() + 64);
            }
            other => panic!("{other:?}"),
        }
    }
}
