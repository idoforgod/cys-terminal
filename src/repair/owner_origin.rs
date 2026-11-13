//! The owner-origin judgment shared by M5 (the autonomous-repair grant, C9) and M13 (the
//! owner-instruction token for rule documents, C10) — track T4.
//!
//! Inputs are the daemon's per-seat prompt record (T3 writes it in `hook.machine_origin`, T4 reads
//! it — RULINGS O-C9-2 (a) / O-C10-1 (a), Q1, Q2, T3, U4): the seat's latest prompt (digest,
//! origin, time), the open question of a master seat (asked_at, path, the first owner-typed answer,
//! whether a machine prompt came between) and the rule-document names of the latest owner-typed
//! prompt. Everything here is pure and FAIL-CLOSED: anything but an owner-typed (`human`) prompt —
//! `machine`, `unknown`, a missing record — is not the owner's (C10 10.1, design §4-13 "확실하지
//! 않으면 주인의 입력이 아닌 것으로"). codex / agy seats have no prompt hook, so no prompt of theirs is
//! ever recorded as the owner's (10.4 — the safe direction).
//!
//! Stated limit (design §3-2 criterion 6, §4-13, D:950): a process running as the same OS user can
//! write the delivery ledger, a token file or the decision file directly, or type into a pane through
//! the terminal itself; none of that is prevented here — the same class as every other approval
//! device of the product.

use super::types::{fold_name, LaneKey, RepairError, RepairId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One open question lapses 30 minutes after it was registered (Q1).
pub const QUESTION_LAPSE_SECS: u64 = 1_800;
/// An owner-instruction token is valid 24 hours (O-C10-2).
pub const TOKEN_TTL_SECS: u64 = 86_400;
/// The prefix of a plain token (O-C10-2: `ot-<base32 128 bit>`).
pub const TOKEN_PREFIX: &str = "ot-";
/// The longest prompt that can be an affirmative answer, in characters (RULINGS Q1: "a long
/// sentence" is not an answer — interpretation: a short reply only).
pub const AFFIRMATIVE_MAX_CHARS: usize = 40;

/// The daemon's fixed list of affirmative replies (RULINGS Q1: 예, 네, 응, 그래, 좋아, 허락, 켜, yes, y,
/// ok, … — polite and casual forms of the same words). Matched on the whole trimmed prompt or its
/// leading word (trailing punctuation removed), Latin letters case-insensitively.
pub const AFFIRMATIVE_WORDS: &[&str] = &[
    "예", "예예", "네", "네네", "넵", "응", "웅", "그래", "그래요", "그럼", "좋아", "좋아요", "좋습니다",
    "허락", "허락해", "허락합니다", "허락해요", "켜", "켜요", "켜줘", "켜세요", "yes", "y", "ok", "okay",
];

/// The prompt-hook verdict strings of the per-seat record (`hook.machine_origin`).
pub const ORIGIN_HUMAN: &str = "human";

/// A view of the daemon's `OpenQuestion` (the lib cannot see the daemon's types).
#[derive(Debug, Clone, Copy)]
pub struct QuestionView<'a> {
    pub asked_at: f64,
    /// `None` = the grant question; `Some(rel)` = a token question for that rule document.
    pub path: Option<&'a str>,
    /// The first owner-typed prompt after the question: (digest, excerpt, at).
    pub first_human: Option<(&'a str, &'a str, f64)>,
    pub machine_seen: bool,
}

/// A view of the daemon's `PromptOrigin` for one master seat.
#[derive(Debug, Clone, Copy)]
pub struct PromptView<'a> {
    /// The seat's LATEST prompt (any origin).
    pub digest: &'a str,
    pub origin: &'a str,
    pub at: f64,
    pub question: Option<QuestionView<'a>>,
    /// Rule-document base names (lower-cased) of the latest OWNER-TYPED prompt (U4(a)).
    pub named_docs: &'a [String],
    /// The latest OWNER-TYPED prompt (digest, time) — kept beside the latest prompt of any origin
    /// (RULINGS CH4), so a machine prompt arriving after the owner's words does not hide them.
    pub last_human: Option<(&'a str, f64)>,
}

/// The owner-typed prompt a decision rests on (kept as provenance: digest, short excerpt, time).
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub digest: String,
    pub excerpt: String,
    pub at: f64,
}

/// Q1's affirmative test: the whole trimmed prompt, or its leading word, is on the fixed list —
/// and the prompt is short (a long sentence is not an answer).
pub fn is_affirmative(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > AFFIRMATIVE_MAX_CHARS {
        return false;
    }
    let clean = |w: &str| -> String {
        w.trim_matches(|c: char| {
            c.is_ascii_punctuation() || matches!(c, '。' | '、' | '…' | '~' | '！' | '？' | '，' | '．')
        })
        .to_lowercase()
    };
    let whole = clean(t);
    let lead = clean(t.split_whitespace().next().unwrap_or(""));
    AFFIRMATIVE_WORDS.contains(&whole.as_str()) || AFFIRMATIVE_WORDS.contains(&lead.as_str())
}

fn lapsed(q: &QuestionView<'_>, now: f64) -> bool {
    now - q.asked_at > QUESTION_LAPSE_SECS as f64
}

/// The sentence of a `grant on` refused for want of the registered question (RULINGS T5).
pub const ASK_FIRST_KO: &str =
    "먼저 `cys repair grant ask`로 질문을 등록하세요 — 그 뒤 주인이 대화창에 직접 친 답만 셉니다";

/// RULINGS Q1 / T3 / T5 / U4(b): `grant on` — the open question is the GRANT question (`path` =
/// None), registered less than 30 minutes ago, the first owner-typed prompt after it exists, no
/// machine-origin prompt came between, and that answer is affirmative. `Err` = the reason (Korean,
/// naming the missing step).
pub fn judge_grant_on(p: Option<&PromptView<'_>>, now: f64) -> Result<Answer, String> {
    let Some(q) = p.and_then(|p| p.question) else {
        return Err(ASK_FIRST_KO.to_string());
    };
    if let Some(path) = q.path {
        return Err(format!(
            "열린 질문은 규칙 문서 표 질문입니다({path}) — 자동 수리 허락은 `cys repair grant ask`로 따로 등록하고 물으세요"
        ));
    }
    if lapsed(&q, now) {
        return Err(
            "질문을 등록한 지 30분이 지나 만료됐습니다 — `cys repair grant ask`로 다시 등록하고 물으세요".into(),
        );
    }
    if q.machine_seen {
        return Err("질문 뒤 주인의 답보다 먼저 기계가 넣은 글(또는 판정할 수 없는 글)이 들어왔습니다 — 확실하지 않으므로 세지 않습니다. `cys repair grant ask`로 다시 등록하고 물으세요".into());
    }
    let Some((digest, excerpt, at)) = q.first_human else {
        return Err(
            "질문 뒤에 주인이 직접 친 답이 아직 없습니다 — 주인의 답을 기다린 뒤 다시 하세요".into(),
        );
    };
    if !is_affirmative(excerpt) {
        return Err("주인의 답이 허락(예·네·응·좋아·허락·켜·yes·ok …)이 아닙니다 — 허락은 그대로입니다(다시 물으려면 `cys repair grant ask`)".into());
    }
    Ok(Answer {
        digest: digest.to_string(),
        excerpt: excerpt.to_string(),
        at,
    })
}

/// RULINGS Q1: `grant off` — the safe direction: accepted whenever an owner-typed prompt exists
/// after the last decision (`decided_at`; `None` = no decision yet): the seat's latest OWNER-TYPED
/// prompt (`last_human`, RULINGS CH4 — a machine prompt after it does not hide it), the latest
/// prompt when it is owner-typed, or the open question's owner-typed answer.
pub fn judge_grant_off(p: Option<&PromptView<'_>>, decided_at: Option<f64>) -> Result<Answer, String> {
    let after = |t: f64| decided_at.is_none_or(|d| t > d);
    if let Some(p) = p {
        if let Some((digest, at)) = p.last_human {
            if after(at) {
                return Ok(Answer {
                    digest: digest.to_string(),
                    excerpt: String::new(),
                    at,
                });
            }
        }
        if p.origin == ORIGIN_HUMAN && after(p.at) {
            return Ok(Answer {
                digest: p.digest.to_string(),
                excerpt: String::new(),
                at: p.at,
            });
        }
        if let Some((digest, excerpt, at)) = p.question.and_then(|q| q.first_human) {
            if after(at) {
                return Ok(Answer {
                    digest: digest.to_string(),
                    excerpt: excerpt.to_string(),
                    at,
                });
            }
        }
    }
    Err("마지막 결정 뒤에 주인이 직접 친 입력이 없습니다 — 주인이 대화창에 '끄라'고 한 바로 그 뒤에 하세요(다른 글이 먼저 들어왔다면 `cys repair grant ask` 뒤 주인의 답)".into())
}

/// How a token request was proven to be the owner's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenBasis {
    /// Q2 (a): the latest owner-typed prompt names the document.
    NamedInPrompt,
    /// Q2 (b): an affirmative owner-typed answer to `owner-token ask --path` for exactly this path.
    AnsweredQuestion,
}

/// The folded base name of a pack-relative path (the C10 classification compares base names after
/// full case folding — BJ3).
pub fn folded_base(rel: &str) -> String {
    fold_name(rel.rsplit(['/', '\\']).next().unwrap_or(rel))
}

/// Two pack-relative paths name the same file for the token's purposes: equal after folding
/// (case-insensitive filesystems treat them as one file; on a case-sensitive one the token covers
/// both spellings of the owner's document — never a different base name).
pub fn same_rel(a: &str, b: &str) -> bool {
    fold_name(&a.replace('\\', "/")) == fold_name(&b.replace('\\', "/"))
}

/// RULINGS Q2 / T3 / U4: `owner-token --path <doc>` — (a) the seat's latest owner-typed prompt
/// names the document (`named_docs` holds its folded base name), or (b) the open question is a
/// token question for EXACTLY this path, not lapsed, answered first by an owner-typed affirmative
/// prompt with no machine prompt between. The token is then scoped to exactly that path.
pub fn judge_owner_token(
    p: Option<&PromptView<'_>>,
    doc_rel: &str,
    now: f64,
) -> Result<(TokenBasis, Answer), String> {
    let base = folded_base(doc_rel);
    let Some(p) = p else {
        return Err(format!("이 좌석에는 주인이 직접 친 입력의 기록이 없습니다 — 주인이 '고쳐라 {base}'처럼 문서를 말하거나, `cys repair owner-token ask --path {doc_rel}` 뒤 주인이 직접 허락해야 합니다"));
    };
    if p.named_docs.iter().any(|n| fold_name(n) == base) {
        // `named_docs` and `last_human` are written together, from the same owner-typed prompt (CH4)
        let (digest, at) = match p.last_human {
            Some((d, at)) => (d.to_string(), at),
            None if p.origin == ORIGIN_HUMAN => (p.digest.to_string(), p.at),
            None => (String::new(), 0.0),
        };
        return Ok((
            TokenBasis::NamedInPrompt,
            Answer {
                digest,
                excerpt: format!("named: {base}"),
                at,
            },
        ));
    }
    if let Some(q) = p.question {
        match q.path {
            Some(qp) if same_rel(qp, doc_rel) => {
                if lapsed(&q, now) {
                    return Err(format!("질문을 등록한 지 30분이 지나 만료됐습니다 — `cys repair owner-token ask --path {doc_rel}`로 다시 등록하고 물으세요"));
                }
                if q.machine_seen {
                    return Err(format!("질문 뒤 주인의 답보다 먼저 기계가 넣은 글(또는 판정할 수 없는 글)이 들어왔습니다 — 확실하지 않으므로 세지 않습니다. `cys repair owner-token ask --path {doc_rel}`로 다시 물으세요"));
                }
                let Some((digest, excerpt, at)) = q.first_human else {
                    return Err("질문 뒤에 주인이 직접 친 답이 아직 없습니다 — 주인의 답을 기다린 뒤 다시 하세요".into());
                };
                if !is_affirmative(excerpt) {
                    return Err("주인의 답이 허락(예·네·응·좋아·허락 …)이 아닙니다 — 표를 만들지 않습니다".into());
                }
                return Ok((
                    TokenBasis::AnsweredQuestion,
                    Answer {
                        digest: digest.to_string(),
                        excerpt: excerpt.to_string(),
                        at,
                    },
                ));
            }
            Some(qp) => {
                return Err(format!("열린 질문은 다른 문서({qp})에 대한 것입니다 — 주인이 이 문서({base})를 말하지 않았으므로 표를 만들지 않습니다"));
            }
            None => {}
        }
    }
    Err(format!("주인이 직접 친 마지막 입력이 이 문서({base})를 이름으로 부르지 않았습니다 — 주인이 '고쳐라 {base}'처럼 문서를 말하거나, `cys repair owner-token ask --path {doc_rel}` 뒤 주인이 직접 허락해야 합니다"))
}

// ───────────────────────────── the token (O-C10-2 · F10 · F11) ─────────────────────────────

/// RFC 4648 base32, lower case, no padding.
fn base32_lower(bytes: &[u8]) -> String {
    const A: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(A[((buf >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(A[((buf << (5 - bits)) & 31) as usize]));
    }
    out
}

/// A plain token from 16 random bytes (128 bit): `ot-` + 26 base32 characters.
pub fn token_from_bytes(b: &[u8; 16]) -> String {
    format!("{TOKEN_PREFIX}{}", base32_lower(b))
}

/// Whether `s` has the shape of a plain token (nothing else is ever looked up).
pub fn is_token_shape(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX).is_some_and(|r| {
        r.len() == 26 && r.bytes().all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
    })
}

/// The ledger key of a token: sha256 of the plain token, lower-case hex.
pub fn token_sha256(token: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// `repairs/owner-tokens/<token_sha256>.json` (N-5 / F11) — written only by the issuing lane's daemon,
/// never an `R-*` record (F10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerToken {
    pub schema: u32,
    pub token_sha256: String,
    pub lane_key: LaneKey,
    pub issued_to_seat: u64,
    pub paths: Vec<String>,
    pub repair: Option<RepairId>,
    pub prompt_digest: String,
    pub prompt_excerpt: String,
    pub issued_at: f64,
    pub expires_at: f64,
    pub used_by_repair: Option<RepairId>,
}

/// `<root>/owner-tokens/<sha>.json` (= `paths::owner_token_path` under that root).
pub fn token_path_in(root: &Path, sha: &str) -> PathBuf {
    root.join("owner-tokens").join(format!("{sha}.json"))
}

fn valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Reads a token record: `Ok(None)` absent; `Err(Corrupt)` unreadable / higher schema / a record
/// that names another hash.
pub fn read_token_in(root: &Path, sha: &str) -> Result<Option<OwnerToken>, RepairError> {
    if !valid_sha(sha) {
        return Ok(None);
    }
    let p = token_path_in(root, sha);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepairError::Corrupt(format!("{}: {e}", p.display()))),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    let schema = v.get("schema").and_then(|s| s.as_u64()).unwrap_or(u64::MAX);
    if schema > u64::from(super::REPAIR_SCHEMA) {
        return Err(RepairError::Corrupt(format!("{}: unknown schema {schema}", p.display())));
    }
    let t: OwnerToken = serde_json::from_value(v)
        .map_err(|e| RepairError::Corrupt(format!("{}: {e}", p.display())))?;
    if t.token_sha256 != sha {
        return Err(RepairError::Corrupt(format!("{}: names another token", p.display())));
    }
    Ok(Some(t))
}

/// Creates a NEW token record (create-new: an existing file is never replaced at issuance).
pub fn create_token_in(root: &Path, t: &OwnerToken) -> Result<(), RepairError> {
    use std::io::Write;
    let p = token_path_in(root, &t.token_sha256);
    let dir = root.join("owner-tokens");
    std::fs::create_dir_all(&dir).map_err(|e| RepairError::Io(format!("{}: {e}", dir.display())))?;
    let mut t = t.clone();
    t.schema = super::REPAIR_SCHEMA;
    let bytes = serde_json::to_vec_pretty(&t).map_err(|e| RepairError::Io(e.to_string()))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))?;
    f.write_all(&bytes)
        .and_then(|()| super::ledger::full_flush_file(&f))
        .map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

/// Rewrites a token record (the binding at `begin` — the issuing lane's daemon only).
pub fn write_token_in(root: &Path, t: &OwnerToken) -> Result<(), RepairError> {
    let p = token_path_in(root, &t.token_sha256);
    let mut t = t.clone();
    t.schema = super::REPAIR_SCHEMA;
    crate::atomic_write_json(&p, &t).map_err(|e| RepairError::Io(format!("{}: {e}", p.display())))
}

/// Whether a token's scope covers `rel` (exactly that path — O-C10-2).
pub fn token_covers(t: &OwnerToken, rel: &str) -> bool {
    t.paths.iter().any(|p| same_rel(p, rel))
}

/// The checks of a token at `begin` (C10 10.6): this lane (tokens are usable only by worker seats of
/// the issuing lane — F11), not expired, covering every target; single repair — once bound, only
/// that repair may use it (`bound_to` = the repair the begin will pick up, if any).
pub fn check_token_for_begin(
    t: &OwnerToken,
    lane: &LaneKey,
    targets: &[String],
    now: f64,
    bound_to: Option<&RepairId>,
) -> Result<(), String> {
    if t.lane_key != *lane {
        return Err(format!(
            "이 주인 지시 표는 부서 {} 에서 발급됐습니다 — 그 부서의 워커만 쓸 수 있습니다",
            t.lane_key.0
        ));
    }
    if now >= t.expires_at {
        return Err("주인 지시 표의 유효 기간(24시간)이 지났습니다 — 주인 지시를 다시 받아 새 표를 받으세요".into());
    }
    if let Some(rel) = targets.iter().find(|r| !token_covers(t, r)) {
        return Err(format!(
            "주인 지시 표가 이 파일을 덮지 않습니다: {rel} (표의 범위: {})",
            t.paths.join(", ")
        ));
    }
    for bound in [t.used_by_repair.as_ref(), t.repair.as_ref()].into_iter().flatten() {
        if bound_to != Some(bound) {
            return Err(format!(
                "이 주인 지시 표는 이미 수리 {} 에 묶였습니다 — 한 표는 한 수리에만 씁니다",
                bound.as_str()
            ));
        }
    }
    Ok(())
}

// ───────────────────────────── the CSO revision marker (RULINGS AQ8 / AI9; D:937) ─────────────────────────────

/// The registrar's marker (`cysjavis-pack/bin/javis_preflight.py:CSO_DIRECTIVE_REV_MARKER` — one
/// Rust constant, pinned by a parity test). Removing it from the CSO directive un-registers the CSO
/// capability gate (`javis_preflight.py` 382–391: conditional registration).
pub const CSO_DIRECTIVE_REV_MARKER: &str = "<!-- cso-directive-rev: 2026-09-06-alert-inbox -->";
/// The marker must be within the first 20 lines (`CSO_DIRECTIVE_MARKER_MAX_LINE`).
pub const CSO_DIRECTIVE_MARKER_MAX_LINE: usize = 20;
/// The folded base name of the CSO directive.
pub const CSO_DIRECTIVE_NAME: &str = "cso_directive.md";

/// `capgate_marker_ok` of `javis_preflight.py`: a line of the first 20, stripped, equals the marker.
pub fn cso_marker_in_head(text: &str) -> bool {
    text.lines()
        .take(CSO_DIRECTIVE_MARKER_MAX_LINE)
        .any(|l| l.trim() == CSO_DIRECTIVE_REV_MARKER)
}

/// The refusal sentence of a CSO-directive candidate that drops the marker (AQ8).
pub fn cso_marker_refusal_ko(rel: &str) -> String {
    format!(
        "{rel}: 수정본의 앞 {CSO_DIRECTIVE_MARKER_MAX_LINE}줄에서 CSO 지침 개정 표지 `{CSO_DIRECTIVE_REV_MARKER}` 가 사라졌습니다 — 이 표지를 지우면 점검 도구가 CSO 능력 게이트의 등록을 풀어 CSO 가 경보 없이 능력만 잃습니다(RULINGS AQ8). 표지 줄을 앞 {CSO_DIRECTIVE_MARKER_MAX_LINE}줄 안에 그대로 두고 다시 기록하세요"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q<'a>(asked_at: f64, path: Option<&'a str>, first: Option<(&'a str, &'a str, f64)>, machine: bool) -> QuestionView<'a> {
        QuestionView {
            asked_at,
            path,
            first_human: first,
            machine_seen: machine,
        }
    }

    fn view<'a>(origin: &'a str, at: f64, question: Option<QuestionView<'a>>, named: &'a [String]) -> PromptView<'a> {
        PromptView {
            digest: "dg",
            origin,
            at,
            question,
            named_docs: named,
            last_human: None,
        }
    }

    /// Q1: the fixed list, whole prompt or leading word; long sentences, other words, empty → no.
    #[test]
    fn affirmative_list() {
        for yes in ["예", " 네 ", "네.", "응!", "그래요", "좋아", "허락", "켜", "yes", "Y", "OK", "ok.", "네, 켜 주세요", "예 고쳐도 됩니다", "Yes please"] {
            assert!(is_affirmative(yes), "{yes:?}");
        }
        for no in [
            "",
            "아니요",
            "안 돼",
            "no",
            "예전에 말했듯이",
            "noted",
            "yesterday",
            "글쎄",
            "네 그런데 잠깐만 다른 얘기부터 하자 이건 아주 긴 문장이라서 대답이 아니다",
            "고쳐라 soul.md",
        ] {
            assert!(!is_affirmative(no), "{no:?}");
        }
    }

    /// Q1 / T3 / T5 / U4(b): every refusal of `grant on` names its reason; the accepted answer is the
    /// question's first owner-typed prompt.
    #[test]
    fn grant_on_judgment() {
        let none: [String; 0] = [];
        assert_eq!(judge_grant_on(None, 10.0).unwrap_err(), ASK_FIRST_KO);
        let p = view("human", 5.0, None, &none);
        assert_eq!(judge_grant_on(Some(&p), 10.0).unwrap_err(), ASK_FIRST_KO, "no question registered");
        let p = view("human", 5.0, Some(q(1.0, Some("soul.md"), Some(("d", "예", 2.0)), false)), &none);
        assert!(judge_grant_on(Some(&p), 10.0).unwrap_err().contains("규칙 문서 표 질문"), "U4(b)");
        let p = view("human", 5.0, Some(q(1.0, None, Some(("d", "예", 2.0)), false)), &none);
        assert!(judge_grant_on(Some(&p), 1.0 + 1_801.0).unwrap_err().contains("만료"), "31 minutes");
        let p = view("human", 5.0, Some(q(1.0, None, Some(("d", "예", 2.0)), true)), &none);
        assert!(judge_grant_on(Some(&p), 10.0).unwrap_err().contains("기계"), "machine between");
        let p = view("machine", 5.0, Some(q(1.0, None, None, true)), &none);
        assert!(judge_grant_on(Some(&p), 10.0).unwrap_err().contains("기계"), "a machine 예 only");
        let p = view("human", 5.0, Some(q(1.0, None, None, false)), &none);
        assert!(judge_grant_on(Some(&p), 10.0).unwrap_err().contains("아직 없습니다"));
        let p = view("human", 5.0, Some(q(1.0, None, Some(("d", "오늘 날씨 어때", 2.0)), false)), &none);
        assert!(judge_grant_on(Some(&p), 10.0).unwrap_err().contains("허락"), "unrelated line");
        let p = view("human", 5.0, Some(q(1.0, None, Some(("dg2", "네", 2.0)), false)), &none);
        assert_eq!(
            judge_grant_on(Some(&p), 10.0).unwrap(),
            Answer { digest: "dg2".into(), excerpt: "네".into(), at: 2.0 }
        );
    }

    /// Q1: `grant off` needs an owner-typed prompt after the last decision.
    #[test]
    fn grant_off_judgment() {
        let none: [String; 0] = [];
        assert!(judge_grant_off(None, None).is_err());
        let p = view("human", 50.0, None, &none);
        assert!(judge_grant_off(Some(&p), Some(40.0)).is_ok());
        assert!(judge_grant_off(Some(&p), None).is_ok());
        assert!(judge_grant_off(Some(&p), Some(60.0)).is_err(), "before the last decision");
        for o in ["machine", "unknown", ""] {
            let p = view(o, 50.0, None, &none);
            assert!(judge_grant_off(Some(&p), Some(40.0)).is_err(), "{o}");
        }
        let p = view("machine", 50.0, Some(q(41.0, None, Some(("d", "아무 말", 45.0)), false)), &none);
        assert!(judge_grant_off(Some(&p), Some(40.0)).is_ok(), "the question's owner-typed answer");
        // CH4: the owner's "끄라" at 45, then a machine prompt at 50 — still accepted
        let mut p = view("machine", 50.0, None, &none);
        p.last_human = Some(("off-digest", 45.0));
        assert_eq!(judge_grant_off(Some(&p), Some(40.0)).unwrap().digest, "off-digest");
        assert!(judge_grant_off(Some(&p), Some(46.0)).is_err(), "the owner's words came before the decision");
    }

    /// Q2 / U4: (a) named in the latest owner-typed prompt; (b) an affirmative answer to a question
    /// for exactly that path; a different document or no owner input → refused.
    #[test]
    fn owner_token_judgment() {
        let soul = vec!["soul.md".to_string()];
        let none: Vec<String> = vec![];
        // "고쳐라 soul.md" → soul.md only
        let p = view("human", 5.0, None, &soul);
        assert_eq!(judge_owner_token(Some(&p), "soul.md", 10.0).unwrap().0, TokenBasis::NamedInPrompt);
        assert_eq!(judge_owner_token(Some(&p), "SOUL.md", 10.0).unwrap().0, TokenBasis::NamedInPrompt);
        assert!(judge_owner_token(Some(&p), "directives/MASTER_DIRECTIVE.md", 10.0).is_err());
        // a later machine prompt does not erase what the owner named (named_docs changes only on an
        // owner-typed prompt) — provenance digest is then unknown
        let p = view("machine", 6.0, None, &soul);
        let (_, a) = judge_owner_token(Some(&p), "soul.md", 10.0).unwrap();
        assert_eq!(a.digest, "");
        // nothing named, no question
        let p = view("human", 5.0, None, &none);
        assert!(judge_owner_token(Some(&p), "soul.md", 10.0).unwrap_err().contains("이름으로 부르지"));
        assert!(judge_owner_token(None, "soul.md", 10.0).is_err());
        // (b)
        let p = view("human", 5.0, Some(q(1.0, Some("soul.md"), Some(("d", "네", 2.0)), false)), &none);
        assert_eq!(judge_owner_token(Some(&p), "soul.md", 10.0).unwrap().0, TokenBasis::AnsweredQuestion);
        assert!(judge_owner_token(Some(&p), "directives/MASTER_DIRECTIVE.md", 10.0).unwrap_err().contains("다른 문서"));
        let p = view("human", 5.0, Some(q(1.0, Some("soul.md"), Some(("d", "네", 2.0)), true)), &none);
        assert!(judge_owner_token(Some(&p), "soul.md", 10.0).is_err(), "machine_seen");
        let p = view("human", 5.0, Some(q(1.0, Some("soul.md"), Some(("d", "글쎄", 2.0)), false)), &none);
        assert!(judge_owner_token(Some(&p), "soul.md", 10.0).is_err(), "not affirmative");
        let p = view("human", 5.0, Some(q(1.0, Some("soul.md"), Some(("d", "네", 2.0)), false)), &none);
        assert!(judge_owner_token(Some(&p), "soul.md", 1_900.0).is_err(), "lapsed");
        // the grant question does not issue tokens
        let p = view("human", 5.0, Some(q(1.0, None, Some(("d", "네", 2.0)), false)), &none);
        assert!(judge_owner_token(Some(&p), "soul.md", 10.0).is_err());
    }

    /// O-C10-2: `ot-` + base32 of 128 bits; the hash is the ledger key; the layout is `paths.rs`'s.
    #[test]
    fn token_format_and_layout() {
        let t = token_from_bytes(&[0u8; 16]);
        assert_eq!(t, "ot-aaaaaaaaaaaaaaaaaaaaaaaaaa");
        let t = token_from_bytes(&[0xff; 16]);
        assert_eq!(t, "ot-77777777777777777777777774");
        assert!(is_token_shape(&t));
        assert!(!is_token_shape("ot-AAAAAAAAAAAAAAAAAAAAAAAAAA"));
        assert!(!is_token_shape("ot-short"));
        assert!(!is_token_shape("xx-aaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert_eq!(base32_lower(b"foobar"), "mzxw6ytboi");
        assert_eq!(token_sha256("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _lock = crate::pack::PACK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let local = std::env::temp_dir().join(format!("cys-t4-oo-{}", std::process::id()));
        let _env = crate::pack::EnvGuard::set("CYS_LOCAL_DIR", &local);
        let sha = token_sha256("x");
        assert_eq!(
            token_path_in(&crate::repair::paths::repairs_root(), &sha),
            crate::repair::paths::owner_token_path(&sha)
        );
    }

    /// The token record round-trips; create-new never replaces; a foreign hash, garbage or a higher
    /// schema is unreadable; begin's checks (lane, expiry, scope, single repair).
    #[test]
    fn token_record_and_begin_checks() {
        let root = std::env::temp_dir().join(format!("cys-t4-tok-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&root);
        let tok = OwnerToken {
            schema: 1,
            token_sha256: token_sha256("ot-x"),
            lane_key: LaneKey("base".into()),
            issued_to_seat: 3,
            paths: vec!["soul.md".into()],
            repair: None,
            prompt_digest: "d".into(),
            prompt_excerpt: "named: soul.md".into(),
            issued_at: 100.0,
            expires_at: 100.0 + TOKEN_TTL_SECS as f64,
            used_by_repair: None,
        };
        create_token_in(&root, &tok).unwrap();
        assert!(create_token_in(&root, &tok).is_err(), "create-new never replaces");
        assert_eq!(read_token_in(&root, &tok.token_sha256).unwrap(), Some(tok.clone()));
        assert_eq!(read_token_in(&root, &token_sha256("other")).unwrap(), None);
        assert_eq!(read_token_in(&root, "../../etc").unwrap(), None, "only a hash is looked up");
        let lane = LaneKey("base".into());
        let soul = vec!["soul.md".to_string()];
        assert!(check_token_for_begin(&tok, &lane, &soul, 200.0, None).is_ok());
        assert!(check_token_for_begin(&tok, &lane, &["SOUL.md".to_string()], 200.0, None).is_ok());
        assert!(check_token_for_begin(&tok, &LaneKey("dept-2".into()), &soul, 200.0, None).unwrap_err().contains("부서"));
        assert!(check_token_for_begin(&tok, &lane, &soul, tok.expires_at, None).unwrap_err().contains("유효 기간"));
        assert!(check_token_for_begin(&tok, &lane, &["directives/MASTER_DIRECTIVE.md".to_string()], 200.0, None).unwrap_err().contains("덮지 않습니다"));
        let r1 = RepairId::parse("R-1").unwrap();
        let mut used = tok.clone();
        used.used_by_repair = Some(r1.clone());
        assert!(check_token_for_begin(&used, &lane, &soul, 200.0, None).unwrap_err().contains("이미 수리 R-1"));
        assert!(check_token_for_begin(&used, &lane, &soul, 200.0, Some(&r1)).is_ok(), "the pick-up of its own repair");
        write_token_in(&root, &used).unwrap();
        assert_eq!(read_token_in(&root, &tok.token_sha256).unwrap().unwrap().used_by_repair, Some(r1));
        std::fs::write(token_path_in(&root, &tok.token_sha256), b"{x").unwrap();
        assert!(read_token_in(&root, &tok.token_sha256).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// AQ8: the Rust marker equals the registrar's Python constant (and its line bound), and the head
    /// test is the registrar's: stripped-line equality within the first 20 lines.
    #[test]
    fn cso_marker_parity_with_preflight() {
        let py = include_str!("../../cysjavis-pack/bin/javis_preflight.py");
        let line = |name: &str| -> String {
            let l = py
                .lines()
                .find(|l| l.starts_with(&format!("{name} = ")))
                .unwrap_or_else(|| panic!("{name} not found in javis_preflight.py"));
            l[name.len() + 3..].split('#').next().unwrap().trim().to_string()
        };
        assert_eq!(line("CSO_DIRECTIVE_REV_MARKER"), format!("\"{CSO_DIRECTIVE_REV_MARKER}\""));
        assert_eq!(line("CSO_DIRECTIVE_MARKER_MAX_LINE"), CSO_DIRECTIVE_MARKER_MAX_LINE.to_string());
        // the shipped CSO directive carries it (a candidate is compared with that pre-image)
        let cso = include_str!("../../cysjavis-pack/directives/CSO_DIRECTIVE.md");
        assert!(cso_marker_in_head(cso));
        let m = CSO_DIRECTIVE_REV_MARKER;
        assert!(cso_marker_in_head(&format!("# x\n   {m}  \n")));
        assert!(!cso_marker_in_head(&format!("# x\nsee {m} here\n")), "quoted in prose is not the marker");
        let mut far = "x\n".repeat(20);
        far.push_str(m);
        assert!(!cso_marker_in_head(&far), "line 21 is outside the head");
        assert_eq!(folded_base("directives/CSO_DIRECTIVE.md"), CSO_DIRECTIVE_NAME);
    }
}
