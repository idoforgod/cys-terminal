//! 순수 3-way 병합 엔진(diffy 0.5 래퍼) + 검증 게이트 — PACK_MERGE_UPSTREAM_DESIGN_V2.md §4·§8.
//! ★T1 시점 비배선: 이 모듈은 어디서도 호출되지 않는다(런타임 파급 0 — grep 0건이 계약).
//!   소비 예정 — T3 install_into Merge3 arm(판정·IO 분리) · T4 pack-merge 대화형(diff3_merge 치환) · T4 doctor(suspect_damage import — 재구현 금지).
//! 계약: 전 함수 순수(IO 0·프로세스 0·env 0) · 입력 UTF-8 &str 한정(비UTF-8은 스윕 L1이 선차단).
//! ★diffy 실측 거동(핀 박제): 충돌 판정이 GNU diff3보다 보수적 — 인접 헝크·무개행 말미·mixed-EOL(LF↔CRLF)은 충돌側. 충돌=healed=손실 0이라 안전측(§8). CRLF 충돌 출력의 마커 줄은 \n(본문 줄은 \r\n 유지).

/// 3-way 병합 결과. `Conflict`는 diff3 스타일 마커 포함 전문(全文)이다.
pub enum Merge3Outcome {
    Clean(String),
    Conflict(String),
}

/// 순수 3-way 병합 — `diffy::merge` 기본 옵션 위임(Ok→Clean, Err→Conflict).
/// base는 비-Option — base 부재 정책은 소비자 몫이다.
pub fn merge3(base: &str, ours: &str, theirs: &str) -> Merge3Outcome {
    match diffy::merge(base, ours, theirs) {
        Ok(merged) => Merge3Outcome::Clean(merged),
        Err(conflicted) => Merge3Outcome::Conflict(conflicted),
    }
}

/// 병합 산출물 손상 의심 사유. 검사 순서는 `suspect_damage` 정의에 고정되어 있다.
#[derive(Debug, PartialEq, Eq)]
pub enum SuspectReason {
    Empty,
    SizeCollapse { base_bytes: usize, got_bytes: usize },
    ConflictMarkerResidue,
    ShebangLost,
    PureDeletionMajority { deleted_lines: usize, base_lines: usize },
}

/// 병합 산출물 손상 의심 게이트 — 검사 순서 고정:
/// Empty → SizeCollapse → ConflictMarkerResidue → ShebangLost → PureDeletionMajority.
pub fn suspect_damage(base: &str, ours: &str, merged: &str) -> Option<SuspectReason> {
    // ① Empty
    if merged.is_empty() {
        return Some(SuspectReason::Empty);
    }
    // ② SizeCollapse — 분모는 base(조상) 바이트 길이. base가 비었으면 비발동.
    if !base.is_empty() && merged.len() * 10 <= base.len() {
        return Some(SuspectReason::SizeCollapse {
            base_bytes: base.len(),
            got_bytes: merged.len(),
        });
    }
    // ③ ConflictMarkerResidue — merged의 어느 줄이 줄머리 정확 7연속 "<<<<<<<"/">>>>>>>"/"|||||||"로 시작.
    //    ★"======="는 검사하지 않는다(마크다운 setext 밑줄 오탐 실측 P18).
    if merged.lines().any(line_is_conflict_marker) {
        return Some(SuspectReason::ConflictMarkerResidue);
    }
    // ④ ShebangLost — 조상이 "#!"로 시작했다는 사실만이 기준(확장자 무관).
    if base.starts_with("#!") && !merged.starts_with("#!") {
        return Some(SuspectReason::ShebangLost);
    }
    // ⑤ PureDeletionMajority — 반드시 ours vs base로 계산(merged 기준 구현은 세탁 방어를
    //    무음 무력화 — 프로브 P7 실측). 줄 단위 = split_inclusive('\n')(종단 개행 포함 비교·
    //    CRLF의 \r는 줄 내용). 정의: ours가 base의 부분수열 ∧ ours≠base(⟺ δ=삭제만·추가 0).
    //    발동: (base_lines − ours_lines)*2 >= base_lines(정확 50% 포함 — 프로브 P17).
    let base_lines: Vec<&str> = base.split_inclusive('\n').collect();
    if base_lines.is_empty() {
        return None; // base_lines==0이면 비발동
    }
    let ours_lines: Vec<&str> = ours.split_inclusive('\n').collect();
    if ours_lines != base_lines && is_subsequence(&ours_lines, &base_lines) {
        let deleted_lines = base_lines.len() - ours_lines.len();
        if deleted_lines * 2 >= base_lines.len() {
            return Some(SuspectReason::PureDeletionMajority {
                deleted_lines,
                base_lines: base_lines.len(),
            });
        }
    }
    None
}

/// JSON 파스 게이트 — `rel`이 ".json"으로 끝나면 serde_json 파스 성공 여부, 아니면 무조건 통과.
pub fn json_gate(rel: &str, content: &str) -> bool {
    if rel.ends_with(".json") {
        serde_json::from_str::<serde_json::Value>(content).is_ok()
    } else {
        true
    }
}

/// 줄머리가 정확 7연속 마커 문자로 시작하는가(8연속 이상은 마커가 아니다).
fn line_is_conflict_marker(line: &str) -> bool {
    for (marker, ch) in [("<<<<<<<", '<'), (">>>>>>>", '>'), ("|||||||", '|')] {
        if let Some(rest) = line.strip_prefix(marker) {
            if !rest.starts_with(ch) {
                return true;
            }
        }
    }
    false
}

/// needle이 haystack의 부분수열(subsequence)인가 — 순서 보존·원소 정확 일치.
fn is_subsequence(needle: &[&str], haystack: &[&str]) -> bool {
    let mut hay = haystack.iter();
    needle.iter().all(|n| hay.any(|h| h == n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_clean(outcome: Merge3Outcome) -> String {
        match outcome {
            Merge3Outcome::Clean(s) => s,
            Merge3Outcome::Conflict(s) => panic!("Clean 기대였으나 Conflict:\n{s}"),
        }
    }

    fn expect_conflict(outcome: Merge3Outcome) -> String {
        match outcome {
            Merge3Outcome::Conflict(s) => s,
            Merge3Outcome::Clean(s) => panic!("Conflict 기대였으나 Clean:\n{s}"),
        }
    }

    // ── merge3 golden 핀 (성찰 프로브 실측 기대값) ──────────────────────────

    #[test]
    fn merge3_clean_disjoint() {
        let merged = expect_clean(merge3("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\nd\n"));
        assert_eq!(merged, "a\nB\nc\nd\n");
    }

    #[test]
    fn merge3_crlf_roundtrip() {
        // 3자 전부 CRLF 동형 → Clean + CRLF 바이트 보존.
        let merged = expect_clean(merge3(
            "a\r\nb\r\nc\r\n",
            "a\r\nB\r\nc\r\n",
            "a\r\nb\r\nc\r\nd\r\n",
        ));
        assert_eq!(merged, "a\r\nB\r\nc\r\nd\r\n");
    }

    #[test]
    fn merge3_mixed_eol_conflicts() {
        // base LF · ours CRLF(전 줄 EOL 전환) · theirs LF(내용 수정) → 충돌側(보수 판정).
        expect_conflict(merge3("a\nb\nc\n", "a\r\nb\r\nc\r\n", "a\nB\nc\n"));
    }

    #[test]
    fn merge3_no_trailing_nl_adjacent_conflicts() {
        // 무개행 말미 + 인접 헝크 → Conflict(보수 판정 박제 — Clean 아님).
        expect_conflict(merge3("a\nb\nc", "a\nB\nc", "a\nb\nc\nd"));
    }

    #[test]
    fn merge3_adjacent_line_edits_conflict() {
        // 인접 줄 분리 수정 → Conflict(diffy 보수성 API 핀).
        expect_conflict(merge3("a\nb\nc\n", "a\nB\nc\n", "a\nb\nC\n"));
    }

    #[test]
    fn merge3_marker_format_pin() {
        // diff3 스타일 기본 + 마커 7자 핀.
        let conflicted = expect_conflict(merge3("a\nb\nc\n", "a\nOURS\nc\n", "a\nTHEIRS\nc\n"));
        assert_eq!(
            conflicted,
            "a\n<<<<<<< ours\nOURS\n||||||| original\nb\n=======\nTHEIRS\n>>>>>>> theirs\nc\n"
        );
    }

    #[test]
    fn merge3_large_smoke() {
        // 40,000줄 합성 3자 → Clean · 내용 보존만 assert(시간 assert 금지 — CI 변동성).
        let mut base = String::new();
        for i in 0..40_000 {
            base.push_str(&format!("line{i}\n"));
        }
        let ours = base.replace("line100\n", "line100-ours\n");
        let theirs = base.replace("line39000\n", "line39000-theirs\n");
        let merged = expect_clean(merge3(&base, &ours, &theirs));
        assert_eq!(merged.lines().count(), 40_000);
        assert!(merged.contains("line100-ours\n"));
        assert!(merged.contains("line39000-theirs\n"));
    }

    // ── suspect_damage 핀 ──────────────────────────────────────────────────

    #[test]
    fn suspect_pure_deletion_wash() {
        // base 10줄 · ours=앞 5줄(순수 삭제) · theirs=2행만 전진 → merge3는 Clean으로 세탁,
        // 게이트가 세탁을 잡는다.
        let base = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let ours = "l1\nl2\nl3\nl4\nl5\n";
        let theirs = "L1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        let merged = expect_clean(merge3(base, ours, theirs));
        assert_eq!(merged, "L1\nL2\nl3\nl4\nl5\n");
        assert_eq!(
            suspect_damage(base, ours, &merged),
            Some(SuspectReason::PureDeletionMajority { deleted_lines: 5, base_lines: 10 })
        );
    }

    #[test]
    fn suspect_pure_deletion_boundary() {
        // 6줄→3줄(정확 50%) → 발동(프로브 P17).
        let base = "a\nb\nc\nd\ne\nf\n";
        let ours = "a\nb\nc\n";
        assert_eq!(
            suspect_damage(base, ours, ours),
            Some(SuspectReason::PureDeletionMajority { deleted_lines: 3, base_lines: 6 })
        );
    }

    #[test]
    fn suspect_marker_carry() {
        // ours에 기존 "<<<<<<<" 마커 잔존 → clean 병합이 운반 → ConflictMarkerResidue(프로브 P16).
        let base = "a\nb\nc\n";
        let ours = "a\n<<<<<<< stale\nc\n";
        let theirs = "a\nb\nc\nd\n";
        let merged = expect_clean(merge3(base, ours, theirs));
        assert_eq!(merged, "a\n<<<<<<< stale\nc\nd\n");
        assert_eq!(
            suspect_damage(base, ours, &merged),
            Some(SuspectReason::ConflictMarkerResidue)
        );
    }

    #[test]
    fn suspect_setext_no_false_positive() {
        // 마크다운 setext 밑줄("=======")만 → None(오탐 금지 핀 — P18).
        let doc = "제목\n=======\n";
        assert_eq!(suspect_damage(doc, doc, doc), None);
    }

    #[test]
    fn suspect_empty() {
        assert_eq!(suspect_damage("a\n", "a\n", ""), Some(SuspectReason::Empty));
    }

    #[test]
    fn suspect_size_collapse() {
        // 경계: merged.len()*10 == base.len() → 발동(<= 포함). base 40바이트 · merged 4바이트.
        let base = "aaaaaaaaa\naaaaaaaaa\naaaaaaaaa\naaaaaaaaa\n";
        let merged = "bbb\n";
        assert_eq!(
            suspect_damage(base, base, merged),
            Some(SuspectReason::SizeCollapse { base_bytes: 40, got_bytes: 4 })
        );
    }

    #[test]
    fn suspect_shebang_lost() {
        // 조상이 "#!"로 시작했고 merged는 아님(확장자 무관).
        let base = "#!/bin/sh\necho hi\n";
        let merged = "echo hi\n";
        assert_eq!(suspect_damage(base, base, merged), Some(SuspectReason::ShebangLost));
    }

    // ── json_gate 핀 ───────────────────────────────────────────────────────

    #[test]
    fn json_gate_pins() {
        assert!(json_gate("conf/settings.json", "{\"k\": [1, 2, {\"n\": null}]}"));
        assert!(!json_gate("conf/settings.json", "{\"k\": broken"));
        assert!(json_gate("notes/readme.txt", "{\"k\": broken"));
    }
}
