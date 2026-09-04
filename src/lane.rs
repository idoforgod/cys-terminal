//! 레인(lane) 판정·경로 규약 — **단일 소유자**.
//!
//! ## 왜 lib 에 있는가(사본 금지의 이유)
//! 같은 규약을 세 곳이 쓴다: 데몬(`cysd`)이 배달 원장·기동 표식을 쓰고, 훅 CLI
//! (`cys hook user-prompt-submit`)가 **임무 대장**을 쓰며(명세 §2-2 e), python
//! (`javis_lane.py`)이 그 셋을 읽는다. 종전에는 판정이 `cysd/delivery.rs` 안에만 있어서
//! CLI 가 같은 규약을 쓰려면 **복사**해야 했다 — 복사본은 언젠가 갈리고, 갈린 순간
//! 대장과 원장이 서로 다른 레인을 가리켜 층1 판정이 **조용히** 무력화된다
//! (2026-09-04 P0 레인 격리 누출의 반대 방향 사고).
//!
//! ## 이 모듈이 지는 계약
//! · 판정은 **경로 모양만** 본다(기계의 실제 홈 경로와 비교하지 않는다) — python 과 공유하는
//!   코퍼스 `cysjavis-pack/bin/tests/fixtures/lane-key-corpus.json` 이 기계마다 같은 답을
//!   내야 하기 때문이다. 알려진 한계(`<임의>/cys/cys.sock` 미러 경로)는 그 코퍼스에
//!   `known_limitation` 으로 고정돼 있다.
//! · **상태 루트는 이 모듈이 정하지 않는다.** 파일명 규약(`*_in(dir, sock)`)과 루트 해소를
//!   가른 것은 의도다 — 데몬은 테스트 빌드에서 실 HOME 대신 샌드박스를 쓰고(delivery.rs
//!   `pack_state_dir`), 훅 CLI 는 python 과 같은 `CYS_STATE_DIR ‖ ~/.cys/state` 를 쓴다.
//!   **이름 규칙은 하나**이고 루트만 호출자가 고른다.

use std::path::{Path, PathBuf};

/// `javis_bootstrap._socket_is_base` 미러. 소켓 미설정('')=base.
pub fn socket_is_base(sock: &str) -> bool {
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
    // ★P0 수리(2026-09-04 실사고): 종전엔 **basename 만** 봤다 — 디렉터리를 무시했으므로
    //   `~/.cys/state-harness/cys.sock` · `/var/folders/…/l1-new-*/cys.sock` 처럼 **관례적
    //   파일명을 유지한 격리 소켓이 전부 base 로 접혔고**, 그 데몬들이 본 레인
    //   `delivery-base.jsonl`·`.epoch.json` 에 썼다(실측: 외부 좌석 레코드 70건 · epoch 덮어씀).
    //   귀결은 본부 임무 게이트 오탐 폐쇄 + 전 노드 층1 원장 오염이다.
    //
    //   역설이 이 결함의 핵심이었다: `/tmp/whatever.sock` 처럼 **파일명이 다르면** 올바로
    //   격리됐다. 즉 **가장 흔한 격리 방식**(관례 파일명 유지 + 디렉터리 분리)만 조용히 실패했다.
    //
    //   수리: 기본 소켓은 **`…/state/cys/cys.sock`** 이므로 **부모 디렉터리 이름까지** 본다.
    //   `cys` 디렉터리 안의 `cys.sock` 만 base 이고 나머지는 전부 자기 레인이다(fail-closed).
    //
    //   ★기계 독립 판정을 유지한다: 이 기계의 실제 홈 경로와 비교하지 않고 **경로 모양**만 본다.
    //   그래야 python 과 공유하는 소켓→lane_key 매트릭스 fixture 가 기계마다 같은 답을 낸다.
    //   하위호환: 진짜 기본 소켓은 여전히 base(기존 `delivery-base.jsonl` 유효) ·
    //   `cys-dept-`·`/tmp/whatever.sock` 은 종전대로 비-base. **바뀌는 것은 누출 경로 하나뿐이다.**
    //
    //   ★**알려진 한계**(숨기지 않는다 · master 조건 ① 2026-09-04): 부모 **이름**만 보므로
    //   `<임의>/cys/cys.sock`(기본 트리를 다른 곳에 그대로 미러한 경로 · 예 `/tmp/cys/cys.sock`)
    //   은 **여전히 base 로 접힌다**. 절대경로를 플랫폼 기본 경로와 통째로 비교하면 막을 수
    //   있으나 그러면 판정이 **기계(홈 경로)에 의존**해 공유 fixture 가 기계마다 다른 답을 내고
    //   2언어 파리티가 깨진다 — 기계 독립을 택하고 한계를 명시한다.
    //   그런 형태로 격리할 때는 **`CYS_STATE_DIR` 를 함께 지정**해야 안전하다(상태 파일이 그
    //   디렉터리 안에만 생긴다 — W-A 실기동 실측). 이 한계는 fixture 에 `known_limitation` 으로
    //   고정돼 있고, 가시화(경고 이벤트)는 master 조건 ② 의 별건이다.
    let mut it = norm.rsplit('/');
    let last = it.next().unwrap_or("");
    if last != "cys" && last != "cys.sock" {
        return false;
    }
    // 부모 디렉터리가 정확히 `cys` 여야 한다(기본 소켓 `…/state/cys/cys.sock` 의 모양).
    it.next() == Some("cys")
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
pub fn sanitize_sock_key(sock: &str) -> String {
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

/// 이 소켓이 속한 **레인 키** — `javis_bootstrap.lane_key`(python `javis_lane.lane_key`) 미러.
/// pane 은 `CYS_SOCKET`(= 그 데몬의 socket_path · state.rs 가 주입)을 보므로 데몬·훅·python 셋이
/// 같은 값을 낸다. 갈리면 대장·원장·표식이 **서로 다른 레인**을 가리킨다(2026-09-04 P0 사고).
pub fn lane_key(socket_path: &Path) -> String {
    let s = socket_path.to_string_lossy();
    if socket_is_base(&s) {
        "base".to_string()
    } else {
        sanitize_sock_key(&s)
    }
}


/// 팩 계약 상태 루트 — `CYS_STATE_DIR` 우선, 없으면 `~/.cys/state`
/// (python `javis_lane.state_dir` 미러 · 훅 CLI 가 쓰는 **생산 경로**).
///
/// ★데몬은 이 함수를 쓰지 않는다 — `cysd/delivery.rs::pack_state_dir` 이 같은 규약에
/// **테스트 샌드박스 분기**를 더해 소유한다(테스트가 라이브 원장을 더럽히지 않게 하는 방어선).
/// 두 함수가 갈리는 것은 루트뿐이고 파일명 규약은 아래 `*_in` 하나가 소유한다.
pub fn state_dir() -> PathBuf {
    match std::env::var("CYS_STATE_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => crate::home_dir().join(".cys").join("state"),
    }
}

/// 배달 원장 경로 — **항상 레인 접미**(base 레인도 `delivery-base.jsonl`).
/// python `lane_state_path("delivery")` 와 같은 이름을 낸다(`ALWAYS_LANE_SUFFIXED`).
pub fn ledger_path_in(dir: &Path, socket_path: &Path) -> PathBuf {
    dir.join(format!("delivery-{}.jsonl", lane_key(socket_path)))
}

/// 데몬 인스턴스 표식 경로 — 임무의 **세션 결박**(과거 임무 무기한 유효 차단).
/// python `lane_state_path("delivery_epoch")` 미러 — 이쪽도 항상 레인 접미다.
pub fn epoch_path_in(dir: &Path, socket_path: &Path) -> PathBuf {
    dir.join(format!("delivery-{}.epoch.json", lane_key(socket_path)))
}

/// 임무 대장 경로 — python `lane_state_path("mission")` 미러.
///
/// ★위 둘과 **접미 규약이 다르다**: `mission` 은 python `ALWAYS_LANE_SUFFIXED` **밖**이라
/// base 레인에서는 역사적 무접미 경로(`mission.json`)이고 비-base 레인에서만
/// `mission-<lane>.json` 이다. 여기서 접미를 항상 붙이면 훅(Rust)이 쓴 대장을 게이트(python)가
/// **찾지 못한다** — 임무가 있는데 없다고 판정하는 무음 결함이라 반드시 python 규약을 따른다.
pub fn mission_path_in(dir: &Path, socket_path: &Path) -> PathBuf {
    let key = lane_key(socket_path);
    if key == "base" {
        dir.join("mission.json")
    } else {
        dir.join(format!("mission-{key}.json"))
    }
}

/// [`mission_path_in`] + [`state_dir`] — 훅 CLI 의 생산 경로.
pub fn mission_path(socket_path: &Path) -> PathBuf {
    mission_path_in(&state_dir(), socket_path)
}

/// [`epoch_path_in`] + [`state_dir`] — 훅 CLI 가 `boot_epoch`(세션 결박)를 **읽는** 경로.
/// 쓰는 쪽은 언제나 데몬이다(`cysd/delivery.rs::write_epoch`).
pub fn epoch_path(socket_path: &Path) -> PathBuf {
    epoch_path_in(&state_dir(), socket_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 아래 두 검체는 `cysd/delivery.rs` 에서 **코드와 함께 옮겨왔다**(B2-c e-1 · 단언 무변경).
    //    주체(sha1 슬러그·긴 경로 키)가 이 모듈로 이사했으므로 검체도 같이 온다.
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

    /// ★H-LANE-PATH-1(B2-c e-1): **파일명 규약이 python `lane_state_path` 와 같은가.**
    ///
    /// 세 종류가 한 함수(`lane_key`)를 통과하되 **접미 규칙은 둘로 갈린다**:
    ///   · `delivery`·`delivery_epoch` — 항상 레인 접미(base 도 `-base`).
    ///   · `mission` — base 는 **무접미**(`mission.json`), 비-base 만 `mission-<lane>.json`.
    /// 이 비대칭을 틀리면 훅(Rust)이 쓴 대장을 게이트(python)가 못 찾는다 — 임무가 있는데
    /// 없다고 판정하는 **무음 결함**이라 값으로 못박는다.
    #[test]
    fn lane_paths_mirror_the_python_lane_state_path_contract() {
        let dir = Path::new("/tmp/state-x");
        // base 레인 — 기본 소켓 모양.
        let base = Path::new("/Users/x/.local/state/cys/cys.sock");
        assert_eq!(lane_key(base), "base");
        assert_eq!(mission_path_in(dir, base), dir.join("mission.json"));
        assert_eq!(ledger_path_in(dir, base), dir.join("delivery-base.jsonl"));
        assert_eq!(epoch_path_in(dir, base), dir.join("delivery-base.epoch.json"));

        // 비-base 레인 — 실사고 격리 소켓. 셋 다 같은 키를 통과한다.
        let iso = Path::new("/Users/x/.cys/state-harness/cys.sock");
        let key = lane_key(iso);
        assert_ne!(key, "base", "격리 소켓이 base 로 접혔다 — P0 회귀");
        assert_eq!(mission_path_in(dir, iso), dir.join(format!("mission-{key}.json")));
        assert_eq!(ledger_path_in(dir, iso), dir.join(format!("delivery-{key}.jsonl")));
        assert_eq!(epoch_path_in(dir, iso), dir.join(format!("delivery-{key}.epoch.json")));

        // ★교차언어 앵커 — 아래 세 문자열은 **python 실측값**이다(`javis_lane.lane_state_path`
        //   를 CYS_STATE_DIR=/tmp/state-x 로 돌려 얻은 basename 그대로). 규칙을 옮겨 적은 것이
        //   아니라 정본이 낸 값을 박제한 것이라, 규칙을 바꾸면 이 셋이 먼저 적색이 된다.
        //   입력은 두 언어가 **오늘 같은 답을 내는** 소켓을 골랐다(python 의 is_base 수리는
        //   W-A 대기분이라 격리 소켓으로는 아직 교차 대조가 성립하지 않는다).
        let custom = Path::new("/tmp/whatever.sock");
        assert_eq!(lane_key(custom), "tmp_whatever.sock");
        assert_eq!(
            mission_path_in(dir, custom),
            dir.join("mission-tmp_whatever.sock.json")
        );
        assert_eq!(
            ledger_path_in(dir, custom),
            dir.join("delivery-tmp_whatever.sock.jsonl")
        );
        assert_eq!(
            epoch_path_in(dir, custom),
            dir.join("delivery-tmp_whatever.sock.epoch.json")
        );

        // ★음성 대조: mission 에 항상-접미 규약을 잘못 적용하면 base 에서 이 단언이 깨진다.
        assert_ne!(
            mission_path_in(dir, base),
            dir.join("mission-base.json"),
            "base 대장에 레인 접미가 붙었다 — python 게이트가 이 파일을 찾지 못한다"
        );
    }

    /// 루트 해소가 `CYS_STATE_DIR` 을 존중하는가 — 격리 하네스(IG-13)의 전제.
    /// env 를 만지므로 다른 테스트와 겹치지 않도록 **이 테스트 안에서만** 세우고 되돌린다.
    #[test]
    fn state_dir_prefers_the_isolation_env() {
        let prev = std::env::var("CYS_STATE_DIR").ok();
        std::env::set_var("CYS_STATE_DIR", "/tmp/lane-iso-root");
        assert_eq!(state_dir(), PathBuf::from("/tmp/lane-iso-root"));
        // 공백만 있는 값은 미설정과 같다(python `or` 의미 — 빈 문자열은 falsy).
        std::env::set_var("CYS_STATE_DIR", "   ");
        assert!(state_dir().ends_with(".cys/state"), "빈 값이 루트를 삼켰다: {:?}", state_dir());
        match prev {
            Some(v) => std::env::set_var("CYS_STATE_DIR", v),
            None => std::env::remove_var("CYS_STATE_DIR"),
        }
    }
}
