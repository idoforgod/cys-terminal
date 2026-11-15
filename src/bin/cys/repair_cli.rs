//! `cys repair …` — the CLI of 「스스로 해결」 (0.15.0 · C3 verbs, N-3 flag shapes · T0 skeleton).
//!
//! Every verb except the LOCAL ones (`rules`, `boot-critical`, `tombstones`, `gate`, hidden
//! `auth-helper`) is a daemon request `repair.<verb>` (C1 1.15: the CLI never writes the ledger).
//! T3 fills `report`, `begin`, `record`, `commit`, `attest-verify`, `attest-audit`, `list`, `status`,
//! `cleanup` (daemon requests) and the local `tombstones`; a failure prints one sentence on stderr and
//! NOTHING on stdout (so a hook that prints stdout prints nothing — RULINGS B10) with the O-C3-6 exit
//! code (93 + the O-C3-7 sentence for an old daemon, 98 when no daemon answers — never autostarted).
//! Every other verb still prints `"이 빌드에는 아직 구현되지 않았습니다: cys repair <verb>"` and exits
//! 99 (N-2) until its track fills it. The verb names and flags below are contract (TRACKS §6.3).
//! T1a fills the local `rules` (the role rule block, C6). The local verbs run before lane discovery
//! (`is_local` / `run_local`, called from `main()` — RULINGS Z3) with an rpc that always errors (AF4).

use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;

/// `--trigger` values (D:607; N-1 wire names in kebab-case on the command line).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TriggerArg {
    OwnerOrder,
    Grant,
    Update,
}

/// `attest-verify --verdict` values (reviewer1).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerifyVerdict {
    Pass,
    Fail,
}

/// `attest-audit --verdict` values (reviewer2).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuditVerdict {
    Ok,
    Finding,
}

/// `grant on|off|ask|status` (`ask` → `repair.grant_ask`, RULINGS Q1; `status` →
/// `repair.grant_status`, the READ of RULINGS CI6 (b)).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrantMode {
    On,
    Off,
    Ask,
    Status,
}

/// `owner-token [ask]` (`ask` → `repair.owner_token_ask`, RULINGS Q2).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnerTokenMode {
    Ask,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum RepairCmd {
    /// 결함 접수 — 데이터일 뿐 권한이 아니다(누구나)
    Report {
        /// 결함이 난 파일(팩 기준 상대 경로)
        #[arg(long)]
        file: String,
        /// 오류 내용
        #[arg(long)]
        error: String,
        #[arg(long)]
        json: bool,
    },
    /// 수리 시작 — 대상 파일·분류·잠금·고치기 전 원본 기록(워커)
    Begin {
        /// 대상 파일들(팩 기준 상대 경로) — `--unregister-hook` 만 있는 일반 수리는 생략 가능(RULINGS Z2(e))
        #[arg(long, num_args = 1.., required_unless_present = "unregister_hook")]
        files: Vec<String>,
        /// 생명줄·보호장치가 아닌 훅의 등록 해제(일반 수리에서만 — RULINGS V1)
        #[arg(long = "unregister-hook", value_name = "EVENT:SCRIPT")]
        unregister_hook: Vec<String>,
        /// 수리의 계기 — 필수(기본값 없음 · RULINGS AD10)
        #[arg(long, value_enum)]
        trigger: TriggerArg,
        /// 규칙 문서 수리의 주인 지시 표
        #[arg(long)]
        owner_token: Option<String>,
        /// 이 부서에서 충돌을 합치는 병합 모드(H8 — 기동 필수·보호장치 경로는 거부)
        #[arg(long, value_name = "R-ID")]
        joins: Option<String>,
        /// 다른 수리의 충돌을 이 부서에서 푸는 수리(RULINGS T1)
        #[arg(long, value_name = "R-ID")]
        resolves: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 변경분·시도 기록(잠금을 쥔 워커)
    Record {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long)]
        note: String,
        #[arg(long)]
        json: bool,
    },
    /// 기동 필수·보호장치 관련 후보 제출 — 번호만 돌려주고 바로 끝난다(잠금을 쥔 워커)
    Submit {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long)]
        json: bool,
    },
    /// 확정 요청 — 데몬이 서명·살아 있는 파일 검사 뒤 확정한다(잠금을 쥔 워커)
    Commit {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long)]
        json: bool,
    },
    /// 검증 서명(리뷰어1 — 자기 수리에는 불가)
    AttestVerify {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long, value_enum)]
        verdict: VerifyVerdict,
        #[arg(long)]
        note: String,
    },
    /// 감사 서명(리뷰어2)
    AttestAudit {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long, value_enum)]
        verdict: AuditVerdict,
        #[arg(long)]
        note: String,
    },
    /// 수리 목록(읽기)
    List {
        #[arg(long)]
        json: bool,
    },
    /// 수리 상태·보관소 칸·부서별 상태(읽기)
    Status {
        #[arg(value_name = "R-ID")]
        repair: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 그 수리 한 건 되돌리기(사람 · 마스터 티켓을 받은 워커 — 마스터는 불가)
    Revert {
        #[arg(value_name = "R-ID")]
        repair: String,
    },
    /// 업데이트로 보류된 기동 필수 수리를 다시 입히기(승인 경유 · 워커)
    Reapply {
        #[arg(value_name = "R-ID")]
        repair: String,
    },
    /// 워커 검토 뒤 보관소 칸 정리
    Cleanup {
        #[arg(value_name = "R-ID")]
        repair: String,
        #[arg(long, value_name = "시각")]
        slot: Option<String>,
    },
    /// 자동 수리 허락 켜기·끄기·질문 등록(마스터 · 주인 입력 확인 뒤에만 효력) · 상태 보기(status · 누구나)
    Grant {
        #[arg(value_enum)]
        mode: GrantMode,
        /// 하루 상한(1~100) — `grant ask` 에서만: 질문에 딸려 저장되고, 지금 상한과 다른 수는 주인이 직접 친 말에 그 숫자가 있어야 한다(`on`·`off`·`status`는 받지 않음)
        #[arg(long, value_name = "N")]
        daily_cap: Option<u32>,
        /// 결과를 JSON 한 줄로(`status`: {state, paused, paused_until, daily_cap})
        #[arg(long)]
        json: bool,
    },
    /// 규칙 문서 수리의 주인 지시 표 발급·질문 등록(마스터 · 주인 입력 확인 뒤에만 효력)
    OwnerToken {
        #[arg(value_enum)]
        mode: Option<OwnerTokenMode>,
        #[arg(long)]
        path: PathBuf,
        #[arg(long, value_name = "R-ID")]
        repair: Option<String>,
    },
    /// 이 문서를 지금 고쳐도 되는가(읽기)
    RuleDocAllowed {
        #[arg(long)]
        path: PathBuf,
    },
    /// 승인되지 않은 변경 검사 요청
    Sweep {
        #[arg(long)]
        json: bool,
        /// RULINGS GK3: confirm the notice keys taken from an earlier sweep reply (repeatable)
        #[arg(long, value_name = "KEY")]
        confirm: Vec<String>,
    },
    /// 이 부서에서 합친 뒤 장부의 검사를 다시 돌리기(병합 잠금을 쥔 워커)
    Recheck {
        #[arg(value_name = "R-ID")]
        repair: String,
    },
    /// 내장 예약 작업의 한 번 실행(내부용)
    #[command(hide = true)]
    Tick,
    /// 역할별 수리 규칙 출력(로컬 — 데몬을 부르지 않는다)
    Rules {
        #[arg(long)]
        role: String,
    },
    /// 기동 필수 목록 조회(로컬)
    BootCritical {
        #[arg(long, conflicts_with = "list", required_unless_present = "list")]
        path: Option<PathBuf>,
        #[arg(long)]
        list: bool,
        #[arg(long)]
        json: bool,
    },
    /// 이 레인의 묘비 조회(로컬)
    Tombstones {
        #[arg(long)]
        json: bool,
    },
    /// 작업 배정 관문 판정(로컬 — 0 열림 · 95 보류)
    Gate {
        #[arg(long)]
        lane: String,
        #[arg(long)]
        json: bool,
    },
    /// 운영체제 사용자 확인 도우미(데몬이 띄운다 · 내부용)
    #[command(hide = true)]
    AuthHelper {
        #[arg(long)]
        request: String,
        #[arg(long)]
        reason: String,
    },
}

/// The command-line name of a verb (exhaustive: a new variant without a name does not compile).
fn verb_name(cmd: &RepairCmd) -> &'static str {
    match cmd {
        RepairCmd::Report { .. } => "report",
        RepairCmd::Begin { .. } => "begin",
        RepairCmd::Record { .. } => "record",
        RepairCmd::Submit { .. } => "submit",
        RepairCmd::Commit { .. } => "commit",
        RepairCmd::AttestVerify { .. } => "attest-verify",
        RepairCmd::AttestAudit { .. } => "attest-audit",
        RepairCmd::List { .. } => "list",
        RepairCmd::Status { .. } => "status",
        RepairCmd::Revert { .. } => "revert",
        RepairCmd::Reapply { .. } => "reapply",
        RepairCmd::Cleanup { .. } => "cleanup",
        RepairCmd::Grant { .. } => "grant",
        RepairCmd::OwnerToken { .. } => "owner-token",
        RepairCmd::RuleDocAllowed { .. } => "rule-doc-allowed",
        RepairCmd::Sweep { .. } => "sweep",
        RepairCmd::Recheck { .. } => "recheck",
        RepairCmd::Tick => "tick",
        RepairCmd::Rules { .. } => "rules",
        RepairCmd::BootCritical { .. } => "boot-critical",
        RepairCmd::Tombstones { .. } => "tombstones",
        RepairCmd::Gate { .. } => "gate",
        RepairCmd::AuthHelper { .. } => "auth-helper",
    }
}

/// Runs a `cys repair` verb and returns the process exit code. A `cys repair` never starts a
/// daemon (appendix A 1-17 — a hook's `cys` must not; exit 98 when none answers): autostart is
/// switched off for this process before the first request.
pub(crate) fn run(cmd: RepairCmd) -> i32 {
    super::AUTOSTART.store(false, std::sync::atomic::Ordering::SeqCst);
    let rpc = |method: &str, params: Value| super::request(method, params);
    run_with(&cmd, &mut std::io::stdout(), &mut std::io::stderr(), &rpc)
}

/// The LOCAL verbs (C3 O-C3-1): they never send a daemon request. `main()` runs them through
/// [`run_local`] BEFORE lane discovery, so not even `system.identify` reaches a daemon (RULINGS Z3 ·
/// B10 · appendix A 1-17).
pub(crate) fn is_local(cmd: &RepairCmd) -> bool {
    matches!(
        cmd,
        RepairCmd::Rules { .. }
            | RepairCmd::BootCritical { .. }
            | RepairCmd::Tombstones { .. }
            | RepairCmd::Gate { .. }
            | RepairCmd::AuthHelper { .. }
    )
}

/// The rpc a LOCAL verb runs with: it ALWAYS errors, so a local verb can never send a request
/// (RULINGS AF4). A local verb that tried would fail closed (exit 98, nothing on stdout).
fn local_rpc(method: &str, _params: Value) -> Result<Value, String> {
    Err(format!("local verb: no daemon request is sent ({method})"))
}

/// [`run`] by reference for the early return of `main()` (RULINGS Z3) — the same dispatch, with
/// [`local_rpc`] in place of the daemon.
pub(crate) fn run_local(cmd: &RepairCmd) -> i32 {
    super::AUTOSTART.store(false, std::sync::atomic::Ordering::SeqCst);
    run_with(cmd, &mut std::io::stdout(), &mut std::io::stderr(), &local_rpc)
}

/// `cys repair rules --role <role>` (C6 O-C6-1, RULINGS B10 — T1a): prints the role's compiled rule
/// block (`rules_text(role_block_for(role))`, the same text `compose_directive` and the `[RESUME]` text
/// carry) to `out` and exits 0. LOCAL and pure: no daemon, no file, no environment read. On any error
/// NOTHING goes to `out` (the session-start hook prints `out` only on exit 0), the reason goes to `err`.
fn run_rules(role: &str, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let block = cys::repair::roles::role_block_for(role);
    match cys::repair::rules::rules_text(block) {
        Ok(text) => match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
            Ok(()) => 0,
            Err(e) => {
                let _ = writeln!(err, "cys repair rules: 출력 실패: {e}");
                1
            }
        },
        Err(e) => {
            let _ = writeln!(err, "cys repair rules: {e:?}");
            e.exit_code()
        }
    }
}

/// O-C3-7: a new CLI meeting an old daemon fails closed with this sentence and exit 93.
pub(crate) const OLD_DAEMON_KO: &str = "이 데몬은 `cys repair`를 모릅니다(0.15.0 미만) — 아무것도 기록되지 않았습니다. 프로그램을 0.15.0 이상으로 업데이트한 뒤 다시 하세요.";

/// The daemon error codes of the `repair.*` namespace (O-C3-6 + Z1) → CLI exit codes.
fn exit_for(code: &str) -> Option<i32> {
    use cys::repair::types as t;
    Some(match code {
        "acl_denied" => t::EXIT_REPAIR_ROLE_REFUSED,
        "method_not_found" => t::EXIT_REPAIR_OLD_DAEMON,
        c if c == t::ERR_LOCKED || c == "target_changed" => t::EXIT_REPAIR_LOCKED,
        c if c == t::ERR_PRECONDITION || c == t::ERR_CORRUPT || c == "not_found" => {
            t::EXIT_REPAIR_PRECONDITION
        }
        c if c == t::ERR_CHECK_FAILED => t::EXIT_REPAIR_CHECK_FAILED,
        c if c == t::ERR_PLATFORM_PENDING => t::EXIT_REPAIR_PLATFORM_PENDING,
        c if c == t::ERR_NOT_IMPLEMENTED => t::EXIT_REPAIR_NOT_IMPLEMENTED,
        "invalid_params" => 2,
        "io" => 1,
        _ => return None,
    })
}

/// Turns a failed request (`"<code>: <message>"` from the daemon, or a transport error) into the
/// sentence on `err` and the exit code. Nothing is written on stdout.
fn fail(err: &mut dyn Write, e: &str) -> i32 {
    let (code, msg) = e.split_once(": ").unwrap_or(("", e));
    match exit_for(code) {
        Some(93) => {
            let _ = writeln!(err, "{OLD_DAEMON_KO}");
            93
        }
        Some(x) => {
            let _ = writeln!(err, "{msg}");
            x
        }
        None => {
            let _ = writeln!(
                err,
                "cysd 에 닿지 못했습니다(cys repair 는 데몬을 자동으로 띄우지 않습니다): {e}"
            );
            cys::repair::types::EXIT_REPAIR_DAEMON_UNREACHABLE
        }
    }
}

type Rpc<'a> = &'a dyn Fn(&str, Value) -> Result<Value, String>;

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn short(h: &str) -> &str {
    h.get(..12).unwrap_or(h)
}

fn lanes_line(v: &Value) -> String {
    match v.get("lanes").and_then(|l| l.as_object()) {
        Some(m) if !m.is_empty() => m
            .iter()
            .map(|(k, st)| format!("{k}={}", st.as_str().unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(" · "),
        _ => "-".into(),
    }
}

/// Writes the success of a daemon verb: the JSON reply with `--json`, else readable lines.
fn print_ok(out: &mut dyn Write, verb: &str, v: &Value, json_out: bool) {
    if json_out {
        let _ = writeln!(out, "{v}");
        return;
    }
    let id = s(v, "repair");
    let _ = match verb {
        "report" => {
            if v.get("excluded").is_some_and(|x| !x.is_null()) {
                writeln!(
                    out,
                    "접수하지 않음(설계상 늘 실패하는 동작 제외 목록: {}) — {}",
                    s(v, "excluded"),
                    s(v, "rel")
                )
            } else if v.get("over_quota") == Some(&Value::Bool(true)) {
                writeln!(
                    out,
                    "접수 한도 초과 — 이번 한 시간 새 결함 접수가 다 찼습니다(기록 안 됨): {}",
                    s(v, "rel")
                )
            } else {
                let joined = s(v, "joined");
                writeln!(
                    out,
                    "결함 접수: {} ({}) — 같은 결함 {}건째 · 서명 {}{}",
                    s(v, "rel"),
                    s(v, "error_class"),
                    s(v, "count"),
                    short(&s(v, "signature")),
                    if joined.is_empty() {
                        String::new()
                    } else {
                        format!(" · 열린 수리 {joined} 에 합류")
                    }
                )
            }
        }
        "begin" if v.get("begin").and_then(|b| b.as_str()) == Some("joins") => writeln!(
            out,
            "{id}: 이 부서({})에서 합치기 시작 — 합친 뒤 `cys repair recheck {id}`",
            s(v, "lane")
        ),
        "begin" => {
            let mut r = writeln!(
                out,
                "수리 {id} {} — {} · 시도 {}/{} · 상태 {}",
                match v.get("begin").and_then(|b| b.as_str()) {
                    Some("picked-up") => "다시 잡음",
                    Some("already-held") => "이미 쥐고 있음",
                    _ => "시작",
                },
                s(v, "classification"),
                s(v, "attempt"),
                s(v, "max_rounds"),
                s(v, "state_label"),
            );
            if let Some(st) = v.get("staging").and_then(|x| x.as_str()) {
                r = r.and_then(|_| writeln!(out, "작업 사본: {st} — 후보는 여기서만 고칩니다"));
            }
            r
        }
        "record" => writeln!(
            out,
            "{id} 기록 — 지문 {} · 시도 {}/{}{}",
            short(&s(v, "fingerprint")),
            s(v, "attempt"),
            s(v, "max_rounds"),
            if v.get("attestations_voided") == Some(&Value::Bool(true)) {
                " · 이전 서명은 무효(다시 받아야 함)"
            } else {
                ""
            }
        ),
        "commit" => {
            let mut r = writeln!(
                out,
                "{id} 확정 — {} · 부서별: {}",
                s(v, "state_label"),
                lanes_line(v)
            );
            // RULINGS AK4: older repairs of the same files now superseded; AK3: warnings
            let ids = |k: &str| -> Vec<String> {
                v.get(k)
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let sup = ids("superseded");
            if r.is_ok() && !sup.is_empty() {
                r = writeln!(out, "  같은 파일의 이전 수리 대체됨: {}", sup.join(", "));
            }
            for w in ids("warnings") {
                if r.is_ok() {
                    r = writeln!(out, "  경고: {w}");
                }
            }
            r
        }
        "attest-verify" | "attest-audit" => {
            let reopened = s(v, "reopened");
            writeln!(
                out,
                "{id} {} 서명({}) — 상태 {}{}",
                if verb == "attest-verify" {
                    "검증"
                } else {
                    "감사"
                },
                s(v, "verdict"),
                s(v, "state_label"),
                if reopened.is_empty() {
                    String::new()
                } else {
                    format!(" · 다시 열림: {reopened}")
                }
            )
        }
        "list" => {
            let mut r = Ok(());
            for rep in v
                .get("repairs")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
            {
                r = r.and_then(|_| {
                    writeln!(
                        out,
                        "{}\t{}\t{}\t{}/{}\t{}",
                        s(rep, "repair"),
                        s(rep, "state_label"),
                        s(rep, "classification_label"),
                        s(rep, "attempt"),
                        s(rep, "max_rounds"),
                        rep.get("files")
                            .and_then(|f| f.as_array())
                            .map(|f| f
                                .iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(","))
                            .unwrap_or_default()
                    )
                });
            }
            for c in v
                .get("corrupt")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
            {
                r = r.and_then(|_| {
                    writeln!(out, "{}\t읽을 수 없음\t{}", s(c, "repair"), s(c, "corrupt"))
                });
            }
            r.and_then(|_| writeln!(out, "열린 결함 {}건", s(v, "open_defects")))
        }
        // RULINGS DA6: a record that cannot be read is named with its diagnosis (the daemon answers
        // `{repair, corrupt}` for it) — never printed as a record with empty fields
        "status" if v.get("corrupt").is_some_and(|c| c.is_string()) => writeln!(
            out,
            "{id}: 수리 기록을 읽을 수 없습니다 — {}",
            s(v, "corrupt")
        ),
        "status" if v.get("repair").is_some() => writeln!(
            out,
            "{id}: {} · {} · 시도 {}/{} · 부서별: {}{}",
            s(v, "state_label"),
            s(v, "classification_label"),
            s(v, "attempt"),
            s(v, "max_rounds"),
            lanes_line(v),
            match v.get("propagation_complete") {
                Some(Value::Bool(true)) => " · 전파 완료".to_string(),
                // T6b · RULINGS HS2: why it is not complete (a repair not in force) is said
                Some(Value::Bool(false)) => match v.get("propagation_note").and_then(|n| n.as_str()).filter(|n| !n.is_empty()) {
                    Some(n) => format!(" · 전파 미완({n})"),
                    None => " · 전파 미완".to_string(),
                },
                _ => format!(" · 전파 완료 판정 불가({})", s(v, "propagation_note")),
            }
        ),
        "status" => writeln!(
            out,
            "레인 {} — 수리 {}건 · 열린 결함 {}건{}",
            s(v, "lane"),
            s(v, "repairs"),
            s(v, "open_defects"),
            if v.pointer("/reviewer_slots/waiting") == Some(&Value::Bool(true)) {
                " · 리뷰어 칸이 비어 있어 수리가 기다립니다"
            } else {
                ""
            }
        )
        .and_then(|_| status_notice_lines(out, v)),
        "cleanup" => writeln!(
            out,
            "{id}: 보관소 칸 정리 {}개 · 거부 {}개",
            v.get("removed")
                .and_then(|x| x.as_array())
                .map_or(0, |a| a.len()),
            v.get("refused")
                .and_then(|x| x.as_array())
                .map_or(0, |a| a.len())
        ),
        _ => writeln!(out, "{v}"),
    };
}

/// The daemon request of a T3 verb: (wire method, params, `--json`). `None` = not a T3 daemon verb.
fn t3_request(cmd: &RepairCmd) -> Option<(&'static str, Value, bool)> {
    Some(match cmd {
        RepairCmd::Report { file, error, json } => (
            "repair.report",
            json!({"file": file, "error": error}),
            *json,
        ),
        RepairCmd::Begin {
            files,
            unregister_hook,
            trigger,
            owner_token,
            joins,
            resolves,
            json,
        } => {
            let mut p = json!({"files": files, "unregister_hook": unregister_hook});
            p["trigger"] = json!(match trigger {
                TriggerArg::OwnerOrder => "owner_order",
                TriggerArg::Grant => "grant",
                TriggerArg::Update => "update",
            });
            if let Some(o) = owner_token {
                p["owner_token"] = json!(o);
            }
            if let Some(j) = joins {
                p["joins"] = json!(j);
            }
            if let Some(r) = resolves {
                p["resolves"] = json!(r);
            }
            ("repair.begin", p, *json)
        }
        RepairCmd::Record { repair, note, json } => (
            "repair.record",
            json!({"repair": repair, "note": note}),
            *json,
        ),
        RepairCmd::Commit { repair, json } => ("repair.commit", json!({"repair": repair}), *json),
        RepairCmd::AttestVerify {
            repair,
            verdict,
            note,
        } => (
            "repair.attest_verify",
            json!({"repair": repair, "note": note, "verdict": match verdict { VerifyVerdict::Pass => "pass", VerifyVerdict::Fail => "fail" }}),
            false,
        ),
        RepairCmd::AttestAudit {
            repair,
            verdict,
            note,
        } => (
            "repair.attest_audit",
            json!({"repair": repair, "note": note, "verdict": match verdict { AuditVerdict::Ok => "ok", AuditVerdict::Finding => "finding" }}),
            false,
        ),
        RepairCmd::List { json } => ("repair.list", json!({}), *json),
        RepairCmd::Status { repair, json } => ("repair.status", json!({"repair": repair}), *json),
        RepairCmd::Cleanup { repair, slot } => (
            "repair.cleanup",
            json!({"repair": repair, "slot": slot}),
            false,
        ),
        _ => return None,
    })
}

/// The daemon request of a T6 verb (C3 `revert`, `reapply`; C7 `sweep`, hidden `tick`): (wire
/// method, params). `None` = not a T6 verb.
fn t6_request(cmd: &RepairCmd) -> Option<(&'static str, Value)> {
    Some(match cmd {
        RepairCmd::Revert { repair } => ("repair.revert", json!({"repair": repair})),
        RepairCmd::Reapply { repair } => ("repair.reapply", json!({"repair": repair})),
        RepairCmd::Sweep { confirm, .. } if !confirm.is_empty() => ("repair.sweep", json!({"confirm": confirm})),
        RepairCmd::Sweep { .. } => ("repair.sweep", json!({})),
        RepairCmd::Tick => ("repair.tick", json!({})),
        _ => return None,
    })
}

/// The printed result of a T6 verb (a refusal is the daemon's sentence on stderr — `fail`).
/// T6 · RULINGS GK3 / GV3 (CCR-T6-2 — the status rendering only): the owner notices closed "not
/// confirmed" (handed out three times without their receiver's confirmation) and a notice state the
/// daemon could not read are printed in the ordinary output of `cys repair status`.
fn status_notice_lines(out: &mut dyn Write, v: &Value) -> std::io::Result<()> {
    match v.get("notices_not_confirmed") {
        Some(Value::Array(keys)) if !keys.is_empty() => {
            writeln!(out, "받았다는 확인이 없는 알림 {}건(세 번 건넸고 더는 건네지 않음):", keys.len())?;
            for k in keys {
                writeln!(out, "  {}", k.as_str().unwrap_or("?"))?;
            }
            Ok(())
        }
        Some(Value::Object(o)) if o.contains_key("unreadable") => writeln!(
            out,
            "알림 상태를 읽을 수 없습니다 — {}",
            o.get("unreadable").and_then(|x| x.as_str()).unwrap_or("?")
        ),
        _ => Ok(()),
    }
}

fn t6_print(cmd: &RepairCmd, out: &mut dyn Write, v: &Value) -> i32 {
    let r = match cmd {
        RepairCmd::Sweep { json: true, .. } => writeln!(out, "{v}"),
        RepairCmd::Revert { .. } => {
            let id = s(v, "repair");
            if v.get("reverted") == Some(&Value::Bool(true)) {
                writeln!(out, "{id} 를 되돌렸습니다 — 고친 사본은 보관소에 남아 있습니다(cys repair status {id})")
            } else {
                writeln!(
                    out,
                    "{id} 는 되돌리지 않았습니다 — {}",
                    v.get("result").map(|r| s(r, "why")).unwrap_or_default()
                )
            }
        }
        RepairCmd::Reapply { .. } => {
            let id = s(v, "repair");
            let mut r = writeln!(
                out,
                "수리 {id} 시작 — {} 의 보관된 수정본을 다시 입힙니다({} · 계기: 업데이트 · 시도 {}/{})",
                s(v, "reapplies"),
                s(v, "classification"),
                s(v, "attempt"),
                s(v, "max_rounds"),
            );
            if let Some(st) = v.get("staging").and_then(|x| x.as_str()) {
                r = r.and_then(|_| writeln!(out, "작업 사본: {st} — 보관된 수정본을 여기서 지금 판과 합칩니다"));
            }
            for p in v.get("parked").and_then(|x| x.as_array()).into_iter().flatten() {
                r = r.and_then(|_| {
                    writeln!(out, "보관된 수정본: {} → {}", s(p, "parked_copy"), s(p, "rel"))
                });
            }
            r.and_then(|_| {
                writeln!(
                    out,
                    "합친 뒤: cys repair record {id} → 검증·감사 서명 → cys repair submit {id}(기동 필수·보호장치 — 주인 최종 승인) 또는 cys repair commit {id}(일반)"
                )
            })
        }
        RepairCmd::Sweep { .. } => {
            let mut r = writeln!(out, "미승인 변경 검사: {}", s(v, "tick"));
            for n in v.get("notices").and_then(|x| x.as_array()).into_iter().flatten() {
                r = r.and_then(|_| writeln!(out, "  {} — {} ({})", s(n, "rel"), s(n, "note"), s(n, "kind")));
            }
            r
        }
        _ => writeln!(out, "{v}"),
    };
    let _ = r;
    0
}

/// The daemon request of a T4 verb (C9 grant, C10 owner token): (wire method, params). `None` = not
/// a T4 verb. `grant ask` / `owner-token ask` are their own requests (RULINGS Q1 / Q2).
fn t4_request(cmd: &RepairCmd) -> Option<(&'static str, Value)> {
    let path_of = |p: &PathBuf| p.to_string_lossy().to_string();
    Some(match cmd {
        RepairCmd::Grant {
            mode: GrantMode::Ask,
            daily_cap,
            ..
        } => ("repair.grant_ask", json!({"daily_cap": daily_cap})),
        // RULINGS CI6 (b); a `--daily-cap` here is forwarded so the daemon refuses it in one place
        RepairCmd::Grant {
            mode: GrantMode::Status,
            daily_cap,
            ..
        } => ("repair.grant_status", json!({"daily_cap": daily_cap})),
        // CI4: forwarded as given — the daemon refuses `--daily-cap` on `on` / `off` with the
        // sentence naming `grant ask --daily-cap` (never silently dropped)
        RepairCmd::Grant {
            mode, daily_cap, ..
        } => (
            "repair.grant",
            json!({"mode": if *mode == GrantMode::On { "on" } else { "off" }, "daily_cap": daily_cap}),
        ),
        RepairCmd::OwnerToken {
            mode: Some(OwnerTokenMode::Ask),
            path,
            ..
        } => ("repair.owner_token_ask", json!({"path": path_of(path)})),
        RepairCmd::OwnerToken {
            mode: None,
            path,
            repair,
        } => (
            "repair.owner_token",
            json!({"path": path_of(path), "repair": repair}),
        ),
        RepairCmd::RuleDocAllowed { path } => {
            ("repair.rule_doc_allowed", json!({"path": path_of(path)}))
        }
        _ => return None,
    })
}

/// Writes the success of a T4 verb and returns the exit code: `rule-doc-allowed` exits 0 only when
/// the daemon allows (the guard hook reads only the exit code), 95 otherwise with the reason on
/// stderr and nothing on stdout.
fn t4_print(cmd: &RepairCmd, out: &mut dyn Write, err: &mut dyn Write, v: &Value) -> i32 {
    let r = match cmd {
        RepairCmd::Grant { json: true, .. } => writeln!(out, "{v}"),
        RepairCmd::Grant {
            mode: GrantMode::Ask,
            ..
        } => {
            // CJ5: the daemon's sentence names the limit; an older reply without it is rebuilt here
            let ask = match v.get("ask_owner").and_then(Value::as_str) {
                Some(a) => a.to_string(),
                None => match v.get("daily_cap").and_then(Value::as_u64) {
                    Some(n) => cys::repair::owner_origin::grant_question_ko(n as u32),
                    None => cys::repair::owner_origin::GRANT_QUESTION_KO.to_string(),
                },
            };
            writeln!(
                out,
                "질문을 등록했습니다(하루 상한 {}건) — 이제 주인께 대화로 물으세요: \"{ask}\" (30분 안에 주인이 직접 친 첫 답만 셉니다)",
                s(v, "daily_cap")
            )
        }
        RepairCmd::Grant {
            mode: GrantMode::Status,
            ..
        } => {
            if v.get("state") == Some(&json!("on")) {
                let pause = if v.get("paused") == Some(&Value::Bool(true)) {
                    let until = v
                        .get("paused_until")
                        .and_then(Value::as_f64)
                        .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
                        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
                        .unwrap_or_else(|| "-".into());
                    format!("일시정지 중({until}까지 · 주인의 입력 뒤 적어도 30분)")
                } else {
                    "일시정지 아님".to_string()
                };
                writeln!(
                    out,
                    "자동 수리 허락: 켬 · 하루 상한 {}건 · {pause}",
                    s(v, "daily_cap")
                )
            } else {
                writeln!(out, "자동 수리 허락: 끔 (하루 상한 {}건)", s(v, "daily_cap"))
            }
        }
        RepairCmd::Grant { .. } => {
            if v.get("grant") == Some(&json!("on")) {
                writeln!(
                    out,
                    "자동 수리 허락: 켬 (하루 상한 {}건 · 주인의 입력이 있으면 그 뒤 적어도 30분 쉼)",
                    s(v, "daily_cap")
                )
            } else {
                writeln!(out, "자동 수리 허락: 끔")
            }
        }
        RepairCmd::OwnerToken {
            mode: Some(OwnerTokenMode::Ask),
            ..
        } => writeln!(
            out,
            "질문을 등록했습니다(규칙 문서 {}) — 주인께 이 문서를 고쳐도 되는지 물으세요(30분 안에 주인이 직접 친 첫 답만 셉니다)",
            s(v, "path")
        ),
        RepairCmd::OwnerToken { .. } => writeln!(
            out,
            "{}\n이 표는 지금 한 번만 보입니다 — 유효 24시간 · 이 부서 워커의 티켓에 그대로 붙이세요(`cys repair begin --owner-token …`)",
            s(v, "ticket_line")
        ),
        RepairCmd::RuleDocAllowed { .. } => {
            if v.get("allowed") == Some(&Value::Bool(true)) {
                writeln!(out, "허용: {} ({})", s(v, "path"), s(v, "repair"))
            } else {
                let _ = writeln!(err, "허용하지 않음: {} — {}", s(v, "path"), s(v, "reason"));
                return cys::repair::types::EXIT_REPAIR_PRECONDITION;
            }
        }
        _ => Ok(()),
    };
    let _ = r;
    0
}

/// O-C1-8: `cys repair tombstones [--json]` — a LOCAL read of this lane's tombstone file (no daemon);
/// the lane is the caller's socket (`CYS_SOCKET`, as `lane_key` does). An unreadable file → the
/// sentence on stderr and exit 95, nothing on stdout (a reader then behaves as today: no tombstones).
/// `cys repair gate --lane <lane_key> [--json]` (T7b · RULINGS I1 / I7 / I8 / P5 · HH7): the LOCAL
/// assignment gate — reads the ledger and the lane's job state, sends nothing, starts nothing; exit 0
/// OPEN, 95 HELD (flagged interpretation 19), 2 a lane key that is not one. The JSON also carries the
/// one summary line of lanes left in 충돌 (HH7 — a pull; the delegation tool prints it).
fn gate_local(lane: &str, json_out: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    gate_local_in(lane, json_out, out, err, &cys::repair::paths::repairs_root(), &cys::lane::registry_path())
}

fn gate_local_in(lane: &str, json_out: bool, out: &mut dyn Write, err: &mut dyn Write, root: &std::path::Path, registry: &std::path::Path) -> i32 {
    let valid = !lane.is_empty()
        && lane != "."
        && lane != ".."
        && lane.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !valid {
        let _ = writeln!(err, "cys repair gate: 레인 키가 아닙니다: {lane:?} (영문·숫자·. _ - 만)");
        return 2;
    }
    let key = cys::repair::types::LaneKey(lane.to_string());
    let verdict = cys::repair::propagate::gate_verdict_in(root, &key);
    let summary = cys::repair::propagate::conflict_summary_line_in(root, registry);
    let (open, cause) = match &verdict {
        cys::repair::ledger::GateVerdict::Open => (true, None),
        cys::repair::ledger::GateVerdict::Held(c) => (false, Some(c)),
    };
    if json_out {
        let cause_json = cause.map(|c| {
            let (pending, conflict, rows) = match c {
                cys::repair::ledger::HoldCause::Pending { pending, conflict } => (Some(*pending), Some(*conflict), None),
                cys::repair::ledger::HoldCause::AwaitingFinalApproval { rows } => (None, None, Some(*rows)),
                _ => (None, None, None),
            };
            json!({"kind": cys::repair::propagate::hold_kind(c), "text": cys::repair::propagate::hold_text(c), "pending": pending, "conflict": conflict, "rows": rows})
        });
        let _ = writeln!(
            out,
            "{}",
            json!({"lane": lane, "verdict": if open { "open" } else { "held" }, "cause": cause_json, "conflict_summary": summary})
        );
    } else {
        match cause {
            None => {
                let _ = writeln!(out, "작업 배정 관문: 열림 ({lane})");
            }
            Some(c) => {
                let _ = writeln!(out, "작업 배정 관문: 보류 ({lane}) — {}", cys::repair::propagate::hold_text(c));
            }
        }
        if let Some(line) = &summary {
            let _ = writeln!(out, "{line}");
        }
    }
    if open {
        0
    } else {
        cys::repair::types::EXIT_REPAIR_PRECONDITION
    }
}

fn tombstones_local(out: &mut dyn Write, err: &mut dyn Write, json_out: bool) -> i32 {
    let lane = cys::repair::types::LaneKey::of_socket(&super::socket_path());
    tombstones_local_in(
        out,
        err,
        json_out,
        lane,
        &cys::repair::paths::repairs_root(),
    )
}

fn tombstones_local_in(
    out: &mut dyn Write,
    err: &mut dyn Write,
    json_out: bool,
    lane: cys::repair::types::LaneKey,
    root: &std::path::Path,
) -> i32 {
    match cys::repair::tombstones::load_in(root, &lane) {
        Ok(list) => {
            if json_out {
                let f = cys::repair::tombstones::TombstoneFile {
                    schema: cys::repair::REPAIR_SCHEMA,
                    lane,
                    tombstones: list,
                };
                let _ = writeln!(
                    out,
                    "{}",
                    serde_json::to_string(&f).unwrap_or_else(|_| "{}".into())
                );
            } else {
                for t in &list {
                    let _ = writeln!(out, "{}\t{}", t.rel, t.repair.as_str());
                }
            }
            0
        }
        Err(e) => {
            let _ = writeln!(err, "이 레인의 묘비 파일을 읽을 수 없습니다(지운 것이 업데이트로 되돌아올 수 있음): {e:?}");
            cys::repair::types::EXIT_REPAIR_PRECONDITION
        }
    }
}

/// Runs a verb against `rpc` (the daemon in production, a fake in the tests). A failing stderr write
/// is ignored (a closed stderr must not turn into a panic).
fn run_with(cmd: &RepairCmd, out: &mut dyn Write, err: &mut dyn Write, rpc: Rpc<'_>) -> i32 {
    if let RepairCmd::Rules { role } = cmd {
        return run_rules(role, out, err);
    }
    if let RepairCmd::Tombstones { json } = cmd {
        return tombstones_local(out, err, *json);
    }
    // T5a — LOCAL query of the compiled boot-critical list (C4 O-C4-2; no daemon request).
    if let RepairCmd::BootCritical { path, list, json } = cmd {
        return cys::repair::boot_critical::run_query(path.as_deref(), *list, *json, out, err);
    }
    if let Some((method, params, json_out)) = t3_request(cmd) {
        return match rpc(method, params) {
            Ok(v) => {
                print_ok(out, verb_name(cmd), &v, json_out);
                0
            }
            Err(e) => fail(err, &e),
        };
    }
    if let Some((method, params)) = t4_request(cmd) {
        return match rpc(method, params) {
            Ok(v) => t4_print(cmd, out, err, &v),
            Err(e) => fail(err, &e),
        };
    }
    if let Some((method, params)) = t6_request(cmd) {
        return match rpc(method, params) {
            Ok(v) => t6_print(cmd, out, &v),
            Err(e) => fail(err, &e),
        };
    }
    // Track T5b — the hidden OS-verification helper (LOCAL: no daemon request; spawned by the daemon)
    if let RepairCmd::AuthHelper { request, reason } = cmd {
        // RULINGS JW1: the request's one-time nonce comes in the environment the daemon set
        let nonce = std::env::var(auth_helper::NONCE_VAR).ok();
        return auth_helper::run(request, reason, nonce.as_deref(), out, err);
    }
    // Track T5b — `submit` returns the request number at once (C5 5.6; no waiting anywhere)
    if let RepairCmd::Submit { repair, json } = cmd {
        return match rpc("repair.submit", serde_json::json!({ "repair": repair })) {
            Ok(v) => {
                if *json {
                    let _ = writeln!(out, "{v}");
                } else {
                    let _ = writeln!(
                        out,
                        "{}: 제출 {} — {}",
                        s(&v, "repair"),
                        s(&v, "request_id"),
                        s(&v, "note")
                    );
                }
                0
            }
            Err(e) => fail(err, &e),
        };
    }
    // T7b — the LOCAL assignment gate (I1: no daemon request) and the daemon's `recheck` (H8).
    if let RepairCmd::Gate { lane, json } = cmd {
        return gate_local(lane, *json, out, err);
    }
    if let RepairCmd::Recheck { repair } = cmd {
        return match rpc("repair.recheck", json!({"repair": repair})) {
            Ok(v) => {
                let _ = writeln!(
                    out,
                    "{}: 이 부서({})에서 다시 검사 — {}",
                    s(&v, "repair"),
                    s(&v, "lane"),
                    s(&v, "lane_state_label")
                );
                0
            }
            Err(e) => fail(err, &e),
        };
    }
    // The verbs whose bodies belong to other tracks (T5b's submit / auth-helper and T7b's gate / recheck
    // have theirs above).
    let _ = writeln!(
        err,
        "{}: cys repair {}",
        cys::repair::NOT_IMPLEMENTED_KO,
        verb_name(cmd)
    );
    cys::repair::types::EXIT_REPAIR_NOT_IMPLEMENTED
}


/// Track T5b — the hidden `cys repair auth-helper` (C5 5.9 / 5.15 · O-C5-4 · RULINGS B8): the
/// short-lived helper the DAEMON spawns on the owner's click. It runs the operating system's user
/// verification (macOS: LocalAuthentication, `deviceOwnerAuthentication` — Touch ID or the login
/// password) and reports ONLY through its exit status: 0 = verified; anything else = not approved.
/// LocalAuthentication is opened at run time in this short-lived process — no framework is linked
/// into `cys` or `cysd` (`scripts/check-no-ioreport-link.sh`; a missing framework can never stop a
/// start). It invalidates its own context at 110 s (the daemon kills + reaps it at 120 s) and opens
/// no file at all (never one under `~/Desktop`, `~/Documents`, `~/Downloads` — measured privacy-dialog
/// trap, `impl/measure/Ma-helper-auth-prompt.md`). ★NEVER run in a test (it would put a dialog on the
/// owner's screen with nobody to answer): in a test build the evaluation is refused before anything is
/// opened. Other systems answer the platform-pending code at once.
#[allow(dead_code)]
mod auth_helper {
    use std::io::Write;

    /// Exit codes (the measured helper's, `Ma-helper-auth-prompt.md` §4).
    pub(crate) const EXIT_USER_CANCEL: i32 = 10;
    pub(crate) const EXIT_APP_CANCEL: i32 = 11;
    pub(crate) const EXIT_SYSTEM_CANCEL: i32 = 12;
    pub(crate) const EXIT_NOT_INTERACTIVE: i32 = 13;
    pub(crate) const EXIT_AUTH_FAILED: i32 = 14;
    pub(crate) const EXIT_USER_FALLBACK: i32 = 15;
    pub(crate) const EXIT_INVALID_CONTEXT: i32 = 16;
    pub(crate) const EXIT_OTHER: i32 = 19;
    pub(crate) const EXIT_CANNOT_EVALUATE: i32 = 20;
    pub(crate) const EXIT_NO_COMPLETION: i32 = 30;
    /// The helper's own deadline (B8: 110 s; the daemon kills it at 120 s).
    pub(crate) const INVALIDATE_AFTER: std::time::Duration = std::time::Duration::from_secs(110);

    /// LAError code → exit status.
    pub(crate) fn exit_for_la_error(code: isize) -> i32 {
        match code {
            -2 => EXIT_USER_CANCEL,
            -9 => EXIT_APP_CANCEL,
            -4 => EXIT_SYSTEM_CANCEL,
            -1004 => EXIT_NOT_INTERACTIVE,
            -1 => EXIT_AUTH_FAILED,
            -3 => EXIT_USER_FALLBACK,
            -10 => EXIT_INVALID_CONTEXT,
            _ => EXIT_OTHER,
        }
    }

    /// A request id the daemon issues (`rq-` + hex) and a reason without control characters.
    pub(crate) fn valid_args(request: &str, reason: &str) -> bool {
        request.len() <= 64
            && request
                .strip_prefix("rq-")
                .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
            && !reason.trim().is_empty()
            && reason.chars().count() <= 400
            && !reason.chars().any(|c| c.is_control())
    }

    /// RULINGS JW1 — the environment variable in which the daemon hands this helper its request's
    /// one-time nonce (the daemon's `HELPER_NONCE_VAR`).
    pub(crate) const NONCE_VAR: &str = "CYS_REPAIR_AUTH_NONCE";

    /// A nonce the daemon issues (hex, 16–64 characters).
    pub(crate) fn valid_nonce(nonce: &str) -> bool {
        (16..=64).contains(&nonce.len()) && nonce.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// RULINGS JW1 (HL3 (ii)) — the helper's answer on its standard output: ONE success record bound
    /// to the request (its id and its one-time nonce), printed ONLY when the system's approval call
    /// returned success (`code` 0); nothing otherwise. A record that cannot be written is no
    /// success.
    pub(crate) fn answer(code: i32, request: &str, nonce: &str, out: &mut dyn Write) -> i32 {
        if code != 0 {
            return code;
        }
        if writeln!(out, "CYS-AUTH-OK v1 request={request} nonce={nonce}")
            .and_then(|()| out.flush())
            .is_err()
        {
            return EXIT_NO_COMPLETION;
        }
        0
    }

    pub(crate) fn run(
        request: &str,
        reason: &str,
        nonce: Option<&str>,
        out: &mut dyn Write,
        err: &mut dyn Write,
    ) -> i32 {
        let nonce = match nonce {
            Some(n) if valid_args(request, reason) && valid_nonce(n) => n,
            _ => {
                let _ = writeln!(err, "auth-helper: 잘못된 인자");
                return EXIT_CANNOT_EVALUATE;
            }
        };
        if !cfg!(target_os = "macos") {
            let _ = writeln!(err, "auth-helper: 이 운영체제에서는 1차 출시에서 최종 승인을 받지 않습니다(대기)");
            return cys::repair::types::EXIT_REPAIR_PLATFORM_PENDING;
        }
        answer(evaluate(reason), request, nonce, out)
    }

    #[cfg(test)]
    fn evaluate(_reason: &str) -> i32 {
        // a test build never opens LocalAuthentication
        EXIT_CANNOT_EVALUATE
    }

    #[cfg(all(target_os = "macos", not(test)))]
    fn evaluate(reason: &str) -> i32 {
        // SAFETY: plain Objective-C runtime calls through dlsym'd symbols with the exact prototypes
        // of the selectors used; every pointer is checked for null before use; the block is a leaked
        // heap literal flagged global (Block_copy returns it unchanged) invoked at most once.
        unsafe { la::evaluate(reason) }
    }

    #[cfg(all(not(target_os = "macos"), not(test)))]
    fn evaluate(_reason: &str) -> i32 {
        cys::repair::types::EXIT_REPAIR_PLATFORM_PENDING
    }

    #[cfg(all(target_os = "macos", not(test)))]
    mod la {
        use super::*;
        use std::ffi::{c_char, c_void, CString};
        use std::sync::mpsc;
        use std::sync::{Mutex, OnceLock};

        type Id = *mut c_void;
        type Sel = *mut c_void;
        const RTLD_NOW: i32 = 0x2;
        const LA_POLICY_DEVICE_OWNER_AUTHENTICATION: isize = 2;
        const BLOCK_IS_GLOBAL: i32 = 1 << 28;

        extern "C" {
            fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }

        #[repr(C)]
        struct BlockDescriptor {
            reserved: usize,
            size: usize,
        }

        #[repr(C)]
        struct BlockLiteral {
            isa: *const c_void,
            flags: i32,
            reserved: i32,
            invoke: unsafe extern "C" fn(*mut BlockLiteral, u8, Id),
            descriptor: *const BlockDescriptor,
        }

        static REPLY: OnceLock<Mutex<Option<mpsc::Sender<(bool, isize)>>>> = OnceLock::new();
        static mut CODE_SEL: Sel = std::ptr::null_mut();
        static mut MSG_SEND: *mut c_void = std::ptr::null_mut();

        unsafe extern "C" fn on_reply(_block: *mut BlockLiteral, success: u8, error: Id) {
            let mut code = 0isize;
            if success == 0 && !error.is_null() && !MSG_SEND.is_null() && !CODE_SEL.is_null() {
                let f: unsafe extern "C" fn(Id, Sel) -> isize = std::mem::transmute(MSG_SEND);
                code = f(error, CODE_SEL);
            }
            if let Some(m) = REPLY.get() {
                if let Ok(g) = m.lock() {
                    if let Some(tx) = g.as_ref() {
                        let _ = tx.send((success != 0, code));
                    }
                }
            }
        }

        unsafe fn sym(handle: *mut c_void, name: &str) -> *mut c_void {
            match CString::new(name) {
                Ok(c) => dlsym(handle, c.as_ptr()),
                Err(_) => std::ptr::null_mut(),
            }
        }

        pub(super) unsafe fn evaluate(reason: &str) -> i32 {
            let Ok(la_path) = CString::new("/System/Library/Frameworks/LocalAuthentication.framework/LocalAuthentication") else {
                return EXIT_CANNOT_EVALUATE;
            };
            let Ok(objc_path) = CString::new("/usr/lib/libobjc.A.dylib") else {
                return EXIT_CANNOT_EVALUATE;
            };
            let objc = dlopen(objc_path.as_ptr(), RTLD_NOW);
            let la = dlopen(la_path.as_ptr(), RTLD_NOW);
            if objc.is_null() || la.is_null() {
                return EXIT_CANNOT_EVALUATE;
            }
            let get_class = sym(objc, "objc_getClass");
            let reg_sel = sym(objc, "sel_registerName");
            let msg = sym(objc, "objc_msgSend");
            // _NSConcreteGlobalBlock lives in libSystem (libclosure)
            let global_block = sym(-2isize as *mut c_void, "_NSConcreteGlobalBlock");
            if get_class.is_null() || reg_sel.is_null() || msg.is_null() || global_block.is_null() {
                return EXIT_CANNOT_EVALUATE;
            }
            MSG_SEND = msg;
            let get_class: unsafe extern "C" fn(*const c_char) -> Id = std::mem::transmute(get_class);
            let reg_sel: unsafe extern "C" fn(*const c_char) -> Sel = std::mem::transmute(reg_sel);
            let sel = |n: &str| -> Sel {
                match CString::new(n) {
                    Ok(c) => reg_sel(c.as_ptr()),
                    Err(_) => std::ptr::null_mut(),
                }
            };
            let class = |n: &str| -> Id {
                match CString::new(n) {
                    Ok(c) => get_class(c.as_ptr()),
                    Err(_) => std::ptr::null_mut(),
                }
            };
            CODE_SEL = sel("code");
            let la_class = class("LAContext");
            let ns_string = class("NSString");
            if la_class.is_null() || ns_string.is_null() {
                return EXIT_CANNOT_EVALUATE;
            }
            let send0: unsafe extern "C" fn(Id, Sel) -> Id = std::mem::transmute(msg);
            let ctx = send0(send0(la_class, sel("alloc")), sel("init"));
            if ctx.is_null() {
                return EXIT_CANNOT_EVALUATE;
            }
            let can: unsafe extern "C" fn(Id, Sel, isize, *mut Id) -> u8 = std::mem::transmute(msg);
            let mut e: Id = std::ptr::null_mut();
            if can(ctx, sel("canEvaluatePolicy:error:"), LA_POLICY_DEVICE_OWNER_AUTHENTICATION, &mut e) == 0 {
                return EXIT_CANNOT_EVALUATE;
            }
            let Ok(c_reason) = CString::new(reason) else {
                return EXIT_CANNOT_EVALUATE;
            };
            let init_str: unsafe extern "C" fn(Id, Sel, *const c_char) -> Id = std::mem::transmute(msg);
            let ns_reason = init_str(send0(ns_string, sel("alloc")), sel("initWithUTF8String:"), c_reason.as_ptr());
            if ns_reason.is_null() {
                return EXIT_CANNOT_EVALUATE;
            }
            let (tx, rx) = mpsc::channel();
            let _ = REPLY.set(Mutex::new(Some(tx)));
            let descriptor: &'static BlockDescriptor = Box::leak(Box::new(BlockDescriptor {
                reserved: 0,
                size: std::mem::size_of::<BlockLiteral>(),
            }));
            let block: &'static mut BlockLiteral = Box::leak(Box::new(BlockLiteral {
                isa: global_block,
                flags: BLOCK_IS_GLOBAL,
                reserved: 0,
                invoke: on_reply,
                descriptor,
            }));
            let eval: unsafe extern "C" fn(Id, Sel, isize, Id, *mut BlockLiteral) = std::mem::transmute(msg);
            eval(ctx, sel("evaluatePolicy:localizedReason:reply:"), LA_POLICY_DEVICE_OWNER_AUTHENTICATION, ns_reason, block);
            let answer = match rx.recv_timeout(INVALIDATE_AFTER) {
                Ok(a) => Some(a),
                Err(_) => {
                    // B8: the helper removes its own dialog (measured: gone within 0.5 s)
                    let inv: unsafe extern "C" fn(Id, Sel) = std::mem::transmute(msg);
                    inv(ctx, sel("invalidate"));
                    rx.recv_timeout(std::time::Duration::from_secs(3)).ok()
                }
            };
            match answer {
                Some((true, _)) => 0,
                Some((false, code)) => exit_for_la_error(code),
                None => EXIT_NO_COMPLETION,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    fn parse(args: &[&str]) -> Result<RepairCmd, String> {
        let argv: Vec<&str> = ["cys", "repair"]
            .iter()
            .copied()
            .chain(args.iter().copied())
            .collect();
        match super::super::Cli::try_parse_from(argv) {
            Ok(cli) => match cli.command {
                super::super::Command::Repair { cmd } => Ok(cmd),
                _ => Err("parsed into another command".to_string()),
            },
            Err(e) => Err(e.to_string()),
        }
    }

    const CLI_NAMES: [&str; 23] = [
        "report",
        "begin",
        "record",
        "submit",
        "commit",
        "attest-verify",
        "attest-audit",
        "list",
        "status",
        "revert",
        "reapply",
        "cleanup",
        "grant",
        "owner-token",
        "rule-doc-allowed",
        "sweep",
        "recheck",
        "tick",
        "rules",
        "boot-critical",
        "tombstones",
        "gate",
        "auth-helper",
    ];
    const LOCAL: [&str; 5] = [
        "rules",
        "boot-critical",
        "tombstones",
        "gate",
        "auth-helper",
    ];

    /// `REPAIR_SUBVERBS` of the daemon (src/bin/cysd/repair.rs), read from its source — the CLI and
    /// the daemon are two binaries, so the constant cannot be imported here.
    fn daemon_subverbs() -> Vec<String> {
        let src = include_str!("../cysd/repair.rs");
        let head = "pub(crate) const REPAIR_SUBVERBS: &[&str] = &[";
        assert_eq!(
            src.matches(head).count(),
            1,
            "REPAIR_SUBVERBS declaration not found exactly once"
        );
        let start = src.find(head).unwrap() + head.len();
        let body = &src[start..start + src[start..].find("];").expect("end of REPAIR_SUBVERBS")];
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    /// The clap names are the 23 contract names; the 18 daemon-backed ones (`-`→`_`) plus the two
    /// `ask` modes and `grant status` (RULINGS CI6 (b)) are daemon sub-verbs and the remaining daemon
    /// sub-verbs are the app-only three; every name has a class in `VERB_CLASSES` (J1).
    #[test]
    fn verb_names_are_contract() {
        let root = <super::super::Cli as clap::CommandFactory>::command();
        let repair = root
            .find_subcommand("repair")
            .expect("`cys repair` sub-command");
        repair.clone().debug_assert(); // clap's own consistency check of every verb and flag
        let mut names: Vec<String> = repair
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .filter(|n| n != "help")
            .collect();
        names.sort();
        let mut want: Vec<String> = CLI_NAMES.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(
            names, want,
            "cys repair verbs differ from the contract (TRACKS §6.3)"
        );
        for hidden in ["tick", "auth-helper"] {
            assert!(
                repair.find_subcommand(hidden).expect(hidden).is_hide_set(),
                "{hidden} must be hidden"
            );
        }

        let subverbs = daemon_subverbs();
        assert_eq!(subverbs.len(), 24, "daemon REPAIR_SUBVERBS: {subverbs:?}");
        let mut backed: Vec<String> = CLI_NAMES
            .iter()
            .filter(|n| !LOCAL.contains(n))
            .map(|n| n.replace('-', "_"))
            .collect();
        assert_eq!(backed.len(), 18);
        backed.push("grant_ask".to_string());
        backed.push("owner_token_ask".to_string());
        backed.push("grant_status".to_string());
        for b in &backed {
            assert!(
                subverbs.contains(b),
                "CLI verb {b} is not a daemon sub-verb"
            );
        }
        let mut app_only: Vec<&str> = subverbs
            .iter()
            .map(String::as_str)
            .filter(|s| !backed.iter().any(|b| b == s))
            .collect();
        app_only.sort_unstable();
        assert_eq!(
            app_only,
            vec!["approval_list", "approval_reject", "approval_start"]
        );

        for n in CLI_NAMES {
            let key = if LOCAL.contains(&n) {
                n.to_string()
            } else {
                n.replace('-', "_")
            };
            assert!(
                cys::repair::roles::verb_class(&key).is_some(),
                "{n} ({key}) has no class in VERB_CLASSES"
            );
        }
        for b in &backed {
            assert!(
                cys::repair::roles::verb_class(b).is_some(),
                "{b} has no class"
            );
        }
    }

    /// Every NOT-yet-implemented verb parses with its contract flags, prints the Korean sentence on
    /// stderr, writes nothing on stdout and exits 99; every verb parses; malformed calls are
    /// refused by the parser.
    #[test]
    fn stub_exit_99() {
        let calls: &[&[&str]] = &[
            &["report", "--file", "bin/a.py", "--error", "boom"],
            &["report", "--file", "bin/a.py", "--error", "boom", "--json"],
            &["begin", "--files", "bin/a.py", "--trigger", "owner-order"],
            &[
                "begin",
                "--files",
                "bin/a.py",
                "hooks/b.sh",
                "--unregister-hook",
                "Stop:x.sh",
                "--unregister-hook",
                "PreToolUse:y.sh",
                "--trigger",
                "owner-order",
                "--owner-token",
                "ot",
                "--joins",
                "R-1",
                "--resolves",
                "R-2",
                "--json",
            ],
            &["begin", "--files", "a", "--trigger", "grant"],
            &["begin", "--files", "a", "--trigger", "update"],
            &["record", "R-1", "--note", "n"],
            &["record", "R-1", "--note", "n", "--json"],
            &["submit", "R-1"],
            &["submit", "R-1", "--json"],
            &["commit", "R-1", "--json"],
            &["attest-verify", "R-1", "--verdict", "pass", "--note", "n"],
            &["attest-verify", "R-1", "--verdict", "fail", "--note", "n"],
            &["attest-audit", "R-1", "--verdict", "ok", "--note", "n"],
            &["attest-audit", "R-1", "--verdict", "finding", "--note", "n"],
            &["list"],
            &["list", "--json"],
            &["status"],
            &["status", "R-1", "--json"],
            &["revert", "R-1"],
            &["reapply", "R-1"],
            &["cleanup", "R-1"],
            &["cleanup", "R-1", "--slot", "20261112T000000000000Z"],
            &["grant", "on", "--daily-cap", "3"],
            &["grant", "off"],
            &["grant", "ask"],
            &["grant", "ask", "--daily-cap", "7"],
            &["grant", "status"],
            &["grant", "status", "--json"],
            &["owner-token", "--path", "directives/MASTER_DIRECTIVE.md"],
            &[
                "owner-token",
                "ask",
                "--path",
                "directives/MASTER_DIRECTIVE.md",
                "--repair",
                "R-1",
            ],
            &[
                "rule-doc-allowed",
                "--path",
                "directives/MASTER_DIRECTIVE.md",
            ],
            &["sweep"],
            &["sweep", "--json"],
            &["recheck", "R-1"],
            &["tick"],
            &["rules", "--role", "worker"],
            &["boot-critical", "--path", "hooks/session-start.sh"],
            &["boot-critical", "--list", "--json"],
            &["tombstones"],
            &["tombstones", "--json"],
            &["gate", "--lane", "base"],
            &["gate", "--lane", "base", "--json"],
            &["auth-helper", "--request", "rq-1", "--reason", "r"],
        ];
        // T3 (rewritten from T0's "every verb exits 99"): the verbs whose bodies are other tracks'
        // still exit 99 with the sentence and nothing on stdout; T3's daemon verbs meeting an OLD
        // daemon exit 93 with the O-C3-7 sentence and nothing on stdout (no request reaches a ledger);
        // the local `tombstones` is T3's reader (tested in `tombstones_local_reader`).
        let old_daemon = |m: &str, _p: Value| -> Result<Value, String> {
            Err(format!("method_not_found: unknown method: {m}"))
        };
        let t3 = [
            "report",
            "begin",
            "record",
            "commit",
            "attest-verify",
            "attest-audit",
            "list",
            "status",
            "cleanup",
            // T4 (rewrite: its daemon verbs left the stub set — an old daemon → 93 like T3's)
            "grant",
            "owner-token",
            "rule-doc-allowed",
            // T6 (rewrite: its daemon verbs left the stub set — an old daemon → 93 like T3's / T4's;
            // the old expectation was exit 99 with the not-implemented sentence)
            "revert",
            "reapply",
            "sweep",
            "tick",
            // T5b (rewrite: `submit` left the stub set — was "exit 99"; an old daemon → 93 like T3's)
            "submit",
            // T7b (rewrite: `recheck` left the stub set — an old daemon → 93 like the others; the old
            // expectation was exit 99 with the not-implemented sentence)
            "recheck",
        ];
        let mut covered: Vec<&str> = Vec::new();
        for argv in calls {
            let cmd = parse(argv)
                .unwrap_or_else(|e| panic!("`cys repair {}` did not parse: {e}", argv.join(" ")));
            let name = verb_name(&cmd);
            assert_eq!(name, argv[0], "verb_name disagrees with the command line");
            if !covered.contains(&name) {
                covered.push(name);
            }
            if name == "tombstones" || name == "boot-critical" || name == "gate" {
                // local verbs: their own tests pin them (T5a: boot_critical_* below; T7b: gate_* —
                // rewrite: the old expectation for `gate` was exit 99 with the not-implemented sentence)
                continue;
            }
            if name == "rules" {
                // T1a: the local `rules` prints the block (byte-exact in
                // `rules_prints_the_block_byte_for_byte`), nothing on stderr, exit 0, no request.
                let (mut out, mut err) = (Vec::new(), Vec::new());
                assert_eq!(run_with(&cmd, &mut out, &mut err, &old_daemon), 0);
                assert!(err.is_empty() && !out.is_empty(), "`cys repair {}`", argv.join(" "));
                continue;
            }
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&cmd, &mut out, &mut err, &old_daemon);
            assert!(
                out.is_empty(),
                "`cys repair {}` wrote to stdout",
                argv.join(" ")
            );
            if name == "auth-helper" {
                // T5b (rewrite: was "exit 99"): the helper's body runs — LOCAL, no request; a test
                // build never opens the OS verification (exit 20 "cannot evaluate" on macOS, the
                // platform-pending 97 elsewhere), nothing on stdout or stderr
                let want = if cfg!(target_os = "macos") { 20 } else { 97 };
                assert_eq!(code, want, "{}", argv.join(" "));
                continue;
            }
            if t3.contains(&name) {
                assert_eq!(code, 93, "{}", argv.join(" "));
                assert_eq!(
                    String::from_utf8(err).unwrap(),
                    format!("{OLD_DAEMON_KO}\n")
                );
            } else {
                assert_eq!(code, 99, "{}", argv.join(" "));
                assert_eq!(
                    String::from_utf8(err).unwrap(),
                    format!("이 빌드에는 아직 구현되지 않았습니다: cys repair {name}\n")
                );
            }
        }
        covered.sort_unstable();
        let mut all = CLI_NAMES.to_vec();
        all.sort_unstable();
        assert_eq!(covered, all, "not every verb was exercised");

        // RULINGS Z2(e): `--files` may be left out only when `--unregister-hook` is given.
        assert!(parse(&[
            "begin",
            "--unregister-hook",
            "Stop:x.sh",
            "--trigger",
            "owner-order"
        ])
        .is_ok());
        // RULINGS AD10: `--trigger` is required (no default — fail closed).
        for bad in [
            &["submit", "R-1", "--staging", "/tmp/x"][..], // U3: no --staging
            &["reapply", "R-1", "--staging", "/tmp/x"],
            &["begin"],
            &["begin", "--json"],
            &["begin", "--trigger", "grant"],
            &["begin", "--files", "a", "--trigger", "owner_order"],
            &["begin", "--files", "a"],
            &["begin", "--unregister-hook", "Stop:x.sh"],
            &["begin", "--files", "a", "--joins", "R-1"],
            &["boot-critical"],
            &["boot-critical", "--json"],
            &["boot-critical", "--path", "a", "--list"],
            &["grant", "maybe"],
            &["grant"],
            &["owner-token", "ask"],
            &["attest-verify", "R-1", "--verdict", "ok", "--note", "n"],
            &["attest-audit", "R-1", "--verdict", "pass", "--note", "n"],
            &["record", "R-1"],
            &["gate"],
            &["rules"],
            &["no-such-verb"],
        ] {
            assert!(
                parse(bad).is_err(),
                "`cys repair {}` must be refused by the parser",
                bad.join(" ")
            );
        }

        // A closed / failing stderr is not a panic and still exits 99.
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // T5b (rewrite: was `submit` → 99, then `revert` → 99; both are daemon-backed now — on this
        // base `recheck` (T7b) is the last verb still answered by the stub, and when T7b merges this
        // check loses its last stub verb and is T7b's to retire)
        let cmd = parse(&["recheck", "R-1"]).unwrap();
        assert_eq!(
            run_with(&cmd, &mut Vec::new(), &mut Broken, &old_daemon),
            99
        );
        let cmd = parse(&["list"]).unwrap();
        assert_eq!(
            run_with(&cmd, &mut Vec::new(), &mut Broken, &old_daemon),
            93
        );

        // Structure: production code of this module never prints to stdout directly — only through
        // the `out` writer of `run_with` (the local `rules` verb, T1a).
        let src = include_str!("repair_cli.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("test module boundary")];
        for needle in ["println!", "print!(", "stdout().write", "io::stdout().lock"] {
            assert!(
                !prod.contains(needle),
                "repair_cli.rs production code prints to stdout ({needle})"
            );
        }
    }

    /// T4: the wire requests of `grant on|off|ask`, `owner-token [ask]` and `rule-doc-allowed`, their
    /// output, and `rule-doc-allowed`'s exit code (0 only when the daemon allows — the guard hook reads
    /// nothing else; 95 with the reason on stderr and nothing on stdout otherwise).
    #[test]
    fn t4_requests_and_outputs() {
        let seen = std::cell::RefCell::new(Vec::<(String, Value)>::new());
        let reply = std::cell::RefCell::new(json!({}));
        let rpc = |m: &str, p: Value| -> Result<Value, String> {
            seen.borrow_mut().push((m.to_string(), p));
            Ok(reply.borrow().clone())
        };
        let run = |argv: &[&str], r: Value| -> (i32, String, String) {
            *reply.borrow_mut() = r;
            let cmd = parse(argv).unwrap();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&cmd, &mut out, &mut err, &rpc);
            (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
        };
        let (c, o, _) = run(&["grant", "on"], json!({"grant": "on", "daily_cap": 3}));
        assert_eq!((c, o.contains("켬") && o.contains("3건")), (0, true));
        assert_eq!(seen.borrow().last().unwrap(), &("repair.grant".to_string(), json!({"mode": "on", "daily_cap": null})));
        // CI4: `--daily-cap` on `on` is forwarded as given (never dropped) — the daemon refuses it
        let _ = run(&["grant", "on", "--daily-cap", "3"], json!({"grant": "on", "daily_cap": 3}));
        assert_eq!(seen.borrow().last().unwrap(), &("repair.grant".to_string(), json!({"mode": "on", "daily_cap": 3})));
        let (c, o, _) = run(&["grant", "off"], json!({"grant": "off"}));
        assert_eq!((c, o.trim()), (0, "자동 수리 허락: 끔"));
        assert_eq!(seen.borrow().last().unwrap().1, json!({"mode": "off", "daily_cap": null}));
        let (c, o, _) = run(&["grant", "ask"], json!({"daily_cap": 5}));
        assert!(c == 0 && o.contains("스스로 고쳐도 될까요? (하루 5건까지)") && o.contains("하루 상한 5건"), "{o}");
        // CJ5: the daemon's sentence is printed as given
        let (_, o, _) = run(&["grant", "ask"], json!({"daily_cap": 9, "ask_owner": "자비스가 발견한 명백한 버그·오류를 스스로 고쳐도 될까요? (하루 9건까지)"}));
        assert!(o.contains("\"자비스가 발견한 명백한 버그·오류를 스스로 고쳐도 될까요? (하루 9건까지)\""), "{o}");
        assert_eq!(seen.borrow().last().unwrap(), &("repair.grant_ask".to_string(), json!({"daily_cap": null})));
        // CI4: the limit belongs to the question
        let _ = run(&["grant", "ask", "--daily-cap", "7"], json!({"daily_cap": 7}));
        assert_eq!(seen.borrow().last().unwrap(), &("repair.grant_ask".to_string(), json!({"daily_cap": 7})));
        // CI6 (b): `grant status [--json]`
        let st = json!({"state": "on", "paused": false, "paused_until": null, "daily_cap": 7});
        let (c, o, e) = run(&["grant", "status"], st.clone());
        assert_eq!((c, o.trim(), e.as_str()), (0, "자동 수리 허락: 켬 · 하루 상한 7건 · 일시정지 아님", ""));
        assert_eq!(seen.borrow().last().unwrap(), &("repair.grant_status".to_string(), json!({"daily_cap": null})));
        let (c, o, _) = run(&["grant", "status", "--json"], st.clone());
        assert_eq!(c, 0);
        assert_eq!(serde_json::from_str::<Value>(o.trim()).unwrap(), st);
        let (_, o, _) = run(&["grant", "status"], json!({"state": "on", "paused": true, "paused_until": 1_800_000_000.0, "daily_cap": 5}));
        assert!(o.contains("일시정지 중(") && o.contains("까지"), "{o}");
        let (_, o, _) = run(&["grant", "status"], json!({"state": "off", "paused": false, "paused_until": null, "daily_cap": 5}));
        assert_eq!(o.trim(), "자동 수리 허락: 끔 (하루 상한 5건)");
        let (c, _, _) = run(&["owner-token", "ask", "--path", "soul.md"], json!({"path": "soul.md"}));
        assert_eq!(c, 0);
        assert_eq!(seen.borrow().last().unwrap(), &("repair.owner_token_ask".to_string(), json!({"path": "soul.md"})));
        let line = "주인 지시 표: ot-aaaaaaaaaaaaaaaaaaaaaaaaaa (규칙 문서 soul.md)";
        let (c, o, _) = run(&["owner-token", "--path", "soul.md", "--repair", "R-2"], json!({"ticket_line": line}));
        assert!(c == 0 && o.starts_with(line), "{o}");
        assert_eq!(seen.borrow().last().unwrap(), &("repair.owner_token".to_string(), json!({"path": "soul.md", "repair": "R-2"})));
        let (c, o, e) = run(&["rule-doc-allowed", "--path", "soul.md"], json!({"allowed": true, "path": "soul.md", "repair": "R-1"}));
        assert_eq!((c, e.as_str()), (0, ""));
        assert!(o.contains("R-1"));
        assert_eq!(seen.borrow().last().unwrap(), &("repair.rule_doc_allowed".to_string(), json!({"path": "soul.md"})));
        let (c, o, e) = run(&["rule-doc-allowed", "--path", "soul.md"], json!({"allowed": false, "path": "soul.md", "reason": "no"}));
        assert_eq!((c, o.as_str()), (95, ""));
        assert!(e.contains("허용하지 않음"));
    }

    /// RULINGS GV3 (codex r6 M3): the ordinary output of `cys repair status` prints the notices the
    /// daemon closed "not confirmed" and an unreadable notice state — fed with the daemon's answers.
    /// Without GV3 only the lane, the counts and the reviewer slots were printed.
    #[test]
    fn status_prints_the_unconfirmed_notices_and_an_unreadable_notice_state() {
        let render = |answer: Value| {
            let rpc = move |_m: &str, _p: Value| -> Result<Value, String> { Ok(answer.clone()) };
            let cmd = parse(&["status"]).unwrap();
            let mut out = Vec::new();
            assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &rpc), 0);
            String::from_utf8(out).unwrap()
        };
        let base = json!({"lane": "base", "repairs": 0, "open_defects": 0, "reviewer_slots": {"waiting": false}});
        let mut exhausted = base.clone();
        exhausted["notices_not_confirmed"] = json!(["f:hooks/x.sh:ab:kept"]);
        let text = render(exhausted);
        assert!(text.contains("확인이 없는 알림 1건") && text.contains("f:hooks/x.sh:ab:kept"), "{text}");
        let mut unreadable = base.clone();
        unreadable["notices_not_confirmed"] = json!({"unreadable": "m10/base.notices-handed.json: 읽을 수 없음"});
        let text = render(unreadable);
        assert!(text.contains("알림 상태를 읽을 수 없습니다") && text.contains("notices-handed.json"), "{text}");
        let mut none = base;
        none["notices_not_confirmed"] = json!([]);
        assert!(!render(none).contains("알림"), "nothing printed when there is none");
    }

    /// RULINGS HS2: a repair that is not in force prints WHY its propagation is not complete (the
    /// daemon's `propagation_note`) — never a bare "전파 미완" that reads as pending; the state sentence
    /// is the daemon's `state_label` as given. Without HS2 the note was dropped.
    #[test]
    fn status_says_why_the_propagation_of_a_repair_not_in_force_is_not_complete() {
        let label = "대체됨 — 업데이트가 수리한 파일을 바꿨습니다(hooks/h.sh) — 아무것도 되돌리지 않습니다(새 버전 우선)";
        let answer = json!({"repair": "R-2", "state": "superseded", "state_label": label, "classification_label": "일반",
            "attempt": 1, "max_rounds": 3, "lanes": {"base": "검증됨"}, "propagation_complete": false,
            "propagation_note": "사용 중인 수리가 아님"});
        let rpc = move |_m: &str, _p: Value| -> Result<Value, String> { Ok(answer.clone()) };
        let cmd = parse(&["status", "R-2"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &rpc), 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(&format!("R-2: {label} · 일반")), "{text}");
        assert!(text.contains("· 전파 미완(사용 중인 수리가 아님)"), "{text}");
    }

    /// RULINGS DA6: `cys repair status <R>` names a corrupt record in its normal output (not only
    /// with `--json`).
    #[test]
    fn status_prints_the_corrupt_diagnosis() {
        let rpc = |_m: &str, _p: Value| -> Result<Value, String> {
            Ok(json!({"repair": "R-7", "corrupt": "repairs/R-7/record.json: not JSON (line 1)"}))
        };
        let cmd = parse(&["status", "R-7"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &rpc), 0);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("R-7")
                && text.contains("읽을 수 없습니다")
                && text.contains("not JSON (line 1)"),
            "{text}"
        );
        assert!(
            !text.contains("시도"),
            "the empty fields of a readable record were printed: {text}"
        );
    }

    /// O-C3-6 exit mapping (+ Z1), the old-daemon sentence, an unreachable daemon → 98, and a
    /// successful reply printed (JSON with `--json`). Nothing on stdout on any failure.
    #[test]
    fn exit_codes_and_sentences() {
        let cases: [(&str, i32); 13] = [
            ("acl_denied: x", 92),
            ("method_not_found: unknown method: repair.begin", 93),
            ("repair_locked: x", 94),
            ("target_changed: x", 94),
            ("repair_precondition: x", 95),
            ("repair_corrupt: x", 95),
            ("not_found: x", 95),
            ("repair_check_failed: x", 96),
            ("repair_pending_platform: x", 97),
            (
                "cannot connect to cysd at /x/cysd.sock: No such file or directory",
                98,
            ),
            ("rpc_timeout: …", 98),
            ("repair_not_implemented: x", 99),
            ("invalid_params: x", 2),
        ];
        let cmd = parse(&["status", "R-1"]).unwrap();
        for (e, want) in cases {
            let err_s = e.to_string();
            let rpc = move |_m: &str, _p: Value| -> Result<Value, String> { Err(err_s.clone()) };
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(run_with(&cmd, &mut out, &mut err, &rpc), want, "{e}");
            assert!(out.is_empty(), "{e}: wrote to stdout");
            let text = String::from_utf8(err).unwrap();
            if want == 93 {
                assert_eq!(text, format!("{OLD_DAEMON_KO}\n"));
            } else if want == 98 {
                assert!(text.contains("자동으로 띄우지 않습니다"), "{text}");
            } else {
                assert!(!text.is_empty());
            }
        }
        // the request carries the verb's wire name and params; a success prints the JSON reply
        let seen = std::cell::RefCell::new(Vec::new());
        let rpc = |m: &str, p: Value| -> Result<Value, String> {
            seen.borrow_mut().push((m.to_string(), p));
            Ok(
                json!({"repair": "R-3", "state_label": "진행 중", "attempt": 1, "max_rounds": 10, "classification": "general", "begin": "new", "staging": null}),
            )
        };
        let cmd = parse(&[
            "begin",
            "--files",
            "bin/a.py",
            "--trigger",
            "update",
            "--resolves",
            "R-2",
            "--json",
        ])
        .unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_with(&cmd, &mut out, &mut err, &rpc), 0);
        assert!(err.is_empty());
        let printed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(printed["repair"], "R-3");
        let (m, p) = seen.borrow()[0].clone();
        assert_eq!(m, "repair.begin");
        assert_eq!(p["files"], json!(["bin/a.py"]));
        assert_eq!(p["resolves"], "R-2");
        assert_eq!(p["trigger"], "update", "the trigger always travels (AD10)");
        assert!(
            p.get("seat").is_none() && p.get("role").is_none(),
            "the CLI never names a seat or role (M1)"
        );
        // human output names the working copy (U3: `begin` prints it)
        let rpc2 = |_m: &str, _p: Value| -> Result<Value, String> {
            Ok(
                json!({"repair": "R-4", "state_label": "진행 중", "attempt": 1, "max_rounds": 10, "classification": "boot_critical", "begin": "new", "staging": "/x/repairs/R-4/staging"}),
            )
        };
        let cmd = parse(&["begin", "--files", "hooks/a.sh", "--trigger", "owner-order"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &rpc2), 0);
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("작업 사본: /x/repairs/R-4/staging"));
        // commit's human output names the superseded repairs and the warnings (AK3 / AK4)
        let rpc3 = |_m: &str, _p: Value| -> Result<Value, String> {
            Ok(
                json!({"repair": "R-5", "state_label": "시험 기간", "lanes": {}, "superseded": ["R-2"], "warnings": ["bin/a.py: 삭제 표식 파일을 읽을 수 없어"]}),
            )
        };
        let cmd = parse(&["commit", "R-5"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &rpc3), 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("이전 수리 대체됨: R-2"), "{text}");
        assert!(text.contains("경고: bin/a.py"), "{text}");
    }

    /// `repair.*` is in neither the streaming nor the server-wait list: the CLI's 40-s cap applies,
    /// so a verb must answer at once (F2; `submit` returns its number only).
    #[test]
    fn repair_requests_keep_the_40s_cap() {
        for m in [
            "repair.report",
            "repair.begin",
            "repair.record",
            "repair.commit",
            "repair.submit",
            "repair.status",
        ] {
            assert_eq!(
                super::super::rpc_idle_timeout_with(
                    m,
                    &json!({"timeout_secs": 3600, "wait": true}),
                    None
                ),
                Some(std::time::Duration::from_secs(40)),
                "{m}"
            );
        }
        // and the repair CLI turns autostart off before any request (appendix A 1-17)
        let src = include_str!("repair_cli.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        assert!(prod.contains("super::AUTOSTART.store(false"));
    }

    /// T7b · RULINGS I1 (flagged interpretation 19): `cys repair gate --lane <key> [--json]` is LOCAL — it
    /// sends no request (`run_local`'s rpc fails every call) — and answers exit 0 OPEN, 95 HELD (here: a
    /// ledger member that cannot be read holds — never "open on error"), 2 for a lane key that is not
    /// one; the JSON carries the verdict, the cause and the summary line.
    #[test]
    fn gate_local_exit_codes_and_json() {
        let base = std::env::temp_dir().join(format!("cys-cli-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // the repairs root's PARENT exists (as `~/.cys/local` does): an absent root is "no repair" only
        // on that positive evidence (FG1) — with no readable parent the gate answers HELD
        std::fs::create_dir_all(&base).unwrap();
        let root = base.join("repairs");
        let registry = base.join("depts.json");
        let mut out = Vec::new();
        assert_eq!(gate_local_in("base", true, &mut out, &mut Vec::new(), &root, &registry), 0);
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["verdict"], "open");
        assert!(v["cause"].is_null());
        std::fs::create_dir_all(root.join("R-1")).unwrap();
        std::fs::write(root.join("R-1").join("record.json"), b"{ broken").unwrap();
        let mut out = Vec::new();
        assert_eq!(gate_local_in("base", true, &mut out, &mut Vec::new(), &root, &registry), 95);
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["verdict"], "held");
        assert_eq!(v["cause"]["kind"], "state_unreadable");
        let mut out = Vec::new();
        assert_eq!(gate_local_in("base", false, &mut out, &mut Vec::new(), &root, &registry), 95);
        assert!(String::from_utf8(out).unwrap().starts_with("작업 배정 관문: 보류 (base)"));
        for bad in ["", "..", "a/b", "a b"] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(gate_local_in(bad, true, &mut out, &mut err, &root, &registry), 2, "{bad:?}");
            assert!(out.is_empty());
        }
        // through the dispatcher with an rpc that refuses everything: no request is ever sent
        let cmd = parse(&["gate", "--lane", "base", "--json"]).unwrap();
        assert!(is_local(&cmd));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// O-C1-8: the local tombstone reader prints the struct's JSON (field order fixed) for the
    /// caller's lane; an unreadable file → exit 95, nothing on stdout.
    #[test]
    fn tombstones_local_reader() {
        use cys::repair::tombstones::{record_in, Tombstone, TombstoneKind};
        use cys::repair::types::{LaneKey, RepairId};
        let root = std::env::temp_dir().join(format!("cys-cli-tomb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lane = LaneKey("base".into());
        let mut out = Vec::new();
        assert_eq!(
            tombstones_local_in(&mut out, &mut Vec::new(), true, lane.clone(), &root),
            0
        );
        assert_eq!(
            String::from_utf8(out).unwrap().trim(),
            r#"{"schema":1,"lane":"base","tombstones":[]}"#
        );
        let t = Tombstone {
            rel: "bin/old.py".into(),
            kind: TombstoneKind::File,
            repair: RepairId::parse("R-2").unwrap(),
        };
        record_in(&root, &lane, &t).unwrap();
        let mut out = Vec::new();
        assert_eq!(
            tombstones_local_in(&mut out, &mut Vec::new(), true, lane.clone(), &root),
            0
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["tombstones"][0]["rel"], "bin/old.py");
        assert_eq!(v["tombstones"][0]["repair"], "R-2");
        std::fs::write(root.join("tombstones").join("base.json"), b"{bad").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            tombstones_local_in(&mut out, &mut err, true, lane, &root),
            95
        );
        assert!(out.is_empty());
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
    /// The role strings `rules --role` accepts in practice: the six of O-C6-1 and real seat names.
    const RULE_ROLES: [&str; 13] = [
        "master",
        "ceo",
        "worker",
        "worker-3",
        "cso",
        "cso-2",
        "reviewer1",
        "reviewer2",
        "reviewer-codex",
        "reviewer-claude-1",
        "planner",
        "scan-bot",
        "",
    ];

    /// A-3s-04 (b) / O-C6-1: `cys repair rules --role <role>` prints exactly the compiled block of that
    /// role (the same text `compose_directive` and `[RESUME]` carry), nothing on stderr, exit 0. (The 2-s
    /// bound is the hook's — `cys_timeout_run 2`, hook check 23d — no wall-clock premise here.)
    #[test]
    fn rules_prints_the_block_byte_for_byte() {
        // AF4: `rules` must not send a request — a counting rpc stays at zero.
        let requests = std::cell::Cell::new(0usize);
        let no_request = |_m: &str, _p: Value| -> Result<Value, String> {
            requests.set(requests.get() + 1);
            Err("no request expected".into())
        };
        for role in RULE_ROLES {
            let cmd = parse(&["rules", "--role", role]).unwrap();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(run_with(&cmd, &mut out, &mut err, &no_request), 0, "{role:?}");
            assert!(err.is_empty(), "{role:?}: {}", String::from_utf8_lossy(&err));
            let want = cys::repair::rules::rules_text(cys::repair::roles::role_block_for(role)).unwrap();
            assert_eq!(String::from_utf8(out).unwrap(), want, "{role:?}");
        }
        // The master text is the common part + the master/CEO constant, byte for byte.
        let cmd = parse(&["rules", "--role", "master"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &no_request), 0);
        let want = format!(
            "{}{}",
            cys::repair::rules::RULES_COMMON,
            cys::repair::rules::RULES_MASTER_CEO
        );
        assert_eq!(out, want.into_bytes());
        // A failing stdout is reported on stderr with a non-zero exit (the hook then prints nothing).
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut err = Vec::new();
        assert_ne!(run_with(&cmd, &mut Broken, &mut err, &no_request), 0);
        assert!(!err.is_empty());
        assert_eq!(requests.get(), 0, "`rules` sent a request");
    }

    /// A-3s-04 (b) / RULINGS B10: `rules` never contacts a daemon. The socket the CLI would use is a
    /// listener of this test; after every `rules` call nothing has connected to it.
    #[cfg(unix)]
    #[test]
    fn rules_never_contacts_a_daemon() {
        let _env = crate::tests::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("cys-t1a-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("d.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind test socket");
        listener.set_nonblocking(true).unwrap();
        let saved = std::env::var_os(cys::ENV_SOCKET);
        std::env::set_var(cys::ENV_SOCKET, &sock);
        let no_request = local_rpc;
        let mut outs = Vec::new();
        for role in RULE_ROLES {
            let cmd = parse(&["rules", "--role", role]).unwrap();
            let mut out = Vec::new();
            outs.push(run_with(&cmd, &mut out, &mut Vec::new(), &no_request));
        }
        let accepted = listener.accept();
        match saved {
            Some(v) => std::env::set_var(cys::ENV_SOCKET, v),
            None => std::env::remove_var(cys::ENV_SOCKET),
        }
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(outs.iter().all(|rc| *rc == 0), "{outs:?}");
        match accepted {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("`cys repair rules` connected to the daemon socket"),
            Err(e) => panic!("unexpected accept error: {e}"),
        }
    }

    /// RULINGS Z3: `is_local` is exactly the five LOCAL verbs of C3 — no daemon-backed verb ever takes
    /// the early return, and no local verb ever reaches lane discovery.
    #[test]
    fn local_verbs_are_exactly_the_contract_five() {
        let minimal: [&[&str]; 23] = [
            &["report", "--file", "a", "--error", "e"],
            &["begin", "--files", "a", "--trigger", "owner-order"],
            &["record", "R-1", "--note", "n"],
            &["submit", "R-1"],
            &["commit", "R-1"],
            &["attest-verify", "R-1", "--verdict", "pass", "--note", "n"],
            &["attest-audit", "R-1", "--verdict", "ok", "--note", "n"],
            &["list"],
            &["status"],
            &["revert", "R-1"],
            &["reapply", "R-1"],
            &["cleanup", "R-1"],
            &["grant", "ask"],
            &["owner-token", "--path", "p"],
            &["rule-doc-allowed", "--path", "p"],
            &["sweep"],
            &["recheck", "R-1"],
            &["tick"],
            &["rules", "--role", "worker"],
            &["boot-critical", "--list"],
            &["tombstones"],
            &["gate", "--lane", "base"],
            &["auth-helper", "--request", "r", "--reason", "x"],
        ];
        let mut seen = Vec::new();
        for argv in minimal {
            let cmd = parse(argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
            let name = verb_name(&cmd);
            assert_eq!(is_local(&cmd), LOCAL.contains(&name), "{name}");
            seen.push(name);
        }
        seen.sort_unstable();
        let mut all = CLI_NAMES.to_vec();
        all.sort_unstable();
        assert_eq!(seen, all, "not every verb was classified");
    }

    /// RULINGS Z3 (B10 · appendix A 1-17): in `main()` the local repair verbs leave through `run_local`
    /// BEFORE `initialize_lane_identity` (lane discovery sends `system.identify`). Fails when the early
    /// return is removed or moved below discovery. Comment lines are ignored (a comment is no code).
    #[test]
    fn main_returns_early_for_local_repair_verbs() {
        let src = include_str!("../cys.rs");
        let start = src.find("\nfn main() {\n").expect("fn main() of the cys CLI");
        let end = start + 1 + src[start + 1..].find("\n}\n").expect("end of fn main()");
        let code: String = src[start..end]
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let parse_at = code.find("let cli = Cli::parse();").expect("argument parsing in main()");
        let early = code
            .find("if let Command::Repair { cmd } = &cli.command {")
            .expect("RULINGS Z3 early return is missing from main()");
        let local = code.find("repair_cli::is_local(cmd)").expect("Z3: is_local test");
        let exit = code
            .find("std::process::exit(repair_cli::run_local(cmd));")
            .expect("Z3: exit through run_local");
        let discovery = code
            .find("initialize_lane_identity(&cli.command);")
            .expect("lane discovery call in main()");
        assert!(
            parse_at < early && early < local && local < exit && exit < discovery,
            "the local-verb early return must sit between argument parsing and lane discovery"
        );
        assert_eq!(code.matches("repair_cli::run_local(").count(), 1, "one insertion only");
    }

    /// C3: every `cys repair <verb>` quoted in a role block is a real `cys repair` sub-command, and the
    /// two quoted `ask` forms parse exactly as written in the master block.
    #[test]
    fn quoted_verbs_are_cli_verbs() {
        use cys::repair::roles::RoleBlock;
        let rx = regex::Regex::new(r"cys repair ([a-z][a-z-]*)").unwrap();
        for b in [
            RoleBlock::MasterCeo,
            RoleBlock::Worker,
            RoleBlock::Cso,
            RoleBlock::Reviewer,
            RoleBlock::CommonOnly,
        ] {
            let text = cys::repair::rules::rules_text(b).unwrap();
            for c in rx.captures_iter(&text) {
                assert!(CLI_NAMES.contains(&&c[1]), "{b:?} quotes `cys repair {}` — no such verb", &c[1]);
            }
        }
        let master = cys::repair::rules::RULES_MASTER_CEO;
        assert!(master.contains("`cys repair grant ask`"));
        assert!(master.contains("`cys repair owner-token ask --path <문서>`"));
        assert!(matches!(
            parse(&["grant", "ask"]).unwrap(),
            RepairCmd::Grant { mode: GrantMode::Ask, .. }
        ));
        assert!(matches!(
            parse(&["owner-token", "ask", "--path", "directives/WORKER_DIRECTIVE.md"]).unwrap(),
            RepairCmd::OwnerToken { mode: Some(OwnerTokenMode::Ask), .. }
        ));
    }

    /// RULINGS AF4: the rpc of `run_local` ALWAYS errors (for every method), and `run_local` uses it — a
    /// local verb run from `main()` can never send a request.
    #[test]
    fn run_local_uses_an_rpc_that_always_errors() {
        for m in ["repair.list", "repair.status", "repair.begin", "system.identify", ""] {
            assert!(local_rpc(m, json!({})).is_err(), "{m}");
        }
        let src = include_str!("repair_cli.rs");
        let start = src.find("\npub(crate) fn run_local(").expect("run_local");
        let body = &src[start..start + src[start..].find("\n}\n").expect("end of run_local")];
        assert!(body.contains("&local_rpc)"), "run_local must pass local_rpc");
        assert!(!body.contains("super::request"), "run_local must not reach the daemon");
    }

    /// T5a: `cys repair boot-critical --list --json` IS the compiled table (the pin of RULINGS
    /// B6 — the break-each-file tests take their fixed input from it), `--list` is one line per
    /// row, and the query runs entirely in-process (exit 0; nothing on stderr).
    #[test]
    fn boot_critical_list_json_pins_entries() {
        let no_daemon = |m: &str, _p: Value| -> Result<Value, String> {
            panic!("`cys repair boot-critical` asked the daemon ({m}) — it is a local query")
        };
        let cmd = parse(&["boot-critical", "--list", "--json"]).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_with(&cmd, &mut out, &mut err, &no_daemon), 0);
        assert!(err.is_empty());
        let v: serde_json::Value = serde_json::from_slice(&out).expect("--list --json is JSON");
        assert_eq!(v["schema"], 1);
        let rows = v["entries"].as_array().unwrap();
        let table = cys::repair::boot_critical::entries();
        assert_eq!(rows.len(), table.len());
        // RULINGS AI6: the protection rows ride in their own key; `entries` holds boot-critical files only
        let prot = v["protection"].as_array().expect("`protection` key");
        let ptable = cys::repair::boot_critical::protection_entries();
        assert_eq!(prot.len(), ptable.len());
        for (r, e) in prot.iter().zip(ptable) {
            assert_eq!(r, &cys::repair::boot_critical::entry_json(e));
            assert_eq!(r["category"], "protection");
        }
        assert!(rows.iter().all(|r| r["category"] != "protection"));
        for (r, e) in rows.iter().zip(table) {
            assert_eq!(r, &cys::repair::boot_critical::list_entry_json(e));
            assert_eq!(r["rel"], e.rel);
            assert_eq!(r["lifeline"], e.lifeline);
            assert!(r["check"]["kind"].is_string() && r["check"]["arg"].is_string());
        }
        let keys: Vec<&str> = rows[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut keys = keys;
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["category", "check", "lifeline", "proof", "rel", "timeout_s"],
            "O-C4-3 entry format + `proof` (RULINGS AL1)"
        );
        let pkeys: Vec<&str> = prot[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut pkeys = pkeys;
        pkeys.sort_unstable();
        assert_eq!(
            pkeys,
            ["category", "check", "lifeline", "rel", "timeout_s"],
            "O-C4-3 entry format"
        );
        let cmd = parse(&["boot-critical", "--list"]).unwrap();
        let mut out = Vec::new();
        assert_eq!(run_with(&cmd, &mut out, &mut Vec::new(), &no_daemon), 0);
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), table.len());
    }

    /// T5a: `--path` exit codes 0 (boot-critical) / 1 (not) / 2 (cannot decide) through the CLI arm.
    #[test]
    fn boot_critical_path_exit_codes() {
        let no_daemon = |m: &str, _p: Value| -> Result<Value, String> {
            panic!("`cys repair boot-critical` asked the daemon ({m}) — it is a local query")
        };
        let dir = std::env::temp_dir().join(format!("t5a-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("hooks")).unwrap();
        let _g = crate::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var(cys::pack::ENV_PACK_DIR).ok();
        std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
        let code = |p: &std::path::Path| {
            let cmd = RepairCmd::BootCritical {
                path: Some(p.to_path_buf()),
                list: false,
                json: true,
            };
            run_with(&cmd, &mut Vec::new(), &mut Vec::new(), &no_daemon)
        };
        assert_eq!(code(&dir.join("hooks/session-start.sh")), 0);
        assert_eq!(code(&dir.join("bin/javis_bootstrap.py")), 0);
        assert_eq!(code(&dir.join("hooks/README.md")), 1);
        assert_eq!(code(&std::env::temp_dir().join("t5a-elsewhere.sh")), 1);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                code(std::path::Path::new(std::ffi::OsStr::from_bytes(
                    b"/tmp/\xff"
                ))),
                2
            );
        }
        match saved {
            Some(v) => std::env::set_var(cys::pack::ENV_PACK_DIR, v),
            None => std::env::remove_var(cys::pack::ENV_PACK_DIR),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// T5b — the helper's arguments are validated before anything else (a daemon-issued `rq-` id, a
    /// reason without control characters); a test build NEVER opens the OS verification.
    #[test]
    fn auth_helper_validates_and_never_evaluates_in_tests() {
        assert!(auth_helper::valid_args("rq-0123abcd", "「스스로 해결」 수리 R-1 최종 승인"));
        for (r, why) in [("rq-", "x"), ("daemon-1", "x"), ("rq-zz", "x"), ("rq-12", ""), ("rq-12", "a\nb")] {
            assert!(!auth_helper::valid_args(r, why), "{r:?} {why:?}");
        }
        // RULINGS JW1 (rewritten — old calls `run("bad", "x", &mut err)` and `run("rq-01", "이유",
        // &mut err)`): the helper now also takes its request's nonce and the output it answers on
        let nonce = "00112233445566778899aabbccddeeff";
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(auth_helper::run("bad", "x", Some(nonce), &mut out, &mut err), auth_helper::EXIT_CANNOT_EVALUATE);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = auth_helper::run("rq-01", "이유", Some(nonce), &mut out, &mut err);
        assert_eq!(code, if cfg!(target_os = "macos") { auth_helper::EXIT_CANNOT_EVALUATE } else { cys::repair::types::EXIT_REPAIR_PLATFORM_PENDING });
        assert!(out.is_empty(), "no record without the approval call's success");
        // no nonce, or a malformed one: never evaluated, no record
        for n in [None, Some("zz"), Some("0011")] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(auth_helper::run("rq-01", "이유", n, &mut out, &mut err), auth_helper::EXIT_CANNOT_EVALUATE, "{n:?}");
            assert!(out.is_empty());
        }
        // RULINGS JW1: the ONE record, only on the approval call's success
        let mut out = Vec::new();
        assert_eq!(auth_helper::answer(0, "rq-01", nonce, &mut out), 0);
        assert_eq!(String::from_utf8(out).unwrap(), format!("CYS-AUTH-OK v1 request=rq-01 nonce={nonce}\n"));
        let mut out = Vec::new();
        assert_eq!(auth_helper::answer(auth_helper::EXIT_AUTH_FAILED, "rq-01", nonce, &mut out), auth_helper::EXIT_AUTH_FAILED);
        assert!(out.is_empty());
        // measured LAError codes → the helper's exit codes (Ma-helper-auth-prompt.md §4)
        assert_eq!(auth_helper::exit_for_la_error(-2), 10);
        assert_eq!(auth_helper::exit_for_la_error(-9), 11);
        assert_eq!(auth_helper::exit_for_la_error(-1004), 13);
        assert_eq!(auth_helper::exit_for_la_error(-1), 14);
        assert_eq!(auth_helper::exit_for_la_error(12345), 19);
        assert_eq!(auth_helper::INVALIDATE_AFTER, std::time::Duration::from_secs(110));
        // the helper opens no file: its source names no path under the user's folders
        let src = include_str!("repair_cli.rs");
        let body = &src[src.find("mod auth_helper {").unwrap()..];
        for p in ["Desktop", "Documents", "Downloads", "std::fs::", "File::open"] {
            assert!(!body[..body.find("\n#[cfg(test)]\nmod tests").unwrap_or(body.len())].contains(p), "the helper names {p}");
        }
    }

    /// T5b — `submit` sends `repair.submit {repair}` (no `--staging` — U3) and prints the number.
    #[test]
    fn submit_sends_the_request_and_prints_the_number() {
        let cmd = parse(&["submit", "R-3"]).unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let rpc = |m: &str, p: Value| -> Result<Value, String> {
            seen.borrow_mut().push((m.to_string(), p));
            Ok(serde_json::json!({"repair": "R-3", "request_id": "rq-00ff", "note": "받았습니다"}))
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run_with(&cmd, &mut out, &mut err, &rpc), 0);
        assert_eq!(seen.borrow().clone(), vec![("repair.submit".to_string(), serde_json::json!({"repair": "R-3"}))]);
        assert_eq!(String::from_utf8(out).unwrap(), "R-3: 제출 rq-00ff — 받았습니다\n");
    }

}
