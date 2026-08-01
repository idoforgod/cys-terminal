//! delivery — **배달 원장(delivery ledger)**: 데몬이 pane stdin 에 밀어 넣은 텍스트를
//! 주입 **직전에** 영속 기록하는 out-of-band 채널 (2026-08-01 R1 구조 수리).
//!
//! ## 왜 필요한가 (사고 기제)
//! 임무 게이트(`javis_mission.py`)는 "이 프롬프트를 오너가 쳤는가, 기계가 밀어 넣었는가"를
//! 갈라야 하는데, 훅은 둘을 **같은 자리(UserPromptSubmit)** 에서 본다. 종전 판별 근거는
//! 문자열 라벨 정규식 하나(`_MACHINE_LABEL`)뿐이었고, 그래서
//!   · 라벨 없는 push (`cys send --to master "다음 액션 착수"` — CEO_TEMPLATE.md:22 등 출하 규약)
//!   · 라벨 규약 우회 5종(중첩 대괄호·라벨 내 개행·80자 초과·선두 비공백·전각 대괄호)
//! 이 전부 **오너 임무**로 기록돼 자율주행 게이트를 열었다(= 사고 그 자체).
//!
//! 라벨 문자열은 **발신자가 고르는 값**이라, 규약을 안 지킨 push 하나로 게이트가 열렸다. 그래서
//! 판별 근거를 문자열에서 **주입한 쪽이 남긴 사실 기록**으로 옮긴다: 데몬이 주입 직전에 쓰는
//! 배달 원장. 훅은 프롬프트를 같은 규칙으로 해시해 원장과 대조하고, 일치하면 "기계가 방금 이
//! pane 에 밀어 넣은 바로 그 문자열" = 오너 임무 아님으로 접는다.
//! ★이것이 닫는 것은 **평시 정상 동작 경로**뿐이다 — 동일 UID 의 의도적 위조는 닫지 못한다.
//!   보장 범위의 정의처(SOT)는 `docs/THREAT-MODEL-mission-gate.md` 하나이며, 아래 '막을 수 없는
//!   것'은 그 문서의 배달층 각주다(모델 서술을 여기서 늘리지 말고 SOT 에 쓴다).
//!
//! ## 절대 불변식 3개 (튜닝 시 반드시 보존)
//! ① **원장 기록이 주입보다 반드시 선행한다.** 구현 규약: 기록을 `write_tx.try_send(..)`
//!    **직전**에 둔다. writer 스레드는 채널 수신 이후에만 PTY 에 쓰므로, 기록→try_send→(수신)→write
//!    순서가 구조적으로 보장된다(happens-before). 반대로 두면 훅이 "아직 없는 원장"을 읽어
//!    기계 push 를 오너 임무로 기록하는 race 가 열린다.
//! ② **데몬이 검증한 오너 GUI 입력만 기록에서 제외한다.** 오너 문장이 자기 해시와 매치돼
//!    기계로 접히면 온보딩이 전면 사망한다(임무를 영영 줄 수 없다). 단 제외 근거는 **클라이언트
//!    자기신고가 아니라 데몬이 아는 사실**이어야 한다 — 아래 R4 참조.
//! ③ **거짓 양성(기계→오너 오인)이 치명 / 거짓 음성(오너→기계 오인)은 경미**. 원장이 넓을수록
//!    안전하다 — 애매하면 기록한다(②의 제외는 **검증됐을 때만** 적용).
//!
//! ## ★R4 수리 — 기록 억제 근거를 '자기신고 human' → '데몬 검증 오퍼레이터'로 교체
//! 종전 `handlers.rs` 는 `if !human { record(..) }` 였다. 그런데 `human` 은 **클라이언트가
//! 스스로 붙이는 불리언**이고, 같은 함수의 ACL 주석이 이미 "어떤 pane 이든 위조 가능"이라
//! 못 박고 있었다 — 즉 **같은 코드가 ACL 목적으로는 human 을 불신하면서 원장 기록 여부만
//! 신뢰하는 비대칭**이 결함의 본체였다. 원시 소켓 한 줄
//! (`{"method":"surface.send_text","params":{...,"human":true}}`)로 원장 무기록 → 훅이 층2
//! 라벨 폴백 → 무라벨이라 통과 → 임무 게이트 개방이 실측됐다(라운드3 검증자 N3).
//!
//! 이제 억제 근거는 **데몬이 기동 시 자기가 발급하고 0600 으로 보관하는 `operator.token`** 이다
//! (`state.rs::write_operator_token` · GUI 승인 채널이 이미 쓰는 그 메커니즘). GUI(Tauri 백엔드)
//! 만이 그 파일을 읽어 첨부하고, 공용 `cys` CLI 는 **어떤 경로에서도 첨부하지 않는다**.
//!
//! ### 두 요구가 충돌하는 지점과 어느 쪽으로 접었는지 (명시)
//!  · 요구 A(불변식 ②): 진짜 오너 GUI 키 입력은 기록되면 **안 된다**.
//!  · 요구 B(fail-closed): 발신 주체를 데몬이 확정 못 하면 **기록해야** 한다(기계로 취급).
//!  두 요구는 "GUI 인지 아닌지 판정 불가"인 지점에서 충돌한다. **B 로 접었다** — 판정 불가는
//!  기록한다. 반대로 B 를 포기하면 원시 소켓 한 줄로 게이트가 열린다(실측된 치명 결함).
//!  그래서 GUI 는 토큰을 첨부하도록 배선하고(`src-tauri/src/main.rs::send_input`), 토큰이 없거나
//!  틀리면 **기록**한다.
//!
//! ## ★★R5 수리 — `operator_token` 은 '사람이 앉은 GUI 세션'이지 '사람이 친 문장'이 아니다
//! 라운드4 검증자 실측(신규 치명): GUI 는 **사용자가 자판으로 친 입력**만 보내는 게 아니라
//! **자기가 만든 문안**도 같은 `surface.send_text` 로 보낸다 — 전출 지시 전문(`ui/src/main.ts`
//! `clear_first:true` + 자동 CR)·노드 재기동 명령·경로 삽입이 그것이다. R4 배선은 그 호출에도
//! 토큰을 붙였으므로 `human_verified=true` 가 되어 **원장에 아무것도 남지 않았고**, 훅은 층2
//! 라벨 폴백으로 내려가 무라벨 문안을 **오너 임무로 기록**했다(실측 rc=0 · 흔적 0).
//!
//! 근본 교정: 토큰은 "이 요청이 오퍼레이터 GUI 세션에서 왔다"만 증명한다. 거기에 더해 UI 가
//! **그 문안을 사람이 쳤는지, UI 코드가 만들었는지**를 알려야 한다. 그래서 신호를 하나 더 흘린다
//! (`machine_origin` · UI → tauri → cysd). **표식이 있으면 토큰이 유효해도 기록**하며 원장
//! `origin` 은 `gui_auto` 로 남아 감사에서 구별된다.
//!
//! ★불변식 ② 절대 보존 — 두 경로를 코드에서 명확히 가른다:
//!   · `ui/src/main.ts::sendRaw`(= `term.onData`/붙여넣기 = **사람이 친 실키**) → 표식 **없음**
//!     → 종전대로 무기록. 여기가 기록되면 오너가 임무를 줄 수 없어 온보딩이 전면 사망한다.
//!   · UI 가 문자열을 조립해 보내는 호출(전출 지시·`launchCmd`·`restartNode`·`injectRawToPane`)
//!     → 표식 **있음** → 기록. 새 자동 주입을 UI 에 추가할 때 표식을 빠뜨리면 이 구멍이 재발하므로,
//!     "UI 가 만든 문자열을 보내면 표식"을 규칙으로 삼는다(사람 키만 예외).
//!
//! ★정직 고지: 이 표식도 **클라이언트가 붙이는 값**이다. 위조 방향은 "표식을 떼는 것"인데,
//!   표식을 떼도 **토큰까지 있어야** 기록이 억제되므로 공격자는 결국 토큰 파일을 읽어야 한다 —
//!   그 순간 §2 OUT OF SCOPE(동일 UID 의도적 위조)이고 `delivery.operator_token_from_pane`
//!   감사 이벤트의 대상이다. 즉 이 표식이 닫는 것은 **평시 정상 동작**(GUI 가 선의로 만든 자동
//!   주입)이 게이트를 여는 경로뿐이다.
//!
//! ## ★막을 수 없는 것 (정직 고지 — 닫는 시늉 금지 · 전문은 SOT)
//! 정의처는 `docs/THREAT-MODEL-mission-gate.md` §2 다. 여기 적는 것은 **배달층에 국한된 각주**이며,
//! 범위·근거·감사 절차를 늘려 쓰고 싶으면 SOT 를 고친다(두 곳에 쓰면 반드시 어긋난다).
//! 이 원장은 **동일 UID 구성요소의 의도적 위조를 막지 못한다.** master 는 Bash 를 가지므로
//! ① `operator.token` 파일을 그대로 읽어 첨부할 수 있고 ② 원장 파일을 직접 지우거나 덮어쓸 수
//! 있고 ③ 임무 대장을 직접 쓸 수 있다. 어떤 in-band 장치로도 원리적으로 닫히지 않는다
//! (동일 신뢰경계 안에서의 자기위조 = 커널 밖에서 풀 수 없는 문제).
//! 이 층이 실제로 보장하는 것은 **평시 정상 동작이 자율 착수 권한을 발급하지 않는다** 하나다.
//! 위조 범주에 대한 무기는 차단이 아니라 **감사 가능성**이다 — 기록 실패·판독 실패·창 밖 배달·
//! pane 에서 온 오퍼레이터 토큰을 전부 사유 코드로 남기고(`delivery.*` 이벤트 + 임무 대장
//! `anomalies`), master 가 그것을 오너에게 보고하게 한다. "완전 방어"를 주장하는 문장은
//! 그 자체가 결함이다.
//!
//! ## 경로 계약 (★두 디렉터리를 혼동하지 말 것)
//! Rust `state::state_dir(socket)` = 소켓 옆(`~/.local/state/cys`) 이고, **팩 계약**은
//! `CYS_STATE_DIR ‖ ~/.cys/state` 다(`javis_bootstrap.state_dir()`). 훅이 읽는 쪽은 후자이므로
//! 원장은 **팩 계약 경로**에 쓴다. 파일명은 항상 레인 접미(`delivery-<lane>.jsonl`) —
//! 부서 레인의 배달이 base master 의 판별에 섞이면 안 된다(`javis_bootstrap.lane_state_path`
//! 의 skip·lock 과 동일 규약).
//!
//! ## 판독자(python) 와의 계약
//! `javis_mission.py` 가 같은 정규화·같은 해시·같은 경로 규약을 구현한다. 양쪽 규칙이 갈리면
//! 원장은 조용히 무력화되므로, 정규화 규칙은 이 파일과 `javis_mission._normalize_delivery`
//! 주석에 **동일 문구로** 박제하고 양쪽에 회귀 테스트를 둔다.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 원장 레코드 스키마 버전. 판독자(javis_mission)가 미지 버전을 만나면 '판독 불가'로 접는다.
pub const LEDGER_SCHEMA: u64 = 1;

/// 원장 파일 크기 상한(바이트). 초과 시 1세대 회전(`.1` 로 rename 후 새 파일).
/// 2 MiB ≈ 레코드 ~1만 건 — 판별 창(수 시간)보다 훨씬 길다.
pub const LEDGER_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// 레코드에 남기는 정규화 본문 미리보기 상한(문자). 판정은 sha256 로 하며 이 값은 **진단용**이다.
/// 원장에 본문 전체를 남기면 그 자체가 프롬프트 유출 저장소가 된다 — 짧게 자른다.
pub const PREVIEW_CHARS: usize = 64;

/// 주입 유래 — 원장의 `origin` 필드. 판정에는 쓰이지 않고 사후 진단·감사에 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `cys send` 직접 배달(handlers::surface.send_text · Data/Inject 양 분기)
    Send,
    /// `--queued` 배달자(watchdog)가 조용해진 pane 에 밀어 넣는 순간
    Queue,
    /// schedule 잡 발화(schedule::inject)
    Schedule,
    /// 승인 Feed 의 CEO 자동 라우팅 주입
    Feed,
    /// 외부 채널(inbox) 봉투 주입
    Channel,
    /// 빈 좌석 승계 고지(`# [cys] …` 주석 주입)
    SeatTakeover,
    /// ★GUI 가 **프로그램적으로 만든** 주입(전출 지시·노드 재기동 명령·경로 삽입 등).
    /// 오퍼레이터 토큰이 붙어 있어도 **기록한다** — 사람이 앉은 세션이라는 사실과 사람이 친
    /// 문장이라는 사실은 다르다(아래 'R5 수리' 절).
    GuiAuto,
    /// ★기동 표식(sentinel) — 실제 배달이 아니다. 데몬이 기동 시 1줄 append 해
    /// "정상 원장은 절대 0바이트가 아니다"를 성립시킨다(판독자의 '빈 파일 = 손상' 근거).
    Boot,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Send => "send",
            Origin::Queue => "queue",
            Origin::Schedule => "schedule",
            Origin::Feed => "feed",
            Origin::Channel => "channel",
            Origin::SeatTakeover => "seat_takeover",
            Origin::Boot => "boot",
            Origin::GuiAuto => "gui_auto",
        }
    }
}

/// `record` 의 결과 — 종전 `bool` 은 "공백이라 안 씀"과 "쓰려다 실패"를 같은 false 로 뭉쳐서
/// **기록 실패가 조용히 사라졌다**(그 상태에서 주입된 텍스트는 훅에 오너 임무로 보인다 = 게이트
/// 개방). 감사 가능성 확보를 위해 셋을 가른다.
#[derive(Debug)]
pub enum Outcome {
    /// 원장에 append 됐다(판별 근거 확보).
    Recorded,
    /// 정규화 후 빈 문자열 — 프롬프트가 될 수 없어 기록 대상이 아니다(정상).
    Blank,
    /// ★기록 **실패**. 이 주입은 원장에 없으므로 훅이 오너 임무로 오인할 수 있다.
    Failed(String),
}

/// ★정규화 규칙 (판독자 `javis_mission._normalize_delivery` 와 **문자 단위로 동일**해야 한다)
///   ① 모든 유니코드 공백(White_Space)을 ASCII 공백 하나로 치환
///   ② 연속 공백을 1개로 접는다
///   ③ 앞뒤 공백 제거
/// 대소문자·유니코드 정규화(NFC)는 **하지 않는다** — 접는 폭이 넓을수록 오너 문장이 기계로
/// 오인될 확률만 커지고(경미하지만 불필요), TUI 는 코드포인트를 바꾸지 않는다.
///
/// 공백 집합을 `char::is_whitespace()`(= Unicode White_Space property)로 정의한 이유:
/// python `re.compile(r"\s+")` 는 여기에 더해 U+001C..U+001F 를 포함해 **미세하게 다르다**.
/// 그래서 판독자도 라이브러리 의미에 기대지 않고 White_Space 집합을 명시 구현한다(양쪽 박제).
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

/// 이미 정규화된 문자열의 sha256 소문자 hex.
fn digest_normalized(norm: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(norm.as_bytes()))
}

/// 정규화 본문의 sha256 소문자 hex. **테스트 전용 편의 함수**다.
///
/// ★R4 정정: 라운드3 보고서는 "cargo check 신규 경고 0" 이라고 썼지만 사실이 아니었다 —
/// 이 함수가 `dead_code` 경고를 내고 있었다(`warning: function \`digest\` is never used`).
/// 생산 경로(`record`)는 `normalize` 결과를 preview·chars 산출에 재사용해야 해서
/// `normalize` + `digest_normalized` 를 따로 부른다(이중 정규화 회피). 즉 이 함수는 배선
/// 누락이 아니라 **교차언어 앵커용 테스트 헬퍼**이므로 test 빌드로 좁힌다(삭제하지 않는 이유:
/// python `javis_mission.delivery_digest` 와의 동치를 박제하는 것이 이 함수의 유일한 임무다).
#[cfg(test)]
pub fn digest(text: &str) -> String {
    digest_normalized(&normalize(text))
}

/// 팩 계약 상태 루트 — `CYS_STATE_DIR` 우선, 없으면 `~/.cys/state`.
/// (`javis_bootstrap.state_dir()` 와 동일 규약 — 사본이 아니라 **미러**이며, 갈리면 원장이
/// 조용히 무력화되므로 양쪽에 테스트를 둔다.)
///
/// ★R5-B 테스트 위생: **테스트 빌드에서는 실 HOME 으로 해소되지 않는다**(`default_state_root`
/// 의 cfg 분기). 근거와 대안 비교는 아래 `tests` 모듈 머리말에 있다.
pub fn pack_state_dir() -> PathBuf {
    // 테스트 스레드 전용 격리 루트가 있으면 그것이 최우선(락 없이 테스트마다 분리된다).
    #[cfg(test)]
    if let Some(p) = tests::thread_state_root() {
        return p;
    }
    match std::env::var("CYS_STATE_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => default_state_root(),
    }
}

/// `CYS_STATE_DIR` 미설정 시의 기본 루트 — **생산 빌드**: 팩 계약 경로(`~/.cys/state`).
#[cfg(not(test))]
fn default_state_root() -> PathBuf {
    cys::home_dir().join(".cys").join("state")
}

/// `CYS_STATE_DIR` 미설정 시의 기본 루트 — **테스트 빌드**: 실 HOME 이 아니라 temp 샌드박스.
/// (개별 테스트가 격리를 잊어도 라이브 원장이 더러워지지 않게 하는 최종 방어선 · R5-B)
#[cfg(test)]
fn default_state_root() -> PathBuf {
    tests::unisolated_sandbox_root()
}

/// `javis_bootstrap._socket_is_base` 미러. 소켓 미설정('')=base.
fn socket_is_base(sock: &str) -> bool {
    let sock = sock.trim();
    if sock.is_empty() {
        return true;
    }
    let norm = sock.replace('\\', "/");
    if sock.starts_with("\\\\") || norm.to_ascii_lowercase().starts_with("//./pipe/") {
        // Windows named pipe — 성분 분해 부적합. 기존 basename 동작 보존.
        let last = norm.rsplit('/').next().unwrap_or("");
        return last == "cys" || last == "cys.sock";
    }
    for part in norm.split('/') {
        if part.starts_with("cys-dept-") {
            return false;
        }
    }
    let last = norm.rsplit('/').next().unwrap_or("");
    last == "cys" || last == "cys.sock"
}

/// SHA-1 hex — **비암호학적 용도 전용**(파일명 슬러그가 `javis_bootstrap._sanitize_sock_key`
/// (`hashlib.sha1(...).hexdigest()[:16]`)와 **정확히** 같아야 하기 때문에 존재한다).
/// 새 크레이트를 들이지 않는다(오프라인 빌드 계약 · sha1 은 Cargo.lock 에 전이로도 없다).
/// 서명·인증에 쓰지 말 것 — 그쪽은 `sha2`/`minisign-verify` 가 소유한다.
/// 정확성은 표준 시험벡터(`sha1("abc")`)로 박제한다.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// `javis_bootstrap._sanitize_sock_key` 미러(경로 구분자·':' → '_' · 과길면 앞 120자+sha1 16자).
fn sanitize_sock_key(sock: &str) -> String {
    let mut raw = sock.trim().to_string();
    if raw.is_empty() {
        raw = "base".to_string();
    }
    // python 은 os.sep, '/', '\\', ':' 를 치환한다. unix os.sep='/', windows os.sep='\\' 이므로
    // 셋의 합집합은 어느 OS 에서도 {'/','\\',':'} 로 동일하다.
    raw = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c })
        .collect();
    let raw = raw.trim_matches('_').to_string();
    let mut raw = if raw.is_empty() { "base".to_string() } else { raw };
    if raw.chars().count() > 160 {
        let head: String = raw.chars().take(120).collect();
        let h = sha1_hex(raw.as_bytes());
        raw = format!("{head}-{}", &h[..16]);
    }
    raw
}

/// 이 데몬이 속한 레인 키 — `javis_bootstrap.lane_key` 미러.
/// pane 은 `CYS_SOCKET`(= 이 데몬의 socket_path, state.rs 가 주입)을 보므로 양쪽 값이 일치한다.
pub fn lane_key(socket_path: &Path) -> String {
    let s = socket_path.to_string_lossy();
    if socket_is_base(&s) {
        "base".to_string()
    } else {
        sanitize_sock_key(&s)
    }
}

/// 배달 원장 경로 — **항상 레인 접미**(base 레인도 `delivery-base.jsonl`).
/// skip·lock 과 같은 규약이다: 역사적 무접미 경로가 없으므로 base 예외를 만들 이유가 없고,
/// 접미가 항상 있으면 "이 파일이 어느 레인 것인가"가 파일명만으로 결정론이다.
pub fn ledger_path(socket_path: &Path) -> PathBuf {
    pack_state_dir().join(format!("delivery-{}.jsonl", lane_key(socket_path)))
}

/// 데몬 인스턴스 표식 경로 — 임무의 **세션 결박**(過去 임무 무기한 유효 차단)에 쓴다.
pub fn epoch_path(socket_path: &Path) -> PathBuf {
    pack_state_dir().join(format!("delivery-{}.epoch.json", lane_key(socket_path)))
}

fn iso_utc(epoch: f64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(
        epoch as i64,
        ((epoch.fract().max(0.0)) * 1e9) as u32,
    )
    .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
    .unwrap_or_default()
}

/// 데몬 기동 시 1회 — 이 인스턴스의 표식을 쓴다(best-effort · 실패해도 기동을 막지 않는다).
///
/// 판독자 계약: `javis_mission.gate()` 는 임무 기록 시점의 `daemon_epoch` 를 대장에 박아 두고,
/// 판정 시 이 파일을 다시 읽어 **같은 데몬 인스턴스인지** 확인한다. 데몬이 재기동했으면
/// (= 사실상 새 세션) 과거 임무는 무효다. 파일이 없으면 결박은 degrade 되고 TTL 만 남는다.
pub fn write_epoch(socket_path: &Path) {
    let p = epoch_path(socket_path);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let epoch = crate::state::now_epoch();
    let rec = json!({
        "v": LEDGER_SCHEMA,
        "daemon_epoch": epoch,
        "started": iso_utc(epoch),
        "pid": std::process::id(),
        "socket": socket_path.to_string_lossy(),
    });
    // 원자 교체 — 판독자가 부분 쓰기를 만나 '판독 불가'로 접지 않게.
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, rec.to_string().as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// ★기동 시 1회 — 원장에 **기동 표식(sentinel)** 1줄을 append 한다(`write_epoch` 와 짝).
///
/// 왜 필요한가(R4 fail-open ② 봉합): 종전 판독자는 "파일이 존재하는데 0바이트"를 **정상**으로
/// 셌다(`bad=0, good=0` → LEDGER_OK). 그래서 원장을 `: > delivery-base.jsonl` 로 비우기만 하면
/// 대조할 해시가 사라져 모든 기계 push 가 층2 라벨 폴백으로 내려가고, 무라벨 push 는 그대로
/// 오너 임무가 됐다. 데몬이 기동 때마다 표식 1줄을 남기면 **정상 원장은 절대 0바이트가 아니다**
/// 가 성립하고, 판독자는 '빈 파일 = 손상'을 fail-closed 로 판정할 수 있다.
///
/// 표식 레코드는 **구 판독자에서도 정상 파싱돼야 한다**(팩 스큐: 신 데몬 + 구 javis_mission).
/// 그래서 `sha256`·`ts_epoch`·`surface` 필드를 전부 채우되, surface 는 어떤 pane 과도 매치되지
/// 않는 `"-"`(surface id 는 항상 정수 문자열)로 둔다 — 구 판독자는 '남의 pane 배달'로 건너뛰고
/// `good` 으로 세므로 손상 오판이 나지 않는다.
pub fn write_boot_sentinel(socket_path: &Path) -> Outcome {
    let p = ledger_path(socket_path);
    if let Some(d) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return Outcome::Failed(format!("상태 디렉터리 생성 실패: {e}"));
        }
    }
    rotate_if_needed(&p);
    let epoch = crate::state::now_epoch();
    let rec = json!({
        "v": LEDGER_SCHEMA,
        "surface": "-",           // 어떤 pane 과도 매치되지 않는 표기(surface id 는 정수 문자열)
        "ts_epoch": epoch,
        "ts": iso_utc(epoch),
        "sha256": "-",            // 어떤 프롬프트의 sha256 과도 같을 수 없다
        "origin": Origin::Boot.as_str(),
        "from": Value::Null,
        "chars": 0,
        "preview": "",
        "daemon_epoch": epoch,
        "pid": std::process::id(),
    });
    append_line(&p, &rec)
}

/// 원장 파일에 JSON 1줄 append(공통부). append 모드 단일 write — O_APPEND 라 여러 스레드가
/// 붙어도 라인이 섞이지 않는다(PIPE_BUF 이하 · 레코드는 수백 바이트).
fn append_line(p: &Path, rec: &Value) -> Outcome {
    let mut line = rec.to_string();
    line.push('\n');
    match std::fs::OpenOptions::new().create(true).append(true).open(p) {
        Ok(mut f) => match f.write_all(line.as_bytes()).and_then(|_| f.flush()) {
            Ok(()) => Outcome::Recorded,
            Err(e) => Outcome::Failed(format!("원장 write 실패: {e}")),
        },
        Err(e) => Outcome::Failed(format!("원장 open 실패({}): {e}", p.display())),
    }
}

/// 크기 상한 초과 시 1세대 회전. 실패는 무시(회전 실패가 기록을 막으면 판별이 열린다 —
/// 원장 부재는 곧 게이트 개방 방향이므로, 회전보다 기록 지속이 우선이다).
fn rotate_if_needed(p: &Path) {
    if let Ok(m) = std::fs::metadata(p) {
        if m.len() > LEDGER_MAX_BYTES {
            let _ = std::fs::rename(p, p.with_extension("jsonl.1"));
        }
    }
}

/// ★주입 **직전** 호출 — 배달 사실을 원장에 append 한다.
///
/// 호출 규약(불변식 ①): 반드시 `write_tx.try_send(..)` **직전**에 부른다. try_send 가 실패해
/// 실제 주입이 없었더라도 원장에 남는 것은 무해하다 — 그 방향의 오류는 '오너 문장이 기계로
/// 오인될 수 있음'(경미)이고, 반대(주입은 됐는데 원장에 없음)는 게이트 개방(치명)이다.
///
/// ★호출부는 되도록 `record_audited` 를 쓴다 — 실패를 이벤트로 남겨야 흔적이 생긴다.
pub fn record(
    socket_path: &Path,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> Outcome {
    let norm = normalize(text);
    if norm.is_empty() {
        return Outcome::Blank; // 공백뿐 — 프롬프트가 될 수 없다(훅도 빈 프롬프트를 판정하지 않는다)
    }
    let p = ledger_path(socket_path);
    if let Some(d) = p.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return Outcome::Failed(format!("상태 디렉터리 생성 실패: {e}"));
        }
    }
    rotate_if_needed(&p);
    let epoch = crate::state::now_epoch();
    let preview: String = norm.chars().take(PREVIEW_CHARS).collect();
    let sha = digest_normalized(&norm);
    let rec = json!({
        "v": LEDGER_SCHEMA,
        // ★surface 는 pane env `CYS_SURFACE_ID` 와 **같은 표기**(정수 문자열)다 —
        //   판독자가 재조립 없이 문자열 비교로 조인한다(state.rs: builder.env(ENV_SURFACE_ID, id)).
        "surface": surface_id.to_string(),
        "ts_epoch": epoch,
        "ts": iso_utc(epoch),
        "sha256": sha,
        "origin": origin.as_str(),
        "from": from_surface.map(|s| s.to_string()),
        "chars": norm.chars().count(),
        "preview": preview,
    });
    // flush 까지 마친 뒤에만 호출자가 try_send 로 넘어간다(불변식 ①).
    append_line(&p, &rec)
}

/// ★감사 포함 기록 — 실패를 **이벤트로 남긴다**(OUT OF SCOPE 대응: 막을 수 없는 것을 보이게).
///
/// 기록 실패는 조용히 지나가면 안 된다. 그 순간 주입된 텍스트는 원장에 없으므로 훅에게
/// **오너 임무로 보이고**, 게이트가 열린다. 차단할 수는 없다(주입을 막으면 배달 자체가 죽는다)
/// — 대신 `delivery.record_failed` 이벤트 + 데몬 로그로 흔적을 남겨 사후에 반드시 드러나게 한다.
/// 반환값은 종전 호출부 호환을 위한 bool(기록됨=true).
pub fn record_audited(
    daemon: &crate::state::Daemon,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> bool {
    match record(&daemon.socket_path, surface_id, text, origin, from_surface) {
        Outcome::Recorded => true,
        Outcome::Blank => false,
        Outcome::Failed(why) => {
            let path = ledger_path(&daemon.socket_path);
            eprintln!(
                "cysd: ★배달 원장 기록 실패 — surface={surface_id} origin={} path={} 사유={why} \
                 (이 주입은 원장에 없어 임무 게이트가 오너 입력으로 오인할 수 있다)",
                origin.as_str(),
                path.display()
            );
            daemon.bus.publish(
                "delivery.record_failed",
                "system",
                Some(surface_id),
                json!({
                    "origin": origin.as_str(),
                    "reason": why,
                    "path": path.to_string_lossy(),
                    "impact": "이 주입은 배달 원장에 없다 — 임무 게이트가 기계 push 를 오너 임무로 \
                               오인할 수 있다(자율 착수 권한 오발급 위험). 오너에게 보고 대상."
                }),
            );
            false
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ══════════════════════════════════════════════════════════════════════════
    // ★R5-B 테스트 상태 격리 — "라이브 원장을 테스트가 더럽히지 않는다"
    //
    // ## 결함(실측)
    // `pack_state_dir()` 는 `CYS_STATE_DIR` 이 없으면 실 HOME 으로 해소된다. 그래서 원장을
    // 건드리는 테스트가 격리를 **잊으면** `~/.cys/state/delivery-<임시소켓>.jsonl` 이 진짜로
    // 생긴다(1회 실행당 9개 · 누적 67개 실측). 원장 배선이 `record_audited` 로 넓어지며
    // 악화됐고, 앞으로 더 넓어질수록 "잊을 자리"도 함께 늘어난다.
    //
    // ## 그래서 방어를 개별 테스트가 아니라 **해소 함수 자체**에 둔다 (해소 우선순위)
    //   ① 이 테스트 스레드의 격리 루트(`isolate_state_dir*`)
    //   ② `CYS_STATE_DIR`(러너 전역 지정)
    //   ③ 그것도 없으면 — **실 HOME 이 아니라** 프로세스별 temp 샌드박스
    // ③ 이 본체다. 개별 테스트가 잊어도 라이브는 절대 안 더러워진다.
    //
    // ## 왜 env 가 아니라 thread-local 인가 (①)
    // `CYS_STATE_DIR` 은 **프로세스 전역**이라 병렬 러너에서 서로를 덮어쓴다 — 그래서 종전
    // `with_state_dir` 은 전역 뮤텍스로 배터리 전체를 직렬화해야 했다. 공용 하네스
    // (`channels::tests::tmp_daemon` 51곳 · `handlers::tests::daemon_with_acl` 15곳)에 그 방식을
    // 그대로 확대하면 ⓐ 66개 테스트가 직렬화되고 ⓑ 기존 `ACL_ENV_LOCK` 과의 **획득 순서**가
    // 갈려 교착이 열린다(실제로 기존 두 테스트는 STATE→ACL 순, 하네스 경유는 ACL→STATE 순이
    // 된다). thread-local 은 락이 없어 두 문제를 **원천적으로 만들지 않는다**.
    //
    // ## 정직 고지 — 이 층이 못 하는 것
    //  · 데몬이 **배경 스레드**에서 쓰는 원장은 thread-local 을 보지 못한다. 그 경로는 ②/③ 으로
    //    내려가며, 라이브 오염을 막는 것은 ③ 이다(격리 정밀도는 떨어지고 안전성은 유지된다).
    //  · 하드 실패(panic)는 **채택하지 않았다**. 배경 스레드에서의 panic 은 조용히 삼켜지거나
    //    (`join().ok()`) 뮤텍스를 poison 시켜 무관한 테스트를 무너뜨린다 — 즉 탐지기로서
    //    신뢰할 수 없고 새 flakiness 원인이 된다. 대신 **탐지를 결정론으로** 돌린다:
    //    격리를 잊은 쓰기는 전부 샌드박스 루트 **최상위**에 떨어지므로, 아래 한 줄이 회귀 게이트다.
    //      `ls "$TMPDIR"/cys-test-state-*/delivery-*.jsonl | wc -l`  → **0 이어야 한다**
    //    (배경 스레드 위반까지 잡는다 — panic 방식은 못 잡는 범주다.)
    // ══════════════════════════════════════════════════════════════════════════

    thread_local! {
        /// 이 테스트 스레드 전용 원장 루트(①). `None` 이면 ②→③ 으로 내려간다.
        static THREAD_STATE_ROOT: std::cell::RefCell<Option<PathBuf>> =
            const { std::cell::RefCell::new(None) };
    }

    /// `pack_state_dir()` 가 최우선으로 참조하는 훅(①).
    pub(crate) fn thread_state_root() -> Option<PathBuf> {
        THREAD_STATE_ROOT.with(|c| c.borrow().clone())
    }

    /// 이 테스트 프로세스의 샌드박스 루트. 격리 디렉터리(①)의 부모이자, 격리를 **잊은**
    /// 쓰기(③)가 떨어지는 자리다. 최상위에 `delivery-*.jsonl` 이 있으면 = 잊은 테스트가 있다.
    pub(crate) fn test_sandbox_root() -> PathBuf {
        let p = std::env::temp_dir().join(format!("cys-test-state-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// ③ 최종 방어선 — 실 HOME 대신 이 경로를 돌려준다. 프로세스당 1회 stderr 로 고지한다
    /// (`cargo test -- --nocapture` 에서 보인다).
    ///
    /// ★이 고지 자체는 **위반 신호가 아니다.** 아래 R5-B 자기검증 테스트 둘이 이 경로를
    /// 일부러 해소하므로 정상 실행에서도 반드시 1회 뜬다. 판정은 **경로 해소**가 아니라
    /// **실제 쓰기**로 한다 — 권위 있는 회귀 게이트는 주석 머리말의 파일 개수 한 줄이다.
    pub(crate) fn unisolated_sandbox_root() -> PathBuf {
        let p = test_sandbox_root();
        static WARN: std::sync::Once = std::sync::Once::new();
        WARN.call_once(|| {
            eprintln!(
                "cysd(test): CYS_STATE_DIR 미설정 — 배달 원장을 실 HOME(~/.cys/state) 대신 \
                 {} 로 강제 격리한다(R5-B). ★위반 판정은 이 줄이 아니라 다음 개수로 한다: \
                 이 디렉터리 **최상위**의 delivery-* 가 0 이 아니면 격리를 잊은 테스트가 있다 \
                 (delivery::tests::isolate_state_dir* 를 그 테스트/하네스에 적용할 것).",
                p.display()
            );
        });
        p
    }

    /// 격리 디렉터리 이름 충돌 방지용 일련번호(스레드 재사용·동일 tag 중복 호출 대비).
    static ISO_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn new_isolated_dir(tag: &str) -> PathBuf {
        let n = ISO_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let slug: String = tag
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let d = test_sandbox_root().join(format!("iso-{n:04}-{slug}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("격리 상태 디렉터리 생성");
        d
    }

    /// 이 테스트 스레드의 원장 루트를 새 임시 디렉터리로 고정한다(①). 가드가 drop 되면
    /// 이전 값으로 복원하고 디렉터리를 지운다 — **가드를 붙들 자리가 있는 테스트**용.
    pub(crate) fn isolate_state_dir(tag: &str) -> StateDirGuard {
        let dir = new_isolated_dir(tag);
        let prev = THREAD_STATE_ROOT.with(|c| c.borrow_mut().replace(dir.clone()));
        StateDirGuard { dir, prev }
    }

    /// 가드를 돌려줄 자리가 없는 **공용 하네스**(`tmp_daemon`·`daemon_with_acl`)용 —
    /// 이 테스트 스레드가 끝날 때까지 유효한 격리 루트를 설정한다(복원 없음 · 호출 시마다 새 dir).
    /// libtest 는 테스트마다 스레드를 새로 띄우므로 스레드 종료가 곧 해제이고, 스레드가 재사용
    /// 되더라도 다음 하네스 호출이 새 디렉터리로 **덮어쓴다**(오염된 값이 살아남지 않는다).
    pub(crate) fn isolate_state_dir_for_thread(tag: &str) -> PathBuf {
        let dir = new_isolated_dir(tag);
        THREAD_STATE_ROOT.with(|c| *c.borrow_mut() = Some(dir.clone()));
        dir
    }

    /// `isolate_state_dir` 의 RAII 가드.
    pub(crate) struct StateDirGuard {
        dir: PathBuf,
        prev: Option<PathBuf>,
    }

    impl StateDirGuard {
        pub(crate) fn path(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for StateDirGuard {
        fn drop(&mut self) {
            THREAD_STATE_ROOT.with(|c| *c.borrow_mut() = self.prev.clone());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// `record` 는 이제 3상(Recorded/Blank/Failed)을 돌려준다 — "기록됐는가"만 보는 축약.
    fn rec_ok(o: Outcome) -> bool {
        matches!(o, Outcome::Recorded)
    }

    fn with_state_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        let g = isolate_state_dir("delivery-unit");
        let dir = g.path().to_path_buf();
        f(&dir)
    }

    /// ★R5-B 회귀: **테스트 빌드는 실 HOME 으로 절대 해소되지 않는다.**
    /// 이것이 깨지면 격리를 잊은 테스트 하나가 곧바로 라이브 원장(`~/.cys/state/delivery-*.jsonl`)
    /// 을 만든다 — 실제로 67개가 그렇게 쌓였다(제품 결함이 아니라 테스트 위생 결함이지만,
    /// 오너의 라이브 상태를 오염시키므로 방어는 코드에 있어야 한다).
    #[test]
    fn test_build_never_resolves_state_root_to_live_home() {
        let live_root = cys::home_dir().join(".cys");
        let got = default_state_root();
        assert!(
            !got.starts_with(&live_root),
            "테스트 빌드의 기본 상태 루트가 라이브 HOME 안이다: {} (라이브: {})",
            got.display(),
            live_root.display()
        );
        assert_eq!(got, test_sandbox_root(), "기본 루트는 프로세스 temp 샌드박스여야 한다");
        assert!(got.starts_with(std::env::temp_dir()), "샌드박스는 temp 아래여야 한다");
    }

    /// ★R5-B 회귀: 스레드 로컬 격리가 실제로 원장 경로를 접고, drop 후 복원되는가.
    /// (env 를 건드리지 않으므로 병렬 러너에서 다른 테스트와 간섭하지 않는다 — 그것이 채택 이유다.)
    #[test]
    fn isolate_state_dir_scopes_paths_to_this_thread_and_restores() {
        let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
        let before = pack_state_dir();
        {
            let g = isolate_state_dir("scope-check");
            assert_eq!(pack_state_dir(), g.path(), "격리 중에는 가드 경로가 이긴다");
            assert!(
                ledger_path(sock).starts_with(g.path()),
                "원장 경로가 격리 디렉터리 밖이다: {}",
                ledger_path(sock).display()
            );
            assert!(
                epoch_path(sock).starts_with(g.path()),
                "epoch 표식도 같은 격리 안에 있어야 한다"
            );
            // 중첩 격리도 성립해야 한다(하네스가 이미 격리한 위에 테스트가 다시 격리하는 형태).
            let inner_dir = {
                let g2 = isolate_state_dir("scope-check-inner");
                assert_eq!(pack_state_dir(), g2.path());
                g2.path().to_path_buf()
            };
            assert_ne!(inner_dir, g.path());
            assert_eq!(pack_state_dir(), g.path(), "중첩 가드 drop 후 바깥 격리로 복원");
        }
        assert_eq!(pack_state_dir(), before, "가드 drop 후 원래 루트로 복원돼야 한다");
    }

    #[test]
    fn normalize_collapses_all_unicode_whitespace() {
        assert_eq!(normalize("  a \t\n b  "), "a b");
        assert_eq!(normalize("[wakeup]\n다음 액션 착수"), "[wakeup] 다음 액션 착수");
        // NBSP·ZWSP 아닌 유니코드 공백류도 접힌다(ideographic space U+3000)
        assert_eq!(normalize("a\u{3000}b"), "a b");
        assert_eq!(normalize("\u{00a0}x\u{00a0}"), "x");
        assert_eq!(normalize("   "), "");
        // 대소문자·유니코드 합성은 건드리지 않는다(과확장 금지)
        assert_eq!(normalize("Abc"), "Abc");
    }

    #[test]
    fn digest_is_whitespace_insensitive_but_content_sensitive() {
        assert_eq!(digest("다음 액션 착수"), digest("  다음   액션\t착수 \n"));
        assert_ne!(digest("다음 액션 착수"), digest("다음 액션 착수2"));
        // 알려진 벡터 — 판독자(python)와의 교차검증 앵커
        assert_eq!(
            digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha1_matches_standard_vectors() {
        // FIPS 180-1 부록 시험벡터 — python hashlib.sha1 과의 동치를 박제한다.
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 64바이트 경계(패딩이 블록을 하나 더 만드는 지점)
        assert_eq!(
            sha1_hex(&[b'a'; 64]),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
    }

    #[test]
    fn long_socket_path_key_uses_sha1_tail_like_pack() {
        // python: raw[:120] + "-" + sha1(raw)[:16]  (raw = 새니타이즈 결과 · 앞뒤 '_' strip 뒤)
        // 비-base 여야 이 분기에 온다 → cys-dept- 성분을 넣는다.
        let long = format!("/{}/cys-dept-x/cys.sock", "d".repeat(200));
        let k = lane_key(Path::new(&long));
        assert_eq!(k.chars().count(), 120 + 1 + 16);
        let raw = format!("{}_cys-dept-x_cys.sock", "d".repeat(200));
        assert!(k.ends_with(&sha1_hex(raw.as_bytes())[..16]), "sha1 꼬리 불일치: {k}");
        assert!(k.starts_with(&"d".repeat(120)));
        // ★교차언어 앵커: 아래 리터럴은 `javis_bootstrap.lane_key(동일 입력)` 실측값이다.
        //   양쪽이 갈리면 원장이 **조용히** 무력화되므로 값 자체를 박제한다.
        assert_eq!(k, format!("{}-24618710175be9a6", "d".repeat(120)));
    }

    #[test]
    fn lane_key_mirrors_pack_contract() {
        // ※ 기대값은 `javis_bootstrap.lane_key` 실행 결과와 대조해 박았다(교차검증 스크립트).
        assert_eq!(lane_key(Path::new("/Users/x/.local/state/cys/cys.sock")), "base");
        assert_eq!(
            lane_key(Path::new("/Users/x/.local/state/cys-dept-dept-1/cys.sock")),
            "Users_x_.local_state_cys-dept-dept-1_cys.sock" // python 은 앞뒤 '_' 를 strip 한다
        );
        // 커스텀 소켓(비-base·비-dept)도 레인 유일 키를 받는다
        assert_eq!(lane_key(Path::new("/tmp/whatever.sock")), "tmp_whatever.sock");
        // Windows named pipe — basename 판정 보존
        assert_eq!(lane_key(Path::new(r"\\.\pipe\cys")), "base");
        assert_eq!(
            lane_key(Path::new(r"\\.\pipe\cys-dept-a")),
            "._pipe_cys-dept-a"
        );
    }

    #[test]
    fn ledger_path_is_always_lane_suffixed_under_pack_state_dir() {
        with_state_dir(|td| {
            let p = ledger_path(Path::new("/Users/x/.local/state/cys/cys.sock"));
            assert_eq!(p, td.join("delivery-base.jsonl"), "팩 계약 경로+레인 접미");
            let d = ledger_path(Path::new("/Users/x/.local/state/cys-dept-a/cys.sock"));
            assert_ne!(d, p, "부서 레인은 base 원장과 분리된다");
        });
    }

    #[test]
    fn record_appends_matchable_line_and_skips_blank() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            assert!(rec_ok(record(sock, 7, "[wakeup] 다음 액션 착수", Origin::Send, Some(3))));
            assert!(!rec_ok(record(sock, 7, "   \n ", Origin::Send, None)), "공백뿐이면 미기록");
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            let lines: Vec<&str> = body.lines().collect();
            assert_eq!(lines.len(), 1);
            let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(v["surface"], "7");
            assert_eq!(v["origin"], "send");
            assert_eq!(v["from"], "3");
            assert_eq!(v["sha256"], digest("[wakeup]  다음 액션 착수"));
            assert!(v["ts_epoch"].as_f64().unwrap() > 0.0);
        });
    }

    #[test]
    fn record_rotates_at_cap() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let p = ledger_path(sock);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, vec![b'x'; (LEDGER_MAX_BYTES + 1) as usize]).unwrap();
            assert!(rec_ok(record(sock, 1, "hello", Origin::Send, None)));
            assert!(p.with_extension("jsonl.1").exists(), "1세대 회전");
            let body = std::fs::read_to_string(&p).unwrap();
            assert_eq!(body.lines().count(), 1, "회전 후 새 파일에 1건");
        });
    }

    /// ★불변식 ① 실증(race 봉쇄): writer 가 PTY 에 바이트를 쓰는 **그 순간** 원장에 이미
    /// 해당 레코드가 있어야 한다. 호출 규약(record → try_send)이 지켜지면 writer 스레드는
    /// 채널 수신 이후에만 쓰므로 구조적으로 보장된다 — 이 테스트가 그 순서를 박제한다.
    /// 규약을 뒤집으면(try_send 후 record) 아래 `seen_in_ledger` 가 false 로 떨어진다.
    #[test]
    fn ledger_record_strictly_precedes_pty_write() {
        use crate::state::WriteReq;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::sync_channel;
        use std::sync::{Arc, Mutex};

        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            let text = "다음 액션 착수"; // ★라벨 없는 사고 문안 그대로
            let want = digest(text);
            let seen_in_ledger = Arc::new(AtomicBool::new(false));
            let wrote = Arc::new(AtomicBool::new(false));

            struct Probe {
                path: PathBuf,
                want: String,
                seen: Arc<AtomicBool>,
                wrote: Arc<AtomicBool>,
            }
            impl std::io::Write for Probe {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    // PTY 쓰기 시점에 원장을 읽는다 — 여기서 이미 보여야 한다.
                    let body = std::fs::read_to_string(&self.path).unwrap_or_default();
                    if body.contains(&self.want) {
                        self.seen.store(true, Ordering::SeqCst);
                    }
                    self.wrote.store(true, Ordering::SeqCst);
                    Ok(buf.len())
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let (tx, rx) = sync_channel::<WriteReq>(1);
            let stop = Arc::new(AtomicBool::new(false));
            let probe = Probe {
                path: ledger_path(sock),
                want: want.clone(),
                seen: Arc::clone(&seen_in_ledger),
                wrote: Arc::clone(&wrote),
            };
            let h = std::thread::spawn(move || crate::state::run_writer_loop(probe, rx, stop));

            // ── 호출 규약 그대로: 기록 **먼저**, 그 다음 writer 채널 인계 ──
            assert!(rec_ok(record(sock, 42, text, Origin::Send, None)));
            tx.send(WriteReq::Data(text.as_bytes().to_vec())).unwrap();
            drop(tx);
            h.join().ok();

            assert!(wrote.load(Ordering::SeqCst), "writer 가 실제로 썼어야 한다");
            assert!(
                seen_in_ledger.load(Ordering::SeqCst),
                "주입 시점에 원장에 레코드가 없었다 — race 봉쇄 실패(훅이 기계 push 를 오너 임무로 기록한다)"
            );
            let _ = Mutex::new(()); // (import 사용 — 하네스 선례와 동일 형태 유지)
        });
    }

    /// 라벨이 **없는** 문안이 원장에 남는가 = 사고 경로(정규식으로는 못 잡던 그것)를 원장이 잡는가.
    #[test]
    fn unlabeled_machine_text_is_recorded() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            for t in [
                "다음 액션 착수",
                "이어서 진행해",
                "[[중첩]] 대괄호 우회",
                "［전각］ 라벨 우회",
            ] {
                assert!(rec_ok(record(sock, 9, t, Origin::Send, None)), "기록 실패: {t}");
            }
            let body = std::fs::read_to_string(ledger_path(sock)).unwrap();
            for t in ["다음 액션 착수", "이어서 진행해", "[[중첩]] 대괄호 우회", "［전각］ 라벨 우회"] {
                assert!(body.contains(&digest(t)), "원장에 없다: {t}");
            }
        });
    }

    #[test]
    fn epoch_marker_is_atomic_and_parseable() {
        with_state_dir(|_td| {
            let sock = Path::new("/Users/x/.local/state/cys/cys.sock");
            write_epoch(sock);
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(epoch_path(sock)).unwrap()).unwrap();
            assert!(v["daemon_epoch"].as_f64().unwrap() > 0.0);
            assert_eq!(v["v"], LEDGER_SCHEMA);
            assert!(!epoch_path(sock).with_extension("json.tmp").exists(), "tmp 잔재 없음");
        });
    }
}
