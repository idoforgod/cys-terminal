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
    /// ★(0.14.31 · CONTRACTS B-3) TTL 만료 시각(epoch초). `None` = 무기한(구 레코드 포함).
    ///
    /// **서명 페이로드에 들어간다** — 그러나 `Some` 일 때만 줄이 덧붙는다(아래 `signing_payload`).
    /// 그래서 TTL 없는 구 레코드의 페이로드는 **바이트 동일**이고 기존 서명이 그대로 유효하다
    /// (승인 전수 무효화 = 자율주행 정지 사고를 만들지 않는다). 반대로 만료 시각을 떼거나
    /// 늘리려는 편집은 페이로드를 바꾸므로 서명 불일치로 hard-reject 된다.
    /// `skip_serializing_if` 로 None 은 키 자체를 쓰지 않는다 — 구 데몬이 읽어도 형상 무변화.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
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
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
        let mut fields = vec![
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
        // ★(0.14.31 · B-3) TTL 은 **조건부 말미 추가**다 — `None` 이면 한 글자도 붙지 않는다.
        //   이 비대칭이 계약의 전부다: 구 레코드(무TTL)의 페이로드가 종전과 바이트 동일해야
        //   기존 서명이 살아남고(무효화 0), TTL 이 있는 레코드는 그 값까지 서명에 묶여
        //   만료 연장·제거가 위조로 판정된다. 순서를 바꾸거나 무조건 추가로 바꾸지 마라 —
        //   그 순간 설치된 모든 승인이 한 번에 무효가 된다(자율주행 전면 정지).
        if let Some(exp) = self.expires_at {
            fields.push(format!("expiresAt={exp}"));
        }
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

    /// ★(0.14.31 · B-3) 이 레코드가 `now` 기준 만료됐는가. TTL 없는 레코드는 만료되지 않는다
    /// (무기한 — 종전 계약 보존). 경계는 `expires_at <= now` = 만료(만료 시각 그 순간은 이미 죽었다).
    pub fn is_expired(&self, now: f64) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
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
///
/// ★(0.14.31 · B-3) 시각·TTL 축은 [`best_match_at`] 이 소유한다. 이 얇은 래퍼는 "지금"과
/// `require_ttl=false`(TTL 요구 없음)로 위임할 뿐이다 — **만료 레코드는 여기서도 매칭되지
/// 않는다**(만료는 요구 여부와 무관한 사실이다).
pub fn best_match<'a>(
    records: &'a [ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
) -> Option<&'a ApprovalRecord> {
    best_match_at(records, secret, command, cwd, env, crate::state::now_epoch(), false)
}

/// [`best_match`] + 시각 주입(순수 테스트용) + `require_ttl`.
///
/// · `require_ttl=false`: 무기한 레코드도 통과(종전 계약) · 만료된 TTL 레코드는 **거부**.
/// · `require_ttl=true` : `expires_at` 이 **있고** 아직 만료되지 않은 레코드만 통과.
///   TTL 없는 구 레코드는 거부된다(계약 B-3 — '무기한 승인으로 TTL 게이트를 통과' 차단).
pub fn best_match_at<'a>(
    records: &'a [ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
) -> Option<&'a ApprovalRecord> {
    best_match_index_at(records, secret, command, cwd, env, now, require_ttl).map(|i| &records[i])
}

/// [`best_match_at`] 과 **같은 선택**의 인덱스 판(호출부가 그 레코드를 갱신해야 할 때).
///
/// ★왜 id 가 아니라 인덱스인가(codex 적대검증 major): `approval.check` 는 매칭 레코드의
/// `updated_at` 을 갱신하고 **재서명**한다. 그 대상을 `id` 로 다시 찾으면, 승인 파일에 **같은 id
/// 를 가진 위조 레코드**(서명 무효)를 앞에 끼워 넣은 공격자가 검증받은 적 없는 그 레코드를
/// 데몬의 손으로 정당 서명시킬 수 있다(서명 세탁 — 다음 check 부터 공격자가 정한 범위·만료가
/// 유효해진다). 검증한 **그 자리**를 갱신하면 그 경로가 원리상 닫힌다.
pub fn best_match_index_at(
    records: &[ApprovalRecord],
    secret: &[u8],
    command: &str,
    cwd: Option<&str>,
    env: &[(String, String)],
    now: f64,
    require_ttl: bool,
) -> Option<usize> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.has_valid_signature(secret)
                && !r.is_expired(now)
                && (!require_ttl || r.expires_at.is_some())
                && r.matches(command, cwd, env)
        })
        .max_by(|(_, a), (_, b)| {
            a.command_prefix
                .len()
                .cmp(&b.command_prefix.len())
                .then(
                    a.updated_at
                        .partial_cmp(&b.updated_at)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        })
        .map(|(i, _)| i)
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
    match std::fs::read(&path) {
        Ok(d) if !d.is_empty() => return Some(d),
        // 0바이트 = 미완성 쓰기의 잔재. 잃을 것이 없으므로 아래에서 새로 만든다.
        Ok(_) => {}
        // 부재 = 아직 만들지 않았다(정상 초기 상태) → 아래에서 만든다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // ★(독립 재유도 · codex 설계비평 #15) **읽지 못한 시크릿은 덮어쓰지 않는다.**
        //   종전엔 권한 오류·IO 오류도 '없음'으로 접고 새 시크릿을 그 자리에 썼다 — 성공하면
        //   기존 승인 **전부**가 서명 불일치로 영구 무효가 된다(승인 저장소 되쓰기와 같은
        //   클래스의 파괴). 모르면 서명 능력을 포기한다: 호출부는 미서명 취급(fail-closed)이라
        //   방향은 '거부'이고, 사람이 권한을 고치면 그대로 돌아온다.
        Err(e) => {
            eprintln!(
                "cysd: 승인 시크릿을 읽지 못했다({}: {e}) — 새로 만들지 않고 미서명으로 접는다\
                 (기존 승인을 무효화하지 않기 위함 · 권한을 고치면 복구된다)",
                path.display()
            );
            return None;
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

/// ★(0.14.31 · WP-4 R2 · codex major) **TTL 레코드 전용 저장소** — `~/.cys/approvals-ttl.json`.
///
/// 【무엇을 고치는가】 승인 저장소는 `$HOME` 아래 **공유 파일**이고, 기준 커밋(0.14.30)의
/// `ApprovalRecord` 에는 `expires_at` 이 없다. 구 데몬은 파일 전체를 typed 레코드로 읽고 다시
/// 직렬화하므로(모르는 필드는 조용히 버린다), 같은 HOME 에서 신·구 데몬이 함께 돌 때 구 데몬이
/// **다른** 승인 하나를 정상 사용하기만 해도 TTL 레코드의 `expires_at` 이 사라진다. 그 값은
/// 서명 페이로드에 들어가 있으므로 신 데몬은 그 뒤로 그 레코드를 **서명 불일치로 영구 거부**한다
/// — 만료 전 승인이 되살릴 수 없이 망가진다(codex 적대검증 R2 major).
///
/// 【어떻게 고치는가】 TTL 이 있는 레코드는 **구 데몬이 존재조차 모르는 파일**에만 쓴다.
///   · 파괴 불가 — 구 writer 는 이 파일을 열지 않는다.
///   · 그리고 **구 데몬이 TTL 승인을 통과시키지도 못한다**(그 데몬은 `expires_at` 을 무시하므로
///     종전 배치에서는 만료된 승인을 무기한으로 오독했다). 못 보는 것이 오독보다 안전하다.
/// 병합은 읽을 때 한 번(`load_records`), 분리는 쓸 때 한 번(`save_records`) 일어나므로 호출부는
/// 종전 그대로다.
fn ttl_records_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cys")
        .join("approvals-ttl.json")
}

/// 한 파일에서 레코드 목록 디코드: `{"records":[...]}` 또는 bare 배열 둘 다(cmux 하위호환).
///
/// ★(0.14.31 · 독립 재유도 · codex blocking #4) **부재와 실패를 가른다.** 종전 판은 열기 실패도
/// 파싱 실패도 `Vec::new()` 로 접었고, 호출부(`mutate_records`)는 그 빈 목록을 **그대로 저장**
/// 했다 — 권한이 막힌 파일·레코드 하나가 깨진 파일이 아무 `approval.check` 한 번에
/// `{"records":[]}` 로 갈아끼워졌다(사람이 서명한 승인의 영구 파괴 · tmp→rename 은 상위
/// 디렉터리 권한만 있으면 성공한다). 그래서:
///   · 파일 **부재** → `Ok(vec![])` (승인이 아직 없다 — 정상 상태다)
///   · 그 밖의 IO 오류·JSON 파싱 실패 → `Err` → 호출부는 **쓰기 없이 트랜잭션을 중단**한다.
/// 판정 방향은 그대로 fail-closed 다: 읽지 못한 저장소는 "승인 없음"으로 답한다(거부).
fn load_records_from(path: &PathBuf) -> Result<Vec<ApprovalRecord>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: 읽기 실패: {e}", path.display())),
    };
    // 빈 파일은 '아직 아무 것도 안 썼다'로 읽는다(0바이트는 JSON 이 아니다 — 여기서 Err 로
    // 접으면 첫 서명 전 상태의 데몬이 승인을 만들 수 없다).
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    // ① {"records":[...]} 형태
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(arr) = v.get("records") {
            return serde_json::from_value::<Vec<ApprovalRecord>>(arr.clone())
                .map_err(|e| format!("{}: records 디코드 실패: {e}", path.display()));
        }
    }
    // ② bare 배열
    serde_json::from_str::<Vec<ApprovalRecord>>(&content)
        .map_err(|e| format!("{}: JSON 디코드 실패: {e}", path.display()))
}

/// 저장 포맷: `{"records":[...]}` 또는 bare 배열 둘 다 디코드(cmux 하위호환).
///
/// ★(R2) 두 저장소(공용 `approvals.json` + TTL 전용 `approvals-ttl.json`)를 **병합**해 돌려준다.
/// 같은 `id` 가 양쪽에 있으면 **TTL 쪽이 이긴다**: 그 상태는 분리 저장 중 중단(TTL 먼저 쓰고
/// 공용을 쓰기 전에 죽음)에서만 생기고, 그때 살아 있는 사실은 만료를 포함한 TTL 판이다.
pub fn load_records() -> Vec<ApprovalRecord> {
    // 읽기·파싱 실패는 **승인 없음**으로 답한다(fail-closed). 저장은 하지 않는다 —
    // 그 결정은 `try_load_records` 를 직접 쓰는 `mutate_records` 가 한다.
    try_load_records().unwrap_or_default()
}

/// [`load_records`] 의 **실패를 숨기지 않는** 판 — 두 저장소 중 하나라도 읽거나 파싱하지
/// 못하면 `Err`. 되쓰기 여부를 결정하는 트랜잭션(`mutate_records`)만 이 판을 쓴다.
pub fn try_load_records() -> Result<Vec<ApprovalRecord>, String> {
    let ttl = load_records_from(&ttl_records_path())?;
    let main = load_records_from(&records_path())?;
    let ttl_ids: std::collections::HashSet<&str> = ttl.iter().map(|r| r.id.as_str()).collect();
    let mut out: Vec<ApprovalRecord> = main
        .into_iter()
        .filter(|r| !ttl_ids.contains(r.id.as_str()))
        .collect();
    out.extend(ttl);
    Ok(out)
}

/// atomic write: tmp 작성·0600 부여 후 rename. 디렉토리 자동 생성.
fn save_records_to(path: &PathBuf, records: &[&ApprovalRecord]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({"records": records}))
        .map_err(|e| e.to_string())?;
    // ★(독립 재유도 · codex major #5 ②) tmp 이름에 **pid** 를 넣어 writer 를 분리한다.
    //   같은 `$HOME` 에 부서 데몬이 여럿 뜨는 배치에서 공용 `<name>.json.tmp` 하나를 두면,
    //   A 가 쓰는 중인 tmp 를 B 가 덮어쓰고 A 가 그 **B 의 내용**을 rename 하는 창이 있다.
    //   rename 은 같은 디렉터리 안이므로 원자성은 그대로다.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body.as_bytes()).map_err(|e| e.to_string())?;
    set_owner_only(&tmp);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    set_owner_only(path);
    Ok(())
}

/// atomic write: tmp 작성·0600 부여 후 rename. 디렉토리 자동 생성.
///
/// ★(R2) `expires_at` 유무로 **두 파일에 나눠** 쓴다(§ttl_records_path). 순서는 **TTL 먼저**다:
/// 중간에 죽으면 TTL 판이 두 파일에 겹쳐 남는데, 그 상태를 `load_records` 가 TTL 우선으로 접어
/// 손실 없이 복구한다(반대 순서면 만료 필드가 없는 판이 살아남아 서명이 깨진다).
/// 공용 파일에 `expires_at` 을 가진 레코드가 있으면 이 쓰기가 **자동 이주**시킨다.
pub fn save_records(records: &[ApprovalRecord]) -> Result<(), String> {
    let (ttl, plain): (Vec<&ApprovalRecord>, Vec<&ApprovalRecord>) =
        records.iter().partition(|r| r.expires_at.is_some());
    save_records_to(&ttl_records_path(), &ttl)?;
    save_records_to(&records_path(), &plain)
}

/// ★(R2 · codex major) 승인 저장소의 **읽기-변경-쓰기 트랜잭션**.
///
/// 【무엇을 고치는가】 호출부는 종전에 `load_records()` 로 스냅샷을 뜨고, 한참 뒤 그 스냅샷
/// 전체를 `save_records()` 로 되썼다. 그 사이 다른 요청이 승인을 하나 추가하면 **그 승인이
/// 사라진다**(갱신 손실). 종전엔 파일이 하나라 같은 클래스의 사고였지만, 저장소가 둘이 되면
/// 일반 승인 검사 하나가 TTL 파일까지 덮으므로 폭발 반경이 커진다.
///
/// 【계약】 이 함수 **안에서** 다시 읽고, 변경하고, 쓴다. 프로세스 안의 모든 변경은 이 뮤텍스로
/// 직렬화된다. 프로세스 **밖**(같은 HOME 의 다른 데몬)은 여전히 경쟁할 수 있다 — 그것은 이
/// 저장소가 처음부터 안고 있던 성질이고(파일 락 없음), 이 커밋의 범위 밖이다.
///
/// ★(0.14.31 · 독립 재유도) 이 트랜잭션이 파일을 **쓰는 조건**은 둘 다 참일 때뿐이다:
///   ① 두 저장소를 **읽어냈다**(`try_load_records` 가 `Ok`) — 읽지 못한 파일을 빈 목록으로
///      되쓰면 사람이 서명한 승인이 영구 소멸한다(codex blocking #4).
///   ② 클로저가 목록을 **실제로 바꿨다** — 무매칭 `approval.check` 하나까지 두 파일을
///      tmp→rename 으로 갈아끼우면, 그 사이 다른 부서 데몬이 서명한 승인이 이 스냅샷에
///      덮여 사라진다(프로세스 뮤텍스 밖 갱신 손실 · claude major / codex major #5).
/// 변경 판정은 **직렬화 동등성**이다: 필드 하나가 늘어도 자동으로 따라오고, `updated_at`
/// 재서명처럼 진짜 변경은 반드시 다르게 나온다. 승인 목록은 수십 건 규모라 비용이 무시된다.
/// 읽기에 실패하면 클로저는 **빈 목록** 위에서 돌고(호출부는 "승인 없음"으로 답한다 —
/// fail-closed) 저장은 하지 않으며, 실패 사유가 `Err` 로 나간다.
pub fn mutate_records<R>(
    f: impl FnOnce(&mut Vec<ApprovalRecord>) -> R,
) -> (Option<R>, Result<(), String>) {
    static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // poison 되어도 승인 저장은 계속돼야 한다(잠금 목적은 순서 직렬화뿐 — 불변식 보호가 아니다).
    let _g = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = match try_load_records() {
        Ok(r) => r,
        Err(e) => {
            // ★읽지 못한 저장소는 **건드리지 않고, 클로저도 돌리지 않는다**(codex 설계비평 #9):
            //   빈 목록 위에서 돌리면 "실패 상태에서 변형·부수효과를 실행한다"는 계약이 되고,
            //   호출부가 그 결과를 성공과 구별할 수 없다. `None` 이 곧 "판정하지 못했다"이고,
            //   호출부는 그것을 **거부**로 접는다(fail-closed).
            return (None, Err(format!("승인 저장소를 읽지 못해 트랜잭션을 중단했다 — {e}")));
        }
    };
    let before = fingerprint(&records);
    let out = f(&mut records);
    if before.is_some() && fingerprint(&records) == before {
        return (Some(out), Ok(())); // 변경 없음 = 쓸 이유 없음(경합 반경 0).
    }
    let saved = save_records(&records);
    (Some(out), saved)
}

/// 변경 판정용 지문 — 직렬화 실패(사실상 불가)는 `None` 으로 두고 "변경됐다"로 접는다
/// (모르면 쓰는 쪽이 데이터 보존 방향이다).
fn fingerprint(records: &[ApprovalRecord]) -> Option<Vec<u8>> {
    serde_json::to_vec(records).ok()
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
            expires_at: None,
            signature: String::new(),
        }
    }

    /// TTL 있는 레코드(만료 시각 지정) — B-3 검체 전용.
    fn rec_ttl(prefix: &[&str], cwd: Option<&str>, expires_at: f64) -> ApprovalRecord {
        let mut r = rec(prefix, cwd, &[]);
        r.expires_at = Some(expires_at);
        r
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

    // ── ★(0.14.31 · CONTRACTS B-3) 승인 TTL ────────────────────────────────────
    //
    // 계약: `sign --ttl <secs>` 가 `expires_at`(epoch)을 **서명 페이로드에 포함**하고,
    // `check --require-ttl` 은 만료되지 않은 TTL 레코드가 있을 때만 통과한다. TTL 없는
    // 구 레코드는 `--require-ttl` 에서 실패한다. 실패 방향은 전부 **거부(deny)** 다.

    /// ★무TTL 레코드의 서명 페이로드는 종전과 **바이트 동일**이어야 한다 — 아니면 이 릴리스가
    /// 설치된 모든 승인 레코드를 한 번에 무효화한다(자율주행 전면 정지 · 가용성 사고).
    ///
    /// ★R1 에서 고친 것(codex 적대검증 major): 종전 검체는 "마지막 줄이 `updatedAt=` 이고
    /// `expiresAt` 이 없다"는 **모양**만 봤다 — `createdAt` 의 포맷을 바꾸거나 `commandPrefix`
    /// 의 구분자를 바꿔도 초록이었다(그러면 설치된 서명이 전부 죽는데도). 그래서 여기서는
    /// **0.14.30 이 실제로 서명하던 그 바이트열과 그 서명**을 냉동 검체로 박는다. 이 두 상수는
    /// 코드에서 파생되지 않는다 — 밖에서 계산한 값이므로 페이로드 조립이 한 글자라도 달라지면
    /// 즉시 빨강이다.
    const FROZEN_LEGACY_PAYLOAD: &str = "version=1\nid=ap-test-1\ncommandPrefix=Z2l0,cHVzaA==\ncwd=L3g=\nenvironment=Q0k==MQ==\ncreatedAt=1000\nupdatedAt=1000";
    const FROZEN_LEGACY_SIG: &str = "2i7Ob8Rdqy1HDkUYBMGWbk9tiZ45iMSmWptf6GwXdVA=";

    #[test]
    fn ttl_absent_payload_is_byte_identical_to_legacy() {
        let r = rec(&["git", "push"], Some("/x"), &[("CI", "1")]);
        let payload = String::from_utf8(r.signing_payload()).unwrap();
        assert_eq!(
            payload, FROZEN_LEGACY_PAYLOAD,
            "무TTL 페이로드가 0.14.30 의 바이트열과 다르다 — 설치된 모든 승인 서명이 죽는다"
        );
        // 그 시절 서명이 **지금 코드에서도** 유효해야 한다(레코드 무효화 0의 진짜 정의).
        let mut legacy = r.clone();
        legacy.signature = FROZEN_LEGACY_SIG.to_string();
        assert!(
            legacy.has_valid_signature(SECRET),
            "구 릴리스가 만든 서명이 이 릴리스에서 거부됐다(승인 전멸 = 자율주행 정지)"
        );
        assert!(!payload.contains("expiresAt"), "무TTL 레코드에 expiresAt 줄이 붙었다");
        // TTL 이 붙으면 줄이 정확히 하나 늘고 그 줄이 말미다 — 그리고 구 서명은 그 순간 무효다.
        let mut t = legacy.clone();
        t.expires_at = Some(4242.0);
        let p2 = String::from_utf8(t.signing_payload()).unwrap();
        assert_eq!(p2, format!("{FROZEN_LEGACY_PAYLOAD}\nexpiresAt=4242"));
        assert!(!t.has_valid_signature(SECRET), "만료 주입이 구 서명을 통과했다");
    }

    /// TTL 은 서명에 묶인다 — 만료 연장(값 변경)·제거·주입 전부 서명 불일치로 거부.
    #[test]
    fn ttl_tamper_rejected() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        assert!(r.has_valid_signature(SECRET));

        let mut extend = r.clone();
        extend.expires_at = Some(9_999_999.0);
        assert!(!extend.has_valid_signature(SECRET), "만료 연장이 서명을 통과했다");

        let mut strip = r.clone();
        strip.expires_at = None;
        assert!(!strip.has_valid_signature(SECRET), "만료 제거(무기한 승격)가 서명을 통과했다");

        // 반대 방향: 무기한 레코드에 만료를 주입해도(=축소) 서명 불일치.
        let mut base = rec(&["git", "push"], Some("/x"), &[]);
        base.sign(SECRET);
        let mut inject = base.clone();
        inject.expires_at = Some(1.0);
        assert!(!inject.has_valid_signature(SECRET), "만료 주입이 서명을 통과했다");
    }

    /// 신선한 TTL 레코드는 `--require-ttl` 유무와 무관하게 매칭된다.
    #[test]
    fn ttl_fresh_matches_both_modes() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        let recs = vec![r];
        let now = 1500.0;
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                .is_some(),
            "신선한 TTL 레코드가 일반 check 에서 탈락"
        );
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                .is_some(),
            "신선한 TTL 레코드가 --require-ttl 에서 탈락"
        );
    }

    /// ★만료 레코드는 **요구 여부와 무관하게** 매칭되지 않는다(만료는 사실이지 옵션이 아니다).
    /// 경계 포함: `now == expires_at` 도 만료다.
    #[test]
    fn ttl_expired_never_matches() {
        let mut r = rec_ttl(&["git", "push"], Some("/x"), 2000.0);
        r.sign(SECRET);
        let recs = vec![r];
        for (now, label) in [(2000.0_f64, "경계(now==expires_at)"), (2000.1, "만료 후")] {
            assert!(
                best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                    .is_none(),
                "{label}: 만료 레코드가 일반 check 를 통과했다"
            );
            assert!(
                best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                    .is_none(),
                "{label}: 만료 레코드가 --require-ttl 을 통과했다"
            );
        }
    }

    /// ★음성 대조(결측은 값이 아니다): TTL **없는** 레코드는 `--require-ttl` 에서 실패하고,
    /// 그 실패가 '일반 check 도 못 쓴다'로 번지지 않는다(무기한 승인의 종전 계약 보존).
    #[test]
    fn ttl_missing_record_fails_require_ttl_but_keeps_legacy_check() {
        let mut r = rec(&["git", "push"], Some("/x"), &[]);
        r.sign(SECRET);
        let recs = vec![r];
        let now = 5000.0;
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, true)
                .is_none(),
            "TTL 없는 레코드가 --require-ttl 을 통과했다(계약 B-3 위반)"
        );
        assert!(
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], now, false)
                .is_some(),
            "TTL 없는 레코드의 종전 check 통과가 깨졌다(구 승인 전멸)"
        );
    }

    /// 만료 레코드가 섞여 있어도 **살아있는 짧은 prefix** 를 가리지 않는다 — 만료분을 먼저
    /// 걸러내지 않고 최장 prefix 를 고르면 "가장 잘 맞는 승인이 죽었다"가 곧 전면 거부가 된다.
    #[test]
    fn expired_longest_prefix_does_not_shadow_live_shorter() {
        let mut dead = rec_ttl(&["git", "push", "origin"], Some("/x"), 1000.0);
        dead.id = "ap-dead".into();
        dead.sign(SECRET);
        let mut live = rec(&["git", "push"], Some("/x"), &[]);
        live.id = "ap-live".into();
        live.sign(SECRET);
        let recs = vec![dead, live];
        let best =
            best_match_at(&recs, SECRET, "git push origin main", Some("/x"), &[], 5000.0, false)
                .expect("살아있는 짧은 prefix 가 선택돼야");
        assert_eq!(best.id, "ap-live", "만료된 최장 prefix 가 살아있는 승인을 가렸다");
    }

    // ── ★서명 세탁·만료 연장 (codex gpt-6-astra 위임 작성 · 전 줄 검토 후 채택) ──
    // 채택 시 수정 1건: T2 의 마지막 단언 문안이 "TTL 필수 여부와 무관하게"라고 말하면서 실제로는
    // `require_ttl=false` 만 재고 있었다 — 말과 측정이 어긋나면 그 문장이 다음 사람을 속인다.
    // 두 모드를 **둘 다 재도록** 고쳐서 문안을 사실로 만들었다.

    /// 같은 id의 위조본을 앞에 삽입하면 id 재검색으로 재서명 대상을 바꿀 수 있다.
    /// 더 긴 prefix와 먼 만료 시각을 가진 위조본도 검증된 인덱스를 대신해서는 안 된다.
    /// 이 검사가 실패하면 approval.check의 재서명이 위조본을 유효한 승인으로 세탁할 수 있다.
    #[test]
    fn duplicate_id_forgery_is_never_the_selected_index() {
        let forged = rec_ttl(&["git", "push", "origin"], None, 1_000_000_000.0);
        let mut valid = rec_ttl(&["git", "push"], None, 2000.0);
        valid.sign(SECRET);

        assert_eq!(forged.id, "ap-test-1", "위조본의 id가 대조 조건과 다릅니다");
        assert_eq!(forged.id, valid.id, "두 검체의 id가 같아야 공격을 재현합니다");
        assert!(!forged.has_valid_signature(SECRET), "위조본은 서명이 무효여야 합니다");
        let records = vec![forged, valid];

        for require_ttl in [false, true] {
            let selected = best_match_index_at(
                &records, SECRET, "git push origin main", None, &[], 1500.0, require_ttl,
            );
            assert_eq!(
                selected,
                Some(1),
                "검증된 두 번째 레코드를 선택해야 합니다: TTL 필수={require_ttl}"
            );
            let idx = selected.expect("유효한 레코드의 인덱스가 누락되었습니다");
            assert!(
                records[idx].has_valid_signature(SECRET),
                "선택된 레코드의 서명이 무효입니다: TTL 필수={require_ttl}"
            );
            assert_eq!(
                records[idx].command_prefix,
                vec!["git".to_string(), "push".to_string()],
                "선택된 prefix가 유효본과 다릅니다: TTL 필수={require_ttl}"
            );
            assert_eq!(
                best_match_index_at(
                    &records[..1], SECRET, "git push origin main", None, &[], 1500.0, require_ttl,
                ),
                None,
                "위조본만 남아도 무효 서명을 선택해서는 안 됩니다: TTL 필수={require_ttl}",
            );
        }
    }

    /// 승인 사용 시 updated_at 갱신과 재서명이 절대 만료 시각을 연장해서는 안 된다.
    /// 만료 직전 사용을 반복하는 공격자가 승인을 계속 살려 둘 수 있는지 검사한다.
    /// 이 검사가 실패하면 처음 정한 TTL 이후에도 같은 승인이 재사용될 수 있다.
    #[test]
    fn resigning_updated_at_preserves_absolute_expiry() {
        let mut record = rec_ttl(&["git", "push"], None, 2000.0);
        let initial_expiry = record.expires_at;
        record.sign(SECRET);
        assert!(record.has_valid_signature(SECRET), "초기 TTL 레코드의 서명이 무효입니다");
        assert_eq!(record.expires_at, initial_expiry, "초기 서명이 만료 시각을 변경했습니다");

        for updated_at in [1200.0, 1600.0, 1999.0] {
            record.updated_at = updated_at;
            record.sign(SECRET);
            assert!(
                record.has_valid_signature(SECRET),
                "updated_at={updated_at} 갱신 후 재서명이 무효입니다"
            );
            assert_eq!(
                record.expires_at, initial_expiry,
                "updated_at={updated_at} 갱신 후 절대 만료 시각이 변경되었습니다"
            );
        }

        let records = [record];
        assert_eq!(
            best_match_index_at(&records, SECRET, "git push origin main", None, &[], 1999.0, false),
            Some(0),
            "재서명된 승인은 초기 만료 시각 전에는 선택되어야 합니다",
        );
        for require_ttl in [false, true] {
            assert_eq!(
                best_match_index_at(
                    &records, SECRET, "git push origin main", None, &[], 2001.0, require_ttl,
                ),
                None,
                "재서명을 반복해도 초기 만료 시각 이후에는 거부해야 합니다: TTL 필수={require_ttl}",
            );
        }
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

    // ── ★(0.14.31 · WP-4 R2 · codex major) TTL 저장소 분리의 **호환성 검체** ──────────────
    // 위임 작성: codex(gpt-6-astra) · 전 줄 검토 후 채택(R4-WP4-r2-tests-prompt.md).
    // 이 두 검체가 재는 사실은 하나다: **구 데몬은 TTL 레코드를 파괴할 수 없다.** 공용 파일에
    // TTL 이 남아 있던 종전 배치에서는 구 데몬이 다른 승인 하나를 정상 사용하기만 해도
    // `expires_at` 이 사라지고, 서명에 그 값이 묶여 있으므로 그 승인은 **영구 거부**된다.
    // (음성 대조가 그 사고를 같은 파일에서 실제로 재현한다 — 수리를 지우면 검체가 빨강이 된다.)
    // ★HOME 을 프로세스 전역으로 바꾸므로 두 검체는 하나의 잠금으로 직렬화한다. 같은 파일의
    //   다른 검체는 HOME 을 만지지 않는다(handlers.rs 의 승인 RPC 검체는 자기 HOME 을 쓰는
    //   기존 관례를 따르며, 그 관례의 프로세스 전역 경합은 이 커밋의 범위 밖이다).
    // approval.rs의 mod tests 안에 그대로 삽입한다. 외부 crate나 기존 테스트 헬퍼는 필요 없다.
    // 함수별 static은 서로 다른 잠금이므로 두 테스트가 공유할 static 하나를 둔다.
    static TTL_STORE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ttl_record_survives_an_old_daemon_rewriting_the_shared_store() {
        // HOME과 secret은 프로세스 전역이다. 두 테스트가 반드시 같은 잠금을 사용한다.
        // 다른 HOME/env 접근 테스트까지 보호하려면 그 테스트도 이 잠금을 공유하거나
        // 테스트 실행기를 --test-threads=1로 실행해야 한다.
        let _lock = TTL_STORE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 잠금보다 나중에 선언하므로 환경 복원/정리가 끝난 뒤 잠금이 해제된다.
        // OsString으로 보관하여 비유니코드 값과 원래 미설정 상태까지 보존한다.
        struct RestoreHome {
            home: Option<std::ffi::OsString>,
            secret: Option<std::ffi::OsString>,
            temporary: std::path::PathBuf,
        }
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                for (key, old) in [
                    ("HOME", &self.home),
                    ("CYS_APPROVAL_SECRET_B64", &self.secret),
                ] {
                    match old {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
                let result = std::fs::remove_dir_all(&self.temporary);
                // 이미 실패하여 unwind 중이면 이중 panic으로 프로세스를 종료하지 않는다.
                if !std::thread::panicking() {
                    result.expect("테스트 종료 후 임시 HOME을 삭제하지 못했습니다");
                }
            }
        }
        let temporary = std::env::temp_dir().join(format!(
            "cys-ttl-old-writer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("고유 임시 HOME 이름을 만들 시스템 시각이 잘못되었습니다")
                .as_nanos()
        ));
        // create_dir은 기존 경로를 재사용하지 않으므로 타인의 파일을 정리하지 않는다.
        std::fs::create_dir(&temporary).expect("격리된 임시 HOME을 만들지 못했습니다");
        let restore = RestoreHome {
            home: std::env::var_os("HOME"),
            secret: std::env::var_os("CYS_APPROVAL_SECRET_B64"),
            temporary,
        };
        let secret: &[u8] = b"ttl-store-compatibility-secret-32-bytes";
        std::env::set_var("HOME", &restore.temporary);
        std::env::set_var("CYS_APPROVAL_SECRET_B64", b64_encode(secret));
        let shared_path = restore.temporary.join(".cys/approvals.json");
        let ttl_path = restore.temporary.join(".cys/approvals-ttl.json");

        let mut legacy = ApprovalRecord {
            version: 1,
            id: "L".into(),
            command_prefix: vec!["git".into(), "status".into()],
            cwd: None,
            environment: vec![("CI".into(), "1".into())],
            created_at: 1000.0,
            updated_at: 1000.0,
            expires_at: None,
            signature: String::new(),
        };
        legacy.sign(secret);
        let mut ttl = ApprovalRecord {
            id: "T".into(),
            expires_at: Some(4_000_000_000.0),
            ..legacy.clone()
        };
        ttl.sign(secret);

        // 구 데몬의 모르는 필드 유실을 JSON 값 편집으로 재현한다.
        // 두 경로 중 공용 파일만 접근하며, 정상/음성 대조 모두 같은 writer를 쓴다.
        let old_daemon_rewrite = || {
            let mut value: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&shared_path).expect("구 데몬이 읽을 공용 승인 파일이 없습니다"),
            )
            .expect("공용 승인 파일이 올바른 JSON이 아닙니다");
            for record in value["records"]
                .as_array_mut()
                .expect("공용 파일에 records 배열이 없습니다")
            {
                record
                    .as_object_mut()
                    .expect("승인 레코드가 JSON 객체가 아닙니다")
                    .remove("expires_at");
            }
            std::fs::write(
                &shared_path,
                serde_json::to_vec(&value).expect("구 데몬 재저장 JSON 생성 실패"),
            )
            .expect("구 데몬이 공용 승인 파일을 덮어쓰지 못했습니다");
        };
        // 반환형이 ()인 배경 API와 Result인 현재 구현 모두에서 파일 내용으로 저장을 검증한다.
        let _ = save_records(&[legacy.clone(), ttl.clone()]);
        let ttl_before = std::fs::read(&ttl_path).expect("TTL 승인은 전용 파일에 저장되어야 합니다");
        old_daemon_rewrite();
        assert_eq!(
            std::fs::read(&ttl_path).expect("TTL 전용 파일이 사라졌습니다"),
            ttl_before,
            "구 데몬의 공용 파일 재저장이 TTL 전용 파일을 변경했습니다"
        );
        let loaded = load_records();
        assert_eq!(
            loaded.len(),
            2,
            "구 데몬 재저장 후 L과 T가 모두 남아야 합니다"
        );
        for expected in [&legacy, &ttl] {
            let actual = loaded
                .iter()
                .find(|r| r.id == expected.id)
                .expect("구 데몬 재저장 후 기존 승인 id가 사라졌습니다");
            assert_eq!(
                actual.expires_at, expected.expires_at,
                "승인 {}의 만료 시각이 구 데몬 재저장으로 변경되었습니다",
                expected.id
            );
            assert!(
                actual.has_valid_signature(secret),
                "승인 {}의 서명이 구 데몬 재저장으로 손상되었습니다",
                expected.id
            );
            assert_eq!(
                actual.signature, expected.signature,
                "승인 {}의 기존 서명이 보존되지 않았습니다",
                expected.id
            );
        }

        // 음성 대조: 분리 전처럼 동일한 서명 검체 T를 공용 파일에 강제로 넣는다.
        // load_records()는 정상 TTL 판을 우선하므로 손상본을 공용 파일에서 직접 읽는다.
        assert!(
            ttl.has_valid_signature(secret),
            "음성 대조의 원본 T부터 서명이 무효라면 유실 사고를 입증할 수 없습니다"
        );
        std::fs::write(
            &shared_path,
            serde_json::to_vec(&serde_json::json!({
                "records": [&legacy, &ttl]
            }))
            .expect("음성 대조 JSON 생성 실패"),
        )
        .expect("음성 대조를 공용 파일에 쓰지 못했습니다");
        old_daemon_rewrite();
        let damaged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&shared_path).expect("음성 대조 공용 파일 읽기 실패"),
        )
        .expect("음성 대조 공용 파일 JSON 해석 실패");
        let damaged: Vec<ApprovalRecord> = serde_json::from_value(damaged["records"].clone())
            .expect("구 데몬 재저장 결과를 승인 레코드로 읽지 못했습니다");
        assert_eq!(
            damaged.len(),
            2,
            "음성 대조에서 필드 제거가 레코드 삭제로 바뀌었습니다"
        );
        let damaged_ttl = damaged
            .iter()
            .find(|r| r.id == "T")
            .expect("음성 대조에서 T 자체가 사라졌습니다");
        assert_eq!(
            damaged_ttl.expires_at, None,
            "구 데몬 재저장 흉내가 T의 expires_at을 제거하지 않았습니다"
        );
        assert_eq!(
            damaged_ttl.signature, ttl.signature,
            "음성 대조는 서명 자체를 바꾸지 않고 만료 필드만 제거해야 합니다"
        );
        assert!(!damaged_ttl.has_valid_signature(secret),
            "공용 파일의 T가 만료 필드를 잃고도 서명 검증을 통과했습니다: 분리 전 사고가 재현되지 않았습니다");
        let intact_legacy = damaged
            .iter()
            .find(|r| r.id == "L")
            .expect("음성 대조에서 L이 사라졌습니다");
        assert!(
            intact_legacy.has_valid_signature(secret),
            "무기한 L까지 손상되었다면 TTL 필드 유실만을 재현한 검체가 아닙니다"
        );
    }

    #[test]
    fn saving_migrates_ttl_records_out_of_the_shared_store() {
        // HOME과 secret은 프로세스 전역이다. 두 테스트가 반드시 같은 잠금을 사용한다.
        // 다른 HOME/env 접근 테스트까지 보호하려면 그 테스트도 이 잠금을 공유하거나
        // 테스트 실행기를 --test-threads=1로 실행해야 한다.
        let _lock = TTL_STORE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 잠금보다 나중에 선언하므로 환경 복원/정리가 끝난 뒤 잠금이 해제된다.
        // OsString으로 보관하여 비유니코드 값과 원래 미설정 상태까지 보존한다.
        struct RestoreHome {
            home: Option<std::ffi::OsString>,
            secret: Option<std::ffi::OsString>,
            temporary: std::path::PathBuf,
        }
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                for (key, old) in [
                    ("HOME", &self.home),
                    ("CYS_APPROVAL_SECRET_B64", &self.secret),
                ] {
                    match old {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
                let result = std::fs::remove_dir_all(&self.temporary);
                // 이미 실패하여 unwind 중이면 이중 panic으로 프로세스를 종료하지 않는다.
                if !std::thread::panicking() {
                    result.expect("테스트 종료 후 임시 HOME을 삭제하지 못했습니다");
                }
            }
        }
        let temporary = std::env::temp_dir().join(format!(
            "cys-ttl-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("고유 임시 HOME 이름을 만들 시스템 시각이 잘못되었습니다")
                .as_nanos()
        ));
        // create_dir은 기존 경로를 재사용하지 않으므로 타인의 파일을 정리하지 않는다.
        std::fs::create_dir(&temporary).expect("격리된 임시 HOME을 만들지 못했습니다");
        let restore = RestoreHome {
            home: std::env::var_os("HOME"),
            secret: std::env::var_os("CYS_APPROVAL_SECRET_B64"),
            temporary,
        };
        let secret: &[u8] = b"ttl-store-compatibility-secret-32-bytes";
        std::env::set_var("HOME", &restore.temporary);
        std::env::set_var("CYS_APPROVAL_SECRET_B64", b64_encode(secret));
        let shared_path = restore.temporary.join(".cys/approvals.json");
        let ttl_path = restore.temporary.join(".cys/approvals-ttl.json");

        let mut legacy = ApprovalRecord {
            version: 1,
            id: "L".into(),
            command_prefix: vec!["git".into(), "status".into()],
            cwd: None,
            environment: vec![("CI".into(), "1".into())],
            created_at: 1000.0,
            updated_at: 1000.0,
            expires_at: None,
            signature: String::new(),
        };
        legacy.sign(secret);
        let mut ttl = ApprovalRecord {
            id: "T".into(),
            expires_at: Some(4_000_000_000.0),
            ..legacy.clone()
        };
        ttl.sign(secret);

        // 분리 저장 이전 배치: 공용 파일 하나에 L, T 순서로 유효한 승인을 둔다.
        std::fs::create_dir_all(
            shared_path
                .parent()
                .expect("공용 파일의 상위 경로가 없습니다"),
        )
        .expect("임시 HOME 안에 승인 디렉터리를 만들지 못했습니다");
        std::fs::write(
            &shared_path,
            serde_json::to_vec(&serde_json::json!({
                "records": [&legacy, &ttl]
            }))
            .expect("이전 배치 JSON 생성 실패"),
        )
        .expect("이전 배치 공용 파일 생성 실패");
        assert!(
            !ttl_path.exists(),
            "이전 배치 검체에는 TTL 전용 파일이 없어야 합니다"
        );
        let loaded = load_records();
        assert_eq!(
            loaded.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L", "T"],
            "분리 전 공용 파일을 읽을 때 승인 id 또는 L, T 순서가 바뀌었습니다"
        );
        for record in &loaded {
            assert!(
                record.has_valid_signature(secret),
                "마이그레이션 전 승인 {}의 서명이 무효입니다",
                record.id
            );
        }
        let before = serde_json::to_value(&loaded).expect("마이그레이션 전 검체 스냅샷 생성 실패");
        let _ = save_records(&loaded);

        let shared: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&shared_path).expect("마이그레이션 후 공용 파일이 없습니다"),
        )
        .expect("마이그레이션 후 공용 파일 JSON 해석 실패");
        let shared_records = shared["records"]
            .as_array()
            .expect("공용 파일에 records 배열이 없습니다");
        assert_eq!(
            shared_records
                .iter()
                .filter(|r| r.get("expires_at").is_some())
                .count(),
            0,
            "공용 파일에 expires_at 키가 남아 있어 구 데몬 재저장으로 TTL 승인이 손상될 수 있습니다"
        );
        assert_eq!(
            shared_records.len(),
            1,
            "마이그레이션 후 공용 파일에는 무기한 L 하나만 있어야 합니다"
        );
        assert_eq!(
            shared_records[0]["id"], "L",
            "공용 파일에서 무기한 L이 사라지거나 T가 남았습니다"
        );
        let dedicated: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&ttl_path).expect("마이그레이션이 TTL 전용 파일을 만들지 않았습니다"),
        )
        .expect("TTL 전용 파일 JSON 해석 실패");
        let dedicated: Vec<ApprovalRecord> = serde_json::from_value(dedicated["records"].clone())
            .expect("TTL 전용 파일의 records를 승인 목록으로 읽지 못했습니다");
        assert_eq!(
            dedicated.len(),
            1,
            "TTL 전용 파일에는 T 하나만 있어야 합니다"
        );
        assert_eq!(
            dedicated[0].id, "T",
            "마이그레이션 후 TTL 전용 파일에 T가 없습니다"
        );
        assert_eq!(
            dedicated[0].expires_at, ttl.expires_at,
            "TTL 전용 파일에서 T의 만료 시각이 변경되었습니다"
        );
        assert!(
            dedicated[0].has_valid_signature(secret),
            "TTL 전용 파일의 T 서명이 마이그레이션 중 손상되었습니다"
        );
        let reloaded = load_records();
        assert_eq!(
            reloaded.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["L", "T"],
            "분리 저장 후 다시 읽을 때 승인 id 또는 L, T 순서가 보존되지 않았습니다"
        );
        for record in &reloaded {
            assert!(
                record.has_valid_signature(secret),
                "분리 저장 후 승인 {}의 서명이 무효입니다",
                record.id
            );
        }
        // 재서명 등 우연한 복구로 검사를 통과하지 못하게 모든 필드와 기존 서명도 비교한다.
        assert_eq!(
            serde_json::to_value(&reloaded).expect("마이그레이션 후 스냅샷 생성 실패"),
            before,
            "마이그레이션은 저장 위치만 바꿔야 하는데 승인 내용, 서명 또는 순서가 변경되었습니다"
        );
    }

}
