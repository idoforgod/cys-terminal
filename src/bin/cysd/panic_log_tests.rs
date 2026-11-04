//! W60 D2 tests (DESIGN-W60-r2 §9.2; HEAD GO 2026-11-04 with codex r2 I1/I3 and the gemini example minor).
//! Pure tests drive `panic_log` through its instance-owned pieces (`SeatSnapshot`, a local sequence counter, a local
//! sink), never the process globals, so they cannot race other tests in this binary. Tests that need the real hook
//! run in a child process (the test binary re-run with `--exact` on `panic_log_child_fixture`).

use crate::panic_log::{self, Fields, SeatSnapshot, Seats, MAX_RECORD, RECORD_BUDGET, SEAT_SLOTS};
use crate::state::HideConsole;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("cys-w60-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn known(ids: &[u64], count: u32) -> Seats {
    let mut arr = [0u64; SEAT_SLOTS];
    for (d, s) in arr.iter_mut().zip(ids) {
        *d = *s;
    }
    Seats::Known { count, ids: arr, held: ids.len().min(SEAT_SLOTS) }
}

fn fields<'a>(seq: u32, msg: Option<&'a [u8]>, loc: Option<(&'a str, u32, u32)>, seats: Seats) -> Fields<'a> {
    Fields { unix_ms: Some(1_793_717_907_975), pid: 39_192, tid: Some(37_468), seq, build: ["0.14.57", "+", "3743ccb754"], seats, loc, msg }
}

fn render(f: &Fields<'_>) -> Vec<u8> {
    let mut buf = [0u8; MAX_RECORD];
    let n = panic_log::format_record(&mut buf, f);
    buf[..n].to_vec()
}

fn one(bytes: &[u8]) -> panic_log::ParsedRecord {
    let mut r = panic_log::parse(bytes);
    assert_eq!((r.accepted.len(), r.torn.len(), r.foreign), (1, 0, 0), "one clean record expected: {:?}", String::from_utf8_lossy(bytes));
    r.accepted.remove(0)
}

const LOC: (&str, u32, u32) = ("D:\\a\\cys\\mio-1.2.1\\src\\sys\\windows\\named_pipe.rs", 969, 24);
const MSG: &[u8] = b"internal error: entered unreachable code";

// ---------------------------------------------------------------- T-D2-1 formatter

/// Golden record. Its header was computed independently (python, FNV-1a 32) for this exact body; it also settles the
/// gemini r2 minor: the formatter derives len/chk from the body it wrote (the design's §3.3 example, len=0303
/// chk=cf8aa9be, was re-measured as matching its own body; this golden uses a non-profile path).
#[test]
fn t_d2_1_golden_record_bytes() {
    let got = render(&fields(1, Some(MSG), Some(LOC), known(&[181, 182, 183, 184, 185, 186, 189], 7)));
    let want = "#CYSPANIC1 len=0238 chk=76a850ef t=2026-11-03T14:58:27.975Z pid=39192 tid=37468 seq=1 build=0.14.57+3743ccb754 \
                seats=7 ids=181,182,183,184,185,186,189 loc=D:\\a\\cys\\mio-1.2.1\\src\\sys\\windows\\named_pipe.rs:969:24 \
                msg=internal%20error:%20entered%20unreachable%20code trunc=-\n";
    assert_eq!(String::from_utf8_lossy(&got), want);
}

#[test]
fn t_d2_1_worst_case_fits_and_names_every_cut() {
    let long_file = format!("C:\\rust\\with space\\{}\\x.rs", "d".repeat(500));
    let huge_msg = vec![b'#'; 10 * 1024];
    let all48: Vec<u64> = (0..48).map(|i| u64::MAX - i).rev().collect();
    let (bx, by) = ("x".repeat(40), "y".repeat(40));
    let f = Fields {
        unix_ms: Some(253_402_300_799_999),
        pid: u32::MAX,
        tid: Some(u64::MAX),
        seq: RECORD_BUDGET,
        build: [bx.as_str(), "+", by.as_str()],
        seats: known(&all48, u32::MAX),
        loc: Some((long_file.as_str(), u32::MAX, u32::MAX)),
        msg: Some(&huge_msg),
    };
    let bytes = render(&f);
    assert!(bytes.len() <= MAX_RECORD, "record {} bytes", bytes.len());
    let rec = one(&bytes);
    assert_eq!(rec.get("trunc"), Some(&b"blm"[..]));
    let msg = rec.get("msg").unwrap();
    assert!(msg.len() * 3 + 2 >= 176 && msg.iter().all(|b| *b == b'#'), "msg keeps at least 176 escaped bytes: {}", msg.len());
    assert!(rec.get("loc").unwrap().ends_with(b"x.rs:4294967295:4294967295"), "the loc tail is kept");
    assert_eq!(rec.get("ids").unwrap(), {
        let listed: Vec<String> = all48.iter().take(14).map(|v| v.to_string()).collect();
        format!("{}+4294967281", listed.join(",")).into_bytes()
    });
}

#[test]
fn t_d2_1_hostile_values_stay_one_record() {
    let cases: [(Option<&[u8]>, Option<(&str, u32, u32)>); 6] = [
        (Some("한글 메시지 \u{1F600}".as_bytes()), Some(("C:\\rust\\path with spaces\\a.rs", 1, 2))),
        (Some(b"line1\nline2\r\n\ttab #hash %pct"), None),
        (Some(b""), Some(("", 0, 0))),
        (None, Some((LOC.0, 9, 9))),
        (Some(&[0u8, 0x7F, 0xFF, b' ']), Some(("#%", 1, 1))),
        (Some(b"#CYSPANIC1 len=0010 chk=00000000 forged\n"), Some(("f.rs", 1, 1))),
    ];
    for (msg, loc) in cases {
        let bytes = render(&fields(2, msg, loc, Seats::Unknown));
        assert!(bytes.len() <= MAX_RECORD);
        assert_eq!(bytes.iter().filter(|b| **b == b'#').count(), 1, "only the start byte is a raw '#'");
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1, "only the end byte is a newline");
        let rec = one(&bytes);
        assert_eq!(rec.get("seats"), Some(&b"?"[..]));
        assert_eq!(rec.get("ids"), Some(&b"?"[..]));
        // T-D2-2 round trip: every value that was not cut decodes to its exact input bytes.
        assert_eq!(rec.get("msg").unwrap(), msg.unwrap_or(b"?"));
        if let Some((file, line, col)) = loc {
            assert_eq!(rec.get("loc").unwrap(), format!("{file}:{line}:{col}").as_bytes());
        }
    }
}

#[test]
fn t_d2_1_time_and_seat_edges() {
    let never = render(&Fields { unix_ms: None, ..fields(1, Some(b"m"), None, Seats::Never) });
    let rec = one(&never);
    assert_eq!((rec.get("t"), rec.get("seats"), rec.get("ids"), rec.get("loc")), (Some(&b"?"[..]), Some(&b"0"[..]), Some(&b"-"[..]), Some(&b"?"[..])));
    for (ms, want) in [(0u64, "1970-01-01T00:00:00.000Z"), (951_782_400_000, "2000-02-29T00:00:00.000Z"), (253_402_300_799_999, "9999-12-31T23:59:59.999Z")] {
        let rec = one(&render(&Fields { unix_ms: Some(ms), ..fields(1, None, None, Seats::Never) }));
        assert_eq!(rec.get("t").unwrap(), want.as_bytes());
    }
    let past = one(&render(&Fields { unix_ms: Some(253_402_300_800_000), ..fields(1, None, None, Seats::Never) }));
    assert_eq!(past.get("t"), Some(&b"?"[..]));
    let empty = one(&render(&fields(1, None, None, known(&[], 0))));
    assert_eq!((empty.get("seats"), empty.get("ids")), (Some(&b"0"[..]), Some(&b"-"[..])));
    let more = one(&render(&fields(1, None, None, known(&[1, 2, 3], 200))));
    assert_eq!(more.get("ids"), Some(&b"1,2,3+197"[..]));
}

// ---------------------------------------------------------------- T-D2-3 resynchronisation (ruling R1-M2)

fn rec_a() -> Vec<u8> {
    render(&fields(1, Some(&[b'A'; 600]), Some(LOC), known(&[5, 6], 2)))
}
fn rec_b() -> Vec<u8> {
    render(&fields(2, Some(b"B"), Some(LOC), known(&[5], 1)))
}

#[test]
fn t_d2_3_codex_case_512_bytes_of_a_then_complete_b() {
    let (a, b) = (rec_a(), rec_b());
    assert!(a.len() > 512);
    let mut file = a[..512].to_vec();
    file.extend_from_slice(&b);
    let r = panic_log::parse(&file);
    assert_eq!((r.accepted.len(), r.torn.len()), (1, 1));
    assert_eq!(r.accepted[0].get("seq"), Some(&b"2"[..]));
    assert_eq!(r.torn[0].0, 0);
}

#[test]
fn t_d2_3_every_short_length_of_a_then_b() {
    let (a, b) = (rec_a(), rec_b());
    // Not vacuous: A and B must each parse as one whole record, so the loop below runs over every prefix length of a real A.
    assert_eq!((panic_log::parse(&a).accepted.len(), panic_log::parse(&b).accepted.len()), (1, 1), "A and B are whole records");
    for k in 0..a.len() {
        let mut file = a[..k].to_vec();
        file.extend_from_slice(&b);
        let r = panic_log::parse(&file);
        let want_torn = usize::from(k > 0);
        assert_eq!((r.accepted.len(), r.torn.len()), (1, want_torn), "k={k}");
        assert_eq!(r.accepted[0].offset, k, "k={k}: B is found where it starts");
        assert_eq!(r.accepted[0].get("seq"), Some(&b"2"[..]), "k={k}");
    }
}

#[test]
fn t_d2_3_interleave_flip_wrong_len_and_foreign_bytes() {
    let (a, b) = (rec_a(), rec_b());
    for m in [1usize, 11, 33, 200, a.len() - 1] {
        let mut file = a[..m].to_vec();
        file.extend_from_slice(&b);
        file.extend_from_slice(&a[m..]);
        let r = panic_log::parse(&file);
        assert_eq!((r.accepted.len(), r.torn.len()), (1, 1), "m={m}");
        assert_eq!(r.accepted[0].get("seq"), Some(&b"2"[..]));
        assert_eq!(r.foreign, a.len() - m, "m={m}: A's tail is foreign bytes");
    }
    // One flipped bit in a complete body: checksum; the next record still accepted.
    let mut flipped = a.clone();
    flipped[100] ^= 0x01;
    flipped.extend_from_slice(&b);
    let r = panic_log::parse(&flipped);
    assert_eq!((r.accepted.len(), r.torn.len(), r.torn[0].1), (1, 1, "checksum"));
    // A wrong declared length.
    let mut wrong = a.clone();
    wrong[18] = if wrong[18] == b'9' { b'0' } else { wrong[18] + 1 };
    wrong.extend_from_slice(&b);
    let r = panic_log::parse(&wrong);
    assert_eq!((r.accepted.len(), r.torn.len()), (1, 1));
    assert!(["newline", "body", "short"].contains(&r.torn[0].1), "{}", r.torn[0].1);
    // Foreign bytes around and between records.
    let mut noisy = b"garbage\n".to_vec();
    noisy.extend_from_slice(&a);
    noisy.extend_from_slice(b"\r\nmore");
    noisy.extend_from_slice(&b);
    noisy.extend_from_slice(b"tail");
    let r = panic_log::parse(&noisy);
    assert_eq!((r.accepted.len(), r.torn.len(), r.foreign), (2, 0, 8 + 6 + 4));
    let r = panic_log::parse(b"");
    assert_eq!((r.accepted.len(), r.torn.len(), r.foreign), (0, 0, 0));
}

// ---------------------------------------------------------------- T-D2-4 start time

#[test]
fn t_d2_4_prepare_rotates_over_1_mib_and_never_repairs() {
    let dir = temp_dir("prep");
    let log = dir.join(panic_log::LOG_NAME);
    let old = dir.join(panic_log::LOG_NAME_OLD);
    std::fs::write(&old, b"older").unwrap();
    std::fs::write(&log, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    drop(panic_log::prepare(&dir).expect("prepare"));
    assert_eq!(std::fs::metadata(&old).unwrap().len(), 1024 * 1024 + 1, "rotated, replacing the older .1");
    assert_eq!(std::fs::metadata(&log).unwrap().len(), 0, "a fresh log is opened");
    let torn = &rec_a()[..300];
    std::fs::write(&log, torn).unwrap();
    drop(panic_log::prepare(&dir).expect("prepare"));
    assert_eq!(std::fs::read(&log).unwrap(), torn, "a torn tail is left byte-identical (no repair)");
    let not_a_dir = dir.join("file");
    std::fs::write(&not_a_dir, b"").unwrap();
    assert!(panic_log::prepare(&not_a_dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t_d2_4_log_names_are_not_update_leftovers() {
    for name in [panic_log::LOG_NAME, panic_log::LOG_NAME_OLD] {
        assert!(!crate::is_update_leftover(name), "{name}");
    }
}

// ---------------------------------------------------------------- T-D2-9 snapshot (ruling R1-M3, codex r2 I1)

#[test]
fn t_d2_9c_publish_keeps_exactly_the_smallest_48() {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut map = HashMap::new();
    while map.len() < 200 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        map.insert(x % 1_000_000, ());
    }
    let snap = SeatSnapshot::new();
    assert_eq!(snap.read(), Seats::Never);
    snap.publish(map.keys().copied(), map.len());
    let mut sorted: Vec<u64> = map.keys().copied().collect();
    sorted.sort_unstable();
    match snap.read() {
        Seats::Known { count, ids, held } => {
            assert_eq!((count, held), (200, SEAT_SLOTS));
            assert_eq!(&ids[..], &sorted[..SEAT_SLOTS]);
        }
        other => panic!("{other:?}"),
    }
    snap.publish(std::iter::empty(), 0);
    assert_eq!(snap.read(), known(&[], 0));
}

#[test]
fn t_d2_9_counters_saturate_never_wrap() {
    // seq: 64 records, then never again — also from a counter already at u32::MAX.
    let seq = AtomicU32::new(0);
    let got: Vec<Option<u32>> = (0..66).map(|_| panic_log::take_seq(&seq)).collect();
    assert_eq!(got[..64].iter().map(|v| v.unwrap()).collect::<Vec<_>>(), (1..=64).collect::<Vec<_>>());
    assert_eq!((got[64], got[65]), (None, None));
    let full = AtomicU32::new(u32::MAX);
    assert_eq!(panic_log::take_seq(&full), None);
    assert_eq!(full.load(Ordering::SeqCst), u32::MAX);
    // generation: completes while it can, then parks at u64::MAX (odd = unknown) and stays there.
    let snap = SeatSnapshot::new();
    snap.force_gen(u64::MAX - 3);
    snap.publish([9u64].into_iter(), 1);
    assert_eq!((snap.gen(), snap.read()), (u64::MAX - 1, known(&[9], 1)));
    snap.publish([8u64].into_iter(), 1);
    assert_eq!((snap.gen(), snap.read()), (u64::MAX, Seats::Unknown));
    snap.publish([7u64].into_iter(), 1);
    assert_eq!((snap.gen(), snap.read()), (u64::MAX, Seats::Unknown));
    let dead = SeatSnapshot::new();
    dead.force_gen(u64::MAX - 2); // a publication died here (odd)
    dead.publish([1u64].into_iter(), 1);
    assert_eq!((dead.gen(), dead.read()), (u64::MAX, Seats::Unknown));
    let dead_low = SeatSnapshot::new();
    dead_low.force_gen(7); // died mid-publication: the next start stays odd, the end is even, the read is accepted
    dead_low.publish([4u64, 2].into_iter(), 2);
    assert_eq!((dead_low.gen(), dead_low.read()), (10, known(&[2, 4], 2)));
}

/// The protocol under sequential consistency (what all-SeqCst code guarantees), every interleaving of one
/// publication against one read, over a count and 2 id words. `start_rule` is the publisher's start computation, so
/// the same model also proves it can SEE the r1 defect (gen + 1 after a dead publication inverts the parity).
fn model_mixed_reads(start_rule: fn(u64) -> u64, dead_first: bool, stop_after: Option<usize>) -> (usize, usize) {
    #[derive(Clone, Copy)]
    struct Mem {
        gen: u64,
        count: u64,
        id: [u64; 2],
    }
    let old = (1u64, [11u64, 0]);
    let new = (2u64, [21u64, 22]);
    let mut base = Mem { gen: 2, count: old.0, id: old.1 };
    if dead_first {
        // A publication died after its start store and one payload store.
        base.gen = 3;
        base.id[0] = 99;
    }
    let start = start_rule(base.gen);
    // Publisher ops: 0 start, 1 count, 2 id0, 3 id1, 4 end.
    let pub_ops = stop_after.unwrap_or(5);
    // Reader ops: 0 g1, 1 count, 2 id0, 3 id1, 4 g2.
    let (mut accepted, mut mixed) = (0usize, 0usize);
    fn walk(mem: Mem, p: usize, r: usize, regs: [u64; 5], pub_ops: usize, start: u64, new: (u64, [u64; 2]), old: (u64, [u64; 2]), acc: &mut usize, mixed: &mut usize) {
        if r == 5 {
            let (g1, c, i0, i1, g2) = (regs[0], regs[1], regs[2], regs[3], regs[4]);
            if g1 != 0 && g1 % 2 == 0 && g1 == g2 {
                *acc += 1;
                let snap = (c, [i0, if c > 1 { i1 } else { 0 }]);
                let norm = |s: (u64, [u64; 2])| (s.0, [s.1[0], if s.0 > 1 { s.1[1] } else { 0 }]);
                if snap != norm(new) && snap != norm(old) {
                    *mixed += 1;
                }
            }
            return;
        }
        if p < pub_ops {
            let mut m = mem;
            match p {
                0 => m.gen = start,
                1 => m.count = new.0,
                2 => m.id[0] = new.1[0],
                3 => m.id[1] = new.1[1],
                _ => m.gen = start + 1,
            }
            walk(m, p + 1, r, regs, pub_ops, start, new, old, acc, mixed);
        }
        let mut regs2 = regs;
        regs2[r] = match r {
            0 | 4 => mem.gen,
            1 => mem.count,
            2 => mem.id[0],
            _ => mem.id[1],
        };
        walk(mem, p, r + 1, regs2, pub_ops, start, new, old, acc, mixed);
    }
    walk(base, 0, 0, [0; 5], pub_ops, start, new, if dead_first { (old.0, [99, 0]) } else { old }, &mut accepted, &mut mixed);
    (accepted, mixed)
}

#[test]
fn t_d2_9a_every_interleaving_accepts_only_whole_publications() {
    let r2_rule: fn(u64) -> u64 = |g| (g + 1) | 1;
    for stop in [None, Some(1), Some(2), Some(3), Some(4)] {
        for dead in [false, true] {
            let (acc, mixed) = model_mixed_reads(r2_rule, dead, stop);
            assert_eq!(mixed, 0, "r2 rule: stop={stop:?} dead_first={dead}");
            if stop.is_none() && !dead {
                assert!(acc > 0, "the model accepts some reads (it is not vacuous)");
            }
        }
    }
    // Positive control: r1's `gen + 1` start after a dead publication makes the model find mixed accepted reads.
    let r1_rule: fn(u64) -> u64 = |g| g + 1;
    let (_, mixed) = model_mixed_reads(r1_rule, true, None);
    assert!(mixed > 0, "the model must be able to see the parity inversion");
    // The model is worth something only if SeatSnapshot implements the rule it checks: after a publication that died with
    // the generation odd (5), the next start must stay odd ((5 + 1) | 1 = 7), its end even (8), and the result read back.
    // r1's rule would start at 6 and end at 7 (odd), so this read would say Unknown.
    let snap = SeatSnapshot::new();
    snap.force_gen(5);
    snap.publish([9u64, 4].into_iter(), 2);
    assert_eq!(snap.gen(), 8, "publish follows the r2 start rule (g + 1) | 1 after a dead publication");
    assert_eq!(snap.read(), known(&[4, 9], 2));
}

#[test]
fn t_d2_9b_stress_no_mixed_snapshot_is_ever_accepted() {
    use std::sync::Arc;
    let snap = Arc::new(SeatSnapshot::new());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let (snap, stop) = (snap.clone(), stop.clone());
            std::thread::spawn(move || {
                let (mut accepted, mut reads) = (0u64, 0u64);
                while !stop.load(Ordering::SeqCst) {
                    reads += 1;
                    if let Seats::Known { count, ids, held } = snap.read() {
                        accepted += 1;
                        if held > 0 {
                            let base = ids[0];
                            assert_eq!(base % 1000, 0, "ids[0] {base}");
                            assert_eq!((base / 1000) % 49, u64::from(count), "count matches its publication");
                            for (i, id) in ids.iter().take(held).enumerate() {
                                assert_eq!(*id, base + i as u64, "one publication only");
                            }
                        }
                    }
                }
                (accepted, reads)
            })
        })
        .collect();
    for k in 1..=200_000u64 {
        let n = (k % 49) as usize;
        snap.publish((0..n as u64).map(|i| k * 1000 + i), n);
    }
    stop.store(true, Ordering::SeqCst);
    let (mut acc, mut reads) = (0, 0);
    for r in readers {
        let (a, n) = r.join().expect("reader");
        acc += a;
        reads += n;
    }
    println!("t_d2_9b: reads={reads} accepted={acc}");
    // Not vacuous: whatever the readers saw, the last publication (k = 200,000: 200,000 % 49 = 31 ids from 200,000,000)
    // reads back whole once the publisher has stopped. A snapshot that never accepts anything fails here.
    let last: Vec<u64> = (0..31u64).map(|i| 200_000_000 + i).collect();
    assert_eq!(snap.read(), known(&last, 31));
}

// ---------------------------------------------------------------- flood (§3.9)

#[test]
fn flood_64_records_then_nothing() {
    let (seq, snap) = (AtomicU32::new(0), SeatSnapshot::new());
    let mut sink: Vec<u8> = Vec::new();
    for i in 1..=65u32 {
        let wrote = panic_log::emit(&mut sink, &seq, &snap, Some(LOC), Some(b"flood"));
        assert_eq!(wrote, i <= RECORD_BUDGET, "panic {i}");
    }
    let r = panic_log::parse(&sink);
    assert_eq!((r.accepted.len(), r.torn.len(), r.foreign), (64, 0, 0));
    let seqs: Vec<String> = r.accepted.iter().map(|x| String::from_utf8_lossy(x.get("seq").unwrap()).into_owned()).collect();
    assert_eq!(seqs, (1..=64).map(|v| v.to_string()).collect::<Vec<_>>());
}

// ---------------------------------------------------------------- T-D2-11 blocked sink (unix; ruling R1-M1, codex r2 I3)

#[cfg(unix)]
struct EnteredSink {
    inner: std::fs::File,
    entered: std::sync::mpsc::Sender<()>,
    calls: std::sync::Arc<AtomicU32>,
}

#[cfg(unix)]
impl std::io::Write for EnteredSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.send(()); // the signal from inside the sink, immediately before the real write
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn fill_pipe(file: &std::fs::File) -> usize {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0);
    let (chunk, mut n) = ([b'x'; 4096], 0usize);
    let mut w = file;
    loop {
        match w.write(&chunk) {
            Ok(k) if k > 0 => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            _ => break,
        }
    }
    assert!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } == 0);
    n
}

/// `(was still inside the write after 300 ms, bytes after the filler, write calls)`.
#[cfg(unix)]
fn blocked_sink_run(fill: bool) -> (bool, Vec<u8>, u32) {
    use std::os::fd::OwnedFd;
    use std::sync::Arc;
    let (mut reader, writer) = std::io::pipe().expect("pipe");
    let file = std::fs::File::from(OwnedFd::from(writer));
    let filled = if fill { fill_pipe(&file) } else { 0 };
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let calls = Arc::new(AtomicU32::new(0));
    let snap = Arc::new(SeatSnapshot::new());
    let worker = {
        let (snap, calls) = (snap.clone(), calls.clone());
        std::thread::spawn(move || {
            let mut sink = EnteredSink { inner: file, entered: entered_tx, calls };
            let seq = AtomicU32::new(0);
            let wrote = panic_log::emit(&mut sink, &seq, &snap, Some(LOC), Some(b"blocked-sink"));
            let _ = done_tx.send(wrote);
            drop(sink); // closes the write end, so the reader sees EOF
        })
    };
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("the worker entered the write"); // wallclock: guard bounds thread start; entry is the signal from inside the sink
    let early = done_rx.recv_timeout(Duration::from_millis(300)); // wallclock: guard a blocked write cannot return before the drain below; an early return on a full pipe is a real failure
    let still_blocked = early.is_err();
    // While the hook waits, others can still publish and read the snapshot: the hook holds no lock of its own.
    snap.publish([3u64, 1].into_iter(), 2);
    assert_eq!(snap.read(), known(&[1, 3], 2));
    let reader_thread = std::thread::spawn(move || {
        let mut drained = Vec::new();
        let _ = reader.read_to_end(&mut drained);
        drained
    });
    let wrote = match early {
        Ok(w) => w,
        Err(_) => done_rx.recv_timeout(Duration::from_secs(30)).expect("the write returns once the sink drains"), // wallclock: guard hang guard after the drain starts
    };
    assert!(wrote);
    worker.join().expect("worker");
    let bytes = reader_thread.join().expect("reader");
    assert!(bytes.len() >= filled);
    (still_blocked, bytes[filled..].to_vec(), calls.load(Ordering::SeqCst))
}

#[cfg(unix)]
#[test]
fn t_d2_11_blocked_sink_waits_inside_one_write_then_completes() {
    // Control: an empty pipe — the same call returns at once after entering.
    let (blocked, record, calls) = blocked_sink_run(false);
    assert!(!blocked, "control: an unblocked sink returns at once");
    assert_eq!(calls, 1, "one write call");
    assert_eq!(one(&record).get("msg"), Some(&b"blocked-sink"[..]));
    // Blocked: the call entered the write, was still inside it 300 ms later, and completed after the drain.
    let (blocked, record, calls) = blocked_sink_run(true);
    assert!(blocked, "the call waits while the sink is blocked (behaviour, not a bound)");
    assert_eq!(calls, 1, "still exactly one write call");
    assert_eq!(one(&record).get("msg"), Some(&b"blocked-sink"[..]));
}

// ---------------------------------------------------------------- census pin (§3.5)

/// `src` with the inside of every comment, string and char literal replaced by spaces; newlines are kept, so line numbers do
/// not move. Braces and call patterns are then counted in code only. Measured at c1: counting braces in raw text let a
/// `"\n}\n"` inside a test module (accounts.rs:4230, governance.rs:15133) end the skip early, and 36 test-only sites leaked.
/// Handles `"…"` and `b"…"` with escapes, `r"…"` / `r#"…"#` / `br#"…"#`, `'c'` / `'\n'` / `'\u{7B}'` (a lifetime `'a` is code),
/// `//` and nested `/* */` comments.
fn code_only(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |out: &mut String, ch: char| out.push(if ch == '\n' { '\n' } else { ' ' });
    let mut i = 0usize;
    while i < c.len() {
        let (ch, next) = (c[i], c.get(i + 1).copied());
        if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                blank(&mut out, c[i]);
                i += 1;
            }
            continue;
        }
        if ch == '/' && next == Some('*') {
            let mut depth = 0i32;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                } else {
                    blank(&mut out, c[i]);
                    i += 1;
                    continue;
                }
                out.push_str("  ");
                i += 2;
                if depth == 0 {
                    break;
                }
            }
            continue;
        }
        let after_ident = i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_');
        if !after_ident && (ch == 'r' || (ch == 'b' && next == Some('r'))) {
            let mut j = i + if ch == 'b' { 2 } else { 1 };
            let mut hashes = 0usize;
            while c.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if c.get(j) == Some(&'"') {
                out.extend(&c[i..=j]);
                j += 1;
                while j < c.len() {
                    if c[j] == '"' && (1..=hashes).all(|h| c.get(j + h) == Some(&'#')) {
                        out.extend(&c[j..=j + hashes]);
                        j += 1 + hashes;
                        break;
                    }
                    blank(&mut out, c[j]);
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        if ch == '"' {
            out.push('"');
            i += 1;
            while i < c.len() {
                if c[i] == '\\' {
                    blank(&mut out, c[i]);
                    if let Some(&e) = c.get(i + 1) {
                        blank(&mut out, e);
                    }
                    i += 2;
                    continue;
                }
                if c[i] == '"' {
                    out.push('"');
                    i += 1;
                    break;
                }
                blank(&mut out, c[i]);
                i += 1;
            }
            continue;
        }
        if ch == '\'' {
            let close = if next == Some('\\') {
                (i + 3..c.len()).take_while(|&k| c[k] != '\n').find(|&k| c[k] == '\'')
            } else if next.is_some() && next != Some('\n') && c.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None // a lifetime or a label: code
            };
            if let Some(end) = close {
                out.push('\'');
                for &k in &c[i + 1..end] {
                    blank(&mut out, k);
                }
                out.push('\'');
                i = end + 1;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Production text of a cysd source: comments, strings and chars blanked ([`code_only`]), and every item under a test-only
/// cfg attribute skipped (by brace balance; an item whose `{` comes on a later line is skipped until it opens and closes).
fn production_text(src: &str) -> String {
    let src = code_only(src);
    let mut out = String::new();
    let (mut pending, mut skip): (bool, Option<(i64, bool)>) = (false, None);
    for line in src.lines() {
        let t = line.trim_start();
        let (opens, closes) = (line.matches('{').count() as i64, line.matches('}').count() as i64);
        if let Some((depth, opened)) = skip.as_mut() {
            *depth += opens - closes;
            *opened |= opens > 0;
            if (*opened && *depth <= 0) || (!*opened && t.trim_end().ends_with(';')) {
                skip = None;
            }
            out.push('\n');
            continue;
        }
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            pending = true;
            out.push('\n');
            continue;
        }
        if pending && !t.starts_with("#[") && !t.is_empty() {
            pending = false;
            let (depth, opened) = (opens - closes, opens > 0);
            if !((opened && depth <= 0) || (!opened && t.trim_end().ends_with(';'))) {
                skip = Some((depth, opened));
            }
            out.push('\n');
            continue;
        }
        out.push_str(if t.starts_with("//") { "" } else { line });
        out.push('\n');
    }
    out
}

/// Text after one call `.name(...)` at the start of `s` (whitespace allowed), for one of `names`.
fn after_call<'a>(s: &'a str, names: &[&str]) -> Option<&'a str> {
    let s = s.trim_start().strip_prefix('.')?.trim_start();
    let name = names.iter().find(|n| s.starts_with(&format!("{n}(")))?;
    let s = &s[name.len()..];
    let mut depth = 0i64;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[i + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `surfaces.<mutation>(` in production text, with whether `panic_log::publish_seats(` follows in the same block.
fn census_sites(name: &str, text: &str) -> Vec<(String, bool)> {
    let ops = ["insert", "remove", "retain", "clear", "drain", "entry"];
    let bytes = text.as_bytes();
    let mut sites = Vec::new();
    let mut from = 0usize;
    while let Some(off) = text[from..].find("surfaces") {
        let at = from + off;
        from = at + "surfaces".len();
        let before_ok = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        let mut rest = &text[from..];
        // `surfaces.lock().unwrap().insert(` and its variants count too.
        if let Some(r) = after_call(rest, &["lock"]) {
            rest = after_call(r, &["unwrap", "unwrap_or_else", "expect"]).unwrap_or(r);
        }
        let Some(after_dot) = rest.trim_start().strip_prefix('.') else { continue };
        let after_dot = after_dot.trim_start();
        let Some(op) = ops.iter().find(|op| after_dot.starts_with(&format!("{op}("))) else { continue };
        if !before_ok {
            continue;
        }
        let line = text[..at].matches('\n').count() + 1;
        let mut depth = 0i64;
        let mut ok = false;
        let tail = &text[at..];
        for (i, ch) in tail.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        break;
                    }
                }
                _ => {}
            }
            if tail[i..].starts_with("panic_log::publish_seats(") {
                ok = true;
                break;
            }
        }
        sites.push((format!("{name}:{line} surfaces.{op}("), ok));
    }
    sites
}

#[test]
fn census_every_surfaces_mutation_publishes() {
    // Negative control: the instrument reports a mutation without a publish.
    let fake = "fn f() {\n    let mut surfaces = d.surfaces.lock().unwrap();\n    surfaces.insert(1, s);\n}\n";
    assert_eq!(census_sites("fake.rs", &production_text(fake)), vec![("fake.rs:3 surfaces.insert(".to_string(), false)]);
    let fake_ok = "fn f() {\n    let mut surfaces = d.surfaces.lock().unwrap();\n    surfaces\n        .remove(&1);\n    crate::panic_log::publish_seats(&surfaces);\n}\n#[cfg(test)]\nmod tests {\n    fn g() { surfaces.insert(2, s); }\n}\n";
    assert_eq!(census_sites("ok.rs", &production_text(fake_ok)), vec![("ok.rs:3 surfaces.remove(".to_string(), true)]);
    let chained = "fn h() {\n    d.surfaces.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, _| true);\n}\n#[cfg(test)]\nfn t()\n{\n    surfaces.clear();\n}\n";
    assert_eq!(census_sites("ch.rs", &production_text(chained)), vec![("ch.rs:2 surfaces.retain(".to_string(), false)]);
    // Negative control for the leak measured at c1: a `"\n}\n"` string and a `'}'` char inside a test module must not end
    // the skip, and a call spelled inside a string or a comment is not a call.
    let leak = "#[cfg(test)]\nmod tests {\n    fn a() { let _ = x.find(\"\\n}\\n\"); let _ = '}'; }\n    fn b() { surfaces.insert(3, s); }\n}\nfn p() {\n    let _ = \"surfaces.insert(\"; // surfaces.remove(\n}\n";
    assert_eq!(census_sites("leak.rs", &production_text(leak)), Vec::<(String, bool)>::new());
    // A lifetime is code, not the start of a char literal: the call after it is still seen.
    let life = "fn q<'a>(v: &'a u8) {\n    surfaces.clear();\n}\n";
    assert_eq!(census_sites("life.rs", &production_text(life)), vec![("life.rs:2 surfaces.clear(".to_string(), false)]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd");
    let mut sites = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir).expect("cysd sources").filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("source");
        let found = census_sites(&name, &production_text(&src));
        // Second instrument: a site at or after the file's `#[cfg(test)]` + `mod tests {` header is a leak of the first.
        if let Some(at) = src.find("\n#[cfg(test)]\nmod tests {") {
            let header_line = src[..at].matches('\n').count() + 2;
            for (site, _) in &found {
                let line: usize = site.split(':').nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok()).unwrap_or(0);
                assert!(line > 0 && line < header_line, "census instrument leaked into {name}'s tests module (header line {header_line}): {site}");
            }
        }
        sites.extend(found);
    }
    println!("census: {sites:?}");
    // The instrument must see the two known sites, or it measured nothing (it refuses rather than passing on 0).
    assert!(sites.iter().any(|(s, _)| s.starts_with("state.rs:") && s.ends_with("surfaces.insert(")), "create site not seen: {sites:?}");
    assert!(sites.iter().any(|(s, _)| s.starts_with("governance.rs:") && s.ends_with("surfaces.remove(")), "close site not seen: {sites:?}");
    let missing: Vec<_> = sites.iter().filter(|(_, ok)| !ok).collect();
    assert!(missing.is_empty(), "production surfaces mutations without panic_log::publish_seats: {missing:?}");
}

// ---------------------------------------------------------------- child-process tests (T-D2-5/6/7/10, unavailable)

/// Fixture: runs only when `W60_PANIC_CHILD_MODE` is set by a parent below.
#[test]
fn panic_log_child_fixture() {
    let Ok(mode) = std::env::var("W60_PANIC_CHILD_MODE") else { return };
    let dir = PathBuf::from(std::env::var("W60_PANIC_CHILD_DIR").expect("child dir"));
    if mode == "localappdata" {
        // T-D2-8: the production entry, so the directory comes from the lane state dir rule (LOCALAPPDATA on Windows).
        panic_log::install(Path::new(T_D2_8_PIPE));
    } else {
        panic_log::install_in(&dir);
    }
    match mode.as_str() {
        "chain" | "unavailable" | "localappdata" => {
            let _ = std::thread::spawn(|| panic!("w60-chain-probe")).join();
        }
        "lock" => {
            let mut seats = HashMap::new();
            for id in [7u64, 3, 5] {
                seats.insert(id, ());
            }
            let held = std::sync::Arc::new(std::sync::Mutex::new(seats));
            let h2 = held.clone();
            let _ = std::thread::spawn(move || {
                let guard = h2.lock().unwrap();
                panic_log::publish_seats(&guard);
                panic!("w60-lock-holder");
            })
            .join();
            assert!(held.is_poisoned());
            let _ = std::thread::spawn(|| panic!("w60-second")).join();
        }
        "tls" => {
            struct Bomb;
            impl Drop for Bomb {
                fn drop(&mut self) {
                    panic!("w60-tls-drop");
                }
            }
            thread_local! { static BOMB: Bomb = const { Bomb }; }
            let _ = std::thread::spawn(|| BOMB.with(|_| ())).join();
        }
        "two" => {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let hs: Vec<_> = (0..2)
                .map(|i| {
                    let b = barrier.clone();
                    std::thread::spawn(move || {
                        b.wait();
                        panic!("w60-two-{i}");
                    })
                })
                .collect();
            for h in hs {
                let _ = h.join();
            }
        }
        other => panic!("unknown mode {other}"),
    }
}

/// A pipe name, so `state_dir` takes the LOCALAPPDATA branch on Windows (a file-system socket path would not).
const T_D2_8_PIPE: &str = r"\\.\pipe\cys-w60-td28";

fn run_child(mode: &str, dir: &Path) -> (Option<std::process::ExitStatus>, String, String) {
    run_child_with(mode, dir, &[])
}

fn run_child_with(mode: &str, dir: &Path, env: &[(&str, &Path)]) -> (Option<std::process::ExitStatus>, String, String) {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(std::env::current_exe().expect("test executable"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .args(["--exact", "panic_log_tests::panic_log_child_fixture", "--nocapture", "--test-threads=1"])
        .env("W60_PANIC_CHILD_MODE", mode)
        .env("W60_PANIC_CHILD_DIR", dir)
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .hide_console()
        .spawn()
        .expect("spawn child");
    let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop { // wallclock: guard the child ends in milliseconds; expiry fails the test as a hang (that is the no-hang check)
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    (status, out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default())
}

fn log_of(dir: &Path) -> panic_log::Report {
    panic_log::parse(&std::fs::read(dir.join(panic_log::LOG_NAME)).unwrap_or_default())
}

#[test]
fn t_d2_5_chaining_keeps_stderr_and_writes_one_record() {
    let dir = temp_dir("chain");
    let (status, _, err) = run_child("chain", &dir);
    assert!(status.map(|s| s.success()).unwrap_or(false), "child status {status:?}\n{err}");
    assert!(err.contains("panicked at") && err.contains("w60-chain-probe"), "default hook output unchanged: {err}");
    let r = log_of(&dir);
    assert_eq!((r.accepted.len(), r.torn.len()), (1, 0));
    assert_eq!(r.accepted[0].get("seq"), Some(&b"1"[..]));
    assert_eq!(r.accepted[0].get("msg"), Some(&b"w60-chain-probe"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t_d2_6_lock_holder_panic_records_seats_and_does_not_hang() {
    let dir = temp_dir("lock");
    let (status, _, err) = run_child("lock", &dir);
    assert!(status.is_some(), "the child finished within its guard (no hang)\n{err}");
    let r = log_of(&dir);
    assert_eq!((r.accepted.len(), r.torn.len()), (2, 0), "{err}");
    let first = &r.accepted[0];
    assert_eq!((first.get("seq"), first.get("seats"), first.get("ids")), (Some(&b"1"[..]), Some(&b"3"[..]), Some(&b"3,5,7"[..])));
    assert_eq!((r.accepted[1].get("seq"), r.accepted[1].get("msg")), (Some(&b"2"[..]), Some(&b"w60-second"[..])));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t_d2_7_tls_teardown_panic_is_recorded_and_the_hook_does_not_panic() {
    let dir = temp_dir("tls");
    let (status, _, err) = run_child("tls", &dir);
    assert!(status.is_some(), "child ended\n{err}");
    assert!(!err.contains("panicked while processing panic"), "the hook itself must not panic: {err}");
    let r = log_of(&dir);
    assert!(r.accepted.iter().any(|x| x.get("msg") == Some(&b"w60-tls-drop"[..])), "{r:?}\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t_d2_10_two_concurrent_panics() {
    let dir = temp_dir("two");
    let (status, _, err) = run_child("two", &dir);
    assert!(status.map(|s| s.success()).unwrap_or(false), "{err}");
    let r = log_of(&dir);
    println!("t_d2_10: accepted={} torn={}", r.accepted.len(), r.torn.len());
    assert_eq!(r.accepted.len() + r.torn.len(), 2);
    let mut seqs: Vec<&[u8]> = r.accepted.iter().filter_map(|x| x.get("seq")).collect();
    seqs.dedup();
    assert_eq!(seqs.len(), r.accepted.len(), "distinct seq values");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unavailable_log_gives_one_stderr_line_and_chain_only() {
    let base = temp_dir("unavail");
    let blocker = base.join("not-a-dir");
    std::fs::write(&blocker, b"").unwrap();
    let (status, _, err) = run_child("unavailable", &blocker);
    assert!(status.map(|s| s.success()).unwrap_or(false), "{err}");
    assert_eq!(err.matches("[cysd] panic-log unavailable:").count(), 1, "{err}");
    assert!(err.contains("panicked at") && err.contains("w60-chain-probe"), "chain-only keeps stderr: {err}");
    let _ = std::fs::remove_dir_all(&base);
}

/// Every `cysd-panic.log` under `root` (depth-first; the tree is the test's own temp dir).
#[cfg(windows)]
fn find_logs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) == Some(panic_log::LOG_NAME) {
                out.push(p);
            }
        }
    }
    out
}

/// T-D2-8 (Windows): LOCALAPPDATA names a directory with spaces and non-ASCII characters; the lane state dir under it
/// is created and the record is appended and accepted (DESIGN-W60-r2 §9.2).
#[cfg(windows)]
#[test]
fn t_d2_8_localappdata_with_spaces_and_non_ascii() {
    let top = temp_dir("lad");
    let base = top.join("W60 état 패닉 dir");
    std::fs::create_dir_all(&base).expect("LOCALAPPDATA stand-in");
    let (status, _, err) = run_child_with("localappdata", &base, &[("LOCALAPPDATA", &base)]);
    assert!(status.map(|s| s.success()).unwrap_or(false), "child status {status:?}\n{err}");
    let found = find_logs(&base);
    assert_eq!(found.len(), 1, "exactly one panic log under LOCALAPPDATA: {found:?}\n{err}");
    assert!(found[0].starts_with(base.join("cys")), "inside <LOCALAPPDATA>\\cys: {found:?}");
    let r = panic_log::parse(&std::fs::read(&found[0]).expect("read the log"));
    assert_eq!((r.accepted.len(), r.torn.len()), (1, 0), "{err}");
    assert_eq!(r.accepted[0].get("msg"), Some(&b"w60-chain-probe"[..]));
    let _ = std::fs::remove_dir_all(&top);
}
