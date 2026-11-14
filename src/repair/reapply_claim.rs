//! RULINGS CT8 / DR4 — ONE re-application per parked edit, across lanes, by the grant's claim
//! protocol (RULINGS G1, G2, G7, K5), not a variant of it. The claim is keyed by the ORIGINAL repair
//! in the shared ledger, made by complete creation + no-replace placement (never a rename over
//! anything): `<root>/reapply-claims/<R>/<k>.json` NAMES ITS HOLDER R′; `<k>.closed.json` closes it.
//! 1. The claimant binds its record FIRST: R′ already exists (`begin`) with `reapplies: R`, under
//!    R′'s own lock, before any claim is placed.
//! 2. It places claim k (the next number) naming R′; a lost placement → it stops.
//! 3. AFTER placement it checks that its own claim has not been closed — an expired or overtaken
//!    claimant that resumes finds the marker and stops (the caller then closes its R′).
//! A closer of claim k creates the marker FIRST and only then reads the holder again (its second
//! read); unreadable holder evidence means "assume held". No cross-lane lock; nothing is ever
//! deleted or rewritten.

use super::types::{LaneKey, RepairError, RepairId};
use std::path::{Path, PathBuf};

pub fn claims_dir_in(root: &Path, original: &RepairId) -> PathBuf {
    root.join("reapply-claims").join(original.as_str())
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Claim {
    pub schema: u32,
    pub original: RepairId,
    pub k: u32,
    pub lane: LaneKey,
    pub holder: RepairId,
    pub at: f64,
}

/// What the claimant's caller knows of a holder R′.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// Unfinished or in force — the claim stands.
    Alive,
    /// Ended (closed, reverted, superseded …) — the claim may be closed.
    Ended,
    /// Its record cannot be read — "assume held" (G7).
    Unreadable,
}

/// Why a claim was not made.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimRefused {
    /// A live (or unreadable — assumed held) R′ holds it.
    Held { holder: RepairId },
    /// Another lane placed the next claim first (the placement was lost).
    LostRace,
    /// Our claim was closed before our post-placement check (we were overtaken while paused).
    ClosedAfterPlacement { k: u32 },
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

fn publish_json<T: serde::Serialize>(p: &Path, v: &T) -> Result<(), RepairError> {
    let mut bytes = serde_json::to_vec_pretty(v).map_err(|e| RepairError::Io(e.to_string()))?;
    bytes.push(b'\n');
    super::vault::publish_new(p, &bytes, 0o644)
}

/// Claim `k` of `original` (`None` = absent or unreadable).
pub fn read_claim_in(root: &Path, original: &RepairId, k: u32) -> Option<Claim> {
    serde_json::from_slice(&std::fs::read(claims_dir_in(root, original).join(format!("{k}.json"))).ok()?).ok()
}

pub fn is_closed(root: &Path, original: &RepairId, k: u32) -> bool {
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

/// Steps 2–3 of the protocol for the claimant whose R′ (`holder`) is already bound. `holder_of(R)`
/// classifies any holder (read at the moment of the call).
pub fn claim_in(
    root: &Path,
    original: &RepairId,
    lane: &LaneKey,
    holder: &RepairId,
    now: f64,
    holder_of: &dyn Fn(&RepairId) -> Holder,
) -> Result<u32, ClaimRefused> {
    let io = |e: RepairError| ClaimRefused::Io(format!("{e:?}"));
    let dir = claims_dir_in(root, original);
    std::fs::create_dir_all(&dir).map_err(|e| ClaimRefused::Io(format!("{}: {e}", dir.display())))?;
    let top = numbers(&dir).last().copied();
    if let Some(k) = top {
        if !is_closed(root, original, k) {
            // an unreadable claim is evidence that cannot be read: assume held
            let Some(c) = read_claim_in(root, original, k) else {
                return Err(ClaimRefused::Held { holder: holder.clone() });
            };
            match holder_of(&c.holder) {
                Holder::Alive | Holder::Unreadable => return Err(ClaimRefused::Held { holder: c.holder }),
                Holder::Ended => {
                    // the closer creates the marker FIRST, then reads the holder again
                    close_in(root, original, k, "다시 입힌 수리가 끝남", now).map_err(io)?;
                    if holder_of(&c.holder) != Holder::Ended {
                        return Err(ClaimRefused::Held { holder: c.holder });
                    }
                }
            }
        }
    }
    let k = top.unwrap_or(0) + 1;
    let claim = Claim { schema: super::REPAIR_SCHEMA, original: original.clone(), k, lane: lane.clone(), holder: holder.clone(), at: now };
    match publish_json(&dir.join(format!("{k}.json")), &claim) {
        Ok(()) => {}
        Err(RepairError::Superseded(_)) => return Err(ClaimRefused::LostRace),
        Err(e) => return Err(io(e)),
    }
    // AFTER placement: our own claim must not have been closed meanwhile
    if is_closed(root, original, k) {
        return Err(ClaimRefused::ClosedAfterPlacement { k });
    }
    Ok(k)
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

    fn rid(s: &str) -> RepairId {
        RepairId::parse(s).unwrap()
    }

    /// Lanes racing for the same original at one instant: exactly one claim; a live or unreadable
    /// holder keeps it; an ended holder's claim is closed (marker first) by the next claimant.
    #[test]
    fn one_claim_per_original_across_lanes() {
        let r = root("race");
        let orig = rid("R-3");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let wins: Vec<_> = (0..8)
            .map(|i| {
                let (r, orig, barrier) = (r.clone(), orig.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let lane = LaneKey(if i % 2 == 0 { "base".into() } else { "dept-2".into() });
                    barrier.wait();
                    claim_in(&r, &orig, &lane, &rid(&format!("R-{}", 10 + i)), 100.0, &|_| Holder::Alive).is_ok()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(wins.iter().filter(|w| **w).count(), 1, "exactly one claim: {wins:?}");
        let b = LaneKey("dept-2".into());
        let holder = read_claim_in(&r, &orig, 1).unwrap().holder;
        assert_eq!(claim_in(&r, &orig, &b, &rid("R-30"), 101.0, &|_| Holder::Alive), Err(ClaimRefused::Held { holder: holder.clone() }));
        assert_eq!(claim_in(&r, &orig, &b, &rid("R-30"), 102.0, &|_| Holder::Unreadable), Err(ClaimRefused::Held { holder: holder.clone() }), "unreadable = assume held");
        assert_eq!(claim_in(&r, &orig, &b, &rid("R-30"), 103.0, &|_| Holder::Ended), Ok(2));
        assert!(is_closed(&r, &orig, 1));
        let _ = std::fs::remove_dir_all(&r);
    }

    /// DR4 — the verdict's delayed-claimant interleaving: A binds R′A and pauses (600 s) before
    /// placing its claim; B binds R′B and places claim 1; A resumes: its placement of claim 1 is lost
    /// → A stops (its caller closes R′A). And a claimant overtaken AFTER its placement (its claim
    /// closed while it was paused) finds the marker in its post-placement check and stops.
    #[test]
    fn a_delayed_claimant_is_fenced() {
        let r = root("delay");
        let orig = rid("R-4");
        let a = LaneKey("base".into());
        let b = LaneKey("dept-2".into());
        // B places claim 1 while A is paused between its `begin` and its placement
        assert_eq!(claim_in(&r, &orig, &b, &rid("R-21"), 100.0, &|_| Holder::Alive), Ok(1));
        // A resumes 600 s later and tries to place claim 1 itself: the placement is lost
        let dir = claims_dir_in(&r, &orig);
        let lost = publish_json(&dir.join("1.json"), &Claim { schema: 1, original: orig.clone(), k: 1, lane: a.clone(), holder: rid("R-20"), at: 700.0 });
        assert!(matches!(lost, Err(RepairError::Superseded(_))), "A's placement of claim 1 is refused");
        // and through the protocol A is refused while R′B lives
        assert_eq!(claim_in(&r, &orig, &a, &rid("R-20"), 700.0, &|_| Holder::Alive), Err(ClaimRefused::Held { holder: rid("R-21") }));
        // post-placement fence: a claim closed between placement and the check is detected
        close_in(&r, &orig, 2, "taken over", 800.0).unwrap();
        assert_eq!(claim_in(&r, &orig, &a, &rid("R-20"), 800.0, &|h| if *h == rid("R-21") { Holder::Ended } else { Holder::Alive }), Err(ClaimRefused::ClosedAfterPlacement { k: 2 }));
        let _ = std::fs::remove_dir_all(&r);
    }
}
