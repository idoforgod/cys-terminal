//! S8 — 오너가 명령할 때만 하는 휴지통 삭제(`trash.*` RPC · 번들 S8).
//!
//! ## 이 모듈이 **거부**하는 것 — 능력이 아니라 거부가 출하 문구다(ADDENDUM-2 C-R3)
//! · **하위 장치(마운트)를 인증할 수 없으면 그 항목 전체를 거부한다**(REFUSES when it cannot certify a
//!   descendant mount). "하위 마운트를 올바로 처리한다"가 아니다 — 후자는 DESIGN-S8.md:59 를 막은 문장
//!   형상이고, 플랫폼 실행은 여전히 **미측정**이다(기계 1대·OS 1종·한 순간).
//! · 시한이 지난 확인(confirm), 최근(recent) 항목, 최근 여부를 판정할 수 없는 항목도 **거부**다
//!   (ADDENDUM-1 — 판정 불가 = 삭제 안 함).
//!
//! ## 왜 인증기와 제거기가 **같은 집합**인가(ADDENDUM-2 C-R1)
//! 인증기는 항목 뿌리 아래 **모든 노드를 한 번의 순회로** 본다. 순회가 들어가지 못하는 노드(다른 장치·
//! 판독 불가·한도 초과)가 **하나라도** 있으면 그 항목은 통째로 거부된다. 그러므로 인증이 성공했다는 것은
//! "이 뿌리 아래에 장치 경계가 없다"는 뜻이고, 그때 `remove_dir_all` 의 순회는 인증기가 본 집합을
//! **넓힐 수 없다**(심링크는 따라가지 않고 링크 자체를 지운다). 인증을 한 순회로 하고 제거를 다른 순회로
//! 하면서 "오늘은 일치한다"고 말하는 구현은 이 조항을 만족하지 못한다 — 그래서 거부 판정이 제거의
//! **선행조건**이고, 그 판정 하나가 두 집합을 묶는다.
//!
//! ## 이 모듈이 **하지 않는** 것
//! 자동 재개 없음(봉인된 항목은 다음 미리보기 없이 다시 손대지 않는다) · 전체선택 없음 · 클릭만으로 넘어가는
//! 확인 단계 없음 · feed 항목 아님(S7 의 자동 판정자가 닿는 면이 아니다 · D-5/D-7) · 새 CLI 하위명령 없음
//! (`cys` 에는 이 동사가 없다 — 에이전트가 평시 동작으로 닿을 경로를 만들지 않는다).
//!
//! ## 정직한 한계(거버넌스 구분이지 보안 경계가 아니다)
//! 오너 등급은 `operator_token` + pane 무귀속이다. 같은 UID 로 토큰 파일을 읽을 수 있는 프로세스는 raw RPC
//! 로 이 등급을 참칭할 수 있다(`handlers.rs` `caller_is_owner` 문서가 스스로 명시). 이 등급이 닫는 것은
//! **지침을 따르는 정상 노드가 평시 동작으로 오너 입력을 사칭하는 경로**다 — 공용 `cys` CLI 는 어떤 경로에서도
//! 두 토큰 키를 붙이지 않는다. 위조된 삭제는 **최종적**이고 복구 단계는 없다(오너 결정 O0).

use crate::handlers::Reply;
use crate::state::{now_epoch, state_dir, Daemon};
use cys::{err_response, ok_response};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

// ─── ADDENDUM-1 의 두 창(이름 있는 상수 · 이 모듈이 유일한 정의처) ────────────────────────────
/// 최근(recent) 창 — 이보다 **어리면** 쓰는 중일 수 있다고 보고 거부한다.
/// ★`recent` 는 **긍정으로** 판정한다: 나이가 유한하고 이 값 **이상**일 때만 '최근 아님'이다.
/// 더 어림·미래 mtime(음수 나이)·비유한·판독 불가는 전부 `recent` = 거부(fail-closed).
pub const TRASH_RECENT_WINDOW_SECS: u64 = 600;
/// 미리보기 확인(confirm) 레코드의 수명 — **단조 시계**로 잰다(벽시계 점프로 늘어나지 않는다).
/// 같음(=TTL 정확히 도달)은 **만료**다.
pub const TRASH_CONFIRM_TTL_SECS: u64 = 300;
/// 인증기 노드 상한(부서장 D2 채택값). 초과 = 인증 불가 = 그 항목 거부.
pub const TRASH_CERTIFY_MAX_NODES: usize = 5_000_000;
/// 인증기 시간 상한(초 · 부서장 D2 채택값). 초과 = 인증 불가 = 그 항목 거부.
pub const TRASH_CERTIFY_MAX_SECS: u64 = 60;
/// 제거가 길어질 때 **보고**만 하는 임계(초). 중단이 아니다(부서장 D2: report, never abort).
pub const TRASH_REMOVE_STALL_REPORT_SECS: u64 = 120;

/// ★MJR-1 — 예산을 **자식 루프 안에서** 다시 보는 간격(노드 수).
/// 실측 근거(`S10-PACING-MEASUREMENT.md` · debug/release 두 프로파일): 노드당 3,368 ns(release) ·
/// `Instant::now()` 34.9 ns ⇒ 128 노드마다면 초과분 상한이 **0.43 ms**(항목 예산 60 s 의 0.0007 %)이고
/// 검사 비용은 **0.008 %** 다. 느린 파일계(노드당 100 µs 가정)에서도 초과분이 12.8 ms 로 묶인다.
/// ───────────────────────────────────────────────────────────────────────────────────────────
/// ★★**1-b 정본 불변식 — 「측정된 시간 상한 T」 형식**(T-1B-FIX · 부서장 판정 S33 §2 · ⛔점근 등급
/// 단독 표기 폐기). 왜 형식을 바꿨나: 세 라운드에서 **등급으로 적을 때마다 그 등급 밖의 선형 항 하나가
/// 남았다**(열거 → 정렬 → **프런티어 재할당 이동** → 해제). 등급은 남는 항을 숨기고, 시간은 숨기지 못한다.
///
/// > **마감을 넘긴 순회의 초과(overshoot)는 — 순회가 돌아오기까지 — 노드 상한에서 측정한
/// > `T_walk ≈ 67.21 ms` 이내다** — ★**단, 이 문장은 자기 단서를 품는다**(아래 두 줄에 기대지 않는다):
/// > 이 값은 **debug 빌드 · 이 기계 · 네 항의 측정을 더한 값**이고, 그중 **ⓐ는 잰 값을 다른 차림으로
/// > 늘린 외삽** · **ⓑⓒ는 합성 프런티어에서 쟀고** · **ⓓ는 소음 지배**다 ⇒ ★**근거이지 보장이 아니며
/// > 최악의 증명도 아니다.** ⛔또 **디스크 지연(fsync) 한 항은 T 에 들어가지 않는다**(아래 ④′).
///
/// **① 기여 항 전수**(★**넷이 전부이고 넷 다 잰 값에서 나왔다** — ⛔「**T 안에 추정은 없다**」가 뜻하는
/// 것은 **「재지 않고 어림한 항이 없다」** 하나다. ★**그 말을 상한의 보증으로 읽지 마라**: ⓐ는
/// **잰 값을 다른 차림으로 늘린 외삽**이고 외삽은 추정과 달리 **측정에 뿌리가 있지만 최악을 증명하지
/// 못한다** — 그래서 아래 네 번째 칸이 **각 줄에서** 그 성격을 말한다 ⛔라벨을 한계 절로 미루지 않는다):
/// | 항 | 측정값 | 어디서 어떻게 재었나 | ★이 숫자는 최악의 증명인가 |
/// |---|---|---|---|
/// | ⓐ다음 검사까지 남은 자식 **≤128** | **817.28 µs** | 자식 단가 **6.385 µs**(자식 16,000 디렉터리를 `certify_entry` 로 5회 순회한 **최악**) × 128 | ⚠**아니다 — 외삽**: 단가를 **자식 16,000개 한 디렉터리**에서 쟀고 5,000,000 노드 트리에서 재지 않았다 |
/// | ⓑ프런티어 `push` **한 번의 최악**(재할당 이동 포함) | **130.792 µs** | **F = 5,000,000** 까지 `Vec<(PathBuf, usize)>` 에 5,000,000회 push 하며 **매 push 를 재서 최악 1회** | ✅상한에서 **직접** · ⚠단 프런티어가 **길이 69자 경로 하나를 복제한 합성**이다 ⇒ **최악의 증명은 아니다** |
/// | ⓒ조기 거부 경로의 **프런티어 해제** | **66.215959 ms** | **F = 5,000,000** 프런티어를 `drop` 한 시간 | ✅상한에서 **직접** · ⚠같은 합성 한계 ⇒ **최악의 증명은 아니다** |
/// | ⓓ거부 레코드 생성(`format!`+`String`)·`ReadDir` 핸들 닫기 | **41.209 µs** = 레코드 **19.709 µs** + 핸들 닫기 **21.5 µs** | 임시 프로브(제거) · 레코드 200,000회 중 최악 · 핸들은 **순회 도중**(한 항목 꺼낸 뒤) 드롭 20,000회 중 최악 · **두 번 돌려 큰 쪽**(규칙을 돌리기 전에 잠갔다) | ⚠**아니다 — 소음 지배**: 두 런의 레코드 최악이 **4 µs ↔ 19.709 µs**(약 5배)로 갈렸다 ⇒ ⓑ와 같은 한계(할당기·페이지 폴트·스케줄 소음) · 경로는 길이 123자 **합성** 하나 |
/// ⇒ **T_walk = ⓐ+ⓑ+ⓒ+ⓓ = 817.28 + 130.792 + 66,215.959 + 41.209 µs ≈ 67.21 ms**.
/// ★**ⓒ가 지배한다**(ⓑ의 약 506배 · ⓓ의 약 1,607배). ★**T 는 네 항의 합이고 ⛔최악의 증명이 아니다**
/// (위 네 번째 칸의 ⚠ 넷이 그 이유다 — ⓓ를 더해도 그 성격은 바뀌지 않는다).
/// ★**왜 ⓓ가 한 번인가**: 순회는 조기 거부로 **한 번** 빠지므로 레코드도 하나이고, 열린 `ReadDir` 핸들은
/// **언제나 한 개**다(하위 디렉터리는 프런티어에 쌓고 이 루프가 끝난 뒤에 열기 때문이다 · 아래 ⒟′-7 주석).
///
/// **② 측정 차림**: **debug 빌드** · 이 기계(Darwin 27.0.0) · **임시 프로브**(측정 뒤 **제거** — 트리에
/// 없다 · ⓐⓑⓒ 는 프로브 하나로 1런, **ⓓ 는 별 프로브를 2런**) · 반복은 ⓐ 5회 중 최악 ·
/// ⓑ 5,000,000회 중 최악 · ⓒ 1회 · ⓓ 레코드 200,000회·핸들 20,000회 중 최악을 **2런에서 큰 쪽**
/// (★그 규칙을 **돌리기 전에** 잠갔다 · 대조: 핸들을 꺼내지 않고 드롭하면 **9.208 µs** 로 더 싸므로
/// 생산 꼴인 **도중 드롭**을 썼다). ⛔**release 미측정**이고 「release 는 더 빠를
/// 것」은 **추정이므로 적지 않는다**. ⛔이 수치를 같은 파일의 **release 수치**(S10 의 노드당 3,368 ns)와
/// **더하지 않는다** — 저울이 다르면 합이 거짓이 된다.
///
/// **③ 의존 전제**(이것이 깨지면 T 도 깨진다):
/// ⒜**`F ≤ TRASH_CERTIFY_MAX_NODES`(5,000,000)** — 프런티어에는 **디렉터리만** 쌓이고 디렉터리 수 ≤ 방문
///   노드 수이며 그 상한이 노드 상한이다 ⇒ ⓑⓒ를 **상한에서 바로** 잰 것이 그래서 유효하다.
/// ⒝**검사 사이에 입력 크기에 비례하는 작업이 없다**(S29 ⒟′-7 의 「모으지 않고 흐른다」 · 정렬 제거).
/// ⒞**`Instant::now()` 가 간격마다 한 번만** 불린다.
/// ⒟**호출 경로**: 이 상한은 `certify_entry_until`(배경 미리보기 잡)에서 참이다. 동기 호출자는 **없다**
///   (`trash.preview` 동사는 `72db4501` 에서 퇴역).
/// ⒠**메모리 압박이 없다** — 스왑이 걸리면 ⓒ는 **측정값보다 훨씬 커질 수 있다**(⛔미측정).
/// ⒡**각 항의 한계는 ①표의 네 번째 칸에 그 숫자 옆으로 적었다**(이 절은 그 되읽기다 ⛔라벨의 집은 여기가
///   아니다): ⓐ는 **외삽**(단가를 자식 16,000개 한 디렉터리에서 쟀다) · ⓑⓒ는 상한에서 직접이지만
///   **길이 69자 경로 하나를 복제한 합성 프런티어**이고 실제 경로는 길이가 다양하다 · ⓓ는 **소음 지배**
///   (2런의 레코드 최악이 4 µs ↔ 19.709 µs). ⇒ **네 항 모두 최악의 증명이 아니다.**
///
/// **④ 예산에 대한 비율**: 항목 예산 `TRASH_CERTIFY_MAX_SECS = 60 s` 의 **0.112 %** ·
/// 세션 예산 `TRASH_PREVIEW_SESSION_MAX_SECS = 120 s` 의 **0.056 %**.
/// **④′ ⛔T 밖에 있는 항 하나 — 무계다**: 순회가 거부로 돌아온 **뒤** 호출자가 그 거부를 **원장에 적고
/// `fsync`** 한다(최대 1건). **디스크 지연은 어떤 측정으로도 묶이지 않으므로** 그것은 T 에 넣지 않고
/// **분리해 적는다** ⇒ ★**「잡이 답을 주기까지」의 상한은 유계가 아니다**. T 가 묶는 것은 **작업 시간**이다.
///
/// **⑤ 구조 개선을 쓰지 않은 근거(측정이 결정했다)**: push 가 기존 항목을 **옮기지 않는** 묶음 프런티어
/// (`Vec<Vec<…>>` · 묶음 1024)를 **같은 프로브에서 함께 재 봤다** — 최악 1회 push **119.959 µs**(평범한
/// `Vec` 은 130.792 µs) · 해제 **67.666625 ms**(평범한 `Vec` 은 66.215959 ms) ⇒ ★**T 를 낮추지 못한다**
/// (지배 항이 **해제**이고, 최악 push 는 둘 다 **120~200 µs 대**로 재할당 이동이 아니라 할당기·페이지
/// 폴트·스케줄 소음에 묶여 있다 — 평범한 `Vec` 의 최악이 `i=65,536` 에서, 묶음의 최악이 `i=2,800,089`
/// 에서 났다는 것이 그 증거다). ⇒ **쓰지 않았다**: 코드를 늘리고 **T 를 바꾸지 않는** 변경이기 때문이다.
/// ⛔**새 사용자 상한을 발명하지 않았다**(동작 변경 0).
/// ───────────────────────────────────────────────────────────────────────────────────────────
/// ★**이 간격(128)이 묶는 것**: 종전에는 **방문(lstat+요약)** 만 묶었고 열거·정렬은 간격 밖이었다.
/// 지금은 **열거와 방문이 한 루프**라 이 간격이 **둘 다** 묶는다 — 자식 하나당 일은 `read_dir` 한 걸음 +
/// `lstat` + 노드 요약 sha256 + 선택적 프런티어 push **각각 상수 횟수**다(그래서 ⓐ가 **자식 단가 × 128** 로
/// 재진다). **K=128 을 그대로 쓴 근거**(새 상수를 만들지 않았다): 열거 단가 실측 **항목당 0.38~0.83 µs**
/// (한계단가 0.46 µs · W=32,000)이고 정렬 항은 **0** 이 됐다(제거) ⇒ 열거 쪽 간격이 약 **59 µs** 다.
pub const TRASH_CERTIFY_BUDGET_CHECK_NODES: usize = 128;
/// ★B-162 — 배경 순회가 디스패치에 **양보하는 간격(ms)**.
/// ⚠선행 설계는 「1 ms / 5,000 inode」라 적었다. 양보 길이는 승계하고 **트리거는 시간으로 바꿨다**:
/// 노드 수는 입력 공백의 상한이 **아니다**(같은 5,000 노드가 release 16.8 ms · debug 41.9 ms 이고
/// 느린 파일계에서는 수백 ms 가 된다 — ★보호가 가장 필요한 상황에서 노드 트리거가 보호를 놓는다).
/// 실측 근거: `sleep(1ms)` 의 실제 길이는 1.498 ms(avg) · 1.527 ms(max) ⇒ 20 ms 간격이면 처리량
/// 비용 **7.0 %**, 입력 공백 상한 ≈ **21.7 ms**(결박선 50 ms 의 2.3배 여유). release 에서 5,000 노드가
/// 16.8 ms 이므로 **동결 제안값과 같은 자리수**다(값을 깎은 것이 아니라 속도 독립 좌표로 다시 썼다).
pub const TRASH_CERTIFY_YIELD_EVERY_MILLIS: u64 = 20;
/// 한 번의 양보 길이(ms · 요청값) — 동결 설계의 1 ms 를 **승계**한다(실제로는 약 1.5 ms 다 · 위 주석).
pub const TRASH_CERTIFY_YIELD_MILLIS: u64 = 1;

/// ★미리보기 **세션** 상한(초) — 다수 항목을 한 잡에서 인증할 때의 전체 예산(델타 §1.3).
/// ★`TRASH_REMOVE_STALL_REPORT_SECS` 와 **숫자가 같지만 다른 것**이다: 그것은 제거가 길어질 때
///   **보고만** 하는 임계(중단 아님)이고, 이것은 **중단시키는 예산**이다. 숫자가 같다는 이유로
///   두 상수를 합치지 않는다(이름이 닮은 상수를 합치면 한쪽의 의미가 조용히 사라진다).
/// ★출처 구분: 세션 120 s = `DESIGN-S8.md:108` @ `68db0e9e…` · 항목별 60 s/5,000,000 =
///   이 파일의 두 상수(코드가 정본이고 그 문서의 20 s/2,000,000 과 **다르다**).
pub const TRASH_PREVIEW_SESSION_MAX_SECS: u64 = 120;
/// 미조회 만료(초) — 마지막 폴링 뒤 이만큼 아무도 묻지 않으면 잡을 버리고 빗장을 놓는다.
pub const TRASH_PREVIEW_JOB_TTL_SECS: u64 = 120;

/// ★⒟-2(판정문 S17 §4 · 교정 뒤에도 **그대로 유지된 지시**) — 크기 캐시를 **읽는 양의 상한**(바이트).
/// 초과하면 캐시를 **무시하고 정상 경로로 간다**(⛔거부로 바꾸지 않는다 — 캐시는 **선택적 가속**이고,
/// 가속이 없으면 느릴 뿐 **틀리지 않는다**: 목록은 그 항목을 `size.state = "pending"` 으로 보여준다).
///
/// 왜 상한이 필요한가: `size_cache_read` 는 종전에 **읽는 양에 상한이 없었다**. 쓰는 쪽은 64항목으로
/// 자르므로(`size_cache_flush`) **자기 손으로는** 커지지 않지만, 그 파일은 휴지통 뿌리의
/// `size-cache.json` 이고 **같은 UID 의 아무 프로세스나 키울 수 있다**. 그리고 그 파일을 읽는 자리는
/// 둘인데 하나가 **오너 전용이 아닌** `trash.list`(`list_payload`)다 ⇒ 상한이 없으면 **감사자도 쓰는
/// 목록 응답**이 파일 크기에 비례해 무계로 늘어난다.
/// ⛔**사실 정정(S40 §4 · 핀 `d6ce4660` 의 이 자리는 틀렸다)**: 옛 문면은 「검증 좌석 실측: 19.8 MB 에서
/// **1.215 s**」라 적어 그 값을 **`list` 응답 시간**으로 인용했다. 그 값은 `list` 가 아니라
/// **`certify_names` 총시간**이다 — S13 프로브 `PROBE-1B` 의 `total=1.215433291s`(`S13:422`)이고 그 프로브가
/// 부른 것은 `certify_names(…)`(`S13:495`)인 **미리보기 인증 경로**다(`list_payload` 를 재지 않았다 ·
/// `S38-MINOR-PROSE-ADDENDA.md` A-7). **`list` 가 같은 캐시 파일을 읽는다는 사실은 맞다**(`list_payload`
/// → `size_cache_read`) — 그래서 상한이 필요하다는 **결론은 그대로**이고, 틀린 것은 **근거로 든 숫자의
/// 정체**다. 큰 캐시를 읽는 **읽기 비용**은 바로 아래 줄들이 직접 쟀다.
///
/// ★값의 실측 근거(이 기계 · debug 빌드 · 각 크기 5회 중 **최악** · S18 임시 프로브 출력 그대로):
/// `10,349 B`(= 우리 쓰기 상한인 64항목의 크기) **451.292 µs** · `163,781 B` **5.278958 ms** ·
/// `246,781 B` **7.858792 ms** · `263,381 B` **8.371083 ms** · `329,781 B` **10.264125 ms** ·
/// `1,159,781 B` **38.001291 ms** · `4,681,781 B` **156.051792 ms** · `21,537,781 B` **727.17 ms**
/// ⇒ 바이트당 약 **33.8 ns** 로 선형이다.
/// ⛔**사실 정정(S40 §4)**: 옛 문면은 여기에 「= 검증 좌석의 **19.8 MB/1.215 s** 와 같은 자리수」를 붙여
/// 대조군으로 썼다. **그 대조는 성립하지 않는다** — 1.215 s 는 `certify_names` **총시간**(순회를 포함한다)
/// 이고 위 수치는 `size_cache_read` **한 번의 읽기**다. 저울이 다른 값을 자리수 대조로 쓰면 선형성 주장이
/// 받쳐지지 않으므로 **그 대조를 뺐다**. 선형성은 위 **여덟 점의 측정**만으로 선다.
/// ★그래서 **256 KiB** 다: ⒜우리가 쓰는 크기(10,349 B)의 **약 25배**라 **자기 손으로는 상한을 넘길 수
/// 없다**(64항목이 평균 4 KB 가 되어야 하는데 한 항목은 약 160 B 다 — 즉 정상 동작에서 이 상한은
/// **발화하지 않는다**) ⒝상한 바로 위 크기(263,381 B)의 최악 읽기가 **8.371083 ms** 로 측정됐으므로
/// 캐시 때문에 `list`·미리보기가 멈추는 시간이 **약 8 ms 로 묶인다**.
/// ⛔**시간 상한(ms)으로 잠그지 않은 이유**: 디스크 지연은 무계라 어떤 ms 단언도 거짓이 된다 —
/// 그래서 묶는 것은 **읽는 바이트 수**이고, 위 수치는 그 선택의 **근거**이지 보장이 아니다.
/// ★초과 뒤의 자기 치유(부수효과 · 목적이 아니다): 상한을 넘긴 파일은 다음 `size_cache_flush` 가
/// **작은 파일로 덮는다**(읽기가 빈 맵을 주므로). 그 파일은 우리 산출물이라 덮는 것이 맞다.
///
/// ★**⒟′-2 개정 문면(S20A §1 · codex R1 M1 수용)** — 셋이 **의무**다:
/// ⒜상한은 **바이트**다 ⛔**항목 수만의 상한은 금지**한다(항목 하나가 커도 읽는 양이 무계가 된다) ·
/// ⒝검사는 **읽기 전에** 파일 크기로 한다 · ⒞검사 자리는 **유일한 독자 `size_cache_read` 안**이다 —
/// ★그래야 **오너 전용이 아닌 `list_payload`(비오너 목록)** 를 포함한 **모든 호출자**가 같은 상한 뒤에 선다
/// (호출자 둘: `size_cache_flush` · `list_payload` · 전수 확인). 초과 = **캐시 부재와 동일**(빈 맵).
/// ⛔**거부로 바꾸지 않는다.**
///
/// ⛔★**범위 — 이 상한은 「양」을 묶고 「기다림」을 묶지 않는다**(S20A §2 · codex R1 의 새 질문):
/// 읽는 **바이트 수**는 묶이지만 **첫 바이트·EOF 를 기다리는 시간**은 묶이지 않는다 ⇒
/// ⛔**「목록 응답성이 유계다」를 주장하지 않는다.** 그중 **FIFO·특수 파일**은 아래 `size_cache_read` 에서
/// **추가 I/O 0 으로 닫았다**(같은 `metadata` 자리에서 파일 종류를 본다 — **syscall 을 더 부르지 않는다**). **비정상적으로 느린
/// 파일시스템의 기다림**은 이 상한으로 닫히지 않는다 — 그것은 **마감 없는 블로킹 시스템콜** 계열이고
/// **A1(S22 설계 라운드)의 세 번째 노출면**으로 올라갔다(⛔내 범위가 아니다).
pub const TRASH_SIZE_CACHE_MAX_BYTES: u64 = 256 * 1024;

/// ★⒟′-6(부서장 판정 2026-10-14 · S20 §2 불변식의 **뒤 항**을 유계로 만든다) — 한 번의 미리보기가 받는
/// **이름 수 상한**. 초과는 **거부**이고(⛔거짓이 아니다 · 잡을 띄우지 않고 즉시 답한다) 메시지가 **나눠 고르라**고 말한다.
///
/// ★**측정 차림(수용 조건 ⒤ · 아래 모든 µs·ms 에 적용)**: **debug 빌드 · 이 기계 · 임시 프로브 1회 ·
/// 한계단가는 n=1024**. ⛔**release 는 미측정**이다(더 빠를 것이라는 추정은 측정이 아니다) ⇒ 이 수치는
/// **상한 값 선택의 근거**이고 **보장이 아니다**. 묶는 것은 시간이 아니라 **개수**다.
///
/// 왜 필요한가 — **예산에 얹은 유계가 유계가 아니었다**(내 실측 · **debug** · 이 기계 · **release 미측정**):
/// 「마지막 마감 검사 → `Ready` 발행」 구간의 단가는 **인증된 이름 하나당 5.12 µs**(n=1024 한계단가 ·
/// `order_digest` 의 이름별 sha256 + 확인 레코드의 키 사본·정렬 + `json!` 의 `to_value` **깊은 복사**)와
/// **거부된 이름 하나당 1.70 µs** 다. 모집단의 크기는 **세션 예산**(120 s)에만 얹혀 있었는데, 인증 루프의
/// 단가가 **빈 디렉터리에서 26.4 µs/건**(가장 싼 인증)이므로 예산 안에 **약 4.55 M건**이 들어가고
/// ⇒ 파생 상한이 **약 23.3 초**(= 예산의 약 19 %)였다. ★**유계가 다른 유계에 얹혀 있고, 그 파생값이
/// 예산과 같은 자리수면 그것은 보장이 아니다.** 그래서 **선언된 상한**을 둔다.
///
/// ★값: **512**. ⒜실측 모집단의 약 **85배** 다(B-064: 휴지통 뿌리 자식 **6개** · 오너는 그 목록에서만 고른다)
/// ⇒ **정상 동작에서 발화하지 않는다** ⒝이 상한 아래에서 구간의 **총량**은
/// **≤ 512 × 5.12 µs ≈ 2.6 ms**(거부 모집단이면 ≈ 0.87 ms)다 — ⛔이 수치는 **근거이지 보장이 아니다**
/// (**debug 측정 · release 미측정** · 이 기계 · 디스크 지연은 무계).
/// ⛔**시간으로 잠그지 않았다**(ms 단언은 거짓이 된다) · ⛔**예산으로 잠그지 않았다**(그것이 위 23.3 초다).
///
/// ★남는 의존: 이 상한은 **RPC 인자 관문**(`names_param`)에 있다 ⇒ `certify_names` 를 **그 관문을 거치지 않고**
/// 부르는 호출자가 생기면(예: 새 동기 동사) 이 항은 **다시 유계가 아니다**. 지금 생산 호출자는
/// `preview_start` 하나이고(동기 동사는 `72db4501` 에서 퇴역), `trash.delete` 의 이름 집합은
/// **확인 레코드와 같아야** 하므로(`asked != c.names` 거부) 그 경로도 이 상한 아래에 있다.
///
/// ⛔★**그 「호출자 전수」는 기계로 강제되지 않는다 — written convention 으로 명시 강등한다**(수용 조건 ⒦).
/// **기계가 받치는 것**: 관문의 **행위** — 상한 초과는 거부이고 **잡 0 · 빗장 free · 원장 미기록**이며
/// **상한 그 자체는 통과**한다(검체 `a_preview_of_more_names_than_the_cap_is_refused_without_starting_a_job`).
/// **기계가 받치지 않는 것**: 「그 관문을 **거치지 않는 호출자가 없다**」는 사실. 그것은 **선언**이다.
/// ⛔**소스를 런타임에 읽는 전수 핀을 새로 만들지 않았다** — 다른 좌석의 편집이 내 모듈에서 붉어지는 선례가
/// 이 레인에 있고(이 파일에 이미 그런 핀이 셋 있다), ★**구멍 난 핀은 확신으로 읽히므로 선언보다 나쁘다**
/// (선례 T-S3-OBL). ⇒ 새 호출자를 만드는 사람이 이 줄을 읽고 **관문을 통과하게** 만들 책임을 진다.
pub const TRASH_PREVIEW_MAX_NAMES: usize = 512;

/// 봉인 접두 — 삭제 명령을 받은 항목은 먼저 이 이름으로 **봉인**된다(옛 이름으로 계속 쓰는 생산자가
/// 봉인된 객체에 자동으로 닿지 못하게). 봉인 뒤에는 되돌리지 않는다(실패하면 봉인 상태로 남고
/// 자동 재개는 없다).
pub const TRASH_SEAL_PREFIX: &str = ".deleting-";
/// 오래됨 표시 파일 이름(D-1) — reap 은 더 이상 지우지 않고 이 표식만 남긴다.
pub const TRASH_OLD_MARKER: &str = ".trash-old";
/// 삭제 원장(D-4) — 의도·거부·결과가 전부 여기로 간다.
pub const DELETE_LEDGER: &str = "DELETE-LEDGER.jsonl";

// ─── 거부 사유 코드(원장·화면이 같은 문자열을 쓴다) ──────────────────────────────────────────
pub const R_DESC_DEVICE: &str = "uncertifiable-descendant-device";
pub const R_UNREADABLE: &str = "uncertifiable-unreadable";
pub const R_TOO_MANY: &str = "uncertifiable-too-many-nodes";
pub const R_TOO_SLOW: &str = "uncertifiable-too-slow";
pub const R_RECENT: &str = "recent_may_be_in_use";
pub const R_CONFIRM_EXPIRED: &str = "confirm_expired";
pub const R_NOT_IN_PREVIEW: &str = "names_not_in_preview";
pub const R_CHANGED: &str = "entry_changed_since_preview";
pub const R_LEDGER_UNWRITABLE: &str = "refusal_record_unwritable";
pub const R_SEAL_FAILED: &str = "seal_failed";
pub const R_REMOVE_ERROR: &str = "removal_error";
pub const R_NOT_OWNER: &str = "owner_gui_required";
pub const R_NOT_BASE: &str = "base_daemon_only";
pub const R_PRODUCER_ACTIVE: &str = "producer_active";
/// ★S8 재착륙 §1 — 비동기 미리보기 잡의 세 코드.
/// ★표기: 델타는 산문에서 `refused:` 접두를 붙여 쓰지만, 이 모듈의 기존 코드는 접두 없는 **맨 코드**다
///   (`R_PRODUCER_ACTIVE` = `producer_active` ↔ 델타 `refused:producer_active`). **기존 관례를 따른다.**
/// 다수 항목 미리보기의 **세션** 상한 초과 — 항목별 상한(`R_TOO_SLOW`)과 **다른 것**이다.
pub const R_SESSION_TOO_SLOW: &str = "cannot_certify_in_time";
/// 단일 플라이트: 이미 도는 미리보기 잡이 있다(기다리지 않고 즉시 거부한다 — 대기가 곧 stall 이다).
pub const R_PREVIEW_BUSY: &str = "preview_busy";
/// 그 `job_id` 를 이 데몬이 모른다(만료됐거나 데몬이 다시 떴다).
pub const R_JOB_UNKNOWN: &str = "job_unknown";
/// ★MJR-2 — 미리보기 **일꾼(배경 스레드)** 을 띄우지 못했다. 잡은 그 자리에서 `Failed` 로 종결되고
/// 빗장은 **즉시** 풀린다(놓지 않으면 데몬 수명 동안 미리보기가 죽는다 = 삭제의 선행조건이 죽는다).
pub const R_PREVIEW_NO_WORKER: &str = "preview_worker_unavailable";

/// 휴지통 뿌리 — 생산자 셋(cys-dept:94 · javis_org.py:564 · factory_reset.rs:299)과 같은 규약 경로.
pub fn trash_root() -> PathBuf {
    cys::home_dir().join(".local/state/cys-trash")
}

// ─── 한 항목의 인증서 ────────────────────────────────────────────────────────────────────────
/// 인증 성공 — 뿌리 아래 **모든** 노드를 한 순회로 봤고 장치 경계가 없다.
#[derive(Debug, Clone, PartialEq)]
pub struct Certificate {
    /// CD(e) 의 신원 절반 — 항목 뿌리의 (dev, ino).
    pub dev: u64,
    pub ino: u64,
    pub nodes: usize,
    pub bytes: u64,
    pub max_depth: usize,
    /// 가장 **새로운** 노드의 mtime(epoch) — 최근 판정은 뿌리 mtime 이 아니라 이 값으로 한다
    /// (뿌리 mtime 은 깊은 곳의 쓰기를 반영하지 않는다).
    pub newest_mtime: f64,
    /// 봉인 전후 대조용 요약 — (상대경로, 크기, mtime 비트, 장치, inode) 전수의 sha256.
    pub digest: String,
    pub walk_secs: f64,
}

/// 인증 실패 — **항목 전체 거부**다(부분 제거·경고 후 제거·경계까지 제거 전부 금지).
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    /// 인증을 무너뜨린 경계 경로(있으면) — 거부가 읽히게 하는 핵심 정보(C-R2).
    pub boundary: Option<String>,
    pub detail: String,
}

pub type CertifyResult = Result<Certificate, Refusal>;

/// 디스패치에 한 번 양보한다(B-162) — ★셀 수 있는 자리로 떼어 둔다. 양보는 **보이지 않는 동작**이라
/// 함수 안에 숨기면 「배경 순회만 양보한다」를 어떤 검사도 재지 못한다.
fn dispatch_yield() {
    #[cfg(test)]
    YIELD_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::sleep(std::time::Duration::from_millis(TRASH_CERTIFY_YIELD_MILLIS));
}

/// 시험 전용 — 지금까지의 양보 횟수(전역이므로 이 값을 읽는 검체는 직렬화한다).
#[cfg(test)]
static YIELD_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub fn yields_for_test() -> usize {
    YIELD_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}
#[cfg(test)]
pub fn reset_yields_for_test() {
    YIELD_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// 장치 번호를 어디서 읽는가 — **시험용 주입점**(C-R4 known-failure 픽스처).
/// 이 기계에는 휴지통 뿌리 아래 교차장치 디렉터리가 **0개**라(부서장 실측 · 내 재측정도 0) 실물로는
/// 이 경로를 밟을 수 없다. 그래서 장치 읽기를 seam 으로 두고, 시험이 경계에서 다른 값을 주입한다.
pub type DevOf<'a> = &'a dyn Fn(&Path, &std::fs::Metadata) -> u64;

/// 운영 기본값 — 실제 `st_dev`.
#[cfg(unix)]
pub fn real_dev(_p: &Path, md: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    md.dev()
}
/// 윈도우에는 `st_dev` 가 없다 — ★윈도우에서 다르게 도는 자리 1.
/// `volume_serial_number` 는 불안정(nightly)이라 쓰지 않는다. 그래서 윈도우에서는 장치 경계를
/// **증명할 수 없고**, 이 모듈은 증명할 수 없는 것을 성공으로 읽지 않는다 — 윈도우 삭제는
/// 기본 꺼짐이고(feature 로만 켠다) 12.W 가 출하 게이트다.
#[cfg(windows)]
pub fn real_dev(_p: &Path, _md: &std::fs::Metadata) -> u64 {
    0
}

/// 한 항목을 인증한다 — **한 번의 순회**(lstat 전용 · 심링크 미추적).
///
/// 거부 조건(하나라도 걸리면 항목 전체 거부): ①어떤 노드든 장치가 뿌리와 다르다 ②어떤 노드든 lstat·
/// 디렉터리 열기가 실패한다 ③노드 수 상한 초과 ④시간 상한 초과. ★`clock`·`dev_of` 는 주입점이다.
///
/// ★**이 함수의 생산 호출자 셋은 `PREVIEW_LOCK` 바깥이고, 그래도 된다**(부서장 판정 2026-10-13 ·
/// 호출자: 신원 재확인 · 봉인 뒤 대조 · 남은 노드 수 집계 — 전부 **집행 경로**다).
/// 왜 되는가: ⒜그 셋은 **오너의 미리보기 발화가 아니다**(단일 플라이트가 묶는 대상은 미리보기끼리의
/// 동시성이다) ⒝세션 예산 없이도 **항목별 상한**(`TRASH_CERTIFY_MAX_SECS` / `TRASH_CERTIFY_MAX_NODES`)
/// 으로 유계다 ⇒ 무한정 도는 순회가 아니다.
/// ⛔**다음 사람에게**: 이것을 결함으로 읽고 빗장을 그 셋까지 넓히지 마라 — 그러면 **쓸기·봉인이
/// 오너의 미리보기를 기다린다**(막힘을 없애려 만든 장치가 새 막힘을 만든다).
/// ★정확한 범위 문면: 「단일 플라이트는 **미리보기 인증자**를 1로 묶는다. 쓸기·봉인 경로의 항목
/// 인증은 빗장 밖이고, 항목별 상한으로는 유계다」 ⇒ 동시성은 **미리보기끼리 1**이고
/// **미리보기 대 쓸기는 1이 아니다**.
pub fn certify_entry(root: &Path, dev_of: DevOf) -> CertifyResult {
    certify_entry_until(root, dev_of, None)
}

/// `certify_entry` 와 같되 **세션 마감**을 함께 받는다(★S8 재착륙 §1).
///
/// ★왜 별 함수인가: 기존 `certify_entry` 의 호출자가 6자리이고 그 중 검체가 있다 — 서명을 바꾸면
/// 무관한 자리가 함께 붉어진다. **가산(additive)** 으로 두고 기존 함수는 이 함수에 위임한다.
/// ★마감은 **항목별 상한과 별개**다: 항목별은 `TRASH_CERTIFY_MAX_SECS`(이 항목만 거부),
/// 세션 마감은 넘으면 `R_SESSION_TOO_SLOW` 로 거부하고 **잡 전체를 종결**시킨다.
/// 마감 검사는 ⒜**매 디렉터리** 경계 ⒝**자식 루프 안에서 `TRASH_CERTIFY_BUDGET_CHECK_NODES` 노드마다**
/// ⒞**루프를 빠져나온 뒤 한 번 더** 걸린다 ⇒ ★`Ok`(그리고 그 위의 `Ready`)는 **예산 안에서 끝난 것만**
/// 뜻한다(델타 §1.3 「세션 상한 도달 시 진행 중인 순회는 중단되고」 · 동결본 `:103` 「up to the budget」).
/// ⛔⒝⒞ 가 없던 동안에는 자식이 아주 많은 **한** 디렉터리가 예산을 넘겨 끝나고도 `Ok` 였다(MJR-1).
/// ★`session_deadline` 이 `Some` 인 순회 ⇔ **배경 잡의 순회**다(동기·제거 경로는 전부 `None` 이다).
/// 그래서 B-162 의 디스패치 양보도 그 조건에 걸린다 — 동기 경로에서 자면 stall 을 **늘린다**.
pub fn certify_entry_until(
    root: &Path,
    dev_of: DevOf,
    session_deadline: Option<Instant>,
) -> CertifyResult {
    certify_entry_until_inner(root, dev_of, session_deadline, #[cfg(test)] None)
}

fn certify_entry_until_inner(
    root: &Path,
    dev_of: DevOf,
    session_deadline: Option<Instant>,
    #[cfg(test)] probe: Option<&tests::CertifyProbe>,
) -> CertifyResult {
    let now = || {
        #[cfg(test)]
        if let Some(p) = probe { return p.now(); }
        Instant::now()
    };
    let t0 = now();
    let root_md = std::fs::symlink_metadata(root).map_err(|e| Refusal {
        code: R_UNREADABLE,
        boundary: Some(root.display().to_string()),
        detail: format!("뿌리 lstat 실패: {e}"),
    })?;
    if !root_md.is_dir() {
        return Err(Refusal {
            code: R_UNREADABLE,
            boundary: Some(root.display().to_string()),
            detail: "항목 뿌리가 디렉터리가 아니다".into(),
        });
    }
    let root_dev = dev_of(root, &root_md);
    let mut nodes = 0usize;
    let mut bytes = 0u64;
    let mut max_depth = 0usize;
    let mut newest = mtime_secs(&root_md);
    // ★R2 D-r2-3: set_digest 는 **순서 무관 128비트 랩어라운드 합**이다(노드 레코드의 sha256 앞 16바이트).
    //   순서에 의존하지 않으므로 순회 순서가 바뀌어도 같은 집합이면 같은 값이다.
    let mut set_digest: u128 = 0;
    // ★⒟′-7(S29 · 관문 codex major 1) — **순회 순서는 더 이상 정렬되지 않는다**(`read_dir` 순서 그대로다).
    //   왜 바꿨나: 종전에는 디렉터리를 **전수 열거해 `Vec` 에 모으고 정렬한 뒤** 자식 루프로 들어갔고,
    //   그 열거·정렬 구간에 **시간 검사가 하나도 없었다** ⇒ 마지막 마감 검사와 다음 검사 사이에
    //   **디렉터리 폭 W** 가 들어갔다(**폐기된 결함의 서술이고 불변식이 아니다** · `O(W log W)` ·
    //   실측 열거 **항목당 0.38~0.83 µs** · 정렬 **항목당
    //   0.84~1.05 µs** · **debug 측정 · release 미측정**). ⇒ 열거와 방문을 **한 루프**로 합쳐 검사 간격
    //   (`TRASH_CERTIFY_BUDGET_CHECK_NODES`)이 그 일 전체를 묶게 했다.
    //   ★**무엇이 바뀌지 않는가(인증서는 그대로다)**: `set_digest` 는 **순서 무관 합**이고(위 주석),
    //   `nodes`·`bytes`·`max_depth`·`newest_mtime` 도 순서에 무관하다 ⇒ **봉인 전후 대조의 기준값이
    //   바뀌지 않는다**(안전 축 무접촉).
    //   ★**무엇이 바뀌는가(정직하게 적는다)**: ⒜순회 순서가 **파일계가 주는 순서**라 **재현되지 않는다**
    //   ⒝그래서 순회 중간에서 끊긴 거부의 `boundary` 에 실리는 **자식 이름이 실행마다 다를 수 있다**
    //   (detail 은 「디렉터리 **안에서** 중단했다」로 그대로 말한다) ⒞기존 검체 기대값은 **그대로 선다**:
    //   `a_budget_that_runs_out_inside_one_directory_refuses_instead_of_certifying` 은 「경계가 **항목 뿌리가
    //   아닌 자식**」만 보고 **특정 이름을 보지 않는다**(그 검체를 읽어 확인했다) ·
    //   `cr4_descendant_device_refuses_the_whole_entry_and_writes_a_refusal_record` 의 경계는 **유일한
    //   교차장치 경로**라 순서와 무관하다. ⇒ 둘 다 **순서 변경에 하중이 없다**(수리 뒤 전수 초록으로 확인).
    //   ★자기정정: 이 문단의 첫 판에 **존재하지 않는 검체 이름**을 적었다(`an_early_refusal_names_the_child_
    //   it_stopped_on`). 지어낸 인용은 다음 사람이 사실로 읽으므로 실재 이름으로 고쳤다.
    //   ⛔조용히 지우지 않았다 — 이 문단이 그 처분의 기록이다.
    // ★**프런티어는 평범한 `Vec` 이다 — 그 선택을 측정으로 변호한다**(T-1B-FIX · 관문 R2 codex major).
    //   사실: `push` 가 재할당을 일으키면 기존 F 개가 **이동한다**(amortized 는 상한이 아니다) ⇒ 그 항이
    //   마감 검사 간격 안에 있다. **그래서 등급으로 적지 않고 재서 적었다**: F=5,000,000 에서 **최악 1회
    //   push = 130.792 µs** 이고, 그 간격의 **지배 항은 그것이 아니라 조기 거부 경로의 해제(66.215959 ms)**
    //   다(약 506배). push 가 기존 항목을 **옮기지 않는** 묶음 프런티어를 같은 프로브에서 함께 재 봤고
    //   **T 를 낮추지 못했다**(최악 push 119.959 µs · 해제 67.666625 ms) ⇒ **바꾸지 않았다**.
    //   전수·차림·전제는 `TRASH_CERTIFY_BUDGET_CHECK_NODES` 머리의 **1-b 정본 불변식** 블록에 있다.
    //   ⛔`with_capacity`·`reserve` 도 쓰지 않았다: F 를 미리 알 수 없고, 큰 예약은 **모든 미리보기가**
    //   쓰지 않을 메모리를 잡는다(흔한 입력은 자식 한 자리 수다 — B-064 실측 6개).
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    // ★B-162: 마지막으로 디스패치에 양보한 시각(간격은 노드 수가 아니라 **시간**이다 · 상수 주석 참조).
    let mut last_yield = t0;
    while let Some((dir, depth)) = stack.pop() {
        #[cfg(test)]
        if let Some(p) = probe { p.head_checks.set(p.head_checks.get() + 1); p.head_before_expiry.set(p.now() < p.deadline()); }
        // ★세션 마감 먼저 — 항목별 상한보다 상위다(넘으면 잡 전체가 끝난다).
        if let Some(dl) = session_deadline {
            if now() >= dl {
                return Err(Refusal {
                    code: R_SESSION_TOO_SLOW,
                    boundary: Some(dir.display().to_string()),
                    detail: format!(
                        "미리보기 세션 상한 {TRASH_PREVIEW_SESSION_MAX_SECS}s 초과 — 순회를 중단했다(이 항목은 삭제 대상이 아니다)"
                    ),
                });
            }
        }
        if now().duration_since(t0).as_secs() > TRASH_CERTIFY_MAX_SECS {
            return Err(Refusal {
                code: R_TOO_SLOW,
                boundary: Some(dir.display().to_string()),
                detail: format!("인증 시간 상한 {TRASH_CERTIFY_MAX_SECS}s 초과 — 이 항목은 삭제 대상이 아니다"),
            });
        }
        let rd = std::fs::read_dir(&dir).map_err(|e| Refusal {
            code: R_UNREADABLE,
            boundary: Some(dir.display().to_string()),
            detail: format!("디렉터리 열기 실패: {e}"),
        })?;
        // ★⒟′-7 — **모으지 않고 흐른다**: 한 디렉터리의 자식을 `Vec` 에 쌓고 정렬한 뒤 방문하던 것을
        //   **열거하면서 바로 방문**하도록 합쳤다. 효과 셋: ⒜검사 간격이 **열거까지 포함해** 묶는다
        //   (마지막 검사와 다음 검사 사이의 일 = **자식 K개**이고 **폭 W 와 무관**하다) ⒝폭 W 크기의
        //   할당이 사라진다(그래서 조기 거부 경로에서 **해제할 `kids` 가 없다**) ⒞열린 `ReadDir` 핸들은
        //   **언제나 한 개**다(하위 디렉터리는 프런티어에 쌓고 이 루프가 끝난 뒤에 열기 때문이다).
        #[cfg(test)]
        let rd = rd.inspect(|ent| {
            if ent.is_ok() { if let Some(p) = probe { p.pulls.set(p.pulls.get() + 1); } }
        });
        for ent in rd {
            let ent = ent.map_err(|e| Refusal {
                code: R_UNREADABLE,
                boundary: Some(dir.display().to_string()),
                detail: format!("디렉터리 항목 읽기 실패: {e}"),
            })?;
            let p = ent.path();
            nodes += 1;
            #[cfg(test)]
            if let Some(p) = probe { p.visits.set(p.visits.get() + 1); }
            if nodes > TRASH_CERTIFY_MAX_NODES {
                return Err(Refusal {
                    code: R_TOO_MANY,
                    boundary: Some(p.display().to_string()),
                    detail: format!("노드 상한 {TRASH_CERTIFY_MAX_NODES} 초과 — 이 항목은 삭제 대상이 아니다"),
                });
            }
            // ★MJR-1 — 예산을 **자식 루프 안에서도** 본다. 시간 검사가 디렉터리 루프 머리에만 있으면
            //   자식이 아주 많은 **한** 디렉터리가 예산을 넘겨 끝나고도 `Ok` → `Done` → `Ready` 가 된다
            //   (동결본 `DESIGN-S8.md:103` 의 「up to the budget」이 최악에서 깨진다).
            //   ★`now()` 는 **한 번만** 부르고 세 판정이 그 값을 나눠 쓴다(단가를 두 번 내지 않는다).
            if nodes % TRASH_CERTIFY_BUDGET_CHECK_NODES == 0 {
                let now_i = now();
                if let Some(dl) = session_deadline {
                    if now_i >= dl {
                        #[cfg(test)]
                        if let Some(p) = probe { p.child_refusals.set(p.child_refusals.get() + 1); }
                        return Err(Refusal {
                            code: R_SESSION_TOO_SLOW,
                            boundary: Some(p.display().to_string()),
                            detail: format!(
                                "미리보기 세션 상한 {TRASH_PREVIEW_SESSION_MAX_SECS}s 초과 — 디렉터리 **안에서** 순회를 중단했다(이 항목은 삭제 대상이 아니다)"
                            ),
                        });
                    }
                }
                if now_i.duration_since(t0).as_secs() > TRASH_CERTIFY_MAX_SECS {
                    return Err(Refusal {
                        code: R_TOO_SLOW,
                        boundary: Some(p.display().to_string()),
                        detail: format!(
                            "인증 시간 상한 {TRASH_CERTIFY_MAX_SECS}s 초과 — 디렉터리 **안에서** 중단했다(이 항목은 삭제 대상이 아니다)"
                        ),
                    });
                }
                // ★B-162 — 배경 잡의 순회만 양보한다(`session_deadline.is_some()` ⇔ 배경 잡).
                if session_deadline.is_some()
                    && now_i.duration_since(last_yield).as_millis() as u64
                        >= TRASH_CERTIFY_YIELD_EVERY_MILLIS
                {
                    dispatch_yield();
                    last_yield = now();
                }
            }
            let md = std::fs::symlink_metadata(&p).map_err(|e| Refusal {
                code: R_UNREADABLE,
                boundary: Some(p.display().to_string()),
                detail: format!("노드 lstat 실패: {e}"),
            })?;
            let dev = dev_of(&p, &md);
            if dev != root_dev {
                // ★C-R1: 하위 장치를 하나라도 만나면 **항목 전체 거부**. 경계까지 지우지 않는다.
                return Err(Refusal {
                    code: R_DESC_DEVICE,
                    boundary: Some(p.display().to_string()),
                    detail: format!(
                        "하위 노드의 장치({dev})가 항목 뿌리의 장치({root_dev})와 다르다 — \
                         인증할 수 없는 장치 경계이므로 이 항목은 **통째로 거부**된다"
                    ),
                });
            }
            bytes = bytes.saturating_add(md.len());
            let mt = mtime_secs(&md);
            if mt.is_finite() && (!newest.is_finite() || mt > newest) {
                newest = mt;
            }
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
            let (_d, ino) = dev_ino(&md);
            let kind = if md.is_dir() { "d" } else if md.file_type().is_symlink() { "l" } else { "f" };
            let mut nh = Sha256::new();
            nh.update(rel.as_bytes());
            nh.update(b"\0");
            nh.update(format!("{kind}:{}:{}:{ino}", md.len(), mtime_bits(&md)).as_bytes());
            let dgt = nh.finalize();
            let mut first16 = [0u8; 16];
            first16.copy_from_slice(&dgt[..16]);
            set_digest = set_digest.wrapping_add(u128::from_be_bytes(first16));
            max_depth = max_depth.max(depth + 1);
            if md.is_dir() {
                stack.push((p, depth + 1));
            }
        }
    }
    // ★MJR-1 의 마지막 구멍 — 루프를 **빠져나온 뒤에도** 마감을 본다. 간격 검사만으로는 간격보다
    //   작은 트리(또는 유별나게 느린 lstat 한 번)가 예산을 넘기고도 `Ok` 로 나갈 수 있다.
    //   ⇒ ★`Ready` 로 가는 길에 **예산을 넘긴 순회가 없다**(부서장이 못 박은 성공 기준).
    if let Some(dl) = session_deadline {
        if now() >= dl {
            return Err(Refusal {
                code: R_SESSION_TOO_SLOW,
                boundary: Some(root.display().to_string()),
                detail: format!(
                    "미리보기 세션 상한 {TRASH_PREVIEW_SESSION_MAX_SECS}s 초과 — 순회는 끝났지만 예산을 넘겼으므로 인증으로 인정하지 않는다"
                ),
            });
        }
    }
    if now().duration_since(t0).as_secs() > TRASH_CERTIFY_MAX_SECS {
        return Err(Refusal {
            code: R_TOO_SLOW,
            boundary: Some(root.display().to_string()),
            detail: format!(
                "인증 시간 상한 {TRASH_CERTIFY_MAX_SECS}s 초과 — 순회는 끝났지만 예산을 넘겼으므로 인증으로 인정하지 않는다"
            ),
        });
    }
    let (rdev, rino) = dev_ino(&root_md);
    Ok(Certificate {
        dev: rdev,
        ino: rino,
        nodes,
        bytes,
        max_depth,
        newest_mtime: newest,
        digest: format!("{set_digest:032x}"),
        walk_secs: now().duration_since(t0).as_secs_f64(),
    })
}

fn mtime_secs(md: &std::fs::Metadata) -> f64 {
    match md.modified() {
        Ok(t) => match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            // UNIX_EPOCH 이전 = 음수. 부호를 살려 둔다(미래·과거 판정은 호출자가 한다).
            Err(e) => -(e.duration().as_secs_f64()),
        },
        Err(_) => f64::NAN,
    }
}
fn mtime_bits(md: &std::fs::Metadata) -> i64 {
    let s = mtime_secs(md);
    if s.is_finite() {
        (s * 1000.0) as i64
    } else {
        i64::MIN
    }
}

/// ★ADDENDUM-1 — '최근'은 **긍정 판정**이다. 나이가 유한하고 창 **이상**일 때만 '최근 아님'.
/// 미래 mtime(음수 나이)·비유한·판독 불가(NaN)는 전부 최근 = 거부.
pub fn is_recent(newest_mtime: f64, now: f64, window_secs: u64) -> bool {
    if !newest_mtime.is_finite() || !now.is_finite() {
        return true;
    }
    let age = now - newest_mtime;
    if !age.is_finite() {
        return true;
    }
    !(age >= window_secs as f64)
}

fn hex32(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ─── 삭제 원장(D-4) — 의도·거부·결과·재고가 전부 한 파일로 간다 ──────────────────────────────
/// 원장 한 줄을 append + fsync 한다. **실패를 삼키지 않는다** — 호출자는 실패를 거부로 바꾼다
/// (C-R2: 쓸 수 없는 거부는 그 자체로 진행 거부다).
pub fn ledger_append(dir: &Path, rec: &Value) -> std::io::Result<()> {
    let line = serde_json::to_string(rec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(DELETE_LEDGER))?;
    f.write_all(format!("{line}\n").as_bytes())?;
    f.sync_all()?;
    Ok(())
}

/// 거부 기록(C-R2) — 거부·그 항목·사유 코드·인증을 무너뜨린 경계 경로를 원장에 남긴다.
/// ★원장에 쓸 수 없으면 `Err` 를 돌려주고, 호출자는 **아무것도 건드리지 않은 채** 거부를 보고한다.
pub fn record_refusal(
    dir: &Path,
    order_id: &str,
    name: &str,
    r: &Refusal,
) -> std::io::Result<()> {
    ledger_append(
        dir,
        &json!({
            "kind": "refusal",
            "ts": now_epoch(),
            "order_id": order_id,
            "entry": name,
            "reason_code": r.code,
            "boundary": r.boundary,
            "detail": r.detail,
            // D-e: origin:"owner_gui" 는 드롭됐다 — 토큰 보유자라는 사실만 적는다(사람 증명 아님).
            "identity": "token_holder",
            "human_verified": false,
        }),
    )
}

// ─── 확인(confirm) 레코드 — 미리보기가 발급하고 명령이 한 번 쓴다 ────────────────────────────
#[derive(Debug, Clone)]
pub struct Confirm {
    pub id: String,
    pub names: Vec<String>,
    pub digest: String,
    /// 단조 시계 기준 발급 시점(벽시계 점프로 수명이 늘어나지 않는다).
    pub issued: Instant,
    /// 미리보기 때의 항목별 인증서(봉인 전후 대조의 기준값).
    pub certs: HashMap<String, Certificate>,
}

fn confirms() -> &'static Mutex<HashMap<String, Confirm>> {
    static C: OnceLock<Mutex<HashMap<String, Confirm>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// ★같음은 만료다 — 나이가 TTL **이상**이면 쓸 수 없다.
pub fn confirm_expired(age_secs: u64, ttl_secs: u64) -> bool {
    age_secs >= ttl_secs
}

/// 여러 이름의 인증서를 하나의 명령 요약으로 접는다(이름 순 — 결정론).
pub fn order_digest(certs: &HashMap<String, Certificate>) -> String {
    let mut names: Vec<&String> = certs.keys().collect();
    names.sort();
    let mut h = Sha256::new();
    for n in names {
        h.update(n.as_bytes());
        h.update(b"\0");
        h.update(certs[n].digest.as_bytes());
        h.update(b"\n");
    }
    hex32(h.finalize().as_slice())
}

// ─── 명령(order) 상태 — `trash.delete` 는 즉시 돌아오고 `trash.order` 가 상태를 돌려준다 ──────
#[derive(Debug, Clone, PartialEq)]
pub struct EntryOutcome {
    pub name: String,
    pub outcome: String,
    pub reason_code: Option<String>,
    pub boundary: Option<String>,
    pub bytes_before: Option<u64>,
    pub duration_ms: u64,
    /// m3: 제거가 시작된 뒤의 오류는 **남은 것이 있을 수 있다**고 정직하게 말한다.
    pub remaining: Option<usize>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OrderState {
    pub order_id: String,
    pub started: f64,
    pub finished: Option<f64>,
    pub per_entry: Vec<EntryOutcome>,
    pub running_long: bool,
}

fn orders() -> &'static Mutex<HashMap<String, OrderState>> {
    static O: OnceLock<Mutex<HashMap<String, OrderState>>> = OnceLock::new();
    O.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 단일 삭제자 — **기다리지 않는** 빗장이다(대기는 그 자체가 stall 이다).
/// `MutexGuard` 는 스레드를 건너가지 못하므로(!Send) 원자 플래그를 쓴다 — 잡기와 놓기가
/// 서로 다른 스레드에서 일어나기 때문이다.
static DELETER_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 잡혔으면 true. 놓는 것은 `deleter_release`.
fn deleter_try_acquire() -> bool {
    DELETER_BUSY
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
}
fn deleter_release() {
    DELETER_BUSY.store(false, std::sync::atomic::Ordering::Release);
}

// ─── 미리보기 잡(비동기) — ★S8 재착륙 §1 · Option A ────────────────────────────────────────────
// 델타 §1.1: `trash.delete` → `order_id` 즉시 반환 → `trash.order` 폴링과 **같은 꼴**로 조립한다.
// 그 선례가 이 파일에 실재하므로(`orders()` · `OrderState`) 새 패턴을 발명하지 않는다.
// ★최상위 불변식(선언 파일 ⒥): **어떤 RPC 도 순회에 블로킹되지 않는다.** `preview_start` ·
// `preview_poll` 은 둘 다 맵 잠금만 잡고 즉시 돌아온다 — 순회는 배경 스레드의 것이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPhase {
    Running,
    Ready,
    Failed,
    /// 미조회 만료 — **묘비로 남긴다**(지우지 않는다). 지워 버리면 다음 폴링이 `job_unknown` 을 받아
    /// 「데몬이 다시 떴다」와 「내가 늦었다」를 **구별할 수 없다**(상태 전이표가 그 둘을 다른 행으로 둔다).
    Expired,
}

#[derive(Debug, Clone)]
pub struct PreviewJob {
    pub job_id: String,
    pub phase: PreviewPhase,
    pub started: Instant,
    pub last_polled: Instant,
    /// 지금까지 인증을 마친 노드 수(항목 경계마다 갱신 — 한 항목 **안**의 진행은 보이지 않는다).
    pub nodes: usize,
    pub result: Option<Value>,
    pub refusal: Option<Value>,
}

fn preview_jobs() -> &'static Mutex<HashMap<String, PreviewJob>> {
    static P: OnceLock<Mutex<HashMap<String, PreviewJob>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 단일 플라이트 빗장 — 도는 잡의 `job_id` 를 쥔다(델타 §1.3 상태 전이표 셋째 칸: `Some(job_id)`/`None`).
/// ★**기다리지 않는다**: 이미 쥐어져 있으면 즉시 `preview_busy` 로 거부한다(대기가 곧 stall 이다).
/// ★이 뮤텍스는 **순회를 건너 쥐지 않는다** — 쥐었다 놓는 구간이 둘 다 상수시간이다.
static PREVIEW_LOCK: Mutex<Option<String>> = Mutex::new(None);

fn preview_lock_try_acquire(job_id: &str) -> bool {
    let mut g = PREVIEW_LOCK.lock().unwrap();
    if g.is_some() {
        return false;
    }
    *g = Some(job_id.to_string());
    true
}

/// 그 잡이 쥔 경우에만 놓는다 — 남의 빗장을 놓지 않는다.
fn preview_lock_release(job_id: &str) {
    let mut g = PREVIEW_LOCK.lock().unwrap();
    if g.as_deref() == Some(job_id) {
        *g = None;
    }
}

/// 미조회 만료 소거(게으른 쓸기 — 타이머 스레드를 만들지 않는다).
/// `running` 인 잡은 **만료시키지 않는다**(배경 스레드가 아직 그것을 쓰고 있다 · 상태 전이표의
/// 「모달 닫기」 행이 「60s 상한까지 완주 후 자동 폐기」라고 적는다).
fn preview_sweep_expired() {
    let ttl = std::time::Duration::from_secs(TRASH_PREVIEW_JOB_TTL_SECS);
    let mut m = preview_jobs().lock().unwrap();
    let mut newly_expired: Vec<String> = Vec::new();
    m.retain(|_, j| {
        let idle = j.last_polled.elapsed();
        match j.phase {
            // 묘비를 한 TTL 더 들고 있다가 버린다(그 뒤의 폴링은 job_unknown 이 맞다).
            PreviewPhase::Expired => idle < ttl.saturating_mul(2),
            PreviewPhase::Running => true,
            _ => {
                if idle >= ttl {
                    newly_expired.push(j.job_id.clone());
                }
                true
            }
        }
    });
    for id in &newly_expired {
        if let Some(j) = m.get_mut(id) {
            j.phase = PreviewPhase::Expired;
            j.result = None;
            j.refusal = None;
        }
    }
    drop(m);
    for id in &newly_expired {
        preview_lock_release(id);
    }
}

/// 미리보기 일꾼을 띄운다 — **시험 주입점**이다(스레드 생성 실패는 실물로 만들 수 없다).
/// ★`Result` 를 **돌려준다**: 호출자가 `Err` 를 처리하지 않으면 컴파일이 통과하지 않도록
/// `.ok()` 를 이 안에 숨기지 않는다(MJR-2 는 바로 그 `.ok()` 에서 났다).
fn preview_spawn<F: FnOnce() + Send + 'static>(body: F) -> std::io::Result<()> {
    #[cfg(test)]
    if PREVIEW_SPAWN_FAILS.load(std::sync::atomic::Ordering::Acquire) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "주입된 스레드 생성 실패(시험 전용)",
        ));
    }
    std::thread::Builder::new()
        .name("trash-preview".into())
        .spawn(body)
        .map(|_| ())
}

/// 시험 전용 — 다음 `preview_spawn` 을 실패시킨다(MJR-2 경로를 밟는 유일한 길).
#[cfg(test)]
static PREVIEW_SPAWN_FAILS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
pub fn set_preview_spawn_fails(v: bool) {
    PREVIEW_SPAWN_FAILS.store(v, std::sync::atomic::Ordering::Release);
}

fn preview_status_str(p: PreviewPhase) -> &'static str {
    match p {
        PreviewPhase::Running => "running",
        PreviewPhase::Ready => "ready",
        PreviewPhase::Failed => "failed",
        PreviewPhase::Expired => "expired",
    }
}

/// 시험 전용 — 모듈 전역 상태를 비운다(검체 간 격리).
#[cfg(test)]
pub fn reset_state_for_test() {
    confirms().lock().unwrap().clear();
    orders().lock().unwrap().clear();
    preview_jobs().lock().unwrap().clear();
    *PREVIEW_LOCK.lock().unwrap() = None;
    set_preview_spawn_fails(false);
    reset_yields_for_test();
}

/// ★시험 전용 단계 훅(R2 §3.5 의 `after_seal`) — 봉인과 대조 **사이**에 끼어드는 쓰기를 만든다.
/// 이 자리를 시험이 잡을 수 없으면 봉인 뒤 대조 조항은 어떤 시험에도 하중이 걸리지 않는다
/// (실측: 이 훅이 없을 때 변이 M5 가 **생존**했다).
/// ★`cargo test` 는 시험을 **병렬**로 돌린다. 훅이 전역 하나면 남의 시험에 끼어든다(실측: 그래서
/// 대조 실행이 1건 빨갰다). 그래서 **등록한 뿌리 아래에서만** 발화한다 — 시험 격리는 기능의
/// 일부다(오염된 초록은 초록이 아니다).
#[cfg(test)]
pub static AFTER_SEAL_HOOK: Mutex<Option<(PathBuf, fn(&Path))>> = Mutex::new(None);

// ─── 한 항목의 집행(순차) — 티켓 ⒡ 의 순서 그대로 ────────────────────────────────────────────
/// 집행 한 항목: 신원 재확인 → 봉인 rename → 재확인 → `remove_dir_all` → lstat 검증.
/// ★호출 전에 인증(certify)이 성공했어야 한다 — 인증 실패는 여기 오지 않고 거부로 끝난다.
#[allow(clippy::too_many_arguments)]
pub fn execute_one(
    root: &Path,
    name: &str,
    baseline: &Certificate,
    dev_of: DevOf,
    now: f64,
    state: &Path,
    order_id: &str,
) -> EntryOutcome {
    let t0 = Instant::now();
    let src = root.join(name);
    let mut out = EntryOutcome {
        name: name.to_string(),
        outcome: "refused".into(),
        reason_code: None,
        boundary: None,
        bytes_before: Some(baseline.bytes),
        duration_ms: 0,
        remaining: None,
        errors: vec![],
    };
    // 이미 사라졌는가 — 손댈 것이 없다(성공도 실패도 아닌 사실).
    if std::fs::symlink_metadata(&src).is_err() {
        out.outcome = "already_gone".into();
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    // (2) 신원 재확인 — 미리보기 이후 바뀌었으면 손대지 않는다.
    let fresh = match certify_entry(&src, dev_of) {
        Ok(c) => c,
        Err(r) => {
            out.reason_code = Some(r.code.to_string());
            out.boundary = r.boundary.clone();
            let _ = record_refusal(state, order_id, name, &r);
            out.duration_ms = t0.elapsed().as_millis() as u64;
            return out;
        }
    };
    if fresh.digest != baseline.digest {
        let r = Refusal {
            code: R_CHANGED,
            boundary: None,
            detail: "미리보기 이후 내용이 바뀌었다 — 새 미리보기가 필요하다".into(),
        };
        out.reason_code = Some(r.code.to_string());
        let _ = record_refusal(state, order_id, name, &r);
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    if is_recent(fresh.newest_mtime, now, TRASH_RECENT_WINDOW_SECS) {
        let r = Refusal {
            code: R_RECENT,
            boundary: None,
            detail: format!(
                "가장 새로운 노드의 나이가 {TRASH_RECENT_WINDOW_SECS}s 미만이거나 판정 불가 — 쓰는 중일 수 있어 거부"
            ),
        };
        out.reason_code = Some(r.code.to_string());
        let _ = record_refusal(state, order_id, name, &r);
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    // (3) 봉인 — 옛 이름으로 계속 쓰는 생산자가 봉인된 객체에 자동으로 닿지 못하게 한다.
    let sealed = root.join(format!("{TRASH_SEAL_PREFIX}{name}"));
    if sealed.exists() {
        let r = Refusal {
            code: R_SEAL_FAILED,
            boundary: Some(sealed.display().to_string()),
            detail: "같은 이름의 봉인이 이미 있다(중단된 명령) — 자동 재개하지 않는다".into(),
        };
        out.reason_code = Some(r.code.to_string());
        out.boundary = r.boundary.clone();
        let _ = record_refusal(state, order_id, name, &r);
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    if let Err(e) = std::fs::rename(&src, &sealed) {
        let r = Refusal {
            code: R_SEAL_FAILED,
            boundary: Some(src.display().to_string()),
            detail: format!("봉인 rename 실패: {e}"),
        };
        out.reason_code = Some(r.code.to_string());
        let _ = record_refusal(state, order_id, name, &r);
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    #[cfg(test)]
    {
        let g = AFTER_SEAL_HOOK.lock().unwrap();
        if let Some((scope, h)) = g.as_ref() {
            if sealed.starts_with(scope) {
                h(&sealed);
            }
        }
    }
    // (4) 봉인 뒤 재확인 — 다르면 **봉인 상태로 남긴다**(되돌리지 않는다 · 자동 재개 없음).
    match certify_entry(&sealed, dev_of) {
        Ok(c) if c.digest == baseline.digest => {}
        Ok(_) => {
            let r = Refusal {
                code: R_CHANGED,
                boundary: Some(sealed.display().to_string()),
                detail: "봉인 뒤 대조가 어긋났다 — 봉인 상태로 남기고 중단(되돌리지 않는다)".into(),
            };
            out.reason_code = Some(r.code.to_string());
            out.boundary = r.boundary.clone();
            out.outcome = "sealed_not_removed".into();
            let _ = record_refusal(state, order_id, name, &r);
            out.duration_ms = t0.elapsed().as_millis() as u64;
            return out;
        }
        Err(r) => {
            out.reason_code = Some(r.code.to_string());
            out.boundary = r.boundary.clone();
            out.outcome = "sealed_not_removed".into();
            let _ = record_refusal(state, order_id, name, &r);
            out.duration_ms = t0.elapsed().as_millis() as u64;
            return out;
        }
    }
    // (5) 제거 — 인증이 "이 뿌리 아래 장치 경계 없음"을 이미 확정했으므로 이 순회는 인증 집합을
    //     넓힐 수 없다(C-R1). 심링크는 따라가지 않고 링크 자체가 지워진다.
    if let Err(e) = std::fs::remove_dir_all(&sealed) {
        // m3: 제거가 **시작된 뒤**의 오류다 — 남은 것이 있을 수 있다고 적는다(무결 주장 금지).
        let remaining = certify_entry(&sealed, dev_of).map(|c| c.nodes).ok();
        // partial = 제거가 시작된 뒤 멈췄다(남은 것이 있을 수 있다 · m3).
        out.outcome = "partial".into();
        out.reason_code = Some(R_REMOVE_ERROR.to_string());
        out.remaining = remaining;
        out.errors.push(format!("{e}"));
        let r = Refusal {
            code: R_REMOVE_ERROR,
            boundary: Some(sealed.display().to_string()),
            detail: format!(
                "제거 중 오류: {e} — 봉인된 잔여가 남아 있을 수 있다(남은 노드 {remaining:?}). \
                 자동 재개하지 않는다; 새 미리보기와 새 명령이 필요하다"
            ),
        };
        let _ = record_refusal(state, order_id, name, &r);
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    // (6) lstat 검증 — 없어졌는가.
    if std::fs::symlink_metadata(&sealed).is_ok() {
        out.outcome = "removal_error".into();
        out.reason_code = Some(R_REMOVE_ERROR.to_string());
        out.errors.push("제거 뒤에도 봉인 경로가 남아 있다".into());
        out.duration_ms = t0.elapsed().as_millis() as u64;
        return out;
    }
    out.outcome = "deleted_verified".into();
    out.duration_ms = t0.elapsed().as_millis() as u64;
    out
}

// ─── 크기 캐시 — ★`trash.list` 는 **걷지 않는다**(DESIGN-S8 4.1) ─────────────────────────────
/// `size-cache.json` — 이름 + 장치 + inode + mtime 으로 키를 만든다. 키가 바뀌면 캐시는 무효다.
/// 걷기는 `trash.preview_start` 의 배경 잡이 할 때만 일어나고(오너가 그 항목을 고른 순간) 그 결과가
/// 여기 쌓인다.
/// 그래서 3.5 GB 합계 계산이 목록 때문에 일어나지 않는다 — 창이 멈추지 않는 이유는 이 한 줄이다.
pub fn size_cache_path(root: &Path) -> PathBuf {
    root.join("size-cache.json")
}
pub fn size_cache_key(name: &str, dev: u64, ino: u64, mtime_bits: i64) -> String {
    format!("{name}|{dev}|{ino}|{mtime_bits}")
}
fn size_cache_read(root: &Path) -> serde_json::Map<String, Value> {
    let path = size_cache_path(root);
    // ★⒟-2: **읽기 전에** 크기를 본다 — 다 읽은 뒤 버리면 상한이 비용을 막지 못한다(이미 다 읽었다).
    //   초과면 **캐시 부재와 똑같이** 빈 맵이다 ⇒ 호출자는 정상 경로(목록은 pending · 미리보기는 순회)로
    //   간다. ⛔거부로 바꾸지 않는다. 판독 실패(권한·비UTF-8)도 종전처럼 빈 맵이다 — 같은 방향이다.
    if let Ok(md) = std::fs::metadata(&path) {
        if md.len() > TRASH_SIZE_CACHE_MAX_BYTES {
            return serde_json::Map::new();
        }
        // ★S20A §2 ⒥ — **정규 파일이 아니면 캐시 부재와 동일**로 접는다(FIFO·장치·소켓·디렉터리).
        //   왜: 크기 상한은 **읽는 양**을 묶지만 FIFO 의 `len()` 은 0 이라 상한을 통과하고, 그 뒤
        //   `read_to_string` 이 **쓰는 쪽을 기다리며 영구히 선다** — 그 자리는 **오너 전용이 아닌**
        //   `list_payload` 도 지나간다. ⒜`metadata` 는 **이미 불렀으니 추가 I/O 가 0** 이고
        //   ⒝`stat` 자신은 FIFO 에서 블로킹하지 않는다(블로킹하는 것은 `open`/`read` 다) ⇒ **syscall 한 번**.
        //   ⛔거부가 아니다(캐시는 선택적 가속 · 목록은 그 항목을 `pending` 으로 보인다).
        if !md.file_type().is_file() {
            return serde_json::Map::new();
        }
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}
/// 유예된 크기 캐시 쓰기를 **한 번에** 반영한다.
///
/// ★⒟′-1(S20 §4 ⒤) — 이 함수는 **종단 판정 뒤에만** 불린다. 종전에는 인증 성공마다 순회 루프
/// **안에서** 썼고, 그래서 **마지막 마감 검사 뒤 `Ready` 발행까지의 구간**에 캐시 파일 크기에 비례하는
/// 작업(읽기+파싱+삽입+직렬화+쓰기 · ⒟-2 실측 **바이트당 약 33.8 ns**)이 **한 항목분 남아 있었다**.
/// 그 작업을 그 구간 **밖으로** 옮긴 것이 ⒟′-1 이고, 결박 검체는
/// `the_size_cache_is_not_written_until_the_terminal_decision_is_made` 다.
///
/// ★파급(제거·이동도 변경이므로 적는다):
/// ⒜**캐시는 선택적 가속이다** — 발행과 이 호출 사이에 프로세스가 죽으면 다음 목록이 그 항목을
///   `size.state = "pending"` 으로 보일 뿐이고, 원장·인증서·확인 레코드는 이 함수와 **무관**하다
///   (⇒ 유예가 「정확성」을 건드리지 않는다. 같은 구간의 **원장 기록은 유예하지 않았다** — 그것은
///   정확성이고, 그 선택은 `a_refusal_is_in_the_ledger_before_the_terminal_decision` 이 잠근다).
/// ⒝읽기·자르기·쓰기를 **항목마다** 하던 것이 **한 번**으로 접혔다(N항목이면 파일 I/O 가 N벌 → 1벌).
/// ⒞`pending` 이 비면 **아무 I/O 도 하지 않는다**(거부만 난 미리보기는 캐시 파일을 건드리지 않는다).
/// ★**⒠ 종단 뒤 이 일의 수명은 누가 묶는가**(S29 · 관문 codex 의 새 질문 · ⛔나는 호출 자리를 **고치지
/// 않았다** — 그 함수는 A1 설계면이라 42 의 R3 소관이고, 여기 적는 것은 **읽어서 확인한 사실**이다):
/// ⒜**누구의 스레드인가**: 미리보기 **배경 일꾼 스레드 자신**이다. 새 스레드를 만들지 않고, 그 스레드의
///   **마지막 문장**으로 돈다(단계 발행 → 빗장 해제 → 이 반영). ⇒ 스레드의 수명이 **이 반영만큼 늘어난다**.
/// ⒝**누가 기다리는가**: ⛔**아무도 없다** — 일꾼은 `JoinHandle` 을 버리는 **분리(detached)** 스레드다
///   (`preview_spawn` 이 `.map(|_| ())` 로 핸들을 버린다) ⇒ join 도 취소도 없고, 데몬이 내려가면
///   **프로세스 종료가 끊는다**(그때 못 쓴 캐시는 다음 목록이 `pending` 으로 보인다 — 무해).
/// ⒞**일의 양은 유계다**: 읽는 양은 **256 KiB 상한**(`TRASH_SIZE_CACHE_MAX_BYTES`) · 쓰는 양은 한 번의
///   원자적 쓰기다. ⛔**지연은 무계다**(fsync) — 실측 **최악 11.833625 ms**(현실 크기 캐시 10,231 B ·
///   20회 중 최악 · **debug 측정 · release 미측정**).
/// ⒟**새 잔여인가**: ★**오너의 답 경로에서는 아니다**(발행 **뒤**에 돈다) · ★**빗장도 막지 않는다**(해제 뒤다).
///   ⇒ 남는 것 둘: ⒤**분리 스레드가 최대 한 반영만큼 더 산다**(위 11.8 ms 규모) ⒥**동시에 도는 반영의
///   개수는 제품이 묶지 않는다** — 빗장은 **순회**를 1로 묶지만 이 꼬리는 빗장 밖이라, 미리보기를 빠르게
///   연달아 시작하면 꼬리가 겹칠 수 있다(UI 는 모달 하나라 1이지만 raw RPC 는 상한이 없다).
///   그 겹침의 결과는 ⒟′-5 에 적은 **잃은 갱신**(가용성 등급 · 찢긴 파일은 원자적 쓰기가 막는다 ·
///   tmp 이름이 호출마다 유일해 섞이지도 않는다)이고 **정확성 축이 아니다**.
/// ⒟`measured_at` 은 이제 **반영 시각**이다(종전엔 항목별 쓰기 시각) — 차이는 한 순회 안의 밀리초이고
///   이 값의 쓰임새는 아래 **LRU 자르기 하나**다(`list_payload` 는 이 값을 읽지 않는다 · 전수 확인).
fn size_cache_flush(root: &Path, pending: &[(String, Certificate)]) {
    if pending.is_empty() {
        return;
    }
    let mut m = size_cache_read(root);
    let at = now_epoch();
    for (key, c) in pending {
        m.insert(
            key.clone(),
            json!({"bytes": c.bytes, "nodes": c.nodes, "max_depth": c.max_depth,
                   "newest_mtime": c.newest_mtime, "measured_at": at, "walk_secs": c.walk_secs}),
        );
    }
    // 유계: 최근 64개만 들고 있는다(휴지통 뿌리의 자식 수는 한 자리 수다 — 넉넉한 상한).
    if m.len() > 64 {
        let mut keys: Vec<(String, f64)> = m
            .iter()
            .map(|(k, v)| (k.clone(), v.get("measured_at").and_then(Value::as_f64).unwrap_or(0.0)))
            .collect();
        keys.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (k, _) in keys.into_iter().take(m.len() - 64) {
            m.remove(&k);
        }
    }
    // ★⒟′-5(부서장 2026-10-14 01:3x · 근거 = S22 심사에서 gemini 가 짚은 기제) — **원자적 쓰기**다.
    //   종전 `std::fs::write` 는 **제자리 truncate** 라 읽고-고쳐-쓰기 경쟁에서 두 축이 열려 있었다:
    //   ⒤**잃은 갱신**(두 쓰기가 겹치면 한 쪽 항목이 사라진다) ⒥**찢긴 파일**(쓰는 중에 읽히면 파싱이
    //   실패한다). ★이 한 줄이 없애는 것은 **⒥ 하나**다 — tmp 에 쓰고 fsync 한 뒤 **rename 으로 교체**하므로
    //   독자는 **옛 파일 전체나 새 파일 전체**만 본다. ⛔**⒤는 남는다**(읽기-쓰기 사이의 창은 그대로다).
    //   ★그래도 손실 등급은 **「가용성·속도」이고 「정확성」이 아니다**: 캐시 키가
    //   `name|dev|ino|mtime_bits`(`size_cache_key`)라서 **낡은 항목이 바뀐 항목으로 오인될 수 없고**,
    //   파싱 실패는 `size_cache_read` 가 **빈 맵으로 접는다** ⇒ ⒤의 영향은 **「다시 재야 한다」뿐**이다.
    //   ⛔**tmp+rename 을 손으로 짜지 않았다** — 같은 일을 하는 공개 함수가 이미 있다:
    //   `cys::pack::write_atomic`(`src/pack.rs` · 같은 디렉터리 tmp → fsync → rename → 디렉터리 fsync ·
    //   tmp 이름은 **호출마다 유일**해서 같은 프로세스의 두 호출도 섞이지 않는다 · H-CONC-3).
    //   ★`Err` 는 종전처럼 **버린다**: 캐시는 선택적 가속이고, 못 썼으면 다음 목록이 `pending` 을 보일 뿐이다
    //   (그래서 **파일 부재는 「인증이 돈 적 없다」의 증거가 아니다** — codex N6 이 바로 그 추론을 깼다).
    //   ★파급: 교체가 **rename** 이므로 ⒜`size-cache.json` 의 **inode 가 매번 바뀐다**(이 값을 읽는 코드 0건) ·
    //   ⒝**휴지통 뿌리 디렉터리의 mtime 이 매 반영마다** 바뀐다(종전엔 파일 생성 때만) — 뿌리 디렉터리 mtime 으로
    //   판정하는 코드는 **없다**(`root_sane` 은 `symlink_metadata` 의 **종류**만 보고, 쓸기는 **자식**의 mtime 을 본다).
    let _ = cys::pack::write_atomic(&size_cache_path(root), Value::Object(m).to_string().as_bytes());
}

/// ★**조언용** 유일본 표지(advisory only · 아무 것도 막지 않는다).
/// 근거(B-064 실측): 부서 격리분의 `account`·`pack-dept-*`·`state`(대화기억)는 그 기계에 **유일본**이고,
/// `factory-reset-*` 는 초기화 당시의 상태 전부다. 표지는 오너가 **보고 판단하도록** 띄우는 것이고,
/// 판정에 쓰이지 않는다 — 조언 신호를 비가역 행동의 하중으로 쓰면 안 된다(부서장 Q1 결정).
pub fn sole_copy_advisory(name: &str, children: &[String]) -> bool {
    if name.starts_with("factory-reset-") {
        return true;
    }
    children.iter().any(|c| {
        c == "state" || c == "account" || c.starts_with("pack-dept-") || c == "pack"
    })
}

/// 이름의 스탬프에서 나이를 읽는다(D-r2-9 — 유예 시계는 **이름의 스탬프**이고 mtime 이 아니다).
/// 생산자가 둘이고 형식이 둘이다: ①epoch 초(cys-dept:271 · javis_org.py:600) ②`%Y%m%dT%H%M%SZ`
/// (factory_reset.rs:546). **둘 다 받고, 알 수 없으면 '오래되지 않음'** 으로 본다(미지 = 안 지운다).
pub fn stamp_epoch_from_name(name: &str) -> Option<f64> {
    let tail = name.rsplit('-').next()?;
    // ② 압축 UTC
    if tail.len() == 16 && tail.ends_with('Z') && tail.as_bytes()[8] == b'T' {
        let (d, t) = tail.split_at(8);
        let (h, mi, s) = (&t[1..3], &t[3..5], &t[5..7]);
        let y: i64 = d[0..4].parse().ok()?;
        let mo: i64 = d[4..6].parse().ok()?;
        let da: i64 = d[6..8].parse().ok()?;
        let hh: i64 = h.parse().ok()?;
        let mm: i64 = mi.parse().ok()?;
        let ss: i64 = s.parse().ok()?;
        // 민간 그레고리력 → epoch (순수 산술 · 윤년 포함 · 외부 의존 없음).
        let days = days_from_civil(y, mo, da)?;
        return Some((days * 86400 + hh * 3600 + mm * 60 + ss) as f64);
    }
    // ① epoch 초
    if tail.len() >= 9 && tail.chars().all(|c| c.is_ascii_digit()) {
        return tail.parse::<f64>().ok();
    }
    None
}
/// Howard Hinnant 의 days_from_civil(순수) — 1970-01-01 기준 일수.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// 이름 스탬프 기준 TTL 초과 여부 — **알 수 없으면 false**(오래되지 않음 = 표시도 안 한다).
pub fn is_old_by_stamp(name: &str, now: f64, ttl_days: f64) -> bool {
    match stamp_epoch_from_name(name) {
        Some(st) if st.is_finite() && now.is_finite() => (now - st) > ttl_days * 86400.0,
        _ => false,
    }
}

// ─── RPC ─────────────────────────────────────────────────────────────────────────────────────
/// `trash.*` 의 단일 진입점. `owner_ok` 는 **handlers 가 판정해** 넘긴다
/// (`from_sid.is_none() && operator_token 일치` — `owner_token` 은 읽지 않는다: 더 넓은 부착면이라
/// S6·S7 이 그 키를 넓히면 삭제까지 함께 넓어진다 · DESIGN-S8.md:52).
pub fn handle(
    daemon: &Arc<Daemon>,
    sub: &str,
    params: &Value,
    id: &Value,
    owner_ok: bool,
    caller_pid: Option<u32>,
) -> Reply {
    // 기지 데몬 전용 — 부서 데몬에는 이 동사가 없다(휴지통은 기계에 하나다).
    if cys::is_dept_socket(&daemon.socket_path) {
        return Reply::Single(err_response(
            id,
            R_NOT_BASE,
            "휴지통 삭제는 기지 데몬 전용이다(휴지통은 기계에 하나 · 부서 데몬에는 이 동사가 없다)",
        ));
    }
    let root = trash_root();
    let state = state_dir(&daemon.socket_path);
    // 뿌리가 심링크거나 디렉터리가 아니면 **아무 동사도** 동작하지 않는다(S8:115).
    if root.exists() && !root_sane(&root) {
        return Reply::Single(err_response(
            id,
            "refused",
            "휴지통 뿌리가 디렉터리가 아니거나 심링크다 — 이 상태에서는 동작하지 않는다",
        ));
    }
    match sub {
        // 읽기 전용 · 어떤 로컬 호출자든 — 감사자가 거부당하지 않게(에이전트는 이미 `ls` 할 수 있고
        // 비밀은 돌려주지 않는다). ★걷지 않는다: 깊이 1 과 캐시만 읽는다.
        "list" => Reply::Single(ok_response(id, list_payload(&root, &state))),
        // ★퇴역 완료 — 동기 미리보기 동사 `trash.preview` 는 **없다**(§1 마지막 조각 · 부서장 판정).
        //   그것은 `certify_names(root, state, &names, None, None)` 을 불러 **세션 예산도 빗장도 양보도
        //   없이** 걷던 **두 번째 인증자**였고, 살아 있는 동안 「`Ready` 는 예산 안에서 인증됨」과
        //   「단일 플라이트」가 **문면으로만** 참이었다. 아래 비동기 두 동사가 그 자리를 대신한다.
        //   (재출현 금지는 `the_synchronous_preview_verb_is_retired_and_cannot_come_back` 가 잠근다.)
        // ★S8 재착륙 §1 — 비동기 미리보기. 두 동사 모두 **삭제 동사와 같은 오너 GUI 전용 계급**이다
        //   (§3.1: `trash.preview_start`·`trash.preview_poll`·`trash.delete` 는 오너 GUI 직접 발화만).
        //   계급을 여기서 **완화하지 않는다** — `owner_ok` 는 `handlers.rs` 의 `trash_owner_grade_ok`
        //   (operator_token 전용 + pane 무귀속)가 판정한 값이고 이 모듈은 그것을 받기만 한다.
        "preview_start" => {
            if !owner_ok {
                return Reply::Single(err_response(id, R_NOT_OWNER, OWNER_REQUIRED_MSG));
            }
            preview_start(id, &root, &state, params)
        }
        "preview_poll" => {
            if !owner_ok {
                return Reply::Single(err_response(id, R_NOT_OWNER, OWNER_REQUIRED_MSG));
            }
            preview_poll(id, params)
        }
        "delete" => {
            if !owner_ok {
                return Reply::Single(err_response(id, R_NOT_OWNER, OWNER_REQUIRED_MSG));
            }
            delete(id, &root, &state, params, caller_pid)
        }
        "order" => {
            let oid = params.get("order_id").and_then(Value::as_str).unwrap_or("");
            let o = orders().lock().unwrap().get(oid).cloned();
            match o {
                Some(o) => Reply::Single(ok_response(id, order_payload(&o))),
                None => Reply::Single(err_response(
                    id,
                    "not_found",
                    "그 order_id 를 이 데몬이 모른다 — 데몬이 다시 뜨면 원장(DELETE-LEDGER.jsonl)이 정본이다",
                )),
            }
        }
        other => Reply::Single(err_response(
            id,
            "unknown_method",
            &format!(
                // ★`preview`(동기)는 목록에서 **빠졌다** — 그 동사가 퇴역했으므로 안내가 없는 것을
                //   가리키면 구판 호출자가 그것을 다시 쓰려 한다.
                "trash.{other} 는 없는 동사다 — list|preview_start|preview_poll|delete|order"
            ),
        )),
    }
}

pub const OWNER_REQUIRED_MSG: &str = "휴지통 삭제는 앱(오너 GUI)에서만 명령한다 — \
    operator token 과 pane 에 붙지 않은 호출자만 이 동사를 쓴다. \
    ★정직한 한계: 같은 UID 로 토큰 파일을 읽는 프로세스는 이 등급을 참칭할 수 있다(거버넌스 구분이지 보안 경계가 아니다).";

fn order_payload(o: &OrderState) -> Value {
    json!({
        "order_id": o.order_id, "started": o.started, "finished": o.finished,
        "running_long": o.running_long,
        "per_entry": o.per_entry.iter().map(|e| json!({
            "name": e.name, "outcome": e.outcome, "reason_code": e.reason_code,
            "boundary": e.boundary, "bytes_before": e.bytes_before,
            "duration_ms": e.duration_ms, "remaining": e.remaining, "errors": e.errors,
        })).collect::<Vec<_>>(),
    })
}

/// 목록 — 걷지 않는다. 이름·나이·깊이1 내용·유일본 조언 표지·오래됨 표시·캐시된 크기만.
pub fn list_payload(root: &Path, state: &Path) -> Value {
    let now = now_epoch();
    let cache = size_cache_read(root);
    let mut entries = Vec::new();
    let mut strays = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut names: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        names.sort();
        for p in names {
            let name = match p.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name == "size-cache.json" {
                continue;
            }
            let md = match std::fs::symlink_metadata(&p) {
                Ok(m) => m,
                Err(e) => {
                    strays.push(json!({"name": name, "why": format!("lstat 실패: {e}")}));
                    continue;
                }
            };
            if !md.is_dir() {
                strays.push(json!({"name": name, "why": "디렉터리가 아니다"}));
                continue;
            }
            // 봉인 잔재 = 중단된 명령. 자동 재개하지 않는다.
            let interrupted = name.starts_with(TRASH_SEAL_PREFIX);
            let mut children: Vec<String> = Vec::new();
            if let Ok(kids) = std::fs::read_dir(&p) {
                for k in kids.flatten() {
                    if let Some(n) = k.file_name().to_str() {
                        // ★점 파일은 **첫 단계 목록에 넣지 않는다**(설계 r2 §3.4: "`.producer` 는
                        //   점 파일이다 — 첫 단계 내용도 아니고 kind 판정에도 쓰이지 않지만 내용
                        //   집합의 노드다"). 내가 f1c4afb6 에서 표식을 쓰기 시작한 뒤로 이 목록에
                        //   `.producer` 가 섞여 오너가 만들지 않은 파일이 보이고 개수가 하나 더
                        //   많았다(성찰 A-2 에서 발견). digest 는 파일시스템을 직접 걷으므로
                        //   이 필터와 무관하다 — **보여주는 목록만** 걸러진다.
                        if n.starts_with('.') {
                            continue;
                        }
                        children.push(n.to_string());
                    }
                }
            }
            children.sort();
            let (dev, ino) = dev_ino(&md);
            let key = size_cache_key(&name, dev, ino, mtime_bits(&md));
            let size = cache.get(&key).cloned();
            let pst = producer_state(&p, &real_alive);
            entries.push(json!({
                "name": name,
                "interrupted_deleting": interrupted,
                "root_mtime_age_secs": (now - mtime_secs(&md)).max(0.0),
                "stamp_epoch": stamp_epoch_from_name(&name),
                "old_by_stamp": is_old_by_stamp(&name, now, ttl_days_default()),
                "old_marked": p.join(TRASH_OLD_MARKER).exists(),
                "children_depth1": children.clone(),
                "children_total": children.len(),
                "sole_copy_advisory": sole_copy_advisory(&name, &children),
                // ★표식 상태는 **한 번만** 읽는다(성찰 A-1). 두 번 읽으면 두 읽기 사이에 생산자가
                //   끝날 수 있어 "생산자가 돌고 있다" 표지와 `deletable: true` 가 같은 줄에 실릴 수
                //   있었다(스스로 모순되는 줄 · 안전하지만 품질 결함). 비용도 절반이다.
                "producer_chip": producer_chip(&pst),
                "deletable": !interrupted && !producer_blocks(&pst),
                // ★크기는 **측정됐을 때만** 숫자다. 아니면 pending — 목록이 걷지 않기 때문이다.
                "size": match size { Some(s) => json!({"state": "measured", "value": s}),
                                     None => json!({"state": "pending"}) },
            }));
        }
    }
    let ledger_ok = ledger_writable(state);
    json!({
        "root": root.display().to_string(),
        "entries": entries,
        "strays": strays,
        "ledger_writable": ledger_ok,
        "ledger_state": if ledger_ok { Value::Null } else { json!(R_LEDGER_UNWRITABLE) },
        "constants": {
            "recent_window_secs": TRASH_RECENT_WINDOW_SECS,
            "confirm_ttl_secs": TRASH_CONFIRM_TTL_SECS,
            "certify_max_nodes": TRASH_CERTIFY_MAX_NODES,
            "certify_max_secs": TRASH_CERTIFY_MAX_SECS,
            "remove_stall_report_secs": TRASH_REMOVE_STALL_REPORT_SECS,
        },
        "delete_supported": delete_supported(),
    })
}

fn ttl_days_default() -> f64 {
    std::env::var("CYS_TRASH_TTL_DAYS")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(14.0)
}

#[cfg(unix)]
fn dev_ino(md: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    (md.dev(), md.ino())
}
#[cfg(windows)]
fn dev_ino(_md: &std::fs::Metadata) -> (u64, u64) {
    // ★윈도우에서 다르게 도는 자리 2 — 안정 API 로 dev·ino 를 얻을 수 없다. 캐시 키는
    // 이름+mtime 만으로 약해지고, 그 약함이 윈도우 삭제를 기본 꺼짐으로 두는 이유의 일부다.
    (0, 0)
}

/// 윈도우 삭제는 **검증된 빌드에서만** 켜진다(12.W 가 돌지 않으면 꺼진 채로 출하된다).
pub fn delete_supported() -> bool {
    if cfg!(windows) {
        cfg!(feature = "trash_win_validated")
    } else {
        true
    }
}

fn ledger_writable(state: &Path) -> bool {
    // 쓸 수 있는가를 **쓰지 않고** 본다: 디렉터리 존재 + append 열기만 시도하고 닫는다.
    if std::fs::create_dir_all(state).is_err() {
        return false;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join(DELETE_LEDGER))
        .is_ok()
}

/// 미리보기 — 오너가 고른 이름들을 인증하고, 확인(confirm) 레코드를 발급한다.
/// ★거부는 여기서 끝난다: 인증할 수 없는 항목은 **확인에 담기지 않으므로** 명령의 대상이 될 수 없다.
/// 이름별 인증의 결과 — 동기·비동기 **공용 본문**의 반환값.
enum NamesOutcome {
    Done {
        shown: Vec<Value>,
        refusals: Vec<Value>,
        certs: HashMap<String, Certificate>,
    },
    InvalidName,
    LedgerUnwritable,
    /// 세션 예산 소진 — **잡 전체를 종결**시킨다(항목별 거부와 다르다).
    SessionExpired(Refusal),
}

/// 이름들을 인증한다 — **동기 경로와 비동기 잡이 같은 본문을 쓴다**(문면이 두 벌이 되지 않게).
///
/// ★`session_deadline` 이 `None` 이면 세션 예산이 **없다**. 부서장 판정 4⒜대로 **동기 경로에 루프
/// 예산을 끼워 넣지 않는다** — 그 경로는 §1 이 퇴역시키는 대상이고, 죽을 코드에 중단을 다는 것은
/// 「막힘」 쪽을 새로 만드는 일이다. 그래서 예산은 **인자로만** 들어오고 호출자가 정한다.
/// ★`on_progress` 는 **항목 경계**에서만 불린다 — 한 항목 안의 진행은 `certify_entry` 가 한 번의
/// 호출로 순회하므로 보이지 않는다(정직한 한계 · 그래서 진행 수치는 항목 단위다).
/// ★⒟′-1(S20 §4) — 크기 캐시 쓰기는 **여기서 하지 않는다**. 인증에 성공한 항목의 캐시 항목은
/// `pending` 에 쌓이고, **종단 판정(그리고 `Ready` 발행·빗장 해제) 뒤에** 호출자가
/// `size_cache_flush` 로 한 번에 반영한다. 이 겉면이 그 쌍(`결과`, `유예분`)을 한 값으로 묶어 돌려주므로
/// **호출자가 유예분을 버리면 컴파일이 통과하지 않는다**(`.ok()` 로 숨겨진 MJR-2 와 같은 길을 막는다).
///
/// ★여기 **남겨 둔 것**과 그 이유: 거부의 **원장 기록**(`record_refusal` → `ledger_append` → `fsync`)은
/// 유예하지 않았다. 유예하면 ⒜발행과 반영 사이에 프로세스가 죽을 때 **거부가 원장에 남지 않고**
/// ⒝「원장을 쓸 수 없다」를 **거부 시점에** 알 수 없다(그 Err 가 `LedgerUnwritable` 로 잡 전체를
/// 종결시키는 길이 사라진다) ⇒ 둘 다 **정확성** 문제이고, 캐시(가속)와 같은 급이 아니다
/// (부서장 S20 §4 ⒟′-1 ⒥ 경고 그대로). 그 결과 구간에 남는 원장 쓰기는 **최대 1건**이다(아래
/// `preview_outcome_phase` 머리의 불변식 ③).
fn certify_names(
    root: &Path,
    state: &Path,
    names: &[String],
    session_deadline: Option<Instant>,
    on_progress: Option<&dyn Fn(usize)>,
) -> (NamesOutcome, Vec<(String, Certificate)>) {
    let mut pending: Vec<(String, Certificate)> = Vec::new();
    let out = certify_names_into(root, state, names, session_deadline, on_progress, &mut pending);
    (out, pending)
}

fn certify_names_into(
    root: &Path,
    state: &Path,
    names: &[String],
    session_deadline: Option<Instant>,
    on_progress: Option<&dyn Fn(usize)>,
    pending: &mut Vec<(String, Certificate)>,
) -> NamesOutcome {
    let now = now_epoch();
    let mut certs: HashMap<String, Certificate> = HashMap::new();
    let mut shown = Vec::new();
    let mut refusals = Vec::new();
    let mut nodes_done = 0usize;
    for name in names {
        if !safe_name(name) {
            return NamesOutcome::InvalidName;
        }
        // ★항목을 **시작하기 전에** 세션 마감을 본다(시작해 놓고 중간에 끊는 것보다 싸다).
        if let Some(dl) = session_deadline {
            if Instant::now() >= dl {
                return NamesOutcome::SessionExpired(Refusal {
                    code: R_SESSION_TOO_SLOW,
                    boundary: Some(root.join(name).display().to_string()),
                    detail: format!(
                        "미리보기 세션 상한 {TRASH_PREVIEW_SESSION_MAX_SECS}s 초과 — 남은 항목을 시작하지 않았다"
                    ),
                });
            }
        }
        let p = root.join(name);
        // D-r2-5 M3: 생산자가 돌고 있으면(또는 생존 판정 불가면) 이 항목은 거부한다.
        let pst = producer_state(&p, &real_alive);
        if producer_blocks(&pst) {
            let r = Refusal {
                code: R_PRODUCER_ACTIVE,
                boundary: Some(p.join(".producer").display().to_string()),
                detail: format!("생산자 표식 상태: {} — 거부", producer_chip(&pst)),
            };
            if record_refusal(state, "(preview)", name, &r).is_err() {
                return NamesOutcome::LedgerUnwritable;
            }
            refusals.push(refusal_json(name, &r));
            continue;
        }
        match certify_entry_until(&p, &real_dev, session_deadline) {
            Ok(c) => {
                let recent = is_recent(c.newest_mtime, now, TRASH_RECENT_WINDOW_SECS);
                if let Ok(md) = std::fs::symlink_metadata(&p) {
                    let (dev, ino) = dev_ino(&md);
                    // ★⒟′-1: 쓰지 않고 **쌓는다** — 이 자리의 쓰기가 마지막 항목에서는 「마감 검사 뒤
                    //   `Ready` 까지」 구간에 들어가고, 그 비용은 **캐시 파일 크기에 비례**했다.
                    pending.push((size_cache_key(name, dev, ino, mtime_bits(&md)), c.clone()));
                }
                nodes_done = nodes_done.saturating_add(c.nodes);
                if let Some(f) = on_progress {
                    f(nodes_done);
                }
                if recent {
                    let r = Refusal {
                        code: R_RECENT,
                        boundary: None,
                        detail: format!(
                            "가장 새로운 노드가 {TRASH_RECENT_WINDOW_SECS}s 안에 쓰였거나 판정 불가 — 쓰는 중일 수 있어 거부"
                        ),
                    };
                    if record_refusal(state, "(preview)", name, &r).is_err() {
                        return NamesOutcome::LedgerUnwritable;
                    }
                    refusals.push(refusal_json(name, &r));
                    continue;
                }
                shown.push(json!({
                    "name": name, "nodes": c.nodes, "bytes": c.bytes, "max_depth": c.max_depth,
                    "newest_mtime": c.newest_mtime, "digest": c.digest, "walk_secs": c.walk_secs,
                }));
                certs.insert(name.clone(), c);
            }
            Err(r) => {
                // ★세션 마감이 원인이면 **이 항목의 거부로 끝내지 않는다** — 잡 전체가 종결된다.
                if r.code == R_SESSION_TOO_SLOW {
                    let _ = record_refusal(state, "(preview)", name, &r);
                    return NamesOutcome::SessionExpired(r);
                }
                // ★C-R1 + C-R2: 인증 실패 = 그 항목 **전체** 거부, 그리고 그 거부는 원장에 남는다.
                if record_refusal(state, "(preview)", name, &r).is_err() {
                    return NamesOutcome::LedgerUnwritable;
                }
                refusals.push(refusal_json(name, &r));
            }
        }
    }
    NamesOutcome::Done {
        shown,
        refusals,
        certs,
    }
}

/// 인증 결과를 미리보기 페이로드로 만든다(성공분이 있으면 `confirm` 을 발급해 등록한다).
/// 동기 경로와 비동기 잡이 **같은 모양의 결과**를 돌려주도록 이 자리를 공용으로 둔다.
fn preview_payload(
    shown: Vec<Value>,
    refusals: Vec<Value>,
    certs: HashMap<String, Certificate>,
) -> Value {
    if certs.is_empty() {
        return json!({"confirm_id": Value::Null, "entries": shown, "refused": refusals,
                      "digest": Value::Null, "confirm_ttl_secs": TRASH_CONFIRM_TTL_SECS});
    }
    let digest = order_digest(&certs);
    let cid = new_id("cf");
    confirms().lock().unwrap().insert(
        cid.clone(),
        Confirm {
            id: cid.clone(),
            names: {
                let mut k: Vec<String> = certs.keys().cloned().collect();
                k.sort();
                k
            },
            digest: digest.clone(),
            issued: Instant::now(),
            certs,
        },
    );
    json!({"confirm_id": cid, "entries": shown, "refused": refusals, "digest": digest,
           "confirm_ttl_secs": TRASH_CONFIRM_TTL_SECS,
           "delete_supported": delete_supported()})
}

fn names_param(id: &Value, params: &Value) -> Result<Vec<String>, Reply> {
    match str_list(params, "names") {
        // ★⒟′-6 — 상한 초과는 **거부**다(잡을 띄우지 않는다 · 빗장을 쥐지 않는다 · 원장에 쓰지 않는다 —
        //   이 관문은 `safe_name` 선검사와 같은 자리이고 그 자리는 원장을 건드리지 않는다).
        //   ⛔사유 **어휘를 새로 만들지 않았다**: 화면이 읽는 표에 없는 코드를 더하면 화면이 사유를 잃는다
        //   (S15 §B1 — 거부 코드 어휘의 단일 정의처가 **없다**). 그래서 기존 `invalid_params` + `detail` 이다.
        Some(v) if v.len() > TRASH_PREVIEW_MAX_NAMES => Err(Reply::Single(err_response(
            id,
            "invalid_params",
            &format!(
                "한 번에 미리볼 수 있는 항목이 {TRASH_PREVIEW_MAX_NAMES}개를 넘었다({}개) — \
                 나눠서 고르십시오(아무 것도 지우지 않았고 미리보기를 시작하지도 않았다)",
                v.len()
            ),
        ))),
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(Reply::Single(err_response(
            id,
            "invalid_params",
            "names 가 없다 — 미리보기는 이름을 하나씩 받는다(전체선택은 없다)",
        ))),
    }
}

const INVALID_NAME_MSG: &str =
    "이름에 경로 구분자·상대 성분이 있다 — 휴지통 뿌리의 직속 자식 이름만 받는다";

/// 잡 결과 → (단계, 결과, 거부) 매핑.
///
/// ★별 함수로 떼어 둔 이유는 **검사가 잡을 자리를 만들기 위해서**다: 부서장이 못 박은 §1 성공 기준은
/// 「예산을 넘긴 순회가 `Ready` 로 끝나는 길이 없다」이고, 그 「길」이 모이는 자리가 이 함수다.
/// ⇒ `Ready` 로 가는 입구는 **`Done` 하나뿐**이고 `SessionExpired` 는 `Failed` 다(스레드 안에 두면
/// 120 s 상수를 주입할 수 없어 어떤 검사도 이 매핑에 하중을 걸 수 없다).
///
/// ───────────────────────────────────────────────────────────────────────────────────────────
/// ★⒟′-3(S20 §3·§4) — **잠근 기준의 처분**. 조용히 다시 읽지 않고 **결함이라고 적는다**.
///
/// ⛔**잠근 기준 「예산을 넘긴 순회가 `Ready` 로 끝나는 길이 **없다**」는 어떤 구현으로도 충족될 수
/// 없다**(ε>0 · S20 M2). 마지막 마감 검사와 `Ready` 발행 사이에는 **항상** 일이 남고, 그 일 안에
/// `fsync` 가 있을 수 있으므로(아래 ③) 그 ε 에 **ms 상한을 걸 수도 없다** — 디스크 지연은 무계다.
/// ⇒ 기준 문면은 **고치지 않고**, 그 옆에 이 사실과 아래 **대체 불변식**을 적는다
/// (레인 법칙 `a-locked-criteria-file-can-be-wrong-and-still-be-the-right-pin`).
/// ⛔**「`Ready` = 모든 인증이 예산 안에」는 쓰지 않는다** — 그것은 부서장이 기각한 ⒝ 다(S20 M3).
///
/// ★**대체 불변식 — 「측정된 시간 상한」 형식**(T-1B-FIX · ⛔점근 등급 단독 표기 폐기 · 정본 형식과 그
/// 기여 항·측정 차림·전제는 `TRASH_CERTIFY_BUDGET_CHECK_NODES` 머리의 **1-b 정본 불변식** 블록에 있다):
/// > **종단 구간(마지막 마감 검사 → `Ready` 발행)의 초과는 측정된 `T_terminal ≈ 2.6 ms` 이내다**
/// > — ★**이 문장도 자기 단서를 품는다**(⒤와 **같은 결함**이므로 같이 고친다): 2.6 ms 는 **잰 상한이
/// > 아니라 선언 유계 512 에 측정 단가를 곱한 파생값**이고 차림은 **debug · 이 기계**다 ⇒ ★**근거이지
/// > 보장이 아니다**(아래 완충과 같은 말이다 — 머리만 읽어도 보이게 여기에 둔다). —
/// > 기여 항은 **인증·거부된 이름 하나당 5.12 µs / 1.70 µs**(한계단가)이고 개수는
/// > **`TRASH_PREVIEW_MAX_NAMES`(512)로 선언 유계**이며(512 × 5.12 µs ≈ 2.6 ms), 순회 입력(노드 수·캐시
/// > 바이트·디렉터리 폭)에 **비례하는 항은 없다**(⒟′-1 이 캐시 쓰기를 구간 밖으로 옮겼다).
/// > ⛔**여기에도 무계 항 하나가 T 밖에 있다**: 마지막 항목이 거부된 경우의 **원장 append 1건의 `fsync`**
/// > (디스크 지연은 측정으로 묶이지 않는다 ⇒ T 에 넣지 않고 분리해 적는다).
/// > **측정 차림**: debug 빌드 · 이 기계 · 프로브 1회 · 한계단가 n=1024. ⛔release 미측정.
/// > ⇒ ★**근거이지 보장이 아니다**(묶는 것은 시간이 아니라 **개수**다 — 512 라는 **선언 유계**에 단가를
/// > 곱한 **파생값**이고, 디스크·스케줄 지연은 그 곱에 들어 있지 않다). ★이 완충은 **교체 때 사라졌던
/// > 것을 되살린 것**이다 — 바로 아래 옛 문면의 마지막 줄이 그것을 품고 있었다(S42 §3 ⒦).
/// 아래 옛 문면(등급 표기)은 **이 블록으로 교체됐다** — 값과 전제는 위 두 줄이 정본이다:
/// > (교체됨) 잔여 = 순회 입력에 비례하는 항 없음 + 이름 수에 비례하는 항(상한 **`TRASH_PREVIEW_MAX_NAMES`
/// > (512)로 선언 유계). 단위: **인증된 이름 하나당 약 5.12 µs** · **거부된 이름 하나당 약 1.70 µs** ·
/// > **총량 ≤ 512 × 5.12 µs ≈ 2.6 ms** — ★차림: **debug 측정 · release 미측정** · 이 기계 · 프로브 1회
/// > ⇒ **근거이지 보장이 아니다**(묶는 것은 시간이 아니라 **개수**다).
/// ★**의존을 적는다**(부서장 ⒞): 뒤 항의 유계는 **상한 상수에 의존한다** — 그 상한은 **RPC 인자 관문
/// `names_param` 에만** 있으므로, 그 관문을 거치지 않고 `certify_names` 를 부르는 호출자가 생기면
/// **이 항은 다시 유계가 아니다**. ⛔★**이 의존은 행위 검사로 받치고, 「호출자 전수」는 written convention
/// 으로 강등했다**(수용 조건 ⒦ · 상수 주석에 그 갈림을 적었다) — 기계가 보는 것은 **관문의 거부 행위**이고,
/// 「관문 밖 호출자가 없다」는 **선언**이다. ⛔**그 전에는 세션 예산에만 얹혀 있었고, 그 파생 상한은 약 23.3 초
/// (예산의 19 %)였다** — 유계가 다른 유계에 얹혀 있었고 파생값이 예산과 같은 자리수였다(= 보장이 아니었다).
///
/// ★**이 불변식은 「종단 구간」의 것이다 — 「순회 구간」의 불변식은 따로 있다**(1-b 정본 블록):
/// > **순회 구간의 초과는 측정된 `T_walk ≈ 67.21 ms`(노드 상한 F=5,000,000 에서 측정) 이내다** —
/// > 기여 항 **넷**은 **남은 자식 ≤128 = 817.28 µs · 프런티어 push 최악 1회 = 130.792 µs ·
/// > 프런티어 해제 = 66.215959 ms · 거부 레코드+`ReadDir` 닫기 = 41.209 µs** 이고 **해제가 지배한다**
/// > (⛔**넷 다 측정이지만 넷 다 최악의 증명은 아니다** — 라벨은 정본 블록의 ①표 네 번째 칸에 있다).
/// > 전수·차림·전제·비율은
/// > `TRASH_CERTIFY_BUDGET_CHECK_NODES` 머리의 **1-b 정본 불변식** 블록이 정본이다.
/// ⇒ 두 구간을 **한 문장으로 합치지 않는다**: 하나는 **순회 도중**(여기서 멈추면 거부가 정상 발부된다) ·
/// 하나는 **종단 뒤**(여기서 늘어나면 답이 늦는다). 성질이 다르므로 유계도 따로 적는다.
///
/// 앞 항의 뜻: 마지막 마감 검사 뒤 `Ready` 발행까지의 잔여에는 **순회 입력에 비례하는 작업·루프가
/// 없다** — **걷는 노드 수**·**캐시 파일 바이트**·**디렉터리 폭** 어디에도 비례하지 않는다.
/// 그 구간에 실제로 남는 것은 셋이고, 셋 다 **읽어서 센 것**이다:
/// ① **순회 꼬리 = 0 노드**(인증 성공 경로). 마감 검사가 순회 **종료 뒤에도** 한 번 더 있고
///   (`certify_entry_until` 의 루프 밖 검사) 그 뒤에는 `dev_ino` 와 인증서 조립뿐이다.
///   ★예외는 마지막 항목이 **자식 루프 안에서** 거부된 경로(`unreadable`·`desc_device`·`too_many`)이고,
///   그때 꼬리는 **< `TRASH_CERTIFY_BUDGET_CHECK_NODES`(128) 노드** 다 — 검사 간격이 상한이므로
///   **입력 크기와 무관한 상수**다.
/// ② **「답 조립」 — 모집단은 「인증된 이름」과 「거부된 이름」 둘이다**(부서장 ⒜에 대한 내 답 · ★그의 전제가
///   한 자리에서 깨진다): `order_digest` 의 이름별 sha256(**인증**) · 확인 레코드의 키 사본·정렬(**인증**) ·
///   그리고 ★`json!({"entries": shown, "refused": refusals, …})` 는 임의 표현식에 대해
///   **`to_value(&…)` 로 깊은 복사**를 한다(serde_json 1.0.150 `src/macros.rs` 의 `($other:expr)` 팔 —
///   잠긴 판에서 확인) ⇒ **거부 목록도 그 구간에서 다시 돈다**. ⒤요청 이름 목록(`names`) 자체는 **돌지 않는다**
///   (`shown`·`refusals` 는 **시도된 항목**만 담는다) ⒥두 모집단 모두 **항목마다 syscall 비용**을 치르고
///   각 시도 앞에 **마감 검사**가 있다.
///   ★**그래서 예산만으로는 유계가 아니다**(실측): 가장 싼 인증(빈 디렉터리)이 **26.4 µs/건**이라 예산 120 s 에
///   **약 4.55 M건**이 들어가고 구간 단가 5.12 µs 를 곱하면 **약 23.3 초**다. ⇒ **부서장 판정대로 상한을
///   신설했다**: `TRASH_PREVIEW_MAX_NAMES`(512) · 초과는 **거부**(거짓이 아니다 · 잡을 띄우지 않는다).
///   ⛔**이름 정렬·중복제거는 기각됐다**(부서장): 그러면 모달 목록·원장 행·**삭제 집행 순서**가 클릭 순 →
///   이름 순으로 바뀐다 — **내부 불변식의 편의로 오너가 보는 순서를 바꾸는 것**이기 때문이다.
/// ③ **최대 1건의 원장 추가(= `fsync` 1회)** — 마지막 항목이 거부된 경우뿐이다(생산자 활성·최근 쓰기·
///   인증 실패 세 분기가 각각 `record_refusal` 1회 뒤 `continue` 다). **건수는 입력에 따라 늘지 않고**,
///   지연은 **무계**다. ★이것을 유예하지 않은 이유는 `certify_names` 머리에 적었다(정확성 ≠ 가속).
///
/// ★**결박 검체가 재는 것과 **재지 않는 것**(검사가 무엇을 재는지 분명히 한다 · 부서장 지시):
/// 검체들은 **앞 항**(순회 입력 비례 작업이 그 구간에 들어오면 FAIL)을 잰다. ⛔**뒤 항(이름 수 비례)은
/// 그 검사의 대상이 아니다** — 그것을 재는 것은 `a_preview_of_more_names_than_the_cap_is_refused_...`
/// 의 **상한**이고, 상한 아래의 단가는 **측정값**(위 µs)이지 단언이 아니다.
/// ★**결박 검체는 시간이 아니라 구조를 잰다**(S20 §2 · ⛔ms 단언 금지):
/// `the_size_cache_is_not_written_until_the_terminal_decision_is_made`(캐시 바이트 항이 구간 밖임) ·
/// `a_refusal_is_in_the_ledger_before_the_terminal_decision`(③을 유예로 「최적화」하면 빨개진다) ·
/// (선존) `the_only_entrance_to_ready_is_a_completed_walk`(예산 초과가 `Ready` 로 못 간다 · **재협상 금지**).
/// ───────────────────────────────────────────────────────────────────────────────────────────
fn preview_outcome_phase(out: NamesOutcome) -> (PreviewPhase, Option<Value>, Option<Value>) {
    match out {
        NamesOutcome::Done {
            shown,
            refusals,
            certs,
        } => (
            PreviewPhase::Ready,
            Some(preview_payload(shown, refusals, certs)),
            None,
        ),
        NamesOutcome::InvalidName => (
            PreviewPhase::Failed,
            None,
            Some(job_refusal("(names)", "invalid_params", INVALID_NAME_MSG)),
        ),
        NamesOutcome::LedgerUnwritable => (
            PreviewPhase::Failed,
            None,
            Some(job_refusal("(ledger)", R_LEDGER_UNWRITABLE, LEDGER_FAIL_MSG)),
        ),
        NamesOutcome::SessionExpired(r) => (
            PreviewPhase::Failed,
            None,
            Some(refusal_json("(session)", &r)),
        ),
    }
}

/// `trash.preview_start` — 배경 잡을 띄우고 **즉시** `job_id` 를 돌려준다(순회를 기다리지 않는다).
///
/// ★최상위 불변식: 이 함수는 파일을 **한 번도 걷지 않는다**. 하는 일은 이름 안전성 검사와 맵 등록뿐이다.
/// ★선언 고지(설계와 다른 자리 · 조용히 고르지 않는다): 델타 §1.3 은 「이름 안전성 **및 생산자 표식**
/// 사전 검증 통과 시」 잡을 만들라고 적는다. 생산자 표식 검증은 **배경 순회 안에서** 한다 —
/// 여기서 한 번 더 하면 같은 거부가 **원장에 두 번** 적히고(그 검사는 `record_refusal` 을 부른다),
/// 두 번 적힌 거부는 원장을 읽는 사람에게 **두 번 거부당한 것**으로 보인다. 이름 안전성만 선검사한다
/// (그것은 원장에 쓰지 않는다).
fn preview_start(id: &Value, root: &Path, state: &Path, params: &Value) -> Reply {
    let names = match names_param(id, params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if names.iter().any(|n| !safe_name(n)) {
        return Reply::Single(err_response(id, "invalid_params", INVALID_NAME_MSG));
    }
    preview_sweep_expired();
    let job_id = new_id("pv");
    if !preview_lock_try_acquire(&job_id) {
        return Reply::Single(err_response(
            id,
            R_PREVIEW_BUSY,
            "이미 도는 미리보기가 있다 — 기다리지 않고 거부한다(그것이 끝나거나 만료된 뒤 다시 누르십시오)",
        ));
    }
    let now_i = Instant::now();
    preview_jobs().lock().unwrap().insert(
        job_id.clone(),
        PreviewJob {
            job_id: job_id.clone(),
            phase: PreviewPhase::Running,
            started: now_i,
            last_polled: now_i,
            nodes: 0,
            result: None,
            refusal: None,
        },
    );
    let root_o = root.to_path_buf();
    let state_o = state.to_path_buf();
    let jid = job_id.clone();
    // ★순회는 **여기서부터 디스패치 경로 밖**이다(D4 stall pair).
    let spawned = preview_spawn(move || {
        let deadline =
            Instant::now() + std::time::Duration::from_secs(TRASH_PREVIEW_SESSION_MAX_SECS);
        let prog_id = jid.clone();
        let prog = move |n: usize| {
            if let Some(j) = preview_jobs().lock().unwrap().get_mut(&prog_id) {
                j.nodes = n;
            }
        };
        let (out, pending) = certify_names(
            &root_o,
            &state_o,
            &names,
            Some(deadline),
            Some(&prog as &dyn Fn(usize)),
        );
        let (phase, result, refusal) = preview_outcome_phase(out);
        if let Some(j) = preview_jobs().lock().unwrap().get_mut(&jid) {
            j.phase = phase;
            j.result = result;
            j.refusal = refusal;
        }
        // ★완료·실패 **즉시** 빗장을 놓는다(상태 전이표 3·4행).
        preview_lock_release(&jid);
        // ★⒟′-1 — 유예한 크기 캐시는 **여기서** 반영한다: 종단 판정 **뒤**이고 빗장 해제 **뒤**다.
        //   ⒜「마지막 마감 검사 → `Ready` 발행」 구간에서 **캐시 파일 크기에 비례하는 작업**이 빠진다
        //   ⒝빗장 밖이므로 이 I/O 가 **다음 미리보기를 기다리게 만들지 않는다**
        //   ⒞실패·예산 소진으로 끝난 잡의 유예분도 반영한다 — 그 측정은 **참이고**, 버리면 다음 목록이
        //     이유 없이 다시 걷는다(캐시는 가속이고, 잡의 결말과 캐시의 진실성은 별 축이다).
        size_cache_flush(&root_o, &pending);
    });
    // ★MJR-2 — 생성 실패를 **버리지 않는다**. 버리면 ⒜잡이 영구 `Running` ⒝빗장이 영구 점유이고,
    //   빗장을 놓는 코드는 **실행되지 않는 클로저 안**이다 ⇒ 이후 모든 `preview_start` 가 영구
    //   `preview_busy`, 모든 `preview_poll` 이 영구 `Running`, 빠져나오는 길은 데몬 재시작뿐이다
    //   (= 한 번의 생성 실패가 데몬 수명 동안 휴지통 미리보기를 죽인다).
    // ★그래서 셋을 **이 자리(스레드 본문 밖)** 에서 한다: 잡을 `Failed` 로 종결 · 빗장 즉시 해제 ·
    //   호출자에게 거부를 **지금** 돌려준다(job_id 를 주고 영원히 폴링하게 두지 않는다).
    if let Err(e) = spawned {
        let detail = format!(
            "미리보기 일꾼을 띄우지 못했다: {e} — 잡을 즉시 종결하고 빗장을 놓았다(다시 시도하십시오)"
        );
        if let Some(j) = preview_jobs().lock().unwrap().get_mut(&job_id) {
            j.phase = PreviewPhase::Failed;
            j.result = None;
            j.refusal = Some(job_refusal("(worker)", R_PREVIEW_NO_WORKER, &detail));
        }
        preview_lock_release(&job_id);
        return Reply::Single(err_response(id, R_PREVIEW_NO_WORKER, &detail));
    }
    Reply::Single(ok_response(id, json!({"job_id": job_id})))
}

/// `trash.preview_poll` — 맵만 읽고 **즉시** 돌아온다(순회를 기다리지 않는다).
fn preview_poll(id: &Value, params: &Value) -> Reply {
    preview_sweep_expired();
    let jid = params.get("job_id").and_then(Value::as_str).unwrap_or("");
    let mut m = preview_jobs().lock().unwrap();
    let Some(j) = m.get_mut(jid) else {
        return Reply::Single(err_response(
            id,
            R_JOB_UNKNOWN,
            "그 job_id 를 이 데몬이 모른다 — 만료됐거나 데몬이 다시 떴다(새 미리보기를 시작하십시오)",
        ));
    };
    j.last_polled = Instant::now();
    let mut out = json!({
        "status": preview_status_str(j.phase),
        "progress": {"nodes": j.nodes, "elapsed_secs": j.started.elapsed().as_secs()},
    });
    if let Some(r) = &j.result {
        out["result"] = r.clone();
    }
    if let Some(r) = &j.refusal {
        out["refusal"] = r.clone();
    }
    Reply::Single(ok_response(id, out))
}

pub const LEDGER_FAIL_MSG: &str = "거부를 원장에 기록할 수 없어 진행하지 않았다 — \
    읽을 수 없는 거부는 거부가 아니다(아무 것도 건드리지 않았다)";

/// 잡 거부의 모양 — ★화면(`ui/src/main.ts:1438` `refused?: { name, reason_code, boundary, detail }[]` ·
/// `ui/src/trashconfirm.ts:172` `reason_code`)이 읽는 열 이름은 **`reason_code` 하나**다.
/// 순회가 만든 거부는 `refusal_json` 이 그 모양으로 적고, 순회 **밖**에서 난 거부(이름·원장·일꾼)는
/// 이 함수가 적는다 — 한 응답 안에 `code` 와 `reason_code` 두 이름이 섞이면 화면이 사유를 놓친다.
fn job_refusal(name: &str, code: &str, detail: &str) -> Value {
    json!({"name": name, "reason_code": code, "boundary": Value::Null, "detail": detail})
}

fn refusal_json(name: &str, r: &Refusal) -> Value {
    json!({"name": name, "reason_code": r.code, "boundary": r.boundary, "detail": r.detail})
}

/// 명령 — 확인을 한 번 쓰고, 의도를 원장에 적고, 백그라운드에서 순차 집행한다. 즉시 돌아온다.
fn delete(id: &Value, root: &Path, state: &Path, params: &Value, caller_pid: Option<u32>) -> Reply {
    if delete_killswitched() {
        return Reply::Single(err_response(
            id,
            "refused",
            "CYS_TRASH_DELETE=off — 이 기계에서 삭제가 꺼져 있다(목록·표시는 그대로 동작한다)",
        ));
    }
    if !delete_supported() {
        return Reply::Single(err_response(
            id,
            "unsupported_platform_pending_validation",
            "이 빌드에서는 휴지통 삭제가 꺼져 있다 — 윈도우 삭제는 12.W 검증을 통과한 빌드에서만 켠다 \
             (목록·표시는 그대로 동작한다)",
        ));
    }
    let cid = params.get("confirm_id").and_then(Value::as_str).unwrap_or("").to_string();
    let names = str_list(params, "names").unwrap_or_default();
    let digest = params.get("digest").and_then(Value::as_str).unwrap_or("").to_string();
    let c = match confirms().lock().unwrap().remove(&cid) {
        // ★한 번만 쓴다 — remove 로 꺼내므로 재생(replay)이 구조적으로 불가능하다.
        Some(c) => c,
        None => {
            return Reply::Single(err_response(
                id,
                R_CONFIRM_EXPIRED,
                "확인 레코드가 없다(이미 썼거나 데몬이 다시 떴다) — 새 미리보기가 필요하다",
            ))
        }
    };
    debug_assert_eq!(c.id, cid, "확인 레코드의 키와 본문 id 가 어긋났다");
    let age = c.issued.elapsed().as_secs();
    if confirm_expired(age, TRASH_CONFIRM_TTL_SECS) {
        let r = Refusal {
            code: R_CONFIRM_EXPIRED,
            boundary: None,
            detail: format!("확인이 {age}s 전에 발급됐다(TTL {TRASH_CONFIRM_TTL_SECS}s · 같음도 만료) — 새 미리보기가 필요하다"),
        };
        if record_refusal(state, "(order)", &cid, &r).is_err() {
            return Reply::Single(err_response(id, R_LEDGER_UNWRITABLE, LEDGER_FAIL_MSG));
        }
        return Reply::Single(err_response(id, r.code, &r.detail));
    }
    let mut asked = names.clone();
    asked.sort();
    if asked != c.names || digest != c.digest {
        let r = Refusal {
            code: R_NOT_IN_PREVIEW,
            boundary: None,
            detail: "명령의 이름 집합·요약이 미리보기와 다르다 — 새 미리보기가 필요하다".into(),
        };
        if record_refusal(state, "(order)", &cid, &r).is_err() {
            return Reply::Single(err_response(id, R_LEDGER_UNWRITABLE, LEDGER_FAIL_MSG));
        }
        return Reply::Single(err_response(id, r.code, &r.detail));
    }
    // 단일 삭제자 — try-lock. 기다리지 않는다(기다리면 그것이 stall 이다).
    if !deleter_try_acquire() {
        return Reply::Single(err_response(
            id,
            "busy",
            "다른 삭제 명령이 진행 중이다 — 끝난 뒤 다시 명령한다(대기하지 않는다)",
        ));
    }
    let order_id = new_id("od");
    // (1) 의도 기록 — **첫 바이트 전에**. 쓸 수 없으면 아무것도 하지 않는다.
    let intent = json!({
        "kind": "intent", "ts": now_epoch(), "order_id": order_id,
        "command": {"identity": "token_holder",
                    "identity_verified": "operator_token_and_not_in_pane_of_this_daemon",
                    "human_verified": false, "operator_token_ok": true,
                    "caller_pid": caller_pid, "caller_in_pane": false,
                    // 신고값이며 검증되지 않는다 — 읽는 사람이 오해하지 않게 이름에 적는다.
                    "claimed_client_unverified": params.get("client").and_then(Value::as_str),
                    "confirm_id": cid, "digest": c.digest},
        "entries": c.names.iter().map(|n| {
            let cert = &c.certs[n];
            json!({"name": n, "nodes": cert.nodes, "bytes": cert.bytes,
                   "newest_mtime": cert.newest_mtime, "digest": cert.digest})
        }).collect::<Vec<_>>(),
    });
    if let Err(e) = ledger_append(state, &intent) {
        deleter_release();
        return Reply::Single(err_response(
            id,
            R_LEDGER_UNWRITABLE,
            &format!("{LEDGER_FAIL_MSG} (의도 기록 실패: {e})"),
        ));
    }
    orders().lock().unwrap().insert(
        order_id.clone(),
        OrderState {
            order_id: order_id.clone(),
            started: now_epoch(),
            finished: None,
            per_entry: vec![],
            running_long: false,
        },
    );
    // 집행은 별도 스레드 — dispatch 스레드를 쥐지 않는다(큐·배달 경로에 영향 0).
    let root2 = root.to_path_buf();
    let state2 = state.to_path_buf();
    let oid2 = order_id.clone();
    // 빗장은 **이미 잡혀 있다**(위 try_acquire). 집행 스레드가 끝에서 놓는다.
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut outs = Vec::new();
        for name in &c.names {
            // CK: 항목 **사이마다** 확인 수명을 다시 본다(지났으면 남은 항목은 손대지 않는다).
            if confirm_expired(c.issued.elapsed().as_secs(), TRASH_CONFIRM_TTL_SECS) {
                let r = Refusal {
                    code: R_CONFIRM_EXPIRED,
                    boundary: None,
                    detail: "집행 중 확인이 만료됐다 — 남은 항목은 손대지 않는다(새 미리보기 필요)".into(),
                };
                let _ = record_refusal(&state2, &oid2, name, &r);
                outs.push(EntryOutcome {
                    name: name.clone(),
                    outcome: "refused".into(),
                    reason_code: Some(r.code.to_string()),
                    boundary: None,
                    bytes_before: None,
                    duration_ms: 0,
                    remaining: None,
                    errors: vec![],
                });
                continue;
            }
            let out = execute_one(
                &root2, name, &c.certs[name], &real_dev, now_epoch(), &state2, &oid2,
            );
            outs.push(out);
            if t0.elapsed().as_secs() >= TRASH_REMOVE_STALL_REPORT_SECS {
                if let Some(o) = orders().lock().unwrap().get_mut(&oid2) {
                    o.running_long = true; // 보고만 — 중단이 아니다.
                }
            }
            if let Some(o) = orders().lock().unwrap().get_mut(&oid2) {
                o.per_entry = outs.clone();
            }
        }
        let result = json!({
            "kind": "result", "ts": now_epoch(), "order_id": oid2,
            "per_entry": outs.iter().map(|e| json!({
                "name": e.name, "outcome": e.outcome, "reason_code": e.reason_code,
                "boundary": e.boundary, "bytes_before": e.bytes_before,
                "duration_ms": e.duration_ms, "remaining": e.remaining, "errors": e.errors,
            })).collect::<Vec<_>>(),
        });
        let _ = ledger_append(&state2, &result);
        if let Some(o) = orders().lock().unwrap().get_mut(&oid2) {
            o.per_entry = outs;
            o.finished = Some(now_epoch());
        }
        deleter_release();
    });
    Reply::Single(ok_response(id, json!({"order_id": order_id})))
}

fn str_list(params: &Value, key: &str) -> Option<Vec<String>> {
    params.get(key)?.as_array().map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

/// 휴지통 뿌리의 **직속 자식 이름**만 받는다 — 경로 구분자·`..`·절대경로는 전부 거부.
pub fn safe_name(n: &str) -> bool {
    !n.is_empty()
        && n != "."
        && n != ".."
        && !n.contains('/')
        && !n.contains('\\')
        && !n.contains('\0')
}

fn new_id(prefix: &str) -> String {
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    h.update(now_epoch().to_bits().to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    format!("{prefix}-{}", &hex32(h.finalize().as_slice())[..16])
}

// ─── D-r2-5 M3: `.producer` 표식 **읽기** — 생산자가 돌고 있으면 거부한다 ─────────────────────
/// 표식 상태 판정(순수 · 주입 가능). `alive` 는 pid+start_token 생존 판정의 결과다.
#[derive(Debug, Clone, PartialEq)]
pub enum ProducerState {
    /// 표식 없음 — 구 버전이나 손으로 만든 항목(M1·M2 만으로 보호된다).
    Unknown,
    /// 돌고 있다 → **거부**(`producer_active`).
    ActiveOpen,
    /// 열린 채 죽었다 → 지울 수 있지만 "생산자가 끝내지 못함(중단)" 표지를 띄운다.
    InterruptedOpen,
    /// 정상 종료(complete·aborted).
    Settled,
    /// 생존 판정 불가 → **살아 있는 것으로 취급**(fail closed).
    Undeterminable,
}

/// `<entry>/.producer` 를 읽어 상태를 돌려준다. 파일이 깨졌으면 Undeterminable(= 살아 있는 쪽).
pub fn producer_state(entry: &Path, alive: &dyn Fn(u32, &str) -> Option<bool>) -> ProducerState {
    let p = entry.join(".producer");
    let txt = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProducerState::Unknown,
        Err(_) => return ProducerState::Undeterminable,
    };
    let v: Value = match serde_json::from_str(&txt) {
        Ok(v) => v,
        Err(_) => return ProducerState::Undeterminable,
    };
    match v.get("state").and_then(Value::as_str) {
        Some("complete") | Some("aborted") => ProducerState::Settled,
        Some("open") => {
            let pid = v.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32;
            let tok = v.get("start_token").and_then(Value::as_str).unwrap_or("");
            match alive(pid, tok) {
                Some(true) => ProducerState::ActiveOpen,
                Some(false) => ProducerState::InterruptedOpen,
                None => ProducerState::Undeterminable,
            }
        }
        _ => ProducerState::Undeterminable,
    }
}

/// 운영 기본 생존 판정 — **pid 존재만 본다.**
/// ★정정(구현을 다시 읽고 고친 주석): `start_token` 은 **대조하지 않는다**(인자 이름이 `_` 로
/// 시작하는 이유다). 종전 주석은 "토큰을 대조할 수 없으면 판정 불가를 돌려준다"고 적었는데 코드는
/// 그렇게 하지 않았다 — 그리고 그렇게 바꾸면 설계 I-5·I-9 의 "open + 죽은 pid → 중단 표지와 함께
/// 삭제 가능" 경로가 영구히 사라진다(토큰이 실린 모든 표식이 '살아 있음'이 되므로). 그래서 **주석을
/// 코드에 맞췄다.** 남는 한계는 정직하게 적는다: **pid 재사용**은 '살아 있다'로 오판될 수 있고,
/// 그 오판의 방향은 거부(안전)다. 세 writer 는 토큰을 기록만 해 둔다(나중에 대조할 수 있게).
/// 윈도우에서는 `kill(0)` 이 없어 판정 불가다(★윈도우에서 다르게 도는 자리 3).
pub fn real_alive(pid: u32, _start_token: &str) -> Option<bool> {
    if pid == 0 {
        return None;
    }
    #[cfg(unix)]
    {
        // signal 0 = 존재 확인만. 권한 오류(EPERM)는 "있다"는 뜻이다.
        let rc = unsafe { libc::kill(pid as i32, 0) };
        if rc == 0 {
            return Some(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) => Some(true),
            Some(libc::ESRCH) => Some(false),
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 생산자 상태가 삭제를 막는가 — `ActiveOpen` 과 `Undeterminable` 은 막는다(fail closed).
pub fn producer_blocks(st: &ProducerState) -> bool {
    matches!(st, ProducerState::ActiveOpen | ProducerState::Undeterminable)
}

/// 화면 표지(칩) 문구 — 상태별로 하나.
pub fn producer_chip(st: &ProducerState) -> &'static str {
    match st {
        ProducerState::Unknown => "producer unknown (older version or hand-made)",
        ProducerState::ActiveOpen => "producer is running",
        ProducerState::InterruptedOpen => "producer did not finish (interrupted)",
        ProducerState::Settled => "",
        ProducerState::Undeterminable => "producer liveness undeterminable (treated as running)",
    }
}

/// ★안전 방향 전용 kill switch(S8:239) — `CYS_TRASH_DELETE=off` 면 삭제만 끈다(목록·표시는 그대로).
/// 위험한 방향으로는 어떤 환경변수도 가지 않는다.
pub fn delete_killswitched() -> bool {
    std::env::var("CYS_TRASH_DELETE")
        .map(|v| v.trim().eq_ignore_ascii_case("off"))
        .unwrap_or(false)
}

/// 휴지통 뿌리의 건전성 — 심링크거나 디렉터리가 아니면 **동작하지 않는다**(S8:115).
pub fn root_sane(root: &Path) -> bool {
    match std::fs::symlink_metadata(root) {
        Ok(md) => md.is_dir() && !md.file_type().is_symlink(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 격리된 임시 뿌리. 시험은 **실제 휴지통을 건드리지 않는다**(살아 있는 자료 무접촉).
    fn rig(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "cys-s8-{tag}-{}-{}",
            std::process::id(),
            now_epoch() as u64
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("rig");
        p
    }
    fn mk(p: &Path, rel: &str, bytes: &[u8]) {
        let f = p.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, bytes).unwrap();
    }
    fn ledger_lines(state: &Path) -> Vec<Value> {
        std::fs::read_to_string(state.join(DELETE_LEDGER))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("원장 줄은 JSON 이어야 한다"))
            .collect()
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★C-R4 KNOWN-FAILURE 픽스처 — 이 기계에는 휴지통 뿌리 아래 교차장치 디렉터리가 **0개**다
    //   (부서장 실측 · 내 재측정 2026-10-12 15:02 에도 cross_device_nodes=0). 그래서 초록은
    //   아무 것도 증명하지 못한다 — 장치 경계를 **의도적으로 만들어** 거부를 재는 시험이다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn cr4_descendant_device_refuses_the_whole_entry_and_writes_a_refusal_record() {
        let root = rig("crossdev");
        let entry = root.join("dept-9-1760000000");
        mk(&entry, "state/keep.txt", b"sole copy");
        mk(&entry, "deep/mount/inside.txt", b"on another device");
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        // 경계에서 장치 번호를 주입한다(합성 마운트).
        let boundary = entry.join("deep/mount");
        let b2 = boundary.clone();
        let dev_of = move |p: &Path, md: &std::fs::Metadata| -> u64 {
            if p == b2.as_path() || p.starts_with(&b2) {
                999_999
            } else {
                real_dev(p, md)
            }
        };
        let r = certify_entry(&entry, &dev_of).expect_err("교차장치면 인증이 거부돼야 한다");
        assert_eq!(r.code, R_DESC_DEVICE, "사유 코드가 장치 경계여야 한다");
        assert_eq!(
            r.boundary.as_deref().map(Path::new),
            Some(boundary.as_path()),
            "거부는 인증을 무너뜨린 **경계 경로**를 들고 있어야 한다 \
             (★표시 문자열이 아니라 경로로 비교한다 — 윈도우는 같은 경로를 \
             역슬래시로 적어서 문자열 비교만 깨졌다 · S68 ①)"
        );
        // ★항목 전체 거부 — 경계 바깥의 파일도 그대로 있어야 한다(부분 제거 금지).
        assert!(entry.join("state/keep.txt").exists(), "경계 밖 파일이 사라졌다 = 부분 제거");
        assert!(boundary.join("inside.txt").exists());
        // ★거부는 기록된다(C-R2).
        record_refusal(&state, "od-test", "dept-9-1760000000", &r).expect("거부 기록");
        let ls = ledger_lines(&state);
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0]["kind"], "refusal");
        assert_eq!(ls[0]["reason_code"], R_DESC_DEVICE);
        assert_eq!(
            Path::new(ls[0]["boundary"].as_str().expect("원장의 boundary 는 문자열이다")),
            boundary.as_path(),
            "원장도 같은 **경계 경로**를 들고 있어야 한다(경로로 비교 · S68 ①)"
        );
        assert_eq!(ls[0]["human_verified"], false, "원장은 사람을 증명하지 않는다");
        assert_eq!(ls[0]["identity"], "token_holder");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 집행 경로까지 포함한 교차장치 거부 — **한 바이트도 지워지지 않는다**.
    #[test]
    fn cr4_execute_refuses_and_removes_nothing_when_a_descendant_device_appears() {
        let root = rig("crossdev-exec");
        let name = "dept-9-1760000000";
        let entry = root.join(name);
        mk(&entry, "state/keep.txt", b"sole copy");
        mk(&entry, "deep/mount/inside.txt", b"other device");
        let state = root.join("state-dir");
        std::fs::create_dir_all(&state).unwrap();
        // 미리보기는 장치가 하나인 세계에서 통과했다고 가정한다(기준 인증서).
        let base = certify_entry(&entry, &real_dev).expect("기준 인증");
        // 집행 시점에 경계가 나타난다(마운트가 그 사이에 생긴 경우).
        let b2 = entry.join("deep/mount");
        let dev_of = move |p: &Path, md: &std::fs::Metadata| -> u64 {
            if p.starts_with(&b2) { 424_242 } else { real_dev(p, md) }
        };
        let out = execute_one(&root, name, &base, &dev_of, now_epoch() + 100_000.0, &state, "od-x");
        assert_eq!(out.outcome, "refused");
        assert_eq!(out.reason_code.as_deref(), Some(R_DESC_DEVICE));
        assert!(entry.exists(), "항목이 봉인·제거돼선 안 된다");
        assert!(entry.join("state/keep.txt").exists(), "한 바이트도 지워지지 않아야 한다");
        assert!(!root.join(format!("{TRASH_SEAL_PREFIX}{name}")).exists(), "봉인조차 하지 않는다");
        let ls = ledger_lines(&state);
        assert!(ls.iter().any(|l| l["reason_code"] == R_DESC_DEVICE), "거부 기록이 있어야 한다");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── ADDENDUM-1 + r2 m1: 두 창의 경계 오라클(같음의 방향이 서로 다르다) ─────────────────
    #[test]
    fn a1_m1_recent_boundary_599_600_601() {
        let now = 1_800_000_000.0;
        let w = TRASH_RECENT_WINDOW_SECS; // 600
        assert!(is_recent(now - (w as f64 - 1.0), now, w), "599s = 최근 = 거부");
        assert!(!is_recent(now - w as f64, now, w), "600s = 최근 아님 = 허용(r2 m1 이 ADDENDUM-1 을 대체)");
        assert!(!is_recent(now - (w as f64 + 1.0), now, w), "601s = 허용");
    }
    #[test]
    fn a1_unknown_recency_is_recent_fail_closed() {
        let now = 1_800_000_000.0;
        let w = TRASH_RECENT_WINDOW_SECS;
        assert!(is_recent(now + 5_000.0, now, w), "미래 mtime = 최근 = 거부");
        assert!(is_recent(f64::NAN, now, w), "판독 불가(NaN) = 거부");
        assert!(is_recent(f64::INFINITY, now, w), "비유한 = 거부");
        assert!(is_recent(now - 10_000.0, f64::NAN, w), "지금을 모르면 = 거부");
    }
    #[test]
    fn a1_m1_confirm_ttl_boundary_equality_expires() {
        let t = TRASH_CONFIRM_TTL_SECS; // 300
        assert!(!confirm_expired(t - 1, t), "299s = 쓸 수 있다");
        assert!(confirm_expired(t, t), "300s = 만료(같음은 만료다)");
        assert!(confirm_expired(t + 1, t), "301s = 만료");
    }

    // ─── D-r2-9: 유예 시계는 이름의 스탬프다(생산자 두 형식 · 미지 = 오래되지 않음) ──────────
    #[test]
    fn r2_old_is_read_from_the_name_stamp_in_both_formats() {
        // ② 압축 UTC — 실 엔트리의 형식.
        let iso = stamp_epoch_from_name("factory-reset-20260912T090920Z").expect("ISO 스탬프");
        // 기대값은 독립 계산으로 확인했다(python datetime(2026,9,12,9,9,20,tz=utc).timestamp()
        // = 1789204160). 처음 손으로 적은 값은 16시간 어긋났고, 이 대조가 그것을 잡았다.
        assert_eq!(iso, 1_789_204_160.0, "2026-09-12T09:09:20Z 의 epoch");
        // ① epoch 초 — cys-dept·javis_org 의 기본값.
        assert_eq!(stamp_epoch_from_name("dept-3-1760000000"), Some(1_760_000_000.0));
        // 미지 = None = 오래되지 않음.
        assert_eq!(stamp_epoch_from_name("hand-made-folder"), None);
        assert!(!is_old_by_stamp("hand-made-folder", 9_999_999_999.0, 14.0), "미지는 절대 old 가 아니다");
        assert!(is_old_by_stamp("dept-3-1760000000", 1_760_000_000.0 + 15.0 * 86400.0, 14.0));
        assert!(!is_old_by_stamp("dept-3-1760000000", 1_760_000_000.0 + 13.0 * 86400.0, 14.0));
        // 미래 스탬프도 old 가 아니다.
        assert!(!is_old_by_stamp("dept-3-1760000000", 1_700_000_000.0, 14.0));
    }

    // ─── D-r2-3: 집합 요약은 순서 무관이고 내용이 바뀌면 바뀐다 ──────────────────────────────
    #[test]
    fn r2_set_digest_changes_with_content_and_identity_is_carried() {
        let root = rig("digest");
        let e = root.join("dept-1-1760000000");
        mk(&e, "a/one.txt", b"1");
        mk(&e, "b/two.txt", b"22");
        let c1 = certify_entry(&e, &real_dev).unwrap();
        assert_eq!(c1.nodes, 4, "a, a/one.txt, b, b/two.txt");
        assert!(c1.dev != 0 || cfg!(windows));
        // 깊은 곳에 한 줄 추가 — 뿌리 mtime 이 아니라 요약이 움직인다(E1 의 교훈).
        mk(&e, "b/three.txt", b"333");
        let c2 = certify_entry(&e, &real_dev).unwrap();
        assert_ne!(c1.digest, c2.digest, "깊은 변경이 요약에 반영돼야 한다");
        assert_eq!(c2.nodes, 5);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── 집행: 정상·변경·이미 사라짐 ─────────────────────────────────────────────────────────
    #[test]
    fn execute_removes_only_the_previewed_set_and_verifies_with_lstat() {
        let root = rig("exec-ok");
        let name = "dept-2-1760000000";
        let e = root.join(name);
        mk(&e, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let base = certify_entry(&e, &real_dev).unwrap();
        let out = execute_one(&root, name, &base, &real_dev, now_epoch() + 100_000.0, &state, "od-1");
        assert_eq!(out.outcome, "deleted_verified", "{out:?}");
        assert!(!e.exists(), "항목이 사라져야 한다");
        assert!(!root.join(format!("{TRASH_SEAL_PREFIX}{name}")).exists(), "봉인 잔재도 없어야 한다");
        assert_eq!(out.remaining, None);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn execute_refuses_when_the_content_changed_after_the_preview() {
        let root = rig("exec-changed");
        let name = "dept-2-1760000000";
        let e = root.join(name);
        mk(&e, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let base = certify_entry(&e, &real_dev).unwrap();
        mk(&e, "state/added-after-preview.txt", b"new");
        let out = execute_one(&root, name, &base, &real_dev, now_epoch() + 100_000.0, &state, "od-2");
        assert_eq!(out.outcome, "refused");
        assert_eq!(out.reason_code.as_deref(), Some(R_CHANGED));
        assert!(e.exists() && e.join("state/x.txt").exists(), "아무 것도 지워지지 않아야 한다");
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn execute_refuses_a_recent_entry_even_after_a_clean_preview() {
        let root = rig("exec-recent");
        let name = "dept-2-1760000000";
        let e = root.join(name);
        mk(&e, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let base = certify_entry(&e, &real_dev).unwrap();
        // 지금 막 쓴 파일이므로 '지금'을 실제 now 로 주면 최근이다.
        let out = execute_one(&root, name, &base, &real_dev, now_epoch(), &state, "od-3");
        assert_eq!(out.reason_code.as_deref(), Some(R_RECENT), "{out:?}");
        assert!(e.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn execute_says_already_gone_without_claiming_a_deletion() {
        let root = rig("exec-gone");
        let name = "dept-2-1760000000";
        let e = root.join(name);
        mk(&e, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let base = certify_entry(&e, &real_dev).unwrap();
        std::fs::remove_dir_all(&e).unwrap();
        let out = execute_one(&root, name, &base, &real_dev, now_epoch() + 100_000.0, &state, "od-4");
        assert_eq!(out.outcome, "already_gone");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── D-r2-5 M3 표식 읽기 ─────────────────────────────────────────────────────────────────
    #[test]
    fn producer_marker_states_and_what_they_block() {
        let root = rig("producer");
        let e = root.join("dept-5-1760000000");
        std::fs::create_dir_all(&e).unwrap();
        assert_eq!(producer_state(&e, &|_, _| Some(true)), ProducerState::Unknown);
        assert!(!producer_blocks(&ProducerState::Unknown), "표식 없음은 막지 않는다(구 생산자)");
        std::fs::write(e.join(".producer"), br#"{"state":"open","pid":4242,"start_token":"t"}"#).unwrap();
        assert_eq!(producer_state(&e, &|_, _| Some(true)), ProducerState::ActiveOpen);
        assert!(producer_blocks(&ProducerState::ActiveOpen), "돌고 있으면 막는다");
        assert_eq!(producer_state(&e, &|_, _| Some(false)), ProducerState::InterruptedOpen);
        assert!(!producer_blocks(&ProducerState::InterruptedOpen));
        assert_eq!(producer_state(&e, &|_, _| None), ProducerState::Undeterminable);
        assert!(producer_blocks(&ProducerState::Undeterminable), "판정 불가는 살아 있는 쪽(fail closed)");
        std::fs::write(e.join(".producer"), b"{ not json").unwrap();
        assert_eq!(producer_state(&e, &|_, _| Some(false)), ProducerState::Undeterminable);
        std::fs::write(e.join(".producer"), br#"{"state":"complete"}"#).unwrap();
        assert_eq!(producer_state(&e, &|_, _| Some(true)), ProducerState::Settled);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── 이름·뿌리 건전성 ────────────────────────────────────────────────────────────────────
    #[test]
    fn only_direct_children_of_the_root_are_addressable() {
        assert!(safe_name("dept-1-1760000000"));
        for bad in ["", ".", "..", "a/b", "..\\b", "x\0y", "/abs"] {
            assert!(!safe_name(bad), "{bad:?} 는 거부돼야 한다");
        }
    }
    /// ★(S8 · 0.14.51 · 부서장 판정 ③) 원장은 **삭제 경로가 닿는 나무 밖**에 산다 — `trash_root()`(이 파일 :81)와
    /// `state_dir()`(state.rs)의 분리를 **붉어지는 형태로** 박는다. 구조적 사실만 있고 핀이 없으면 장식이다:
    /// 누군가 원장을 휴지통 뿌리로 되돌리면(폐기된 `DESIGN-S8.md:101` 의 자리 · 정본은
    /// `DESIGN-S8-SUPERSESSION-1-ledger-location.md`) 이 검체가 붉어진다.
    /// ★세 번째 단언이 **왜 성분 비교여야 하는지**를 같이 박는다 — 문자열 접두로 비교하면 `…/cys-trash` 가
    /// `…/cys` 로 시작하므로 "뿌리 안이다"로 잘못 읽힌다(`Path::starts_with` 는 성분 단위다).
    #[test]
    fn the_delete_ledger_lives_outside_the_tree_the_delete_path_can_reach() {
        let root = trash_root();
        // 기지 데몬의 제품 규약 소켓 — 그 부모가 상태 폴더다(state.rs `state_dir`).
        let sock = cys::home_dir().join(".local/state/cys/cysd.sock");
        let state = crate::state::state_dir(&sock);
        let ledger = state.join(DELETE_LEDGER);
        assert!(!ledger.starts_with(&root),
                "원장이 휴지통 뿌리 안이다 — 삭제 경로가 자기 원장을 지울 수 있다: {} ⊂ {}",
                ledger.display(), root.display());
        assert!(!state.starts_with(&root) && !root.starts_with(&state),
                "상태 폴더와 휴지통 뿌리가 포함 관계다: {} / {}", state.display(), root.display());
        let rn = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let sn = state.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        assert!(!rn.is_empty() && !sn.is_empty() && rn.starts_with(&sn),
                "전제: 두 이름은 문자열로는 접두 관계다(그래서 성분 비교가 필요하다): {rn} / {sn}");
    }

    #[test]
    fn a_symlinked_or_nondir_root_is_refused() {
        let base = rig("rootsane");
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        assert!(root_sane(&real));
        let file = base.join("afile");
        std::fs::write(&file, b"x").unwrap();
        assert!(!root_sane(&file), "파일이면 동작하지 않는다");
        #[cfg(unix)]
        {
            let link = base.join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert!(!root_sane(&link), "심링크면 동작하지 않는다");
        }
        assert!(!root_sane(&base.join("missing")));
        let _ = std::fs::remove_dir_all(&base);
    }

    // ─── 조언 표지는 조언이다 ────────────────────────────────────────────────────────────────
    #[test]
    fn sole_copy_flag_is_advisory_and_errs_toward_sole() {
        assert!(sole_copy_advisory("factory-reset-20260912T090920Z", &[]));
        assert!(sole_copy_advisory("dept-4-1", &["state".into(), "pack".into()]));
        assert!(sole_copy_advisory("dept-4-1", &["pack-dept-dept-4".into()]));
        assert!(!sole_copy_advisory("dept-4-1", &["workdir".into()]));
    }

    // ─── 목록은 걷지 않는다 ─────────────────────────────────────────────────────────────────
    #[test]
    fn list_does_not_walk_and_reports_pending_sizes() {
        let root = rig("list");
        let e = root.join("dept-7-1760000000");
        mk(&e, "state/deep/a.txt", b"a");
        mk(&e, "pack/b.txt", b"b");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let v = list_payload(&root, &state);
        let ents = v["entries"].as_array().unwrap();
        assert_eq!(ents.len(), 2, "뿌리의 직속 자식만 — state-dir 도 하나로 보인다");
        let me = ents.iter().find(|e| e["name"] == "dept-7-1760000000").unwrap();
        assert_eq!(me["size"]["state"], "pending", "측정 전에는 pending — 목록이 걷지 않는 증거");
        let kids = me["children_depth1"].as_array().unwrap();
        assert_eq!(kids.len(), 2, "깊이 1 만 — 'deep' 은 보이지 않는다");
        assert_eq!(me["sole_copy_advisory"], true);
        assert_eq!(v["constants"]["recent_window_secs"], 600);
        assert_eq!(v["constants"]["confirm_ttl_secs"], 300);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★성찰 A-2 의 수정: 점 파일(`.producer`)은 **오너가 보는 첫 단계 목록에 섞이지 않는다**
    /// (설계 r2 §3.4). 표식을 쓰기 시작한 뒤 생긴 결함이라 시험이 그 조합을 그대로 만든다.
    /// ★성찰 A-1 의 수정: 같은 줄의 `producer_chip` 과 `deletable` 은 **한 번의 판독**에서 나와야
    /// 서로 모순될 수 없다 — 아래의 **불변식**이 그 일치를 잠근다(막는 표지 ⇒ 고를 수 없다).
    /// ★S68 ②: 살아 있는 pid / 죽은 pid 의 **리터럴 매핑은 이 검체가 더 이상 주장하지 않는다** —
    /// non-unix 에서 `real_alive` 가 모든 pid 에 `None` 을 돌려 성립하지 않기 때문이다(D2).
    /// 그 리터럴은 바로 아래 `list_maps_a_live_and_a_dead_producer_pid_to_running_and_interrupted_unix`
    /// (`#[cfg(unix)]`)가 지킨다.
    #[test]
    fn list_hides_dot_files_and_never_contradicts_itself_about_the_producer() {
        let root = rig("dotfiles");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();

        // (가) 살아 있는 생산자(내 pid) → 표지 "돌고 있다" ∧ 고를 수 없음
        let live = root.join("dept-9-1760000000");
        mk(&live, "state/x.txt", b"x");
        std::fs::write(
            live.join(".producer"),
            format!(
                r#"{{"v":1,"state":"open","producer":"t","pid":{},"start_token":"","ts":1}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        // (나) 끝내지 못한 생산자(없는 pid) → 표지 "끝내지 못함" ∧ 고를 수 있음
        let dead = root.join("dept-10-1760000000");
        mk(&dead, "state/y.txt", b"y");
        std::fs::write(
            dead.join(".producer"),
            r#"{"v":1,"state":"open","producer":"t","pid":4294967294,"start_token":"","ts":1}"#,
        )
        .unwrap();

        let v = list_payload(&root, &state);
        let ents = v["entries"].as_array().unwrap();
        // (1) 점 파일은 첫 단계 목록에 **한 항목에서도** 섞이지 않고, 개수가 그 목록과 일치한다.
        //     ★종전엔 한 항목(`dept-9…`)의 리터럴만 봤다 — 전수 불변식으로 넓힌다(S68 ②).
        for e in ents {
            let kids = e["children_depth1"].as_array().unwrap();
            for k in kids {
                assert!(!k.as_str().unwrap().starts_with('.'), "점 파일이 목록에 들어왔다: {k}");
            }
            assert_eq!(
                e["children_total"].as_u64().unwrap() as usize,
                kids.len(),
                "개수와 목록이 어긋난다: {e:?}"
            );
        }
        let g = ents.iter().find(|e| e["name"] == "dept-9-1760000000").unwrap();
        assert_eq!(g["children_depth1"].as_array().unwrap()[0], "state");
        // (2) ★같은 줄이 스스로를 반박하지 않는다 — **막는 표지**(돌고 있다·판정 불가)이면서
        //     고를 수 있는 줄은 있을 수 없다. 리터럴 두 개가 아니라 **불변식**으로 적는다:
        //     윈도우에서는 생산자 생존이 `None`(판정 불가)이라 리터럴 쪽이 구조적으로 성립하지 않는다
        //     (D2 · S68 ②) — 그 리터럴은 아래 `#[cfg(unix)]` 절반이 지킨다.
        let mut blocking_rows = 0usize;
        for e in ents {
            let chip = e["producer_chip"].as_str().unwrap();
            let deletable = e["deletable"].as_bool().unwrap();
            let blocking = chip == producer_chip(&ProducerState::ActiveOpen)
                || chip == producer_chip(&ProducerState::Undeterminable);
            if blocking {
                blocking_rows += 1;
                assert!(!deletable, "표지가 {chip:?} 인데 줄을 고를 수 있다 = 자기모순");
            }
            // ★한 방향만 불변식이다: `deletable` 은 봉인 잔재(`interrupted`)에서도 거짓이므로
            //   "막지 않는 표지 ⇒ 고를 수 있다" 는 성립하지 않는다(`:1390` 의 두 항 논리곱).
        }
        // ★불변식이 **공허하지 않음**을 같은 자리에서 단언한다(이 묶음의 추가분 · 리그 probe 에는
        //   없었다): 막는 표지가 한 줄도 없으면 위 루프는 무엇도 거부할 수 없고, 표식 쓰기가
        //   조용히 깨진 날에도 초록이 난다. 거부할 수 없는 게이트는 게이트가 아니다.
        //   살아 있는 pid 는 unix 에서 `돌고 있다`·non-unix 에서 `판정 불가`이고 **둘 다 막는 표지**라
        //   이 단언은 전 플랫폼에서 성립한다.
        assert!(
            blocking_rows >= 1,
            "막는 표지가 한 줄도 없다 = 위 불변식이 공허하다(표식 픽스처가 깨졌는지 보라)"
        );

        // 판독은 항목마다 한 번이다 — 두 번 읽으면 두 값이 서로 다른 순간을 볼 수 있다.
        let src = include_str!("trash.rs");
        let body = &src[src.find("pub fn list_payload").unwrap()..src.find("fn ttl_days_default").unwrap()];
        assert_eq!(
            body.matches("producer_state(&p, &real_alive)").count(),
            1,
            "list_payload 이 표식 상태를 두 번 읽는다(A-1 회귀)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★S68 ② 의 분할된 절반 — **pid 생존 매핑**은 unix 에서만 성립한다. `real_alive` 가
    /// non-unix 에서 **모든 pid 에 `None`** 을 돌려 표지가 `판정 불가`가 되기 때문이다(D2).
    /// 이름이 그 사실을 말한다 — 위 크로스플랫폼 절반은 pid 매핑을 약속하지 않는다.
    /// ★D2 가 고쳐지면 이 절반은 `cfg` 없이 전 플랫폼으로 돌아온다(그것이 진짜 수리다).
    #[cfg(unix)]
    #[test]
    fn list_maps_a_live_and_a_dead_producer_pid_to_running_and_interrupted_unix() {
        let root = rig("dotfiles-unix");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let live = root.join("dept-9-1760000000");
        mk(&live, "state/x.txt", b"x");
        std::fs::write(
            live.join(".producer"),
            format!(
                r#"{{"v":1,"state":"open","producer":"t","pid":{},"start_token":"","ts":1}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        let dead = root.join("dept-10-1760000000");
        mk(&dead, "state/y.txt", b"y");
        std::fs::write(
            dead.join(".producer"),
            r#"{"v":1,"state":"open","producer":"t","pid":4294967294,"start_token":"","ts":1}"#,
        )
        .unwrap();
        let v = list_payload(&root, &state);
        let ents = v["entries"].as_array().unwrap();
        let g = ents.iter().find(|e| e["name"] == "dept-9-1760000000").unwrap();
        assert_eq!(g["producer_chip"], producer_chip(&ProducerState::ActiveOpen));
        assert_eq!(g["deletable"], false, "돌고 있다고 적으면서 고를 수 있게 두면 모순이다");
        let d = ents.iter().find(|e| e["name"] == "dept-10-1760000000").unwrap();
        assert_eq!(d["producer_chip"], producer_chip(&ProducerState::InterruptedOpen));
        assert_eq!(d["deletable"], true, "끝내지 못한 생산자는 타이핑 확인 뒤 지울 수 있다");
        assert_eq!(d["children_depth1"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── 봉인 잔재는 자동 재개되지 않는다 ───────────────────────────────────────────────────
    #[test]
    fn a_sealed_leftover_is_listed_and_never_resumed() {
        let root = rig("sealed");
        let name = "dept-8-1760000000";
        let sealed = root.join(format!("{TRASH_SEAL_PREFIX}{name}"));
        mk(&sealed, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let v = list_payload(&root, &state);
        let me = v["entries"].as_array().unwrap().iter()
            .find(|e| e["name"].as_str().unwrap().starts_with(TRASH_SEAL_PREFIX)).unwrap().clone();
        assert_eq!(me["interrupted_deleting"], true);
        assert_eq!(me["deletable"], false, "중단된 봉인은 선택 대상이 아니다");
        // 같은 이름으로 다시 명령해도 봉인이 있으면 거부다.
        let e2 = root.join(name);
        mk(&e2, "state/x.txt", b"x");
        let base = certify_entry(&e2, &real_dev).unwrap();
        let out = execute_one(&root, name, &base, &real_dev, now_epoch() + 100_000.0, &state, "od-5");
        assert_eq!(out.reason_code.as_deref(), Some(R_SEAL_FAILED));
        assert!(sealed.exists() && e2.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★봉인 뒤 대조의 하중 시험 — 봉인과 대조 사이에 파일이 하나 들어오면 **제거하지 않고**
    /// 봉인 상태로 멈춘다(되돌리지 않는다). 변이 M5(대조 제거)가 이 시험을 빨갛게 만든다.
    #[test]
    fn a_change_between_the_seal_and_the_compare_stops_in_the_sealed_state() {
        let root = rig("after-seal");
        let name = "dept-6-1760000000";
        let e = root.join(name);
        mk(&e, "state/x.txt", b"x");
        let state = root.join("sd");
        std::fs::create_dir_all(&state).unwrap();
        let base = certify_entry(&e, &real_dev).unwrap();
        fn inject(sealed: &Path) {
            std::fs::write(sealed.join("state/snuck-in.txt"), b"after the seal").unwrap();
        }
        *AFTER_SEAL_HOOK.lock().unwrap() = Some((root.clone(), inject));
        let out = execute_one(&root, name, &base, &real_dev, now_epoch() + 100_000.0, &state, "od-seal");
        *AFTER_SEAL_HOOK.lock().unwrap() = None;
        assert_eq!(out.outcome, "sealed_not_removed", "{out:?}");
        assert_eq!(out.reason_code.as_deref(), Some(R_CHANGED));
        let sealed = root.join(format!("{TRASH_SEAL_PREFIX}{name}"));
        assert!(sealed.exists(), "봉인 상태로 남아야 한다");
        assert!(sealed.join("state/x.txt").exists(), "한 바이트도 지워지지 않아야 한다");
        assert!(sealed.join("state/snuck-in.txt").exists(), "끼어든 파일도 그대로다");
        assert!(!e.exists(), "되돌리지 않는다(옛 이름으로 돌아가지 않는다)");
        let ls = ledger_lines(&state);
        assert!(ls.iter().any(|l| l["reason_code"] == R_CHANGED));
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── 핀: 에이전트가 닿을 CLI 경로가 없다 ────────────────────────────────────────────────
    #[test]
    fn no_cli_verb_reaches_the_trash_rpcs() {
        let cys = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cys.rs"),
        )
        .expect("cys.rs");
        // ★`trash.preview` 가 이 배열에서 빠진 이유: 그 **동사 자체가 퇴역**해 이제 어떤 층에도 없다
        //   (바늘을 남겨 두면 「CLI 에 없다」는 참이지만 **무엇에 대해 참인지**가 거짓이 된다).
        //   비동기 두 동사는 아래 줄이 함께 잠근다.
        for needle in [
            "trash.delete",
            "trash.preview_start",
            "trash.preview_poll",
            "trash.order",
            "trash.list",
        ] {
            assert!(!cys.contains(needle), "cys.rs 에 {needle} 가 있다 — 에이전트 경로가 열린다");
        }
    }
    #[test]
    fn the_windows_validation_feature_is_not_a_default_feature() {
        let toml = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .expect("Cargo.toml");
        assert!(toml.contains("trash_win_validated = []"), "feature 선언이 없다");
        let feats = toml.split("[features]").nth(1).expect("[features] 절");
        let default_line = feats
            .lines()
            .find(|l| l.trim_start().starts_with("default ="))
            .expect("default 줄");
        assert!(
            !default_line.contains("trash_win_validated"),
            "검증 feature 가 default 에 들어갔다 — 미검증 윈도우 삭제가 켜진다"
        );
    }
    /// ADDENDUM-1 의 grep 핀 — 이 모듈 안에 두 창의 리터럴이 상수 정의 밖에서 나타나지 않는다.
    #[test]
    fn the_two_windows_are_named_constants_with_no_other_literal_in_this_module() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd/trash.rs"),
        )
        .expect("trash.rs");
        for (lit, konst) in [("600", "TRASH_RECENT_WINDOW_SECS"), ("300", "TRASH_CONFIRM_TTL_SECS")] {
            for (i, line) in src.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                    continue;
                }
                if line.contains(konst) || line.contains("#[test]") {
                    continue;
                }
                // 시험 본문의 경계 계산은 상수에서 파생되므로 리터럴이 없어야 한다.
                assert!(
                    !line.contains(&format!("= {lit}")) || line.contains(&format!("{konst}: u64 = {lit}")),
                    "{}행에 {lit} 리터럴이 상수 정의 밖에서 있다: {line}",
                    i + 1
                );
            }
        }
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★MJR-2 결박 — 일꾼 생성 실패가 **빗장을 영구 점유하지 않는다**.
    //   무엇을 재는가: ⒜호출자가 즉시 거부를 받는다 ⒝빗장이 **스레드 본문 밖에서** 풀린다
    //   ⒞잡이 `Failed` + 거부 코드로 종결된다 ⒟**다음** 미리보기가 막히지 않는다.
    //   ⒟가 핵이다 — 열린 결함의 증상이 「데몬 수명 동안 영구 `preview_busy`」였다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    /// 미리보기 전역(빗장·잡 맵·주입 플래그)을 만지는 검체끼리 섞이지 않게 직렬화한다.
    static PREVIEW_TEST_SERIAL: Mutex<()> = Mutex::new(());
    fn single(r: Reply) -> Value {
        match r {
            Reply::Single(v) => v,
            _ => panic!("preview 동사는 Reply::Single 만 돌려준다"),
        }
    }

    #[test]
    fn a_worker_that_cannot_be_spawned_fails_the_job_and_releases_the_latch() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("spawn-fail");
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        reset_state_for_test();

        set_preview_spawn_fails(true);
        let params = json!({"names": ["dept-9-1760000000"]});
        let v = single(preview_start(&json!(1), &root, &state, &params));
        // ⒝ ★먼저 **빗장**을 본다(결함의 본체다 · 뮤턴트는 여기서 FAIL 해야 한다) —
        //    스레드 본문은 돌지 않았으므로 「본문 밖에서 풀렸다」는 것만이 증거다.
        assert!(
            PREVIEW_LOCK.lock().unwrap().is_none(),
            "일꾼이 뜨지 못했는데 빗장이 쥐여 있다 = 데몬 수명 동안 미리보기가 죽는다"
        );
        // ⒞ 잡은 영구 `Running` 이 아니라 `Failed` 묘비다(폴링이 이유를 읽는다).
        {
            let m = preview_jobs().lock().unwrap();
            assert_eq!(m.len(), 1, "잡 하나만 등록돼 있어야 한다");
            let j = m.values().next().unwrap();
            assert_eq!(j.phase, PreviewPhase::Failed, "생성 실패 잡이 Running 으로 남았다");
            assert_eq!(
                j.refusal.as_ref().expect("거부가 있어야 한다")["reason_code"],
                R_PREVIEW_NO_WORKER
            );
        }

        // ⒜ 호출자는 **지금** 거부를 받는다(job_id 를 받고 영원히 폴링하지 않는다).
        assert_eq!(v["ok"], false, "생성 실패인데 성공 응답이 나갔다");
        assert_eq!(v["error"]["code"], R_PREVIEW_NO_WORKER);

        // ⒟ ★다음 미리보기가 막히지 않는다 — 한 번의 생성 실패가 기능을 죽이지 않는다.
        set_preview_spawn_fails(false);
        let v2 = single(preview_start(&json!(2), &root, &state, &params));
        assert_eq!(
            v2["ok"], true,
            "생성 실패 뒤의 미리보기가 거부됐다({}) = 빗장이 회수되지 않았다",
            v2["error"]["code"]
        );
        let jid2 = v2["result"]["job_id"].as_str().expect("job_id").to_string();
        // 배경 일꾼이 끝나기를 기다린다(검체가 리그를 지우기 전에 — 최대 5s).
        let t = Instant::now();
        loop {
            let done = preview_jobs()
                .lock()
                .unwrap()
                .get(&jid2)
                .map(|j| j.phase != PreviewPhase::Running)
                .unwrap_or(true);
            if done || t.elapsed().as_secs() >= 5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        reset_state_for_test();
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★MJR-1 결박 — **예산을 넘긴 순회가 `Ready` 로 끝나는 길이 없다**(부서장이 못 박은 성공 기준).
    //   성공 기준이 「거부 코드가 구별된다」가 아닌 이유: 예산 초과 `Ready` 에는 거부가 **아예 없어서**
    //   코드 구별이 하중을 받지 못한다. 그래서 네 자리를 따로 잰다 —
    //   ⒜디렉터리 **안**(자식 루프) ⒝간격보다 작은 트리(루프 **뒤**) ⒞이름 묶음 전체의 결말
    //   ⒟결말 → 단계 매핑(`Ready` 의 입구가 하나뿐임).
    //   순회를 느리게 만드는 수단은 **기존 `dev_of` 주입점**이다(노드마다 불린다 — 60 s·120 s 상수를
    //   기다리지 않고 예산 초과를 만드는 유일한 길이고, 새 주입점을 만들지 않는다).
    // ───────────────────────────────────────────────────────────────────────────────────────

    /// 노드마다 `micros` 만큼 느려지는 `dev_of` — 장치 번호 자체는 실값을 돌려준다.
    fn slow_dev(micros: u64) -> impl Fn(&Path, &std::fs::Metadata) -> u64 {
        move |p: &Path, md: &std::fs::Metadata| {
            std::thread::sleep(std::time::Duration::from_micros(micros));
            real_dev(p, md)
        }
    }

    #[test]
    fn a_budget_that_runs_out_inside_one_directory_refuses_instead_of_certifying() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("mjr1-inside");
        let entry = root.join("dept-9-1760000000");
        std::fs::create_dir_all(&entry).unwrap();
        // 검사 간격(128)보다 **많은** 자식 — 자식 루프 안에서 마감이 지나간다.
        let kids = TRASH_CERTIFY_BUDGET_CHECK_NODES * 3;
        for i in 0..kids {
            std::fs::write(entry.join(format!("f{i:05}")), b"x").unwrap();
        }
        let dev_of = slow_dev(500);
        // 마감은 **디렉터리 머리 검사는 통과**하고(아직 지나지 않았다) 자식 루프 중간에 지나간다.
        let dl = Instant::now() + std::time::Duration::from_millis(50);
        let r = certify_entry_until(&entry, &dev_of, Some(dl))
            .expect_err("예산을 넘긴 순회는 인증이 아니다");
        assert_eq!(r.code, R_SESSION_TOO_SLOW, "세션 예산 소진 코드여야 한다");
        // ★경계가 **자식 경로**다 = 디렉터리 머리도, 루프 뒤도 아닌 **자식 루프 안**에서 끊겼다는 증거
        //   (머리 검사는 디렉터리 경로를, 루프 뒤 검사는 항목 뿌리를 경계로 적는다).
        let b = r.boundary.clone().expect("경계가 있어야 한다");
        assert_ne!(b, entry.display().to_string(), "경계가 항목 뿌리면 자식 루프에서 끊긴 것이 아니다");
        assert!(
            b.starts_with(&entry.display().to_string()),
            "경계가 이 항목 안이어야 한다: {b}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tree_smaller_than_the_check_cadence_still_cannot_certify_past_the_budget() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("mjr1-small");
        let entry = root.join("dept-9-1760000001");
        std::fs::create_dir_all(&entry).unwrap();
        // 자식이 **검사 간격보다 적다** ⇒ 자식 루프의 간격 검사는 **한 번도 불리지 않는다**.
        for i in 0..5 {
            std::fs::write(entry.join(format!("f{i}")), b"x").unwrap();
        }
        assert!(5 < TRASH_CERTIFY_BUDGET_CHECK_NODES, "이 검체의 전제");
        let dev_of = slow_dev(2_000); // 노드당 2 ms ⇒ 6 노드면 12 ms 이상
        let dl = Instant::now() + std::time::Duration::from_millis(4);
        let r = certify_entry_until(&entry, &dev_of, Some(dl))
            .expect_err("순회가 끝났더라도 예산을 넘겼으면 인증이 아니다");
        assert_eq!(r.code, R_SESSION_TOO_SLOW);
        // ★루프 **뒤** 검사가 잡았다 = 경계는 항목 뿌리다.
        assert_eq!(
            r.boundary.as_deref(),
            Some(entry.display().to_string().as_str()),
            "간격보다 작은 트리는 루프 뒤 검사가 잡아야 한다"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_set_whose_session_budget_runs_out_ends_the_job_instead_of_showing_a_preview() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("mjr1-names");
        let name = "dept-9-1760000002";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        for i in 0..3_000 {
            std::fs::write(entry.join(format!("f{i:05}")), b"x").unwrap();
        }
        // 먼저 **예산 없이** 재서 이 기계에서의 순회 시간을 얻는다(예측하지 않는다).
        let t = Instant::now();
        let full = certify_entry(&entry, &real_dev).expect("예산 없는 순회는 인증된다");
        let whole = t.elapsed();
        assert_eq!(full.nodes, 3_000);
        // 그 절반을 예산으로 준다 ⇒ 순회는 반드시 예산 안에 끝나지 못한다.
        let dl = Instant::now() + whole / 2;
        let (out, _pending) = certify_names(&root, &state, &[name.to_string()], Some(dl), None);
        // ★결말이 `Done` 이면 안 된다 — `Done` 은 `Ready` 의 **유일한 입구**이기 때문이다(⒟ 검체).
        //   ⚠선존 결함에서는 여기서 `Done` 이 나왔다(그 안의 항목은 `recent` 로 거부되지만 **단계는
        //   `Ready`** 다 — 예산을 넘긴 순회가 `Ready` 로 끝나는 바로 그 길이다).
        match &out {
            NamesOutcome::SessionExpired(r) => {
                assert_eq!(r.code, R_SESSION_TOO_SLOW);
            }
            NamesOutcome::Done { shown, refusals, .. } => panic!(
                "예산을 넘긴 순회가 Done 으로 끝났다(= Ready) — shown {} · refusals {}",
                shown.len(),
                refusals.len()
            ),
            NamesOutcome::InvalidName => panic!("이름이 거부됐다 — 이 검체의 전제가 깨졌다"),
            NamesOutcome::LedgerUnwritable => panic!("원장을 쓸 수 없다 — 이 검체의 전제가 깨졌다"),
        }
        // ⒟ 과 묶어서 읽는다: 이 결말의 단계는 `Failed` 다.
        let (phase, result, refusal) = preview_outcome_phase(out);
        assert_eq!(phase, PreviewPhase::Failed, "예산 초과가 Ready 로 매핑됐다");
        assert!(result.is_none());
        assert_eq!(refusal.expect("거부")["reason_code"], R_SESSION_TOO_SLOW);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_only_entrance_to_ready_is_a_completed_walk() {
        let done = NamesOutcome::Done {
            shown: vec![],
            refusals: vec![],
            certs: HashMap::new(),
        };
        assert_eq!(preview_outcome_phase(done).0, PreviewPhase::Ready);
        for out in [
            NamesOutcome::InvalidName,
            NamesOutcome::LedgerUnwritable,
            NamesOutcome::SessionExpired(Refusal {
                code: R_SESSION_TOO_SLOW,
                boundary: None,
                detail: "x".into(),
            }),
        ] {
            let (phase, result, refusal) = preview_outcome_phase(out);
            assert_eq!(phase, PreviewPhase::Failed, "Ready 의 입구는 Done 하나뿐이다");
            assert!(result.is_none(), "실패에 결과가 실리면 화면이 반쯤 참을 보여준다");
            let r = refusal.expect("실패는 이유를 들고 있어야 한다");
            assert!(
                r["reason_code"].is_string(),
                "화면이 읽는 열 이름은 reason_code 하나다(code 로 적으면 사유를 놓친다): {r}"
            );
        }
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★측정 전용 — 박자 파라미터의 **근거를 재는** 자리(B-162 의 「측정으로 조율할 파라미터」).
    //   초록/빨강 판정이 아니므로 `#[ignore]` 다: `cargo test --bin cysd -- --ignored --nocapture
    //   measure_the_pacing_parameters` 로 돌리고 **그 출력을 선언문에 인용한다**.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    #[ignore = "측정 전용 — 파라미터 근거를 재는 자리다(합격 판정이 아니다)"]
    fn measure_the_pacing_parameters() {
        let root = rig("pacing");
        let entry = root.join("flat");
        std::fs::create_dir_all(&entry).unwrap();
        const N: usize = 20_000;
        for i in 0..N {
            std::fs::write(entry.join(format!("f{i:06}")), b"x").unwrap();
        }
        // ⒜ 노드당 순회 비용(lstat + sha256 + 집계) — 예산 초과량의 단위다.
        let t0 = Instant::now();
        let c = certify_entry(&entry, &real_dev).expect("인증");
        let walk = t0.elapsed();
        let per_node_ns = walk.as_nanos() as f64 / c.nodes as f64;
        // ⒝ `Instant::now()` 1회 비용 — 매 노드 검사의 상한 비용이다.
        let t1 = Instant::now();
        let mut acc = 0u128;
        for _ in 0..N {
            acc = acc.wrapping_add(std::hint::black_box(Instant::now()).elapsed().as_nanos());
        }
        let now_ns = t1.elapsed().as_nanos() as f64 / N as f64;
        // ⒞ `sleep(1ms)` 의 **실제** 길이 — 이 기계의 양보 단가(요청값보다 길다).
        let mut slept: Vec<f64> = Vec::new();
        for _ in 0..20 {
            let t2 = Instant::now();
            std::thread::sleep(std::time::Duration::from_millis(1));
            slept.push(t2.elapsed().as_secs_f64() * 1e3);
        }
        let sleep_avg = slept.iter().sum::<f64>() / slept.len() as f64;
        let sleep_max = slept.iter().cloned().fold(0.0f64, f64::max);
        println!("PACING-MEASURE nodes={} walk_ms={:.3} per_node_ns={:.1} instant_now_ns={:.1} sleep1ms_avg_ms={:.3} sleep1ms_max_ms={:.3} blackbox_acc={}",
            c.nodes, walk.as_secs_f64() * 1e3, per_node_ns, now_ns, sleep_avg, sleep_max, acc != 0);
        // 파생값 — 선언문에 인용할 수치.
        for cadence in [256usize, 512, 1_000, 2_000, 5_000] {
            println!(
                "PACING-DERIVE cadence={cadence} overshoot_ms={:.3} check_overhead_pct={:.4} yield_cost_pct={:.2}",
                cadence as f64 * per_node_ns / 1e6,
                now_ns / (cadence as f64 * per_node_ns) * 100.0,
                sleep_avg * 1e6 / (cadence as f64 * per_node_ns) * 100.0,
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_background_walk_yields_to_the_dispatch_path() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("b162-yield");
        let entry = root.join("dept-9-1760000003");
        std::fs::create_dir_all(&entry).unwrap();
        // 노드당 0.5 ms × 300 = 약 150 ms 의 일 ⇒ 20 ms 간격이면 양보가 여러 번 일어난다.
        for i in 0..300 {
            std::fs::write(entry.join(format!("f{i:04}")), b"x").unwrap();
        }
        let dev_of = slow_dev(500);

        // ⒜ 배경 잡(마감 있음 · 넉넉하게) — 양보한다.
        reset_yields_for_test();
        let far = Instant::now() + std::time::Duration::from_secs(60);
        certify_entry_until(&entry, &dev_of, Some(far)).expect("넉넉한 예산이면 인증된다");
        let bg = yields_for_test();
        assert!(
            bg >= 2,
            "배경 순회가 디스패치에 양보하지 않았다(양보 {bg}회) — B-162 가 배선되지 않았다"
        );

        // ⒝ ★동기·제거 경로(마감 없음) — **양보하지 않는다**(그 경로에서 자면 stall 을 늘린다).
        reset_yields_for_test();
        certify_entry(&entry, &dev_of).expect("동기 경로도 인증된다");
        assert_eq!(
            yields_for_test(),
            0,
            "마감 없는 순회(동기·제거 경로)가 양보했다 — 디스패치를 더 오래 막는다"
        );
        reset_yields_for_test();
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★결박 검사 ⒜ — **느린 미리보기 동안 입력이 뒤에 줄 서지 않는다**(티켓 §1 5⒜ · MJR-6).
    //   무엇을 재는가: ⑴기동 RPC 자체의 왕복 ⑵★**순회 배리어 안에서** 다른 동사들의 왕복
    //   ⑶배리어가 실제로 열려 있었다는 증거(`running` 을 봤다).
    //   ★음성 대조는 **뮤턴트가 아니다**: 퇴역 중인 **동기 경로를 그대로 불러** 같은 리그에서
    //   같은 자를 들이댄다 — 동기 경로의 왕복은 순회 전체이므로 같은 단언이 FAIL 한다.
    //   (뮤턴트로 코드를 되돌리는 것보다 강하다: 비교 대상이 가설이 아니라 **살아 있는 코드**다.)
    //   ⚠못 보는 것: 이것은 **데몬 디스패치 경로**의 왕복이다. Tauri 의 공유 연결 셀
    //   (`conn_cell`)은 요청 1건의 수명만큼만 잠기므로(`rpc_full`), 데몬이 상수시간에 답하면
    //   GUI 입력이 그 뒤에 줄 서지 않는다 — 그 연결 셀 자체는 이 검체가 보지 못한다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn test_gui_input_latency_during_slow_preview() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("latency");
        let name = "dept-9-1760000004";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        for i in 0..30_000 {
            std::fs::write(entry.join(format!("f{i:05}")), b"x").unwrap();
        }
        // ★배리어가 성립하는지 **먼저 재서 확인한다** — 순회가 빠르면 이 검체는 아무 것도 재지 못한다.
        let t = Instant::now();
        let c = certify_entry(&entry, &real_dev).expect("기준 순회");
        let whole = t.elapsed();
        assert_eq!(c.nodes, 30_000);
        assert!(
            whole.as_millis() >= 100,
            "리그가 너무 빨라 배리어가 성립하지 않는다({whole:?}) — 파일 수를 늘려야 한다"
        );
        reset_state_for_test();
        // ★기준선 — 순회가 **돌지 않는** 동안의 `list` 왕복. 이 동사는 깊이 1 자식을 열거하므로
        //   그 자체로 비싸다(30,000 자식). 배리어 안의 값을 **이 기준선과 비교**해야 「순회가
        //   지연을 **더했는가**」를 재는 것이 된다 — 절대값만 재면 비싼 동사를 탓하게 된다.
        let tb = Instant::now();
        let _ = single(Reply::Single(ok_response(&json!(0), list_payload(&root, &state))));
        let list_base = tb.elapsed();

        // ⑴ 기동 — 순회를 기다리지 않는다.
        let t0 = Instant::now();
        let v = single(preview_start(&json!(1), &root, &state, &json!({"names": [name]})));
        let start_rtt = t0.elapsed();
        assert_eq!(v["ok"], true, "기동이 거부됐다: {v}");
        assert!(
            start_rtt.as_millis() < 50,
            "preview_start 왕복이 {start_rtt:?} — 순회를 기다린다(기동은 상수시간이어야 한다)"
        );
        let jid = v["result"]["job_id"].as_str().expect("job_id").to_string();

        // ⑵⑶ 배리어 **안에서** 왕복을 잰다 — 폴링과, 미리보기와 무관한 동사(list) 둘 다.
        let mut saw_running = false;
        let mut worst_poll = std::time::Duration::ZERO;
        let mut worst_list = std::time::Duration::ZERO;
        for _ in 0..8 {
            // ⒤ 화면이 대기 중에 실제로 부르는 동사 — 500ms 마다 이것 하나다.
            let t1 = Instant::now();
            let p = single(preview_poll(&json!(2), &json!({"job_id": jid})));
            worst_poll = worst_poll.max(t1.elapsed());
            if p["result"]["status"] == "running" {
                saw_running = true;
            }
            // ⒥ 미리보기와 **무관한** 동사 — 순회가 다른 요청을 막지 않는가.
            let t2 = Instant::now();
            let _ = single(Reply::Single(ok_response(&json!(3), list_payload(&root, &state))));
            worst_list = worst_list.max(t2.elapsed());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            saw_running,
            "순회가 이미 끝나 있었다 — 배리어 **안에서** 재지 못했다(이 검체는 그러면 무의미하다)"
        );
        assert!(
            worst_poll.as_millis() < 50,
            "배리어 안의 폴링 왕복 최악이 {worst_poll:?} — 입력이 순회 뒤에 줄 서고 있다"
        );
        // ★비싼 동사는 **기준선 대비**로 본다: 순회가 지연을 더했다면 몇 배로 벌어진다.
        assert!(
            worst_list < list_base * 3 + std::time::Duration::from_millis(10),
            "배리어 안의 list 왕복 {worst_list:?} 가 기준선 {list_base:?} 대비 3배를 넘었다 — 순회가 다른 요청을 막고 있다"
        );

        // 배경 일꾼이 끝나기를 기다린다(다음 단락의 동기 호출과 디스크를 다투지 않게).
        let w = Instant::now();
        while w.elapsed().as_secs() < 10 {
            let running = preview_jobs()
                .lock()
                .unwrap()
                .get(&jid)
                .map(|j| j.phase == PreviewPhase::Running)
                .unwrap_or(false);
            if !running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // ★음성 대조 — 디스패치 **스레드에서 직접** 걷는 비용. 종전에는 퇴역한 동사
        //   `trash.preview` 를 불러 쟀는데 그 동사가 없어졌으므로, 그것이 부르던 **같은 본문**
        //   (`certify_names(.., None, None)` = 세션 예산 없는 동기 순회)을 **여기서 직접** 부른다.
        //   ⇒ 여전히 뮤턴트가 아니다: 비교 대상이 가설이 아니라 **살아 있는 생산 코드**다.
        let t3 = Instant::now();
        let (_out, _pending) = certify_names(&root, &state, &[name.to_string()], None, None);
        let sync_rtt = t3.elapsed();
        assert!(
            sync_rtt.as_millis() >= 50,
            "동기 순회의 왕복이 {sync_rtt:?} — 이 기계에서는 음성 대조가 성립하지 않는다(리그를 키워라)"
        );
        println!(
            "LATENCY-MEASURE walk_ms={:.1} start_rtt_us={} worst_poll_us={} list_base_us={} worst_list_us={} sync_rtt_ms={:.1}",
            whole.as_secs_f64() * 1e3,
            start_rtt.as_micros(),
            worst_poll.as_micros(),
            list_base.as_micros(),
            worst_list.as_micros(),
            sync_rtt.as_secs_f64() * 1e3
        );
        reset_state_for_test();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★§1 마지막 조각 — **동기 미리보기 동사는 돌아올 수 없다**(부서장 판정 2026-10-13 23:0x).
    ///
    /// 왜 이 핀이 있는가: `trash.preview` 는 `certify_names(root, state, &names, None, None)` 을 불러
    /// **세션 예산도 빗장도 양보도 없이** 걷는 **두 번째 인증자**였다. 그것이 살아 있는 동안
    /// 「`Ready` 는 예산 안에서 인증됨」과 「단일 플라이트가 인증자를 1로 묶는다」는 R4 선언의 두
    /// 불변식이 **문면으로만** 참이었다(codex MIN-2 의 잔여). 퇴역 전 호출자는 **0**이었다:
    /// UI 0 · Tauri 0 · `cys` CLI 0 · 트리 전체에서 그 문자열은 **검사 배열 1건**뿐이었다.
    #[test]
    fn the_synchronous_preview_verb_is_retired_and_cannot_come_back() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd/trash.rs"),
        )
        .expect("trash.rs");
        let prod = &src[..src
            .find("#[cfg(test)]\nmod tests {")
            .expect("테스트 모듈 경계 소실")];
        // ⒜ 디스패치 팔이 없다.
        assert!(
            !prod.lines().any(|l| l.trim() == "\"preview\" => {"),
            "동기 미리보기 디스패치 팔이 다시 생겼다 — 빗장에 참여하지 않는 두 번째 인증자다"
        );
        // ⒝ 그 함수가 없다.
        assert!(
            !prod.contains("fn preview(id: &Value"),
            "동기 미리보기 함수가 되살아났다 — 예산 없는 순회 경로가 다시 열린다"
        );
        // ⒞ 그리고 **대신 서는 둘**은 그대로 있다(퇴역이 기능을 지운 것이 아님을 같은 자리에서 말한다).
        for keep in ["\"preview_start\" => {", "\"preview_poll\" => {"] {
            assert!(prod.contains(keep), "{keep} 가 없다 — 퇴역이 미리보기 자체를 지웠다");
        }
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟-2 결박 — **상한을 넘긴 크기 캐시는 무시된다(거부가 아니다)**.
    //   무엇을 재는가: ⒜상한 초과면 목록이 `pending` 으로 돌아간다(= 캐시를 읽지 않았다) ⒝그래도
    //   응답은 **정상**이다(거부·오류가 아니다 · 항목이 그대로 보인다) ⒞같은 키를 **상한 아래** 크기로
    //   두면 `measured` 가 된다 ⇒ ⒜의 원인이 **크기 상한**이고 키 불일치가 아니라는 증거
    //   ⒟읽기는 그 파일을 **바꾸지 않는다**(읽기 경로의 부작용 0) ⒠쓰기가 지나가면 파일이 작아진다
    //   (주석에 적은 자기 치유 — 목적이 아니라 부수효과임을 검체도 그렇게 적는다).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_size_cache_over_the_cap_is_ignored_and_the_list_stays_normal() {
        let root = rig("d2-cap");
        let name = "dept-9-1760000200";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("f0"), b"x").unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        // 이 항목에 **맞는** 캐시 키를 만든다(이름+장치+inode+mtime 비트).
        let md = std::fs::symlink_metadata(&entry).unwrap();
        let (dev, ino) = dev_ino(&md);
        let key = size_cache_key(name, dev, ino, mtime_bits(&md));
        let hit = json!({"bytes": 4242, "nodes": 7, "max_depth": 1,
                         "newest_mtime": mtime_secs(&md), "measured_at": now_epoch(), "walk_secs": 0.01});

        // ⒜ 상한을 **넘긴** 파일 — 같은 키가 들어 있어도 읽히지 않아야 한다.
        // ★항목 수는 **계산해서 한 번에** 만든다(상한을 넘길 때까지 매번 직렬화하면 O(n²) 이고
        //   검체가 느려진다 — 느린 검체는 다음 사람이 지운다). 한 항목은 약 160 B 이므로
        //   2,000항목 ≈ 330 KB 로 256 KiB 를 넘긴다(실측: 2,000항목 = 329,781 B).
        let mut big = serde_json::Map::new();
        big.insert(key.clone(), hit.clone());
        for i in 0..2_000usize {
            big.insert(
                format!("dept-{i}-1760000000|16777232|{i}|1760000000000"),
                json!({"bytes":12345,"nodes":2572,"max_depth":7,
                       "newest_mtime":1760000000.0,"measured_at":1760000000.0,"walk_secs":0.25}),
            );
        }
        std::fs::write(size_cache_path(&root), Value::Object(big).to_string()).unwrap();
        let over = std::fs::metadata(size_cache_path(&root)).unwrap().len();
        assert!(over > TRASH_SIZE_CACHE_MAX_BYTES, "검체 전제: 파일이 상한을 넘겨야 한다({over})");

        let v = list_payload(&root, &state);
        let ents = v["entries"].as_array().expect("entries 배열");
        let me = ents
            .iter()
            .find(|e| e["name"] == name)
            .expect("항목이 목록에 그대로 있어야 한다(거부가 아니다)");
        assert_eq!(
            me["size"]["state"], "pending",
            "상한을 넘긴 캐시가 읽혔다 — 읽는 비용이 파일 크기에 비례해 무계가 된다: {me}"
        );
        // ⒝ 응답은 정상이다 — 오류·거부 필드가 아니라 평소의 목록이다.
        assert!(v["root"].is_string() && v["ledger_writable"].is_boolean(), "목록 응답이 평소 모양이어야 한다: {v}");
        // ⒟ 읽기는 그 파일을 바꾸지 않는다.
        assert_eq!(
            std::fs::metadata(size_cache_path(&root)).unwrap().len(),
            over,
            "읽기 경로가 캐시 파일을 건드렸다"
        );

        // ⒞ 같은 키를 **상한 아래** 크기로 두면 `measured` 다 ⇒ 원인이 크기 상한이라는 증거.
        let mut small = serde_json::Map::new();
        small.insert(key.clone(), hit);
        std::fs::write(size_cache_path(&root), Value::Object(small).to_string()).unwrap();
        let under = std::fs::metadata(size_cache_path(&root)).unwrap().len();
        assert!(under <= TRASH_SIZE_CACHE_MAX_BYTES, "검체 전제: 이번엔 상한 아래여야 한다({under})");
        let v2 = list_payload(&root, &state);
        let me2 = v2["entries"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap();
        assert_eq!(
            me2["size"]["state"], "measured",
            "상한 아래인데 캐시가 안 읽혔다 — 키가 아니라 상한이 원인이라는 전제가 깨졌다: {me2}"
        );
        assert_eq!(me2["size"]["value"]["bytes"], 4242);

        // ⒠ 자기 치유(부수효과): 다시 상한을 넘긴 뒤 쓰기가 지나가면 파일이 작아진다.
        let mut big2 = serde_json::Map::new();
        for j in 0..2_000usize {
            big2.insert(
                format!("dept-{j}-1760000000|16777232|{j}|1760000000000"),
                json!({"bytes":12345,"nodes":2572,"max_depth":7,
                       "newest_mtime":1760000000.0,"measured_at":1760000000.0,"walk_secs":0.25}),
            );
        }
        std::fs::write(size_cache_path(&root), Value::Object(big2).to_string()).unwrap();
        let over2 = std::fs::metadata(size_cache_path(&root)).unwrap().len();
        assert!(over2 > TRASH_SIZE_CACHE_MAX_BYTES, "검체 전제: 다시 상한을 넘겨야 한다({over2})");
        let c = certify_entry(&entry, &real_dev).unwrap();
        size_cache_flush(&root, &[(key.clone(), c.clone())]);
        let after = std::fs::metadata(size_cache_path(&root)).unwrap().len();
        assert!(
            after <= TRASH_SIZE_CACHE_MAX_BYTES,
            "쓰기가 지나갔는데 파일이 여전히 상한 위다({after}) — 자기 치유 서술이 거짓이 된다"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟-4 — 「단일 플라이트가 인증자 **전체**를 1로 묶는다」는 **과장이었다**. 그 주장을 산문으로
    //   두지 않고 **세어서** 잠근다: 집행 경로(쓸기·봉인)의 `certify_entry(&…)` 호출자는 **정확히 셋**이고,
    //   그 셋은 빗장 밖이다. 넷째가 생기면 이 검사가 빨개져 **문면을 함께 고치게** 만든다.
    //   ⛔이 검사는 「빗장을 넓혀라」가 아니다 — 넓히면 쓸기가 오너의 미리보기를 기다린다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_enforcement_path_certifiers_outside_the_latch_are_exactly_three() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd/trash.rs"),
        )
        .expect("trash.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계 소실")];
        // `certify_entry(&` 는 집행 경로의 호출만 잡는다 — 미리보기는 `certify_entry_until(&` 라 안 걸린다.
        let callers = prod.matches("certify_entry(&").count();
        assert_eq!(
            callers, 3,
            "집행 경로의 인증 호출자가 {callers}건이다 — 셋이 아니면 `certify_entry` 머리 주석의 범위 문면과 \
             `src-tauri` 12.G 핀 주석을 **함께** 고쳐야 한다(단일 플라이트가 묶는 것은 미리보기 인증자다)"
        );
        // 그 범위 문면이 실제로 그 자리에 **있다**(산문이 사라지면 이 숫자가 뜻을 잃는다).
        assert!(
            prod.contains("단일 플라이트는 **미리보기 인증자**를 1로 묶는다"),
            "`certify_entry` 머리의 범위 문면이 사라졌다 — 숫자만 남은 핀은 뜻을 잃는다"
        );
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟′-1 결박 — **크기 캐시 쓰기가 「마지막 마감 검사 → `Ready` 발행」 구간에 없다**.
    //   ⛔**시간을 재지 않는다**(S20 §2: fsync 지연이 무계라 어떤 ms 단언도 거짓이 된다) — 재는 것은
    //   **구조** 둘이다:
    //     ⒜**행위**: `certify_names` 가 돌아온 **그 순간**(= 종단 판정을 손에 쥔 시점) 캐시 파일이
    //       아직 **없다**. 그리고 유예분을 `size_cache_flush` 에 넘기면 **그때** 생긴다
    //       ⇒ 입력 바이트에 비례하는 그 작업이 구간 **밖**이라는 직접 증거다.
    //     ⒝**순서**: 생산 코드에서 그 호출이 `preview_lock_release` **뒤**에 있다(발행→해제→반영).
    //       ★왜 소스를 읽는가: 그 순서는 배경 스레드 안의 일이라 밖에서 **결정론으로 관측할 수 없다**
    //       (경합 없이 끼어들 자리가 없다) ⇒ 관측 가능한 형태로 남은 것이 **원문 순서**다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_size_cache_is_not_written_until_the_terminal_decision_is_made() {
        let root = rig("dp1-defer");
        let name = "dept-9-1760000300";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("f0"), b"xy").unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        assert!(
            !size_cache_path(&root).exists(),
            "검체 전제: 시작 시 캐시 파일이 없어야 한다"
        );

        // 예산 없이(= 마감이 순회를 끊지 않는 조건에서) 인증한다 — 갓 만든 항목이라 `recent` 로
        // 거부되지만, **인증 자체는 성공**했으므로 유예분이 쌓인다(그 둘은 다른 축이다).
        let (out, pending) = certify_names(&root, &state, &[name.to_string()], None, None);
        match &out {
            NamesOutcome::Done { shown, refusals, .. } => {
                assert!(shown.is_empty() && refusals.len() == 1, "전제: 갓 만든 항목은 recent 거부다");
            }
            NamesOutcome::InvalidName => panic!("전제가 깨졌다 — 이름이 거부됐다"),
            NamesOutcome::LedgerUnwritable => panic!("전제가 깨졌다 — 원장을 쓸 수 없다"),
            NamesOutcome::SessionExpired(r) => {
                panic!("전제가 깨졌다 — 예산 없이 불렀는데 세션 만료다: {}", r.code)
            }
        }
        assert_eq!(pending.len(), 1, "인증에 성공했으면 유예분이 한 건이어야 한다");

        // ⒜ 종단 판정을 손에 쥔 이 시점에 캐시 파일은 **아직 없다**.
        assert!(
            !size_cache_path(&root).exists(),
            "인증 루프 안에서 캐시를 썼다 — 그 작업(파일 크기에 비례)이 「마감 검사 → Ready」 구간에 \
             들어간다(⒟′-1 이 그것을 빼라고 한 바로 그 작업이다)"
        );
        size_cache_flush(&root, &pending);
        let m = size_cache_read(&root);
        assert_eq!(m.len(), 1, "반영 뒤에는 한 건이어야 한다: {m:?}");
        assert!(m.contains_key(&pending[0].0), "유예분의 키가 반영되지 않았다: {m:?}");

        // ⒞ 유예분이 비면 **I/O 가 아예 없다**(거부만 난 미리보기가 파일을 건드리지 않는다).
        let root2 = rig("dp1-empty");
        size_cache_flush(&root2, &[]);
        assert!(
            !size_cache_path(&root2).exists(),
            "빈 유예분으로도 파일을 만들었다 — 쓸 것이 없을 때 쓰지 않는다는 서술이 거짓이 된다"
        );

        // ⒝ 원문 순서 — 발행·빗장 해제 **뒤에** 반영한다.
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd/trash.rs"),
        )
        .expect("trash.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계 소실")];
        let i_rel = prod
            .find("preview_lock_release(&jid);")
            .expect("잡 본문의 빗장 해제가 사라졌다");
        let i_flush = prod
            .find("size_cache_flush(&root_o, &pending);")
            .expect("잡 본문의 캐시 반영이 사라졌다 — 유예분을 버렸다면 측정이 영구히 pending 이 된다");
        assert!(
            i_rel < i_flush,
            "캐시 반영이 빗장 해제보다 **앞**에 있다 — 그러면 그 I/O 가 다시 구간 안이고 빗장 안이다"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟′-1 의 **반대쪽** 결박 — 거부의 **원장 기록은 유예하지 않았다**.
    //   왜 이 검체가 있는가: 위 검체는 「구간에서 일을 빼라」고 말하고, 그 압력은 다음 사람에게
    //   **원장 쓰기도 모아서 발행 뒤에 하라**는 「최적화」로 보인다. 그렇게 하면 ⒜발행과 반영 사이에
    //   죽을 때 **거부가 원장에 남지 않고** ⒝「원장을 쓸 수 없다」를 거부 시점에 알 수 없다
    //   ⇒ 둘 다 **정확성** 문제다(부서장 S20 §4 ⒟′-1 ⒥ 경고). 그 길을 이 검체가 막는다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_refusal_is_in_the_ledger_before_the_terminal_decision() {
        let root = rig("dp1-ledger");
        let name = "dept-9-1760000400";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("f0"), b"z").unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();

        let (out, _pending) = certify_names(&root, &state, &[name.to_string()], None, None);
        assert!(matches!(out, NamesOutcome::Done { .. }), "전제: Done");
        // ★`certify_names` 가 돌아온 **직후** — 아직 어떤 반영도 하지 않은 지금 — 원장에 있어야 한다.
        let rows = ledger_lines(&state);
        let hit = rows.iter().find(|r| r["entry"] == name && r["kind"] == "refusal");
        let hit = hit.unwrap_or_else(|| {
            panic!("거부가 원장에 없다 — 기록을 발행 뒤로 미루면 거부가 사라지는 길이 생긴다: {rows:?}")
        });
        assert_eq!(hit["reason_code"], R_RECENT, "사유 코드가 다르다: {hit}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★S20A §2 ⒥ 결박 — 크기 캐시 경로가 **FIFO** 면 캐시 부재와 동일로 접는다(열지 않는다).
    //   왜 FIFO 로 재는가: **디렉터리로는 변이를 가릴 수 없다**(종류 검사를 지워도 `read_to_string` 이
    //   실패해 같은 빈 맵이 되므로 검체가 **살아남는다**). FIFO 는 종류 검사를 지우면 `read_to_string`
    //   이 **쓰는 쪽을 기다리며 선다** ⇒ 그 차이가 이 검체의 하중이다.
    //   ★여기 쓰는 5초는 **주장이 아니라 하네스 장치**다(⛔응답성 상한을 단언하지 않는다 · S20A §2):
    //   멈춤은 「느린 실패」가 아니라 **무한 대기**라서, 다른 스레드에서 돌리고 받지 못하면
    //   **빨갛게 만들어야** 한다 — 그러지 않으면 변이가 「출력 없는 멈춤」으로 끝나 아무도 모른다.
    //   ⛔`cfg(unix)`: `mkfifo` 가 없는 플랫폼에서는 이 자극 자체를 만들 수 없다(윈도우는 미측정이다).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[cfg(unix)]
    #[test]
    fn a_size_cache_path_that_is_a_fifo_is_treated_as_absent_and_never_opened() {
        let root = rig("s20a-fifo");
        let name = "dept-9-1760000500";
        std::fs::create_dir_all(root.join(name)).unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let fifo = size_cache_path(&root);
        let st = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo 실행");
        assert!(st.success(), "검체 전제: mkfifo 가 성공해야 한다");
        let md = std::fs::metadata(&fifo).expect("FIFO stat");
        assert!(!md.file_type().is_file(), "검체 전제: 정규 파일이 아니어야 한다");
        assert_eq!(md.len(), 0, "검체 전제: FIFO 의 len 은 0 이라 **크기 상한으로는 안 걸린다**");

        let (root_t, state_t) = (root.clone(), state.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let v = list_payload(&root_t, &state_t);
            let _ = tx.send(v);
        });
        let v = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|e| {
                panic!(
                    "목록이 5초 안에 돌아오지 않았다({e}) — 캐시 경로를 **열었고** 쓰는 쪽을 기다리고 있다. \
                     `size_cache_read` 의 파일 종류 검사가 그 길을 막는 자리다(S20A §2 ⒥)."
                )
            });
        let me = v["entries"]
            .as_array()
            .expect("entries 배열")
            .iter()
            .find(|e| e["name"] == name)
            .expect("항목이 목록에 그대로 있어야 한다(거부가 아니다)");
        assert_eq!(me["size"]["state"], "pending", "캐시 부재와 동일이어야 한다: {me}");
        let _ = std::fs::remove_file(&fifo);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★T-1B-FIX 결박 ⒤ — **T 의 지배 항(프런티어 해제)의 단가가 선언한 봉투 안에 있다**.
    //   왜 이 검체가 있는가: 1-b 정본 불변식의 T 는 **해제 항이 지배**한다(F=5,000,000 에서 66.215959 ms ·
    //   항목당 **13.24 ns**). 그 단가가 조용히 나빠지면(프런티어 원소 꼴이 바뀌거나 해제가 비싼 컨테이너로
    //   갈아타면) **T 가 거짓이 되는데 아무 것도 붉어지지 않는다** ⇒ 이 검체가 그 자리를 지킨다.
    //   ⛔**이 검체는 불변식 자체가 아니다** — **봉투(envelope)** 다: 선언 단가의 약 15배인 **200 ns/항목**
    //   을 상한으로 쓴다(부하가 걸린 기계에서 거짓 적색이 나지 않게). ★시간을 재는 것이 이 불변식의
    //   **형식**이므로(등급이 아니라 측정값) 시간 단언이 여기서는 **정합**이다 — 다만 **봉투는 넉넉히** 둔다.
    //   측정 차림: **debug 빌드** · 이 기계 · 이 검체 자신이 한 번 잰다(⛔release 미측정).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_frontier_release_rate_stays_within_the_declared_envelope() {
        const N: usize = 500_000;
        const CEILING_NS_PER_ENTRY: u128 = 200;
        // 생산 코드의 프런티어와 **같은 꼴**이다(`certify_entry_until` 의 `stack`).
        // ★경로 값은 **개인 홈 경로를 쓰지 않는다** — 이 트리는 공개되고 `scripts/secret-scan.sh`
        //   (PUBLIC 발행 전 fail-closed 게이트 · 오너 2026-06-14)가 개인 경로를 차단한다.
        //   이 값은 **디스크에 닿지 않는 합성 입력**이다(clone 되어 Vec 에 쌓였다가 해제될 뿐
        //   열거·stat 를 하지 않는다). ⛔**바이트 길이를 바꾸지 마라** — 이 검체가 재는 것은
        //   해제 단가이고 그 단가는 값의 **할당 크기**에 딸려 있다(실측: 58 B 유지 시 13 ns ·
        //   `std::env::temp_dir()` 기반 83 B 로 바꾸면 중앙값 15 ns·최대 19 ns 로 올랐다).
        let seed = PathBuf::from("/home/user/.local/state/cys-trash/dept-9-1760000000/d00000");
        let mut front: Vec<(PathBuf, usize)> = Vec::new();
        for i in 0..N {
            front.push((seed.clone(), i % 7));
        }
        assert_eq!(front.len(), N, "검체 전제: 프런티어 크기");
        let t = Instant::now();
        drop(front);
        let took = t.elapsed();
        let per = took.as_nanos() / N as u128;
        assert!(
            per <= CEILING_NS_PER_ENTRY,
            "프런티어 해제가 항목당 {per} ns 다(봉투 {CEILING_NS_PER_ENTRY} ns · 전체 {took:?} / {N}개) — \
             1-b 정본 불변식의 **지배 항**이 선언보다 비싸졌다. T(≈67.21 ms @ F=5,000,000)를 **다시 재서** \
             불변식 블록의 숫자와 기여 항을 고쳐라(숫자를 그대로 두고 통과시키지 마라)."
        );
        println!("T1B-ENVELOPE frontier_release N={N} total={took:?} per_entry={per}ns");
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★T-1B-FIX 결박 ⒥ — **마감 초과가 그 순회의 작은 몫이다**(= 간격 안에 큰 항이 들어오면 붉어진다).
    //   ⛔ms 상수를 단언하지 않는다: **같은 실행에서** 전체 순회를 재고(대조) 마감을 그 **1/8** 로 준 뒤
    //   **초과분이 전체의 절반 미만**인지 본다. 프런티어가 쌓이는 픽스처(디렉터리 2,000개)라 **해제 항도
    //   초과분에 포함**된다. ⛔**그러나 「이 검체가 T 의 기여 항 ⓐ·ⓒ 를 지킨다」고는 적지 않는다**
    //   (S42 §3 ⒨ · 그 주장을 여기서 **철회한다** — 단서를 11줄 아래 두면 머리만 읽는 사람에게는
    //   무조건 보증이다): 아래 변이 **A·B 가 둘 다 살아남았으므로** 이 검체가 받치는 것은
    //   **이 픽스처 규모(F≈2,000 · 디렉터리당 폭 1)에서의 ⓐ·ⓒ 뿐**이고, **폭 항과 ⓑ(이동 항)는
    //   받치지 못한다**. 무엇이 무엇을 받치는지는 아래 네 항목과 끝의 「실제로 받치는 것」이 정본이다.
    //   ★**무엇이 이 검체를 죽이고 무엇이 죽이지 못하는가 — ⛔돌려서 잰 것만 적는다**:
    //   · **변이 A**(열거+정렬 되살림 = 폭 항 복귀) → ★**살아남았다**. 이 픽스처는 **디렉터리당 폭이 1**
    //     이고 최상위 2,000개 열거는 **마감 전에** 끝나므로 폭 항이 초과 창에 들어오지 않는다 ⇒ 폭 항을
    //     죽이는 것은 **S29 의 폭 검체**(`the_deadline_is_enforced_while_a_wide_directory_is_being_enumerated`)
    //     다 — 같은 변이에서 **FAILED rc 101**(답 25.522333ms vs 순수 열거 대조 6.678875ms).
    //   · **변이 B**(프런티어 `push` 마다 `shrink_to_fit` 으로 **재할당·이동 강제**) → ★**살아남았다**
    //     (초과분 **127.844 µs**). 이동 항은 **F 가 수백만일 때** 보이고(정본 블록 ⓑ = F=5,000,000 에서
    //     **130.792 µs**) 이 픽스처의 **F≈2,000** 에서는 µs 규모다 ⇒ ★**ⓑ 를 받치는 검체는 없다 —
    //     그 항은 측정으로만 선다**(숨기지 않고 적는다).
    //   · **변이 C**(봉투를 측정값 아래로 조임) → 옆 검체 `the_frontier_release_rate_stays_within_the_declared_envelope`
    //     가 **FAILED rc 101**(항목당 16 ns vs 봉투 5 ns) ⇒ 그 검체는 **공허하지 않다**.
    //   ⇒ **이 검체가 실제로 받치는 것**: ⓐ(남은 자식 ≤128)와 ⓒ(해제)를 **이 픽스처 규모에서** 함께 — 그리고
    //     ⓒ의 **단가**는 옆 봉투 검체가 500,000 항목에서 받친다.
    //   ★**자기신고**: 이 주석의 첫 판에 「열거+정렬을 되살리면 죽고 재할당을 강제하면 죽는다」를
    //     **돌리기 전에** 적었고 **둘 다 틀렸다**. 변이 결과를 미리 적지 않는다 — 이 레인 규율이고
    //     내가 두 번째로 밟았다(그래서 이 줄을 지우지 않고 남긴다).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_deadline_overshoot_is_a_small_fraction_of_the_walk_it_interrupted() {
        let root = rig("t1b-overshoot");
        let entry = root.join("dept-9-1760001100");
        std::fs::create_dir_all(&entry).unwrap();
        let d = 2_000usize;
        for i in 0..d {
            let sub = entry.join(format!("d{i:05}"));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("f"), b"").unwrap();
        }
        // 대조 — 예산 없이 한 번(이 기계의 전체 순회 비용).
        let t_full = Instant::now();
        let full = certify_entry(&entry, &real_dev).expect("예산 없는 순회는 인증된다");
        let whole = t_full.elapsed();
        assert_eq!(full.nodes, d * 2, "픽스처 전제: 노드 수");
        assert!(
            whole >= std::time::Duration::from_millis(4),
            "픽스처 전제: 전체 순회가 너무 빨라 마감을 나눌 수 없다({whole:?})"
        );

        let budget = whole / 8;
        let started = Instant::now();
        let r = certify_entry_until(&entry, &real_dev, Some(started + budget))
            .expect_err("예산을 넘긴 순회는 인증이 아니다");
        let elapsed = started.elapsed();
        assert_eq!(r.code, R_SESSION_TOO_SLOW, "세션 예산 소진 코드여야 한다: {r:?}");
        let overshoot = elapsed.saturating_sub(budget);
        assert!(
            overshoot < whole / 2,
            "마감 초과분이 {overshoot:?} 다(마감 {budget:?} · 전체 순회 대조 {whole:?}) — 마감 검사 사이에 \
             큰 항이 들어왔다. 1-b 정본 불변식의 기여 항 전수(남은 자식 ≤128 · 프런티어 push 최악 · 해제 · \
             거부 레코드+ReadDir 닫기)를 \
             **다시 재서** 그 블록을 고쳐라."
        );
        println!("T1B-OVERSHOOT whole={whole:?} budget={budget:?} elapsed={elapsed:?} overshoot={overshoot:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // CEO-approved D contract: trace actual iterator pulls at expiry. The retired ratio
    // remains a real-clock diagnostic below, never evidence of traversal position.
    /// Call-owned observation only; never compiled into a shipped binary.
    pub(super) struct CertifyProbe {
        base: Instant,
        expire_on_pull: bool,
        initially_expired: bool,
        pub(super) pulls: std::cell::Cell<usize>,
        pub(super) visits: std::cell::Cell<usize>,
        pub(super) child_refusals: std::cell::Cell<usize>,
        pub(super) head_checks: std::cell::Cell<usize>,
        pub(super) head_before_expiry: std::cell::Cell<bool>,
    }
    impl CertifyProbe {
        fn new(expire_on_pull: bool, initially_expired: bool) -> Self {
            Self { base: Instant::now(), expire_on_pull, initially_expired,
                pulls: 0.into(), visits: 0.into(), child_refusals: 0.into(),
                head_checks: 0.into(), head_before_expiry: false.into() }
        }
        pub(super) fn deadline(&self) -> Instant { self.base + std::time::Duration::from_nanos(1) }
        pub(super) fn now(&self) -> Instant {
            if self.initially_expired || (self.expire_on_pull && self.pulls.get() > 0) { self.deadline() } else { self.base }
        }
    }

    // CEO D-1/D-2: literal structural rule; independent of the production cadence constant.
    fn d_assert_checkpoint(pulls: usize, visits: usize, refusals: usize) {
        // NO elapsed-time claim on a stalling filesystem: this checks iterator/checkpoint structure only.
        assert_eq!(pulls, 128, "D structural rule: exactly 128 underlying iterator pulls");
        assert_eq!(visits, 128, "D structural rule: exactly 128 visits");
        assert_eq!(refusals, 1, "D structural rule: one child checkpoint refusal");
    }

    #[test]
    fn d_contract_negative_cheap_precollection() {
        let full = std::time::Duration::from_nanos(426_036_300);
        let took = std::time::Duration::from_millis(10);
        assert!(took < full / 8, "the old ratio must accept this named synthetic");
        eprintln!("D1_OLD_RATIO_ACCEPT full={full:?} took={took:?} pulls=16000 visits=128");
        let red = std::panic::catch_unwind(|| d_assert_checkpoint(16_000, 128, 1));
        assert!(red.is_err(), "D-1: new rule must reject the old ratio's cheap precollection");
    }

    #[test]
    fn d_contract_negative_127_pulls() {
        assert!(std::panic::catch_unwind(|| d_assert_checkpoint(127, 128, 1)).is_err(),
            "D-2: literal 128 must reject 127 pulls");
    }

    #[test]
    fn the_deadline_is_enforced_while_a_wide_directory_is_being_enumerated() {
        let root = rig("dp7-wide");
        let entry = root.join("dept-9-1760000800");
        std::fs::create_dir_all(&entry).unwrap();
        let w = 16_000usize;
        for i in 0..w {
            std::fs::write(entry.join(format!("f{i:06}")), b"").unwrap();
        }

        // ⒜ ★대조 — **같은 픽스처를 마감 없이 전부 인증**하는 비용(S68 ③ · Option A).
        //   종전 대조는 **순수 열거**(`read_dir().count()`)였다: 검사도 lstat 도 없는 **다른 일**이라
        //   양쪽이 서로 다른 모집단이었고, 플랫폼의 열거/인증 비용비가 달라지면 바가 **뒤집힌다**
        //   (윈도우 적색의 기제 · S68 판정 §6). 이제 양쪽이 **같은 노드당 일**이므로 비율은
        //   속도 차가 아니라 **멈춘 지점의 상한**을 준다 — 성립하는 것은 「폭 전체를 걷기
    //   전에 멈췄다」까지이고 ⛔「폭의 **어디까지** 걸었나」는 **가리키지 못한다**(codex N2).
        let t_ctl = Instant::now();
        let c = certify_entry_until(&entry, &real_dev, None).expect("대조는 인증에 성공해야 한다");
        let full = t_ctl.elapsed();
        assert_eq!(c.nodes, w, "픽스처 전제: 대조가 자식 {w} 개를 전부 방문해야 한다");
        let probe = CertifyProbe::new(true, false);
        let r = certify_entry_until_inner(&entry, &real_dev, Some(probe.deadline()), Some(&probe))
            .expect_err("예산을 넘긴 순회는 인증이 아니다");
        assert_eq!(r.code, R_SESSION_TOO_SLOW, "세션 예산 소진 코드여야 한다: {r:?}");
        assert_eq!(probe.head_checks.get(), 1);
        assert!(probe.head_before_expiry.get(), "root must be checked before the first yield expires the clock");
        assert_eq!(probe.now(), probe.deadline(), "subject must end at equality, not beyond it");
        // NO elapsed-time claim on a stalling filesystem: the locked 128 is an iterator/checkpoint rule.
        d_assert_checkpoint(probe.pulls.get(), probe.visits.get(), probe.child_refusals.get());

        // Non-gating real-clock diagnostics retain the retired full/took/ratio observation.
        let t = Instant::now();
        let diagnostic = certify_entry_until(&entry, &real_dev, Some(Instant::now() + full / 64));
        let took = t.elapsed();
        eprintln!("D_REAL_CLOCK full={full:?} took={took:?} ratio={} outcome={diagnostic:?} (non-gating)",
            took.as_secs_f64() / full.as_secs_f64());
        // ⒟ 경계가 자식 경로다(머리 검사는 디렉터리를, 루프 뒤 검사는 항목 뿌리를 적는다).
        let b = r.boundary.clone().expect("경계가 있어야 한다");
        assert_ne!(b, entry.display().to_string(), "경계가 항목 뿌리면 순회 중간에서 끊긴 것이 아니다");
        assert!(
            Path::new(&b).starts_with(entry.as_path()),
            "경계가 이 항목 안이어야 한다(★문자열 접두가 아니라 **경로**로 비교한다 — addendum-1 교훈): {b}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn d_expired_root_refuses_before_any_iterator_pull() {
        let root = rig("d-root");
        std::fs::write(root.join("one"), b"").unwrap();
        let p = CertifyProbe::new(false, true);
        let r = certify_entry_until_inner(&root, &real_dev, Some(p.deadline()), Some(&p)).expect_err("expired root");
        assert_eq!(r.code, R_SESSION_TOO_SLOW);
        assert_eq!(r.boundary.as_deref(), Some(root.to_str().unwrap()));
        assert_eq!(p.pulls.get(), 0, "root check must precede enumeration");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn d_expired_tail_refuses_without_a_certificate() {
        let root = rig("d-tail");
        for i in 0..7 { std::fs::write(root.join(format!("f{i}")), b"").unwrap(); }
        let p = CertifyProbe::new(true, false);
        let r = certify_entry_until_inner(&root, &real_dev, Some(p.deadline()), Some(&p)).expect_err("expired tail must not certify");
        assert_eq!(r.code, R_SESSION_TOO_SLOW);
        assert_eq!(r.boundary.as_deref(), Some(root.to_str().unwrap()));
        assert_eq!(p.pulls.get(), 7);
        assert_eq!(p.child_refusals.get(), 0, "width below the child checkpoint");
        let control = CertifyProbe::new(true, false);
        let c = certify_entry_until_inner(&root, &real_dev, None, Some(&control)).expect("no deadline control");
        assert_eq!((c.nodes, control.pulls.get(), control.visits.get()), (7, 7, 7));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn d_public_wrapper_forwards_deadline_and_real_clock() {
        let root = rig("d-wrapper");
        let r = certify_entry_until(&root, &real_dev, Some(Instant::now())).expect_err("public wrapper must forward expired deadline");
        assert_eq!(r.code, R_SESSION_TOO_SLOW);
        assert!(certify_entry_until(&root, &real_dev, None).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟′-7 결박 ⒥ — **조기 거부의 해제 비용이 답을 지배하지 않는다**(부서장 ⒞).
    //   무엇을 재는가: 거부가 결정된 **뒤에** 남는 일은 **프런티어(미처리 디렉터리) 해제**뿐이다
    //   (⒟′-7 로 폭 W 의 `kids` 가 사라졌으므로 그 축은 없어졌다). 디렉터리가 많은 픽스처에서
    //   조기 거부를 받고, 답이 **같은 실행에서 잰 열거 대조값의 절반 미만**에 오는지 본다.
    //   ⛔단위당 비용(ns)을 단언하지 않는다 — 그것은 측정값이고 산출물에 적었다(해제는 항목당 10~37 ns ·
    //   **debug 측정 · release 미측정**). 이 검체가 받치는 것은 **「지배하지 않는다」** 하나다.
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn an_early_refusal_is_not_dominated_by_releasing_the_frontier() {
        let root = rig("dp7-frontier");
        let entry = root.join("dept-9-1760000900");
        std::fs::create_dir_all(&entry).unwrap();
        // 프런티어에 쌓일 디렉터리를 많이 만든다(각 디렉터리에 자식 하나 — 열거도 돌게).
        let d = 2_000usize;
        for i in 0..d {
            let sub = entry.join(format!("d{i:05}"));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("f"), b"").unwrap();
        }
        let t_ctl = Instant::now();
        let counted = std::fs::read_dir(&entry).unwrap().filter_map(|e| e.ok()).count();
        let ctl = t_ctl.elapsed();
        assert_eq!(counted, d, "픽스처 전제: 최상위 디렉터리 수가 {d} 여야 한다");

        // 예산 없이 한 번 재서 **전체 순회 비용**을 얻고(대조), 그 1/8 을 마감으로 준다.
        let t_full = Instant::now();
        let full = certify_entry(&entry, &real_dev).expect("예산 없는 순회는 인증된다");
        let whole = t_full.elapsed();
        assert_eq!(full.nodes, d * 2, "픽스처 전제: 노드 수가 {}(디렉터리+파일) 여야 한다", d * 2);
        let t = Instant::now();
        let r = certify_entry_until(&entry, &real_dev, Some(Instant::now() + whole / 8))
            .expect_err("예산을 넘긴 순회는 인증이 아니다");
        let took = t.elapsed();
        assert_eq!(r.code, R_SESSION_TOO_SLOW, "세션 예산 소진 코드여야 한다: {r:?}");
        assert!(
            took < whole / 2,
            "조기 거부의 답이 {took:?} 걸렸다(전체 순회 {whole:?} · 최상위 열거 대조 {ctl:?}) — \
             거부 결정 뒤의 해제가 답을 지배한다면 묶음 분할이 필요하다(⒟′-7 ⒞)."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟′-6 결박 — 잔여의 **뒤 항**(이름 수 비례)이 **선언 유계**다.
    //   무엇을 재는가: ⒜상한 **초과**는 거부이고 ⒝그 거부가 **잡을 띄우지 않고 빗장을 쥐지 않는다**
    //   (= 거부가 다음 미리보기를 막지 않는다 — MJR-2 가 가르친 자리다) ⒞**상한 그 자체(경계값)는 통과**한다
    //   (경계가 한 칸 밀리면 상한의 뜻이 달라지므로 `>` 와 `>=` 를 가른다) ⒟거부 메시지가 **무엇을 하라고**
    //   말한다(나눠 고르기) · **아무 것도 지우지 않았다**고 말한다.
    //   ★경계값 검사는 `names_param` 을 **직접** 부른다 — `preview_start` 로 512개를 통과시키면 배경 일꾼이
    //   512항목을 걷고(없는 항목이라 거부 512건 · 각 거부가 fsync 라 실측 4 ms/건) **빗장을 쥔 채 수 초**를
    //   쓴다 ⇒ 다른 검체를 오염시킨다. **관문을 재는 검체는 관문을 부른다.**
    //   ⛔이 검체는 **단가(µs)를 단언하지 않는다** — 단가는 측정값(**debug · release 미측정**)이고
    //   **상한만이 구조**다. ★그리고 이 검체가 받치는 것은 **관문의 행위**이며, 「관문을 거치지 않는 호출자가
    //   없다」는 **written convention** 이다(상수 주석의 강등 문면 참조 · ⛔소스 전수 핀을 만들지 않았다).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn a_preview_of_more_names_than_the_cap_is_refused_without_starting_a_job() {
        let _g = PREVIEW_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = rig("dp6-cap");
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        reset_state_for_test();

        let mk_names = |k: usize| -> Vec<String> {
            (0..k).map(|i| format!("dept-9-17600{i:05}")).collect()
        };

        // ⒞ 경계값: 상한 **그 자체**는 관문을 통과한다(잡을 띄우지 않고 관문만 본다).
        let at_cap = json!({"names": mk_names(TRASH_PREVIEW_MAX_NAMES)});
        match names_param(&json!(1), &at_cap) {
            Ok(v) => assert_eq!(
                v.len(),
                TRASH_PREVIEW_MAX_NAMES,
                "상한 그 자체가 통과해야 한다 — 경계가 한 칸 밀리면 상한의 뜻이 달라진다"
            ),
            Err(_) => panic!("상한 그 자체가 거부됐다(`>` 가 `>=` 로 밀렸다)"),
        }
        // 초과는 관문에서 거부다.
        let over = json!({"names": mk_names(TRASH_PREVIEW_MAX_NAMES + 1)});
        let err = match names_param(&json!(2), &over) {
            Ok(_) => panic!("상한을 넘겼는데 관문이 통과시켰다 — 잔여의 뒤 항이 다시 무계가 된다"),
            Err(r) => single(r),
        };
        assert_eq!(err["error"]["code"], "invalid_params", "사유 어휘가 바뀌었다: {err}");
        let msg = err["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains(&TRASH_PREVIEW_MAX_NAMES.to_string()) && msg.contains("나눠서"),
            "거부가 **무엇을 하라고** 말하지 않는다: {msg}"
        );
        assert!(msg.contains("지우지 않았"), "거부가 **아무 것도 지우지 않았다**고 말하지 않는다: {msg}");

        // ⒝ 그 거부가 잡도 빗장도 남기지 않는다(동사 전체를 지나가며 본다).
        let v = single(preview_start(&json!(3), &root, &state, &over));
        assert_eq!(v["error"]["code"], "invalid_params", "동사가 다른 사유로 답했다: {v}");
        assert!(v.get("result").is_none(), "거부에 결과가 실렸다: {v}");
        assert!(
            PREVIEW_LOCK.lock().unwrap().is_none(),
            "상한 거부가 **빗장을 쥐었다** — 그러면 거부 한 번이 다음 미리보기를 막는다"
        );
        assert!(
            preview_jobs().lock().unwrap().is_empty(),
            "상한 거부가 **잡을 만들었다** — 폴링할 잡을 주고 아무 것도 하지 않는 길이다"
        );
        // ⒟ 원장도 건드리지 않는다(이 관문은 `safe_name` 선검사와 같은 자리다).
        assert!(
            !state.join(DELETE_LEDGER).exists(),
            "관문 거부가 원장에 썼다 — 같은 거부가 두 번 적히는 길이 열린다"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ───────────────────────────────────────────────────────────────────────────────────────
    // ★⒟′-5 결박 — 캐시 쓰기가 **원자적**이다(찢긴 파일 축이 닫혔다).
    //   ⒜**구조**: 생산 코드에 `std::fs::write(size_cache_path` 가 **0건**이고
    //     `cys::pack::write_atomic(&size_cache_path(` 가 **1건**이다. ★행위로는 가릴 수 없다 —
    //     원자성은 **경쟁 중에만** 보이고, 경쟁을 결정론으로 만들면 그것은 더 이상 경쟁이 아니다.
    //   ⒝**행위(곁다리로 얻는 것)**: 반영 뒤 뿌리에 **tmp 잔존이 0** 이고 파일은 **읽을 수 있다**
    //     (손으로 짠 tmp+rename 이 잔존을 남기는 길을 함께 막는다).
    //   ⛔이 검체는 **잃은 갱신을 막지 않는다**(그 축은 남아 있고, 영향은 「다시 재야 한다」뿐이다).
    // ───────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn the_size_cache_write_is_atomic_so_a_torn_file_is_not_a_reachable_state() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cysd/trash.rs"),
        )
        .expect("trash.rs");
        let prod = &src[..src.find("#[cfg(test)]\nmod tests {").expect("테스트 모듈 경계 소실")];
        assert_eq!(
            prod.matches("std::fs::write(size_cache_path").count(),
            0,
            "캐시를 **제자리 truncate** 로 쓴다 — 쓰는 중에 읽히면 찢긴 파일이 보인다(⒟′-5)"
        );
        assert_eq!(
            prod.matches("cys::pack::write_atomic(&size_cache_path(").count(),
            1,
            "원자적 쓰기 호출이 하나가 아니다 — 쓰는 자리가 둘이면 한쪽이 옛 방식으로 남는다"
        );

        let root = rig("dp5-atomic");
        let name = "dept-9-1760000600";
        let entry = root.join(name);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("f0"), b"q").unwrap();
        let md = std::fs::symlink_metadata(&entry).unwrap();
        let (dev, ino) = dev_ino(&md);
        let key = size_cache_key(name, dev, ino, mtime_bits(&md));
        let c = certify_entry(&entry, &real_dev).unwrap();
        size_cache_flush(&root, &[(key.clone(), c)]);
        let m = size_cache_read(&root);
        assert!(m.contains_key(&key), "원자적 쓰기 뒤 캐시를 읽을 수 없다: {m:?}");
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|n| n.contains("size-cache") && n != "size-cache.json")
            .collect();
        assert!(leftovers.is_empty(), "tmp 잔존이 있다: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
