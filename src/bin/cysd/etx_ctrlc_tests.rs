//! ★(0.14.55 · P4 C-13) `C-c` / ETX on a PowerShell seat — the POSIX half of the measurement.
//!
//! The record said "the ETX byte is already emitted — handlers.rs:7543". At the release base that line
//! is a **comment**: it records the key→byte mapping, it is not the doing site. The corrected chain,
//! every line verified at the pin:
//!
//! | step | site | what it does |
//! |---|---|---|
//! | 1 | `src/lib.rs:2805` | `key_to_bytes` |
//! | 2 | `src/lib.rs:2814` | `(lower as u8) & 0x1f` ⇒ `'c' & 0x1f` = 0x03. There is **no literal `"C-c"` arm**; the byte is derived |
//! | 3 | `src/bin/cysd/handlers.rs:7270` | the `surface.send_key` RPC arm |
//! | 4 | `src/bin/cysd/handlers.rs:7277` | `cys::key_to_bytes(&key)` |
//! | 5 | `src/bin/cysd/handlers.rs:2412` | `submit_gap_for_key` is `Return`/`Enter` only ⇒ `C-c` is a plain `WriteReq::Data` |
//! | 6 | `src/bin/cysd/handlers.rs:7551` | `C-c` (0x03) classifies as `DirectSendKind::CancelKey` |
//! | 7 | `src/bin/cysd/state.rs:8071` | `writer.write_all(&bytes)` — the byte leaves the daemon into the PTY master |
//!
//! So "the ETX is already sent" is TRUE, and step 7 is that sentence's evidence — which is what these
//! cells measure. They also pin the exception the record's framing did not cover (§5-2): because `C-c`
//! is a `CancelKey` it passes through `governance::draft_gate`, so there are **two** outcomes that look
//! identical from a seat — *refused* (0 bytes written, a typed error reply) and *sent and ineffective*
//! (the byte went out and the shell stayed in its continuation). A measurement that does not separate
//! these attributes a daemon-side refusal to ConPTY.
//!
//! ⛔**The Windows half stays UNMEASURED and no cell here stands in for it.** A recorder seat built on
//! `stty raw -echo; exec cat` is a POSIX instrument: that shell idiom does not exist on a native Windows
//! ConPTY seat, and changing console modes in order to record would change the conditions being
//! measured. What only a native observation can establish — that the byte reached the ConPTY client,
//! the input/output handle modes (`ENABLE_PROCESSED_INPUT`, `ENABLE_VIRTUAL_TERMINAL_INPUT`,
//! `ENABLE_LINE_INPUT`), whether a `CTRL_C_EVENT` was delivered to the process group, and a human
//! `Ctrl+C` control in the same `>>` state — needs a real Windows seat. These cells prove the claim
//! about **our own code**, on macOS, and nothing more. C-13 ships no product change in 0.14.55.
#![cfg(test)]

use crate::governance::{draft_gate_verdict, DirectSendKind};
use crate::handlers::dispatch;
use crate::state::{Daemon, Surface};
use cys::Request;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

struct Fx {
    daemon: Arc<Daemon>,
    dir: std::path::PathBuf,
    _g: MutexGuard<'static, ()>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        crate::state::test_stop_daemon(&self.daemon);
        std::env::remove_var(cys::pack::ENV_PACK_DIR);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fx(tag: &str) -> Fx {
    let g = crate::governance::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::delivery::tests::isolate_state_dir_for_thread(tag);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cys-etx-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("acl.json"), r#"{"default":"allow","rules":[]}"#).unwrap();
    std::env::set_var(cys::pack::ENV_PACK_DIR, &dir);
    let daemon = Daemon::new(dir.join("cysd.sock"));
    crate::agent_write::set_test_sync_settle(true);
    Fx { daemon, dir, _g: g }
}

/// A seat whose receiver records every byte it is given, raw and unechoed — the bytes the request put
/// on the wire. POSIX only, by construction (see the module note).
fn recorder_seat(fx: &Fx, tag: &str) -> (Arc<Surface>, std::path::PathBuf) {
    let out = fx.dir.join(format!("rx-{tag}.bin"));
    let cmd = format!("stty raw -echo; exec cat > '{}'", out.display());
    let s = fx.daemon.create_surface(None, Some(cmd), None, None, 24, 200).expect("recorder seat");
    fx.daemon.surfaces.lock().unwrap().insert(s.id, s.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !out.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(out.exists(), "recorder seat never opened its sink");
    std::thread::sleep(Duration::from_millis(300)); // stty applied before the first byte
    (s, out)
}

fn rpc(fx: &Fx, method: &str, params: Value) -> Value {
    let req = Request { id: json!(1), method: method.into(), params };
    let crate::handlers::Reply::Single(resp) = dispatch(&fx.daemon, req, Some(1)) else {
        panic!("single reply")
    };
    resp
}

fn send_key(fx: &Fx, s: &Arc<Surface>, key: &str) -> Value {
    rpc(fx, "surface.send_key", json!({"surface_id": s.id, "key": key, "from": "surface:900"}))
}

/// Wait until the recorder file holds `want`, or the deadline passes. Returns the bytes seen.
fn wire_bytes(out: &std::path::Path, want: u8, ms: u64) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        let b = std::fs::read(out).unwrap_or_default();
        if b.contains(&want) || Instant::now() >= deadline {
            return b;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Put a human draft on the seat's input line: the gate reads `human.min(pending)`, so both must be set.
fn human_draft(s: &Arc<Surface>, bytes: u64) {
    s.set_pending_input(bytes);
    s.pending_input.lock().unwrap_or_else(|e| e.into_inner()).human = bytes;
}

/// ★C-13 step 6, as a pure function: the classification axis is the BYTE, so `C-c` (0x03) is a
/// `CancelKey`, and that arm denies on a human draft and only on a human draft.
#[test]
fn ctrl_c_classifies_as_cancel_key_and_denies_only_on_a_human_draft() {
    assert_eq!(cys::key_to_bytes("C-c"), Some(vec![0x03]), "premise: the byte is derived, not literal");
    // a human draft on the line -> denied
    assert!(
        draft_gate_verdict(DirectSendKind::CancelKey, 3, 3, None, false, false).is_some(),
        "a cancel key must not delete a human's unsubmitted draft"
    );
    // no human draft -> allowed, even with screen residue (fail-open is deliberate: a render remnant
    // must not block cycle-agent's `send_key C-u`, which is the no-clear direction)
    assert!(
        draft_gate_verdict(DirectSendKind::CancelKey, 3, 0, None, false, false).is_none(),
        "counted-but-not-human input must not block a cancel key"
    );
    assert!(draft_gate_verdict(DirectSendKind::CancelKey, 0, 0, None, false, false).is_none());
}

/// ★C-13 step 7 — the cell R2 asks for: the daemon's writer thread puts 0x03 on the wire. This is a
/// statement about OUR code, valid on macOS. It does not say the byte reached a ConPTY client.
#[test]
#[cfg_attr(not(unix), ignore = "recorder seat uses POSIX shell syntax (stty/exec cat)")]
fn ctrl_c_reaches_the_wire_as_a_single_etx_byte() {
    let fx = fx("etx-wire");
    let (s, out) = recorder_seat(&fx, "wire");
    let reply = send_key(&fx, &s, "C-c");
    assert!(reply.get("error").is_none(), "premise: the RPC was accepted: {reply}");
    let seen = wire_bytes(&out, 0x03, 3000);
    let _ = s.child.lock().unwrap().kill();
    assert!(seen.contains(&0x03), "ETX never reached the wire; bytes seen: {seen:?}");
    assert_eq!(seen.iter().filter(|b| **b == 0x03).count(), 1, "exactly one ETX, not a retype: {seen:?}");
}

/// ★C-13 §5-2 — the outcome the record's framing did not cover: with a human draft on the line the
/// cancel key is REFUSED and **no byte is written at all**. Without this cell a measurement would
/// attribute a daemon-side refusal to ConPTY.
#[test]
#[cfg_attr(not(unix), ignore = "recorder seat uses POSIX shell syntax (stty/exec cat)")]
fn ctrl_c_on_a_human_draft_is_refused_and_writes_nothing() {
    let fx = fx("etx-refuse");
    let (s, out) = recorder_seat(&fx, "refuse");
    human_draft(&s, 7);
    let reply = send_key(&fx, &s, "C-c");
    // give a would-be write the same window the positive cell gets, then read
    let seen = wire_bytes(&out, 0x03, 1500);
    let _ = s.child.lock().unwrap().kill();
    assert!(reply.get("error").is_some(), "a refused cancel key must answer with a typed error: {reply}");
    assert!(
        !seen.contains(&0x03),
        "refused, yet an ETX reached the wire — the two 'nothing happened' outcomes are not separated: {seen:?}"
    );
}

/// Control for the cell above: the same rig DOES carry a key to the wire when the gate allows it. Without
/// this, a zero in the refusal cell could be a broken rig rather than a refusal.
#[test]
#[cfg_attr(not(unix), ignore = "recorder seat uses POSIX shell syntax (stty/exec cat)")]
fn control_the_rig_does_carry_a_key_to_the_wire() {
    let fx = fx("etx-control");
    let (s, out) = recorder_seat(&fx, "control");
    let reply = send_key(&fx, &s, "C-c");
    assert!(reply.get("error").is_none(), "premise: accepted: {reply}");
    let seen = wire_bytes(&out, 0x03, 3000);
    let _ = s.child.lock().unwrap().kill();
    assert!(seen.contains(&0x03), "the rig cannot carry a byte, so no refusal cell is attributable: {seen:?}");
}
