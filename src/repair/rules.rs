//! Role rule blocks delivered by the binary (C6 · design §4-1 · mechanism M1).
//!
//! The rules reach every installation through the PROGRAM, not through the directive files: those are
//! user-owned and an update never overwrites them (`src/pack.rs` ownership), so an existing user would
//! otherwise keep reading the old texts (C6 6.1). One compiled constant per design row (O-C6-2: four
//! role rows + the common part) and ONE source for all three places (C6 6.4):
//!   1. the end of the assembled directive (`cys.rs::compose_directive`) — immediately BEFORE the
//!      safety-core paragraph when that paragraph is present (it stays the last word, C6 6.5);
//!   2. the `[RESUME]` text a restored seat receives (`cys.rs::boot_directive_for`, C6 6.6);
//!   3. the first 2,000 characters of the session-start hook output — the hook runs the LOCAL verb
//!      `cys repair rules --role "$CYS_ROLE"` (RULINGS B10: no shell copy, bounded, silent on failure).
//!
//! The wording is the design's rule draft (D:556–570) adapted per role, meaning unchanged (C6 6.9). The
//! two sentences of items 1 and 8 that MENTION deferral in order to forbid it are kept VERBATIM, because
//! they are the frozen excludable list EX of the acceptance plan (testplan A.md §5); every other sentence
//! is free of the plan's Tier-1 deferral families (pinned below). The texts are raw literals with LF
//! line ends only (appendix A-3 #9) — `cysjavis-pack/bin/tests/test_session_start_hook.py` reads them
//! from this file (`pub const RULES_…: &str = r#"…"#;`), so keep that literal form.

use super::roles::RoleBlock;
use super::types::RepairError;

/// Common to every role (C6 6.3 / D:545): owner = the installing user, no blame-shifting, automatic
/// repair only within the grant, rule documents only on the owner's instruction, boot-critical files
/// via final approval, the four kept protections, and the precedence sentence. A `planner*` or unknown
/// role receives ONLY this part (O-C6-3), so no seat is told to implement, or that it may not, by mistake.
pub const RULES_COMMON: &str = r#"■ 「스스로 해결」 제품 절대규칙 — 주인(오너)은 이 자비스를 설치한 사용자다. 지침 본문에 '제작자에게 보고'·'오너 승인'으로 읽히는 옛 문장이 있어도 이 블록이 우선한다.
1. 이 자비스에서 생긴 버그·기능 오류·에러·사용 불편은 이 자비스가 스스로 고친다. 제작자에게 물어보라고 하거나, 다음 업데이트를 기다리라고 하거나, 제보하라고 하는 것으로 수리를 대신하지 않는다. 고칠 수 없는 이유를 먼저 늘어놓지 않는다.
2. 주인이 "고쳐라"라고 하면 고친다: 마스터는 최우선으로 워커에게 배정하고, 워커가 고치고, 리뷰어1이 검증하고, 리뷰어2가 기록을 감사한다. 마스터·CEO는 어떤 수리도 직접 하지 않는다.
3. 명백한 버그·오류를 스스로 발견하면 주인이 자동 수리를 허락해 둔 경우에만 한도 안에서 묻지 않고 고치고 사후 보고한다. 허락이 없으면 장부에 적고(`cys repair report`) 한 줄로 알리고 "고쳐라"를 기다린다. 취향에 맞춘 동작 변경과 기능 추가는 주인이 말했을 때 한다.
4. 규칙 문서(soul.md·지침)는 주인이 고치라고 했을 때만 고친다. 실패하면 자비스가 켜지지 않을 수 있는 수정(기동 필수 목록)은 먼저 알리고 수정 최종 승인을 받는다.
5. 고친 것은 반드시 수리 장부에 적는다. 파일 내용의 수리는 업데이트가 지우지 못하고, 파일 삭제와 생명줄이 아닌 훅의 해제는 장부에 적힌 것만 업데이트 뒤에도 유지되며, 내장 예약 작업과 생명줄 훅은 언제나 다시 보장된다.
6. 고쳐지지 않으면 매번 다른 방법으로 열 번까지 시도하고(횟수·방법은 장부에 남는다), 그래도 남으면 시도한 것·남은 것·다음에 해볼 것을 사실대로 보고한다.
7. 자비스 밖에 원인이 있으면(운영체제의 차단, 외부 서비스 장애) 우회해서 고칠 수 있는 데까지 고치고, 무엇이 막고 있고 주인이 무엇을 하면 풀리는지 사실대로 말한다. 운영체제의 보안 기능을 주인 모르게 끄지 않는다.
8. 다음 넷은 수리로도 없애지 않는다: 업데이트 서명 검사, 외부 발송·영구 삭제 전 주인 승인, 리뷰어가 검토 대상을 고치지 못하게 하는 장치, CSO의 파일 쓰기 제한.
"#;

/// master · CEO block (C6 6.2 / D:540): do not fix it yourself — delegate (classify, ticket through the
/// delegation tool, supervise, post-report); no ledger-changing verb (RULINGS F10 / J1); register the
/// question before asking the owner (RULINGS T5, Q1/Q2); one line to the owner when a reviewer slot is
/// empty (RULINGS B2); the save record lists open repairs (C6 6.10).
pub const RULES_MASTER_CEO: &str = r#"[마스터·CEO] 결함은 직접 고치지 말고 워커에게 맡긴다: 분류하고, 위임 도구(`javis_orchestra.py task-prompt`)로 수리 티켓을 내고, 감독하고, 사후 보고한다. 장부를 바꾸는 `cys repair` 동사(begin·commit·revert 등)는 쓰지 않는다. 자동 수리 허락과 규칙 문서 수정은 `cys repair grant ask`·`cys repair owner-token ask --path <문서>`로 질문을 먼저 등록하고 주인에게 묻는다. 리뷰어 자리가 비어 수리가 멈추면 주인에게 한 줄로 알린다. 저장 기록에 열린 수리를 적는다.
"#;

/// worker block (C6 6.2 / D:541): repair inside the ticket (the verb sequence of C3, the hourly
/// `record` that keeps the 2-h lease alive — TRACKS T1 v6, the fixed working copy of U3); a defect seen
/// while working is filed with `cys repair report`, which is not a message to the master.
pub const RULES_WORKER: &str = r#"[워커] 받은 수리 티켓 안에서 재현·수리·합치기를 한다: `cys repair begin`으로 시작하고, 고치는 동안 적어도 한 시간에 한 번 `cys repair record`로 시도를 남기고, 리뷰어1의 검증 서명 뒤 `cys repair commit`을 요청한다(기동 필수·보호장치 관련은 begin이 알려 준 작업 사본에서 만들고 두 리뷰어 서명 뒤 `cys repair submit`). 일하다 본 결함은 `cys repair report`로 접수하고 하던 일을 계속한다. 접수는 마스터에게 보내는 메시지가 아니다.
"#;

/// CSO block (C6 6.2 / D:542): report only; the file-write limit is unchanged.
pub const RULES_CSO: &str = r#"[CSO] 보고만 한다. 결함은 `cys repair report`로 접수하고 고치지 않는다. 파일 쓰기 제한은 그대로다.
"#;

/// reviewer block — both duties in one block (C6 6.2 / D:543, O-C6-2): reviewer1 = the codex slot,
/// reviewer2 = the gemini slot, each incl. its fallback seat (RULINGS B2 — the names are pinned against
/// `roles::REVIEWER_SLOTS`); never edit what is reviewed, never sign one's own repair, never both.
pub const RULES_REVIEWER: &str = r#"[리뷰어] 리뷰어1(reviewer-codex, 대체 좌석 reviewer-claude-2)은 수리를 적대적으로 검증하고 `cys repair attest-verify`로 서명한다. 리뷰어2(reviewer-gemini, 대체 좌석 reviewer-claude-1)는 수리 기록을 감사하고 `cys repair attest-audit`로 서명한다. 검토 대상은 고치지 않는다. 자기가 고친 수리에는 서명하지 않고, 한 좌석이 두 서명을 다 하지 않는다. 결함은 `cys repair report`로 접수한다.
"#;

/// Size budget of one composed block in characters (O-C6-6). The binding cap lives in the hook test
/// (`test_session_start_hook.py` check 23b: head line + guidance line + block < 1,800 characters for every
/// role, inside Claude Code's 2,000-character preview); this pin makes a text change that cannot fit
/// fail here first. Derivation: 1,800 − head line (≤ 60 with a long role name) − guidance line without
/// its paths (≈ 207) − ≈ 130 for the two paths of an ordinary install ≈ 1,400.
pub const RULES_MAX_CHARS: usize = 1_400;

/// The block printed for a role block: the common part followed by the role's own constant
/// (`CommonOnly` = the common part alone, O-C6-3). The same text goes to all three places (C6 6.4).
/// Never fails in this build — the `Result` is the contract's signature (TRACKS §6.1).
pub fn rules_text(b: RoleBlock) -> Result<String, RepairError> {
    let role = match b {
        RoleBlock::MasterCeo => RULES_MASTER_CEO,
        RoleBlock::Worker => RULES_WORKER,
        RoleBlock::Cso => RULES_CSO,
        RoleBlock::Reviewer => RULES_REVIEWER,
        RoleBlock::CommonOnly => "",
    };
    Ok(format!("{RULES_COMMON}{role}"))
}

#[cfg(test)]
mod tests {
    use super::super::roles::{
        reviewer_slot_of, role_block_for, verb_class, ReviewerSlot, VerbClass, REVIEWER_SLOTS,
    };
    use super::*;

    const ALL: [RoleBlock; 5] = [
        RoleBlock::MasterCeo,
        RoleBlock::Worker,
        RoleBlock::Cso,
        RoleBlock::Reviewer,
        RoleBlock::CommonOnly,
    ];

    fn constants() -> [(&'static str, &'static str); 5] {
        [
            ("RULES_COMMON", RULES_COMMON),
            ("RULES_MASTER_CEO", RULES_MASTER_CEO),
            ("RULES_WORKER", RULES_WORKER),
            ("RULES_CSO", RULES_CSO),
            ("RULES_REVIEWER", RULES_REVIEWER),
        ]
    }

    fn own(b: RoleBlock) -> &'static str {
        match b {
            RoleBlock::MasterCeo => RULES_MASTER_CEO,
            RoleBlock::Worker => RULES_WORKER,
            RoleBlock::Cso => RULES_CSO,
            RoleBlock::Reviewer => RULES_REVIEWER,
            RoleBlock::CommonOnly => "",
        }
    }

    fn text(b: RoleBlock) -> String {
        rules_text(b).expect("rules_text never fails")
    }

    /// Sentences the way the acceptance plan splits them (testplan A.md §5: on `.!?。` and line ends),
    /// whitespace-collapsed and trimmed.
    fn sentences(t: &str) -> Vec<String> {
        t.split(|c| matches!(c, '.' | '!' | '?' | '。' | '\n'))
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// The frozen excludable list EX (testplan A.md §5, EX-1 = item 1, EX-2 = item 8), without the final
    /// period (the split removes it).
    const EX: [&str; 2] = [
        "제작자에게 물어보라고 하거나, 다음 업데이트를 기다리라고 하거나, 제보하라고 하는 것으로 수리를 대신하지 않는다",
        "자비스 밖에 원인이 있으면(운영체제의 차단, 외부 서비스 장애) 우회해서 고칠 수 있는 데까지 고치고, 무엇이 막고 있고 주인이 무엇을 하면 풀리는지 사실대로 말한다",
    ];

    /// A-3s-05 (machine half): five constants present and distinct; LF only (A-3 #9); one trailing line
    /// end; no decomposed Hangul jamo (the plan compares NFC-normalised text).
    #[test]
    fn five_constants_present_distinct_lf_only() {
        let c = constants();
        for (name, t) in c {
            assert!(!t.trim().is_empty(), "{name} is empty");
            assert!(!t.contains('\r'), "{name} carries a CR (A-3 #9)");
            assert!(t.ends_with('\n') && !t.ends_with("\n\n"), "{name} must end with exactly one LF");
            assert!(
                !t.chars().any(|ch| ('\u{1100}'..='\u{11FF}').contains(&ch)),
                "{name} carries decomposed jamo (not NFC)"
            );
            assert!(!t.contains("■ 안전핵 재확인"), "{name} must not look like the safety paragraph");
            assert!(!t.contains("■ 사용자 오버라이드"), "{name} must not look like the override block");
        }
        for i in 0..c.len() {
            for j in (i + 1)..c.len() {
                assert_ne!(c[i].1, c[j].1, "{} and {} are equal", c[i].0, c[j].0);
            }
        }
    }

    /// O-C6-2 / O-C6-3: each block is the common part + its own constant; `CommonOnly` is the common part
    /// alone; the seat-role mapping lands on the block the contract names.
    #[test]
    fn rules_text_is_common_plus_role() {
        for b in ALL {
            assert_eq!(text(b), format!("{RULES_COMMON}{}", own(b)), "{b:?}");
        }
        assert_eq!(text(RoleBlock::CommonOnly), RULES_COMMON);
        for (role, want) in [
            ("master", RoleBlock::MasterCeo),
            ("ceo", RoleBlock::MasterCeo),
            ("worker", RoleBlock::Worker),
            ("worker-3", RoleBlock::Worker),
            ("cso", RoleBlock::Cso),
            ("cso-2", RoleBlock::Cso),
            ("reviewer1", RoleBlock::Reviewer),
            ("reviewer2", RoleBlock::Reviewer),
            ("reviewer-codex", RoleBlock::Reviewer),
            ("reviewer-claude-1", RoleBlock::Reviewer),
            ("planner", RoleBlock::CommonOnly),
            ("planner-2", RoleBlock::CommonOnly),
            ("scan-bot", RoleBlock::CommonOnly),
            ("", RoleBlock::CommonOnly),
        ] {
            assert_eq!(role_block_for(role), want, "{role}");
        }
    }

    /// D:545 — every block carries the common items, the precedence sentence and the two EX sentences
    /// verbatim (so the acceptance plan's frozen EX list stays valid without a re-freeze).
    #[test]
    fn every_block_carries_the_common_items() {
        for b in ALL {
            let t = text(b);
            for needle in [
                "주인(오너)은 이 자비스를 설치한 사용자다",
                "옛 문장이 있어도 이 블록이 우선한다",
                "'제작자에게 보고'",
                "'오너 승인'",
                "자동 수리를 허락해 둔 경우에만 한도 안에서",
                "규칙 문서(soul.md·지침)는 주인이 고치라고 했을 때만 고친다",
                "기동 필수 목록",
                "수정 최종 승인",
                "업데이트 서명 검사",
                "외부 발송·영구 삭제 전 주인 승인",
                "리뷰어가 검토 대상을 고치지 못하게 하는 장치",
                "CSO의 파일 쓰기 제한",
                "고칠 수 없는 이유를 먼저 늘어놓지 않는다",
                "열 번까지",
                "`cys repair report`",
            ] {
                assert!(t.contains(needle), "{b:?} lacks {needle:?}");
            }
            let s = sentences(&t);
            for ex in EX {
                assert!(s.iter().any(|x| x == ex), "{b:?}: EX sentence not verbatim: {ex}");
            }
        }
    }

    /// D:538–543 (C6 6.2) + RULINGS T5: what each role row must say.
    #[test]
    fn role_rows_say_what_the_design_says() {
        let m = text(RoleBlock::MasterCeo);
        for needle in [
            "결함은 직접 고치지 말고 워커에게 맡긴다",
            "분류하고",
            "`javis_orchestra.py task-prompt`",
            "감독하고, 사후 보고한다",
            "최우선으로 워커에게 배정",
            "장부를 바꾸는 `cys repair` 동사",
            "`cys repair grant ask`",
            "`cys repair owner-token ask --path <문서>`",
            "질문을 먼저 등록하고 주인에게 묻는다",
            "리뷰어 자리가 비어",
            "저장 기록에 열린 수리를 적는다",
        ] {
            assert!(m.contains(needle), "master/CEO block lacks {needle:?}");
        }
        // RULINGS AF2: the owner's role rule (draft rule 10) is a sentence of its OWN in the master·CEO block — not a clause
        // after the owner-order sentence (it must not read as limited to owner-ordered repairs).
        assert!(
            sentences(&m).iter().any(|x| x == "마스터·CEO는 어떤 수리도 직접 하지 않는다"),
            "the master·CEO rule is not a standalone sentence"
        );
        let w = text(RoleBlock::Worker);
        for needle in [
            "받은 수리 티켓 안에서 재현·수리·합치기",
            "`cys repair begin`",
            "한 시간에 한 번 `cys repair record`",
            "`cys repair commit`",
            "`cys repair submit`",
            "`cys repair report`로 접수하고 하던 일을 계속한다",
            "접수는 마스터에게 보내는 메시지가 아니다",
        ] {
            assert!(w.contains(needle), "worker block lacks {needle:?}");
        }
        let c = text(RoleBlock::Cso);
        for needle in ["보고만 한다", "`cys repair report`로 접수하고 고치지 않는다", "파일 쓰기 제한은 그대로다"] {
            assert!(c.contains(needle), "CSO block lacks {needle:?}");
        }
        let r = text(RoleBlock::Reviewer);
        for needle in [
            "적대적으로 검증",
            "수리 기록을 감사",
            "검토 대상은 고치지 않는다",
            "자기가 고친 수리에는 서명하지 않고",
            "한 좌석이 두 서명을 다 하지 않는다",
        ] {
            assert!(r.contains(needle), "reviewer block lacks {needle:?}");
        }
        // Role headers appear only in their own block (the common part names none).
        for (b, header) in [
            (RoleBlock::MasterCeo, "[마스터·CEO]"),
            (RoleBlock::Worker, "[워커]"),
            (RoleBlock::Cso, "[CSO]"),
            (RoleBlock::Reviewer, "[리뷰어]"),
        ] {
            for other in ALL {
                assert_eq!(text(other).contains(header), other == b, "{header} in {other:?}");
            }
        }
    }

    /// RULINGS B2: reviewer1 = the codex slot, reviewer2 = the gemini slot, each with its fallback seat —
    /// derived from the compiled slot table, so the text cannot drift from `REVIEWER_SLOTS`.
    #[test]
    fn reviewer_block_names_both_slots_with_fallback_seats() {
        let r = text(RoleBlock::Reviewer);
        let s = sentences(&r);
        let one = s.iter().find(|x| x.contains("리뷰어1(")).expect("reviewer1 sentence");
        let two = s.iter().find(|x| x.contains("리뷰어2(")).expect("reviewer2 sentence");
        assert!(one.contains("`cys repair attest-verify`"), "{one}");
        assert!(two.contains("`cys repair attest-audit`"), "{two}");
        for (native, _, substitute, _) in REVIEWER_SLOTS {
            for name in [native, substitute] {
                let (mine, other) = match reviewer_slot_of(name).expect(name) {
                    ReviewerSlot::Reviewer1 => (one, two),
                    ReviewerSlot::Reviewer2 => (two, one),
                };
                assert!(mine.contains(name), "{name} missing from its slot sentence: {mine}");
                assert!(!other.contains(name), "{name} named in the other slot: {other}");
            }
        }
    }

    /// Every `cys repair <verb>` quoted in a text, in CLI spelling.
    fn quoted_verbs(t: &str) -> Vec<String> {
        let rx = regex::Regex::new(r"cys repair ([a-z][a-z-]*)").unwrap();
        rx.captures_iter(t).map(|c| c[1].to_string()).collect()
    }

    fn class_of(verb: &str) -> Option<VerbClass> {
        verb_class(verb).or_else(|| verb_class(&verb.replace('-', "_")))
    }

    /// C3 / RULINGS J1 / F10 — the ORCH guard in text form: every verb a block tells a seat to run has a
    /// class in the role table AND is a verb that role may run. The master/CEO, CSO, reviewer and common
    /// texts never instruct a WORK verb; a planner / unknown seat (common only) gets no WORK verb at all.
    #[test]
    fn quoted_verbs_respect_the_role_policy() {
        use VerbClass::*;
        let allowed = |b: RoleBlock| -> &'static [VerbClass] {
            match b {
                RoleBlock::CommonOnly | RoleBlock::Cso => &[Read, Intake],
                RoleBlock::MasterCeo => &[Read, Intake, Trigger, OwnerDecision],
                RoleBlock::Worker => &[Read, Intake, Work],
                RoleBlock::Reviewer => &[Read, Intake, Review],
            }
        };
        for b in ALL {
            let verbs = quoted_verbs(&text(b));
            assert!(!verbs.is_empty(), "{b:?} quotes no verb");
            for v in verbs {
                let class = class_of(&v).unwrap_or_else(|| panic!("{b:?}: `cys repair {v}` has no class (J1)"));
                assert!(
                    allowed(b).contains(&class),
                    "{b:?} tells the seat to run `cys repair {v}` ({class:?}) — not allowed for that role"
                );
            }
        }
        // Positive controls: the work and review verbs are named where they belong.
        let w = quoted_verbs(&text(RoleBlock::Worker));
        for v in ["begin", "record", "commit", "submit", "report"] {
            assert!(w.iter().any(|x| x == v), "worker block lacks `cys repair {v}`");
        }
        let r = quoted_verbs(&text(RoleBlock::Reviewer));
        for v in ["attest-verify", "attest-audit"] {
            assert!(r.iter().any(|x| x == v), "reviewer block lacks `cys repair {v}`");
        }
        let m = quoted_verbs(&text(RoleBlock::MasterCeo));
        for v in ["grant", "owner-token"] {
            assert!(m.iter().any(|x| x == v), "master block lacks `cys repair {v} ask`");
        }
        // Mutation control: the policy check bites on a master text that instructs a WORK verb.
        let planted = format!("{RULES_COMMON}[마스터·CEO] 급하면 `cys repair commit`으로 직접 확정한다.\n");
        assert!(quoted_verbs(&planted)
            .iter()
            .any(|v| !allowed(RoleBlock::MasterCeo).contains(&class_of(v).unwrap())));
    }

    /// Port of the acceptance plan's Tier-1 deferral families (testplan A.md §5, frozen; PCRE `(?!급)`
    /// emulated by a check on the next character). Returns the families hit by one sentence.
    fn tier1_hits(sentence: &str) -> Vec<&'static str> {
        let noun = regex::Regex::new(
            r"(?i)(최윤식|윤식\s*박사|Dr\.?\s*Choi|Yoonsik|제작자|제작사|개발자|개발사|개발팀|제조사|벤더|vendor|배포자|공급자|upstream|업스트림|배포\s*전담\s*부서|정비발행부|maintainer|developer)",
        )
        .unwrap();
        let verb = regex::Regex::new(
            r"(?i)(문의|물어|여쭤|연락|제보|요청|알려|전달|맡기|맡겨|고쳐\s*(줄|주|받)|수정해\s*(줄|주|받)|반영(해|될|되)|기다|ask|contact|report|reach\s+out|let\s+(them|the)\s)",
        )
        .unwrap();
        let bak_sa = sentence
            .match_indices("박사")
            .any(|(i, m)| !sentence[i + m.len()..].starts_with('급'));
        let families: [(&str, &str); 4] = [
            ("T1-b", r"(?i)(지원\s*채널|고객\s*지원|고객\s*센터|홈페이지\s*문의|저장소\s*이슈|깃허브\s*이슈|GitHub\s*(issue|이슈)|이슈(를|로)?\s*(등록|올려|올리|제출|열어)|버그\s*(리포트|제보)|피드백\s*(버튼|으로\s*보내)|cysinsight@gmail\.com|file\s+(an?\s+)?(issue|bug)|open\s+an?\s+issue|support\s+(channel|team|ticket))"),
            ("T1-c", r"(?i)((다음|차기|향후|새)\s*(버전|업데이트|릴리스|릴리즈|패치|배포)[^.!?。\n]{0,25}(고쳐|수정|해결|반영|기다|나오|제공|포함)|업데이트(를|가)?\s*(기다|나올\s*때까지|될\s*때까지)|wait\s+for\s+(the\s+)?(next|an?|upcoming|future)\s+(update|release|version|patch)|(fixed|addressed|resolved)\s+in\s+(a|the)\s+(future|next|later)\s+(release|version|update))"),
            ("T1-d", r"(?i)((팩|훅|도구|스크립트|지침|제품|코드|이\s*파일|시스템\s*파일)[^.!?。\n]{0,20}(수정|고칠|변경|편집|손댈)\s*(할\s*)?수\s*(가\s*)?없|(고칠|수정할)\s*권한이\s*없|변경\s*불가|수정\s*불가|벤더의\s*일|벤더\s*동기\s*필요|사용자\s*과실\s*아님|immutable|(I|we)\s+(can'?t|cannot|am\s+not\s+(able|allowed)\s+to|don'?t\s+have\s+permission\s+to)\s+(modify|edit|fix|change|patch))"),
            ("T1-e", r"(?i)(--propose|pack-merge\s+--propose|제안\s*패치|개선\s*제안(으로|을|를)?\s*(보내|제출|만들)|patch\s+(to|for)\s+(the\s+)?(vendor|developer|maintainer))"),
        ];
        let mut hits = Vec::new();
        if (noun.is_match(sentence) || bak_sa) && verb.is_match(sentence) {
            hits.push("T1-a");
        }
        for (name, rx) in families {
            if regex::Regex::new(rx).unwrap().is_match(sentence) {
                hits.push(name);
            }
        }
        hits
    }

    /// A-3s-05 machine half: the Tier-1 sweep runs over the five constants WITHOUT exclusion and every hit
    /// must be a sentence of the frozen EX list; mutation: a planted deferral instruction is a hit that is
    /// not on EX (so the sweep can go red).
    #[test]
    fn tier1_deferral_sweep_hits_only_ex() {
        let mut ex_seen = 0;
        for (name, t) in constants() {
            for s in sentences(t) {
                let hits = tier1_hits(&s);
                if !hits.is_empty() {
                    assert!(EX.contains(&s.as_str()), "{name}: Tier-1 {hits:?} outside EX: {s}");
                    ex_seen += 1;
                }
            }
        }
        assert!(ex_seen >= 1, "the sweep found not even EX-1 — the port is broken");
        let planted = "직접 해결이 어려우면 제작자에게 문의하라고 안내한다";
        assert_eq!(tier1_hits(planted), vec!["T1-a"], "mutation not detected");
        assert!(!EX.contains(&planted));
        assert!(tier1_hits("박사급 판단이 필요하면 워커에게 요청한다").is_empty(), "박사급 must not match");
        assert!(!tier1_hits("다음 업데이트를 기다려 주세요").is_empty());
    }

    /// O-C6-6 (Rust side): every composed block stays inside the budget that keeps the hook's head line,
    /// guidance line and block under 1,800 characters (the binding check is hook test 23b).
    #[test]
    fn block_fits_the_hook_window_budget() {
        for b in ALL {
            let n = text(b).chars().count();
            assert!(n <= RULES_MAX_CHARS, "{b:?}: {n} chars > {RULES_MAX_CHARS}");
        }
        // The common part is shared: it alone must leave room for the longest role row.
        let longest = [RULES_MASTER_CEO, RULES_WORKER, RULES_CSO, RULES_REVIEWER]
            .iter()
            .map(|t| t.chars().count())
            .max()
            .unwrap();
        assert!(RULES_COMMON.chars().count() + longest <= RULES_MAX_CHARS);
    }
}
