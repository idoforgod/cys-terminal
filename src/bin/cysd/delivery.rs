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
//! 문자열은 발신자가 고를 수 있다. 그래서 판별 근거를 **master 가 고를 수 없는 층**으로 내린다:
//! **데몬만이 쓸 수 있는 배달 원장**. 훅은 프롬프트를 같은 규칙으로 해시해 원장과 대조한다.
//! 일치 = 기계가 방금 이 pane 에 밀어 넣은 바로 그 문자열 = 오너 임무 아님.
//!
//! ## 절대 불변식 3개 (튜닝 시 반드시 보존)
//! ① **원장 기록이 주입보다 반드시 선행한다.** 구현 규약: 기록을 `write_tx.try_send(..)`
//!    **직전**에 둔다. writer 스레드는 채널 수신 이후에만 PTY 에 쓰므로, 기록→try_send→(수신)→write
//!    순서가 구조적으로 보장된다(happens-before). 반대로 두면 훅이 "아직 없는 원장"을 읽어
//!    기계 push 를 오너 임무로 기록하는 race 가 열린다.
//! ② **사람 입력(human:true·GUI 키)은 절대 기록하지 않는다.** 오너 문장이 자기 해시와 매치돼
//!    기계로 접히면 온보딩이 전면 사망한다(임무를 영영 줄 수 없다). 기록 대상은 기계 유래 경로뿐.
//! ③ **거짓 양성(기계→오너 오인)이 치명 / 거짓 음성(오너→기계 오인)은 경미**. 원장이 넓을수록
//!    안전하다 — 애매하면 기록한다(단 ②는 예외 없음).
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

use serde_json::json;
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
        }
    }
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

/// 정규화 본문의 sha256 소문자 hex. 판정의 유일한 대조 키.
pub fn digest(text: &str) -> String {
    digest_normalized(&normalize(text))
}

/// 팩 계약 상태 루트 — `CYS_STATE_DIR` 우선, 없으면 `~/.cys/state`.
/// (`javis_bootstrap.state_dir()` 와 동일 규약 — 사본이 아니라 **미러**이며, 갈리면 원장이
/// 조용히 무력화되므로 양쪽에 테스트를 둔다.)
pub fn pack_state_dir() -> PathBuf {
    match std::env::var("CYS_STATE_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => cys::home_dir().join(".cys").join("state"),
    }
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

/// 크기 상한 초과 시 1세대 회전. 실패는 무시(회전 실패가 기록을 막으면 판별이 열린다 —
/// 원장 부재는 곧 게이트 개방 방향이므로, 회전보다 기록 지속이 우선이다).
fn rotate_if_needed(p: &Path) {
    if let Ok(m) = std::fs::metadata(p) {
        if m.len() > LEDGER_MAX_BYTES {
            let _ = std::fs::rename(p, p.with_extension("jsonl.1"));
        }
    }
}

/// ★주입 **직전** 호출 — 배달 사실을 원장에 append 한다. 반환: 기록 성공 여부(진단용).
///
/// 호출 규약(불변식 ①): 반드시 `write_tx.try_send(..)` **직전**에 부른다. try_send 가 실패해
/// 실제 주입이 없었더라도 원장에 남는 것은 무해하다 — 그 방향의 오류는 '오너 문장이 기계로
/// 오인될 수 있음'(경미)이고, 반대(주입은 됐는데 원장에 없음)는 게이트 개방(치명)이다.
pub fn record(
    socket_path: &Path,
    surface_id: u64,
    text: &str,
    origin: Origin,
    from_surface: Option<u64>,
) -> bool {
    let norm = normalize(text);
    if norm.is_empty() {
        return false; // 공백뿐 — 프롬프트가 될 수 없다(훅도 빈 프롬프트를 판정하지 않는다)
    }
    let p = ledger_path(socket_path);
    if let Some(d) = p.parent() {
        if std::fs::create_dir_all(d).is_err() {
            return false;
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
    let mut line = rec.to_string();
    line.push('\n');
    // append 모드 단일 write — 같은 파일에 여러 스레드가 붙어도 O_APPEND 로 라인이 섞이지 않는다
    // (PIPE_BUF 이하 · 레코드는 수백 바이트). flush 까지 마친 뒤에만 호출자가 try_send 로 넘어간다.
    match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(mut f) => f.write_all(line.as_bytes()).and_then(|_| f.flush()).is_ok(),
        Err(_) => false,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `CYS_STATE_DIR` 은 프로세스 전역 env — 이 배터리 전체를 직렬화한다(pack.rs PACK_ENV_LOCK 패턴).
    pub(crate) static STATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_state_dir<T>(f: impl FnOnce(&Path) -> T) -> T {
        let _g = STATE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = std::env::temp_dir().join(format!("cys-deliv-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&td);
        std::fs::create_dir_all(&td).unwrap();
        let prev = std::env::var("CYS_STATE_DIR").ok();
        std::env::set_var("CYS_STATE_DIR", &td);
        let out = f(&td);
        match prev {
            Some(v) => std::env::set_var("CYS_STATE_DIR", v),
            None => std::env::remove_var("CYS_STATE_DIR"),
        }
        let _ = std::fs::remove_dir_all(&td);
        out
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
            assert!(record(sock, 7, "[wakeup] 다음 액션 착수", Origin::Send, Some(3)));
            assert!(!record(sock, 7, "   \n ", Origin::Send, None), "공백뿐이면 미기록");
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
            assert!(record(sock, 1, "hello", Origin::Send, None));
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
            assert!(record(sock, 42, text, Origin::Send, None));
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
                assert!(record(sock, 9, t, Origin::Send, None), "기록 실패: {t}");
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
