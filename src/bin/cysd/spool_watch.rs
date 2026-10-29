//! X-8 CARRIER (0.14.56) - the per-daemon spool watch (DESIGN-r3 e04f3f11, 4-2 .. 4-9).
//!
//! EVERY daemon (base and each dept - no dept skip, unlike the bridge spawner) reads the HUD bridge's tokenless `/health`
//! once a minute, keeps the entries that are ITS OWN (dept: `socket` equals this daemon's socket path; base: `socket: null`
//! lanes plus the global `_bridge` / `_fleet`), adds checks that do not trust the bridge's verdict (W1-W5) and publishes ONE
//! observation named [`OFFICE_SPOOL_EVENT`] on its OWN bus. The same daemon's alert router delivers it to its own master
//! (and, on the base, a bounded digest to the Secretariat). This module only READS: one local HTTP GET a minute and one
//! `stat` of its own pack's spool - `spool_watch_never_writes` pins that no write, append, rename or remove call is here.

use crate::alert_route::OFFICE_SPOOL_EVENT;
use crate::state::Daemon;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// One probe a minute (the bridge supervisor's own interval - one source).
pub const WATCH_INTERVAL_SECS: u64 = crate::office_bridge::PROBE_INTERVAL_SECS;
/// The only `/health` spool schema this watch understands; any other value is treated like no answer (W1).
pub const SPOOL_SCHEMA: u64 = 1;
/// W1: no answer / non-200 / malformed / no `spool` / unknown schema on this many consecutive probes.
pub const UNREACHABLE_PROBES: u32 = 3;
/// W2: `spool_checked` still false on this many consecutive answered probes (about 5 minutes).
pub const NOT_READY_PROBES: u32 = 5;
/// W3: the bridge has checked its lanes but lists no entry for this daemon on this many consecutive probes.
pub const UNOBSERVED_PROBES: u32 = 3;
/// W5: own spool larger than the reader offset with no offset progress on this many consecutive probes.
pub const STALL_PROBES: u32 = 3;
/// The spool path formula, `<pack_dir>/<SPOOL_STATE_DIRNAME>/<SPOOL_BASENAME>` - pinned to the Python leaf
/// `cysjavis-pack/bin/javis_spool_path.py` by `spool_formula_matches_the_python_leaf` (one formula, two languages).
pub const SPOOL_STATE_DIRNAME: &str = "state";
pub const SPOOL_BASENAME: &str = "evt_spool.jsonl";

/// Who this daemon is, for the watch.
#[derive(Clone, Debug)]
pub struct Own {
    /// This daemon's socket path, as the fleet registry spells it (compared verbatim with the `/health` entry's `socket`).
    pub socket: String,
    /// The base daemon (not a `cys-dept-*` socket).
    pub is_base: bool,
    /// Lane id for this daemon's own W notices: `main`, or the dept from the socket path (`lane::socket_dept`).
    pub lane: String,
    /// This daemon's own pack spool - W4 compares the bridge's path with it, W5 stats it.
    pub spool_path: PathBuf,
}

impl Own {
    pub fn of(daemon: &Arc<Daemon>) -> Own {
        let socket = daemon.socket_path.to_string_lossy().to_string();
        let dept = cys::lane::socket_dept(&socket);
        Own {
            is_base: dept.is_none(),
            lane: dept.unwrap_or_else(|| "main".to_string()),
            socket,
            spool_path: spool_path_for_pack(&cys::pack::pack_dir()),
        }
    }
}

pub fn spool_path_for_pack(pack_dir: &Path) -> PathBuf {
    pack_dir.join(SPOOL_STATE_DIRNAME).join(SPOOL_BASENAME)
}

/// Counters across probes (in memory; a restart starts them again - the episode record lives in the router).
#[derive(Debug, Default)]
pub struct WatchState {
    fails: u32,
    unready: u32,
    absent: u32,
    stall_offset: Option<u64>,
    stall_count: u32,
}

/// One `/health` probe: `Ok(body)` only for a 200, complete, JSON object with a `spool` array and `spool_schema == 1`.
pub fn probe_once(port: u16) -> Result<Value, ()> {
    let reply = cys::bridge_probe_get(port, "/health").map_err(|_| ())?;
    if reply.status != 200 || !reply.complete {
        return Err(());
    }
    parse_health(&reply.body)
}

pub fn parse_health(body: &[u8]) -> Result<Value, ()> {
    let v: Value = serde_json::from_slice(body).map_err(|_| ())?;
    if v.get("spool_schema").and_then(Value::as_u64) != Some(SPOOL_SCHEMA) || !v.get("spool").is_some_and(Value::is_array) {
        return Err(());
    }
    Ok(v)
}

/// Path equality for W4: separators and a trailing separator do not count; Windows compares without case.
fn same_path(reported: &str, own: &Path) -> bool {
    fn norm(s: &str) -> String {
        let t = s.replace('\\', "/");
        let t = t.trim_end_matches('/').to_string();
        if cfg!(windows) { t.to_lowercase() } else { t }
    }
    norm(reported) == norm(&own.to_string_lossy())
}

fn lane_entry(lane: &str, state: &str, fault: bool, hint: Option<String>) -> Value {
    json!({"lane": lane, "state": state, "fault": fault, "hint": hint})
}

/// One observation from one probe result. Returns the `office.spool` payload `{lanes, digest}`; `digest` is set on the
/// base daemon only. Reads nothing but `own.spool_path`'s size (W5).
pub fn observe(own: &Own, probe: &Result<Value, ()>, st: &mut WatchState) -> Value {
    let mut lanes: Vec<Value> = Vec::new();
    let mut digest = Value::Null;
    match probe {
        Err(()) => {
            st.fails = st.fails.saturating_add(1);
            if st.fails >= UNREACHABLE_PROBES {
                let hint = if own.is_base {
                    "the HUD bridge gave no usable /health spool report (no answer, non-200, or an older bridge without spool_schema 1) on 3 \
                     probes in a row - this daemon runs it"
                        .to_string()
                } else {
                    "the HUD bridge gave no usable /health spool report (no answer, non-200, or an older bridge without spool_schema 1) on 3 \
                     probes in a row - owned by the base lane, the CEO is told; nothing to fix in this lane"
                        .to_string()
                };
                lanes.push(lane_entry(&own.lane, "bridge_unreachable", true, Some(hint)));
                if own.is_base {
                    digest = json!({"faults": [], "unreachable": true});
                }
            }
        }
        Ok(h) => {
            st.fails = 0;
            let checked = h.get("spool_checked").and_then(Value::as_bool).unwrap_or(false);
            let entries = h.get("spool").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut mine = 0usize;
            let mut faults: Vec<String> = Vec::new();
            for e in &entries {
                let lane = e.get("lane").and_then(Value::as_str).unwrap_or("");
                let state = e.get("state").and_then(Value::as_str).unwrap_or("");
                let fault = e.get("fault").and_then(Value::as_bool).unwrap_or(false);
                if lane.is_empty() || state.is_empty() {
                    continue;
                }
                if fault {
                    faults.push(format!("{lane} {state}"));
                }
                let socket = e.get("socket").and_then(Value::as_str);
                let global = lane.starts_with('_');
                let is_mine = if own.is_base { socket.is_none() && !global } else { socket == Some(own.socket.as_str()) };
                if is_mine {
                    mine += 1;
                    // W4: the reader's path for this lane is not this daemon's own pack spool.
                    if let Some(p) = e.get("path").and_then(Value::as_str) {
                        if !same_path(p, &own.spool_path) {
                            lanes.push(lane_entry(
                                lane,
                                "path_disagrees",
                                true,
                                Some(format!("the bridge reads {p}, this daemon's pack spool is {}", own.spool_path.display())),
                            ));
                        }
                    }
                    // W5: the lane is read (no fault) but its offset does not follow a growing spool.
                    let offset = e.get("offset").and_then(Value::as_u64);
                    let size = std::fs::metadata(&own.spool_path).ok().map(|m| m.len());
                    match (fault, offset, size) {
                        (false, Some(off), Some(sz)) if sz > off => {
                            if st.stall_offset == Some(off) {
                                st.stall_count = st.stall_count.saturating_add(1);
                            } else {
                                st.stall_offset = Some(off);
                                st.stall_count = 1;
                            }
                            if st.stall_count >= STALL_PROBES {
                                lanes.push(lane_entry(lane, "reader_stalled", true, None));
                            }
                        }
                        _ => {
                            st.stall_offset = None;
                            st.stall_count = 0;
                        }
                    }
                }
                if is_mine || (own.is_base && global) {
                    lanes.push(lane_entry(lane, state, fault, None));
                }
            }
            // W2: the bridge never finished its self-check.
            if checked {
                st.unready = 0;
            } else {
                st.unready = st.unready.saturating_add(1);
                if st.unready >= NOT_READY_PROBES {
                    lanes.push(lane_entry(&own.lane, "not_ready", true, None));
                }
            }
            // W3: checked, yet no entry for this daemon - one transient absence is not a notice.
            if mine > 0 {
                st.absent = 0;
            } else if checked {
                st.absent = st.absent.saturating_add(1);
                if st.absent >= UNOBSERVED_PROBES {
                    lanes.push(lane_entry(&own.lane, "lane_unobserved", true, None));
                }
            }
            if own.is_base {
                faults.sort();
                digest = json!({"faults": faults, "unreachable": false});
            }
        }
    }
    json!({"lanes": lanes, "digest": digest})
}

/// Start the watch beside the alert router (every daemon). The first probe waits one interval (the bridge starts after).
pub fn spawn(daemon: Arc<Daemon>) {
    let own = Own::of(&daemon);
    let port = crate::office_bridge::bridge_port();
    tokio::spawn(async move {
        let mut st = WatchState::default();
        loop {
            tokio::time::sleep(Duration::from_secs(WATCH_INTERVAL_SECS)).await;
            let probe = tokio::task::spawn_blocking(move || probe_once(port)).await.unwrap_or(Err(()));
            let payload = observe(&own, &probe, &mut st);
            // Quiet unless the router has something to do (a steady all-ok daemon publishes nothing - R1-1).
            if crate::alert_route::office_spool_wants(&daemon, &payload) {
                daemon.bus.publish(OFFICE_SPOOL_EVENT, "office_spool", None, payload);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own_dept(spool: &Path) -> Own {
        Own { socket: "/x/cys-dept-dept-5/cys.sock".into(), is_base: false, lane: "dept-5".into(), spool_path: spool.to_path_buf() }
    }
    fn own_base(spool: &Path) -> Own {
        Own { socket: "/x/cys/cys.sock".into(), is_base: true, lane: "main".into(), spool_path: spool.to_path_buf() }
    }
    fn health(entries: Vec<Value>, checked: bool) -> Result<Value, ()> {
        Ok(json!({"spool_schema": 1, "spool_checked": checked, "spool": entries}))
    }
    fn entry(lane: &str, socket: Option<&str>, state: &str, fault: bool, path: Option<&str>, offset: Option<u64>) -> Value {
        json!({"lane": lane, "socket": socket, "state": state, "fault": fault, "path": path, "bytes": null,
               "archives": 0, "archive_bytes": 0, "offset": offset, "foreign_key_dropped": 0})
    }
    fn lanes(p: &Value) -> Vec<(String, String, bool)> {
        p["lanes"].as_array().unwrap().iter()
            .map(|l| (l["lane"].as_str().unwrap().to_string(), l["state"].as_str().unwrap().to_string(), l["fault"].as_bool().unwrap()))
            .collect()
    }
    fn tmp(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("cys-spw-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(d.join("state"));
        d
    }
    fn fleet(own_spool: &Path) -> Vec<Value> {
        let p = own_spool.to_string_lossy().to_string();
        vec![
            entry("main", None, "ok", false, Some("/x/pack/state/evt_spool.jsonl"), Some(0)),
            entry("dept-3", Some("/x/cys-dept-dept-3/cys.sock"), "pack_missing", true, None, None),
            entry("dept-5", Some("/x/cys-dept-dept-5/cys.sock"), "ok", false, Some(&p), Some(0)),
            entry("_bridge", None, "ok", false, None, None),
            entry("_fleet", None, "ok", false, None, None),
        ]
    }

    /// W1 (Q4): three failed probes in a row, never fewer; a dept notice names the base lane as owner; the base digest says so.
    #[test]
    fn spw_w1_unreachable_after_exactly_three_failed_probes() {
        let d = tmp("w1");
        let sp = spool_path_for_pack(&d);
        let (mut s5, mut sb) = (WatchState::default(), WatchState::default());
        let mut seen = Vec::new();
        for _ in 0..3 {
            let p5 = observe(&own_dept(&sp), &Err(()), &mut s5);
            let pb = observe(&own_base(&sp), &Err(()), &mut sb);
            seen.push((lanes(&p5).len(), lanes(&pb).len(), pb["digest"]["unreachable"].as_bool()));
        }
        println!("SPW-W1 per probe (dept lanes, base lanes, base digest unreachable) = {seen:?}");
        assert_eq!(seen, vec![(0, 0, None), (0, 0, None), (1, 1, Some(true))]);
        let p5 = observe(&own_dept(&sp), &Err(()), &mut s5);
        assert_eq!(lanes(&p5), vec![("dept-5".to_string(), "bridge_unreachable".to_string(), true)]);
        assert!(p5["lanes"][0]["hint"].as_str().unwrap().contains("owned by the base lane"));
        // one answer resets the count
        observe(&own_dept(&sp), &health(fleet(&sp), true), &mut s5);
        assert!(lanes(&observe(&own_dept(&sp), &Err(()), &mut s5)).is_empty());
    }

    /// W1 input: a malformed body, another schema or no `spool` array is the same as no answer.
    #[test]
    fn spw_w1_parse_health_rejects_what_it_does_not_understand() {
        assert!(parse_health(b"{\"spool_schema\":1,\"spool_checked\":true,\"spool\":[]}").is_ok());
        assert!(parse_health(b"{\"spool_schema\":2,\"spool\":[]}").is_err());
        assert!(parse_health(b"{\"spool_schema\":1}").is_err());
        assert!(parse_health(b"{\"ok\":true}").is_err(), "a pre-0.14.56 bridge body has no spool field");
        assert!(parse_health(b"not json").is_err());
    }

    /// Q2 / Q8 (single-HQ negative): a dept daemon keeps ONLY its own socket's entry - never another dept's fault.
    #[test]
    fn spw_q8_dept_daemon_keeps_only_its_own_entry() {
        let d = tmp("q8");
        let sp = spool_path_for_pack(&d);
        let p = observe(&own_dept(&sp), &health(fleet(&sp), true), &mut WatchState::default());
        println!("SPW-Q8 dept-5 lanes={:?}", lanes(&p));
        assert_eq!(lanes(&p), vec![("dept-5".to_string(), "ok".to_string(), false)]);
        assert!(p["digest"].is_null(), "only the base daemon builds the digest");
    }

    /// Q1 / Q3: the base daemon keeps main + the globals for its master (the CEO) and NEVER a dept lane;
    /// every lane's fault goes into the Secretariat digest instead.
    #[test]
    fn spw_q1_base_daemon_never_takes_dept_lanes_for_its_master_but_digests_them() {
        let d = tmp("q1");
        let sp = spool_path_for_pack(&d);
        let mut e = fleet(&sp);
        e[0] = entry("main", None, "ok", false, Some(&sp.to_string_lossy()), Some(0));
        let p = observe(&own_base(&sp), &health(e, true), &mut WatchState::default());
        let l = lanes(&p);
        println!("SPW-Q1 base lanes={l:?} digest={}", p["digest"]);
        assert_eq!(l.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec!["main", "_bridge", "_fleet"]);
        assert_eq!(p["digest"]["faults"], json!(["dept-3 pack_missing"]));
    }

    /// W2: `spool_checked` false on 5 answered probes in a row -> not_ready; a checked body resets the count.
    #[test]
    fn spw_w2_not_ready_after_five_unchecked_probes() {
        let d = tmp("w2");
        let sp = spool_path_for_pack(&d);
        let mut st = WatchState::default();
        let mut n = Vec::new();
        for _ in 0..5 {
            let p = observe(&own_dept(&sp), &health(vec![], false), &mut st);
            n.push(lanes(&p).iter().filter(|x| x.1 == "not_ready").count());
        }
        assert_eq!(n, vec![0, 0, 0, 0, 1]);
        observe(&own_dept(&sp), &health(fleet(&sp), true), &mut st);
        assert_eq!(lanes(&observe(&own_dept(&sp), &health(vec![], false), &mut st)).len(), 0);
    }

    /// W3 (Q5): checked but no entry for this daemon - a notice only on the THIRD absence in a row.
    #[test]
    fn spw_w3_lane_unobserved_needs_three_absences() {
        let d = tmp("w3");
        let sp = spool_path_for_pack(&d);
        let mut st = WatchState::default();
        let other = vec![entry("main", None, "ok", false, None, Some(0))];
        let a = lanes(&observe(&own_dept(&sp), &health(other.clone(), true), &mut st)).len();
        let b = lanes(&observe(&own_dept(&sp), &health(other.clone(), true), &mut st)).len();
        let c = lanes(&observe(&own_dept(&sp), &health(other.clone(), true), &mut st));
        println!("SPW-W3 after 1/2 absences={a}/{b}, after 3={c:?}");
        assert_eq!((a, b), (0, 0));
        assert_eq!(c, vec![("dept-5".to_string(), "lane_unobserved".to_string(), true)]);
        // unchecked absences do not count
        let mut st2 = WatchState::default();
        for _ in 0..3 {
            assert!(lanes(&observe(&own_dept(&sp), &health(other.clone(), false), &mut st2)).iter().all(|x| x.1 != "lane_unobserved"));
        }
    }

    /// W4 (T1): the bridge's path for this lane is not this daemon's pack spool -> path_disagrees; separators do not count.
    #[test]
    fn spw_w4_path_disagrees() {
        let d = tmp("w4");
        let sp = spool_path_for_pack(&d);
        let mut e = fleet(&sp);
        e[2] = entry("dept-5", Some("/x/cys-dept-dept-5/cys.sock"), "ok", false, Some("/elsewhere/state/evt_spool.jsonl"), Some(0));
        let p = observe(&own_dept(&sp), &health(e, true), &mut WatchState::default());
        assert!(lanes(&p).contains(&("dept-5".to_string(), "path_disagrees".to_string(), true)));
        let mut e2 = fleet(&sp);
        e2[2] = entry("dept-5", Some("/x/cys-dept-dept-5/cys.sock"), "ok", false, Some(&format!("{}/", sp.display())), Some(0));
        let p2 = observe(&own_dept(&sp), &health(e2, true), &mut WatchState::default());
        assert!(!lanes(&p2).iter().any(|x| x.1 == "path_disagrees"));
    }

    /// W5 (T1): own spool larger than the offset with no progress -> reader_stalled on the THIRD probe; progress resets.
    #[test]
    fn spw_w5_reader_stalled_on_the_third_probe_without_progress() {
        let d = tmp("w5");
        let sp = spool_path_for_pack(&d);
        std::fs::write(&sp, vec![b'x'; 100]).unwrap();
        let at = |off: u64| {
            let mut e = fleet(&sp);
            e[2] = entry("dept-5", Some("/x/cys-dept-dept-5/cys.sock"), "ok", false, Some(&sp.to_string_lossy()), Some(off));
            health(e, true)
        };
        let mut st = WatchState::default();
        let stalled = |p: Value| lanes(&p).iter().filter(|x| x.1 == "reader_stalled").count();
        let n: Vec<usize> = (0..3).map(|_| stalled(observe(&own_dept(&sp), &at(40), &mut st))).collect();
        println!("SPW-W5 per probe = {n:?}");
        assert_eq!(n, vec![0, 0, 1]);
        assert_eq!(stalled(observe(&own_dept(&sp), &at(60), &mut st)), 0, "progress resets the count");
        assert_eq!(stalled(observe(&own_dept(&sp), &at(100), &mut st)), 0, "caught up is not stalled");
        // a lane in fault is not read - W5 does not add to it
        let mut e = fleet(&sp);
        e[2] = entry("dept-5", Some("/x/cys-dept-dept-5/cys.sock"), "mismatch", true, Some(&sp.to_string_lossy()), Some(0));
        let mut st2 = WatchState::default();
        for _ in 0..3 {
            assert_eq!(stalled(observe(&own_dept(&sp), &health(e.clone(), true), &mut st2)), 0);
        }
    }

    /// Golden (R2 D2-1): the bridge's own `health_body()` output, frozen by the Python test that writes it, parses here and
    /// yields exactly the expected own entries - so a drift of the cross-language contract fails in CI, not at runtime.
    #[test]
    fn spw_golden_health_fixture_from_the_bridge_parses_to_the_expected_own_entries() {
        let body = include_str!("../../../cysjavis-pack/bin/tests/fixtures/office_spool_health_golden.json");
        let h = parse_health(body.as_bytes());
        assert!(h.is_ok(), "the bridge's golden /health must be understood (schema 1, spool array)");
        let d5 = Own {
            socket: "/Users/x/.local/state/cys-dept-dept-5/cys.sock".into(),
            is_base: false,
            lane: "dept-5".into(),
            spool_path: PathBuf::from("/RIG/dept-5/state/evt_spool.jsonl"),
        };
        let p5 = observe(&d5, &h, &mut WatchState::default());
        assert_eq!(lanes(&p5), vec![("dept-5".to_string(), "mismatch".to_string(), true)], "dept-5 keeps only its own entry");
        let base = Own {
            socket: "/Users/x/.local/state/cys/cys.sock".into(),
            is_base: true,
            lane: "main".into(),
            spool_path: PathBuf::from("/RIG/main/state/evt_spool.jsonl"),
        };
        let pb = observe(&base, &h, &mut WatchState::default());
        assert_eq!(
            lanes(&pb),
            vec![("main".to_string(), "ok".to_string(), false), ("_bridge".to_string(), "ok".to_string(), false),
                 ("_fleet".to_string(), "ok".to_string(), false)],
            "the base keeps main + the globals, no W4 (same path), no dept lane"
        );
        assert_eq!(pb["digest"]["faults"], json!(["dept-5 mismatch", "dept-7 unknown_pack"]));
    }

    /// End to end on the real carrier D (no 60 s timer): a fake bridge serves `/health` over TCP -> the real `probe_once`
    /// (`cys::bridge_probe_get`) -> `observe` -> the publish gate -> the daemon's own bus -> the real router task
    /// (`alert_route::spawn`, its pump and executor) -> `enqueue_into_seat` -> this dept's master queue. Nothing else hears it.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spw_e2e_fake_bridge_to_own_master_through_the_real_router_task() {
        use std::io::{Read, Write};
        let root = tmp("e2e");
        let dir = root.join("cys-dept-dept-5");
        std::fs::create_dir_all(&dir).unwrap();
        let d5 = Daemon::new(dir.join("cys.sock"));
        let s = d5.create_surface(None, Some("sleep 30".into()), None, Some("master".into()), 24, 80).expect("seat");
        d5.roles.lock().unwrap().insert("master".into(), s.id);
        d5.surfaces.lock().unwrap().insert(s.id, s.clone());
        *s.agent_meta.lock().unwrap() = Some(("claude".into(), "/usr/local/bin/claude".into()));
        let sock = d5.socket_path.to_string_lossy().to_string();
        let body = json!({"spool_schema": 1, "spool_checked": true, "spool": [
            {"lane": "dept-5", "socket": sock, "state": "mismatch", "fault": true, "path": null, "bytes": null,
             "archives": 0, "archive_bytes": 0, "offset": null, "foreign_key_dropped": 0},
            {"lane": "dept-3", "socket": "/x/cys-dept-dept-3/cys.sock", "state": "pack_missing", "fault": true, "path": null,
             "bytes": null, "archives": 0, "archive_bytes": 0, "offset": null, "foreign_key_dropped": 0}]}).to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            c.write_all(resp.as_bytes()).unwrap();
        });
        crate::alert_route::spawn(Arc::clone(&d5));
        let probe = tokio::task::spawn_blocking(move || probe_once(port)).await.unwrap();
        srv.join().unwrap();
        assert!(probe.is_ok(), "the real probe must parse the fake bridge's /health");
        let own = Own { socket: sock.clone(), is_base: false, lane: "dept-5".into(), spool_path: root.join("state").join(SPOOL_BASENAME) };
        let payload = observe(&own, &probe, &mut WatchState::default());
        assert!(crate::alert_route::office_spool_wants(&d5, &payload));
        d5.bus.publish(OFFICE_SPOOL_EVENT, "office_spool", None, payload);
        let mut got = Vec::new();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            got = s.pending_queue.lock().unwrap().iter().map(|e| e.text.clone()).collect();
            if !got.is_empty() {
                break;
            }
        }
        println!("SPW-E2E master queue = {got:?}");
        assert_eq!(got.len(), 1, "the router task delivered the observation to this dept's own master");
        assert!(got[0].contains("dept-5 mismatch") && !got[0].contains("dept-3"));
    }

    /// T6 (formula pin): this file's spool formula is the Python leaf's, in both languages, by text.
    #[test]
    fn spool_formula_matches_the_python_leaf() {
        let leaf = include_str!("../../../cysjavis-pack/bin/javis_spool_path.py");
        assert!(leaf.contains(&format!("SPOOL_STATE_DIRNAME = \"{SPOOL_STATE_DIRNAME}\"")), "leaf state dir name drifted");
        assert!(leaf.contains(&format!("SPOOL_BASENAME = \"{SPOOL_BASENAME}\"")), "leaf basename drifted");
        assert!(leaf.contains("return os.path.join(pack_dir, SPOOL_STATE_DIRNAME, SPOOL_BASENAME)"), "leaf formula drifted");
        assert_eq!(spool_path_for_pack(Path::new("/p")), Path::new("/p/state/evt_spool.jsonl"));
    }

    /// T6 (inverse pin): the watch only reads - no write, append, rename or remove call in its production part.
    #[test]
    fn spool_watch_never_writes() {
        let src = include_str!("spool_watch.rs");
        let prod = src.split("#[cfg(test)]\nmod tests {").next().unwrap();
        for bad in ["fs::write", "File::create", "OpenOptions", "fs::rename", "remove_file", "remove_dir", "create_dir",
                    "set_len", "write_all", "append("] {
            assert!(!prod.contains(bad), "spool_watch.rs production part calls `{bad}`");
        }
        assert!(prod.contains("std::fs::metadata("), "positive control: the one read (W5 stat) is visible to this pin");
        assert!(
            prod.contains("if crate::alert_route::office_spool_wants(&daemon, &payload) {\n                daemon.bus.publish(OFFICE_SPOOL_EVENT"),
            "the publish is gated by office_spool_wants (R1-1: a steady all-ok daemon publishes nothing)"
        );
        assert_eq!(prod.matches(".bus.publish(").count(), 1, "one publish site, the gated one");
    }

    /// T6 (disjointness pin): the router's production part names none of the faulting carrier's access tokens
    /// (DESIGN-r3 4-5; the topic word `spool` is allowed - the router must name the event).
    #[test]
    fn office_spool_router_never_touches_the_faulting_carrier() {
        let src = include_str!("alert_route.rs");
        let prod = src.split("\nmod drills {").next().unwrap();
        assert!(prod.contains("pub const OFFICE_SPOOL_EVENT"), "positive control: the router's office.spool leg is in the measured part");
        assert!(prod.matches("enqueue_into_seat(").count() >= 3, "positive control: the delivery primitive is visible");
        let toks = ["evt_", "javis_hud_bridge", "javis_event", "HUD_STATE_DIR", "HUD_PORT", "8642", "bridge_probe_get"];
        for t in toks {
            let tok = if t == "evt_" { "evt_spool" } else { t };
            assert!(!prod.contains(tok), "alert_route.rs production part names the faulting carrier token `{tok}`");
        }
    }
}
