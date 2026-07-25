//! HMAC signed-prefix 승인 primitive (규약→기술 강제 ①).
//!
//! 자율주행 denylist 위험명령의 "1회 승인"을 `command_prefix + cwd + environment`에 대한
//! HMAC-SHA256 서명 레코드로 영속하고, 이후 동일 prefix 명령은 서명 검증으로만 자동 통과시킨다.
//! 시크릿 없이는 레코드를 위조할 수 없으므로(서명 불일치 hard-reject) 승인은 암호학적으로
//! 위조 불가능하다. base64·HMAC-SHA256은 외부 crate 0(sha2 0.10만)으로 수동 구현한다 —
//! recall.rs:hash_step의 Sha256 패턴을 ipad/opad로 확장. RFC 4231 KAT로 정확성을 박제한다.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ── 자료구조 ────────────────────────────────────────────────────────────────

/// 서명된 prefix 승인 레코드. cmux SurfaceResumeApprovalRecord의 cys 단순화(단일머신).
/// environment는 정렬된 Vec<(String,String)>로 — serde_json 맵 순서 비결정성을 피하고
/// 서명 직렬화와 일치시킨다(결정론 서명의 핵심).
#[derive(Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub version: u32,
    pub id: String,
    pub command_prefix: Vec<String>, // 빈 벡터 금지(폴백 차단)
    pub cwd: Option<String>,         // normalized
    pub environment: Vec<(String, String)>, // 정렬·민감키 drop 후
    pub created_at: f64,
    pub updated_at: f64,
    pub signature: String, // base64(HMAC-SHA256(payload))
}

// ── 수동 base64 (표준 알파벳, 의존 0) ─────────────────────────────────────────

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 표준 base64 인코딩(패딩 `=` 포함). 서명 직렬화·HMAC 출력 인코딩 전용.
pub fn b64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 표준 base64 디코딩(패딩 허용·내부 공백 무시). 잘못된 문자가 있으면 None.
pub fn b64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut symbols: Vec<u32> = Vec::with_capacity(input.len());
    let mut pad = 0usize;
    let mut seen_pad = false;
    for &c in input.as_bytes() {
        match c {
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            b'=' => {
                pad += 1;
                seen_pad = true;
            }
            _ => {
                if seen_pad {
                    return None; // 패딩 뒤 데이터 = 손상
                }
                symbols.push(val(c)?);
            }
        }
    }
    if (symbols.len() + pad) % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(symbols.len() / 4 * 3);
    for chunk in symbols.chunks(4) {
        let n = (chunk[0] << 18)
            | (chunk.get(1).copied().unwrap_or(0) << 12)
            | (chunk.get(2).copied().unwrap_or(0) << 6)
            | chunk.get(3).copied().unwrap_or(0);
        out.push(((n >> 16) & 0xff) as u8);
        if chunk.len() > 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if chunk.len() > 3 {
            out.push((n & 0xff) as u8);
        }
    }
    Some(out)
}

// ── 수동 HMAC-SHA256 (recall.rs:hash_step 확장 — ipad/opad, 의존 0) ────────────

/// HMAC-SHA256(RFC 2104). 키>64B면 sha256(key)로 축약, 키<64B면 0패딩.
/// RFC 4231 KAT(hmac_kat 테스트)가 정확성을 증명한다 — KAT 실패=구현 버그.
pub fn hmac_sha256(secret: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key = [0u8; BLOCK];
    if secret.len() > BLOCK {
        let h: [u8; 32] = {
            let mut s = Sha256::new();
            s.update(secret);
            s.finalize().into()
        };
        key[..32].copy_from_slice(&h);
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let inner: [u8; 32] = {
        let mut s = Sha256::new();
        s.update(ipad);
        s.update(msg);
        s.finalize().into()
    };
    let outer: [u8; 32] = {
        let mut s = Sha256::new();
        s.update(opad);
        s.update(inner);
        s.finalize().into()
    };
    outer
}

/// 상수시간 바이트 비교 — 서명 검증의 조기반환 타이밍 사이드채널 차단.
/// 길이 다르면 즉시 false(길이는 비밀이 아님), 같으면 전 바이트 XOR 누적 후 0 판정.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// ── 결정론 직렬화 + 서명/검증 ─────────────────────────────────────────────────

impl ApprovalRecord {
    /// cmux signingPayloadData 1:1 흡수 — 각 필드 base64 후 newline-join, environment는
    /// 키 정렬(이미 정렬 전제) 후 `b64key=b64val,…`. base64+구분자가 충돌(따옴표/등호/콤마/
    /// 줄바꿈을 값에 넣어 필드 경계를 위조)을 차단한다.
    pub fn signing_payload(&self) -> Vec<u8> {
        let prefix = self
            .command_prefix
            .iter()
            .map(|t| b64_encode(t.as_bytes()))
            .collect::<Vec<_>>()
            .join(",");
        let env = self
            .environment
            .iter() // 이미 정렬됨(sort_norm_env가 보장)
            .map(|(k, v)| format!("{}={}", b64_encode(k.as_bytes()), b64_encode(v.as_bytes())))
            .collect::<Vec<_>>()
            .join(",");
        let fields = [
            format!("version={}", self.version),
            format!("id={}", self.id),
            format!("commandPrefix={prefix}"),
            format!(
                "cwd={}",
                self.cwd
                    .as_deref()
                    .map(|c| b64_encode(c.as_bytes()))
                    .unwrap_or_default()
            ),
            format!("environment={env}"),
            format!("createdAt={}", self.created_at),
            format!("updatedAt={}", self.updated_at),
        ];
        fields.join("\n").into_bytes()
    }

    pub fn sign(&mut self, secret: &[u8]) {
        self.signature = b64_encode(&hmac_sha256(secret, &self.signing_payload()));
    }

    /// 상수시간 비교로 서명 검증 — 재서명 후 동치 비교(타이밍릭 차단).
    pub fn has_valid_signature(&self, secret: &[u8]) -> bool {
        let expect = b64_encode(&hmac_sha256(secret, &self.signing_payload()));
        constant_time_eq(self.signature.as_bytes(), expect.as_bytes())
    }

    /// 명령이 이 레코드 prefix에 매칭하는가: prefix가 명령 토큰의 정확한 접두 + cwd 완전일치
    /// + environment 부분집합(레코드 env가 호출 env에 모두 포함). 빈 prefix·미닫힌 따옴표 거부.
    pub fn matches(&self, command: &str, cwd: Option<&str>, env: &[(String, String)]) -> bool {
        if self.command_prefix.is_empty() {
            return false; // 폴백 차단
        }
        let Some(toks) = tokenize(command) else {
            return false; // 미닫힌 따옴표 = prefix injection 차단
        };
        if toks.len() < self.command_prefix.len() {
            return false;
        }
        if toks[..self.command_prefix.len()] != self.command_prefix[..] {
            return false;
        }
        if let Some(rc) = &self.cwd {
            if normalize_cwd(cwd).as_deref() != Some(rc.as_str()) {
                return false;
            }
        }
        // environment 부분집합: 레코드의 (민감키 drop·정렬된) env 항목이 모두 호출 env에 존재.
        // 호출 측이 추가 env를 더 가져도 매칭(미세 변동 내성) — 단 레코드가 요구한 키-값은 강제.
        let call_env = sort_norm_env(env);
        self.environment
            .iter()
            .all(|kv| call_env.binary_search(kv).is_ok())
    }
}

/// 서명 유효 + 매칭 레코드 중 최장 prefix(동률은 updated_at 최신) 선택.
pub fn best_match<'a>(
    records: &'a [ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
) -> Option<&'a ApprovalRecord> {
    records
        .iter()
        .filter(|r| r.has_valid_signature(secret)) // 서명 유효만
        .filter(|r| r.matches(command, cwd, env))
        .max_by(|a, b| {
            a.command_prefix
                .len()
                .cmp(&b.command_prefix.len())
                .then(
                    a.updated_at
                        .partial_cmp(&b.updated_at)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        })
}

// ── 토큰화 / 정규화 / 민감 env ─────────────────────────────────────────────────

/// 셸 토크나이저(cmux SurfaceResumeCommandCanonicalizer.tokens 포팅): 따옴표('/")·백슬래시
/// 인식. 미닫힌 따옴표는 None(거부). shell Turing-complete 한계(파이프·;·$())는 prefix
/// 매칭으로 blast radius만 좁힌다(완전차단 아님).
pub fn tokenize(command: &str) -> Option<Vec<String>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut has_token = false;
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None; // 따옴표 닫힘
                } else if c == '\\' && q == '"' {
                    // 큰따옴표 안의 백슬래시: 다음 문자 리터럴(POSIX 근사)
                    if let Some(&n) = chars.peek() {
                        if n == '"' || n == '\\' || n == '$' || n == '`' {
                            cur.push(chars.next().unwrap());
                        } else {
                            cur.push('\\');
                        }
                    } else {
                        cur.push('\\');
                    }
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    has_token = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                        has_token = true;
                    }
                }
                ' ' | '\t' | '\n' | '\r' => {
                    if has_token {
                        tokens.push(std::mem::take(&mut cur));
                        has_token = false;
                    }
                }
                _ => {
                    cur.push(c);
                    has_token = true;
                }
            },
        }
    }
    if quote.is_some() {
        return None; // 미닫힌 따옴표 = 거부
    }
    if has_token {
        tokens.push(cur);
    }
    Some(tokens)
}

/// cwd 정규화 — tilde 확장 + 후행 슬래시 제거. 단일머신 전제(symlink 정규화는 비용·미사용).
pub fn normalize_cwd(cwd: Option<&str>) -> Option<String> {
    let raw = cwd?;
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(rest).to_string_lossy().to_string()
        } else {
            raw.to_string()
        }
    } else if raw == "~" {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.to_string())
    } else {
        raw.to_string()
    };
    let trimmed = expanded.trim_end_matches('/');
    Some(if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    })
}

/// 민감키 drop(서명 페이로드에 시크릿값 미포함) + 키 정렬(결정론·binary_search 전제).
/// cmux isSensitiveEnvironmentKey 흡수 — 키 대문자화 후 부분일치 drop.
pub fn sort_norm_env(env: &[(String, String)]) -> Vec<(String, String)> {
    const SENSITIVE: &[&str] = &[
        "API_KEY",
        "ACCESS_KEY",
        "AUTH_TOKEN",
        "BEARER_TOKEN",
        "PRIVATE_KEY",
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "TOKEN",
        "CREDENTIAL",
        "COOKIE",
    ];
    let mut out: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| {
            let up = k.to_uppercase();
            !SENSITIVE.iter().any(|s| up.contains(s))
        })
        .cloned()
        .collect();
    out.sort();
    out
}

// ── 시크릿 저장 (cmux fileBackedSecret 흡수 — keyring crate 부재 대안) ──────────

const ENV_SECRET_B64: &str = "CYS_APPROVAL_SECRET_B64";

/// 시크릿 파일 경로: ~/.cys/.approval-secret — pack(~/.cys/pack) 밖, ~/.cys/ 직하.
/// pack은 배포·git 추적 대상일 수 있으므로 시크릿이 새지 않게 분리한다.
fn secret_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys")
        .join(".approval-secret")
}

/// 승인 레코드 영속 경로: ~/.cys/approvals.json (0600).
fn records_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys")
        .join("approvals.json")
}

/// 우선순위: ① env override(B64, 로깅 금지) → ② 0600 파일 → ③ 생성·0600 저장.
/// 파일 쓰기 실패해도 in-memory secret 반환(세션 한정 — 재시작 시 재생성).
pub fn signing_secret() -> Option<Vec<u8>> {
    // ① env override(B64) — 로깅·이벤트 payload에 절대 미포함.
    if let Ok(b64) = std::env::var(ENV_SECRET_B64) {
        if let Some(d) = b64_decode(&b64) {
            if !d.is_empty() {
                return Some(d);
            }
        }
    }
    // ② 0600 파일(pack 밖 ~/.cys/ 하위) — Keychain crate 부재라 1차 경로.
    let path = secret_path();
    if let Ok(d) = std::fs::read(&path) {
        if !d.is_empty() {
            return Some(d);
        }
    }
    // ③ 생성 + 0600 저장.
    let secret = random_32()?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, &secret).is_ok() {
        set_owner_only(&path);
        return Some(secret);
    }
    Some(secret) // 파일 실패해도 세션 한정 secret 반환.
}

/// 0600 권한 부여(Unix). Windows는 ACL 미설정(단일 사용자 데스크톱 전제).
fn set_owner_only(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path; // no-op
    }
}

/// 32바이트 난수. Unix=/dev/urandom 직접 read(getrandom/OsRng crate 부재).
/// 비-Unix는 시간 기반 PRNG 폴백(단일 데스크톱 전제 — 위협모델상 허용, 명문화).
fn random_32() -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        // ★/dev/urandom은 무한 스트림 — std::fs::read(전체 읽기)는 EOF가 없어 영영 반환하지
        //   않는다(hang+무한메모리). 반드시 read_exact로 32바이트만 채운다.
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 32];
            if f.read_exact(&mut buf).is_ok() {
                return Some(buf.to_vec());
            }
        }
        // /dev/urandom 읽기 실패(컨테이너 등) — 아래 폴백으로.
    }
    // 폴백: 시간+pid 기반 SHA256(약한 엔트로피 — Unix urandom 실패 시 한정).
    let seed = format!(
        "{}-{}-{:?}",
        std::process::id(),
        crate::state::now_epoch(),
        std::time::SystemTime::now()
    );
    let mut s = Sha256::new();
    s.update(seed.as_bytes());
    let h: [u8; 32] = s.finalize().into();
    Some(h.to_vec())
}

// ── 레코드 영속 (JSON 0600, atomic tmp+rename) ────────────────────────────────

/// 저장 포맷: `{"records":[...]}` 또는 bare 배열 둘 다 디코드(cmux 하위호환).
pub fn load_records() -> Vec<ApprovalRecord> {
    let path = records_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    // ① {"records":[...]} 형태
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(arr) = v.get("records") {
            if let Ok(recs) = serde_json::from_value::<Vec<ApprovalRecord>>(arr.clone()) {
                return recs;
            }
        }
    }
    // ② bare 배열
    serde_json::from_str::<Vec<ApprovalRecord>>(&content).unwrap_or_default()
}

/// atomic write: tmp 작성·0600 부여 후 rename. 디렉토리 자동 생성.
pub fn save_records(records: &[ApprovalRecord]) -> Result<(), String> {
    let path = records_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({"records": records}))
        .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body.as_bytes()).map_err(|e| e.to_string())?;
    set_owner_only(&tmp);
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    set_owner_only(&path);
    Ok(())
}

/// 신규 레코드 id 생성: epoch초 + 프로세스 카운터(동일 초 충돌 차단).
pub fn new_record_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "ap-{}-{}-{}",
        std::process::id(),
        crate::state::now_epoch() as u64,
        COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

/// JSON 객체 {"K":"V",...}를 정렬·민감키 drop된 Vec<(String,String)>로 — RPC env 파라미터 정규화.
pub fn env_from_json(v: &serde_json::Value) -> Vec<(String, String)> {
    let raw: Vec<(String, String)> = v
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    sort_norm_env(&raw)
}

// ── 테스트 (E-n: 10종, hmac_kat = RFC 4231) ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-approval-secret-32-bytes!!!";

    fn rec(prefix: &[&str], cwd: Option<&str>, env: &[(&str, &str)]) -> ApprovalRecord {
        ApprovalRecord {
            version: 1,
            id: "ap-test-1".to_string(),
            command_prefix: prefix.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(|c| c.to_string()),
            environment: sort_norm_env(
                &env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<Vec<_>>(),
            ),
            created_at: 1000.0,
            updated_at: 1000.0,
            signature: String::new(),
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        assert!(!r.signature.is_empty());
        assert!(r.has_valid_signature(SECRET));
    }

    #[test]
    fn tampered_rejected() {
        // 변종 1: command_prefix 변조
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        let mut t1 = r.clone();
        t1.command_prefix = vec!["git".into(), "pull".into()];
        assert!(!t1.has_valid_signature(SECRET), "command_prefix 변조 통과");

        // 변종 2: cwd 변조
        let mut r2 = rec(&["git", "push"], Some("/a"), &[]);
        r2.sign(SECRET);
        let mut t2 = r2.clone();
        t2.cwd = Some("/b".into());
        assert!(!t2.has_valid_signature(SECRET), "cwd 변조 통과");

        // 변종 3: env 변조
        let mut r3 = rec(&["git", "push"], None, &[("CI", "1")]);
        r3.sign(SECRET);
        let mut t3 = r3.clone();
        t3.environment = vec![("CI".into(), "2".into())];
        assert!(!t3.has_valid_signature(SECRET), "env 변조 통과");

        // 변종 4: signature 한 글자 변조
        let mut t4 = r.clone();
        let mut sig: Vec<char> = t4.signature.chars().collect();
        // 첫 글자를 다른 base64 문자로 치환
        sig[0] = if sig[0] == 'A' { 'B' } else { 'A' };
        t4.signature = sig.into_iter().collect();
        assert!(!t4.has_valid_signature(SECRET), "signature 변조 통과");
    }

    #[test]
    fn wrong_secret_rejected() {
        let mut r = rec(&["git", "push"], None, &[]);
        r.sign(SECRET);
        assert!(!r.has_valid_signature(b"a-completely-different-secret-key"));
    }

    #[test]
    fn prefix_match() {
        let r = rec(&["git", "push"], None, &[]);
        assert!(r.matches("git push origin main", None, &[]));
        assert!(!r.matches("git status", None, &[]), "git status 오매칭");
        assert!(!r.matches("git", None, &[]), "토큰 부족인데 매칭");
    }

    #[test]
    fn empty_prefix_no_fallback() {
        let r = rec(&[], None, &[]);
        assert!(!r.matches("anything goes here", None, &[]));
    }

    #[test]
    fn unclosed_quote_rejected() {
        assert!(tokenize("git push 'x").is_none(), "미닫힌 따옴표 토큰화 통과");
        let r = rec(&["git", "push"], None, &[]);
        assert!(
            !r.matches("git push 'unterminated", None, &[]),
            "미닫힌 따옴표 명령 매칭 통과(prefix injection)"
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let mut short = rec(&["git"], None, &[]);
        short.id = "ap-short".into();
        short.sign(SECRET);
        let mut long = rec(&["git", "push"], None, &[]);
        long.id = "ap-long".into();
        long.sign(SECRET);
        let recs = vec![short, long];
        let best = best_match(&recs, SECRET, "git push origin main", None, &[]).unwrap();
        assert_eq!(best.id, "ap-long", "최장 prefix 미선택");
    }

    #[test]
    fn sensitive_env_dropped() {
        let r = rec(&["deploy"], None, &[("API_KEY", "leak"), ("CI", "1")]);
        // 레코드 environment에서 API_KEY가 제거되고 CI만 남는다.
        assert!(
            r.environment.iter().all(|(k, _)| k != "API_KEY"),
            "API_KEY가 레코드에 잔존"
        );
        assert!(r.environment.iter().any(|(k, _)| k == "CI"));
        // 서명 페이로드에도 시크릿값이 없어야 한다.
        let payload = String::from_utf8(r.signing_payload()).unwrap();
        let leaked = b64_encode(b"leak");
        assert!(!payload.contains(&leaked), "시크릿값이 서명 페이로드에 유출");
    }

    #[test]
    fn determinism() {
        let mut r1 = rec(&["git", "push"], Some("/x"), &[("CI", "1"), ("AAA", "2")]);
        let mut r2 = rec(&["git", "push"], Some("/x"), &[("AAA", "2"), ("CI", "1")]);
        r1.sign(SECRET);
        r2.sign(SECRET);
        assert_eq!(r1.signature, r2.signature, "동일 입력 2회 서명 불일치(비결정)");
    }

    /// RFC 4231 Test Case 2 — 수동 HMAC-SHA256 정확성 박제.
    /// Key = "Jefe", Data = "what do ya want for nothing?"
    /// HMAC-SHA256 = 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
    #[test]
    fn hmac_kat() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let expected: [u8; 32] = [
            0x5b, 0xdc, 0xc1, 0x46, 0xbf, 0x60, 0x75, 0x4e, 0x6a, 0x04, 0x24, 0x26, 0x08, 0x95,
            0x75, 0xc7, 0x5a, 0x00, 0x3f, 0x08, 0x9d, 0x27, 0x39, 0x83, 0x9d, 0xec, 0x58, 0xb9,
            0x64, 0xec, 0x38, 0x43,
        ];
        assert_eq!(hmac_sha256(key, data), expected, "RFC 4231 TC2 KAT 실패");

        // RFC 4231 Test Case 1 — Key = 0x0b*20, Data = "Hi There"
        // HMAC-SHA256 = b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7
        let key1 = [0x0bu8; 20];
        let data1 = b"Hi There";
        let expected1: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(hmac_sha256(&key1, data1), expected1, "RFC 4231 TC1 KAT 실패");

        // RFC 4231 Test Case 3 — long key path (key>64B 축약 검증):
        // Key = 0xaa*131, Data = "Test Using Larger Than Block-Size Key - Hash Key First"
        // HMAC-SHA256 = 60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54
        let key3 = [0xaau8; 131];
        let data3 = b"Test Using Larger Than Block-Size Key - Hash Key First";
        let expected3: [u8; 32] = [
            0x60, 0xe4, 0x31, 0x59, 0x1e, 0xe0, 0xb6, 0x7f, 0x0d, 0x8a, 0x26, 0xaa, 0xcb, 0xf5,
            0xb7, 0x7f, 0x8e, 0x0b, 0xc6, 0x21, 0x37, 0x28, 0xc5, 0x14, 0x05, 0x46, 0x04, 0x0f,
            0x0e, 0xe3, 0x7f, 0x54,
        ];
        assert_eq!(
            hmac_sha256(&key3, data3),
            expected3,
            "RFC 4231 TC3 KAT 실패(키 축약)"
        );
    }

    #[test]
    fn b64_roundtrip() {
        for s in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"\x00\xff\x10",
        ] {
            let enc = b64_encode(s);
            assert_eq!(b64_decode(&enc).as_deref(), Some(s), "base64 라운드트립 실패");
        }
        // 알려진 벡터(RFC 4648)
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }
}
