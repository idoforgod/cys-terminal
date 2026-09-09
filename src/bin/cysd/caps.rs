//! 능력(capability) 모델 — surface별 read/search/edit/commit/write-shell 권한 집합.
//! reviewer-*/planner 페인의 편집·커밋·write-shell을 cysd 원장이 deny-by-default로 좁혀
//! producer≠evaluator(리뷰어가 자기 산출물을 수정하는 reward-hack)를 코드로 봉쇄한다.
//! (T4-4 권한층 + T6-P3 역할 잠금의 단일 구현 — 두 진입, 한 모델.)
//!
//! penpot 클린룸 근거(개념·계약만·코드복사 0): register.cljs:64-76의
//! `cond-> content:write ⇒ content:read` 자동 함의(write⊇read 봉쇄 의미론)와
//! check-permission의 deny-by-default 멤버십 *계약*만 차용한다. penpot의 plugin permission
//! enum(11종)·all-zero+mcp 이중 우회·CLJS 본문은 옮기지 않으며, cys는 역할-기반 caps 집합 +
//! Rust 순수 정규화 함수로 재구현한다(MPL 파일전염 회피). 정직: penpot은 plugin permission을
//! 다루지 비-악의 에이전트의 내부 Edit 도구를 물리 차단하지 않는다 — cys 기여는 프로세스 원장
//! 인-프로세스 가드(cysd-매개 경로) + PreToolUse hook(에이전트 내부 도구의 실제 물리 enforcer).
//!
//! ★enforcement boundary (정직): caps 집합 자체는 *정책*이다. 이 정책을 강제하는 곳은 둘이다 —
//!   (1) cysd-매개 경로(send/scoped run write-shell): handlers.rs가 직접 게이트.
//!   (2) 에이전트 내부 도구(Claude Code Edit/Write/Bash): cysd는 *직접 못 막는다* —
//!       PreToolUse hook(cysjavis-pack/hooks/role-capability-gate.sh)이 실 enforcer.
//!   cysd가 에이전트 내부 Edit 도구를 막는다고 주장하지 않는다(불가능).

/// 능력 한 종류 — deny-by-default 멤버십의 원소.
/// read/search = 비변형(리뷰어 허용). edit/commit/write_shell = 변형(reviewer/planner deny).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")] // -> "read","search","edit","commit","write-shell"
pub enum Cap {
    Read,
    Search,
    Edit,
    Commit,
    WriteShell,
}

impl Cap {
    pub fn as_str(self) -> &'static str {
        match self {
            Cap::Read => "read",
            Cap::Search => "search",
            Cap::Edit => "edit",
            Cap::Commit => "commit",
            Cap::WriteShell => "write-shell",
        }
    }
    /// 변형(mutation) 능력 — 산출물을 바꾸는 권한. reviewer/planner에게 deny 대상.
    pub fn is_mutation(self) -> bool {
        matches!(self, Cap::Edit | Cap::Commit | Cap::WriteShell)
    }
}

/// surface별 권한 집합. deny-by-default 멤버십(allow 집합에 명시된 것만 허용).
/// 순수 additive — 기존 Surface 필드 불변, 신규 필드 1개로만 부착.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Caps {
    /// 허용 능력의 정규화된 집합(write⊇read 자동 함의 적용 후).
    pub allow: std::collections::BTreeSet<Cap>,
}

impl Caps {
    /// deny-by-default — 빈 집합(아무 능력 없음). 미지/외부 surface의 안전측 기본.
    pub fn none() -> Self {
        Caps {
            allow: std::collections::BTreeSet::new(),
        }
    }

    /// 역할 문자열에서 기본 caps를 도출한다(launch-agent/claim-role 시점 기록).
    /// reviewer-*/planner = 비변형만(read,search). worker/master/cso = full.
    /// 그 외(미지 role/없음) = deny-by-default none — 안전측.
    pub fn for_role(role: Option<&str>) -> Self {
        match role {
            Some(r) if is_reviewer_or_planner(r) => Caps::from_iter([Cap::Read, Cap::Search]),
            Some(r) if is_full_trust(r) => Caps::from_iter([
                Cap::Read,
                Cap::Search,
                Cap::Edit,
                Cap::Commit,
                Cap::WriteShell,
            ]),
            _ => Caps::none(),
        }
    }

    /// 명시 능력들로부터 정규화된 caps 구성(write⊇read 자동 함의 적용).
    pub fn from_iter<I: IntoIterator<Item = Cap>>(it: I) -> Self {
        let mut allow: std::collections::BTreeSet<Cap> = it.into_iter().collect();
        normalize_write_implies_read(&mut allow);
        Caps { allow }
    }

    /// 멤버십 술어 — deny-by-default(allow에 있어야 true).
    pub fn allows(&self, cap: Cap) -> bool {
        self.allow.contains(&cap)
    }

    /// 변형 능력을 하나라도 가지면 true(full-trust 식별·진단용 — 테스트가 박제).
    #[allow(dead_code)]
    pub fn can_mutate(&self) -> bool {
        self.allow.iter().any(|c| c.is_mutation())
    }
}

/// write⊇read 정규화(penpot register.cljs:64-76 cond-> 자동 함의의 클린룸 등가, 코드복사 0).
/// 변형 능력(edit/commit/write-shell)을 가지면 read를 자동 포함한다 — 쓸 수 있으면 읽을 수 있다.
/// 순수 함수 4줄: edit/commit/write-shell ⇒ read 봉쇄.
pub fn normalize_write_implies_read(allow: &mut std::collections::BTreeSet<Cap>) {
    if allow.iter().any(|c| c.is_mutation()) {
        allow.insert(Cap::Read);
    }
}

/// reviewer-*/planner 역할 식별 — 편집/커밋/write-shell deny 대상.
/// reviewer-gemini·reviewer-codex(agy/codex) 및 planner 변형 포함.
pub fn is_reviewer_or_planner(role: &str) -> bool {
    role.starts_with("reviewer") || role == "planner" || role.starts_with("planner-")
}

/// full-trust 역할 — worker/master/cso(및 worker-N dedup 변형).
///
/// ★(성찰 A10) **의도적으로 정확 일치**다. 특권 *좌석* 집합(`handlers::privileged_role` — 접두
/// `cso*`, 빈 좌석 탈취 봉쇄)과 full-trust *능력* 집합은 다른 축이다: 좌석 보호는 넓게, 능력은
/// 좁게. `cso-2` 를 특권 좌석으로 넓혀도 능력은 새지 않는다(권한 상승 0).
fn is_full_trust(role: &str) -> bool {
    role == "master"
        || role == "cso"
        || role == "worker"
        || role.starts_with("worker-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn reviewer_is_read_search_only() {
        let c = Caps::for_role(Some("reviewer-codex"));
        assert!(c.allows(Cap::Read));
        assert!(c.allows(Cap::Search));
        assert!(!c.allows(Cap::Edit));
        assert!(!c.allows(Cap::Commit));
        assert!(!c.allows(Cap::WriteShell));
        assert!(!c.can_mutate());
    }

    #[test]
    fn reviewer_gemini_and_planner_denied_mutation() {
        for r in ["reviewer-gemini", "reviewer", "planner", "planner-x"] {
            let c = Caps::for_role(Some(r));
            assert!(!c.allows(Cap::Edit), "{r} must not edit");
            assert!(!c.allows(Cap::WriteShell), "{r} must not write-shell");
        }
    }

    /// ★(성찰 A10) `cso-*` 변형은 특권 **좌석**(claim 게이트)이지만 full-trust **능력**은 아니다.
    #[test]
    fn cso_variants_are_privileged_seats_but_not_full_trust() {
        for r in ["cso-2", "cso-fresh-1700000000"] {
            assert!(crate::handlers::privileged_role(r), "{r} 좌석이 특권 게이트 밖이다");
            assert!(!is_full_trust(r), "{r} 가 full-trust 능력을 얻었다(권한 상승)");
            assert!(!Caps::for_role(Some(r)).allows(Cap::Edit), "{r} 가 edit 능력을 얻었다");
        }
        assert!(is_full_trust("cso") && crate::handlers::privileged_role("cso"));
    }

    #[test]
    fn worker_master_cso_full() {
        for r in ["worker", "worker-2", "master", "cso"] {
            let c = Caps::for_role(Some(r));
            assert!(c.allows(Cap::Edit), "{r} edit");
            assert!(c.allows(Cap::Commit), "{r} commit");
            assert!(c.allows(Cap::WriteShell), "{r} write-shell");
            assert!(c.allows(Cap::Read), "{r} read");
        }
    }

    #[test]
    fn unknown_and_none_role_deny_by_default() {
        assert_eq!(Caps::for_role(None).allow.len(), 0);
        assert_eq!(Caps::for_role(Some("totally-unknown")).allow.len(), 0);
        // deny-by-default: 빈 caps는 어떤 능력도 불허
        let c = Caps::none();
        assert!(!c.allows(Cap::Read));
        assert!(!c.allows(Cap::Edit));
    }

    #[test]
    fn write_implies_read_normalization() {
        // edit만 줘도 read가 자동 포함되어야 한다(write⊇read).
        let mut s: BTreeSet<Cap> = [Cap::Edit].into_iter().collect();
        normalize_write_implies_read(&mut s);
        assert!(s.contains(&Cap::Read), "edit must auto-include read");

        let mut s2: BTreeSet<Cap> = [Cap::WriteShell].into_iter().collect();
        normalize_write_implies_read(&mut s2);
        assert!(s2.contains(&Cap::Read), "write-shell must auto-include read");

        let mut s3: BTreeSet<Cap> = [Cap::Commit].into_iter().collect();
        normalize_write_implies_read(&mut s3);
        assert!(s3.contains(&Cap::Read), "commit must auto-include read");
    }

    #[test]
    fn read_does_not_imply_write() {
        // 역은 성립하지 않는다 — read/search만으로 edit 권한이 생기지 않는다(deny-by-default).
        let mut s: BTreeSet<Cap> = [Cap::Read, Cap::Search].into_iter().collect();
        normalize_write_implies_read(&mut s);
        assert!(!s.contains(&Cap::Edit));
        assert!(!s.contains(&Cap::Commit));
        assert!(!s.contains(&Cap::WriteShell));
    }

    #[test]
    fn from_iter_applies_normalization() {
        // Caps::from_iter도 정규화를 적용(edit→read 자동).
        let c = Caps::from_iter([Cap::Edit]);
        assert!(c.allows(Cap::Read));
        assert!(c.allows(Cap::Edit));
    }
}
