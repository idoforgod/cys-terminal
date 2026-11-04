//! W60 D2 — the cysd panic record (DESIGN-W60-r2 §3; HEAD GO 2026-11-04 with codex r2 I1/I3).
//!
//! Every cysd panic after the single-instance gate appends one line to `<lane state dir>/cysd-panic.log`, best effort,
//! within a budget of 64 records per process. The line is self-delimiting (§3.3):
//! `#CYSPANIC1 len=DDDD chk=HHHHHHHH <body>\n` — `len` is the body length, `chk` the FNV-1a 32-bit hash of the body,
//! and every field value is escaped so a body never holds a raw space-in-value, `#`, `%` or newline. A reader that
//! re-synchronises on `#` therefore finds every record that landed whole, whatever was torn before it ([`parse`]).
//!
//! The hook contract (§3.6) — keep it when editing anything reachable from [`record`]:
//! * no `unwrap`/`expect`, no slice indexing outside fixed bounds, no `RefCell`, no lock, no allocation of its own;
//! * never `std::thread::current()` (it panics after TLS teardown, and a panic inside the hook aborts the process);
//! * ONE write call of at most one record (≤ 1024 bytes) to the file opened at start; no fsync, no retry, no lock.
//!   That call CAN block if the storage stalls — the daemon is already failing then; std's own default hook writes
//!   stderr synchronously the same way. A record is durable against a process abort only for the bytes that write
//!   call returned; an OS crash can lose it (no fsync).
//! * counters never wrap (codex r2 I1): the record sequence saturates at the budget, and the seat generation parks at
//!   `u64::MAX` (odd = unknown) instead of overflowing.
//!
//! Seat snapshot (§3.5): a seqlock whose every load and store is SeqCst, written only inside the existing `surfaces`
//! critical section (one serialized publisher; [`publish_seats`] at the two census-pinned sites).
//! Accepted read: two equal even generation loads around the payload loads. Under SeqCst all these operations lie in
//! one total order S; an even value e > 0 was stored by the end store of exactly one publication k (values strictly
//! increase, publishers are serialized, and the generation parks at an odd value instead of wrapping); both generation
//! loads read e, so every payload load lies after k's end store and before k+1's start store, and k writes every
//! payload word, so the snapshot is exactly publication k. A publication that died mid-way leaves an odd value, and
//! the next start `(g + 1) | 1` stays odd, so the parity never inverts.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};
use std::sync::OnceLock;

pub const LOG_NAME: &str = "cysd-panic.log";
pub const LOG_NAME_OLD: &str = "cysd-panic.log.1";
pub const MAX_RECORD: usize = 1024;
pub const HEADER_LEN: usize = 33;
pub const MAX_BODY: usize = MAX_RECORD - HEADER_LEN - 1;
pub const RECORD_BUDGET: u32 = 64;
pub const SEAT_SLOTS: usize = 48;
pub const ROTATE_AT: u64 = 1024 * 1024;
#[cfg_attr(not(test), allow(dead_code))] // the reader (§3.3 parse rule): cysd itself only writes; the tests read
pub const FIELDS: [&str; 10] = ["t", "pid", "tid", "seq", "build", "seats", "ids", "loc", "msg", "trunc"];
const TOKEN: &[u8] = b"#CYSPANIC1 ";
const IDS_CAP: usize = 320;
/// Room kept in `ids=` for `+` and up to 10 digits.
const IDS_SUFFIX_ROOM: usize = 11;
/// ` trunc=` plus its 3-letter value, kept free while `msg` is written.
const TRUNC_ROOM: usize = 10;

static LOG: OnceLock<File> = OnceLock::new();
static SEQ: AtomicU32 = AtomicU32::new(0);
static SEATS: SeatSnapshot = SeatSnapshot::new();

// ---------------------------------------------------------------- seat snapshot (§3.5)

/// What the record says about seats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seats {
    /// Nothing was ever published: a known empty set (`seats=0 ids=-`).
    Never,
    /// In progress, changed during the read, or parked at saturation (`seats=? ids=?`).
    Unknown,
    /// A whole publication: the count and the smallest `held` ids, ascending.
    Known { count: u32, ids: [u64; SEAT_SLOTS], held: usize },
}

pub struct SeatSnapshot {
    gen: AtomicU64,
    count: AtomicU32,
    ids: [AtomicU64; SEAT_SLOTS],
}

impl SeatSnapshot {
    pub const fn new() -> Self {
        SeatSnapshot { gen: AtomicU64::new(0), count: AtomicU32::new(0), ids: [const { AtomicU64::new(0) }; SEAT_SLOTS] }
    }

    /// One publication. Callers serialize (the `surfaces` mutex in production). Never panics, never allocates.
    pub fn publish(&self, keys: impl Iterator<Item = u64>, n: usize) {
        let g = self.gen.load(SeqCst);
        let start = match g.checked_add(1) {
            Some(v) => v | 1,
            None => return self.gen.store(u64::MAX, SeqCst),
        };
        let Some(end) = start.checked_add(1) else { return self.gen.store(u64::MAX, SeqCst) };
        self.gen.store(start, SeqCst);
        let mut kept = [0u64; SEAT_SLOTS];
        let mut held = 0usize;
        for key in keys {
            keep_smallest(&mut kept, &mut held, key);
        }
        self.count.store(u32::try_from(n).unwrap_or(u32::MAX), SeqCst);
        for (slot, value) in self.ids.iter().zip(kept.iter()) {
            slot.store(*value, SeqCst);
        }
        self.gen.store(end, SeqCst);
    }

    /// Single pass, no spin, no retry.
    pub fn read(&self) -> Seats {
        let g1 = self.gen.load(SeqCst);
        if g1 == 0 {
            return Seats::Never;
        }
        if g1 & 1 == 1 {
            return Seats::Unknown;
        }
        let count = self.count.load(SeqCst);
        let held = usize::try_from(count).unwrap_or(SEAT_SLOTS).min(SEAT_SLOTS);
        let mut ids = [0u64; SEAT_SLOTS];
        for (dst, src) in ids.iter_mut().zip(self.ids.iter()).take(held) {
            *dst = src.load(SeqCst);
        }
        if self.gen.load(SeqCst) != g1 {
            return Seats::Unknown;
        }
        Seats::Known { count, ids, held }
    }

    #[cfg(test)]
    pub fn force_gen(&self, g: u64) {
        self.gen.store(g, SeqCst);
    }

    #[cfg(test)]
    pub fn gen(&self) -> u64 {
        self.gen.load(SeqCst)
    }
}

/// `kept[..held]` stays sorted ascending and holds the smallest keys seen so far (at most `SEAT_SLOTS`).
fn keep_smallest(kept: &mut [u64; SEAT_SLOTS], held: &mut usize, key: u64) {
    let mut i = *held;
    if i == SEAT_SLOTS {
        if key >= kept[SEAT_SLOTS - 1] {
            return;
        }
        i = SEAT_SLOTS - 1; // the largest kept id is dropped
    } else {
        *held += 1;
    }
    while i > 0 && kept[i - 1] > key {
        kept[i] = kept[i - 1];
        i -= 1;
    }
    kept[i] = key;
}

/// Called inside the existing `surfaces` critical section after each production mutation of the map
/// (census-pinned: `panic_log_tests::census_every_surfaces_mutation_publishes`).
pub fn publish_seats<V>(map: &HashMap<u64, V>) {
    SEATS.publish(map.keys().copied(), map.len());
}

// ---------------------------------------------------------------- record budget (§3.9)

/// The next `seq` (1..=budget), or `None` once the budget is spent. Saturates: never wraps back into the window.
pub fn take_seq(counter: &AtomicU32) -> Option<u32> {
    counter.fetch_update(SeqCst, SeqCst, |v| if v < RECORD_BUDGET { Some(v + 1) } else { None }).ok().map(|v| v + 1)
}

// ---------------------------------------------------------------- formatting (§3.3)

pub struct Fields<'a> {
    /// Milliseconds since the Unix epoch; `None` gives `t=?`.
    pub unix_ms: Option<u64>,
    pub pid: u32,
    pub tid: Option<u64>,
    pub seq: u32,
    pub build: [&'a str; 3],
    pub seats: Seats,
    pub loc: Option<(&'a str, u32, u32)>,
    pub msg: Option<&'a [u8]>,
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn raw_ok(b: u8) -> bool {
    (0x21..=0x7E).contains(&b) && b != b'%' && b != b'#'
}

fn esc_width(b: u8) -> usize {
    if raw_ok(b) { 1 } else { 3 }
}

struct Cur<'b> {
    buf: &'b mut [u8],
    len: usize,
}

impl Cur<'_> {
    fn room(&self) -> usize {
        self.buf.len().saturating_sub(self.len)
    }
    fn raw(&mut self, s: &[u8]) {
        for &b in s {
            if let Some(slot) = self.buf.get_mut(self.len) {
                *slot = b;
                self.len += 1;
            }
        }
    }
    fn esc_byte(&mut self, b: u8) {
        if raw_ok(b) {
            self.raw(&[b]);
        } else {
            self.raw(&[b'%', HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]]);
        }
    }
    /// Escaped bytes of `src` while they fit in `cap`; true if something was cut.
    fn esc(&mut self, src: &[u8], cap: usize) -> bool {
        let mut used = 0usize;
        for &b in src {
            let w = esc_width(b);
            if used + w > cap || w > self.room() {
                return true;
            }
            self.esc_byte(b);
            used += w;
        }
        false
    }
    fn num(&mut self, mut v: u64) {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        self.raw(digits.get(i..).unwrap_or(&[]));
    }
}

fn num_width(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 10 {
        v /= 10;
        n += 1;
    }
    n
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` (days-from-civil inverse, Howard Hinnant). `None` past year 9999.
fn utc(unix_ms: u64) -> Option<[u8; 24]> {
    let secs = unix_ms / 1000;
    if secs >= 253_402_300_800 {
        return None;
    }
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe as i64 + era * 400 + i64::from(m <= 2);
    let mut out = *b"0000-00-00T00:00:00.000Z";
    let put = |out: &mut [u8; 24], at: usize, width: usize, mut v: u64| {
        for k in (0..width).rev() {
            out[at + k] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    };
    put(&mut out, 0, 4, y as u64);
    put(&mut out, 5, 2, m);
    put(&mut out, 8, 2, d);
    put(&mut out, 11, 2, rem / 3600);
    put(&mut out, 14, 2, rem / 60 % 60);
    put(&mut out, 17, 2, rem % 60);
    put(&mut out, 20, 3, unix_ms % 1000);
    Some(out)
}

/// FNV-1a 32-bit.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Formats one whole record into `buf`; returns its length (≤ `MAX_RECORD`). Never panics, never allocates.
pub fn format_record(buf: &mut [u8; MAX_RECORD], f: &Fields<'_>) -> usize {
    let (head, rest) = buf.split_at_mut(HEADER_LEN);
    let Some(body_area) = rest.get_mut(..MAX_BODY) else { return 0 };
    let mut c = Cur { buf: body_area, len: 0 };
    let mut cut = [0u8; 3];
    let mut ncut = 0usize;

    c.raw(b"t=");
    match f.unix_ms.and_then(utc) {
        Some(t) => c.raw(&t),
        None => c.raw(b"?"),
    }
    c.raw(b" pid=");
    c.num(u64::from(f.pid));
    c.raw(b" tid=");
    match f.tid {
        Some(t) => c.num(t),
        None => c.raw(b"?"),
    }
    c.raw(b" seq=");
    c.num(u64::from(f.seq));
    c.raw(b" build=");
    let mut build_room = 64usize;
    let mut build_cut = false;
    for part in f.build {
        let before = c.len;
        build_cut |= c.esc(part.as_bytes(), build_room);
        build_room = build_room.saturating_sub(c.len - before);
    }
    if build_cut {
        cut[ncut] = b'b';
        ncut += 1;
    }
    c.raw(b" seats=");
    match f.seats {
        Seats::Never => c.raw(b"0 ids=-"),
        Seats::Unknown => c.raw(b"? ids=?"),
        Seats::Known { count, ids, held } => {
            c.num(u64::from(count));
            c.raw(b" ids=");
            if count == 0 {
                c.raw(b"-");
            } else {
                let mut used = 0usize;
                let mut listed = 0u64;
                for id in ids.iter().take(held) {
                    let w = num_width(*id) + usize::from(listed > 0);
                    if used + w > IDS_CAP - IDS_SUFFIX_ROOM {
                        break;
                    }
                    if listed > 0 {
                        c.raw(b",");
                    }
                    c.num(*id);
                    used += w;
                    listed += 1;
                }
                let more = u64::from(count).saturating_sub(listed);
                if more > 0 {
                    c.raw(b"+");
                    c.num(more);
                }
            }
        }
    }
    c.raw(b" loc=");
    match f.loc {
        None => c.raw(b"?"),
        Some((file, line, col)) => {
            let suffix_w = 2 + num_width(u64::from(line)) + num_width(u64::from(col));
            let file = file.as_bytes();
            let file_w: usize = file.iter().map(|b| esc_width(*b)).sum();
            let file_cap = 300usize.saturating_sub(suffix_w);
            let mut start = 0usize;
            if file_w > file_cap {
                cut[ncut] = b'l';
                ncut += 1;
                // Keep the tail: walk back while the escaped tail fits.
                let (mut i, mut w) = (file.len(), 0usize);
                while i > 0 && w + esc_width(file[i - 1]) <= file_cap {
                    w += esc_width(file[i - 1]);
                    i -= 1;
                }
                start = i;
            }
            c.esc(file.get(start..).unwrap_or(&[]), file_cap);
            c.raw(b":");
            c.num(u64::from(line));
            c.raw(b":");
            c.num(u64::from(col));
        }
    }
    c.raw(b" msg=");
    match f.msg {
        None => c.raw(b"?"),
        Some(m) => {
            let cap = c.room().saturating_sub(TRUNC_ROOM);
            if c.esc(m, cap) {
                cut[ncut] = b'm';
                ncut += 1;
            }
        }
    }
    c.raw(b" trunc=");
    if ncut == 0 {
        c.raw(b"-");
    } else {
        c.raw(cut.get(..ncut).unwrap_or(&[]));
    }
    let body_len = c.len;

    let mut h = Cur { buf: head, len: 0 };
    h.raw(TOKEN);
    h.raw(b"len=");
    let mut digits = [b'0'; 4];
    let mut v = body_len;
    for d in digits.iter_mut().rev() {
        *d = b'0' + (v % 10) as u8;
        v /= 10;
    }
    h.raw(&digits);
    h.raw(b" chk=");
    let sum = fnv1a32(rest.get(..body_len).unwrap_or(&[]));
    for k in (0..8).rev() {
        h.raw(&[b"0123456789abcdef"[((sum >> (k * 4)) & 15) as usize]]);
    }
    h.raw(b" ");
    if let Some(nl) = rest.get_mut(body_len) {
        *nl = b'\n';
    }
    HEADER_LEN + body_len + 1
}

// ---------------------------------------------------------------- the hook (§3.6)

fn now_ms() -> Option<u64> {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn current_tid() -> Option<u64> {
    #[cfg(windows)]
    {
        Some(u64::from(unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }))
    }
    #[cfg(target_os = "macos")]
    {
        let mut tid = 0u64;
        // pthread_self() names this thread explicitly (pthread_t is uintptr_t in libc 0.2.186, so a literal 0 also
        // compiles and Apple reads it as "the calling thread"; the explicit handle does not lean on that convention).
        let rc = unsafe { libc::pthread_threadid_np(libc::pthread_self(), &mut tid) };
        (rc == 0).then_some(tid)
    }
    #[cfg(target_os = "linux")]
    {
        let tid = unsafe { libc::syscall(libc::SYS_gettid) };
        u64::try_from(tid).ok()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Builds and emits one record for a panic at `loc` with `msg`, through ONE `write` call on `sink`.
/// Returns false when the budget is spent (nothing written). The write result is ignored.
pub fn emit<W: Write>(sink: &mut W, seq: &AtomicU32, seats: &SeatSnapshot, loc: Option<(&str, u32, u32)>, msg: Option<&[u8]>) -> bool {
    let Some(n) = take_seq(seq) else { return false };
    let fields = Fields {
        unix_ms: now_ms(),
        pid: std::process::id(),
        tid: current_tid(),
        seq: n,
        build: [env!("CARGO_PKG_VERSION"), "+", cys::pack::build_id()],
        seats: seats.read(),
        loc,
        msg,
    };
    let mut buf = [0u8; MAX_RECORD];
    let len = format_record(&mut buf, &fields);
    if let Some(record) = buf.get(..len) {
        let _ = sink.write(record);
    }
    true
}

/// The panic hook body. Writes nothing if the log could not be opened at start (chain-only mode).
pub fn record(info: &std::panic::PanicHookInfo<'_>) {
    let Some(file) = LOG.get() else { return };
    let loc = info.location().map(|l| (l.file(), l.line(), l.column()));
    let mut sink = file;
    emit(&mut sink, &SEQ, &SEATS, loc, info.payload_as_str().map(str::as_bytes));
}

/// Start-time preparation (§3.2): mkdir, rotate over 1 MiB, open for append. Never called from the hook.
pub fn prepare(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(LOG_NAME);
    if std::fs::metadata(&path).map(|m| m.len() > ROTATE_AT).unwrap_or(false) {
        std::fs::rename(&path, dir.join(LOG_NAME_OLD))?;
    }
    OpenOptions::new().create(true).append(true).open(&path)
}

/// Installs the hook in front of the current one (std's default), after the single-instance gate.
pub fn install(socket_path: &Path) {
    install_in(&crate::state::state_dir(socket_path));
}

pub fn install_in(dir: &Path) {
    match prepare(dir) {
        Ok(file) => {
            let _ = LOG.set(file);
        }
        Err(e) => eprintln!("[cysd] panic-log unavailable: {}: {e}", dir.join(LOG_NAME).display()),
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record(info);
        prev(info);
    }));
}

// ---------------------------------------------------------------- the reader (§3.3 parse rule)

#[cfg_attr(not(test), allow(dead_code))] // the reader (§3.3 parse rule): cysd itself only writes; the tests read
#[derive(Debug)]
pub struct ParsedRecord {
    pub offset: usize,
    pub fields: Vec<(String, Vec<u8>)>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl ParsedRecord {
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Default)]
pub struct Report {
    pub accepted: Vec<ParsedRecord>,
    pub torn: Vec<(usize, &'static str)>,
    pub foreign: usize,
}

#[cfg_attr(not(test), allow(dead_code))]
fn unescape(v: &[u8]) -> Option<Vec<u8>> {
    let hex = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        let b = v[i];
        if b == b'%' {
            let hi = hex(*v.get(i + 1)?)?;
            let lo = hex(*v.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else if raw_ok(b) {
            out.push(b);
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

#[cfg_attr(not(test), allow(dead_code))]
fn check(bytes: &[u8], p: usize) -> Result<(ParsedRecord, usize), &'static str> {
    let head = bytes.get(p..p + HEADER_LEN).ok_or("short")?;
    let digits = |s: &[u8], hexa: bool| s.iter().all(|b| b.is_ascii_digit() || (hexa && (b'a'..=b'f').contains(b)));
    if !head.starts_with(TOKEN)
        || &head[11..15] != b"len="
        || !digits(&head[15..19], false)
        || &head[19..24] != b" chk="
        || !digits(&head[24..32], true)
        || head[32] != b' '
    {
        return Err("header");
    }
    let len: usize = std::str::from_utf8(&head[15..19]).ok().and_then(|s| s.parse().ok()).ok_or("header")?;
    if len > MAX_BODY {
        return Err("length");
    }
    let end = p + HEADER_LEN + len;
    let nl = *bytes.get(end).ok_or("short")?;
    if nl != b'\n' {
        return Err("newline");
    }
    let body = &bytes[p + HEADER_LEN..end];
    if body.iter().any(|b| *b == b'#' || *b == b'\n') {
        return Err("body");
    }
    let want = u32::from_str_radix(std::str::from_utf8(&head[24..32]).map_err(|_| "header")?, 16).map_err(|_| "header")?;
    if fnv1a32(body) != want {
        return Err("checksum");
    }
    let parts: Vec<&[u8]> = body.split(|b| *b == b' ').collect();
    if parts.len() != FIELDS.len() {
        return Err("fields");
    }
    let mut fields = Vec::with_capacity(FIELDS.len());
    for (part, key) in parts.iter().zip(FIELDS) {
        let value = part.strip_prefix(key.as_bytes()).and_then(|r| r.strip_prefix(b"=")).ok_or("fields")?;
        fields.push((key.to_string(), unescape(value).ok_or("fields")?));
    }
    Ok((ParsedRecord { offset: p, fields }, end + 1))
}

/// The reader contract: scan for `#`, validate, accept or report torn, re-synchronise on the next `#`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse(bytes: &[u8]) -> Report {
    let mut report = Report::default();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'#' {
            report.foreign += 1;
            i += 1;
            continue;
        }
        match check(bytes, i) {
            Ok((rec, next)) => {
                report.accepted.push(rec);
                i = next;
            }
            Err(reason) => {
                report.torn.push((i, reason));
                // The torn record runs to the next `#` (the resynchronisation point); its bytes are not foreign.
                i += 1;
                while i < bytes.len() && bytes[i] != b'#' {
                    i += 1;
                }
            }
        }
    }
    report
}
