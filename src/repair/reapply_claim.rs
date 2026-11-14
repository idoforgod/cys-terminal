//! RULINGS CT8 — ONE re-application per parked edit, across lanes. AR1's uniqueness is a durable
//! exclusive claim keyed by the ORIGINAL repair in the shared ledger, made the way the grant's claims
//! are (complete creation + no-replace placement — never a rename over anything):
//! `<root>/reapply-claims/<R>/<k>.json` (the claim, `k` = 1, 2, …), `<k>.holder.json` (the R′ that
//! holds it, written once after `begin`), `<k>.closed.json` (closed — the claim no longer blocks).
//! There is no cross-lane lock: two lanes racing for `k` meet in the no-replace placement and exactly
//! one wins. A claim whose holder ended, or that never got a holder (a crash between the claim and
//! `begin`) and is older than [`CLAIM_STARTING_SECS`], is closed by the next claimant — closable
//! after a crash. Nothing here is ever deleted or rewritten.

use super::types::{LaneKey, RepairError, RepairId};
use std::path::{Path, PathBuf};

/// How long a claim without a holder blocks (the claimant is between its claim and `begin`, which
/// takes milliseconds; after this it is taken for a crash and closed).
pub const CLAIM_STARTING_SECS: f64 = 600.0;

pub fn claims_dir_in(root: &Path, original: &RepairId) -> PathBuf {
    root.join("reapply-claims").join(original.as_str())
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Claim {
    pub schema: u32,
    pub original: RepairId,
    pub k: u32,
    pub lane: LaneKey,
    pub at: f64,
}

/// Why a claim was not made.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimRefused {
    /// An unfinished or in-force R′ holds it.
    Held { holder: RepairId },
    /// Another lane claimed it a moment ago and is starting its R′.
    Starting { lane: LaneKey },
    /// Another lane placed the next claim first (the race was lost).
    LostRace,
    Io(String),
}

fn numbers(dir: &Path) -> Vec<u32> {
    let mut v: Vec<u32> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_suffix(".json")?.parse::<u32>().ok()
        })
        .collect();
    v.sort_unstable();
    v
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn publish_json<T: serde::Serialize>(p: &Path, v: &T) -> Result<(), RepairError> {
    let mut bytes = serde_json::to_vec_pretty(v).map_err(|e| RepairError::Io(e.to_string()))?;
    bytes.push(b'\n');
    super::vault::publish_new(p, &bytes, 0o644)
}

/// The holder recorded for claim `k` of `original`.
pub fn holder_in(root: &Path, original: &RepairId, k: u32) -> Option<RepairId> {
    let v: serde_json::Value = read_json(&claims_dir_in(root, original).join(format!("{k}.holder.json")))?;
    RepairId::parse(v.get("holder")?.as_str()?).ok()
}

fn is_closed(root: &Path, original: &RepairId, k: u32) -> bool {
    claims_dir_in(root, original).join(format!("{k}.closed.json")).exists()
}

/// Closes claim `k` (idempotent: an existing close stands).
pub fn close_in(root: &Path, original: &RepairId, k: u32, why: &str, now: f64) -> Result<(), RepairError> {
    let p = claims_dir_in(root, original).join(format!("{k}.closed.json"));
    match publish_json(&p, &serde_json::json!({"schema": super::REPAIR_SCHEMA, "k": k, "why": why, "at": now})) {
        Ok(()) | Err(RepairError::Superseded(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Makes the next claim for `original` from `lane`; `holder_alive(R′)` = R′ is unfinished or in force.
pub fn claim_in(
    root: &Path,
    original: &RepairId,
    lane: &LaneKey,
    now: f64,
    holder_alive: &dyn Fn(&RepairId) -> bool,
) -> Result<u32, ClaimRefused> {
    let dir = claims_dir_in(root, original);
    std::fs::create_dir_all(&dir).map_err(|e| ClaimRefused::Io(format!("{}: {e}", dir.display())))?;
    let top = numbers(&dir).last().copied();
    if let Some(k) = top {
        if !is_closed(root, original, k) {
            match holder_in(root, original, k) {
                Some(h) if holder_alive(&h) => return Err(ClaimRefused::Held { holder: h }),
                Some(_) => close_in(root, original, k, "다시 입힌 수리가 끝남", now).map_err(|e| ClaimRefused::Io(format!("{e:?}")))?,
                None => {
                    let c: Option<Claim> = read_json(&dir.join(format!("{k}.json")));
                    match c {
                        Some(c) if now - c.at < CLAIM_STARTING_SECS => return Err(ClaimRefused::Starting { lane: c.lane }),
                        _ => close_in(root, original, k, "수리 번호를 받기 전에 끊긴 주장", now).map_err(|e| ClaimRefused::Io(format!("{e:?}")))?,
                    }
                }
            }
        }
    }
    let k = top.unwrap_or(0) + 1;
    let claim = Claim { schema: super::REPAIR_SCHEMA, original: original.clone(), k, lane: lane.clone(), at: now };
    match publish_json(&dir.join(format!("{k}.json")), &claim) {
        Ok(()) => Ok(k),
        Err(RepairError::Superseded(_)) => Err(ClaimRefused::LostRace),
        Err(e) => Err(ClaimRefused::Io(format!("{e:?}"))),
    }
}

/// Records the R′ that holds claim `k` (once).
pub fn set_holder_in(root: &Path, original: &RepairId, k: u32, holder: &RepairId) -> Result<(), RepairError> {
    publish_json(
        &claims_dir_in(root, original).join(format!("{k}.holder.json")),
        &serde_json::json!({"schema": super::REPAIR_SCHEMA, "k": k, "holder": holder.as_str()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cys-reapply-claim-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Two lanes race for the same original: exactly one claim wins; the loser is told; the winner's
    /// R′ holds it while alive; an ended holder or a stale holderless claim is closed by the next
    /// claimant (closable after a crash); nothing is ever overwritten.
    #[test]
    fn one_claim_per_original_across_lanes() {
        let r = root("race");
        let orig = RepairId::parse("R-3").unwrap();
        let a = LaneKey("base".into());
        let b = LaneKey("dept-2".into());
        let alive = |_: &RepairId| true;
        let dead = |_: &RepairId| false;
        // concurrent: 8 threads, 4 per lane, at the same instant
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let wins: Vec<_> = (0..8)
            .map(|i| {
                let (r, orig, lane, barrier) = (r.clone(), orig.clone(), if i % 2 == 0 { a.clone() } else { b.clone() }, barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_in(&r, &orig, &lane, 100.0, &|_| true).is_ok()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(wins.iter().filter(|w| **w).count(), 1, "exactly one claim: {wins:?}");
        // the winner is starting (no holder yet) → the others are refused
        assert!(matches!(claim_in(&r, &orig, &b, 101.0, &alive), Err(ClaimRefused::Starting { .. })));
        set_holder_in(&r, &orig, 1, &RepairId::parse("R-9").unwrap()).unwrap();
        assert_eq!(claim_in(&r, &orig, &b, 102.0, &alive), Err(ClaimRefused::Held { holder: RepairId::parse("R-9").unwrap() }));
        // the holder ended → closed by the next claimant, who gets claim 2
        assert_eq!(claim_in(&r, &orig, &b, 103.0, &dead), Ok(2));
        assert!(is_closed(&r, &orig, 1));
        // a holderless claim older than the starting window (a crash before `begin`) is closed too
        assert_eq!(claim_in(&r, &orig, &a, 103.0 + CLAIM_STARTING_SECS + 1.0, &alive), Ok(3));
        assert!(is_closed(&r, &orig, 2));
        let _ = std::fs::remove_dir_all(&r);
    }
}
