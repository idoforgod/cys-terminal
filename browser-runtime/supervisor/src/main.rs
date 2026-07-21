use cys::browser_runtime::{BrokerHello, TargetTriple};
use cys_browserd::{
    BrokerChannel, CommandEngineLauncher, Supervisor, SupervisorConfig, SupervisorError,
};
use std::io::{Read, Write};
use std::path::PathBuf;

const EX_USAGE: i32 = 64;
const EX_SOFTWARE: i32 = 70;
const EX_NOPERM: i32 = 77;

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("cys-browserd: {error}");
            if error.code() == "AUTHORITY_REJECTED" {
                EX_NOPERM
            } else if error.code() == "INVALID_ARGUMENT" {
                EX_USAGE
            } else {
                EX_SOFTWARE
            }
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn run() -> Result<(), SupervisorError> {
    let config = parse_args(std::env::args_os().skip(1))?;
    let parent = verified_cysd_parent()?;
    let mut stdin = std::io::stdin();
    let mut session_key = [0_u8; 32];
    stdin.read_exact(&mut session_key).map_err(|error| {
        SupervisorError::authority(format!("inherited broker session key unavailable: {error}"))
    })?;
    let line = read_bounded_line(&mut stdin, 16 * 1024)?;
    let hello = BrokerHello::parse(&line)
        .and_then(|hello| hello.verify(parent, session_key))
        .map_err(|error| SupervisorError::authority(error.to_string()))?;
    let broker = BrokerChannel::from_verified(hello, session_key);
    let mut running = Supervisor::start(config, Some(broker), &CommandEngineLauncher::default())?;

    let mut output = serde_json::to_vec(&running.state)
        .map_err(|error| SupervisorError::new("RUNTIME_START_FAILED", error.to_string()))?;
    output.push(b'\n');
    std::io::stdout()
        .write_all(&output)
        .and_then(|_| std::io::stdout().flush())
        .map_err(|error| {
            SupervisorError::new(
                "RUNTIME_START_FAILED",
                format!("broker readiness pipe failed: {error}"),
            )
        })?;

    let (lost_tx, lost_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut byte = [0_u8; 1];
        let result = loop {
            match stdin.read(&mut byte) {
                Ok(0) => break "broker liveness pipe closed",
                Ok(_) => break "unexpected command on startup-only broker channel",
                Err(_) => break "broker liveness pipe failed",
            }
        };
        let _ = lost_tx.send(result);
    });

    loop {
        if let Ok(reason) = lost_rx.try_recv() {
            return Err(SupervisorError::authority(reason));
        }
        if let Some(status) = running.engine.wait().map_err(|error| {
            SupervisorError::new("ENGINE_EXITED", format!("engine wait: {error}"))
        })? {
            return Err(SupervisorError::new(
                "ENGINE_EXITED",
                format!("engine exited: {status}"),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn read_bounded_line(reader: &mut impl Read, limit: usize) -> Result<String, SupervisorError> {
    let mut bytes = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while bytes.len() <= limit {
        let count = reader.read(&mut byte).map_err(|error| {
            SupervisorError::authority(format!("broker hello read failed: {error}"))
        })?;
        if count == 0 {
            return Err(SupervisorError::authority(
                "broker hello ended before newline",
            ));
        }
        if byte[0] == b'\n' {
            return String::from_utf8(bytes)
                .map_err(|_| SupervisorError::authority("broker hello is not valid UTF-8"));
        }
        bytes.push(byte[0]);
    }
    Err(SupervisorError::authority("broker hello exceeds 16 KiB"))
}

fn verified_cysd_parent() -> Result<u32, SupervisorError> {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let current = system
        .process(Pid::from_u32(std::process::id()))
        .ok_or_else(|| SupervisorError::authority("current process identity unavailable"))?;
    let parent_pid = current
        .parent()
        .ok_or_else(|| SupervisorError::authority("broker parent identity unavailable"))?;
    let parent = system
        .process(parent_pid)
        .ok_or_else(|| SupervisorError::authority("broker parent process unavailable"))?;
    let filename = parent
        .exe()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let expected = if cfg!(windows) { "cysd.exe" } else { "cysd" };
    if filename != expected {
        return Err(SupervisorError::authority(format!(
            "direct invocation denied: parent executable is not {expected}"
        )));
    }
    Ok(parent_pid.as_u32())
}

fn parse_args(
    args: impl Iterator<Item = std::ffi::OsString>,
) -> Result<SupervisorConfig, SupervisorError> {
    let args: Vec<_> = args.collect();
    if args.first().and_then(|value| value.to_str()) != Some("supervise") {
        return Err(SupervisorError::new(
            "INVALID_ARGUMENT",
            "private entry requires `supervise --broker-handle inherited-stdin`",
        ));
    }
    let mut manifest_path = None;
    let mut runtime_root = None;
    let mut state_path = None;
    let mut lock_path = None;
    let mut target = None;
    let mut broker_handle = false;
    let mut index = 1;
    while index < args.len() {
        let key = args[index]
            .to_str()
            .ok_or_else(|| SupervisorError::new("INVALID_ARGUMENT", "argument is not UTF-8"))?;
        let value = args
            .get(index + 1)
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                SupervisorError::new("INVALID_ARGUMENT", format!("missing value for {key}"))
            })?;
        match key {
            "--broker-handle" if value == "inherited-stdin" => broker_handle = true,
            "--manifest" => manifest_path = Some(PathBuf::from(value)),
            "--runtime-root" => runtime_root = Some(PathBuf::from(value)),
            "--state" => state_path = Some(PathBuf::from(value)),
            "--lock" => lock_path = Some(PathBuf::from(value)),
            "--target" => {
                target = Some(match value {
                    "aarch64-apple-darwin" => TargetTriple::Aarch64AppleDarwin,
                    "x86_64-apple-darwin" => TargetTriple::X86_64AppleDarwin,
                    "x86_64-pc-windows-msvc" => TargetTriple::X86_64PcWindowsMsvc,
                    _ => {
                        return Err(SupervisorError::new(
                            "INVALID_ARGUMENT",
                            "unsupported Browser Runtime target",
                        ));
                    }
                })
            }
            _ => {
                return Err(SupervisorError::new(
                    "INVALID_ARGUMENT",
                    format!("unknown private supervisor argument {key}"),
                ));
            }
        }
        index += 2;
    }
    if !broker_handle {
        return Err(SupervisorError::authority(
            "inherited broker handle is required",
        ));
    }
    let config = SupervisorConfig {
        manifest_path: manifest_path.ok_or_else(missing_args)?,
        runtime_root: runtime_root.ok_or_else(missing_args)?,
        state_path: state_path.ok_or_else(missing_args)?,
        lock_path: lock_path.ok_or_else(missing_args)?,
        target: target.ok_or_else(missing_args)?,
    };
    if !config.manifest_path.is_absolute()
        || !config.runtime_root.is_absolute()
        || !config.state_path.is_absolute()
        || !config.lock_path.is_absolute()
    {
        return Err(SupervisorError::new(
            "INVALID_ARGUMENT",
            "all private runtime paths must be absolute",
        ));
    }
    Ok(config)
}

fn missing_args() -> SupervisorError {
    SupervisorError::new(
        "INVALID_ARGUMENT",
        "private supervisor arguments are incomplete",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_public_entry_is_not_a_supported_mode() {
        let error = parse_args(std::iter::empty()).expect_err("no public supervisor mode exists");
        assert_eq!(error.code(), "INVALID_ARGUMENT");
    }
}
