//! W60 D2 — the cysd panic record. ★c1 STUB (commit H-T): the public API of DESIGN-W60-r2 §3 with no-op bodies,
//! so `panic_log_tests` compiles and goes RED on behaviour. Commit H (c2) replaces this whole file with the real
//! module; nothing in production calls it at this commit (no hook installed, no seat publication).
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicU32;

pub const LOG_NAME: &str = "cysd-panic.log";
pub const LOG_NAME_OLD: &str = "cysd-panic.log.1";
pub const MAX_RECORD: usize = 1024;
pub const HEADER_LEN: usize = 33;
pub const MAX_BODY: usize = MAX_RECORD - HEADER_LEN - 1;
pub const RECORD_BUDGET: u32 = 64;
pub const SEAT_SLOTS: usize = 48;
pub const ROTATE_AT: u64 = 1024 * 1024;
pub const FIELDS: [&str; 10] = ["t", "pid", "tid", "seq", "build", "seats", "ids", "loc", "msg", "trunc"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seats {
    Never,
    Unknown,
    Known { count: u32, ids: [u64; SEAT_SLOTS], held: usize },
}

pub struct SeatSnapshot;

impl SeatSnapshot {
    pub const fn new() -> Self {
        SeatSnapshot
    }

    pub fn publish(&self, _keys: impl Iterator<Item = u64>, _n: usize) {}

    pub fn read(&self) -> Seats {
        Seats::Never
    }

    #[cfg(test)]
    pub fn force_gen(&self, _g: u64) {}

    #[cfg(test)]
    pub fn gen(&self) -> u64 {
        0
    }
}

pub fn publish_seats<V>(_map: &HashMap<u64, V>) {}

pub fn take_seq(_counter: &AtomicU32) -> Option<u32> {
    None
}

pub struct Fields<'a> {
    pub unix_ms: Option<u64>,
    pub pid: u32,
    pub tid: Option<u64>,
    pub seq: u32,
    pub build: [&'a str; 3],
    pub seats: Seats,
    pub loc: Option<(&'a str, u32, u32)>,
    pub msg: Option<&'a [u8]>,
}

pub fn fnv1a32(_bytes: &[u8]) -> u32 {
    0
}

pub fn format_record(_buf: &mut [u8; MAX_RECORD], _f: &Fields<'_>) -> usize {
    0
}

pub fn emit<W: Write>(_sink: &mut W, _seq: &AtomicU32, _seats: &SeatSnapshot, _loc: Option<(&str, u32, u32)>, _msg: Option<&[u8]>) -> bool {
    false
}

pub fn record(_info: &std::panic::PanicHookInfo<'_>) {}

pub fn prepare(_dir: &Path) -> std::io::Result<File> {
    Err(std::io::Error::other("W60 c1 stub: prepare is not implemented"))
}

pub fn install(_socket_path: &Path) {}

pub fn install_in(_dir: &Path) {}

#[derive(Debug)]
pub struct ParsedRecord {
    pub offset: usize,
    pub fields: Vec<(String, Vec<u8>)>,
}

impl ParsedRecord {
    pub fn get(&self, _key: &str) -> Option<&[u8]> {
        None
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub accepted: Vec<ParsedRecord>,
    pub torn: Vec<(usize, &'static str)>,
    pub foreign: usize,
}

pub fn parse(_bytes: &[u8]) -> Report {
    Report::default()
}
