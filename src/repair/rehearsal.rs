//! The rehearsal runner (M8 · design §4-8 · RULINGS F4, F8, H2, K4/O3, AD9 (S3)). Track T5b.
//!
//! `run_check` checks ONE file of a pack layout: the file is `<scratch_root>/<entry_rel>`. Its kind
//! is the file's row on a compiled list (boot-critical first, then protection), else by file type
//! (S3: shell → syntax check, Python → compile, JSON → parse, otherwise no rehearsal possible).
//! Checks that only READ the file (syntax, compile, parse) run on it in place; checks that EXECUTE
//! it (`self-test`, `hook-replay`) first copy the layout into a fresh scratch root and obey the
//! replay rules of C4 (`replay.rs`): fixed clean environment, the stub `cys` on a scratch socket,
//! own process group killed and reaped at the deadline — never a daemon, never the real socket.
//! A hook replay is judged by its output and markers, not by its exit code (D:779): hooks end with
//! exit 0 on their own errors.

use super::boot_critical::{boot_critical_entry, protection_entry, CheckKind};
use super::replay::{self, ChildOutcome, ScratchRoot, ScratchSocket};
use super::types::RepairError;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// One check of one boot-critical entry against a scratch copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRequest {
    pub entry_rel: String,
    pub scratch_root: PathBuf,
    pub recorded_input: Option<PathBuf>,
    pub deadline: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Pass,
    Fail(String),
    Timeout,
}

/// RULINGS K4 / O3: a created file is blamed for a timeout only when its commit-time duration ×
/// (reference now ÷ reference at commit) is under the deadline ÷ 1.5, after a control pass and a
/// second timeout.
pub const REFERENCE_SLOWDOWN_PERMILLE: u32 = 1_500;

/// The sentence the record carries for a file no rehearsal can check (S3 / D:781 "사전 예행 불가").
pub const NOT_REHEARSABLE_KO: &str = "사전 예행 불가 — 이 종류의 파일은 미리 돌려 볼 수 없습니다";

/// The check kind and its argument for a pack-relative rel: its boot-critical row, else its
/// protection row, else by file type (S3).
pub fn check_for(rel: &str) -> (CheckKind, &'static str) {
    if let Some(e) = boot_critical_entry(rel) {
        return (e.check, e.check_arg);
    }
    if let Some(e) = protection_entry(rel) {
        return (e.check, e.check_arg);
    }
    (kind_by_type(rel), "")
}

/// S3: the per-file-type table the list derivation uses.
pub fn kind_by_type(rel: &str) -> CheckKind {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rsplit_once('.').map(|(_, ext)| ext) {
        Some("sh") => CheckKind::ShSyntax,
        Some("py") => CheckKind::PyCompile,
        Some("json") => CheckKind::JsonParse,
        _ => CheckKind::NonePrerehearsal,
    }
}

/// Whether a candidate of this rel can be rehearsed before approval (C4 4.12, D:781): the card's
/// "사전 예행 불가" marker is shown when this is false.
pub fn prerehearsal_possible(rel: &str) -> bool {
    check_for(rel).0 != CheckKind::NonePrerehearsal
}

fn rel_is_plain(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty() && !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Runs one check child (killed and reaped at the deadline).
pub fn run_check(req: &CheckRequest) -> Result<CheckResult, RepairError> {
    if !rel_is_plain(&req.entry_rel) {
        return Ok(CheckResult::Fail(format!(
            "{}: 팩 안의 상대 경로가 아닙니다",
            req.entry_rel
        )));
    }
    let file = req.scratch_root.join(&req.entry_rel);
    let (kind, arg) = check_for(&req.entry_rel);
    run_kind(kind, arg, &req.entry_rel, &req.scratch_root, &file, req)
}

fn run_kind(
    kind: CheckKind,
    arg: &str,
    rel: &str,
    root: &Path,
    file: &Path,
    req: &CheckRequest,
) -> Result<CheckResult, RepairError> {
    let meta = std::fs::symlink_metadata(file);
    let is_file = meta
        .as_ref()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false);
    match kind {
        CheckKind::NonePrerehearsal => {
            if arg == "json-parse" {
                if !is_file {
                    return Ok(CheckResult::Fail(format!("{rel}: 파일이 없습니다")));
                }
                return Ok(json_parse(file, rel));
            }
            // S3: nothing can be rehearsed for this kind — not a failure; the record names the kind
            Ok(CheckResult::Pass)
        }
        _ if !is_file => Ok(CheckResult::Fail(format!(
            "{rel}: 일반 파일이 아닙니다(없거나 링크·폴더)"
        ))),
        CheckKind::JsonParse => Ok(json_parse(file, rel)),
        CheckKind::ShSyntax => Ok(sh_syntax(file, rel, req.deadline)),
        CheckKind::PyCompile => py_compile(file, rel, req.deadline),
        CheckKind::SelfTest => executed(root, rel, req, Exec::SelfTest(arg)),
        CheckKind::HookReplay => executed(root, rel, req, Exec::Hook(arg)),
    }
}

fn json_parse(file: &Path, rel: &str) -> CheckResult {
    match std::fs::read(file) {
        Ok(b) => match serde_json::from_slice::<serde_json::Value>(&b) {
            Ok(_) => CheckResult::Pass,
            Err(e) => CheckResult::Fail(format!("{rel}: JSON 으로 읽히지 않습니다 — {e}")),
        },
        Err(e) => CheckResult::Fail(format!("{rel}: 읽을 수 없습니다 — {e}")),
    }
}

fn first_line(file: &Path) -> String {
    use std::io::{BufRead, BufReader};
    std::fs::File::open(file)
        .ok()
        .and_then(|f| BufReader::new(f).lines().next())
        .and_then(|l| l.ok())
        .unwrap_or_default()
}

fn tail(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    let s = s.trim();
    let start = s.char_indices().rev().nth(400).map(|(i, _)| i).unwrap_or(0);
    s[start..].to_string()
}

/// RULINGS CZ4 / DF5: a run that left a process or a held pipe behind did not end cleanly — the
/// check fails, whatever the run printed.
fn left_behind(o: &ChildOutcome, rel: &str, what: &str) -> Option<CheckResult> {
    o.left_something().map(|why| {
        CheckResult::Fail(format!(
            "{rel}: {what} 이(가) 끝난 뒤에도 남긴 것이 있습니다 — {why}"
        ))
    })
}

fn outcome_result(o: &ChildOutcome, rel: &str, what: &str) -> CheckResult {
    if o.timed_out {
        return CheckResult::Timeout;
    }
    if let Some(f) = left_behind(o, rel, what) {
        return f;
    }
    if let Some(e) = &o.spawn_error {
        return CheckResult::Fail(format!("{rel}: {what} 을(를) 시작하지 못했습니다 — {e}"));
    }
    if o.code == Some(0) {
        CheckResult::Pass
    } else {
        CheckResult::Fail(format!(
            "{rel}: {what} 실패(종료 {:?}) — {}",
            o.code.or(o.signal),
            tail(&o.stderr)
        ))
    }
}

fn plain_env() -> Vec<(String, String)> {
    vec![
        ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("LANG".into(), "en_US.UTF-8".into()),
        ("LC_ALL".into(), "en_US.UTF-8".into()),
    ]
}

fn sh_syntax(file: &Path, rel: &str, deadline: Duration) -> CheckResult {
    // a bash script (shebang names bash) is checked by bash; everything else by sh
    let shell = if first_line(file).contains("bash") {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    let argv = vec![
        shell.to_string(),
        "-n".to_string(),
        file.to_string_lossy().into_owned(),
    ];
    let cwd = file.parent().unwrap_or_else(|| Path::new("/"));
    let o = replay::run_child(&argv, &plain_env(), cwd, b"", deadline);
    outcome_result(&o, rel, "셸 문법 검사")
}

const PY_COMPILE: &str =
    "import sys\nsrc = open(sys.argv[1], 'rb').read()\ncompile(src, sys.argv[1], 'exec')\n";

fn py_compile(file: &Path, rel: &str, deadline: Duration) -> Result<CheckResult, RepairError> {
    let Some(py) = replay::bundled_interpreter() else {
        return Ok(CheckResult::Fail(format!(
            "{rel}: 파이썬 실행기를 찾지 못해 컴파일 검사를 할 수 없습니다"
        )));
    };
    // -I -S -B: no user site, no site, never a __pycache__ written next to the file
    let argv = vec![
        py.to_string_lossy().into_owned(),
        "-I".into(),
        "-S".into(),
        "-B".into(),
        "-c".into(),
        PY_COMPILE.into(),
        file.to_string_lossy().into_owned(),
    ];
    let mut env = plain_env();
    env.push(("PYTHONDONTWRITEBYTECODE".into(), "1".into()));
    let cwd = file.parent().unwrap_or_else(|| Path::new("/"));
    let o = replay::run_child(&argv, &env, cwd, b"", deadline);
    Ok(outcome_result(&o, rel, "파이썬 컴파일 검사"))
}

enum Exec<'a> {
    SelfTest(&'a str),
    Hook(&'a str),
}

/// Where the scratch roots of rehearsals and screens live: a cache folder of the user's account,
/// never under `~/.cys` (a device's path rules must not see a ledger or pack path in a probe's
/// scratch), never under a temporary folder (the registrar refuses a pack there — preflight
/// `_path_under_tempdir`), never under `~/Desktop`, `~/Documents`, `~/Downloads` (B8).
pub fn scratch_base() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(".cache")
        })
        .join("cys-repair-replay")
}

/// Folders of a layout never copied into a rehearsal (state, logs, work rounds).
const NOT_COPIED: &[&str] = &["state", "round", "_round", "logs", "log", ".git"];
/// Upper bounds of one copied layout.
const COPY_MAX_FILES: usize = 20_000;
const COPY_MAX_BYTES: u64 = 128 * 1024 * 1024;

/// Copies a pack layout into `dst` (bytes, modes, links as links), leaving out [`NOT_COPIED`].
pub fn copy_layout(src: &Path, dst: &Path) -> Result<(), RepairError> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_dir(src, dst, true, &mut files, &mut bytes)
}

fn copy_dir(
    src: &Path,
    dst: &Path,
    top: bool,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<(), RepairError> {
    std::fs::create_dir_all(dst).map_err(|e| RepairError::Io(format!("{}: {e}", dst.display())))?;
    let rd =
        std::fs::read_dir(src).map_err(|e| RepairError::Io(format!("{}: {e}", src.display())))?;
    for e in rd.flatten() {
        let name = e.file_name();
        if top && NOT_COPIED.iter().any(|n| name.to_string_lossy() == *n) {
            continue;
        }
        let from = e.path();
        let to = dst.join(&name);
        let Ok(m) = std::fs::symlink_metadata(&from) else {
            continue;
        };
        *files += 1;
        if *files > COPY_MAX_FILES || *bytes > COPY_MAX_BYTES {
            return Err(RepairError::Precondition(format!(
                "{}: 예행 사본이 너무 큽니다",
                src.display()
            )));
        }
        if m.file_type().is_dir() {
            copy_dir(&from, &to, false, files, bytes)?;
        } else {
            *bytes += m.len();
            let s = replay::LaidSource::read(&from)?;
            replay::lay_out(&to, &s)?;
        }
    }
    Ok(())
}

/// The built-in recorded input of a hook event (the rehearsal's "기록된 입력"), as JSON text.
pub fn recorded_input_for(event: &str, cwd: &Path) -> String {
    let cwd = cwd.to_string_lossy();
    let v = match event {
        "PreToolUse" | "PermissionRequest" => serde_json::json!({
            "session_id": "rehearsal", "hook_event_name": event, "cwd": cwd,
            "tool_name": "Bash", "tool_input": {"command": "ls"}}),
        "PostToolUse" => serde_json::json!({
            "session_id": "rehearsal", "hook_event_name": event, "cwd": cwd,
            "tool_name": "Bash", "tool_input": {"command": "ls"},
            "tool_response": {"stdout": "", "stderr": "", "interrupted": false}}),
        "UserPromptSubmit" => serde_json::json!({
            "session_id": "rehearsal", "hook_event_name": event, "cwd": cwd,
            "prompt": "안녕하세요"}),
        "SessionStart" => serde_json::json!({
            "session_id": "rehearsal", "hook_event_name": event, "cwd": cwd,
            "source": "startup"}),
        other => serde_json::json!({
            "session_id": "rehearsal", "hook_event_name": other, "cwd": cwd}),
    };
    v.to_string()
}

/// Lines that tell that a hook (or self-test) broke instead of deciding: shell errors and the
/// common frame's degrade marker.
fn broken_marker(stderr: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(stderr);
    for line in s.lines() {
        let l = line.trim();
        if l.contains("_lib.sh 소실")
            || l.contains(": syntax error")
            || l.contains("unexpected EOF")
            || l.contains("Traceback (most recent call last)")
            || l.contains("SyntaxError:")
        {
            return Some(l.chars().take(300).collect());
        }
    }
    None
}

fn hook_stdout_is_json(b: &[u8]) -> bool {
    String::from_utf8_lossy(b)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
}

/// One executed run of a hook / self-test inside a fresh copy of `root` (replay rules).
fn run_in_copy(
    root: &Path,
    rel: &str,
    content: Option<&replay::LaidSource>,
    stdin: Option<&[u8]>,
    exec: &Exec<'_>,
    event: &str,
    deadline: Duration,
) -> Result<ChildOutcome, RepairError> {
    let scratch = ScratchRoot::new(&scratch_base(), "rh-")?;
    let s = scratch.path();
    let pack = s.join("pack");
    copy_layout(root, &pack)?;
    if let Some(c) = content {
        let f = pack.join(rel);
        let _ = std::fs::remove_file(&f);
        replay::lay_out(&f, c)?;
    }
    let home = s.join("h");
    let tmp = s.join("t");
    let work = s.join("w");
    for d in [&home, &tmp, &work, &home.join(".claude")] {
        std::fs::create_dir_all(d).map_err(|e| RepairError::Io(format!("{}: {e}", d.display())))?;
    }
    std::fs::write(home.join(".claude").join("settings.json"), "{}\n")
        .map_err(|e| RepairError::Io(e.to_string()))?;
    let bin = s.join("bin");
    replay::write_stub(&bin, "worker", false)?;
    let sock = ScratchSocket::bind(s)?;
    let py = replay::bundled_interpreter();
    let mut path_dirs = vec![bin.clone()];
    if let Some(p) = &py {
        path_dirs.push(replay::interpreter_dir(s, p)?);
    }
    let mut env = replay::base_env(&home, &tmp, &path_dirs);
    env.push(("CYS_PACK_DIR".into(), pack.to_string_lossy().into_owned()));
    env.push((
        "CYS_SOCKET".into(),
        sock.path().to_string_lossy().into_owned(),
    ));
    env.push(("CYS_SURFACE_ID".into(), "7".into()));
    env.push(("CYS_ROLE".into(), "worker".into()));
    env.push(("CYS_NO_AUTOSTART".into(), "1".into()));
    if let Some(p) = &py {
        env.push(("CYS_PY".into(), p.to_string_lossy().into_owned()));
    }
    let script = pack.join(rel).to_string_lossy().into_owned();
    let mut argv = vec!["/bin/sh".to_string(), script];
    let input = match exec {
        Exec::SelfTest(a) => {
            if !a.is_empty() {
                argv.push(a.to_string());
            }
            Vec::new()
        }
        Exec::Hook(_) => match stdin {
            Some(b) => b.to_vec(),
            None => recorded_input_for(event, &work).into_bytes(),
        },
    };
    let o = replay::run_child(&argv, &env, &work, &input, deadline);
    drop(sock);
    // RULINGS CZ4: a scratch root that cannot be removed is returned (the check is not measured)
    scratch.close()?;
    Ok(o)
}

fn executed(
    root: &Path,
    rel: &str,
    req: &CheckRequest,
    exec: Exec<'_>,
) -> Result<CheckResult, RepairError> {
    let started = std::time::Instant::now();
    match &exec {
        Exec::SelfTest(_) => {
            let o = run_in_copy(root, rel, None, None, &exec, "", req.deadline)?;
            if let Some(m) = broken_marker(&o.stderr) {
                if !o.timed_out {
                    return Ok(CheckResult::Fail(format!(
                        "{rel}: 자체 시험이 깨졌습니다 — {m}"
                    )));
                }
            }
            Ok(outcome_result(&o, rel, "자체 시험"))
        }
        Exec::Hook(events) => {
            // the syntax check first (design §4-8 "문법 검사를 돌린다"): a hook that `exec`s into its
            // interpreter early never parses its own tail, so the replay alone cannot see an error there
            if rel.ends_with(".sh") {
                if let CheckResult::Fail(m) = sh_syntax(&root.join(rel), rel, req.deadline) {
                    return Ok(CheckResult::Fail(m));
                }
            }
            let recorded = match &req.recorded_input {
                Some(p) => Some(
                    std::fs::read(p)
                        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?,
                ),
                None => None,
            };
            let events: Vec<&str> = events
                .split(',')
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .collect();
            let events = if events.is_empty() {
                vec!["PreToolUse"]
            } else {
                events
            };
            // the released copy of this hook, for the expected-output comparison
            let released = released_copy(rel);
            for ev in events {
                let left = req.deadline.saturating_sub(started.elapsed());
                if left < replay::MIN_CHILD_SLICE {
                    return Ok(CheckResult::Timeout);
                }
                let o = run_in_copy(root, rel, None, recorded.as_deref(), &exec, ev, left)?;
                if o.timed_out {
                    return Ok(CheckResult::Timeout);
                }
                if let Some(f) = left_behind(&o, rel, "훅 재생") {
                    return Ok(f);
                }
                if let Some(e) = &o.spawn_error {
                    return Ok(CheckResult::Fail(format!(
                        "{rel}: 훅을 시작하지 못했습니다 — {e}"
                    )));
                }
                if let Some(m) = broken_marker(&o.stderr) {
                    return Ok(CheckResult::Fail(format!(
                        "{rel}({ev}): 훅이 깨졌습니다 — {m}"
                    )));
                }
                if !matches!(o.code, Some(0) | Some(2)) {
                    return Ok(CheckResult::Fail(format!(
                        "{rel}({ev}): 훅이 비정상으로 끝났습니다(종료 {:?}) — {}",
                        o.code.or(o.signal),
                        tail(&o.stderr)
                    )));
                }
                // expected output: what the released hook prints for the same input in the same
                // layout (a hook that prints for this input must still print, JSON stays JSON)
                if let Some(rc) = &released {
                    let left = req.deadline.saturating_sub(started.elapsed());
                    if left < replay::MIN_CHILD_SLICE {
                        return Ok(CheckResult::Timeout);
                    }
                    let r = run_in_copy(root, rel, Some(rc), recorded.as_deref(), &exec, ev, left)?;
                    if let Some(f) = left_behind(&r, rel, "출시본 훅 재생") {
                        return Ok(f);
                    }
                    if r.code == Some(0) || r.code == Some(2) {
                        let r_out = !String::from_utf8_lossy(&r.stdout).trim().is_empty();
                        let c_out = !String::from_utf8_lossy(&o.stdout).trim().is_empty();
                        if r_out && !c_out {
                            return Ok(CheckResult::Fail(format!(
                                "{rel}({ev}): 기대한 출력이 없습니다(출시본은 같은 입력에 출력을 냅니다)"
                            )));
                        }
                        if r_out
                            && hook_stdout_is_json(&r.stdout)
                            && !hook_stdout_is_json(&o.stdout)
                        {
                            return Ok(CheckResult::Fail(format!(
                                "{rel}({ev}): 출력이 JSON 이 아닙니다(출시본은 JSON 을 냅니다)"
                            )));
                        }
                    }
                }
            }
            Ok(CheckResult::Pass)
        }
    }
}

/// The copy of `rel` embedded in this binary (the released copy), if any.
pub fn released_copy(rel: &str) -> Option<replay::LaidSource> {
    replay::released_text(rel).map(|c| replay::LaidSource::Bytes {
        bytes: c.as_bytes().to_vec(),
        mode: released_mode(rel),
    })
}

/// The mode a released file is installed with (scripts executable).
pub fn released_mode(rel: &str) -> u32 {
    if rel.ends_with(".sh") || rel.ends_with(".py") || rel.starts_with("bin/") {
        0o755
    } else {
        0o644
    }
}

/// The built-in known-good fixture of a check kind (RULINGS H2).
pub fn reference_fixture(kind: CheckKind) -> Result<&'static [u8], RepairError> {
    Ok(match kind {
        CheckKind::ShSyntax => b"#!/bin/sh\nset -u\nx=1\nif [ \"$x\" = 1 ]; then\n  : ok\nfi\n",
        CheckKind::PyCompile => b"import sys\n\n\ndef main():\n    return 0\n\n\nif __name__ == '__main__':\n    sys.exit(main())\n",
        CheckKind::JsonParse => b"{\"schema\": 1, \"ok\": true, \"items\": [1, 2, 3]}\n",
        CheckKind::SelfTest => b"#!/bin/sh\n[ \"${1:-}\" = \"--self-test\" ] && exit 0\nexit 0\n",
        CheckKind::HookReplay => b"#!/bin/sh\ncat >/dev/null\nexit 0\n",
        CheckKind::NonePrerehearsal => b"",
    })
}

/// Runs the reference check of a kind on its fixture (RULINGS H2), with the same deadline.
pub fn run_reference_check(
    kind: CheckKind,
    deadline: std::time::Duration,
) -> Result<CheckResult, RepairError> {
    let fixture = reference_fixture(kind)?;
    if kind == CheckKind::NonePrerehearsal {
        return Ok(CheckResult::Pass);
    }
    let scratch = ScratchRoot::new(&scratch_base(), "rf-")?;
    let name = match kind {
        CheckKind::ShSyntax | CheckKind::SelfTest | CheckKind::HookReplay => "reference.sh",
        CheckKind::PyCompile => "reference.py",
        _ => "reference.json",
    };
    let file = scratch.path().join(name);
    std::fs::write(&file, fixture)
        .map_err(|e| RepairError::Io(format!("{}: {e}", file.display())))?;
    let req = CheckRequest {
        entry_rel: name.to_string(),
        scratch_root: scratch.path().to_path_buf(),
        recorded_input: None,
        deadline,
    };
    let arg = if kind == CheckKind::SelfTest {
        "--self-test"
    } else {
        ""
    };
    let arg = if kind == CheckKind::HookReplay {
        "PreToolUse"
    } else {
        arg
    };
    run_kind(kind, arg, name, scratch.path(), &file, &req)
}

#[cfg(test)]
#[path = "rehearsal_tests.rs"]
mod tests;
