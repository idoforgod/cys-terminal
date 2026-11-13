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

/// `grant on|off|ask` (`ask` → `repair.grant_ask`, RULINGS Q1).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrantMode {
    On,
    Off,
    Ask,
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
    /// 자동 수리 허락 켜기·끄기·질문 등록(마스터 · 주인 입력 확인 뒤에만 효력)
    Grant {
        #[arg(value_enum)]
        mode: GrantMode,
        #[arg(long)]
        daily_cap: Option<u32>,
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
                Some(Value::Bool(false)) => " · 전파 미완".to_string(),
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
        ),
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

/// O-C1-8: `cys repair tombstones [--json]` — a LOCAL read of this lane's tombstone file (no daemon);
/// the lane is the caller's socket (`CYS_SOCKET`, as `lane_key` does). An unreadable file → the
/// sentence on stderr and exit 95, nothing on stdout (a reader then behaves as today: no tombstones).
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
    if let RepairCmd::Tombstones { json } = cmd {
        return tombstones_local(out, err, *json);
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
    // The verbs whose bodies belong to other tracks (T1a rules · T4 grant/owner-token/rule-doc-allowed ·
    // T5a boot-critical · T5b submit/auth-helper · T6 revert/sweep/tick/reapply · T7b gate/recheck).
    let _ = writeln!(
        err,
        "{}: cys repair {}",
        cys::repair::NOT_IMPLEMENTED_KO,
        verb_name(cmd)
    );
    cys::repair::types::EXIT_REPAIR_NOT_IMPLEMENTED
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
    /// `ask` modes are daemon sub-verbs and the remaining daemon sub-verbs are the app-only three;
    /// every name has a class in `VERB_CLASSES` (J1).
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
        assert_eq!(subverbs.len(), 23, "daemon REPAIR_SUBVERBS: {subverbs:?}");
        let mut backed: Vec<String> = CLI_NAMES
            .iter()
            .filter(|n| !LOCAL.contains(n))
            .map(|n| n.replace('-', "_"))
            .collect();
        assert_eq!(backed.len(), 18);
        backed.push("grant_ask".to_string());
        backed.push("owner_token_ask".to_string());
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

    /// Every verb parses with its contract flags, prints the Korean sentence on stderr, writes
    /// nothing on stdout and exits 99; malformed calls are refused by the parser.
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
            if name == "tombstones" {
                continue;
            }
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&cmd, &mut out, &mut err, &old_daemon);
            assert!(
                out.is_empty(),
                "`cys repair {}` wrote to stdout",
                argv.join(" ")
            );
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
        let cmd = parse(&["submit", "R-1"]).unwrap();
        assert_eq!(
            run_with(&cmd, &mut Vec::new(), &mut Broken, &old_daemon),
            99
        );
        let cmd = parse(&["list"]).unwrap();
        assert_eq!(
            run_with(&cmd, &mut Vec::new(), &mut Broken, &old_daemon),
            93
        );

        // Structure: production code of this module never prints to stdout.
        let src = include_str!("repair_cli.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("test module boundary")];
        for needle in ["println!", "print!(", "stdout().write", "io::stdout().lock"] {
            assert!(
                !prod.contains(needle),
                "repair_cli.rs production code prints to stdout ({needle})"
            );
        }
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
}
