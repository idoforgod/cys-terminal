//! `cys repair …` — the CLI of 「스스로 해결」 (0.15.0 · C3 verbs, N-3 flag shapes · T0 skeleton).
//!
//! Every verb except the LOCAL ones (`rules`, `boot-critical`, `tombstones`, `gate`, hidden
//! `auth-helper`) is a daemon request `repair.<verb>` (C1 1.15: the CLI never writes the ledger).
//! T0: every verb prints `"이 빌드에는 아직 구현되지 않았습니다: cys repair <verb>"` on stderr,
//! writes NOTHING on stdout (so a hook that prints stdout prints nothing — RULINGS B10) and exits 99
//! (N-2). The tracks fill `run` in place; the verb names and flags below are contract (TRACKS §6.3).

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
        /// 대상 파일들(팩 기준 상대 경로)
        #[arg(long, num_args = 1.., required = true)]
        files: Vec<String>,
        /// 생명줄·보호장치가 아닌 훅의 등록 해제(일반 수리에서만 — RULINGS V1)
        #[arg(long = "unregister-hook", value_name = "EVENT:SCRIPT")]
        unregister_hook: Vec<String>,
        #[arg(long, value_enum)]
        trigger: Option<TriggerArg>,
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

/// Runs a `cys repair` verb and returns the process exit code.
pub(crate) fn run(cmd: RepairCmd) -> i32 {
    run_with(&cmd, &mut std::io::stdout(), &mut std::io::stderr())
}

/// T0 stub: the sentence on `err`, nothing on `out`, exit 99. A failing stderr write is ignored
/// (a closed stderr must not turn into a panic).
fn run_with(cmd: &RepairCmd, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let _ = out;
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
            &["begin", "--files", "bin/a.py"],
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
        let mut covered: Vec<&str> = Vec::new();
        for argv in calls {
            let cmd = parse(argv)
                .unwrap_or_else(|e| panic!("`cys repair {}` did not parse: {e}", argv.join(" ")));
            let name = verb_name(&cmd);
            assert_eq!(name, argv[0], "verb_name disagrees with the command line");
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(run_with(&cmd, &mut out, &mut err), 99, "{}", argv.join(" "));
            assert!(
                out.is_empty(),
                "`cys repair {}` wrote to stdout",
                argv.join(" ")
            );
            assert_eq!(
                String::from_utf8(err).unwrap(),
                format!("이 빌드에는 아직 구현되지 않았습니다: cys repair {name}\n")
            );
            if !covered.contains(&name) {
                covered.push(name);
            }
        }
        covered.sort_unstable();
        let mut all = CLI_NAMES.to_vec();
        all.sort_unstable();
        assert_eq!(covered, all, "not every verb was exercised");

        for bad in [
            &["submit", "R-1", "--staging", "/tmp/x"][..], // U3: no --staging
            &["reapply", "R-1", "--staging", "/tmp/x"],
            &["begin"],
            &["begin", "--trigger", "grant"],
            &["begin", "--files", "a", "--trigger", "owner_order"],
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
        let cmd = parse(&["list"]).unwrap();
        assert_eq!(run_with(&cmd, &mut Vec::new(), &mut Broken), 99);

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
}
